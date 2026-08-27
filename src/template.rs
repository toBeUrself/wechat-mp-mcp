use handlebars::Handlebars;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 32;
const MAX_CONTENT_CHARS: usize = 20_000;
const MAX_CONTENT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BookNoteInput {
    #[schemars(description = "WeChat article title, at most 32 characters")]
    pub title: String,
    #[schemars(description = "Book name shown in the article body")]
    pub book_name: String,
    #[schemars(description = "Book author, distinct from the WeChat article author")]
    pub author: String,
    #[schemars(description = "One-sentence summary; plain text")]
    pub summary: String,
    #[schemars(description = "Ordered core ideas; at least one item")]
    pub core_points: Vec<CorePointInput>,
    #[schemars(description = "Personal reflections; plain text")]
    pub thoughts: String,
    #[schemars(description = "Concrete actions rendered as a checklist; at least one item")]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CorePointInput {
    pub title: String,
    #[schemars(description = "Core-point explanation; plain text")]
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct RenderedNote {
    pub html: String,
    pub char_count: usize,
    pub byte_count: usize,
}

pub struct BookNoteRenderer {
    handlebars: Handlebars<'static>,
}

impl BookNoteRenderer {
    pub fn new() -> Result<Self, String> {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        handlebars
            .register_template_string("book_note", include_str!("../templates/book_note.html.hbs"))
            .map_err(|error| format!("failed to compile book-note template: {error}"))?;
        Ok(Self { handlebars })
    }

    pub fn render(&self, note: &BookNoteInput) -> Result<RenderedNote, String> {
        validate_non_empty("title", &note.title)?;
        validate_non_empty("book_name", &note.book_name)?;
        validate_non_empty("author", &note.author)?;
        validate_non_empty("summary", &note.summary)?;
        validate_non_empty("thoughts", &note.thoughts)?;

        let title_chars = note.title.chars().count();
        if title_chars > MAX_TITLE_CHARS {
            return Err(format!(
                "title must contain at most {MAX_TITLE_CHARS} characters; got {title_chars}"
            ));
        }
        if note.core_points.is_empty() {
            return Err("core_points must contain at least one item".into());
        }
        if note.actions.is_empty() {
            return Err("actions must contain at least one item".into());
        }

        let core_points = note
            .core_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                validate_non_empty("core_points[].title", &point.title)?;
                validate_non_empty("core_points[].content", &point.content)?;
                Ok(RenderCorePoint {
                    number: format!("{:02}", index + 1),
                    title: point.title.trim(),
                    content_html: escape_multiline(&point.content),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let actions = note
            .actions
            .iter()
            .map(|action| {
                validate_non_empty("actions[]", action)?;
                Ok(RenderAction {
                    text_html: escape_multiline(action),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let context = RenderContext {
            book_name: note.book_name.trim(),
            author: note.author.trim(),
            summary_html: escape_multiline(&note.summary),
            core_points,
            thoughts_html: escape_multiline(&note.thoughts),
            actions,
        };
        let html = self
            .handlebars
            .render("book_note", &context)
            .map_err(|error| format!("failed to render book-note template: {error}"))?;
        let char_count = html.chars().count();
        let byte_count = html.len();
        if char_count >= MAX_CONTENT_CHARS {
            return Err(format!(
                "rendered HTML must contain fewer than {MAX_CONTENT_CHARS} characters; got {char_count}"
            ));
        }
        if byte_count >= MAX_CONTENT_BYTES {
            return Err(format!(
                "rendered HTML must be smaller than {MAX_CONTENT_BYTES} bytes; got {byte_count}"
            ));
        }
        Ok(RenderedNote {
            html,
            char_count,
            byte_count,
        })
    }
}

#[derive(Serialize)]
struct RenderContext<'a> {
    book_name: &'a str,
    author: &'a str,
    summary_html: String,
    core_points: Vec<RenderCorePoint<'a>>,
    thoughts_html: String,
    actions: Vec<RenderAction>,
}

#[derive(Serialize)]
struct RenderCorePoint<'a> {
    number: String,
    title: &'a str,
    content_html: String,
}

#[derive(Serialize)]
struct RenderAction {
    text_html: String,
}

fn validate_non_empty(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

fn escape_multiline(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.trim().chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            '\n' => escaped.push_str("<br>"),
            '\r' => {}
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> BookNoteInput {
        BookNoteInput {
            title: "系统思维读书笔记".into(),
            book_name: "系统之美".into(),
            author: "德内拉·梅多斯".into(),
            summary: "结构决定行为。".into(),
            core_points: vec![CorePointInput {
                title: "系统思维".into(),
                content: "关注结构，而不只是事件。".into(),
            }],
            thoughts: "先看整体，\n再看局部。".into(),
            actions: vec!["画出反馈回路".into()],
        }
    }

    #[test]
    fn renders_book_subtitle_and_ordered_sections() {
        let rendered = BookNoteRenderer::new().unwrap().render(&note()).unwrap();
        assert!(rendered.html.contains("《系统之美》"));
        assert!(rendered.html.contains("01 · 系统思维"));
        assert!(rendered.html.contains("先看整体，<br>再看局部。"));
        assert!(rendered.html.contains("✅ 画出反馈回路"));
        assert!(!rendered.html.contains("<h1"));
    }

    #[test]
    fn escapes_untrusted_text_and_template_syntax() {
        let mut input = note();
        input.summary = "<script>alert(\"x\")</script> & {{book_name}}".into();
        let rendered = BookNoteRenderer::new().unwrap().render(&input).unwrap();
        assert!(
            rendered
                .html
                .contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; {{book_name}}")
        );
        assert!(!rendered.html.contains("<script>"));
    }

    #[test]
    fn rejects_missing_sections_and_long_titles() {
        let renderer = BookNoteRenderer::new().unwrap();
        let mut input = note();
        input.actions.clear();
        assert!(renderer.render(&input).unwrap_err().contains("actions"));

        input = note();
        input.title = "书".repeat(33);
        assert!(renderer.render(&input).unwrap_err().contains("at most 32"));
    }
}
