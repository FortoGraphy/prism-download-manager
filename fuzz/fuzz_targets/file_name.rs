//! A server's `Content-Disposition` header and URL, through the file-name derivation. Whatever
//! the server sends, the result must stay a single, valid file name inside the download folder.
#![no_main]

use libfuzzer_sys::fuzz_target;
use prism_engine::Url;
use prism_engine::filename::{derive_file_name, parse_content_disposition, sanitize};

fn check(name: &str) {
    assert!(!name.is_empty(), "empty name");
    assert!(name.len() <= 200, "name too long: {} bytes", name.len());
    assert!(
        !name.contains(['/', '\\']) && !name.chars().any(char::is_control),
        "separator or control character in {name:?}"
    );
    assert!(name != "." && name != "..", "{name:?} is not a file name");
    assert!(
        !name.ends_with(['.', ' ']) && !name.starts_with(char::is_whitespace),
        "{name:?} is changed or refused by Windows"
    );
}

fuzz_target!(|input: (&str, &str)| {
    let (header, path) = input;
    if let Some(raw) = parse_content_disposition(header) {
        check(&sanitize(&raw));
    }
    check(&sanitize(header));
    if let Ok(url) = Url::parse("https://example.com/").and_then(|b| b.join(path)) {
        check(&derive_file_name(Some(header), &url, Some("video/mp4")));
        check(&derive_file_name(None, &url, None));
    }
});
