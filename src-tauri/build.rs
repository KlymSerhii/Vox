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
        ("idle", Box::new(|w| tray_glyphs::idle(w))),
        ("recording", Box::new(|w| tray_glyphs::recording(w, 0.25))),
        ("processing", Box::new(|w| tray_glyphs::processing(w, 0.0))),
        ("error", Box::new(|w| tray_glyphs::error(w))),
        ("permission", Box::new(|w| tray_glyphs::permission(w))),
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

fn main() {
    // On macOS, AXIsProcessTrusted lives in ApplicationServices.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
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
    }

    emit_tray_icons();
    tauri_build::build()
}
