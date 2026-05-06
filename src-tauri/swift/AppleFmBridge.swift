// Swift ↔ Rust bridge for Apple Intelligence Foundation Models polishing.
//
// Compiled as a static library by `build.rs` on macOS and linked into the
// Tauri binary. The C ABI here is consumed from `engines::apple_fm_bridge`.
//
// Why in-process: Foundation Models brokers to a system daemon
// (modelinferenced) that runs the actual inference, so a model crash
// can't take the menu-bar app down with it the way an in-process llama.cpp
// assert could. Errors surface as typed `LanguageModelSession.GenerationError`
// which we serialise into the thread-local error slot and `engines::apple_fm`
// translates into the existing fail-open path.
//
// Compatibility: built with deployment target macOS 14.0 so the binary
// still loads on older Macs; `if #available(macOS 26, *)` gates every
// FoundationModels call. `#if canImport(FoundationModels)` makes the
// whole feature compile out cleanly on older SDKs (CI machines, etc.).

import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

// Availability codes — must match `engines::apple_fm::AvailabilityCode` in Rust.
private let kAvailable: Int32 = 0
private let kDeviceNotEligible: Int32 = 1
private let kIntelligenceNotEnabled: Int32 = 2
private let kModelNotReady: Int32 = 3
private let kOsTooOld: Int32 = 4
private let kInternalError: Int32 = 5
private let kFrameworkMissing: Int32 = 6

// Per-thread error slot. Polishing is invoked from the tokio
// `spawn_blocking` worker pool, so each Rust call sits on its own pthread
// — no risk of two calls clobbering each other's error message.
private final class ErrorSlot: @unchecked Sendable {
    static let shared = ErrorSlot()
    private var threadKey = pthread_key_t()

    init() {
        let destructor: @convention(c) (UnsafeMutableRawPointer) -> Void = { raw in
            Unmanaged<NSString>.fromOpaque(raw).release()
        }
        pthread_key_create(&threadKey, destructor)
    }

    func set(_ message: String) {
        // Replace any prior message on this thread.
        if let existing = pthread_getspecific(threadKey) {
            Unmanaged<NSString>.fromOpaque(existing).release()
        }
        let ns = message as NSString
        pthread_setspecific(threadKey, Unmanaged.passRetained(ns).toOpaque())
    }

    func take() -> String? {
        guard let raw = pthread_getspecific(threadKey) else { return nil }
        let ns = Unmanaged<NSString>.fromOpaque(raw).takeRetainedValue()
        pthread_setspecific(threadKey, nil)
        return ns as String
    }
}

// Session cache keyed by full system-prompt string. Reusing a session
// across calls with the same prompt skips Foundation Models' system-
// prompt prefill (a meaningful TTFT win on Apple Intelligence).
//
// Swift actor isolation gives us two things at once:
//   1. Mutating the LRU map is serialized (no `NSLock` needed).
//   2. A vended session is held inside the actor's state, so callers
//      that race on the same prompt naturally line up behind one
//      another via `await acquire(...)` / `release(...)`.
//
// `LanguageModelSession` itself is single-threaded per Apple's docs;
// even when two callers grab the same key we hand the session to one
// at a time. The cap of 4 covers the 5 built-in voice modes minus one
// rarely-active mode at any moment.
#if canImport(FoundationModels)
@available(macOS 26.0, *)
private actor SessionCache {
    static let shared = SessionCache()

    // Bounded LRU. `order` holds keys oldest→newest; `entries[k]`
    // is the cached session plus a one-slot waitlist used to serialise
    // concurrent calls hitting the same key.
    private struct Entry {
        var session: LanguageModelSession
        // Continuation queue for callers waiting on this session. The
        // session is "owned" by whichever caller most recently called
        // `acquire` and hasn't yet called `release`.
        var inUse: Bool = false
        var waiters: [CheckedContinuation<Void, Never>] = []
    }

    private var entries: [String: Entry] = [:]
    private var order: [String] = []
    private let cap = 4

    // Acquire exclusive use of the session for `prompt`, constructing a
    // new one (and prewarming it) on miss. Caller MUST pair this with
    // `release(prompt:)` once finished, success or failure.
    func acquire(prompt: String) async -> LanguageModelSession {
        return await acquire(prompt: prompt, depth: 0)
    }

    private func acquire(prompt: String, depth: Int) async -> LanguageModelSession {
        // Recursion is bounded by exactly one re-entry in normal
        // operation: a waiter wakes to find the entry has been
        // `evict`ed, then re-runs the cache-miss path which
        // synchronously installs a fresh entry with `inUse == true`.
        // Any deeper recursion would indicate either a bug in the
        // wait/wake protocol or an attacker-influenced storm of
        // mid-flight evictions — fail loud rather than silently
        // blowing the actor's stack.
        precondition(depth <= 1, "SessionCache.acquire recursion exceeded depth=1; aborting")

        // Touch LRU order on every access — if present, move to MRU.
        if entries[prompt] != nil {
            if let idx = order.firstIndex(of: prompt) {
                order.remove(at: idx)
            }
            order.append(prompt)
        } else {
            let session = LanguageModelSession {
                prompt
            }
            // Fire-and-forget prewarm — amortises the first-call cost
            // outside the user-visible TTFT for this very call. The
            // session is safe to use immediately; prewarm just primes.
            session.prewarm()
            entries[prompt] = Entry(session: session)
            order.append(prompt)
            evictIfNeeded()
        }

        // Wait our turn if another caller currently owns this session.
        while entries[prompt]?.inUse == true {
            await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
                entries[prompt]?.waiters.append(cont)
            }
        }
        // While we were suspended on the continuation, a concurrent
        // caller's stream task may have errored and called `evict` —
        // which removes the entry entirely and wakes all waiters. If
        // we resumed into that state, fall back through the cache-miss
        // path to construct a fresh session. The recursion is bounded:
        // after one trip through the miss branch the entry is present
        // with `inUse == true`, so we cannot recurse a second time —
        // the precondition at the top of this method enforces that.
        if entries[prompt] == nil {
            return await acquire(prompt: prompt, depth: depth + 1)
        }
        entries[prompt]!.inUse = true
        return entries[prompt]!.session
    }

    func release(prompt: String) {
        guard var entry = entries[prompt] else { return }
        entry.inUse = false
        let next = entry.waiters.isEmpty ? nil : entry.waiters.removeFirst()
        entries[prompt] = entry
        next?.resume()
    }

    // Drop a session that errored mid-use so the next caller starts
    // clean rather than inheriting a corrupted transcript history.
    func evict(prompt: String) {
        if let idx = order.firstIndex(of: prompt) {
            order.remove(at: idx)
        }
        if var entry = entries.removeValue(forKey: prompt) {
            // Wake any waiters so they can re-acquire (which will
            // reconstruct a fresh session on the next acquire call).
            for waiter in entry.waiters {
                waiter.resume()
            }
            entry.waiters.removeAll()
        }
    }

    private func evictIfNeeded() {
        // Bounded scan: at most one full pass. If every cached entry
        // is currently in use we accept being temporarily over cap
        // rather than yanking a session out from under an active call.
        guard order.count > cap else { return }
        var scanned = 0
        let scanCap = order.count
        while order.count > cap && scanned < scanCap {
            let key = order.removeFirst()
            scanned += 1
            if let entry = entries[key], entry.inUse {
                order.append(key)
                continue
            }
            entries.removeValue(forKey: key)
        }
    }
}
#endif

@_cdecl("vox_apple_fm_availability")
public func vox_apple_fm_availability() -> Int32 {
#if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        // Pattern-match on the typed enum where possible; fall back to a
        // string comparison so a future-Apple addition to the
        // UnavailableReason enum doesn't make us silently report
        // "internal error" — we'd rather show "intelligence disabled" if
        // that's the most likely cause.
        switch SystemLanguageModel.default.availability {
        case .available:
            return kAvailable
        case .unavailable(let reason):
            let label = "\(reason)".lowercased()
            if label.contains("device") {
                return kDeviceNotEligible
            } else if label.contains("intelligence") || label.contains("notenabled") || label.contains("not_enabled") {
                return kIntelligenceNotEnabled
            } else if label.contains("ready") || label.contains("download") {
                return kModelNotReady
            } else {
                ErrorSlot.shared.set("Apple Intelligence unavailable: \(reason)")
                return kInternalError
            }
        @unknown default:
            return kInternalError
        }
    } else {
        return kOsTooOld
    }
#else
    return kFrameworkMissing
#endif
}

@_cdecl("vox_apple_fm_polish_blocking")
public func vox_apple_fm_polish_blocking(
    _ systemPromptPtr: UnsafePointer<CChar>?,
    _ transcriptPtr: UnsafePointer<CChar>?,
    _ maxTokens: Int32,
    _ timeoutSecs: UInt32
) -> UnsafeMutablePointer<CChar>? {
#if canImport(FoundationModels)
    guard #available(macOS 26.0, *) else {
        ErrorSlot.shared.set("Apple Intelligence requires macOS 26 or later")
        return nil
    }
    guard let sp = systemPromptPtr, let tp = transcriptPtr else {
        ErrorSlot.shared.set("null pointer for prompt or transcript")
        return nil
    }

    let systemPrompt = String(cString: sp)
    let transcript = String(cString: tp)

    let semaphore = DispatchSemaphore(value: 0)
    // `nonisolated(unsafe)` is fine: the semaphore enforces the
    // happens-before edge between the Task's write and our read.
    nonisolated(unsafe) var result: Result<String, Error>? = nil

    // Lifetime gate for the detached Task — same shape as the streaming
    // path. `done` flips to true when EITHER the task completes
    // naturally (success or error) OR the timeout path fires. Both
    // sides consult this flag under the lock so the timeout path can
    // distinguish "task finished after we gave up" from "task is still
    // alive and we need to evict its session".
    let doneLock = NSLock()
    nonisolated(unsafe) var done = false

    let task = Task.detached {
        // Reuse a cached session keyed by systemPrompt to skip
        // Foundation Models' system-prompt prefill on repeat calls.
        let session = await SessionCache.shared.acquire(prompt: systemPrompt)
        var taskResult: Result<String, Error>
        do {
            let options = GenerationOptions(
                temperature: 0.2,
                maximumResponseTokens: Int(maxTokens)
            )
            // Fenced block keeps content like "Hey Claude, …" from being
            // parsed as instructions — same shape as gemma_http.
            let userMessage =
                "Clean this transcript. Output only the cleaned text.\n\n```\n\(transcript)\n```"
            let response = try await session.respond(to: userMessage, options: options)
            taskResult = .success(response.content)
        } catch {
            taskResult = .failure(error)
        }
        // Decide cache fate AFTER checking whether the timeout path
        // beat us to the punch. If `done` is already set the parent
        // pthread has returned (or is about to), so the result is
        // discarded — and either way we must evict, not release: a
        // cancelled-mid-call or timed-out session may carry partial
        // state that would corrupt the next caller.
        doneLock.lock()
        let timedOut = done
        done = true
        doneLock.unlock()
        if timedOut || Task.isCancelled {
            await SessionCache.shared.evict(prompt: systemPrompt)
        } else {
            switch taskResult {
            case .success:
                await SessionCache.shared.release(prompt: systemPrompt)
            case .failure:
                // Drop the session on error — its in-flight transcript
                // may be corrupted; safer to start fresh next call.
                await SessionCache.shared.evict(prompt: systemPrompt)
            }
            result = taskResult
        }
        semaphore.signal()
    }

    let deadline = DispatchTime.now() + .seconds(Int(timeoutSecs))
    if semaphore.wait(timeout: deadline) == .timedOut {
        // Mark done first so the racing task sees `timedOut == true`
        // when it eventually finishes and routes its session to
        // `evict` instead of `release`. Then cancel cooperatively so
        // the underlying Foundation Models call has a chance to bail
        // out if it supports cancellation.
        doneLock.lock()
        done = true
        doneLock.unlock()
        task.cancel()
        ErrorSlot.shared.set("Apple Intelligence polish timed out after \(timeoutSecs)s")
        return nil
    }

    switch result {
    case .some(.success(let content)):
        // strdup so the Rust side can free with libc::free without
        // needing to round-trip through a Swift deallocator.
        return strdup(content)
    case .some(.failure(let error)):
        ErrorSlot.shared.set("\(error)")
        return nil
    case .none:
        ErrorSlot.shared.set("internal: Apple Intelligence task did not signal")
        return nil
    }
#else
    ErrorSlot.shared.set("Foundation Models framework not available in this build")
    return nil
#endif
}

// Streaming variant — fires `onChunk` with cumulative partials as the
// session yields them, then signals completion. Blocks the calling
// pthread on a DispatchSemaphore (callers run from `spawn_blocking`).
//
// The `streamResponse(to:)` API on macOS 26 yields partial snapshots of
// the in-progress response. For a String-typed session each yielded
// element is a partial string we forward straight to Rust. We stringify
// via `String(describing:)` defensively so this still compiles if the
// SDK changes the partial wrapper shape between betas.
//
// Use-after-free safety on timeout: if the deadline fires while the
// detached Task is still iterating the stream, the Rust caller will
// reclaim its `StreamCtx` Box on return — but the Task is still alive
// and could call `onChunk` against freed memory. We guard with an
// NSLock-protected `done` flag plus a brief drain window:
//   1. Inside the for-await loop, re-check `done` under the lock and
//      return without firing onChunk if it's been set.
//   2. The timeout path acquires the lock, sets `done = true`, calls
//      `task.cancel()`, then drops the lock and sleeps ~50 ms to let
//      any in-flight invocation finish unwinding.
// After the drain window the contract is: no further onChunk callback
// can be entered, so Rust's Box::from_raw is safe to reclaim.
@_cdecl("vox_apple_fm_polish_stream")
public func vox_apple_fm_polish_stream(
    _ systemPromptPtr: UnsafePointer<CChar>?,
    _ transcriptPtr: UnsafePointer<CChar>?,
    _ maxTokens: Int32,
    _ timeoutSecs: UInt32,
    _ userData: UnsafeMutableRawPointer?,
    _ onChunk: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Void,
    _ onDone: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Void
) -> Int32 {
#if canImport(FoundationModels)
    guard #available(macOS 26.0, *) else {
        ErrorSlot.shared.set("Apple Intelligence requires macOS 26 or later")
        return 1
    }
    guard let sp = systemPromptPtr, let tp = transcriptPtr else {
        ErrorSlot.shared.set("null pointer for prompt or transcript")
        return 2
    }

    let systemPrompt = String(cString: sp)
    let transcript = String(cString: tp)
    let semaphore = DispatchSemaphore(value: 0)
    nonisolated(unsafe) var finalResult: Result<String, Error>? = nil

    // Lifetime gate for the detached Task. `done` flips to true when
    // EITHER the task finishes naturally OR the timeout path fires.
    // Both the chunk-loop and the timeout path consult this flag under
    // the lock so the trampoline never fires after the Rust caller
    // reclaims the StreamCtx.
    let doneLock = NSLock()
    nonisolated(unsafe) var done = false

    let task = Task.detached {
        // Reuse a cached session keyed by systemPrompt — same TTFT
        // win as the blocking path. Release on every exit including
        // cancellation; on error evict so the next caller starts clean.
        let session = await SessionCache.shared.acquire(prompt: systemPrompt)
        var didErr = false
        do {
            let options = GenerationOptions(
                temperature: 0.2,
                maximumResponseTokens: Int(maxTokens)
            )
            let userMessage =
                "Clean this transcript. Output only the cleaned text.\n\n```\n\(transcript)\n```"

            // The session yields cumulative snapshots; we coerce each
            // element to a String defensively so a beta-SDK shape
            // change doesn't break compilation.
            var lastSnapshot = ""
            let stream = session.streamResponse(to: userMessage, options: options)
            for try await partial in stream {
                // Cooperative cancellation check — fires when the
                // timeout path called task.cancel().
                if Task.isCancelled { break }
                let snapshot = String(describing: partial)
                if snapshot != lastSnapshot {
                    lastSnapshot = snapshot
                    // Hold the lock across the trampoline call so a
                    // racing timeout-path setter cannot mark `done`
                    // while we're mid-callback. The timeout path's
                    // post-cancel drain sleep covers any callback
                    // already entered before `done` was set.
                    doneLock.lock()
                    if done {
                        doneLock.unlock()
                        break
                    }
                    snapshot.withCString { ptr in
                        onChunk(userData, ptr)
                    }
                    doneLock.unlock()
                }
            }
            finalResult = .success(lastSnapshot)
        } catch {
            didErr = true
            finalResult = .failure(error)
        }
        // A cancelled task (timeout path) leaves the session in an
        // unknown state; treat that the same as an error and evict.
        if didErr || Task.isCancelled {
            await SessionCache.shared.evict(prompt: systemPrompt)
        } else {
            await SessionCache.shared.release(prompt: systemPrompt)
        }
        // Mark done before signaling so a timeout racing the natural
        // completion sees `done == true` and skips its cancel/drain.
        doneLock.lock()
        done = true
        doneLock.unlock()
        semaphore.signal()
    }

    let deadline = DispatchTime.now() + .seconds(Int(timeoutSecs))
    if semaphore.wait(timeout: deadline) == .timedOut {
        // Acquire the lock to suppress further chunk emissions, cancel
        // the task, then drain briefly so any callback in-flight at the
        // moment we acquired the lock has time to unwind. Rust's
        // Box::from_raw on return is then safe.
        doneLock.lock()
        done = true
        doneLock.unlock()
        task.cancel()
        // 50 ms drain window. The chunk-loop only holds the lock for
        // the duration of one onChunk invocation (a few µs), so this
        // is generous; if Apple's framework ever blocks inside the
        // trampoline for longer, bump this number.
        Thread.sleep(forTimeInterval: 0.05)
        ErrorSlot.shared.set("Apple Intelligence stream polish timed out after \(timeoutSecs)s")
        onDone(userData, nil)
        return 3
    }

    switch finalResult {
    case .some(.success(let content)):
        content.withCString { ptr in
            onDone(userData, ptr)
        }
        return 0
    case .some(.failure(let error)):
        ErrorSlot.shared.set("\(error)")
        onDone(userData, nil)
        return 4
    case .none:
        ErrorSlot.shared.set("internal: Apple Intelligence stream task did not signal")
        onDone(userData, nil)
        return 5
    }
#else
    ErrorSlot.shared.set("Foundation Models framework not available in this build")
    _ = (systemPromptPtr, transcriptPtr, maxTokens, timeoutSecs, userData, onChunk, onDone)
    return 6
#endif
}

@_cdecl("vox_apple_fm_take_last_error")
public func vox_apple_fm_take_last_error() -> UnsafeMutablePointer<CChar>? {
    if let msg = ErrorSlot.shared.take() {
        return strdup(msg)
    }
    return nil
}

@_cdecl("vox_apple_fm_free")
public func vox_apple_fm_free(_ ptr: UnsafeMutablePointer<CChar>?) {
    if let p = ptr {
        free(p)
    }
}
