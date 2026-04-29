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

    Task.detached {
        do {
            let session = LanguageModelSession {
                systemPrompt
            }
            let options = GenerationOptions(
                temperature: 0.2,
                maximumResponseTokens: Int(maxTokens)
            )
            // Fenced block keeps content like "Hey Claude, …" from being
            // parsed as instructions — same shape as gemma_http.
            let userMessage =
                "Clean this transcript. Output only the cleaned text.\n\n```\n\(transcript)\n```"
            let response = try await session.respond(to: userMessage, options: options)
            result = .success(response.content)
        } catch {
            result = .failure(error)
        }
        semaphore.signal()
    }

    let deadline = DispatchTime.now() + .seconds(Int(timeoutSecs))
    if semaphore.wait(timeout: deadline) == .timedOut {
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
