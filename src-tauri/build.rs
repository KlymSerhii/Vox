fn main() {
    // On macOS, AXIsProcessTrusted lives in ApplicationServices.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
    }
    tauri_build::build()
}
