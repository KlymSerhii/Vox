<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="88" alt="Vox" />
  <h1>Vox</h1>
  <p><b>On-device dictation for macOS and Windows. Hit a hotkey, talk, paste.</b></p>

  <p>
    <a href="https://github.com/KlymSerhii/vox-releases/releases/latest"><img alt="Download for macOS and Windows" src="https://img.shields.io/badge/%E2%AC%87_download-macOS_%7C_Windows-2ea44f"></a>
    <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
    <img alt="Platform: macOS and Windows" src="https://img.shields.io/badge/platform-macOS_%7C_Windows-lightgrey">
    <a href="https://rizenhq.com"><img alt="Part of Rizen" src="https://img.shields.io/badge/part_of-Rizen-7c3aed"></a>
  </p>

  <p><a href="https://vox.rizenhq.com">vox.rizenhq.com</a></p>
</div>

> **Free for everyone, including at work.** Vox is open source under the [MIT License](LICENSE). Use it yourself, roll it out to your whole team, change it, share it — no license fees, no sign-up. More under [Licensing](#licensing).

<!-- TODO: drop a short looping demo GIF here — it's worth more than any paragraph.
     A good one: hit ⌘⌥., the pill starts listening with a live waveform, you mumble
     "um, send john the report by friday", let go, and the clean text lands on the
     clipboard. ~8 seconds, looped. Save it as docs/assets/demo.gif and swap in:
     ![Vox demo](docs/assets/demo.gif) -->

<p align="center"><i>🎬 Demo GIF coming soon.</i></p>

## What it does

Press a key, say what you're thinking, and a second later the cleaned-up text is sitting on your clipboard. Paste it wherever you were headed — Slack, your editor, an email, some form on a website. That's the whole loop, and most of the time you'll forget there was software in the middle.

Behind it are two speech engines (Whisper and NVIDIA's Parakeet) and two ways to tidy up what you said (Apple Intelligence, or a Gemma model running locally). You don't have to think about any of that — onboarding picks sensible defaults for your machine, and you can change your mind later in Settings.

The thing we actually care about: **none of this leaves your computer.** No servers, no API keys, no account, no analytics. Your audio sits in memory just long enough to be transcribed and is then thrown away; it never touches the disk. The only time Vox reaches for the network is to download a model or check for an update.

## Highlights

- Talk-to-paste in about a second, triggered by `⌘⌥.` — which, handily, needs no Accessibility permission. Don't feel like holding a key down? There's a hands-free toggle and a "Hey Vox" wake word.
- **VoxNotch**, a tiny to-do list that lives in your MacBook's notch and takes orders by voice. More on that below.
- Eight built-in Voice Modes (email, chat, notes, code comments, a summarizer, a stream-of-thought rewriter, and a couple more), plus your own if the built-ins don't fit. Vox can suggest one based on whatever app you're in.
- A personal dictionary, so your names, acronyms, and odd bits of jargon come out spelled the way you meant them.
- Lives in the menu bar on a Mac and the system tray on Windows. Same core on both.

## VoxNotch

On a MacBook, VoxNotch turns the black bar around your camera into a small, always-there to-do list. Most of the time it's just a slim nub you'd never notice. Hover over it (or click, if you'd rather) and it drops down to show your top few tasks. Tick a box to finish one, drag to reorder, type to add a new one.

The good part is editing it out loud. Press the notch shortcut — `⌘⌥L` by default — and then just talk: *"remind me to call the dentist,"* *"knock out the deploy and write the changelog,"* *"done with the invoice."* Vox works out what you meant and updates the list instead of copying anything to your clipboard. There are no magic phrases to memorize; it's reading your intent. If something you say would delete or clear tasks, you can have it check with you first.

It's macOS-only for now, since it leans on the physical notch — though it works fine on Macs without one, just hanging from the top edge of the screen.

<!-- TODO: showcase VoxNotch here. A clip of the notch expanding on hover, then a
     voice edit ("add 'review the PR', mark the deploy done") landing in the list,
     reads better than words. Save as docs/assets/voxnotch.gif and swap in:
     ![VoxNotch](docs/assets/voxnotch.gif) -->

<p align="center"><i>🎬 VoxNotch clip coming soon.</i></p>

## Get it

The quick way is to grab a build from **[Releases](https://github.com/KlymSerhii/vox-releases/releases/latest)**. You only need the build-from-source steps further down if you want to tinker.

**macOS** — download the `.dmg`, open it, drag Vox into Applications. You'll need Apple Silicon. The first launch runs a short setup that asks for the microphone and downloads whatever models you picked.

**Windows** — download the `-setup.exe` and run it. Windows 10 or 11 on x86_64 (it also runs on Windows-on-ARM through emulation). If you have a DirectX 12 GPU it speeds up the Parakeet engine; if not, Vox quietly falls back to the CPU.

A couple of things worth knowing up front:

| | macOS | Windows |
| --- | --- | --- |
| Chip | Apple Silicon | x86_64 (ARM via emulation) |
| Speech | Whisper or Parakeet (Neural Engine) | Whisper or Parakeet (DirectML / CPU) |
| Cleanup | Apple Intelligence (macOS 26+) or local Gemma | Local Gemma |

Bigger, higher-quality models like more RAM (think 12 GB and up); Vox recommends a size that fits your machine.

## First run

1. Install Vox and open it. It lives in the menu bar (Mac) or system tray (Windows) — there's no window in the Dock or taskbar.
2. Say yes to the microphone when setup asks, and let it pull down your models.
3. Click into any text field, press `⌘⌥.` (or your hotkey on Windows), and talk.
4. Let go. Vox transcribes, cleans it up, and copies the result.
5. Paste with `⌘V` / `Ctrl+V`. If you'd rather skip that step, turn on auto-paste in Settings.

## How it works

```
key down → listening (live waveform) → key up
   → transcribe   (Whisper or Parakeet)
   → polish       (Apple Intelligence or local Gemma, using your active Voice Mode)
   → copied, ready to paste
```

Speech goes through Whisper (in-process, on every platform, and the dependable fallback) or NVIDIA Parakeet, which is the better pick where it's available — the Neural Engine on a Mac, DirectML on Windows, and 25 European languages either way. Cleanup is handled by Apple Intelligence on macOS 26+ or by a local Gemma model that Vox runs for you in the background.

One design choice worth calling out: if cleanup fails for any reason, you still get the raw transcript on your clipboard. You never lose what you said.

## Privacy

The whole point of Vox is that your voice stays on your machine, so to be specific about it:

- No telemetry, no analytics, no account, no API keys.
- Recorded audio lives in RAM and is freed right after transcription. It is never written to disk.
- Transcripts only exist for the session; the "recent" list in the tray is in memory and gone when you quit.
- Every bit of inference runs locally — Neural Engine, Metal, Core ML, DirectML, or CPU, depending on your setup.
- The only network traffic is fetching models and the `llama-server` binary, plus signed update checks.

## Build from source

Vox is a [Tauri 2](https://tauri.app) app: a Rust core with a SolidJS + Tailwind front end.

```bash
npm install
npm run tauri dev
```

That first compile takes a while — it's building the native speech engines. Before sending a change, the four commands we lean on are:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml --lib
npx tsc --noEmit
npx vite build
```

The per-OS toolchain notes (Swift on macOS, MSVC plus `LIBCLANG_PATH` on Windows) and the flags for building without Xcode live in [CONTRIBUTING.md](CONTRIBUTING.md).

## Part of the Rizen ecosystem

Vox comes from [Rizen](https://rizenhq.com) (Resonance Logic, LLC) — *the career operating system*, built around three ideas: **Get Hired. Get Productive. Get Promoted.**

Vox is the **Get Productive** piece: workplace dictation that runs on your own device. It's the first Rizen product out the door, with more on the way (Mind, Meet, Ledger, and others). The pitch for the whole suite is turning everyday work into raises, promotions, and better offers.

Have a look at the rest at [rizenhq.com](https://rizenhq.com).

## Licensing

Vox is **open source under the [MIT License](LICENSE)** and free for everyone. In plain terms:

- Use it for yourself, for freelance or client work, or at your job. Free.
- Companies can put it on every machine on the team. Also free, with no commercial license to buy.
- Read the code, change it, fork it, and share your version.
- The one ask: keep the copyright and license notice in any copies you pass on.

Vox comes as is, without warranty; the [LICENSE](LICENSE) file has the exact wording. If Vox is useful to you and you'd like to support it, there's a [donation page](https://vox.rizenhq.com/donation).

## Known limitations

A few honest rough edges:

- On macOS, Apple Intelligence sometimes refuses certain content. When that happens Vox just hands you the raw transcript; switch to Gemma in Settings if it bothers you.
- On Windows, cleanup is Gemma only — there's no Apple Intelligence equivalent over there.
- Windows builds are x86_64 for now. They run on Windows-on-ARM through emulation, but there's no native ARM64 build yet.
- Parakeet doesn't use your custom dictionary, and a single very long dictation (past four or five minutes) can get cut off. For dictionary-heavy or long sessions, Whisper is the safer bet.

## Contributing

Issues and pull requests are welcome. If you're planning something bigger than a small fix, open an issue first so we can talk it through before you write a lot of code.

- [CONTRIBUTING.md](CONTRIBUTING.md) — getting set up and the checks to run.
- [SECURITY.md](SECURITY.md) — found a vulnerability? Report it privately, not in a public issue.
- [Code of Conduct](CODE_OF_CONDUCT.md) — the short version: be decent.

## Thanks

Vox wouldn't exist without a lot of other people's work: [whisper.cpp](https://github.com/ggerganov/whisper.cpp) and [whisper-rs](https://github.com/tazz4843/whisper-rs), FluidAudio, NVIDIA's Parakeet (and the [ONNX build](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) we use on Windows), [llama.cpp](https://github.com/ggml-org/llama.cpp), Google's [Gemma](https://ai.google.dev/gemma), [Tauri](https://tauri.app), and [SolidJS](https://www.solidjs.com). The typeface is [Fraunces](https://github.com/undercasetype/Fraunces). Full credits in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

---

<div align="center">
  <sub>Vox is a <a href="https://rizenhq.com">Rizen</a> product by Resonance Logic, LLC. <i>Speak. Map. Rise.</i></sub>
</div>
