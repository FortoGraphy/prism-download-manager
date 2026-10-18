//! Native YouTube format resolution without yt-dlp: POST to YouTube's own "Innertube"
//! player API (`/youtubei/v1/player`) as the page's logged-in browser session, using the
//! `WEB_CREATOR` client, whose format entries carry direct final `url`s — no
//! player-JavaScript signature descrambling, so no yt-dlp + deno spawn.
//!
//! Every doubt declines: [`try_resolve`] returns `None` and the caller falls back to the
//! normal yt-dlp path. That covers: no video id in the URL, playlist requests, no usable
//! session cookies (an anonymous request is untested and never attempted), non-`OK`
//! playability, *any* format carrying `signatureCipher`/`cipher` (the whole response is
//! refused, never a partial list), and any network or parse trouble.
//!
//! Untested-by-capture details, kept conservative: the request omits
//! `visitorData`/`deviceExperimentId`/`rolloutToken` (per-session tokens the extension
//! does not capture) and sends only the documented `SAPISIDHASH <ts>_<hash>` auth scheme
//! (the captured IDM request also carried 1P/3P variants and a `_u` suffix of unknown
//! meaning; neither is part of the documented scheme). If YouTube ever rejects the
//! result, the silent yt-dlp fallback is the designed behavior.
//!
//! `CLIENT_VERSION` tracks a real web-client version and will go stale; a current one is
//! visible in any youtube.com page's `ytcfg` or in yt-dlp's extractor source.
//!
//! The `WEB_CREATOR` fast path and its auth scheme were reverse-engineered from IDM's own
//! captured traffic by Sourabh Kushwah (<uuc110@gmail.com>).

use std::time::Duration;

use serde_json::{Value, json};
use sha2::Digest;

use crate::model::{RequestContext, VideoOptions};

const INNERTUBE_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const ORIGIN: &str = "https://www.youtube.com";
const CLIENT_NAME: &str = "WEB_CREATOR";
const CLIENT_VERSION: &str = "1.20260708.06.00";
/// `X-Youtube-Client-Name` of `WEB_CREATOR`.
const CLIENT_ID: &str = "62";
/// Captured `signatureTimestamp`; for `WEB_CREATOR` no descrambling happens anyway.
const SIGNATURE_TIMESTAMP: i64 = 20739;
/// Fallback only: the request normally uses the browser's User-Agent from the context.
const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// Resolve a YouTube page to a `yt-dlp -J`-shaped JSON string, or `None` to fall back to
/// yt-dlp. Never fails loudly.
pub(crate) async fn try_resolve(
    url: &str,
    ctx: &RequestContext,
    opts: &VideoOptions,
) -> Option<String> {
    if ctx.want_playlist {
        return None;
    }
    let video_id = video_id(url)?;
    let cookies = ctx.cookies.as_deref().filter(|c| !c.is_empty())?;
    if !has_session(cookies) {
        tracing::debug!("youtube native: no SAPISID session cookies, using yt-dlp");
        return None;
    }
    let auth = authorization(cookies, chrono::Utc::now().timestamp());
    let ua = ctx
        .user_agent
        .as_deref()
        .or(opts.user_agent.as_deref())
        .unwrap_or(DEFAULT_UA);

    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(15));
    if let Some(p) = &opts.proxy {
        builder = builder.proxy(reqwest::Proxy::all(p.as_str()).ok()?);
    }
    let client = builder.build().ok()?;

    let body = serde_json::to_string(&request_body(&video_id, ua)).ok()?;
    let response = client
        .post(INNERTUBE_URL)
        .timeout(Duration::from_secs(20))
        .header("User-Agent", ua)
        .header("Accept", "*/*")
        .header("Accept-Language", "en")
        .header("Content-Type", "application/json")
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .header("Origin", ORIGIN)
        .header("X-Origin", ORIGIN)
        .header("X-Youtube-Client-Name", CLIENT_ID)
        .header("X-Youtube-Client-Version", CLIENT_VERSION)
        .header("X-Youtube-Bootstrap-Logged-In", "true")
        .header("X-Goog-AuthUser", "0")
        .body(body)
        .send()
        .await
        .ok()?;
    let status = response.status();
    if !status.is_success() {
        tracing::debug!("youtube native: innertube returned {status}, using yt-dlp");
        return None;
    }
    let text = response.text().await.ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    response_to_info_json(&parsed, &video_id, ua)
}

/// The site's video id where it is in the URL (YouTube watch/shorts/live/embed/youtu.be
/// links). `None` for anything else (playlists, channels, other sites).
fn video_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    match host {
        "youtube.com" | "music.youtube.com" => u
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .or_else(|| {
                let mut seg = u.path_segments()?;
                matches!(seg.next(), Some("shorts" | "live" | "embed"))
                    .then(|| seg.next().map(String::from))?
            }),
        "youtu.be" => u.path_segments()?.next().map(String::from),
        _ => None,
    }
    .filter(|id| !id.is_empty())
}

/// Value of one cookie in a `Cookie` header value (`a=b; c=d`).
fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// A logged-in Google session in the cookie header: `SAPISID` or `__Secure-3PAPISID`
/// is what the authorization hash is computed from.
fn has_session(cookies: &str) -> bool {
    cookie_value(cookies, "SAPISID").is_some() || cookie_value(cookies, "__Secure-3PAPISID").is_some()
}

/// `Authorization: SAPISIDHASH <ts>_<sha1(ts + " " + cookie + " " + origin)>` — Google's
/// documented scheme (YouTube Data API docs / yt-dlp). All present cookie variants share
/// one timestamp; the hash input is `<ts> <cookie value> https://www.youtube.com`.
fn authorization(cookies: &str, ts: i64) -> String {
    [
        ("SAPISIDHASH", "SAPISID"),
        ("SAPISID1PHASH", "__Secure-1PAPISID"),
        ("SAPISID3PHASH", "__Secure-3PAPISID"),
    ]
    .into_iter()
    .filter_map(|(scheme, cookie)| {
        let hash = sapisid_hash(ts, cookie_value(cookies, cookie)?);
        Some(format!("{scheme} {ts}_{hash}"))
    })
    .collect::<Vec<_>>()
    .join(" ")
}

fn sapisid_hash(ts: i64, cookie_value: &str) -> String {
    crate::tools::hex(&sha1::Sha1::digest(
        format!("{ts} {cookie_value} {ORIGIN}").as_bytes(),
    ))
}

/// The Innertube `player` request body (captured shape minus the per-session tokens the
/// extension does not capture: `visitorData`, `deviceExperimentId`, `rolloutToken`).
fn request_body(video_id: &str, ua: &str) -> Value {
    json!({
        "videoId": video_id,
        "context": {
            "client": {
                "userAgent": ua,
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "osName": std::env::consts::OS,
                "osVersion": "10.0",
                "deviceMake": "",
                "deviceModel": "",
                "clientFormFactor": "UNKNOWN_FORM_FACTOR",
                "screenPixelDensity": 1,
                "screenDensityFloat": 1.0,
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
                "hl": "en"
            }
        },
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS",
                "signatureTimestamp": SIGNATURE_TIMESTAMP
            }
        },
        "contentCheckOk": true,
        "racyCheckOk": true
    })
}

/// Validate an Innertube player response and map it to `yt-dlp -J`-shaped JSON that
/// [`crate::model::parse_probe_json`] consumes unchanged. `None` = not usable.
fn response_to_info_json(response: &Value, video_id: &str, ua: &str) -> Option<String> {
    let status = response
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)?;
    if status != "OK" {
        tracing::debug!("youtube native: playabilityStatus {status}, using yt-dlp");
        return None;
    }
    let streaming = response.get("streamingData")?;
    let raw: Vec<&Value> = ["formats", "adaptiveFormats"]
        .into_iter()
        .filter_map(|k| streaming.get(k))
        .filter_map(Value::as_array)
        .flatten()
        .collect();
    if raw.is_empty() {
        return None;
    }
    let mut formats = Vec::with_capacity(raw.len());
    for f in raw {
        // One ciphered entry invalidates the whole response; a partial list is never served.
        if f.get("signatureCipher").is_some() || f.get("cipher").is_some() {
            tracing::debug!("youtube native: signatureCipher present, using yt-dlp");
            return None;
        }
        let url = f.get("url").and_then(Value::as_str)?;
        formats.push(map_format(f, url, ua)?);
    }
    let details = response.get("videoDetails")?;
    let author = details.get("author").and_then(Value::as_str);
    let duration = details
        .get("lengthSeconds")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok());
    let thumbnail = details
        .pointer("/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|t| t.last())
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str);
    let info = json!({
        "id": video_id,
        "title": details.get("title").and_then(Value::as_str),
        "uploader": author,
        "channel": author,
        "duration": duration,
        "thumbnail": thumbnail,
        "webpage_url": format!("https://www.youtube.com/watch?v={video_id}"),
        "extractor": "youtube",
        "formats": formats
    });
    serde_json::to_string(&info).ok()
}

/// One Innertube format entry → the snake_case shape yt-dlp emits (consumed by
/// [`crate::model::RawFormat`]). `None` for an entry without an `itag`.
fn map_format(f: &Value, url: &str, ua: &str) -> Option<Value> {
    let itag = f.get("itag").and_then(Value::as_i64)?;
    let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let (mime_type, ext, codecs) = parse_mime(mime);
    let is_audio = mime_type.as_deref() == Some("audio");
    let bitrate = f
        .get("bitrate")
        .and_then(Value::as_f64)
        .or_else(|| f.get("averageBitrate").and_then(Value::as_f64));
    let tbr = bitrate.map(|b| b / 1000.0);
    let (vcodec, acodec) = match mime_type.as_deref() {
        // Audio-only: the single codec is the audio one.
        Some("audio") => (Some("none".to_string()), codecs.first().cloned()),
        // Progressive carries two codecs; a single codec means video-only.
        Some("video") => (
            codecs.first().cloned(),
            Some(codecs.get(1).cloned().unwrap_or_else(|| "none".to_string())),
        ),
        _ => (None, None),
    };
    let num = |k: &str| f.get(k).and_then(Value::as_f64);
    let filesize = f
        .get("contentLength")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok());
    let asr = if is_audio {
        f.get("audioSampleRate")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
    } else {
        None
    };
    let resolution = match (num("width"), num("height")) {
        (Some(w), Some(h)) => format!("{}x{}", w as i64, h as i64),
        _ if is_audio => "audio only".to_string(),
        _ => String::new(),
    };
    Some(json!({
        "format_id": itag.to_string(),
        "ext": ext,
        "vcodec": vcodec,
        "acodec": acodec,
        "width": num("width"),
        "height": num("height"),
        "fps": num("fps"),
        "tbr": tbr,
        "vbr": if !is_audio { tbr } else { None },
        "abr": if is_audio { tbr } else { None },
        "asr": asr,
        "filesize": filesize,
        "resolution": resolution,
        "format_note": f.get("qualityLabel").and_then(Value::as_str),
        "protocol": "https",
        "url": url,
        "http_headers": { "User-Agent": ua }
    }))
}

/// `"video/mp4; codecs=\"avc1.64001f, mp4a.40.2\""` → (`video`, `mp4`, `[avc1…, mp4a…]`).
fn parse_mime(mime: &str) -> (Option<String>, Option<String>, Vec<String>) {
    let mut segments = mime.split(';');
    let base = segments.next().unwrap_or_default().trim();
    let (mime_type, ext) = base
        .split_once('/')
        .map(|(t, s)| (Some(t.trim().to_string()), Some(s.trim().to_string())))
        .unwrap_or((None, None));
    let codecs = segments
        .find_map(|s| s.trim().strip_prefix("codecs="))
        .map(|c| {
            c.trim_matches('"')
                .split(',')
                .map(|c| c.trim().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (mime_type, ext, codecs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_are_read_from_every_url_shape() {
        for (url, id) in [
            ("https://www.youtube.com/watch?v=jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://youtube.com/watch?t=1&v=jNQXAC9IVRw&list=x", "jNQXAC9IVRw"),
            ("https://m.youtube.com/watch?v=jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://music.youtube.com/watch?v=jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://www.youtube.com/shorts/jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://www.youtube.com/live/jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://www.youtube.com/embed/jNQXAC9IVRw", "jNQXAC9IVRw"),
            ("https://youtu.be/jNQXAC9IVRw?t=1", "jNQXAC9IVRw"),
        ] {
            assert_eq!(video_id(url).as_deref(), Some(id), "{url}");
        }
        // Playlists, channels and other sites are not handled.
        for url in [
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/@channel",
            "https://vimeo.com/12345",
            "https://example.com/watch?v=jNQXAC9IVRw",
        ] {
            assert_eq!(video_id(url), None, "{url}");
        }
    }

    #[test]
    fn a_google_session_gate() {
        assert!(has_session("LOGIN_INFO=x; SAPISID=abc; PREF=y"));
        assert!(has_session("__Secure-3PAPISID=abc"));
        assert!(!has_session("LOGIN_INFO=x; PREF=y"));
        assert!(!has_session(""));
    }

    #[test]
    fn authorization_hashes_the_documented_input() {
        // The captured request's scheme: `<ts>_<sha1(ts + " " + value + " " + origin)>`.
        let expected = sapisid_hash(1_700_000_000, "test-sapisid-value");
        assert_eq!(
            authorization("PREF=1; SAPISID=test-sapisid-value; LOGIN_INFO=x", 1_700_000_000),
            format!("SAPISIDHASH 1700000000_{expected}"),
            "one variant per session cookie present"
        );
        let auth3p = authorization(
            "__Secure-3PAPISID=third-party; __Secure-1PAPISID=first-party",
            1_700_000_000,
        );
        assert!(auth3p.starts_with("SAPISID1PHASH 1700000000_"));
        assert!(auth3p.contains(" SAPISID3PHASH 1700000000_"));
        // The separator chain never leaks a lone scheme.
        assert_eq!(authorization("PREF=1", 1), "");
    }

    #[test]
    fn the_request_impersonates_the_web_creator_client() {
        let body = request_body("jNQXAC9IVRw", "UA");
        assert_eq!(body["videoId"], "jNQXAC9IVRw");
        assert_eq!(body["context"]["client"]["clientName"], CLIENT_NAME);
        // The body's version must match the header's version; both come from one constant.
        assert_eq!(
            body["context"]["client"]["clientVersion"].as_str().unwrap(),
            CLIENT_VERSION
        );
        assert_eq!(body["context"]["client"]["userAgent"], "UA");
        assert_eq!(body["contentCheckOk"], true);
        assert_eq!(body["racyCheckOk"], true);
        assert_eq!(
            body["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"],
            SIGNATURE_TIMESTAMP
        );
    }

    /// A synthetic but captured-shape `OK` response with one progressive, one
    /// video-only and one audio-only format (placeholder data only).
    fn ok_response() -> Value {
        json!({
            "playabilityStatus": {"status": "OK"},
            "videoDetails": {
                "videoId": "jNQXAC9IVRw",
                "title": "Me at the zoo",
                "author": "jawed",
                "lengthSeconds": "19",
                "thumbnail": {"thumbnails": [
                    {"url": "https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg", "width": 480}
                ]}
            },
            "streamingData": {
                "formats": [{
                    "itag": 18,
                    "url": "https://rr1---sn-x.googlevideo.com/videoplayback?itag=18&c=WEB_CREATOR",
                    "mimeType": "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"",
                    "bitrate": 389000,
                    "width": 320, "height": 240, "fps": 25,
                    "qualityLabel": "240p",
                    "contentLength": "955268",
                    "approxDurationMs": "19063"
                }],
                "adaptiveFormats": [
                    {
                        "itag": 247,
                        "url": "https://rr1---sn-x.googlevideo.com/videoplayback?itag=247&c=WEB_CREATOR",
                        "mimeType": "video/webm; codecs=\"vp9\"",
                        "bitrate": 741000,
                        "width": 640, "height": 360, "fps": 25,
                        "qualityLabel": "360p",
                        "contentLength": "1800000"
                    },
                    {
                        "itag": 249,
                        "url": "https://rr1---sn-x.googlevideo.com/videoplayback?itag=249&c=WEB_CREATOR",
                        "mimeType": "audio/webm; codecs=\"opus\"",
                        "bitrate": 59000,
                        "audioQuality": "AUDIO_QUALITY_LOW",
                        "audioSampleRate": "48000",
                        "contentLength": "252182",
                        "approxDurationMs": "19063"
                    }
                ]
            }
        })
    }

    #[test]
    fn an_ok_response_maps_to_the_ytdlp_probe_shape() {
        let json = response_to_info_json(&ok_response(), "jNQXAC9IVRw", "test UA").unwrap();
        let probe = crate::model::parse_probe_json(&json).unwrap();
        let crate::model::Probe::Video(info) = probe else {
            panic!("expected a single video");
        };
        assert_eq!(info.title, "Me at the zoo");
        assert_eq!(info.duration, Some(19.0));
        assert_eq!(info.uploader.as_deref(), Some("jawed"));
        assert_eq!(
            info.webpage_url,
            "https://www.youtube.com/watch?v=jNQXAC9IVRw"
        );
        assert_eq!(info.extractor, "youtube");
        assert_eq!(
            info.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg")
        );

        let by_id = |id: &str| {
            info.formats
                .iter()
                .find(|f| f.format_id == id)
                .unwrap_or_else(|| panic!("format {id} missing"))
        };
        let muxed = by_id("18");
        assert_eq!(muxed.vcodec.as_deref(), Some("avc1.42001E"));
        assert_eq!(muxed.acodec.as_deref(), Some("mp4a.40.2"));
        assert_eq!(muxed.ext, "mp4");
        assert_eq!(muxed.filesize, Some(955_268));
        assert_eq!(muxed.protocol, "https");
        assert!((muxed.tbr.unwrap() - 389.0).abs() < 0.5);
        assert_eq!(
            muxed.http_headers.get("User-Agent").map(String::as_str),
            Some("test UA")
        );
        let video_only = by_id("247");
        assert_eq!(video_only.vcodec.as_deref(), Some("vp9"));
        assert_eq!(video_only.acodec, None, "yt-dlp shape drops acodec=none");
        assert_eq!(video_only.width, Some(640));
        let audio = by_id("249");
        assert_eq!(audio.vcodec, None);
        assert_eq!(audio.acodec.as_deref(), Some("opus"));
        assert_eq!(audio.asr, Some(48_000));
        assert!((audio.abr.unwrap() - 59.0).abs() < 0.5);

        use crate::model::FormatKind;
        assert_eq!(by_id("18").kind, FormatKind::Muxed);
        assert_eq!(by_id("247").kind, FormatKind::Video);
        assert_eq!(by_id("249").kind, FormatKind::Audio);
        // Downstream compatibility: quality choices come out as they do for yt-dlp output.
        assert!(!info.choices.is_empty());
    }

    #[test]
    fn non_ok_playability_declines() {
        let mut r = ok_response();
        r["playabilityStatus"]["status"] = json!("LOGIN_REQUIRED");
        assert_eq!(response_to_info_json(&r, "x", "UA"), None);
    }

    #[test]
    fn any_signature_cipher_declines_the_whole_response() {
        let mut r = ok_response();
        r["streamingData"]["adaptiveFormats"][1]
            .as_object_mut()
            .unwrap()
            .remove("url");
        r["streamingData"]["adaptiveFormats"][1]["signatureCipher"] =
            json!("s=abc&sp=sig&url=https://googlevideo.com/x");
        assert_eq!(response_to_info_json(&r, "x", "UA"), None);
    }

    #[test]
    fn empty_streaming_data_declines() {
        let mut r = ok_response();
        r["streamingData"]["formats"] = json!([]);
        r["streamingData"]["adaptiveFormats"] = json!([]);
        assert_eq!(response_to_info_json(&r, "x", "UA"), None);
    }
}
