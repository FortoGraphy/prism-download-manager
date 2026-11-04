fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let ext_chromium = manifest_dir.join("../../extension/build/chromium");
    let ext_firefox = manifest_dir.join("../../extension/build/firefox");
    if !ext_chromium.exists() {
        let _ = std::fs::create_dir_all(&ext_chromium);
    }
    if !ext_firefox.exists() {
        let _ = std::fs::create_dir_all(&ext_firefox);
    }
    tauri_build::build()
}
