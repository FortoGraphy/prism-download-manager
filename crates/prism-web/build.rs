//! Embeds the Vite build so `prism-web` is a single self-contained binary.
//! The UI is still built from the same `app/src` tree as the desktop app.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist = crate_dir.join("../../app/dist");
    println!("cargo:rerun-if-changed={}", dist.display());

    let mut files = Vec::new();
    if dist.is_dir() {
        walk(&dist, &dist, &mut files);
        files.sort();
    } else {
        println!("cargo:warning=app/dist missing; prism-web will serve assets from PRISM_WEB_DIST");
    }

    let mut out = String::from("pub static ASSETS: &[(&str, &str, &[u8])] = &[\n");
    for (url, path) in files {
        let source = path
            .strip_prefix(&crate_dir)
            .expect("asset inside workspace")
            .to_string_lossy()
            .replace('\\', "/");
        out.push_str(&format!(
            "    (\"{url}\", \"{}\", include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{source}\"))),\n",
            mime(&path)
        ));
    }
    out.push_str("];\n");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("assets.rs"), out).expect("write assets.rs");
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else if path.is_file() && path.extension().is_none_or(|e| e != "map") {
            let relative = path
                .strip_prefix(root)
                .expect("child of root")
                .to_string_lossy()
                .replace('\\', "/");
            let url = if relative == "index.html" {
                "/".to_string()
            } else {
                format!("/{relative}")
            };
            out.push((url, path));
        }
    }
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "wav" => "audio/wav",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}
