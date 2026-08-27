mod client;
mod template;
mod tools;

use std::{env, fs, net::SocketAddr, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use rmcp::{
    ServiceExt,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};

use crate::{
    client::WechatClient,
    template::{BookNoteInput, BookNoteRenderer},
    tools::WechatMpMcp,
};

const DEFAULT_HTTP_BIND: &str = "127.0.0.1:8000";
const DEFAULT_MAX_HTTP_REQUEST_BYTES: usize = 1024 * 1024;

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(path) = render_command_path()? {
        print!("{}", render_note_file(&path)?);
        return Ok(());
    }
    let client = WechatClient::from_env()?;
    let handler = WechatMpMcp::new(client)?;
    match env::var("WECHAT_TRANSPORT")
        .unwrap_or_else(|_| "http".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "http" | "streamable_http" => run_http(handler).await,
        "stdio" => run_stdio(handler).await,
        other => bail!("unsupported WECHAT_TRANSPORT={other}; use http or stdio"),
    }
}

fn render_command_path() -> Result<Option<String>> {
    let mut arguments = env::args().skip(1);
    let Some(command) = arguments.next() else {
        return Ok(None);
    };
    if command != "render" {
        bail!("unknown command {command}; use `render <note.json>` or start without arguments");
    }
    let path = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: wechat-mp-mcp render <note.json>"))?;
    if arguments.next().is_some() {
        bail!("usage: wechat-mp-mcp render <note.json>");
    }
    Ok(Some(path))
}

fn render_note_file(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let input = fs::read_to_string(path)
        .with_context(|| format!("failed to read note JSON {}", path.display()))?;
    let note: BookNoteInput = serde_json::from_str(&input)
        .with_context(|| format!("failed to parse note JSON {}", path.display()))?;
    let renderer = BookNoteRenderer::new().map_err(anyhow::Error::msg)?;
    let rendered = renderer.render(&note).map_err(anyhow::Error::msg)?;
    Ok(rendered.html)
}

async fn run_stdio(handler: WechatMpMcp) -> Result<()> {
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

async fn run_http(handler: WechatMpMcp) -> Result<()> {
    let bind = env::var("WECHAT_HTTP_BIND").unwrap_or_else(|_| DEFAULT_HTTP_BIND.into());
    let address: SocketAddr = bind.parse().with_context(|| {
        format!("WECHAT_HTTP_BIND must be an IP:port socket address; got {bind}")
    })?;
    let allowed_hosts = allowed_hosts(address)?;
    let allowed_origins = comma_separated_env("WECHAT_HTTP_ALLOWED_ORIGINS");
    let max_request_body_bytes = env::var("WECHAT_HTTP_MAX_REQUEST_BYTES")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("WECHAT_HTTP_MAX_REQUEST_BYTES must be a positive integer")?
        .unwrap_or(DEFAULT_MAX_HTTP_REQUEST_BYTES);
    if max_request_body_bytes == 0 {
        bail!("WECHAT_HTTP_MAX_REQUEST_BYTES must be greater than zero");
    }

    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts)
        .with_max_request_body_bytes(max_request_body_bytes);
    if !allowed_origins.is_empty() {
        config = config.with_allowed_origins(allowed_origins);
    }
    let service: StreamableHttpService<WechatMpMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
    let protected = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(we_user_auth));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(protected);

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind WeChat MCP HTTP server to {address}"))?;
    eprintln!("wechat-mp-mcp listening on http://{address}/mcp");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn we_user_auth(request: Request<Body>, next: Next) -> Response {
    if we_user_matches(request.headers().get("we-user")) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

fn we_user_matches(value: Option<&axum::http::HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "tobeurself")
}

fn allowed_hosts(address: SocketAddr) -> Result<Vec<String>> {
    allowed_hosts_with_config(address, comma_separated_env("WECHAT_HTTP_ALLOWED_HOSTS"))
}

fn allowed_hosts_with_config(address: SocketAddr, configured: Vec<String>) -> Result<Vec<String>> {
    if !configured.is_empty() {
        return Ok(configured);
    }
    if !address.ip().is_loopback() {
        bail!(
            "WECHAT_HTTP_ALLOWED_HOSTS is required when WECHAT_HTTP_BIND is not loopback; set it to the public IP/domain optionally followed by :port"
        );
    }
    Ok(vec!["localhost".into(), "127.0.0.1".into(), "::1".into()])
}

fn comma_separated_env(name: &str) -> Vec<String> {
    env::var(name)
        .ok()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn we_user_auth_requires_exact_header_value() {
        let valid = axum::http::HeaderValue::from_static("tobeurself");
        let wrong = axum::http::HeaderValue::from_static("other");
        assert!(we_user_matches(Some(&valid)));
        assert!(!we_user_matches(Some(&wrong)));
        assert!(!we_user_matches(None));
    }

    #[test]
    fn remote_bind_requires_explicit_allowed_hosts() {
        let address: SocketAddr = "0.0.0.0:8000".parse().unwrap();
        assert!(
            allowed_hosts_with_config(address, vec![])
                .unwrap_err()
                .to_string()
                .contains("required")
        );
    }

    #[test]
    fn render_command_reads_note_json_without_service_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.json");
        fs::write(
            &path,
            r#"{
              "title":"系统思维读书笔记",
              "book_name":"系统之美",
              "author":"德内拉·梅多斯",
              "summary":"结构决定行为。",
              "core_points":[{"title":"系统思维","content":"先看结构。"}],
              "thoughts":"从整体开始。",
              "actions":["画出反馈回路"]
            }"#,
        )
        .unwrap();
        let html = render_note_file(&path).unwrap();
        assert!(html.contains("《系统之美》"));
    }
}
