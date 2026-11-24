//! File-name detection (Content-Disposition, URL, MIME type), sanitization and conflict-free
//! path selection.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use url::Url;

/// Maximum file-name length in bytes (well below the 255 limit, leaving room for
/// ` (123)` and the part suffix).
const MAX_NAME_BYTES: usize = 200;

/// Fallback when nothing better is known.
pub const DEFAULT_NAME: &str = "download";

/// Extracts the file name from a `Content-Disposition` header value. `filename*` (RFC 5987/6266,
/// `UTF-8''…` or `ISO-8859-1''…`) wins over `filename`. Returns the raw (unsanitized) name.
pub fn parse_content_disposition(value: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;
    for (key, val) in disposition_params(value) {
        match key.as_str() {
            "filename*" => {
                if let Some(v) = decode_ext_value(&val) {
                    extended = Some(v);
                }
            }
            "filename" => {
                // Some servers send raw percent-encoded UTF-8 in the plain parameter.
                let v = if val.contains('%') {
                    percent_decode_str(&val)
                        .decode_utf8()
                        .map(|c| c.into_owned())
                        .unwrap_or(val)
                } else {
                    val
                };
                plain = Some(v);
            }
            _ => {}
        }
    }
    extended.or(plain).filter(|s| !s.trim().is_empty())
}

/// Splits `type; k=v; k="quoted"` into lower-case keys and unquoted values.
fn disposition_params(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes: Vec<char> = value.chars().collect();
    let mut i = 0;
    // Skip the disposition type.
    while i < bytes.len() && bytes[i] != ';' {
        i += 1;
    }
    while i < bytes.len() {
        i += 1; // skip ';'
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        let ks = i;
        while i < bytes.len() && bytes[i] != '=' && bytes[i] != ';' {
            i += 1;
        }
        let key: String = bytes[ks..i]
            .iter()
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();
        if i >= bytes.len() || bytes[i] == ';' {
            continue;
        }
        i += 1; // '='
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == '"' {
            i += 1;
            while i < bytes.len() && bytes[i] != '"' {
                if bytes[i] == '\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                val.push(bytes[i]);
                i += 1;
            }
            // Skip to the next ';'.
            while i < bytes.len() && bytes[i] != ';' {
                i += 1;
            }
        } else {
            while i < bytes.len() && bytes[i] != ';' {
                val.push(bytes[i]);
                i += 1;
            }
            val = val.trim().to_string();
        }
        if !key.is_empty() {
            out.push((key, val));
        }
    }
    out
}

/// Decodes an RFC 5987 `ext-value`: `charset'lang'pct-encoded`.
fn decode_ext_value(v: &str) -> Option<String> {
    let mut parts = v.splitn(3, '\'');
    let charset = parts.next()?.trim().to_ascii_lowercase();
    let _lang = parts.next()?;
    let enc = parts.next()?;
    let raw: Vec<u8> = percent_decode_str(enc).collect();
    match charset.as_str() {
        "utf-8" | "utf8" | "" => String::from_utf8(raw).ok(),
        // ISO-8859-1 maps 1:1 to the first 256 code points.
        "iso-8859-1" | "latin1" => Some(raw.iter().map(|&b| b as char).collect()),
        _ => None,
    }
}

/// The last non-empty path segment of `url`, percent-decoded.
pub fn name_from_url(url: &Url) -> Option<String> {
    let seg = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if decoded.trim().is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// Preferred file extension for a MIME type (without the dot), if it is meaningful.
pub fn extension_for_mime(mime: &str) -> Option<String> {
    let essence = mime.split(';').next()?.trim().to_ascii_lowercase();
    let preferred = match essence.as_str() {
        "" | "application/octet-stream" | "binary/octet-stream" | "application/unknown" => {
            return None;
        }
        "text/html" => "html",
        "text/plain" => "txt",
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        "video/mp4" => "mp4",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "video/mp2t" => "ts",
        "application/x-mpegurl" | "application/vnd.apple.mpegurl" => "m3u8",
        "application/dash+xml" => "mpd",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/x-7z-compressed" => "7z",
        "application/vnd.rar" | "application/x-rar-compressed" => "rar",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/pdf" => "pdf",
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => "exe",
        "application/x-msi" => "msi",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        other => {
            return mime_guess::get_mime_extensions_str(other)
                .and_then(|e| e.first())
                .map(|e| e.to_string());
        }
    };
    Some(preferred.to_string())
}

/// Derives the file name for a download: Content-Disposition → URL path → `download`; adds an
/// extension from the MIME type when the name has none. The result is sanitized.
pub fn derive_file_name(
    content_disposition: Option<&str>,
    url: &Url,
    mime: Option<&str>,
) -> String {
    let raw = content_disposition
        .and_then(parse_content_disposition)
        .or_else(|| name_from_url(url))
        .unwrap_or_else(|| DEFAULT_NAME.to_string());
    let mut name = sanitize(&raw);
    if Path::new(&name).extension().is_none()
        && let Some(ext) = mime.and_then(extension_for_mime)
    {
        name = sanitize(&format!("{name}.{ext}"));
    }
    name
}

/// `name` with the MIME type's extension when it has none (`photo` + `image/webp` →
/// `photo.webp`, #121); `None` when it keeps its name (it has an extension, or the type says
/// nothing, like `application/octet-stream`).
pub fn with_mime_extension(name: &str, mime: Option<&str>) -> Option<String> {
    if name.is_empty() || Path::new(name).extension().is_some() {
        return None;
    }
    let ext = mime.and_then(extension_for_mime)?;
    Some(sanitize(&format!("{name}.{ext}")))
}

/// A UTF-8 name that was read one byte per character on its way here (as Latin-1, which is how
/// Firefox hands over response headers) becomes readable again: `à¸­` → `อ` (#91). Only a name
/// made of such characters that forms valid UTF-8 changes; `café` or `naïve` stay as they are.
pub fn repair_mojibake(name: &str) -> std::borrow::Cow<'_, str> {
    if name.is_ascii() || name.chars().any(|c| c as u32 > 0xFF) {
        return name.into();
    }
    let bytes: Vec<u8> = name.chars().map(|c| c as u32 as u8).collect();
    match String::from_utf8(bytes) {
        Ok(fixed) => fixed.into(),
        Err(_) => name.into(),
    }
}

/// Makes `name` safe on Windows, Linux and macOS: strips directory components, replaces
/// reserved characters and control characters with `_`, trims spaces and trailing dots,
/// escapes reserved device names (`CON`, `COM1`, …) and limits the length (keeping the
/// extension). A UTF-8 name read as Latin-1 is repaired first ([`repair_mojibake`]).
pub fn sanitize(name: &str) -> String {
    let name = repair_mojibake(name);
    // Keep only the last path component (servers sometimes send paths).
    let base = name.rsplit(['/', '\\']).next().unwrap_or(&name);
    let mut s: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        return DEFAULT_NAME.to_string();
    }
    let stem_upper = s
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let is_reserved = RESERVED.contains(&stem_upper.as_str())
        || ((stem_upper.starts_with("COM") || stem_upper.starts_with("LPT"))
            && stem_upper.len() == 4
            && stem_upper.as_bytes()[3].is_ascii_digit());
    if is_reserved {
        s.insert(0, '_');
    }
    truncate_keep_extension(&s, MAX_NAME_BYTES)
}

fn truncate_keep_extension(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let (stem, ext) = match s.rfind('.') {
        Some(i) if i > 0 && s.len() - i <= 16 => (&s[..i], &s[i..]),
        _ => (s, ""),
    };
    let mut cut = max.saturating_sub(ext.len());
    while cut > 0 && !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", stem[..cut].trim_end_matches(['.', ' ']), ext)
}

/// `name (n).ext` for `n >= 1`.
pub fn numbered_name(name: &str, n: u32) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    format!("{stem} ({n}){ext}")
}

/// Path of the in-progress file for a final path.
pub fn part_path_for(final_path: &Path, suffix: &str) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Picks `dir/name`, or `dir/name (1).ext`, … such that neither the final path nor its part
/// file exists. With `overwrite`, the plain name is used when only the final file exists (it
/// is replaced on completion). Returns `None` when 10 000 candidates are taken.
pub fn unique_path(dir: &Path, name: &str, suffix: &str, overwrite: bool) -> Option<PathBuf> {
    for n in 0..10_000u32 {
        let candidate = if n == 0 {
            name.to_string()
        } else {
            numbered_name(name, n)
        };
        let p = dir.join(&candidate);
        let part = part_path_for(&p, suffix);
        if part.exists() {
            continue;
        }
        if overwrite || !p.exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(
            parse_content_disposition("attachment; filename=\"a b.zip\"").unwrap(),
            "a b.zip"
        );
        assert_eq!(
            parse_content_disposition("attachment; filename=plain.txt").unwrap(),
            "plain.txt"
        );
        assert_eq!(
            parse_content_disposition(
                "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E2%82%AC%20rates.txt"
            )
            .unwrap(),
            "€ rates.txt"
        );
        assert_eq!(
            parse_content_disposition("attachment; filename*=utf-8'en'na%C3%AFve.pdf").unwrap(),
            "naïve.pdf"
        );
        assert_eq!(
            parse_content_disposition("attachment; filename*=iso-8859-1''caf%E9.txt").unwrap(),
            "café.txt"
        );
        assert_eq!(
            parse_content_disposition(r#"inline; filename="quo\"te.txt""#).unwrap(),
            "quo\"te.txt"
        );
        assert_eq!(
            parse_content_disposition("attachment; filename=\"x;y.txt\"; size=3").unwrap(),
            "x;y.txt"
        );
        assert!(parse_content_disposition("attachment").is_none());
        assert!(parse_content_disposition("attachment; filename=\"\"").is_none());
    }

    #[test]
    fn url_and_mime() {
        let u = Url::parse("https://h/dir/my%20file%E2%82%AC.tar.gz?x=1").unwrap();
        assert_eq!(name_from_url(&u).unwrap(), "my file€.tar.gz");
        assert_eq!(
            name_from_url(&Url::parse("https://h/a/b/").unwrap()).unwrap(),
            "b"
        );
        assert!(name_from_url(&Url::parse("https://h/").unwrap()).is_none());
        assert_eq!(extension_for_mime("image/jpeg").unwrap(), "jpg");
        assert_eq!(
            extension_for_mime("application/pdf; charset=x").unwrap(),
            "pdf"
        );
        assert!(extension_for_mime("application/octet-stream").is_none());
        // #121: a name without an extension gets the type's; one with an extension keeps it.
        assert_eq!(
            with_mime_extension("photo", Some("image/webp")).as_deref(),
            Some("photo.webp")
        );
        assert_eq!(with_mime_extension("photo.png", Some("image/webp")), None);
        assert_eq!(
            with_mime_extension("photo", Some("application/octet-stream")),
            None
        );
        assert_eq!(with_mime_extension("photo", None), None);
        assert_eq!(with_mime_extension("", Some("image/webp")), None);
        assert_eq!(
            derive_file_name(
                None,
                &Url::parse("https://h/").unwrap(),
                Some("application/zip")
            ),
            "download.zip"
        );
        assert_eq!(
            derive_file_name(
                None,
                &Url::parse("https://h/get").unwrap(),
                Some("video/mp4")
            ),
            "get.mp4"
        );
        assert_eq!(
            derive_file_name(
                Some("attachment; filename=r.bin"),
                &Url::parse("https://h/x.zip").unwrap(),
                None
            ),
            "r.bin"
        );
    }

    #[test]
    fn sanitization() {
        assert_eq!(sanitize("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(sanitize("name. . ."), "name");
        assert_eq!(sanitize("  spaced  "), "spaced");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("COM1.log"), "_COM1.log");
        assert_eq!(sanitize("LPT9"), "_LPT9");
        assert_eq!(sanitize("COMPUTER.txt"), "COMPUTER.txt");
        assert_eq!(sanitize("tab\there"), "tab_here");
        assert_eq!(sanitize(""), "download");
        assert_eq!(sanitize("..."), "download");
        let long = format!("{}.zip", "é".repeat(300));
        let s = sanitize(&long);
        assert!(s.len() <= MAX_NAME_BYTES && s.ends_with(".zip"), "{s}");
    }

    #[test]
    fn numbering_and_unique_paths() {
        assert_eq!(numbered_name("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(numbered_name("noext", 1), "noext (1)");
        let dir = tempfile::tempdir().unwrap();
        let p = unique_path(dir.path(), "f.bin", ".prismpart", false).unwrap();
        assert_eq!(p, dir.path().join("f.bin"));
        std::fs::write(dir.path().join("f.bin"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "f.bin", ".prismpart", false).unwrap(),
            dir.path().join("f (1).bin")
        );
        assert_eq!(
            unique_path(dir.path(), "f.bin", ".prismpart", true).unwrap(),
            dir.path().join("f.bin")
        );
        std::fs::write(dir.path().join("f (1).bin.prismpart"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "f.bin", ".prismpart", false).unwrap(),
            dir.path().join("f (2).bin")
        );
    }

    #[test]
    fn utf8_read_as_latin1_is_repaired() {
        // A Thai name in UTF-8, one byte per character (#91).
        let thai = "อัพเดท.zip";
        let garbled: String = thai.bytes().map(char::from).collect();
        assert_ne!(garbled, thai);
        assert_eq!(repair_mojibake(&garbled), thai);
        assert_eq!(sanitize(&garbled), thai);
        let cafe: String = "café.txt".bytes().map(char::from).collect();
        assert_eq!(sanitize(&cafe), "café.txt");
        // Names that are fine stay as they are.
        for ok in [
            "café.txt",
            "naïve résumé.pdf",
            "Größe.zip",
            "plain.bin",
            "อัพเดท.zip",
            "数据.7z",
        ] {
            assert_eq!(repair_mojibake(ok), ok);
            assert_eq!(sanitize(ok), ok);
        }
    }
}
