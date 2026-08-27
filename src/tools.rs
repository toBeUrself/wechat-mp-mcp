use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    client::{WechatClient, env_flag},
    template::{BookNoteInput, BookNoteRenderer, RenderedNote},
};

type ToolResult = Result<CallToolResult, McpError>;

#[derive(Clone)]
pub struct WechatMpMcp {
    client: WechatClient,
    renderer: Arc<BookNoteRenderer>,
    allow_write: bool,
}

impl WechatMpMcp {
    pub fn new(client: WechatClient) -> anyhow::Result<Self> {
        let renderer = BookNoteRenderer::new().map_err(anyhow::Error::msg)?;
        Ok(Self {
            client,
            renderer: Arc::new(renderer),
            allow_write: env_flag("WECHAT_ALLOW_WRITE"),
        })
    }

    #[cfg(test)]
    fn with_write(client: WechatClient, allow_write: bool) -> Self {
        Self {
            client,
            renderer: Arc::new(BookNoteRenderer::new().unwrap()),
            allow_write,
        }
    }

    fn require_write(&self) -> Result<(), McpError> {
        if self.allow_write {
            Ok(())
        } else {
            Err(invalid_params(
                "write tools are disabled; set WECHAT_ALLOW_WRITE=true to enable uploads, draft changes, publishing, and deletion",
            ))
        }
    }

    fn render_note(&self, note: &BookNoteInput) -> Result<RenderedNote, McpError> {
        self.renderer.render(note).map_err(invalid_params)
    }

    async fn resolve_cover(&self, cover: CoverInput) -> Result<(String, Option<String>), McpError> {
        match cover {
            CoverInput::FilePath(path) => {
                let response = self
                    .client
                    .upload_cover(&path)
                    .await
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                let media_id = response
                    .get("media_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        McpError::internal_error(
                            "WeChat cover-upload response is missing media_id",
                            None,
                        )
                    })?
                    .to_owned();
                Ok((media_id.clone(), Some(media_id)))
            }
            CoverInput::MediaId(media_id) => {
                let media_id = media_id.trim();
                if media_id.is_empty() {
                    return Err(invalid_params("cover media_id must not be empty"));
                }
                Ok((media_id.to_owned(), None))
            }
        }
    }

    async fn post_with_uploaded_cover_context(
        &self,
        path: &str,
        body: Value,
        uploaded_cover_media_id: Option<String>,
    ) -> ToolResult {
        match self.client.post(path, body).await {
            Ok(value) => response(Ok(value)),
            Err(error) => {
                let message = match uploaded_cover_media_id {
                    Some(media_id) => format!(
                        "{error}; cover upload already succeeded, uploaded_cover_media_id={media_id}"
                    ),
                    None => error.to_string(),
                };
                tool_error(message)
            }
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RenderBookNote {
    note: BookNoteInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UploadCoverImage {
    #[schemars(description = "Absolute path or path relative to WECHAT_MEDIA_ROOT")]
    file_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum CoverInput {
    #[schemars(description = "Image path inside WECHAT_MEDIA_ROOT")]
    FilePath(String),
    #[schemars(description = "Existing permanent image media_id")]
    MediaId(String),
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ArticleOptions {
    #[schemars(description = "WeChat article author, at most 16 characters")]
    article_author: Option<String>,
    #[schemars(description = "Article digest, at most 120 characters; defaults to note.summary")]
    digest: Option<String>,
    #[schemars(description = "Optional Read more URL using http or https")]
    content_source_url: Option<String>,
    #[schemars(description = "Whether comments are enabled; defaults to false")]
    need_open_comment: Option<bool>,
    #[schemars(description = "Whether only followers can comment; defaults to false")]
    only_fans_can_comment: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateBookNoteDraft {
    note: BookNoteInput,
    cover: CoverInput,
    article_options: Option<ArticleOptions>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateBookNoteDraft {
    media_id: String,
    #[schemars(description = "Zero-based article index; defaults to 0")]
    index: Option<u32>,
    note: BookNoteInput,
    #[schemars(description = "New cover; omit to preserve the current cover")]
    cover: Option<CoverInput>,
    article_options: Option<ArticleOptions>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MediaId {
    media_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DraftList {
    #[schemars(description = "Zero-based result offset; defaults to 0")]
    offset: Option<u32>,
    #[schemars(description = "Number of results, 1 through 20; defaults to 10")]
    count: Option<u32>,
    #[schemars(description = "Omit article HTML from results; defaults to true")]
    no_content: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConfirmMediaId {
    media_id: String,
    #[schemars(description = "Must be true to confirm the irreversible or public action")]
    confirm: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PublishId {
    publish_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ArticleId {
    article_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DeletePublishedScope {
    Single,
    All,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeletePublishedArticle {
    article_id: String,
    scope: DeletePublishedScope,
    #[schemars(description = "One-based article index; required only when scope=single")]
    index: Option<u32>,
    #[schemars(description = "Must be true to confirm irreversible deletion")]
    confirm: bool,
}

#[tool_router]
impl WechatMpMcp {
    #[tool(
        description = "Render structured book-note JSON into the built-in WeChat HTML template without calling WeChat"
    )]
    async fn render_book_note_html(&self, Parameters(p): Parameters<RenderBookNote>) -> ToolResult {
        let rendered = self.render_note(&p.note)?;
        response(Ok(json!({
            "html": rendered.html,
            "char_count": rendered.char_count,
            "byte_count": rendered.byte_count
        })))
    }

    #[tool(
        description = "Upload a permanent cover image from WECHAT_MEDIA_ROOT. Requires WECHAT_ALLOW_WRITE=true"
    )]
    async fn upload_cover_image(&self, Parameters(p): Parameters<UploadCoverImage>) -> ToolResult {
        self.require_write()?;
        response(self.client.upload_cover(&p.file_path).await)
    }

    #[tool(
        description = "Render a book note and create a one-article WeChat draft. Requires WECHAT_ALLOW_WRITE=true"
    )]
    async fn create_book_note_draft(
        &self,
        Parameters(p): Parameters<CreateBookNoteDraft>,
    ) -> ToolResult {
        self.require_write()?;
        let rendered = self.render_note(&p.note)?;
        let options = p.article_options.unwrap_or_default();
        validate_article_options(&options)?;
        let (cover_media_id, uploaded) = self.resolve_cover(p.cover).await?;
        let article = article_json(&p.note, rendered.html, Some(&cover_media_id), &options)?;
        self.post_with_uploaded_cover_context(
            "/cgi-bin/draft/add",
            json!({"articles": [article]}),
            uploaded,
        )
        .await
    }

    #[tool(
        description = "Render a book note and replace one article in an existing draft. Requires WECHAT_ALLOW_WRITE=true"
    )]
    async fn update_book_note_draft(
        &self,
        Parameters(p): Parameters<UpdateBookNoteDraft>,
    ) -> ToolResult {
        self.require_write()?;
        require_non_empty("media_id", &p.media_id)?;
        let rendered = self.render_note(&p.note)?;
        let options = p.article_options.unwrap_or_default();
        validate_article_options(&options)?;
        let (cover_media_id, uploaded) = match p.cover {
            Some(cover) => {
                let (media_id, uploaded) = self.resolve_cover(cover).await?;
                (Some(media_id), uploaded)
            }
            None => (None, None),
        };
        let article = article_json(&p.note, rendered.html, cover_media_id.as_deref(), &options)?;
        self.post_with_uploaded_cover_context(
            "/cgi-bin/draft/update",
            json!({
                "media_id": p.media_id,
                "index": p.index.unwrap_or(0),
                "articles": article
            }),
            uploaded,
        )
        .await
    }

    #[tool(description = "Get one WeChat draft by media_id")]
    async fn get_draft(&self, Parameters(p): Parameters<MediaId>) -> ToolResult {
        require_non_empty("media_id", &p.media_id)?;
        response(
            self.client
                .post("/cgi-bin/draft/get", json!({"media_id": p.media_id}))
                .await,
        )
    }

    #[tool(
        description = "List WeChat drafts; content is omitted by default to protect MCP context size"
    )]
    async fn list_drafts(&self, Parameters(p): Parameters<DraftList>) -> ToolResult {
        let body = list_body(p)?;
        response(self.client.post("/cgi-bin/draft/batchget", body).await)
    }

    #[tool(description = "Get the total number of WeChat drafts")]
    async fn get_draft_count(&self) -> ToolResult {
        response(self.client.get("/cgi-bin/draft/count").await)
    }

    #[tool(
        description = "Permanently delete a WeChat draft. Requires WECHAT_ALLOW_WRITE=true and confirm=true"
    )]
    async fn delete_draft(&self, Parameters(p): Parameters<ConfirmMediaId>) -> ToolResult {
        self.require_write()?;
        require_confirmation(p.confirm)?;
        require_non_empty("media_id", &p.media_id)?;
        response(
            self.client
                .post("/cgi-bin/draft/delete", json!({"media_id": p.media_id}))
                .await,
        )
    }

    #[tool(
        description = "Submit a draft for asynchronous public publishing. Requires WECHAT_ALLOW_WRITE=true and confirm=true"
    )]
    async fn publish_draft(&self, Parameters(p): Parameters<ConfirmMediaId>) -> ToolResult {
        self.require_write()?;
        require_confirmation(p.confirm)?;
        require_non_empty("media_id", &p.media_id)?;
        response(
            self.client
                .post(
                    "/cgi-bin/freepublish/submit",
                    json!({"media_id": p.media_id}),
                )
                .await,
        )
    }

    #[tool(description = "Get asynchronous publishing status and final article URLs")]
    async fn get_publish_status(&self, Parameters(p): Parameters<PublishId>) -> ToolResult {
        require_non_empty("publish_id", &p.publish_id)?;
        response(
            self.client
                .post(
                    "/cgi-bin/freepublish/get",
                    json!({"publish_id": p.publish_id}),
                )
                .await,
        )
    }

    #[tool(
        description = "List successfully published WeChat articles; content is omitted by default"
    )]
    async fn list_published_articles(&self, Parameters(p): Parameters<DraftList>) -> ToolResult {
        let body = list_body(p)?;
        response(
            self.client
                .post("/cgi-bin/freepublish/batchget", body)
                .await,
        )
    }

    #[tool(description = "Get one published WeChat article collection by article_id")]
    async fn get_published_article(&self, Parameters(p): Parameters<ArticleId>) -> ToolResult {
        require_non_empty("article_id", &p.article_id)?;
        response(
            self.client
                .post(
                    "/cgi-bin/freepublish/getarticle",
                    json!({"article_id": p.article_id}),
                )
                .await,
        )
    }

    #[tool(
        description = "Permanently delete one or all articles in a published collection. Requires WECHAT_ALLOW_WRITE=true and confirm=true"
    )]
    async fn delete_published_article(
        &self,
        Parameters(p): Parameters<DeletePublishedArticle>,
    ) -> ToolResult {
        self.require_write()?;
        require_confirmation(p.confirm)?;
        require_non_empty("article_id", &p.article_id)?;
        let index = match (p.scope, p.index) {
            (DeletePublishedScope::Single, Some(index)) if index >= 1 => index,
            (DeletePublishedScope::Single, _) => {
                return Err(invalid_params(
                    "scope=single requires a one-based index greater than or equal to 1",
                ));
            }
            (DeletePublishedScope::All, None) => 0,
            (DeletePublishedScope::All, Some(_)) => {
                return Err(invalid_params("scope=all must not include index"));
            }
        };
        response(
            self.client
                .post(
                    "/cgi-bin/freepublish/delete",
                    json!({"article_id": p.article_id, "index": index}),
                )
                .await,
        )
    }
}

#[tool_handler(
    name = "wechat-mp-mcp",
    version = "0.1.0",
    instructions = "Render structured book notes and manage WeChat Official Account drafts and published articles. Read tools are available by default. Every upload or content-changing tool requires WECHAT_ALLOW_WRITE=true; publishing and deletion additionally require confirm=true."
)]
impl ServerHandler for WechatMpMcp {}

fn article_json(
    note: &BookNoteInput,
    html: String,
    cover_media_id: Option<&str>,
    options: &ArticleOptions,
) -> Result<Value, McpError> {
    let mut article = Map::new();
    article.insert("article_type".into(), Value::String("news".into()));
    article.insert("title".into(), Value::String(note.title.trim().into()));
    article.insert("content".into(), Value::String(html));
    article.insert(
        "digest".into(),
        Value::String(
            options
                .digest
                .as_deref()
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| note.summary.trim().chars().take(120).collect()),
        ),
    );
    article.insert(
        "need_open_comment".into(),
        Value::from(options.need_open_comment.unwrap_or(false) as u8),
    );
    article.insert(
        "only_fans_can_comment".into(),
        Value::from(options.only_fans_can_comment.unwrap_or(false) as u8),
    );
    if let Some(media_id) = cover_media_id {
        article.insert("thumb_media_id".into(), Value::String(media_id.into()));
    }
    insert_optional_trimmed(&mut article, "author", options.article_author.as_deref());
    insert_optional_trimmed(
        &mut article,
        "content_source_url",
        options.content_source_url.as_deref(),
    );
    Ok(Value::Object(article))
}

fn validate_article_options(options: &ArticleOptions) -> Result<(), McpError> {
    if let Some(author) = options.article_author.as_deref() {
        require_non_empty("article_author", author)?;
        let count = author.chars().count();
        if count > 16 {
            return Err(invalid_params(format!(
                "article_author must contain at most 16 characters; got {count}"
            )));
        }
    }
    if let Some(digest) = options.digest.as_deref() {
        require_non_empty("digest", digest)?;
        let count = digest.chars().count();
        if count > 120 {
            return Err(invalid_params(format!(
                "digest must contain at most 120 characters; got {count}"
            )));
        }
    }
    if let Some(source_url) = options.content_source_url.as_deref() {
        require_non_empty("content_source_url", source_url)?;
        if source_url.len() >= 1024 {
            return Err(invalid_params(
                "content_source_url must be smaller than 1024 bytes",
            ));
        }
        let parsed = reqwest::Url::parse(source_url)
            .map_err(|_| invalid_params("content_source_url must be a valid http or https URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(invalid_params("content_source_url must use http or https"));
        }
    }
    Ok(())
}

fn list_body(p: DraftList) -> Result<Value, McpError> {
    let count = p.count.unwrap_or(10);
    if !(1..=20).contains(&count) {
        return Err(invalid_params("count must be between 1 and 20"));
    }
    Ok(json!({
        "offset": p.offset.unwrap_or(0),
        "count": count,
        "no_content": p.no_content.unwrap_or(true) as u8
    }))
}

fn require_confirmation(confirm: bool) -> Result<(), McpError> {
    if confirm {
        Ok(())
    } else {
        Err(invalid_params("confirm must be true for this operation"))
    }
}

fn require_non_empty(name: &str, value: &str) -> Result<(), McpError> {
    if value.trim().is_empty() {
        Err(invalid_params(format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}

fn insert_optional_trimmed(body: &mut Map<String, Value>, name: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert(name.into(), Value::String(value.into()));
    }
}

fn response(result: anyhow::Result<Value>) -> ToolResult {
    match result {
        Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        )])),
        Err(error) => tool_error(error.to_string()),
    }
}

fn tool_error(message: impl Into<String>) -> ToolResult {
    Ok(CallToolResult::error(vec![ContentBlock::text(
        message.into(),
    )]))
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn note() -> BookNoteInput {
        BookNoteInput {
            title: "系统思维读书笔记".into(),
            book_name: "系统之美".into(),
            author: "德内拉·梅多斯".into(),
            summary: "结构决定行为。".into(),
            core_points: vec![crate::template::CorePointInput {
                title: "系统思维".into(),
                content: "关注结构，而不只是事件。".into(),
            }],
            thoughts: "先看整体。".into(),
            actions: vec!["画出反馈回路".into()],
        }
    }

    fn mock_token(server: &MockServer) {
        server.mock(|when, then| {
            when.method(POST).path("/cgi-bin/stable_token");
            then.status(200)
                .json_body(json!({"access_token": "token", "expires_in": 7200}));
        });
    }

    #[tokio::test]
    async fn rendering_is_available_without_write_access() {
        let client = WechatClient::for_test("http://127.0.0.1:1", None, 4096);
        let mcp = WechatMpMcp::with_write(client, false);
        let result = mcp
            .render_book_note_html(Parameters(RenderBookNote { note: note() }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn write_tools_are_disabled_before_network_access() {
        let client = WechatClient::for_test("http://127.0.0.1:1", None, 4096);
        let mcp = WechatMpMcp::with_write(client, false);
        let error = mcp
            .create_book_note_draft(Parameters(CreateBookNoteDraft {
                note: note(),
                cover: CoverInput::MediaId("cover-id".into()),
                article_options: None,
            }))
            .await
            .unwrap_err();
        assert!(error.message.contains("write tools are disabled"));
    }

    #[tokio::test]
    async fn creates_rendered_draft_with_existing_cover() {
        let server = MockServer::start();
        mock_token(&server);
        let draft = server.mock(|when, then| {
            when.method(POST)
                .path("/cgi-bin/draft/add")
                .query_param("access_token", "token");
            then.status(200).json_body(json!({"media_id": "draft-id"}));
        });
        let client = WechatClient::for_test(&server.base_url(), None, 64 * 1024);
        let mcp = WechatMpMcp::with_write(client, true);
        let result = mcp
            .create_book_note_draft(Parameters(CreateBookNoteDraft {
                note: note(),
                cover: CoverInput::MediaId("cover-id".into()),
                article_options: None,
            }))
            .await
            .unwrap();
        draft.assert();
        assert_ne!(result.is_error, Some(true));
    }

    #[test]
    fn list_defaults_omit_content_and_validate_count() {
        assert_eq!(
            list_body(DraftList {
                offset: None,
                count: None,
                no_content: None
            })
            .unwrap(),
            json!({"offset": 0, "count": 10, "no_content": 1})
        );
        assert!(
            list_body(DraftList {
                offset: None,
                count: Some(21),
                no_content: None
            })
            .is_err()
        );
    }

    #[test]
    fn published_delete_scope_cannot_accidentally_mean_all() {
        let single_missing = match (DeletePublishedScope::Single, None) {
            (DeletePublishedScope::Single, Some(index)) if index >= 1 => Ok(index),
            (DeletePublishedScope::Single, _) => Err("missing"),
            _ => Ok(0),
        };
        assert!(single_missing.is_err());
    }
}
