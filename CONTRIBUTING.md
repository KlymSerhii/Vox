# Contributing to Vox

Thanks for wanting to help out. Bug reports, fixes, and new features are all
welcome.

> **Heads up before a big PR:** for anything non-trivial, please
> [open an issue](https://github.com/KlymSerhii/Vox/issues) first so we can
> agree on the approach before you spend time building it.

Vox is [Tauri 2](https://tauri.app): a Rust core (`src-tauri/`) with a SolidJS +
Tailwind 4 frontend (`src/`). It targets **macOS (Apple Silicon)** and
**Windows (x86_64)** from one codebase; OS-specific behaviour is `cfg`-gated in
Rust and branched on `src/lib/platform.ts` in the UI.

## Prerequisites

**Both platforms**

- [Node.js](https://nodejs.org/) (LTS) and npm
- A [Rust toolchain](https://rustup.rs/) (stable)

**macOS**

- Xcode command-line tools, including `swiftc` — `build.rs` compiles two Swift
  bridges (Apple Intelligence + FluidAudio/Parakeet). Builds without `swiftc`
  fall back to Gemma-only (see the feature-flag note below).

**Windows (x86_64)**

- An MSVC developer shell (so `cl.exe` / `link.exe` are on `PATH`)
- `LIBCLANG_PATH` pointing at a 64-bit LLVM (needed by `whisper-rs-sys` bindgen)
- Network egress on first build — the `ort` crate downloads a prebuilt ONNX
  Runtime + DirectML DLL

## Dev loop

```bash
npm install
npm run tauri dev          # first compile is slow (it builds the native engines)
```

For UI-only iteration you can skip the heavy native engines:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features
```

CI / machines without `swiftc` can build the Rust core without the Swift bridges:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features local-engines
```

Logs: `VOX_LOG=debug npm run tauri dev` (defaults to `info`).

## Before you open a PR

Please run the project smoke test and make sure all four pass:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml --lib
npx tsc --noEmit
npx vite build
```

In your PR description, note which **OS(es)** you tested on and — for changes
that touch dictation — which **STT/LLM engine** you used.

## Style &amp; conventions

- Match the surrounding code: its naming, comment density, and idioms.
- Keep user-facing strings OS-neutral — branch on `isMac` / `deviceWord` from
  `src/lib/platform.ts` instead of hard-coding "Mac" / "Apple Silicon".
- Keep the privacy guarantees intact: no audio to disk, no telemetry, no network
  calls beyond model/binary downloads and signed updates.

## Contribution terms

Vox is open source under the [MIT License](LICENSE). By submitting a
contribution, you confirm you have the right to submit it and you agree that
your contribution is licensed under the same MIT License. If you're contributing
on behalf of an employer, make sure you have permission to do so.

## Security

Found a vulnerability? Please **don't** open a public issue — follow
[SECURITY.md](SECURITY.md) to report it privately.
