//! Headless Prism with the same React UI served over HTTP.

mod args;
mod auth;
mod commands;
mod ipc;
mod web;

use args::Args;
use auth::Auth;
use prism_core::manager::{Core, CoreOptions};
use prism_video::{VideoOptions, VideoService};
use serde_json::json;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
    ).init();

    let args = match Args::parse() {
        Ok(a) => a,
        Err(e) if e == "help" => {
            println!("{}", Args::usage());
            return;
        }
        Err(e) => {
            eprintln!("prism-web: {e}\n\n{}", Args::usage());
            std::process::exit(2);
        }
    };
    let data_dir = args.data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("prism-web: cannot create {}: {e}", data_dir.display());
        std::process::exit(1);
    }

    if args.warmup {
        let tools = VideoService::new(data_dir.join("tools"), VideoOptions {
            prefer_system_ffmpeg: false,
            ..Default::default()
        });
        match tools.ensure_tools(|p| tracing::info!("tool download: {p:?}")).await {
            Ok(status) => {
                println!("{}", serde_json::to_string_pretty(&status).unwrap_or_default());
            }
            Err(e) => {
                eprintln!("prism-web: tool warmup failed: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }

    let core = match Core::start(CoreOptions {
        data_dir: data_dir.clone(),
        ipc: false,
        clipboard: false,
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("prism-web: failed to start: {e:#}");
            std::process::exit(1);
        }
    };

    // Only set the folder on a fresh install; never overwrite a user's saved choice.
    if let Some(dir) = args.download_dir()
        && core.settings().downloads.default_dir == prism_core::paths::default_download_dir()
        && let Err(e) = core.update_settings(&json!({
            "downloads": {"defaultDir": dir, "useCategoryFolders": false}
        }))
    {
        eprintln!("prism-web: cannot set download folder: {e:#}");
    }

    let token = args.token();
    let auth = match Auth::new(&data_dir, args.port, args.no_auth_token, token.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("prism-web: cannot store access token: {e}");
            core.shutdown().await;
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("prism-web: cannot bind {}:{}: {e}", args.host, args.port);
            core.shutdown().await;
            std::process::exit(1);
        }
    };
    let mut ipc_auth = ipc::IpcAuth::new(args.ipc_file(), token.as_deref());
    if let Err(e) = ipc_auth.publish(args.public_port()) {
        eprintln!("prism-web: cannot write {}: {e}", ipc_auth.file().display());
        core.shutdown().await;
        std::process::exit(1);
    }
    let url = format!("http://{}:{}", args.host, args.port);
    println!("Prism Web listening on {url}");
    if token.is_none() {
        println!("IPC info: {}", ipc_auth.file().display());
    }
    if token.is_some() && !args.no_auth_token {
        println!("Access token: from PRISM_WEB_TOKEN");
    } else if let Some(token) = auth.token() {
        println!("Access token: {token}");
        println!("Saved in: {}", auth.file().display());
    } else {
        println!("WARNING: authentication is disabled; anyone who can reach {url} controls this downloader.");
    }

    let server = axum::serve(listener, web::router(core.clone(), auth, ipc_auth))
        .with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; });
    if let Err(e) = server.await {
        tracing::error!("web server stopped: {e}");
    }
    core.shutdown().await;
}
