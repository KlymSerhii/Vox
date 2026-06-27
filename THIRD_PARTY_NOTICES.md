# Third-Party Notices

Vox is built with, bundles, and downloads at runtime a number of third-party
components. This file collects their attributions and licenses. It is provided
for convenience and is not exhaustive of every transitive dependency — see
`package-lock.json` and `src-tauri/Cargo.lock` for the complete dependency
trees, and each upstream project for its authoritative license text.

## Bundled in this repository

| Component | Used for | License |
| --- | --- | --- |
| [Fraunces](https://github.com/undercasetype/Fraunces) | The serif typeface in the UI | SIL Open Font License 1.1 — full text at [`src/assets/fonts/fraunces/OFL.txt`](src/assets/fonts/fraunces/OFL.txt) |

## Core frameworks &amp; libraries

| Component | Used for | License |
| --- | --- | --- |
| [Tauri](https://tauri.app) | App shell / Rust ↔ webview bridge | MIT / Apache-2.0 |
| [SolidJS](https://www.solidjs.com) | Frontend UI | MIT |
| [Tailwind CSS](https://tailwindcss.com) | Styling | MIT |
| [whisper-rs](https://github.com/tazz4843/whisper-rs) / [whisper.cpp](https://github.com/ggerganov/whisper.cpp) | Whisper speech-to-text (in-process) | MIT |
| [FluidAudio](https://github.com/FluidInference/FluidAudio) | Parakeet via Core ML / Apple Neural Engine (macOS) | See upstream |
| [parakeet-rs](https://crates.io/crates/parakeet-rs) + [ort](https://github.com/pykeio/ort) | Parakeet via ONNX Runtime / DirectML (Windows) | Apache-2.0 / MIT (see upstream) |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) (`llama-server`) | Local Gemma polishing (out-of-process) | MIT |
| [minisign-verify](https://crates.io/crates/minisign-verify) | Verifying signed update artifacts | MIT |

## Models downloaded at runtime

Vox downloads model weights on demand; they are **not** redistributed in this
repository. Each is subject to its own upstream license — please review them
before commercial use.

| Model | Source | Notes |
| --- | --- | --- |
| Whisper (GGML weights) | Hugging Face (ggerganov / community) | MIT-licensed code; weights per upstream |
| NVIDIA Parakeet TDT 0.6B v3 | [nvidia/parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3); ONNX build from [istupakov/parakeet-tdt-0.6b-v3-onnx](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) | See the model cards for license terms |
| Google Gemma (GGUF) | [Gemma](https://ai.google.dev/gemma) (bartowski / Unsloth conversions) | Subject to the [Gemma Terms of Use](https://ai.google.dev/gemma/terms) |

## Apple frameworks

On macOS, Vox calls Apple system frameworks (Foundation Models for Apple
Intelligence polishing, Core ML / Metal / AVFoundation) provided by the
operating system under Apple's SDK and platform terms.

---

*If you are an author of a component listed here and would like the attribution
corrected, please reach out at vox@rizenhq.com.*
