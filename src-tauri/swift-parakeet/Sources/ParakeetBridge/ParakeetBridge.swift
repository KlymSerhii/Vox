// Swift ↔ Rust bridge for the FluidAudio Parakeet ASR engine.
//
// Compiled as a static library by `build.rs` on macOS arm64 and linked
// into the Tauri binary. The C ABI here is consumed from
// `engines::parakeet_bridge`.
//
// Why in-process: FluidAudio dispatches to CoreML which itself brokers
// to the Apple Neural Engine via the system `aned` daemon, so a model
// crash can't take the menu-bar app down — same crash-isolation
// argument as the AppleFmBridge precedent. In-process gets us
// sub-millisecond bridge overhead instead of stdin/stdout JSON.
//
// Compatibility: deployment target macOS 14.0 (FluidAudio's floor).
// Apple-Silicon-only at link time — gated upstream in build.rs and
// Cargo features so x86_64 builds skip this module entirely.

import Foundation
import os
import FluidAudio

// Per-thread error slot. Same shape as AppleFmBridge's — STT runs from
// the tokio `spawn_blocking` worker pool, so each Rust call sits on
// its own pthread.
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

// Single cached `AsrModels` + `AsrManager` keyed by source directory.
// FluidAudio's `AsrManager` is an `actor`, so concurrent transcribe
// calls naturally serialize via Swift actor isolation — we don't need
// our own mutex around the manager.
private actor ModelsCache {
    static let shared = ModelsCache()

    private var loadedDir: String?
    private var manager: AsrManager?

    /// Load (idempotent) the AsrModels at `directory` and configure an
    /// `AsrManager` against them. Throws on load failure.
    func loadIfNeeded(directory: String) async throws -> AsrManager {
        if let mgr = manager, loadedDir == directory {
            return mgr
        }

        let url = URL(fileURLWithPath: directory)
        let models = try await AsrModels.load(from: url, version: .v3)

        let mgr = AsrManager(config: .default, models: models)
        try await mgr.loadModels(models)

        manager = mgr
        loadedDir = directory

        await warmup()

        return mgr
    }

    /// Run one synthetic transcribe on 1 s of silence so CoreML kernel
    /// JIT compile + ANE prime happen here instead of on the user's
    /// first hotkey press.
    private func warmup() async {
        guard let mgr = manager else { return }
        let silence = [Float](repeating: 0, count: 16_000)
        var state = TdtDecoderState.make()
        do {
            _ = try await mgr.transcribe(silence, decoderState: &state, language: nil)
        } catch {
            print("[ParakeetBridge] warmup failed: \(error)")
        }
    }

    func currentManager() -> AsrManager? {
        return manager
    }

    /// Drop the cached `AsrManager` + loaded-dir pointer so the next
    /// `loadIfNeeded` re-reads model files from disk. Called from the
    /// Rust side after `models::delete_model` so a re-download lands
    /// on a fresh (re-loaded) manager instead of the stale in-memory
    /// copy from before the delete.
    func evict() {
        manager = nil
        loadedDir = nil
    }
}

@_cdecl("vox_parakeet_load")
public func vox_parakeet_load(_ modelsDirPtr: UnsafePointer<CChar>?) -> Int32 {
    guard let p = modelsDirPtr else {
        ErrorSlot.shared.set("null pointer for modelsDir")
        return 1
    }
    let dir = String(cString: p)

    let semaphore = DispatchSemaphore(value: 0)
    nonisolated(unsafe) var result: Result<Void, Error>? = nil

    Task.detached {
        do {
            _ = try await ModelsCache.shared.loadIfNeeded(directory: dir)
            result = .success(())
        } catch {
            result = .failure(error)
        }
        semaphore.signal()
    }

    semaphore.wait()

    switch result {
    case .some(.success):
        return 0
    case .some(.failure(let err)):
        ErrorSlot.shared.set("\(err)")
        return 1
    case .none:
        ErrorSlot.shared.set("internal: parakeet load did not signal")
        return 1
    }
}

@_cdecl("vox_parakeet_transcribe_blocking")
public func vox_parakeet_transcribe_blocking(
    _ samplesPtr: UnsafePointer<Float>?,
    _ samplesLen: Int,
    _ timeoutSecs: Int32,
    _ outText: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    guard let sp = samplesPtr, samplesLen > 0 else {
        ErrorSlot.shared.set("null or empty samples buffer")
        return 1
    }
    guard let outSlot = outText else {
        ErrorSlot.shared.set("null outText pointer")
        return 1
    }

    // Copy PCM out of the Rust-owned buffer up-front. The Task below
    // outlives our stack frame on timeout, so the Swift side must own
    // the samples by the time it dispatches.
    let samples = Array(UnsafeBufferPointer(start: sp, count: samplesLen))

    let semaphore = DispatchSemaphore(value: 0)
    nonisolated(unsafe) var result: Result<String, Error>? = nil

    // Lifetime gate so a timed-out Task doesn't write to `result` after
    // the Rust caller has reclaimed the stack. `OSAllocatedUnfairLock`
    // is the async-safe replacement for NSLock under Swift 6 strict
    // concurrency (NSLock.lock() is unavailable from async contexts).
    let doneFlag = OSAllocatedUnfairLock<Bool>(initialState: false)

    let task = Task.detached {
        var taskResult: Result<String, Error>
        do {
            guard let mgr = await ModelsCache.shared.currentManager() else {
                throw NSError(
                    domain: "ParakeetBridge",
                    code: -1,
                    userInfo: [NSLocalizedDescriptionKey: "AsrManager not loaded — call vox_parakeet_load first"]
                )
            }
            var state = TdtDecoderState.make()
            let asr = try await mgr.transcribe(samples, decoderState: &state, language: nil)
            taskResult = .success(asr.text)
        } catch {
            taskResult = .failure(error)
        }
        let timedOut = doneFlag.withLock { d -> Bool in
            let was = d
            d = true
            return was
        }
        if !timedOut && !Task.isCancelled {
            result = taskResult
        }
        semaphore.signal()
    }

    let deadline = DispatchTime.now() + .seconds(Int(timeoutSecs))
    if semaphore.wait(timeout: deadline) == .timedOut {
        doneFlag.withLock { d in d = true }
        task.cancel()
        // TODO(parakeet-streaming-v2): timed-out task still occupies
        // AsrManager's actor isolation — the cancel above only flips
        // the cooperative cancellation flag, but FluidAudio's
        // transcribe loop doesn't check it between CoreML calls. Until
        // we can either bound the per-call duration inside FluidAudio
        // or recreate the AsrManager on timeout, subsequent transcribes
        // will queue behind the zombie until it finishes naturally.
        ErrorSlot.shared.set("Parakeet transcription timed out after \(timeoutSecs)s")
        return 2
    }

    switch result {
    case .some(.success(let text)):
        outSlot.pointee = strdup(text)
        return 0
    case .some(.failure(let err)):
        ErrorSlot.shared.set("\(err)")
        return 1
    case .none:
        ErrorSlot.shared.set("internal: parakeet transcribe did not signal")
        return 1
    }
}

@_cdecl("vox_parakeet_take_last_error")
public func vox_parakeet_take_last_error() -> UnsafeMutablePointer<CChar>? {
    if let msg = ErrorSlot.shared.take() {
        return strdup(msg)
    }
    return nil
}

@_cdecl("vox_parakeet_free_string")
public func vox_parakeet_free_string(_ ptr: UnsafeMutablePointer<CChar>?) {
    if let p = ptr {
        free(p)
    }
}

@_cdecl("vox_parakeet_evict")
public func vox_parakeet_evict() {
    let semaphore = DispatchSemaphore(value: 0)
    Task.detached {
        await ModelsCache.shared.evict()
        semaphore.signal()
    }
    semaphore.wait()
}
