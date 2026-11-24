//! Windows (MSVC): link the VC runtime statically and the Universal CRT (part of Windows)
//! dynamically, as tauri-build does for the app. Without it the host needs vcruntime140.dll,
//! and the browser can't start it where the Visual C++ Redistributable is missing (#111).
fn main() {
    static_vcruntime::metabuild();
}
