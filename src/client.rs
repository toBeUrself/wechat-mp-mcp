use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, Method, Response, multipart};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_048_576;
const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;
const TOKEN_EARLY_EXPIRY_SECONDS: u64 = 300;

#[derive(Clone)]
pub struct WechatClient {
    base_url: String,
    app_id: String,
    app_secret: String,
    client: Client,
    token: Arc<Mutex<Option<CachedToken>>>,
    media_root: Option<PathBuf>,
    max_response_bytes: usize,
}

struct CachedToken {
    value: String,
    valid_until: Instant,
}

#[derive(Debug)]
struct MediaFile {
    bytes: Vec<u8>,
    filename: String,
    mime: &'static str,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
    rid: Option<String>,
}

impl WechatClient {
    pub fn from_env() -> Result<Self> {
        let app_id = required_env("WECHAT_APP_ID")?;
        let app_secret = required_env("WECHAT_APP_SECRET")?;
        let max_response_bytes = env::var("WECHAT_MAX_RESPONSE_BYTES")
            .ok()
            .map(|value| value.parse::<usize>())
            .transpose()
            .context("WECHAT_MAX_RESPONSE_BYTES must be a positive integer")?
            .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES);
        if max_response_bytes == 0 {
            bail!("WECHAT_MAX_RESPONSE_BYTES must be greater than zero");
        }

        let media_root = env::var("WECHAT_MEDIA_ROOT")
            .ok()
            .map(|value| canonical_media_root(Path::new(&value)))
            .transpose()?;
        Self::build(
            "https://api.weixin.qq.com",
            app_id,
            app_secret,
            media_root,
            max_response_bytes,
        )
    }

    fn build(
        base_url: &str,
        app_id: String,
        app_secret: String,
        media_root: Option<PathBuf>,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("wechat-mp-mcp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            app_id,
            app_secret,
            client,
            token: Arc::new(Mutex::new(None)),
            media_root,
            max_response_bytes,
        })
    }

    #[cfg(test)]
    pub fn for_test(
        base_url: &str,
        media_root: Option<PathBuf>,
        max_response_bytes: usize,
    ) -> Self {
        Self::build(
            base_url,
            "test-app-id".into(),
            "test-app-secret".into(),
            media_root,
            max_response_bytes,
        )
        .unwrap()
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.send_json(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.send_json(Method::POST, path, Some(body)).await
    }

    pub async fn upload_cover(&self, file_path: &str) -> Result<Value> {
        let media = self.read_cover(file_path).await?;
        let token = self.access_token().await?;
        let first = self
            .send_cover_once(&token, &media.bytes, &media.filename, media.mime)
            .await;
        match first {
            Err(error) if is_token_error(&error) => {
                self.invalidate_token().await;
                let token = self.access_token().await?;
                self.send_cover_once(&token, &media.bytes, &media.filename, media.mime)
                    .await
            }
            result => result,
        }
    }

    async fn send_json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let token = self.access_token().await?;
        let first = self
            .send_json_once(method.clone(), path, body.clone(), &token)
            .await;
        match first {
            Err(error) if is_token_error(&error) => {
                self.invalidate_token().await;
                let token = self.access_token().await?;
                self.send_json_once(method, path, body, &token).await
            }
            result => result,
        }
    }

    async fn send_json_once(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: &str,
    ) -> Result<Value> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .client
            .request(method, &url)
            .query(&[("access_token", token)]);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("failed to connect to WeChat API at {}", self.base_url))?;
        self.decode_api_response(response).await
    }

    async fn send_cover_once(
        &self,
        token: &str,
        bytes: &[u8],
        filename: &str,
        mime: &str,
    ) -> Result<Value> {
        let part = multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_owned())
            .mime_str(mime)?;
        let form = multipart::Form::new().part("media", part);
        let response = self
            .client
            .post(format!("{}/cgi-bin/material/add_material", self.base_url))
            .query(&[("access_token", token), ("type", "image")])
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("failed to connect to WeChat API at {}", self.base_url))?;
        self.decode_api_response(response).await
    }

    async fn access_token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| token.valid_until > Instant::now())
        {
            return Ok(token.value.clone());
        }

        let response = self
            .client
            .post(format!("{}/cgi-bin/stable_token", self.base_url))
            .json(&json!({
                "grant_type": "client_credential",
                "appid": self.app_id,
                "secret": self.app_secret,
                "force_refresh": false
            }))
            .send()
            .await
            .with_context(|| {
                format!("failed to obtain WeChat stable token at {}", self.base_url)
            })?;
        let value = self.decode_json_response(response).await?;
        let token_response: TokenResponse = serde_json::from_value(value.clone())
            .context("WeChat stable-token response has an unexpected shape")?;
        if let Some(code) = token_response.errcode.filter(|code| *code != 0) {
            return Err(wechat_error(
                code,
                token_response.errmsg.as_deref(),
                token_response.rid.as_deref(),
            ));
        }
        let value = token_response
            .access_token
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("WeChat stable-token response is missing access_token"))?;
        let expires_in = token_response
            .expires_in
            .ok_or_else(|| anyhow!("WeChat stable-token response is missing expires_in"))?;
        let cache_seconds = expires_in.saturating_sub(TOKEN_EARLY_EXPIRY_SECONDS).max(1);
        *cached = Some(CachedToken {
            value: value.clone(),
            valid_until: Instant::now() + Duration::from_secs(cache_seconds),
        });
        Ok(value)
    }

    async fn invalidate_token(&self) {
        *self.token.lock().await = None;
    }

    async fn decode_api_response(&self, response: Response) -> Result<Value> {
        let value = self.decode_json_response(response).await?;
        if let Some(code) = value.get("errcode").and_then(Value::as_i64)
            && code != 0
        {
            return Err(wechat_error(
                code,
                value.get("errmsg").and_then(Value::as_str),
                value.get("rid").and_then(Value::as_str),
            ));
        }
        Ok(value)
    }

    async fn decode_json_response(&self, response: Response) -> Result<Value> {
        let status = response.status();
        if let Some(length) = response.content_length()
            && length > self.max_response_bytes as u64
        {
            bail!(
                "WeChat response is {length} bytes, exceeding the configured {} byte limit",
                self.max_response_bytes
            );
        }
        let bytes = response.bytes().await?;
        if bytes.len() > self.max_response_bytes {
            bail!(
                "WeChat response exceeds the configured {} byte limit",
                self.max_response_bytes
            );
        }
        let value: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        if !status.is_success() {
            let detail = serde_json::to_string(&value).unwrap_or_else(|_| "unknown error".into());
            return Err(anyhow!("WeChat API returned HTTP {status}: {detail}"));
        }
        Ok(value)
    }

    async fn read_cover(&self, file_path: &str) -> Result<MediaFile> {
        let root = self.media_root.as_ref().ok_or_else(|| {
            anyhow!("file uploads are disabled; set WECHAT_MEDIA_ROOT to an allowed directory")
        })?;
        if file_path.trim().is_empty() {
            bail!("file_path must not be empty");
        }
        let requested = Path::new(file_path);
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            root.join(requested)
        };
        let canonical = tokio::fs::canonicalize(&joined)
            .await
            .with_context(|| format!("failed to resolve media file {}", joined.display()))?;
        if !canonical.starts_with(root) {
            bail!(
                "media file {} is outside WECHAT_MEDIA_ROOT {}",
                canonical.display(),
                root.display()
            );
        }
        let metadata = tokio::fs::metadata(&canonical).await?;
        if !metadata.is_file() {
            bail!("media path {} is not a regular file", canonical.display());
        }
        if metadata.len() == 0 || metadata.len() > MAX_COVER_BYTES {
            bail!("cover image must be between 1 byte and 10 MiB");
        }

        let extension = canonical
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mime = match extension.as_str() {
            "bmp" => "image/bmp",
            "gif" => "image/gif",
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            _ => bail!("cover image must use bmp, gif, jpg, jpeg, or png format"),
        };
        let filename = canonical
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("media filename is not valid UTF-8"))?
            .to_owned();
        let bytes = tokio::fs::read(&canonical).await?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_COVER_BYTES {
            bail!("cover image must be between 1 byte and 10 MiB");
        }
        Ok(MediaFile {
            bytes,
            filename,
            mime,
        })
    }
}

fn canonical_media_root(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve WECHAT_MEDIA_ROOT {}", path.display()))?;
    if !canonical.is_dir() {
        bail!(
            "WECHAT_MEDIA_ROOT {} is not a directory",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing required environment variable {name}"))
}

pub fn env_flag(name: &str) -> bool {
    env::var(name)
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn wechat_error(code: i64, message: Option<&str>, rid: Option<&str>) -> anyhow::Error {
    let message = message.unwrap_or("unknown error");
    match rid {
        Some(rid) => anyhow!("WeChat API error {code}: {message}; rid={rid}"),
        None => anyhow!("WeChat API error {code}: {message}"),
    }
}

fn is_token_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("WeChat API error 40014:") || message.contains("WeChat API error 42001:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Mock;
    use httpmock::prelude::*;
    use tempfile::tempdir;

    fn token_mock(server: &MockServer) -> Mock<'_> {
        server.mock(|when, then| {
            when.method(POST)
                .path("/cgi-bin/stable_token")
                .json_body(json!({
                    "grant_type": "client_credential",
                    "appid": "test-app-id",
                    "secret": "test-app-secret",
                    "force_refresh": false
                }));
            then.status(200)
                .json_body(json!({"access_token": "secret-token", "expires_in": 7200}));
        })
    }

    #[tokio::test]
    async fn caches_stable_token_between_requests() {
        let server = MockServer::start();
        let token = token_mock(&server);
        let api = server.mock(|when, then| {
            when.method(GET)
                .path("/cgi-bin/draft/count")
                .query_param("access_token", "secret-token");
            then.status(200).json_body(json!({"total_count": 3}));
        });
        let client = WechatClient::for_test(&server.base_url(), None, 4096);

        client.get("/cgi-bin/draft/count").await.unwrap();
        client.get("/cgi-bin/draft/count").await.unwrap();

        token.assert_calls(1);
        api.assert_calls(2);
    }

    #[tokio::test]
    async fn maps_wechat_error_envelopes() {
        let server = MockServer::start();
        let _token = token_mock(&server);
        let _api = server.mock(|when, then| {
            when.method(GET).path("/cgi-bin/draft/count");
            then.status(200).json_body(json!({
                "errcode": 48001,
                "errmsg": "api unauthorized",
                "rid": "request-id"
            }));
        });
        let client = WechatClient::for_test(&server.base_url(), None, 4096);
        let error = client.get("/cgi-bin/draft/count").await.unwrap_err();
        assert!(error.to_string().contains("48001"));
        assert!(error.to_string().contains("request-id"));
        assert!(!error.to_string().contains("secret-token"));
    }

    #[tokio::test]
    async fn rejects_media_outside_configured_root() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("cover.png");
        std::fs::write(&outside_file, b"image").unwrap();
        let client = WechatClient::for_test(
            "http://127.0.0.1:1",
            Some(root.path().canonicalize().unwrap()),
            4096,
        );
        let error = client
            .read_cover(outside_file.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("outside WECHAT_MEDIA_ROOT"));
    }

    #[test]
    fn recognizes_only_supported_token_errors() {
        assert!(is_token_error(&anyhow!(
            "WeChat API error 40014: invalid token"
        )));
        assert!(is_token_error(&anyhow!(
            "WeChat API error 42001: expired token"
        )));
        assert!(!is_token_error(&anyhow!(
            "WeChat API error 48001: unauthorized"
        )));
    }
}
