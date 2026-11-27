//! Output-file naming: yt-dlp's output template syntax (the parts that make sense in a file
//! name) and Windows-safe filename sanitization (same replacement characters as
//! yt-dlp's `--windows-filenames`, so both download paths produce the same names).
//!
//! A field is `%(name)s` with a name from [`field_value`]. Like in yt-dlp it can list
//! alternatives (`%(artist,uploader)s`: the first one the video has), a replacement used when
//! it has one (`%(album&on an album)s`, `{}` is the value), a default used when it has none
//! (`%(artist|Unknown)s`), a date format (`%(upload_date>%Y-%m-%d)s`: `%Y %y %m %d`), a width
//! for numbers (`%(playlist_index)03d`) and a cut (`%(title).50s`). `%%` is a `%`.

use std::path::PathBuf;

use crate::model::VideoInfo;

const MAX_COMPONENT_CHARS: usize = 180;

/// Replace characters that are invalid in file names on Windows (and `/` everywhere).
pub fn sanitize_component(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '"' => '＂',
            ':' => '：',
            '?' => '？',
            '*' => '＊',
            '|' => '｜',
            '<' => '＜',
            '>' => '＞',
            '/' => '⧸',
            '\\' => '⧹',
            c if (c as u32) < 0x20 || c == '\u{7f}' => ' ',
            c => c,
        })
        .collect();
    // Collapse whitespace runs produced by control characters.
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    let mut out = out.trim().trim_end_matches(['.', ' ']).to_string();
    if out.chars().count() > MAX_COMPONENT_CHARS {
        out = out
            .chars()
            .take(MAX_COMPONENT_CHARS)
            .collect::<String>()
            .trim_end_matches(['.', ' '])
            .to_string();
    }
    let upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "_".into();
    }
    out
}

/// The value of a template field (yt-dlp's `-J` names) for this video; `None` when the video
/// has none (or an empty one), or when the field isn't one Prism knows.
pub fn field_value(info: &VideoInfo, name: &str, ext: &str) -> Option<String> {
    let value = match name {
        "title" | "fulltitle" => Some(info.title.clone()),
        "id" => Some(info.id.clone()),
        "ext" => Some(ext.to_string()),
        "uploader" => info.uploader.clone(),
        "uploader_id" => info.uploader_id.clone(),
        "channel" => info.channel.clone().or_else(|| info.uploader.clone()),
        "artist" => info.artist.clone(),
        "album" => info.album.clone(),
        "track" => info.track.clone(),
        "creator" => info.creator.clone(),
        "upload_date" => info.upload_date.clone(),
        "playlist_title" | "playlist" => info.playlist_title.clone(),
        // As wide as the last place, like in yt-dlp: 01 … 12.
        "playlist_index" => info.playlist_index.map(|i| {
            let width = info
                .last_playlist_index
                .unwrap_or(0)
                .max(i)
                .to_string()
                .len();
            format!("{i:0width$}")
        }),
        "extractor" => Some(info.extractor.clone()),
        "extractor_key" => info
            .extractor_key
            .clone()
            .or_else(|| Some(info.extractor.clone())),
        "duration" => info
            .duration
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(|d| (d as u64).to_string()),
        _ => None,
    };
    value.filter(|v| !v.trim().is_empty())
}

/// The field names a template uses, alternatives included (`%(artist,uploader)s` uses both).
pub fn template_fields(template: &str) -> Vec<&str> {
    tokens(template)
        .into_iter()
        .filter_map(|t| match t {
            Token::Field(key, _) => Some(Key::parse(key).alternatives),
            Token::Text(_) => None,
        })
        .flatten()
        .map(|(name, _)| name)
        .collect()
}

/// Render a yt-dlp style template (`%(title)s [%(id)s].%(ext)s`; `/` separates
/// sub-directories). Fields the video doesn't have become `NA` like in yt-dlp.
pub fn render_template(template: &str, info: &VideoInfo, ext: &str) -> PathBuf {
    let template = if template.trim().is_empty() {
        crate::model::DEFAULT_TEMPLATE
    } else {
        template
    };
    let mut path = PathBuf::new();
    for comp in template.split(['/', '\\']) {
        let out: String = expand(comp, info, ext)
            .into_iter()
            .map(|part| match part {
                Part::Text(t) => t,
                Part::Value(v) => in_name(&v.unwrap_or_else(|| "NA".into())),
            })
            .collect();
        if out.is_empty() || out == "." || out == ".." {
            continue;
        }
        path.push(sanitize_component(&out));
    }
    if path.as_os_str().is_empty() {
        path.push(format!("{}.{ext}", sanitize_component(&info.id)));
    }
    path
}

/// The name Prism gives a video (without its extension) from the name template in Settings ›
/// Video (#145). Unlike yt-dlp, which writes `NA`, a field the video doesn't have is left out
/// together with what only separated it: `%(artist)s - %(title)s` without an artist is the
/// title, `%(title)s [%(id)s]` without an id drops the brackets, `%(artist)s - %(album)s -
/// %(title)s` without an album keeps one dash. A trailing `.%(ext)s` is left out (the chosen
/// format decides the extension). An empty template, or one that leaves nothing, gives the
/// title. The caller still makes the result safe as a file name (folders in the template
/// aren't created: a value's `/` becomes `⧸`).
pub fn render_name(template: &str, info: &VideoInfo, ext: &str) -> String {
    let tpl = template.trim();
    let tpl = tpl.strip_suffix(".%(ext)s").unwrap_or(tpl);
    if tpl.is_empty() {
        return info.title.clone();
    }
    let mut parts = expand(tpl, info, ext);
    let mut out = String::new();
    // The text after a missing value loses its leading separators.
    let mut strip_next = false;
    for i in 0..parts.len() {
        match std::mem::replace(&mut parts[i], Part::Text(String::new())) {
            Part::Text(t) => {
                out.push_str(if strip_next {
                    t.trim_start_matches(is_separator)
                } else {
                    t.as_str()
                });
                strip_next = false;
            }
            Part::Value(Some(v)) if !v.is_empty() => {
                out.push_str(&in_name(&v));
                strip_next = false;
            }
            Part::Value(_) => {
                // Brackets around nothing go: `[%(id)s]`, `(%(artist)s)`.
                let open = out.trim_end().chars().last();
                if let (Some(open @ ('[' | '(' | '{')), Some(Part::Text(next))) =
                    (open, parts.get_mut(i + 1))
                {
                    let close = match open {
                        '[' => ']',
                        '(' => ')',
                        _ => '}',
                    };
                    if let Some(after) = next.trim_start().strip_prefix(close) {
                        *next = after.to_string();
                        out.truncate(out.trim_end().len() - 1);
                    }
                }
                // The separator before it goes with it; at the start of the name or right
                // after an opening bracket, the one after it.
                let kept = out.trim_end_matches(is_separator).len();
                if kept == 0 {
                    out.clear();
                    strip_next = true;
                } else if kept < out.len() {
                    out.truncate(kept);
                } else if out.ends_with(['[', '(', '{']) {
                    strip_next = true;
                }
            }
        }
    }
    for empty in ["[]", "()", "{}"] {
        out = out.replace(empty, "");
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace());
    if out.is_empty() {
        info.title.clone()
    } else {
        out.to_string()
    }
}

/// Safe stem for temporary stream files (ASCII-ish id).
pub fn temp_stem(info: &VideoInfo) -> String {
    let s: String = info
        .id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if s.is_empty() { "video".into() } else { s }
}

/// Characters that only separate the parts of a name (`Artist - Title`, `01. Title`).
fn is_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '–'
                | '—'
                | '_'
                | '|'
                | '｜'
                | ','
                | ';'
                | ':'
                | '.'
                | '·'
                | '•'
                | '~'
                | '+'
                | '&'
                | '/'
                | '\\'
        )
}

/// Values must not create path separators (same glyphs as yt-dlp).
fn in_name(value: &str) -> String {
    value.replace('/', "⧸").replace('\\', "⧹")
}

enum Token<'a> {
    Text(String),
    /// `%(key)conversion`
    Field(&'a str, Conversion),
}

fn tokens(template: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('%') {
        text.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('%') {
            text.push('%');
            rest = r;
        } else if let Some((key, r)) = after.strip_prefix('(').and_then(|r| r.split_once(')')) {
            let (conversion, len) = Conversion::parse(r);
            if !text.is_empty() {
                out.push(Token::Text(std::mem::take(&mut text)));
            }
            out.push(Token::Field(key, conversion));
            rest = &r[len..];
        } else {
            text.push('%');
            rest = after;
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    out
}

/// One part of an expanded template.
enum Part {
    /// Text of the template itself.
    Text(String),
    /// A field, shown as the template asks; `None` when the video has no value for it.
    Value(Option<String>),
}

fn expand(template: &str, info: &VideoInfo, ext: &str) -> Vec<Part> {
    tokens(template)
        .into_iter()
        .map(|t| match t {
            Token::Text(t) => Part::Text(t),
            Token::Field(key, conversion) => {
                Part::Value(Key::parse(key).value(&conversion, info, ext))
            }
        })
        .collect()
}

/// What is inside `%( )`, in yt-dlp's order: `name>date format,other name&replacement|default`.
struct Key<'a> {
    /// Field names, each with its date format.
    alternatives: Vec<(&'a str, Option<&'a str>)>,
    replacement: Option<&'a str>,
    default: Option<&'a str>,
}

impl<'a> Key<'a> {
    fn parse(key: &'a str) -> Self {
        let (key, default) = match key.split_once('|') {
            Some((k, d)) => (k, Some(d)),
            None => (key, None),
        };
        let (key, replacement) = match key.split_once('&') {
            Some((k, r)) => (k, Some(r)),
            None => (key, None),
        };
        let alternatives = key
            .split(',')
            .map(|alt| match alt.split_once('>') {
                Some((name, format)) => (name.trim(), Some(format)),
                None => (alt.trim(), None),
            })
            .filter(|(name, _)| !name.is_empty())
            .collect();
        Key {
            alternatives,
            replacement,
            default,
        }
    }

    fn value(&self, conversion: &Conversion, info: &VideoInfo, ext: &str) -> Option<String> {
        let found = self.alternatives.iter().find_map(|(name, format)| {
            let v = field_value(info, name, ext)?;
            match format {
                Some(f) => format_date(&v, f),
                None => Some(v),
            }
        });
        let shown = match (found, self.replacement) {
            (Some(v), Some(r)) => Some(replace_braces(r, &v)),
            (Some(v), None) => conversion.apply(&v),
            (None, _) => None,
        };
        shown.or_else(|| self.default.map(String::from))
    }
}

/// yt-dlp's `{}` (with or without a format inside) in a replacement becomes the value.
fn replace_braces(replacement: &str, value: &str) -> String {
    let mut out = String::new();
    let mut rest = replacement;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(value);
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

/// A `YYYYMMDD` date (as yt-dlp gives `upload_date`) in a format with `%Y %y %m %d`; other
/// directives stay as they are. `None` for a value that isn't such a date (yt-dlp: `NA`).
fn format_date(value: &str, format: &str) -> Option<String> {
    let d = value.trim();
    if d.len() != 8 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&d[..4]),
            Some('y') => out.push_str(&d[2..4]),
            Some('m') => out.push_str(&d[4..6]),
            Some('d') => out.push_str(&d[6..]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    Some(out)
}

/// The printf-style conversion after `%(key)`: `s`, `03d`, `.50s`, `-10s`, `.2f`.
#[derive(Debug, Default, PartialEq)]
struct Conversion {
    zero: bool,
    left: bool,
    width: usize,
    precision: Option<usize>,
    kind: char,
}

/// Conversion types yt-dlp accepts (the ones Prism doesn't format show the value as it is).
const CONVERSION_TYPES: &str = "diouxXeEfFgGcrsaBjhlqDSU";

impl Conversion {
    /// The conversion at the start of `s` and its length; a plain `s` of length 0 when no
    /// conversion type follows (the text stays as it is).
    fn parse(s: &str) -> (Conversion, usize) {
        let mut c = Conversion::default();
        let mut i = 0;
        let b = s.as_bytes();
        while let Some(&f) = b.get(i).filter(|f| b"#0-+ ".contains(*f)) {
            c.zero |= f == b'0';
            c.left |= f == b'-';
            i += 1;
        }
        let digits =
            |from: usize| from + b[from..].iter().take_while(|d| d.is_ascii_digit()).count();
        let end = digits(i);
        c.width = s[i..end].parse().unwrap_or(0);
        i = end;
        if b.get(i) == Some(&b'.') {
            let end = digits(i + 1);
            c.precision = Some(s[i + 1..end].parse().unwrap_or(0));
            i = end;
        }
        match s[i..].chars().next() {
            Some(t) if CONVERSION_TYPES.contains(t) => {
                c.kind = t;
                (c, i + 1)
            }
            _ => (
                Conversion {
                    kind: 's',
                    ..Default::default()
                },
                0,
            ),
        }
    }

    /// The value as this conversion shows it; `None` for a number conversion of a value that
    /// isn't a number (yt-dlp: `NA`).
    fn apply(&self, value: &str) -> Option<String> {
        let number = || value.trim().parse::<f64>().ok().filter(|n| n.is_finite());
        let numeric = matches!(self.kind, 'd' | 'i' | 'u' | 'f' | 'F');
        let s = match self.kind {
            'd' | 'i' | 'u' => (number()?.trunc() as i64).to_string(),
            'f' | 'F' => format!("{:.*}", self.precision.unwrap_or(6), number()?),
            _ => match self.precision {
                Some(p) => value.chars().take(p).collect(),
                None => value.to_string(),
            },
        };
        let fill = self.width.saturating_sub(s.chars().count());
        Some(if fill == 0 {
            s
        } else if self.left {
            format!("{s}{}", " ".repeat(fill))
        } else if self.zero && numeric {
            match s.strip_prefix('-') {
                Some(digits) => format!("-{}{digits}", "0".repeat(fill)),
                None => format!("{}{s}", "0".repeat(fill)),
            }
        } else {
            format!("{}{s}", " ".repeat(fill))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song's page: an artist and an uploader.
    fn song() -> VideoInfo {
        VideoInfo {
            id: "dQw4w9WgXcQ".into(),
            title: "Pop".into(),
            uploader: Some("Label Channel".into()),
            channel: Some("Label Channel".into()),
            artist: Some("Singer".into()),
            album: Some("Hits".into()),
            upload_date: Some("20261124".into()),
            extractor: "youtube".into(),
            extractor_key: Some("Youtube".into()),
            duration: Some(213.6),
            ..Default::default()
        }
    }

    fn name(template: &str, info: &VideoInfo) -> String {
        render_name(template, info, "mp4")
    }

    #[test]
    fn the_default_template_names_as_before() {
        let t = crate::model::DEFAULT_TEMPLATE;
        assert_eq!(name(t, &song()), "Pop [dQw4w9WgXcQ]");
        // No id known (an address without one): no empty brackets.
        let no_id = VideoInfo {
            id: String::new(),
            ..song()
        };
        assert_eq!(name(t, &no_id), "Pop");
        // Whitespace runs collapse and dashes at the ends go, as they always did.
        let messy = VideoInfo {
            title: "-  Intro  -".into(),
            id: String::new(),
            ..song()
        };
        assert_eq!(name(t, &messy), "Intro");
        assert_eq!(name("", &song()), "Pop");
        assert_eq!(name("%(title)s.%(ext)s", &song()), "Pop");
    }

    #[test]
    fn the_artist_when_there_is_one() {
        assert_eq!(
            name("%(artist,uploader)s - %(title)s.%(ext)s", &song()),
            "Singer - Pop"
        );
        assert_eq!(name("%(artist)s - %(title)s", &song()), "Singer - Pop");
        assert_eq!(
            name("%(channel)s - %(album)s - %(track,title)s", &song()),
            "Label Channel - Hits - Pop"
        );
    }

    #[test]
    fn no_artist_falls_back_to_the_uploader() {
        let video = VideoInfo {
            artist: None,
            ..song()
        };
        assert_eq!(
            name("%(artist,uploader)s - %(title)s.%(ext)s", &video),
            "Label Channel - Pop"
        );
        // An empty value counts as none.
        let empty = VideoInfo {
            artist: Some(" ".into()),
            ..song()
        };
        assert_eq!(
            name("%(artist,uploader)s - %(title)s", &empty),
            "Label Channel - Pop"
        );
    }

    #[test]
    fn neither_artist_nor_uploader_leaves_the_title() {
        let video = VideoInfo {
            artist: None,
            uploader: None,
            channel: None,
            album: None,
            ..song()
        };
        // #145: the template of the report.
        assert_eq!(
            name("%(artist,uploader)s - %(title)s.%(ext)s", &video),
            "Pop"
        );
        assert_eq!(name("%(title)s - %(artist)s", &video), "Pop");
        assert_eq!(
            name("%(title)s - %(artist)s [%(id)s]", &video),
            "Pop [dQw4w9WgXcQ]"
        );
        assert_eq!(name("%(title)s (%(artist)s)", &video), "Pop");
        assert_eq!(
            name("%(title)s (%(artist)s, %(upload_date>%Y)s)", &video),
            "Pop (2026)"
        );
        // A missing middle part keeps one separator.
        assert_eq!(
            name("%(uploader)s - %(album)s - %(title)s", &song()),
            "Label Channel - Hits - Pop"
        );
        let no_album = VideoInfo {
            album: None,
            ..song()
        };
        assert_eq!(
            name("%(uploader)s - %(album)s - %(title)s", &no_album),
            "Label Channel - Pop"
        );
        assert_eq!(name("%(artist)s - %(album)s - %(title)s", &video), "Pop");
        // A default instead.
        assert_eq!(
            name("%(artist|Unknown artist)s - %(title)s", &video),
            "Unknown artist - Pop"
        );
        // Unknown fields too.
        assert_eq!(name("%(nope)s - %(title)s", &video), "Pop");
        // Nothing left: the title.
        assert_eq!(name("%(artist)s", &video), "Pop");
    }

    #[test]
    fn playlist_index_is_padded() {
        let entry = VideoInfo {
            playlist_index: Some(7),
            playlist_title: Some("Best of".into()),
            ..song()
        };
        assert_eq!(
            name("%(playlist_index)03d - %(title)s", &entry),
            "007 - Pop"
        );
        assert_eq!(name("%(playlist_index)d. %(title)s", &entry), "7. Pop");
        assert_eq!(
            name(
                "%(playlist_title)s - %(playlist_index)s - %(title)s",
                &entry
            ),
            "Best of - 7 - Pop"
        );
        // `s` is as wide as the last place picked, as in yt-dlp.
        let wide = VideoInfo {
            last_playlist_index: Some(120),
            ..entry.clone()
        };
        assert_eq!(name("%(playlist_index)s - %(title)s", &wide), "007 - Pop");
        assert_eq!(name("%(playlist_index)05d %(title)s", &wide), "00007 Pop");
        // Not in a playlist: no number, no dangling separator.
        assert_eq!(name("%(playlist_index)03d - %(title)s", &song()), "Pop");
        assert_eq!(name("%(playlist_index)s. %(title)s", &song()), "Pop");
    }

    #[test]
    fn other_fields_and_conversions() {
        let v = song();
        assert_eq!(
            name("%(upload_date)s %(title)s [%(extractor_key)s]", &v),
            "20261124 Pop [Youtube]"
        );
        assert_eq!(
            name("%(upload_date>%Y-%m-%d)s %(title)s", &v),
            "2026-11-24 Pop"
        );
        assert_eq!(name("%(title)s (%(duration)ss)", &v), "Pop (213s)");
        assert_eq!(name("%(title).2s", &v), "Po");
        assert_eq!(
            name("%(title)s%(album& - from an album)s", &v),
            "Pop - from an album"
        );
        assert_eq!(name("%(title)s [%(album&{})s]", &v), "Pop [Hits]");
        assert_eq!(name("%(title)s 100%%", &v), "Pop 100%");
        // A value never makes a folder.
        let slash = VideoInfo {
            title: "AC/DC - Live".into(),
            ..v.clone()
        };
        assert_eq!(name("%(title)s", &slash), "AC⧸DC - Live");
        // A number conversion of text: like a missing value.
        assert_eq!(name("%(title)d - %(id)s", &v), "dQw4w9WgXcQ");
        // No conversion type after it: still the value, and the text after it stays.
        assert_eq!(name("%(title)s [%(id)]", &v), "Pop [dQw4w9WgXcQ]");
        assert_eq!(
            template_fields("%(artist,uploader)s - %(title)s [%(upload_date>%Y)s|x]"),
            ["artist", "uploader", "title", "upload_date"]
        );
    }

    #[test]
    fn fields_come_from_the_info_json() {
        let json = r#"{
            "_type": "video", "id": "abc", "title": "Pop", "uploader": "Label",
            "uploader_id": "@label", "channel": "Label", "artists": ["Singer", "Band"],
            "album": "Hits", "track": "Pop (Radio Edit)", "creators": null,
            "upload_date": "20261124", "playlist_index": null,
            "extractor": "youtube", "extractor_key": "Youtube", "formats": []
        }"#;
        let crate::Probe::Video(v) = crate::model::parse_probe_json(json).unwrap() else {
            panic!("expected a video");
        };
        assert_eq!(v.artist.as_deref(), Some("Singer, Band"));
        assert_eq!(v.creator, None);
        assert_eq!(v.playlist_index, None);
        assert_eq!(
            name(
                "%(artist)s - %(track)s [%(album)s, %(uploader_id)s, %(extractor_key)s]",
                &v
            ),
            "Singer, Band - Pop (Radio Edit) [Hits, @label, Youtube]"
        );
        // The probe sent to the window stays as it was without them.
        let plain = serde_json::to_string(&song()).unwrap();
        assert!(plain.contains(r#""artist":"Singer""#));
        assert!(!plain.contains("playlistIndex") && !plain.contains("track"));
    }

    #[test]
    fn ytdlp_templates_write_na() {
        let v = VideoInfo {
            artist: None,
            ..song()
        };
        assert_eq!(
            render_template("%(artist)s - %(title)s.%(ext)s", &v, "mp4"),
            PathBuf::from("NA - Pop.mp4")
        );
        assert_eq!(
            render_template("%(artist,uploader)s - %(title)s.%(ext)s", &v, "mp4"),
            PathBuf::from("Label Channel - Pop.mp4")
        );
        assert_eq!(
            render_template("%(playlist_index|0)03d %(title)s.%(ext)s", &v, "mp4"),
            PathBuf::from("0 Pop.mp4")
        );
        assert_eq!(
            render_template("%(title)s [%(id)s].%(ext)s", &v, "mkv"),
            PathBuf::from("Pop [dQw4w9WgXcQ].mkv")
        );
    }
}
