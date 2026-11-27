use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub host: String,
    pub port: u16,
    pub no_auth_token: bool,
    pub data_dir: Option<PathBuf>,
    pub download_dir: Option<PathBuf>,
    pub ipc_file: Option<PathBuf>,
    pub warmup: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
            no_auth_token: false,
            data_dir: None,
            download_dir: None,
            ipc_file: None,
            warmup: false,
        }
    }
}

impl Args {
    pub fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args())
    }

    pub fn parse_from<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.into_iter().skip(1);
        while let Some(arg) = it.next() {
            let (key, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v.to_string())));
            let mut value = |name: &str| -> Result<String, String> {
                inline
                    .clone()
                    .or_else(|| it.next())
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match key {
                "--help" | "-h" => return Err("help".into()),
                "--host" => out.host = value(key)?,
                "--port" => {
                    out.port = value(key)?
                        .parse()
                        .map_err(|_| format!("invalid port: {}", arg))?
                }
                "--no-auth-token" => out.no_auth_token = true,
                "--data-dir" => out.data_dir = Some(PathBuf::from(value(key)?)),
                "--download-dir" => out.download_dir = Some(PathBuf::from(value(key)?)),
                "--ipc-file" => out.ipc_file = Some(PathBuf::from(value(key)?)),
                "--warmup" => out.warmup = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if out.no_auth_token && !crate::auth::is_loopback_host(&out.host) {
            return Err("--no-auth-token requires a loopback bind address".into());
        }
        Ok(out)
    }

    pub fn data_dir(&self) -> PathBuf {
        if let Some(dir) = &self.data_dir {
            return dir.clone();
        }
        if let Some(dir) = std::env::var_os("PRISM_WEB_DATA_DIR") {
            return PathBuf::from(dir);
        }
        let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join(if cfg!(windows) { "Prism Web" } else { "prism-web" })
    }

    pub fn download_dir(&self) -> Option<PathBuf> {
        self.download_dir
            .clone()
            .or_else(|| std::env::var_os("PRISM_WEB_DOWNLOAD_DIR").map(PathBuf::from))
    }

    /// Where the native host looks for `ipc.json`. Kept separate from the web data dir.
    pub fn ipc_file(&self) -> PathBuf {
        self.ipc_file
            .clone()
            .or_else(|| std::env::var_os("PRISM_WEB_IPC_FILE").map(PathBuf::from))
            .unwrap_or_else(prism_core::paths::ipc_file)
    }

    /// Port written to `ipc.json`: the published port when behind Docker NAT.
    pub fn public_port(&self) -> u16 {
        std::env::var("PRISM_WEB_PUBLIC_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(self.port)
    }

    pub fn usage() -> &'static str {
        "prism-web — headless Prism download manager\n\
         \n\
         Usage: prism-web [OPTIONS]\n\
         \n\
         Options:\n\
           --host <addr>          Bind address (default: 127.0.0.1)\n\
           --port <port>          Bind port (default: 8080)\n\
           --no-auth-token        Serve without a bearer token (trusted networks only)\n\
           --data-dir <path>      Data directory (or PRISM_WEB_DATA_DIR)\n\
           --download-dir <path>  Default save folder on a fresh install\n\
           --ipc-file <path>      Native-host discovery file (or PRISM_WEB_IPC_FILE)\n\
           --warmup               Download yt-dlp/ffmpeg/deno, then exit\n\
           -h, --help             Show this help"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let a = Args::default();
        assert_eq!(a.host, "127.0.0.1");
        assert_eq!(a.port, 8080);
        assert!(!a.no_auth_token);
        assert!(!a.warmup);
    }

    #[test]
    fn parses_flags_and_equals_form() {
        let a = parse(&[
            "prism-web",
            "--host",
            "127.0.0.1",
            "--port=9090",
            "--no-auth-token",
            "--download-dir=/downloads",
            "--ipc-file=/host-ipc/ipc.json",
            "--warmup",
        ])
        .unwrap();
        assert_eq!(a.host, "127.0.0.1");
        assert_eq!(a.port, 9090);
        assert!(a.no_auth_token);
        assert_eq!(a.download_dir, Some(PathBuf::from("/downloads")));
        assert_eq!(a.ipc_file(), PathBuf::from("/host-ipc/ipc.json"));
        assert!(a.warmup);

        // A token-protected server can still bind every interface (Docker).
        let a = parse(&["prism-web", "--host", "0.0.0.0"]).unwrap();
        assert_eq!(a.host, "0.0.0.0");
    }

    #[test]
    fn refuses_no_auth_on_non_loopback_bind() {
        assert!(parse(&["prism-web", "--host", "0.0.0.0", "--no-auth-token"]).is_err());
        assert!(parse(&["prism-web", "--host", "example.com", "--no-auth-token"]).is_err());
        assert!(parse(&["prism-web", "--no-auth-token"]).is_ok());
    }

    #[test]
    fn rejects_unknown_and_bad_port() {
        assert!(parse(&["prism-web", "--nope"]).is_err());
        assert!(parse(&["prism-web", "--port", "x"]).is_err());
        assert!(parse(&["prism-web", "--port"]).is_err());
    }
}
