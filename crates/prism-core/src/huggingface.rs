//! Models (and datasets) from Hugging Face (#29): a model is found by its name (`owner/name`), a
//! link to it or a search; its files are listed with their sizes and SHA-256; Prism suggests the
//! files worth downloading (one GGUF quantization, the weights in one format), and the chosen
//! files go into the "Hugging Face" queue as ordinary multi-connection downloads, checked
//! against their SHA-256 when they finish.
//!
//! The server is huggingface.co, or a mirror (hf-mirror.com) the user picks or `HF_ENDPOINT`
//! names. The access token (for gated and private models) is never stored with the downloads:
//! [`authorize`] adds it when a download starts, only for the server's own links (the engine
//! leaves it out of requests to the download servers they redirect to).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::events::CoreEvent;
use crate::manager::{Core, NewItem, StartMode};
use crate::model::{Queue, Status};
use crate::settings::{ConflictAction, HuggingFace, Settings};

pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";
pub const MIRROR_ENDPOINT: &str = "https://hf-mirror.com";
/// The queue model files go into (created when first needed).
pub const QUEUE_ID: &str = "huggingface";
const QUEUE_NAME: &str = "Hugging Face";
/// Files up to this size go first: configs and tokenizers are ready before the weights.
const SMALL_FILE: u64 = 64 << 20;
const API_TIMEOUT: Duration = Duration::from_secs(30);
/// A repository listing (thousands of files at most) or a search answer.
const MAX_API_BODY: usize = 32 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepoKind {
    #[default]
    Model,
    Dataset,
}

impl RepoKind {
    fn api(self) -> &'static str {
        match self {
            RepoKind::Model => "models",
            RepoKind::Dataset => "datasets",
        }
    }

    fn url_prefix(self) -> &'static str {
        match self {
            RepoKind::Model => "",
            RepoKind::Dataset => "datasets/",
        }
    }
}

/// A repository the user named: `owner/name`, or a link to it or into it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub kind: RepoKind,
    /// `owner/name`.
    pub id: String,
    /// A branch, tag or commit from the link (`None`: main).
    pub revision: Option<String>,
    /// The folder or file inside the repository the link pointed at.
    pub path: Option<String>,
}

/// What a link to Hugging Face is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A repository's page, or a folder in it: opens the model window.
    Repo(RepoRef),
    /// A file's page (`/blob/`): the file itself is at this `/resolve/` link.
    FilePage(String),
}

/// First path segments on the site that are not owners.
const RESERVED: &[&str] = &[
    "api",
    "docs",
    "spaces",
    "models",
    "settings",
    "blog",
    "pricing",
    "organizations",
    "collections",
    "papers",
    "learn",
    "join",
    "login",
    "logout",
    "posts",
    "chat",
    "enterprise",
    "tasks",
    "search",
    "new",
    "new-dataset",
    "new-space",
    "new-collection",
    "notifications",
    "inference-endpoints",
    "welcome",
    "terms-of-service",
    "privacy",
    "changelog",
    "hub",
    "oauth",
    "billing",
    "storage",
    "subscribe",
    "status",
    "upload",
    "inference",
    "mcp",
    "deploy",
];

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !s.starts_with(['.', '-'])
        && !s.contains("..")
}

fn host_of(endpoint: &str) -> Option<String> {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// Hosts that serve Hugging Face's pages: huggingface.co, hf.co, the mirror and the server the
/// user set.
fn is_hub_host(host: &str, endpoint: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    matches!(host, "huggingface.co" | "hf.co" | "hf-mirror.com")
        || host_of(endpoint).is_some_and(|h| h.strip_prefix("www.").unwrap_or(&h) == host)
}

/// What `input` names: `owner/name`, `datasets/owner/name`, or a link to a repository, a folder
/// in it or a file's page. `None` for anything else (a file's own `/resolve/` link included: it
/// downloads like any other link).
pub fn parse(input: &str, endpoint: &str) -> Option<Link> {
    let input = input.trim();
    if input.contains("://") {
        return parse_url(input, endpoint);
    }
    let parts: Vec<&str> = input.trim_matches('/').split('/').collect();
    let (kind, rest) = match parts.as_slice() {
        ["datasets", rest @ ..] => (RepoKind::Dataset, rest),
        rest => (RepoKind::Model, rest),
    };
    match rest {
        [owner, name] if valid_part(owner) && valid_part(name) && !RESERVED.contains(owner) => {
            Some(Link::Repo(RepoRef {
                kind,
                id: format!("{owner}/{name}"),
                revision: None,
                path: None,
            }))
        }
        _ => None,
    }
}

fn parse_url(input: &str, endpoint: &str) -> Option<Link> {
    let url = url::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https") || !is_hub_host(url.host_str()?, endpoint) {
        return None;
    }
    let segments: Vec<String> = url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .map(|s| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8_lossy()
                .into_owned()
        })
        .collect();
    let (kind, rest) = match segments.split_first() {
        Some((first, rest)) if first == "datasets" => (RepoKind::Dataset, rest),
        _ => (RepoKind::Model, segments.as_slice()),
    };
    let [owner, name, tail @ ..] = rest else {
        return None;
    };
    if !valid_part(owner) || !valid_part(name) || RESERVED.contains(&owner.as_str()) {
        return None;
    }
    let id = format!("{owner}/{name}");
    match tail {
        [action, _, path @ ..] if action == "blob" && !path.is_empty() => {
            let mut file = url.clone();
            let mut new_path = url.path().to_string();
            if let Some(i) = new_path.find("/blob/") {
                new_path.replace_range(i..i + 6, "/resolve/");
            }
            file.set_path(&new_path);
            file.set_query(None);
            file.set_fragment(None);
            Some(Link::FilePage(file.to_string()))
        }
        [action, ..] if action == "resolve" => None,
        [action, rev, path @ ..] if action == "tree" => Some(Link::Repo(RepoRef {
            kind,
            id,
            revision: Some(rev.clone()),
            path: (!path.is_empty()).then(|| path.join("/")),
        })),
        _ => Some(Link::Repo(RepoRef {
            kind,
            id,
            revision: None,
            path: None,
        })),
    }
}

/// The server: the setting, else `HF_ENDPOINT`, else huggingface.co (no trailing slash).
pub fn endpoint(s: &HuggingFace) -> String {
    let env = std::env::var("HF_ENDPOINT").ok();
    endpoint_from(&s.endpoint, env.as_deref())
}

fn endpoint_from(setting: &str, env: Option<&str>) -> String {
    let pick = |v: &str| {
        let v = v.trim().trim_end_matches('/');
        url::Url::parse(v)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .map(|_| v.to_string())
    };
    pick(setting)
        .or_else(|| env.and_then(pick))
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

/// Where the access token came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenSource {
    /// Typed into Settings.
    Settings,
    /// `HF_TOKEN` (or the older `HUGGING_FACE_HUB_TOKEN`).
    Environment,
    /// Saved by `hf auth login` / `huggingface-cli login`.
    Login,
}

/// The access token to use, if there is one.
pub fn token(s: &HuggingFace) -> Option<(String, TokenSource)> {
    let clean = |v: String| {
        Some(v.trim().to_string()).filter(|v| !v.is_empty() && !v.contains(char::is_whitespace))
    };
    if let Some(t) = clean(s.token.clone()) {
        return Some((t, TokenSource::Settings));
    }
    for var in ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"] {
        if let Some(t) = std::env::var(var).ok().and_then(clean) {
            return Some((t, TokenSource::Environment));
        }
    }
    login_token_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(clean)
        .map(|t| (t, TokenSource::Login))
}

/// The token file the Hugging Face tools save (`$HF_TOKEN_PATH`, `$HF_HOME/token`, or
/// `~/.cache/huggingface/token`).
fn login_token_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HF_TOKEN_PATH") {
        return Some(PathBuf::from(p));
    }
    if let Some(home) = std::env::var_os("HF_HOME") {
        return Some(PathBuf::from(home).join("token"));
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))?;
    Some(cache.join("huggingface").join("token"))
}

/// Adds the access token to a download from the Hugging Face server (a `/resolve/` link on it).
/// It is added when the download starts, so it is never stored with it and a changed token
/// applies to the next try.
pub(crate) fn authorize(req: &mut prism_engine::DownloadRequest, settings: &Settings) {
    let s = &settings.hugging_face;
    let server = endpoint(s);
    let on_server = url::Url::parse(&server).is_ok_and(|e| e.origin() == req.url.origin())
        || (server == DEFAULT_ENDPOINT
            && req.url.scheme() == "https"
            && req.url.port().is_none()
            && matches!(
                req.url.host_str(),
                Some("huggingface.co" | "www.huggingface.co" | "hf.co")
            ));
    if !on_server
        || !req.url.path().contains("/resolve/")
        || req
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
    {
        return;
    }
    if let Some((t, _)) = token(s) {
        req.headers
            .push(("Authorization".into(), format!("Bearer {t}")));
    }
}

/// A file's download link, pinned to a commit so every file comes from the same version.
pub fn file_url(endpoint: &str, kind: RepoKind, id: &str, revision: &str, path: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    const SEGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'`')
        .add(b'{')
        .add(b'}')
        .add(b'/');
    let path = path
        .split('/')
        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");
    let revision = utf8_percent_encode(revision, SEGMENT);
    format!(
        "{}/{}{id}/resolve/{revision}/{path}",
        endpoint.trim_end_matches('/'),
        kind.url_prefix()
    )
}

/// A file in a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
    /// For files stored with Git LFS / Xet (the weights): checked after the download.
    pub sha256: Option<String>,
}

/// One version of a GGUF model (a quantization, maybe split into parts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    /// "Q4_K_M", "Q8_0", "F16", …
    pub label: String,
    pub files: Vec<String>,
    pub size: u64,
}

/// The files Prism suggests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// GGUF versions, smallest first (empty when there is only one).
    pub variants: Vec<Variant>,
    /// The version picked by default.
    pub variant: Option<String>,
    /// A GGUF model's vision part (`mmproj`), added to whichever version is picked.
    pub extras: Vec<String>,
    /// The suggested files.
    pub files: Vec<String>,
}

/// A repository with its files.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub kind: RepoKind,
    pub id: String,
    /// The commit the files are downloaded from.
    pub sha: String,
    pub revision: String,
    pub gated: bool,
    pub private: bool,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    pub pipeline_tag: Option<String>,
    pub library: Option<String>,
    pub gguf: bool,
    pub files: Vec<RepoFile>,
    /// The folder or file the link pointed at.
    pub path: Option<String>,
    pub page_url: String,
    /// The default save folder: `<models folder>/<owner>/<name>`.
    pub folder: String,
    pub suggestion: Suggestion,
}

/// A search result.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub id: String,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    pub pipeline_tag: Option<String>,
    pub gguf: bool,
    pub gated: bool,
}

/// Server, token and folders, for the window and Settings.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub endpoint: String,
    pub token: Option<TokenSource>,
    pub folder: String,
    /// LM Studio's models folder, when LM Studio is installed (it reads `<owner>/<name>` too).
    pub lm_studio: Option<String>,
}

/// Files of a repository that are already there.
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Present {
    /// On disk with the right size.
    pub on_disk: Vec<String>,
    /// In the download list already (not failed, not in the trash).
    pub listed: Vec<String>,
}

/// What the window adds.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddModel {
    pub kind: RepoKind,
    pub id: String,
    pub sha: String,
    pub files: Vec<RepoFile>,
    pub folder: PathBuf,
    /// `None`: the "Hugging Face" queue, started.
    #[serde(default)]
    pub mode: Option<StartMode>,
}

/// Why a repository couldn't be read, for the window to explain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No such model (or a private one without a token).
    NotFound,
    /// Needs an access token (gated or private).
    NeedsToken,
    /// The token was refused.
    BadToken,
    /// Gated: the terms on the model's page aren't accepted yet.
    Gated,
    RateLimited,
    Server,
    Network,
}

impl Problem {
    pub fn code(self) -> &'static str {
        match self {
            Problem::NotFound => "hf:notFound",
            Problem::NeedsToken => "hf:needsToken",
            Problem::BadToken => "hf:badToken",
            Problem::Gated => "hf:gated",
            Problem::RateLimited => "hf:rateLimited",
            Problem::Server => "hf:server",
            Problem::Network => "hf:network",
        }
    }

    fn from_status(status: u16, error_code: Option<&str>, has_token: bool) -> Problem {
        match (status, error_code) {
            // Without a token, a private model, a gated one and a typo all answer 401.
            (401, _) if !has_token => Problem::NeedsToken,
            (_, Some("GatedRepo")) | (403, _) => Problem::Gated,
            (401, _) => Problem::BadToken,
            (404, _) | (_, Some("RepoNotFound" | "RevisionNotFound" | "EntryNotFound")) => {
                Problem::NotFound
            }
            (429, _) => Problem::RateLimited,
            _ => Problem::Server,
        }
    }
}

// ----- choosing files -----------------------------------------------------------------------

static QUANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[-_.\s])((?:UD-)?(?:I?Q[1-8](?:_[0-9A-Z]{1,4}){0,3}|TQ[12]_0|MXFP4(?:_MOE)?|BF16|FP16|F16|FP32|F32|FP8|F8(?:_E4M3)?))(?:$|[-_.\s])",
    )
    .expect("quant pattern")
});
static SPLIT_PART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)-\d{5}-of-\d{5}$").expect("split pattern"));

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn lower_ext(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// A GGUF file's quantization ("Q4_K_M"), from its name or else its folder.
fn quant_of(path: &str) -> Option<String> {
    let stem = file_name(path);
    let stem = stem
        .strip_suffix(".gguf")
        .or_else(|| stem.strip_suffix(".GGUF"))
        .unwrap_or(stem);
    let stem = SPLIT_PART.replace(stem, "");
    let last = |s: &str| {
        QUANT
            .captures_iter(s)
            .last()
            .map(|c| c[1].to_ascii_uppercase())
    };
    last(&stem).or_else(|| {
        let folder = path.rsplit_once('/').map(|(d, _)| d)?;
        last(file_name(folder))
    })
}

fn is_mmproj(path: &str) -> bool {
    file_name(path).to_ascii_lowercase().contains("mmproj")
}

/// The version picked when nothing says otherwise: the usual balance of size and quality first.
const PREFERRED: &[&str] = &[
    "Q4_K_M",
    "UD-Q4_K_XL",
    "Q4_K_XL",
    "Q4_K_S",
    "IQ4_XS",
    "IQ4_NL",
    "Q4_0",
    "MXFP4",
    "MXFP4_MOE",
    "Q5_K_M",
    "UD-Q5_K_XL",
    "Q5_K_S",
    "Q6_K",
    "UD-Q6_K_XL",
    "Q8_0",
    "UD-Q8_K_XL",
];

/// Folders and formats that repeat the main weights in another form.
fn is_alternative_format(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let top = lower.split('/').next().unwrap_or("");
    matches!(
        top,
        "onnx" | "openvino" | "coreml" | "tflite" | "mlx" | "original" | "tensorrt"
    ) || matches!(
        lower_ext(&lower).as_str(),
        "h5" | "msgpack" | "ot" | "onnx" | "onnx_data" | "tflite" | "mlmodel" | "pb"
    ) || lower.ends_with("/model.mlpackage")
}

/// The files Prism suggests for a repository (`path`: the folder or file a link pointed at).
pub fn suggest(files: &[RepoFile], kind: RepoKind, path: Option<&str>) -> Suggestion {
    if let Some(p) = path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty()) {
        let inside: Vec<String> = files
            .iter()
            .filter(|f| f.path == p || f.path.starts_with(&format!("{p}/")))
            .map(|f| f.path.clone())
            .collect();
        if !inside.is_empty() {
            let mut s = suggest_all(files, kind);
            s.files = inside;
            s.variant = None;
            return s;
        }
    }
    suggest_all(files, kind)
}

fn suggest_all(files: &[RepoFile], kind: RepoKind) -> Suggestion {
    if kind == RepoKind::Dataset {
        return Suggestion {
            files: files
                .iter()
                .filter(|f| f.path != ".gitattributes")
                .map(|f| f.path.clone())
                .collect(),
            ..Default::default()
        };
    }
    let gguf: Vec<&RepoFile> = files
        .iter()
        .filter(|f| lower_ext(&f.path) == "gguf")
        .collect();
    let weights_elsewhere = files.iter().any(|f| {
        matches!(
            lower_ext(&f.path).as_str(),
            "safetensors" | "bin" | "pt" | "pth"
        ) && f.size > SMALL_FILE
    });
    if !gguf.is_empty() && !weights_elsewhere {
        return suggest_gguf(&gguf);
    }
    Suggestion {
        files: suggest_weights(files),
        ..Default::default()
    }
}

fn suggest_gguf(gguf: &[&RepoFile]) -> Suggestion {
    let mut groups: BTreeMap<String, Vec<&RepoFile>> = BTreeMap::new();
    let mut extras: Vec<&RepoFile> = Vec::new();
    for f in gguf {
        if is_mmproj(&f.path) {
            extras.push(f);
            continue;
        }
        let label = quant_of(&f.path).unwrap_or_else(|| {
            let stem = file_name(&f.path);
            SPLIT_PART
                .replace(stem.trim_end_matches(".gguf"), "")
                .into_owned()
        });
        groups.entry(label).or_default().push(f);
    }
    let mut variants: Vec<Variant> = groups
        .into_iter()
        .map(|(label, mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Variant {
                size: files.iter().map(|f| f.size).sum(),
                files: files.iter().map(|f| f.path.clone()).collect(),
                label,
            }
        })
        .collect();
    variants.sort_by(|a, b| a.size.cmp(&b.size).then(a.label.cmp(&b.label)));
    let picked = PREFERRED
        .iter()
        .find_map(|p| variants.iter().find(|v| v.label == *p))
        .or_else(|| variants.get(variants.len().saturating_sub(1) / 2))
        .cloned();
    // One vision part is enough: 16-bit if there is one.
    let extra = extras
        .iter()
        .min_by_key(|f| {
            let q = quant_of(&f.path).unwrap_or_default();
            (
                match q.as_str() {
                    "F16" | "FP16" => 0,
                    "BF16" => 1,
                    "Q8_0" => 2,
                    "F32" | "FP32" => 4,
                    _ => 3,
                },
                f.path.clone(),
            )
        })
        .map(|f| f.path.clone());
    let mut files = picked.as_ref().map(|v| v.files.clone()).unwrap_or_default();
    files.extend(extra.clone());
    Suggestion {
        variant: picked.map(|v| v.label),
        variants: if variants.len() > 1 {
            variants
        } else {
            Vec::new()
        },
        extras: extra.into_iter().collect(),
        files,
    }
}

/// Everything needed to load the model, in one weights format: safetensors when there are any
/// (not also the PyTorch `.bin`, ONNX, TensorFlow or the `original/` checkpoint), and of a
/// `x.safetensors` with an `x.fp16.safetensors` beside it, only the first.
fn suggest_weights(files: &[RepoFile]) -> Vec<String> {
    let has = |ext: &str| files.iter().any(|f| lower_ext(&f.path) == ext);
    let safetensors = has("safetensors");
    let names: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    files
        .iter()
        .filter(|f| f.path != ".gitattributes")
        .filter(|f| !is_alternative_format(&f.path))
        .filter(|f| {
            let ext = lower_ext(&f.path);
            match ext.as_str() {
                "bin" | "pt" | "pth" | "ckpt" if safetensors => {
                    // Small .bin files that aren't weights (e.g. training_args.bin) stay.
                    f.size <= SMALL_FILE && !file_name(&f.path).starts_with("pytorch_model")
                }
                "gguf" => false,
                _ => true,
            }
        })
        .filter(|f| {
            // `unet/model.fp16.safetensors` next to `unet/model.safetensors`.
            let p = f.path.as_str();
            !["fp16", "fp32", "bf16"].iter().any(|v| {
                p.rsplit_once('.').is_some_and(|(stem, ext)| {
                    stem.strip_suffix(&format!(".{v}"))
                        .is_some_and(|base| names.contains(format!("{base}.{ext}").as_str()))
                })
            })
        })
        .map(|f| f.path.clone())
        .collect()
}

/// Small files first, then by path (the parts of split weights in order).
fn download_order(files: &mut [RepoFile]) {
    files.sort_by(|a, b| {
        (a.size > SMALL_FILE)
            .cmp(&(b.size > SMALL_FILE))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// A repository file's place in the save folder; `None` for paths that would leave it.
fn local_path(folder: &Path, path: &str) -> Option<PathBuf> {
    let mut out = folder.to_path_buf();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':']) {
            return None;
        }
        out.push(crate::manager::sanitize_name(part));
    }
    Some(out)
}

// ----- the API ------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiLfs {
    sha256: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiSibling {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<ApiLfs>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiRepo {
    id: String,
    sha: String,
    gated: serde_json::Value,
    private: bool,
    downloads: Option<u64>,
    likes: Option<u64>,
    pipeline_tag: Option<String>,
    library_name: Option<String>,
    tags: Vec<String>,
    siblings: Vec<ApiSibling>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiHit {
    id: String,
    downloads: Option<u64>,
    likes: Option<u64>,
    pipeline_tag: Option<String>,
    tags: Vec<String>,
    gated: serde_json::Value,
}

fn is_gated(v: &serde_json::Value) -> bool {
    !matches!(v, serde_json::Value::Null | serde_json::Value::Bool(false))
}

fn repo_files(siblings: Vec<ApiSibling>) -> Vec<RepoFile> {
    siblings
        .into_iter()
        .filter(|s| !s.rfilename.is_empty())
        .map(|s| {
            let lfs = s.lfs.unwrap_or_default();
            RepoFile {
                size: s.size.or(lfs.size).unwrap_or(0),
                sha256: lfs
                    .sha256
                    .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())),
                path: s.rfilename,
            }
        })
        .collect()
}

impl Core {
    /// Server, token and folders for the model window and Settings.
    pub fn hf_context(&self) -> Context {
        let settings = self.settings();
        let s = &settings.hugging_face;
        Context {
            endpoint: endpoint(s),
            token: token(s).map(|(_, source)| source),
            folder: models_folder(&settings).to_string_lossy().into_owned(),
            lm_studio: lm_studio_folder().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    /// Reads a repository named by `input` (a name or a link) with its files.
    pub async fn hf_repo(&self, input: &str) -> Result<Repo> {
        let settings = self.settings();
        let server = endpoint(&settings.hugging_face);
        let repo = match parse(input, &server) {
            Some(Link::Repo(r)) => r,
            _ => return Err(anyhow!(Problem::NotFound.code())),
        };
        let revision = repo.revision.clone().unwrap_or_else(|| "main".into());
        let api = format!(
            "{server}/api/{}/{}/revision/{}?blobs=true",
            repo.kind.api(),
            repo.id,
            percent_encoding::utf8_percent_encode(&revision, percent_encoding::NON_ALPHANUMERIC)
        );
        let info: ApiRepo = self.hf_get(&api, &settings, true).await?;
        let files = repo_files(info.siblings);
        let id = if info.id.is_empty() {
            repo.id.clone()
        } else {
            info.id
        };
        // `<models>/<owner>/<name>`, with the system's own separators.
        let mut folder = models_folder(&settings);
        if repo.kind == RepoKind::Dataset {
            folder.push("datasets");
        }
        folder.extend(id.split('/'));
        Ok(Repo {
            suggestion: suggest(&files, repo.kind, repo.path.as_deref()),
            kind: repo.kind,
            sha: if info.sha.is_empty() {
                revision.clone()
            } else {
                info.sha
            },
            revision,
            gated: is_gated(&info.gated),
            private: info.private,
            downloads: info.downloads,
            likes: info.likes,
            pipeline_tag: info.pipeline_tag,
            library: info.library_name,
            gguf: info.tags.iter().any(|t| t == "gguf")
                || files.iter().any(|f| lower_ext(&f.path) == "gguf"),
            page_url: format!("{server}/{}{id}", repo.kind.url_prefix()),
            folder: folder.to_string_lossy().into_owned(),
            path: repo.path,
            files,
            id,
        })
    }

    /// Models whose name matches `query`, most downloaded first (`""`: what's trending).
    pub async fn hf_search(&self, query: &str) -> Result<Vec<Hit>> {
        let settings = self.settings();
        let server = endpoint(&settings.hugging_face);
        let q = query.trim();
        let api = if q.is_empty() {
            format!("{server}/api/models?sort=trendingScore&direction=-1&limit=12")
        } else {
            format!(
                "{server}/api/models?search={}&sort=downloads&direction=-1&limit=12",
                percent_encoding::utf8_percent_encode(q, percent_encoding::NON_ALPHANUMERIC)
            )
        };
        let hits: Vec<ApiHit> = self.hf_get(&api, &settings, false).await?;
        Ok(hits
            .into_iter()
            .filter(|h| h.id.contains('/'))
            .map(|h| Hit {
                gguf: h.tags.iter().any(|t| t == "gguf"),
                gated: is_gated(&h.gated),
                id: h.id,
                downloads: h.downloads,
                likes: h.likes,
                pipeline_tag: h.pipeline_tag,
            })
            .collect())
    }

    async fn hf_get<T: serde::de::DeserializeOwned>(
        &self,
        api: &str,
        settings: &Settings,
        with_token: bool,
    ) -> Result<T> {
        let url = url::Url::parse(api).map_err(|_| anyhow!(Problem::NotFound.code()))?;
        let mut req = prism_engine::DownloadRequest::new(url, PathBuf::new());
        req.proxy = Some(crate::manager::proxy_config(settings));
        req.http = crate::manager::http_preference(settings);
        req.headers
            .push(("Accept".into(), "application/json".into()));
        let token = with_token.then(|| token(&settings.hugging_face)).flatten();
        if let Some((t, _)) = &token {
            req.headers
                .push(("Authorization".into(), format!("Bearer {t}")));
        }
        let cfg = crate::manager::engine_config(settings);
        let resp = tokio::time::timeout(
            API_TIMEOUT,
            prism_engine::fetch_small(&cfg, &req, MAX_API_BODY),
        )
        .await
        .map_err(|_| anyhow!(Problem::Network.code()))?
        .map_err(|e| {
            tracing::warn!("Hugging Face: {e}");
            anyhow!(Problem::Network.code())
        })?;
        if !(200..300).contains(&resp.status) {
            let problem =
                Problem::from_status(resp.status, resp.header("x-error-code"), token.is_some());
            tracing::info!(
                "Hugging Face answered {} ({}) for {}",
                resp.status,
                resp.header("x-error-code").unwrap_or("-"),
                resp.final_url.path()
            );
            return Err(anyhow!(problem.code()));
        }
        serde_json::from_slice(&resp.bytes).map_err(|e| {
            tracing::warn!("Hugging Face sent something unexpected: {e}");
            anyhow!(Problem::Server.code())
        })
    }

    /// Which of `files` are already in `folder` (right size) or in the download list.
    pub fn hf_present(
        &self,
        kind: RepoKind,
        id: &str,
        sha: &str,
        folder: &Path,
        files: &[RepoFile],
    ) -> Present {
        let server = endpoint(&self.settings().hugging_face);
        let listed_urls: HashSet<String> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .filter(|i| !i.trashed && !matches!(i.status, Status::Failed))
                .map(|i| i.url.clone())
                .collect()
        };
        let mut present = Present::default();
        for f in files {
            if listed_urls.contains(&file_url(&server, kind, id, sha, &f.path)) {
                present.listed.push(f.path.clone());
            } else if local_path(folder, &f.path)
                .and_then(|p| std::fs::metadata(p).ok())
                .is_some_and(|m| m.is_file() && m.len() == f.size)
            {
                present.on_disk.push(f.path.clone());
            }
        }
        present
    }

    /// Adds the chosen files: by default into the "Hugging Face" queue, which starts.
    pub fn hf_add(&self, d: AddModel) -> Result<usize> {
        let _rt = crate::rt::enter();
        let settings = self.settings();
        let server = endpoint(&settings.hugging_face);
        let mut files = d.files;
        download_order(&mut files);
        let mut entries = Vec::with_capacity(files.len());
        for f in &files {
            let target = local_path(&d.folder, &f.path)
                .ok_or_else(|| anyhow!("unusual file name: {}", f.path))?;
            let dir = target.parent().unwrap_or(&d.folder).to_path_buf();
            entries.push(NewItem {
                url: file_url(&server, d.kind, &d.id, &d.sha, &f.path),
                file_name: Some(file_name(&f.path).to_string()),
                save_dir: Some(dir),
                category: Some(crate::categories::OTHER.to_string()),
                description: format!("{} · Hugging Face", d.id),
                size: (f.size > 0).then_some(f.size),
                checksum: f.sha256.as_ref().map(|h| format!("sha256:{h}")),
                // The folder is the model's own: a file left from an earlier try is replaced
                // (one with the right size was offered as already downloaded).
                conflict: Some(ConflictAction::Overwrite),
                ..Default::default()
            });
        }
        if entries.is_empty() {
            return Ok(0);
        }
        let count = entries.len();
        match d.mode {
            Some(mode) => {
                self.add_items(entries, mode)?;
            }
            None => {
                self.ensure_hf_queue();
                self.add_items(entries, StartMode::Queue(QUEUE_ID.into()))?;
                self.start_queue(QUEUE_ID);
            }
        }
        tracing::info!("added {count} files of {} from Hugging Face", d.id);
        Ok(count)
    }

    /// The "Hugging Face" queue: two files at a time (a big one and the small ones beside it).
    fn ensure_hf_queue(&self) {
        let exists = self.0.state.lock().queues.iter().any(|q| q.id == QUEUE_ID);
        if exists {
            return;
        }
        let position = self.0.state.lock().queues.len() as i64;
        let _ = self.upsert_queue(Queue {
            id: QUEUE_ID.into(),
            name: QUEUE_NAME.into(),
            max_concurrent: 2,
            speed_limit: 0,
            position,
            builtin: false,
            schedule: Default::default(),
            running: false,
        });
        self.emit(CoreEvent::QueuesChanged(self.queues()));
    }
}

/// `<setting>`, or `Models` in the download folder.
fn models_folder(settings: &Settings) -> PathBuf {
    let s = settings.hugging_face.folder.trim();
    if s.is_empty() {
        settings.downloads.default_dir.join("Models")
    } else {
        PathBuf::from(s)
    }
}

/// LM Studio's models folder (`~/.lmstudio/models`), when it exists.
fn lm_studio_folder() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    [
        home.join(".lmstudio").join("models"),
        home.join(".cache").join("lm-studio").join("models"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.into(),
            size,
            sha256: None,
        }
    }

    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;

    fn repo(id: &str) -> Option<Link> {
        Some(Link::Repo(RepoRef {
            kind: RepoKind::Model,
            id: id.into(),
            revision: None,
            path: None,
        }))
    }

    #[test]
    fn names_and_links() {
        let e = DEFAULT_ENDPOINT;
        assert_eq!(parse("Qwen/Qwen3-8B", e), repo("Qwen/Qwen3-8B"));
        assert_eq!(
            parse("  unsloth/gpt-oss-20b-GGUF ", e),
            repo("unsloth/gpt-oss-20b-GGUF")
        );
        assert_eq!(
            parse("https://huggingface.co/Qwen/Qwen3-8B", e),
            repo("Qwen/Qwen3-8B")
        );
        assert_eq!(
            parse("https://hf.co/Qwen/Qwen3-8B/discussions/3", e),
            repo("Qwen/Qwen3-8B")
        );
        assert_eq!(
            parse("https://hf-mirror.com/Qwen/Qwen3-8B", e),
            repo("Qwen/Qwen3-8B")
        );
        assert_eq!(
            parse(
                "https://huggingface.co/unsloth/Qwen3-30B-A3B-GGUF/tree/main/Q4_K_M",
                e
            ),
            Some(Link::Repo(RepoRef {
                kind: RepoKind::Model,
                id: "unsloth/Qwen3-30B-A3B-GGUF".into(),
                revision: Some("main".into()),
                path: Some("Q4_K_M".into()),
            }))
        );
        assert_eq!(
            parse("https://huggingface.co/datasets/HuggingFaceFW/fineweb", e),
            Some(Link::Repo(RepoRef {
                kind: RepoKind::Dataset,
                id: "HuggingFaceFW/fineweb".into(),
                revision: None,
                path: None,
            }))
        );
        assert_eq!(
            parse("https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/blob/main/qwen2.5-0.5b-instruct-q2_k.gguf?x=1", e),
            Some(Link::FilePage(
                "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q2_k.gguf".into()
            ))
        );
        // A file's own link downloads as it is; pages that aren't repositories are left alone.
        assert_eq!(
            parse(
                "https://huggingface.co/Qwen/Qwen3-8B/resolve/main/config.json",
                e
            ),
            None
        );
        assert_eq!(parse("https://huggingface.co/docs/hub/index", e), None);
        assert_eq!(parse("https://huggingface.co/spaces/owner/app", e), None);
        assert_eq!(parse("https://huggingface.co/Qwen", e), None);
        assert_eq!(parse("https://example.com/Qwen/Qwen3-8B", e), None);
        assert_eq!(
            parse("https://example.com/Qwen/Qwen3-8B", "https://example.com"),
            repo("Qwen/Qwen3-8B")
        );
        assert_eq!(parse("just some words", e), None);
        assert_eq!(parse("../etc/passwd", e), None);
        assert_eq!(parse("C:/Users/x", e), None);
    }

    #[test]
    fn server_and_file_links() {
        assert_eq!(endpoint_from("", None), DEFAULT_ENDPOINT);
        assert_eq!(
            endpoint_from("", Some("https://hf-mirror.com/")),
            MIRROR_ENDPOINT
        );
        assert_eq!(
            endpoint_from("https://my.mirror/", Some("https://hf-mirror.com")),
            "https://my.mirror"
        );
        assert_eq!(endpoint_from("not a url", None), DEFAULT_ENDPOINT);
        assert_eq!(
            file_url(
                DEFAULT_ENDPOINT,
                RepoKind::Model,
                "o/m",
                "abc123",
                "Q4_K_M/model 1#.gguf"
            ),
            "https://huggingface.co/o/m/resolve/abc123/Q4_K_M/model%201%23.gguf"
        );
        assert_eq!(
            file_url(
                MIRROR_ENDPOINT,
                RepoKind::Dataset,
                "o/d",
                "refs/pr/1",
                "data/x.parquet"
            ),
            "https://hf-mirror.com/datasets/o/d/resolve/refs%2Fpr%2F1/data/x.parquet"
        );
    }

    #[test]
    fn the_token_goes_only_to_the_servers_own_file_links() {
        let mut settings = Settings::default();
        settings.hugging_face.token = "hf_secret".into();
        let auth = |url: &str, s: &Settings| {
            let mut req =
                prism_engine::DownloadRequest::new(url::Url::parse(url).unwrap(), PathBuf::new());
            authorize(&mut req, s);
            req.headers
                .iter()
                .find(|(k, _)| k == "Authorization")
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            auth(
                "https://huggingface.co/o/m/resolve/abc/model.gguf",
                &settings
            )
            .as_deref(),
            Some("Bearer hf_secret")
        );
        assert_eq!(auth("https://huggingface.co/o/m", &settings), None);
        assert_eq!(auth("https://cdn-lfs.hf.co/o/m/resolve/x", &settings), None);
        assert_eq!(
            auth("https://example.com/o/m/resolve/abc/model.gguf", &settings),
            None
        );
        assert_eq!(
            auth(
                "http://huggingface.co/o/m/resolve/abc/model.gguf",
                &settings
            ),
            None
        );
        assert_eq!(
            auth("http://hf.co/o/m/resolve/abc/model.gguf", &settings),
            None
        );
        settings.hugging_face.endpoint = MIRROR_ENDPOINT.into();
        assert_eq!(
            auth(
                "https://huggingface.co/o/m/resolve/abc/model.gguf",
                &settings
            ),
            None
        );
        assert!(
            auth(
                "https://hf-mirror.com/o/m/resolve/abc/model.gguf",
                &settings
            )
            .is_some()
        );
    }

    #[test]
    fn quantizations() {
        assert_eq!(
            quant_of("qwen2.5-0.5b-instruct-q4_k_m.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(
            quant_of("Llama-3.2-3B-Instruct-Q8_0.gguf").as_deref(),
            Some("Q8_0")
        );
        assert_eq!(
            quant_of("gpt-oss-20b-UD-Q4_K_XL.gguf").as_deref(),
            Some("UD-Q4_K_XL")
        );
        assert_eq!(
            quant_of("model-IQ4_XS-00002-of-00003.gguf").as_deref(),
            Some("IQ4_XS")
        );
        assert_eq!(quant_of("gemma-3-4b-it-BF16.gguf").as_deref(), Some("BF16"));
        assert_eq!(
            quant_of("Q4_K_M/Qwen3-235B-A22B-00001-of-00003.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(quant_of("gpt-oss-20b-mxfp4.gguf").as_deref(), Some("MXFP4"));
        assert_eq!(quant_of("ggml-model.gguf"), None);
    }

    #[test]
    fn one_gguf_version_is_suggested() {
        let files = vec![
            f(".gitattributes", 1500),
            f("README.md", 5000),
            f("m-Q2_K.gguf", GB),
            f("m-Q4_K_M.gguf", 2 * GB),
            f("m-Q8_0.gguf", 4 * GB),
            f("m-F16.gguf", 8 * GB),
            f("mmproj-m-F32.gguf", 800 * MB),
            f("mmproj-m-F16.gguf", 400 * MB),
        ];
        let s = suggest(&files, RepoKind::Model, None);
        assert_eq!(s.variant.as_deref(), Some("Q4_K_M"));
        assert_eq!(s.files, vec!["m-Q4_K_M.gguf", "mmproj-m-F16.gguf"]);
        assert_eq!(
            s.variants
                .iter()
                .map(|v| v.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Q2_K", "Q4_K_M", "Q8_0", "F16"]
        );
        assert_eq!(s.extras, vec!["mmproj-m-F16.gguf"]);
    }

    #[test]
    fn split_gguf_parts_stay_together() {
        let files = vec![
            f("Q8_0/big-Q8_0-00002-of-00002.gguf", 40 * GB),
            f("Q8_0/big-Q8_0-00001-of-00002.gguf", 50 * GB),
            f("Q4_K_M/big-Q4_K_M-00001-of-00002.gguf", 30 * GB),
            f("Q4_K_M/big-Q4_K_M-00002-of-00002.gguf", 20 * GB),
        ];
        let s = suggest(&files, RepoKind::Model, None);
        assert_eq!(s.variant.as_deref(), Some("Q4_K_M"));
        assert_eq!(
            s.files,
            vec![
                "Q4_K_M/big-Q4_K_M-00001-of-00002.gguf",
                "Q4_K_M/big-Q4_K_M-00002-of-00002.gguf"
            ]
        );
        assert_eq!(s.variants[0].size, 50 * GB);
        // A link to one version's folder picks that folder.
        let s = suggest(&files, RepoKind::Model, Some("Q8_0"));
        assert_eq!(s.files.len(), 2);
        assert!(s.files.iter().all(|p| p.starts_with("Q8_0/")));
    }

    #[test]
    fn transformers_weights_in_one_format() {
        let files = vec![
            f(".gitattributes", 1500),
            f("README.md", 9000),
            f("config.json", 700),
            f("generation_config.json", 200),
            f("model-00001-of-00002.safetensors", 4 * GB),
            f("model-00002-of-00002.safetensors", 3 * GB),
            f("model.safetensors.index.json", 30000),
            f("pytorch_model-00001-of-00002.bin", 4 * GB),
            f("pytorch_model-00002-of-00002.bin", 3 * GB),
            f("pytorch_model.bin.index.json", 30000),
            f("tokenizer.json", 9 * MB),
            f("tokenizer_config.json", 50000),
            f("training_args.bin", 5000),
            f("original/consolidated.00.pth", 15 * GB),
            f("onnx/model.onnx", 7 * GB),
            f("flax_model.msgpack", 7 * GB),
            f("unet/diffusion_pytorch_model.safetensors", 3 * GB),
            f("unet/diffusion_pytorch_model.fp16.safetensors", GB),
        ];
        let s = suggest(&files, RepoKind::Model, None);
        assert!(s.variants.is_empty());
        assert_eq!(
            s.files,
            vec![
                "README.md",
                "config.json",
                "generation_config.json",
                "model-00001-of-00002.safetensors",
                "model-00002-of-00002.safetensors",
                "model.safetensors.index.json",
                "pytorch_model.bin.index.json",
                "tokenizer.json",
                "tokenizer_config.json",
                "training_args.bin",
                "unet/diffusion_pytorch_model.safetensors",
            ]
        );
        // Without safetensors, the PyTorch weights are the model.
        let old = vec![
            f("config.json", 700),
            f("pytorch_model.bin", 500 * MB),
            f("tf_model.h5", 500 * MB),
        ];
        assert_eq!(
            suggest(&old, RepoKind::Model, None).files,
            vec!["config.json", "pytorch_model.bin"]
        );
    }

    #[test]
    fn small_files_download_first_and_paths_stay_inside() {
        let mut files = vec![
            f("model-00002.safetensors", 3 * GB),
            f("config.json", 700),
            f("model-00001.safetensors", 4 * GB),
            f("tokenizer.json", 9 * MB),
        ];
        download_order(&mut files);
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec![
                "config.json",
                "tokenizer.json",
                "model-00001.safetensors",
                "model-00002.safetensors"
            ]
        );
        let base = Path::new("/models/o/m");
        assert_eq!(
            local_path(base, "unet/model.safetensors"),
            Some(base.join("unet").join("model.safetensors"))
        );
        assert_eq!(local_path(base, "../evil"), None);
        assert_eq!(local_path(base, "a//b"), None);
        assert_eq!(local_path(base, "C:/x"), None);
    }

    #[test]
    fn api_answers() {
        assert_eq!(
            Problem::from_status(401, Some("RepoNotFound"), false),
            Problem::NeedsToken
        );
        assert_eq!(
            Problem::from_status(401, Some("GatedRepo"), false),
            Problem::NeedsToken
        );
        assert_eq!(
            Problem::from_status(404, Some("RepoNotFound"), true),
            Problem::NotFound
        );
        assert_eq!(Problem::from_status(401, None, true), Problem::BadToken);
        assert_eq!(
            Problem::from_status(403, Some("GatedRepo"), true),
            Problem::Gated
        );
        assert_eq!(Problem::from_status(429, None, false), Problem::RateLimited);
        let api: ApiRepo = serde_json::from_str(
            r#"{"id":"o/m","sha":"abc","gated":"manual","siblings":[
                {"rfilename":"a.gguf","size":10,"lfs":{"sha256":"9ee36184e616dfc76df4f5dd66f908dbde6979524ae36e6cefb67f532f798cb8","size":10}},
                {"rfilename":"README.md","size":3}]}"#,
        )
        .unwrap();
        assert!(is_gated(&api.gated));
        let files = repo_files(api.siblings);
        assert_eq!(files[0].sha256.as_deref().map(str::len), Some(64));
        assert_eq!(files[1].sha256, None);
    }
}
