use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// Share the runtime drawing code via `include!` so build.rs and the
// running app never disagree about what each state looks like. The
// runtime module uses only `std::f32::consts`, so it compiles in either
// context without the Tauri/AppKit dependencies pulling in.
mod tray_glyphs {
    include!("src/tray_glyphs.rs");
}

fn write_png(path: &Path, rgba: &[u8], w: u32) {
    let file = fs::File::create(path).expect("create tray PNG");
    let buf = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(buf, w, w);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write png header");
    writer.write_image_data(rgba).expect("write png data");
}

fn emit_tray_icons() {
    // Re-run when either the build script or the shared drawing module
    // changes — `include!` doesn't add a dep edge by itself.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/tray_glyphs.rs");

    let dir = Path::new("icons/tray");
    if !dir.exists() {
        fs::create_dir_all(dir).expect("create icons/tray");
    }

    // Designer reference dump only — the runtime renders procedurally
    // and doesn't read these files. For animated states we sample a
    // representative phase: 0.25 catches the recording dot at peak
    // breath (sin = 1, halo just emerging); 0.0 anchors the spinner gap
    // at the right edge of the ring.
    type Draw = Box<dyn Fn(i32) -> Vec<u8>>;
    let states: &[(&str, Draw)] = &[
        ("idle", Box::new(tray_glyphs::idle)),
        ("recording", Box::new(|w| tray_glyphs::recording(w, 0.25))),
        // Designer dump uses the dark-mode ink (white) so the reference
        // PNG matches the most common menu-bar appearance; the runtime
        // re-derives ink from `mac_appearance::is_dark_menu_bar`.
        ("processing", Box::new(|w| tray_glyphs::processing(w, 0.0, [255, 255, 255]))),
        ("error", Box::new(tray_glyphs::error)),
        ("permission", Box::new(tray_glyphs::permission)),
    ];

    for (name, draw) in states {
        write_png(&dir.join(format!("{name}.png")), &draw(16), 16);
        write_png(&dir.join(format!("{name}@2x.png")), &draw(32), 32);
    }
}

/// Compile `swift/AppleFmBridge.swift` into a static library and emit the
/// link directives that pull it into the final binary.
///
/// Why a static lib (not a subprocess or a dylib): Foundation Models
/// brokers the heavy work to a system daemon (modelinferenced) so a
/// model crash can't crash the menu-bar app. That removes the
/// crash-isolation argument that drove `llama-server` out-of-process.
/// In-process gives us sub-millisecond bridge overhead instead of the
/// stdin/stdout JSON-LINES dance, and avoids shipping a second signed
/// binary in the .app bundle.
///
/// Deployment target is macOS 14.0 even though Foundation Models needs
/// macOS 26.0 — the framework gets weakly linked, so the binary still
/// loads on older Macs and the runtime `if #available(macOS 26, *)` in
/// `AppleFmBridge.swift` returns the right "OS too old" code.
#[cfg(target_os = "macos")]
fn build_apple_fm_bridge() -> Result<(), String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|e| format!("CARGO_MANIFEST_DIR: {e}"))?;
    let out_dir = env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?;

    let swift_src = PathBuf::from(&manifest_dir).join("swift/AppleFmBridge.swift");
    if !swift_src.exists() {
        return Err(format!("missing swift source: {}", swift_src.display()));
    }
    println!("cargo:rerun-if-changed={}", swift_src.display());

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let swift_target = match target_arch.as_str() {
        "aarch64" => "arm64-apple-macos14.0",
        "x86_64" => "x86_64-apple-macos14.0",
        other => return Err(format!("unsupported macOS arch: {other}")),
    };

    let lib_path = PathBuf::from(&out_dir).join("libapple_fm_bridge.a");
    let module_path = PathBuf::from(&out_dir).join("AppleFmBridge.swiftmodule");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let opt_flag = if profile == "release" { "-O" } else { "-Onone" };

    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-emit-library",
        "-static",
        "-parse-as-library",
        opt_flag,
        "-target",
        swift_target,
        "-module-name",
        "AppleFmBridge",
        "-emit-module-path",
        &module_path.display().to_string(),
        "-o",
        &lib_path.display().to_string(),
        &swift_src.display().to_string(),
    ]);

    let output = cmd.output().map_err(|e| format!("invoke swiftc: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "swiftc failed (exit {}):\nstderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    println!("cargo:rustc-link-search=native={}", out_dir);
    println!("cargo:rustc-link-lib=static=apple_fm_bridge");

    // Foundation hard-linked. FoundationModels weak-linked because it
    // only exists on macOS 26 — must come through as adjacent linker
    // args, hence the split `-weak_framework` / framework-name pair.
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-arg=-weak_framework");
    println!("cargo:rustc-link-arg=FoundationModels");

    // Swift runtime ships with macOS at /usr/lib/swift since the 5.0
    // ABI-stability cutover, but the linker still needs the rpath so
    // dyld can resolve the embedded references at startup. Linking the
    // concurrency lib explicitly is required for `Task.detached` /
    // `await` plumbing in the bridge.
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    Ok(())
}

/// Build the Parakeet STT bridge: a SwiftPM package at
/// `src-tauri/swift-parakeet/` that pulls in FluidAudio and exports a
/// C ABI consumed from `engines::parakeet_bridge`.
///
/// Why SwiftPM instead of a single-file `swiftc` invocation like
/// AppleFmBridge: FluidAudio is a third-party library with its own
/// transitive C/Cxx wrappers (FastClusterWrapper, MachTaskSelfWrapper).
/// `swift build` is the only path that resolves and compiles those
/// alongside our bridge sources without us re-implementing dependency
/// resolution by hand.
///
/// Failure modes are non-fatal — emitted as `cargo:warning` so a
/// machine without `swift` installed, without network for the SwiftPM
/// dependency fetch, or with a transient compile failure still
/// produces a working Vox binary. The Parakeet engine module's runtime
/// `is_available()` returns false in that case and `Registry::stt`
/// returns a clear error if anyone selects `parakeet-local`.
#[cfg(target_os = "macos")]
fn build_parakeet_bridge() {
    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(p) => p,
        Err(e) => {
            println!("cargo:warning=parakeet bridge: CARGO_MANIFEST_DIR missing ({e}), feature disabled");
            return;
        }
    };

    let pkg_dir = PathBuf::from(&manifest_dir).join("swift-parakeet");
    let pkg_swift = pkg_dir.join("Package.swift");
    let bridge_src = pkg_dir.join("Sources/ParakeetBridge/ParakeetBridge.swift");

    if !pkg_swift.exists() || !bridge_src.exists() {
        println!(
            "cargo:warning=parakeet bridge: missing Swift sources at {}, feature disabled at runtime",
            pkg_dir.display()
        );
        return;
    }

    println!("cargo:rerun-if-changed={}", pkg_swift.display());
    println!("cargo:rerun-if-changed={}", bridge_src.display());

    if Command::new("swift").arg("--version").output().is_err() {
        println!("cargo:warning=parakeet bridge: `swift` CLI not found, feature disabled at runtime");
        return;
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    if target_arch != "aarch64" {
        println!(
            "cargo:warning=parakeet bridge: target arch '{target_arch}' unsupported by FluidAudio (arm64 only), feature disabled at runtime"
        );
        return;
    }

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let swift_config = if profile == "release" { "release" } else { "debug" };

    let output = match Command::new("swift")
        .args([
            "build",
            "-c",
            swift_config,
            "--package-path",
            &pkg_dir.display().to_string(),
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            println!("cargo:warning=parakeet bridge: `swift build` failed to start ({e}), feature disabled at runtime");
            return;
        }
    };

    if !output.status.success() {
        println!(
            "cargo:warning=parakeet bridge: `swift build` failed (exit {}), feature disabled at runtime. stderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // Resolve the bin path SwiftPM produced — `swift build --show-bin-path`
    // emits the absolute directory containing libParakeetBridge.a plus
    // the resolved transitive .a files (FastClusterWrapper et al.).
    let bin_path_out = match Command::new("swift")
        .args([
            "build",
            "-c",
            swift_config,
            "--package-path",
            &pkg_dir.display().to_string(),
            "--show-bin-path",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            println!(
                "cargo:warning=parakeet bridge: `swift build --show-bin-path` failed (exit {}), feature disabled at runtime. stderr:\n{}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            );
            return;
        }
        Err(e) => {
            println!("cargo:warning=parakeet bridge: --show-bin-path failed ({e}), feature disabled at runtime");
            return;
        }
    };
    let bin_path = String::from_utf8_lossy(&bin_path_out.stdout)
        .trim()
        .to_string();
    let bin_dir = Path::new(&bin_path);

    if !bin_dir.join("libParakeetBridge.a").exists() {
        println!(
            "cargo:warning=parakeet bridge: libParakeetBridge.a missing at {}, feature disabled at runtime",
            bin_dir.display()
        );
        return;
    }

    println!("cargo:rustc-link-search=native={}", bin_dir.display());
    // SwiftPM merges every transitive object (FluidAudio, FastCluster,
    // MachTaskSelfWrapper, etc.) into a single libParakeetBridge.a, so
    // there is exactly one archive to link from the Rust side.
    println!("cargo:rustc-link-lib=static=ParakeetBridge");

    // Apple frameworks FluidAudio's ASR pipeline reaches: Foundation +
    // AVFoundation for audio I/O, CoreML for the .mlmodelc inference,
    // Accelerate for the FastClusterWrapper BLAS path, Metal +
    // MetalPerformanceShaders for CoreML's GPU compute units. libc++
    // covers the Cxx 17 wrappers in FluidAudio's Package.swift.
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    println!("cargo:rustc-link-lib=framework=CoreML");
    println!("cargo:rustc-link-lib=framework=Accelerate");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
    println!("cargo:rustc-link-lib=dylib=c++");

    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

/// Explicitly link `libclang_rt.osx.a` so symbols emitted by
/// `__builtin_available()` / `@available()` checks (notably
/// `___isPlatformVersionAtLeast` in whisper-rs's `ggml-metal-device.m`)
/// resolve at link time.
///
/// On developer Macs the static linker normally finds these via
/// `libSystem.tbd`, but the symbol table varies across SDK / Xcode
/// versions, and CI runners with mismatched Xcode + SDK pairings
/// produce `Undefined symbols: ___isPlatformVersionAtLeast`.
/// Linking clang's compiler-rt fallback guarantees resolution.
#[cfg(target_os = "macos")]
fn link_clang_rt_osx() {
    // `clang -print-resource-dir` points at e.g.
    //   /Applications/Xcode_*.app/Contents/Developer/Toolchains/
    //     XcodeDefault.xctoolchain/usr/lib/clang/<ver>
    // — the lib/darwin/ subdirectory under it is where clang_rt lives.
    let output = match Command::new("clang").arg("-print-resource-dir").output() {
        Ok(o) if o.status.success() => o,
        _ => {
            println!(
                "cargo:warning=link_clang_rt_osx: `clang -print-resource-dir` failed; \
                 macOS availability symbols may be unresolved at link time"
            );
            return;
        }
    };
    let rdir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if rdir.is_empty() {
        return;
    }
    let lib_darwin = PathBuf::from(&rdir).join("lib").join("darwin");
    if !lib_darwin.exists() {
        println!(
            "cargo:warning=link_clang_rt_osx: {} not found; \
             skipping clang_rt.osx link",
            lib_darwin.display()
        );
        return;
    }
    println!("cargo:rustc-link-search=native={}", lib_darwin.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}

fn main() {
    // On macOS, AXIsProcessTrusted lives in ApplicationServices.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        link_clang_rt_osx();
    }

    #[cfg(target_os = "macos")]
    {
        // Only build the Swift bridge when the apple-fm feature is on.
        // CI without Xcode SDK can opt out via
        // `--no-default-features --features local-engines`.
        if env::var("CARGO_FEATURE_APPLE_FM").is_ok() {
            if let Err(e) = build_apple_fm_bridge() {
                // Hard-fail rather than silently producing a binary that
                // links against a missing static lib. The error message
                // tells the user how to opt out.
                panic!(
                    "apple-fm bridge build failed:\n  {e}\n\nIf you don't have Xcode/Swift, build without it:\n  cargo build --no-default-features --features local-engines"
                );
            }
        }
        if env::var("CARGO_FEATURE_PARAKEET").is_ok() {
            // Soft-fail (warn-only) rather than panic — Parakeet is a
            // secondary STT engine and a missing/broken Swift toolchain
            // shouldn't block the rest of the app.
            build_parakeet_bridge();
        }
    }

    emit_tray_icons();
    tauri_build::build()
}
