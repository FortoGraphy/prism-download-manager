//! Site Grabber: a polite, bounded web crawler that finds downloadable files.
//!
//! The crawler explores HTML pages breadth-first from a start URL up to a depth
//! limit, following only links allowed by the scope rules, and collects links
//! (and optionally embedded media/images) whose extension matches the file filter.
//! Results stream to the caller so the UI can render the file tree live.

use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// Only pages on the start URL's host.
    #[default]
    SameHost,
    /// Same registrable domain (e.g. `cdn.example.com` for `www.example.com`).
    SameDomain,
    /// Only pages below the start URL's directory.
    SubPath,
    /// Anything (still depth-limited).
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct GrabberConfig {
    pub start_url: String,
    pub depth: u32,
    pub scope: Scope,
    /// File extensions to collect, e.g. `["pdf", "zip"]`. Empty = any non-HTML link.
    pub extensions: Vec<String>,
    /// Also collect `<img>`, `<video>`, `<audio>`, `<source>` URLs.
    pub include_embedded: bool,
    /// Files may live on other hosts (CDNs) even when pages may not.
    pub files_any_host: bool,
    pub max_pages: u32,
    pub concurrency: u32,
    /// URL substrings to skip (e.g. `logout`, `?sort=`).
    pub exclude: Vec<String>,
    /// Ask the server for sizes (HEAD) of found files.
    pub fetch_sizes: bool,
    pub login: Option<Login>,
    pub user_agent: String,
    pub cookies: String,
}

impl Default for GrabberConfig {
    fn default() -> Self {
        Self {
            start_url: String::new(),
            depth: 2,
            scope: Scope::SameHost,
            extensions: Vec::new(),
            include_embedded: false,
            files_any_host: true,
            max_pages: 2000,
            concurrency: 6,
            exclude: vec!["logout".into(), "signout".into()],
            fetch_sizes: true,
            login: None,
            user_agent: String::new(),
            cookies: String::new(),
        }
    }
}

/// Credentials for sites that need them: HTTP auth or a form post performed first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Login {
    pub username: String,
    pub password: String,
    /// When set, POST `form_fields` (with `{username}`/`{password}` placeholders) here first.
    pub form_url: Option<String>,
    pub form_fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FoundFile {
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    /// Page where the link was found.
    pub referer: String,
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GrabberEvent {
    PageVisited {
        url: String,
        depth: u32,
        pages: u32,
        queued: u32,
    },
    FileFound {
        file: FoundFile,
    },
    PageError {
        url: String,
        error: String,
    },
    Finished {
        pages: u32,
        files: u32,
        cancelled: bool,
    },
}

const HTML_EXTENSIONS: &[&str] = &[
    "html", "htm", "php", "asp", "aspx", "jsp", "shtml", "cgi", "",
];

/// Handle to stop a running crawl.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

fn registrable_domain(host: &str) -> String {
    // Good-enough heuristic without a public-suffix list: keep the last two labels,
    // or three for common second-level ccTLD suffixes like co.uk / com.au.
    let labels: Vec<&str> = host.split('.').collect();
    let keep = if labels.len() >= 3
        && matches!(
            labels[labels.len() - 2],
            "co" | "com" | "org" | "net" | "ac" | "gov"
        ) {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

fn in_scope(scope: Scope, start: &Url, candidate: &Url) -> bool {
    let (Some(a), Some(b)) = (start.host_str(), candidate.host_str()) else {
        return false;
    };
    match scope {
        Scope::Any => true,
        Scope::SameHost => a.eq_ignore_ascii_case(b),
        Scope::SameDomain => registrable_domain(a).eq_ignore_ascii_case(&registrable_domain(b)),
        Scope::SubPath => {
            let dir = &start.path()[..start.path().rfind('/').map(|i| i + 1).unwrap_or(0)];
            a.eq_ignore_ascii_case(b) && candidate.path().starts_with(dir)
        }
    }
}

/// Whether the configured cookies and HTTP basic-auth may be sent to `url`: the start site's own
/// domain only, like a `Domain=.example.com` cookie in a browser.
pub(crate) fn credentials_allowed(start: &Url, url: &Url) -> bool {
    let (Some(a), Some(b)) = (start.host_str(), url.host_str()) else {
        return false;
    };
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    // An IP or a bare name has no domain, so nothing else can share it.
    if a.parse::<std::net::IpAddr>().is_ok()
        || b.parse::<std::net::IpAddr>().is_ok()
        || !a.contains('.')
        || !b.contains('.')
    {
        return false;
    }
    registrable_domain(a).eq_ignore_ascii_case(&registrable_domain(b))
}

fn extension(url: &Url) -> String {
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(last).decode_utf8_lossy();
    decoded
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn file_name(url: &Url) -> String {
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let name = percent_encoding::percent_decode_str(last)
        .decode_utf8_lossy()
        .to_string();
    if name.is_empty() {
        url.host_str().unwrap_or("download").to_string()
    } else {
        name
    }
}

/// Links found in one HTML document: `(pages to follow, candidate files)`.
pub fn extract_links(base: &Url, html: &str, include_embedded: bool) -> (Vec<Url>, Vec<Url>) {
    let doc = Html::parse_document(html);
    // A <base href> changes how relative links resolve.
    let base = Selector::parse("base[href]")
        .ok()
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("href"))
                .and_then(|h| base.join(h).ok())
        })
        .unwrap_or_else(|| base.clone());
    let mut links = Vec::new();
    let mut embedded = Vec::new();
    let anchors =
        Selector::parse("a[href], area[href], frame[src], iframe[src]").expect("valid selector");
    for el in doc.select(&anchors) {
        let attr = el
            .value()
            .attr("href")
            .or_else(|| el.value().attr("src"))
            .unwrap_or("");
        if let Ok(mut u) = base.join(attr.trim())
            && matches!(u.scheme(), "http" | "https" | "ftp")
        {
            u.set_fragment(None);
            links.push(u);
        }
    }
    if include_embedded {
        let media = Selector::parse(
            "img[src], img[data-src], video[src], audio[src], source[src], embed[src]",
        )
        .expect("valid selector");
        for el in doc.select(&media) {
            let v = el.value();
            let attr = v.attr("src").or_else(|| v.attr("data-src")).unwrap_or("");
            if let Ok(u) = base.join(attr.trim())
                && matches!(u.scheme(), "http" | "https")
            {
                embedded.push(u);
            }
        }
        let srcset = Selector::parse("img[srcset], source[srcset]").expect("valid selector");
        for el in doc.select(&srcset) {
            for part in el.value().attr("srcset").unwrap_or("").split(',') {
                if let Some(Ok(u)) = part.split_whitespace().next().map(|p| base.join(p)) {
                    embedded.push(u);
                }
            }
        }
    }
    (links, embedded)
}

/// Run a crawl to completion (or cancellation), streaming events to `tx`.
pub async fn crawl(
    config: GrabberConfig,
    cancel: CancelFlag,
    tx: mpsc::UnboundedSender<GrabberEvent>,
) {
    let send = |e: GrabberEvent| {
        let _ = tx.send(e);
    };
    let Ok(start) = Url::parse(config.start_url.trim()) else {
        send(GrabberEvent::PageError {
            url: config.start_url.clone(),
            error: "Invalid start URL".into(),
        });
        send(GrabberEvent::Finished {
            pages: 0,
            files: 0,
            cancelled: false,
        });
        return;
    };
    let ua = if config.user_agent.is_empty() {
        crate::DEFAULT_USER_AGENT
    } else {
        &config.user_agent
    };
    let client = match reqwest::Client::builder()
        .user_agent(ua)
        .cookie_store(true)
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            send(GrabberEvent::PageError {
                url: start.to_string(),
                error: e.to_string(),
            });
            send(GrabberEvent::Finished {
                pages: 0,
                files: 0,
                cancelled: false,
            });
            return;
        }
    };
    let auth = config
        .login
        .as_ref()
        .filter(|l| !l.username.is_empty() && l.form_url.is_none());
    if let Some(login) = &config.login
        && let Some(form_url) = &login.form_url
    {
        let fields: Vec<(String, String)> = login
            .form_fields
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.replace("{username}", &login.username)
                        .replace("{password}", &login.password),
                )
            })
            .collect();
        if let Err(e) = client.post(form_url).form(&fields).send().await {
            send(GrabberEvent::PageError {
                url: form_url.clone(),
                error: format!("Login failed: {e}"),
            });
        }
    }
    let wanted: HashSet<String> = config
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();
    let is_wanted_file = |u: &Url| {
        let ext = extension(u);
        if wanted.is_empty() {
            !HTML_EXTENSIONS.contains(&ext.as_str())
        } else {
            wanted.contains(&ext)
        }
    };
    let excluded = |u: &Url| {
        config
            .exclude
            .iter()
            .any(|x| !x.is_empty() && u.as_str().contains(x.as_str()))
    };

    let mut seen_pages: HashSet<String> = HashSet::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(Url, u32)> = VecDeque::from([(start.clone(), 0)]);
    seen_pages.insert(start.to_string());
    let mut pages = 0u32;
    let mut files = 0u32;
    let concurrency = config.concurrency.clamp(1, 16) as usize;

    while !queue.is_empty() && !cancel.is_cancelled() && pages < config.max_pages {
        // Fetch a batch of pages concurrently, then process their links in order.
        let batch: Vec<(Url, u32)> = (0..concurrency).filter_map(|_| queue.pop_front()).collect();
        let fetches = batch.into_iter().map(|(url, depth)| {
            let with_credentials = credentials_allowed(&start, &url);
            let mut req = client.get(url.clone());
            if with_credentials {
                if !config.cookies.is_empty() {
                    req = req.header(reqwest::header::COOKIE, &config.cookies);
                }
                if let Some(l) = auth {
                    req = req.basic_auth(&l.username, Some(&l.password));
                }
            }
            async move {
                let result = async {
                    let resp = req.send().await?.error_for_status()?;
                    let ctype = resp
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !ctype.is_empty() && !ctype.contains("html") {
                        return Ok::<_, reqwest::Error>((resp.url().clone(), None, ctype));
                    }
                    let final_url = resp.url().clone();
                    let text = resp.text().await?;
                    Ok((final_url, Some(text), ctype))
                }
                .await;
                (url, depth, result)
            }
        });
        for (url, depth, result) in futures_util::future::join_all(fetches).await {
            pages += 1;
            match result {
                Ok((final_url, Some(html), _)) => {
                    let (links, embedded) =
                        extract_links(&final_url, &html, config.include_embedded);
                    let mut candidates: Vec<Url> = embedded;
                    for link in links {
                        if excluded(&link) {
                            continue;
                        }
                        if is_wanted_file(&link) {
                            candidates.push(link);
                        } else if depth < config.depth
                            && HTML_EXTENSIONS.contains(&extension(&link).as_str())
                            && in_scope(config.scope, &start, &link)
                            && seen_pages.insert(link.to_string())
                        {
                            queue.push_back((link, depth + 1));
                        }
                    }
                    for file in candidates {
                        if !is_wanted_file(&file)
                            || (!config.files_any_host && !in_scope(config.scope, &start, &file))
                        {
                            continue;
                        }
                        if seen_files.insert(file.to_string()) {
                            files += 1;
                            let found = FoundFile {
                                file_name: file_name(&file),
                                url: file.to_string(),
                                size: None,
                                mime: None,
                                referer: final_url.to_string(),
                                depth,
                            };
                            send(GrabberEvent::FileFound { file: found });
                        }
                    }
                    send(GrabberEvent::PageVisited {
                        url: url.to_string(),
                        depth,
                        pages,
                        queued: queue.len() as u32,
                    });
                }
                Ok((final_url, None, ctype)) => {
                    // A "page" link turned out to be a file (e.g. download.php?id=1).
                    if seen_files.insert(final_url.to_string()) {
                        files += 1;
                        send(GrabberEvent::FileFound {
                            file: FoundFile {
                                file_name: file_name(&final_url),
                                url: final_url.to_string(),
                                size: None,
                                mime: Some(ctype),
                                referer: start.to_string(),
                                depth,
                            },
                        });
                    }
                }
                Err(e) => send(GrabberEvent::PageError {
                    url: url.to_string(),
                    error: e.to_string(),
                }),
            }
        }
    }
    let cancelled = cancel.is_cancelled();
    send(GrabberEvent::Finished {
        pages,
        files,
        cancelled,
    });
}

/// Fill in sizes/MIME types of found files with HEAD requests (bounded concurrency).
///
/// `start` scopes the cookie: files on other hosts (`files_any_host`) get a plain HEAD.
pub async fn fetch_sizes(
    files: Vec<FoundFile>,
    user_agent: &str,
    cookies: &str,
    start: Option<&Url>,
) -> Vec<FoundFile> {
    let ua = if user_agent.is_empty() {
        crate::DEFAULT_USER_AGENT
    } else {
        user_agent
    };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(ua)
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return files;
    };
    use futures_util::StreamExt;
    futures_util::stream::iter(files.into_iter().map(|mut f| {
        let client = client.clone();
        let cookies = cookies.to_string();
        let start = start.cloned();
        async move {
            let mut req = client
                .head(&f.url)
                .header(reqwest::header::REFERER, &f.referer);
            let file_url = Url::parse(&f.url).ok();
            let with_credentials = match (&start, &file_url) {
                (Some(s), Some(u)) => credentials_allowed(s, u),
                _ => false,
            };
            if with_credentials && !cookies.is_empty() {
                req = req.header(reqwest::header::COOKIE, cookies);
            }
            if let Ok(resp) = req.send().await {
                f.size = resp
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                f.mime = resp
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
            }
            f
        }
    }))
    .buffer_unordered(8)
    .collect()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_extraction_resolves_relative_and_base() {
        let base = Url::parse("https://ex.com/dir/page.html").unwrap();
        let html = r#"<html><head><base href="https://ex.com/files/"></head><body>
            <a href="a.zip">a</a> <a href="/top.pdf#frag">b</a> <a href="mailto:x@y">m</a>
            <img src="pic.png" srcset="pic-2x.png 2x, pic-3x.png 3x"></body></html>"#;
        let (links, embedded) = extract_links(&base, html, true);
        let links: Vec<String> = links.iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            vec!["https://ex.com/files/a.zip", "https://ex.com/top.pdf"]
        );
        assert_eq!(embedded.len(), 3);
    }

    #[test]
    fn scope_rules() {
        let start = Url::parse("https://www.ex.co.uk/docs/index.html").unwrap();
        let same_domain = Url::parse("https://cdn.ex.co.uk/x").unwrap();
        let sub = Url::parse("https://www.ex.co.uk/docs/a/b.html").unwrap();
        let outside = Url::parse("https://www.ex.co.uk/blog/").unwrap();
        assert!(in_scope(Scope::SameDomain, &start, &same_domain));
        assert!(!in_scope(Scope::SameHost, &start, &same_domain));
        assert!(in_scope(Scope::SubPath, &start, &sub));
        assert!(!in_scope(Scope::SubPath, &start, &outside));
    }

    #[tokio::test]
    async fn crawls_local_site() {
        use axum::{Router, response::Html as H, routing::get};
        let app = Router::new()
            .route("/", get(|| async { H(r#"<a href="/p2.html">next</a><a href="/f1.zip">f1</a>"#) }))
            .route("/p2.html", get(|| async { H(r#"<a href="/p3.html">deeper</a><a href="/f2.zip">f2</a><a href="/doc.pdf">d</a>"#) }))
            .route("/p3.html", get(|| async { H(r#"<a href="/f3.zip">f3</a>"#) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let cfg = GrabberConfig {
            start_url: format!("http://{addr}/"),
            depth: 1,
            extensions: vec!["zip".into()],
            ..Default::default()
        };
        crawl(cfg, CancelFlag::default(), tx).await;
        let mut found = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let GrabberEvent::FileFound { file } = ev {
                found.push(file.file_name);
            }
        }
        // Depth 1: start page (0) and p2 (1) are crawled, p3 is not.
        assert_eq!(found, vec!["f1.zip", "f2.zip"]);
    }

    #[test]
    fn credentials_stay_with_the_start_site() {
        let start = Url::parse("https://www.example.com/docs/").unwrap();
        let same = Url::parse("https://www.example.com/secret.zip").unwrap();
        let sub = Url::parse("https://files.example.com/secret.zip").unwrap();
        let other = Url::parse("https://cdn.unrelated.net/secret.zip").unwrap();
        let lookalike = Url::parse("https://example.com.evil.net/x").unwrap();
        assert!(credentials_allowed(&start, &same));
        assert!(credentials_allowed(&start, &sub));
        assert!(!credentials_allowed(&start, &other));
        assert!(!credentials_allowed(&start, &lookalike));
    }

    #[test]
    fn credentials_never_cross_a_bare_host_or_an_ip() {
        let ip = Url::parse("http://127.0.0.1:8080/a").unwrap();
        let other_ip = Url::parse("http://10.0.0.5:8080/a").unwrap();
        let name = Url::parse("http://localhost:8080/a").unwrap();
        assert!(credentials_allowed(&ip, &ip));
        assert!(!credentials_allowed(&ip, &other_ip));
        assert!(!credentials_allowed(&ip, &name));
        assert!(!credentials_allowed(&name, &ip));
    }

    /// `Cookie` header seen per path.
    type Seen = Arc<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>>;

    /// Records the `Cookie` header of every request; serves `home` on `/`.
    async fn cookie_recorder(home: impl FnOnce(u16) -> String) -> (u16, Seen) {
        use axum::extract::State;
        use axum::http::{HeaderMap, Uri};
        use axum::response::Html;
        async fn handle(
            State((seen, home)): State<(Seen, String)>,
            uri: Uri,
            headers: HeaderMap,
        ) -> Html<String> {
            seen.lock().unwrap().insert(
                uri.path().to_string(),
                headers
                    .get(reqwest::header::COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from),
            );
            Html(if uri.path() == "/" {
                home
            } else {
                "<html>leaf</html>".to_string()
            })
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let app = axum::Router::new()
            .fallback(handle)
            .with_state((seen.clone(), home(port)));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (port, seen)
    }

    #[tokio::test]
    async fn crawl_sends_cookies_only_to_the_start_host() {
        let (port, seen) = cookie_recorder(|port| {
            format!(
                r#"<a href="/same.html">same</a><a href="http://localhost:{port}/other.html">other</a>"#
            )
        })
        .await;
        let (tx, rx) = mpsc::unbounded_channel();
        let cfg = GrabberConfig {
            start_url: format!("http://127.0.0.1:{port}/"),
            depth: 1,
            scope: Scope::Any,
            cookies: "session=SECRET".into(),
            fetch_sizes: false,
            ..Default::default()
        };
        crawl(cfg, CancelFlag::default(), tx).await;
        drop(rx);
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.get("/").and_then(|c| c.as_deref()),
            Some("session=SECRET")
        );
        assert_eq!(
            seen.get("/same.html").and_then(|c| c.as_deref()),
            Some("session=SECRET")
        );
        assert!(
            seen.contains_key("/other.html"),
            "the third-party host must have been crawled"
        );
        assert_eq!(seen.get("/other.html").and_then(|c| c.as_deref()), None);
    }

    #[tokio::test]
    async fn fetch_sizes_sends_the_cookie_only_to_the_start_host() {
        let (port, seen) = cookie_recorder(|_| String::new()).await;
        let start = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let file = |host: &str, name: &str| FoundFile {
            url: format!("http://{host}:{port}/{name}"),
            file_name: name.into(),
            size: None,
            mime: None,
            referer: format!("http://127.0.0.1:{port}/"),
            depth: 0,
        };
        let out = fetch_sizes(
            vec![file("127.0.0.1", "own.zip"), file("localhost", "third.zip")],
            "",
            "session=SECRET",
            Some(&start),
        )
        .await;
        assert_eq!(out.len(), 2);
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.get("/own.zip").and_then(|c| c.as_deref()),
            Some("session=SECRET")
        );
        assert!(
            seen.contains_key("/third.zip"),
            "the third-party host must have been reached"
        );
        assert_eq!(seen.get("/third.zip").and_then(|c| c.as_deref()), None);
    }
}
