use handlebars::Handlebars;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 32;
const MAX_CONTENT_CHARS: usize = 20_000;
const MAX_CONTENT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BookNoteInput {
    #[serde(default)]
    #[schemars(
        description = "Optional WeChat article title, at most 32 characters; defaults to book name plus 读书笔记"
    )]
    pub title: Option<String>,
    #[serde(default)]
    pub style: NoteStyle,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub book_meta: Option<BookMetaInput>,
    pub book_name: String,
    pub author: String,
    pub why_read: String,
    pub summary: String,
    #[serde(default)]
    pub core_points: Vec<CorePointInput>,
    #[serde(default)]
    pub sections: Option<Vec<SectionInput>>,
    #[serde(default)]
    pub example: Option<String>,
    pub thoughts: String,
    pub target_reader: String,
    pub actions: Vec<ActionInput>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BookMetaInput {
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub rating_count: Option<String>,
    #[serde(default)]
    pub want_to_read: Option<String>,
    #[serde(default)]
    pub reading_count: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NoteStyle {
    #[default]
    Reading,
    Business,
    Tech,
    Invest,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CorePointInput {
    #[serde(default)]
    pub number: Option<String>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub extension: Option<String>,
    #[serde(default)]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SectionInput {
    #[serde(default)]
    pub number: Option<String>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub extension: Option<String>,
    #[serde(default)]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ActionInput {
    pub text: String,
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
            .register_template_string(
                "reading",
                include_str!("../templates/book_note_reading_v2.html"),
            )
            .map_err(|error| format!("failed to compile book-note template: {error}"))?;
        handlebars
            .register_template_string(
                "business",
                include_str!("../templates/book_note_business.html"),
            )
            .map_err(|error| format!("failed to compile business template: {error}"))?;
        handlebars
            .register_template_string("tech", include_str!("../templates/book_note_tech.html"))
            .map_err(|error| format!("failed to compile tech template: {error}"))?;
        handlebars
            .register_template_string("invest", include_str!("../templates/book_note_invest.html"))
            .map_err(|error| format!("failed to compile invest template: {error}"))?;
        Ok(Self { handlebars })
    }

    pub fn render(&self, note: &BookNoteInput) -> Result<RenderedNote, String> {
        let _ = (&note.style, &note.category, &note.tags);
        validate_non_empty("book_name", &note.book_name)?;
        validate_non_empty("author", &note.author)?;
        validate_non_empty("why_read", &note.why_read)?;
        validate_non_empty("summary", &note.summary)?;
        if let Some(example) = note.example.as_deref()
            && !example.trim().is_empty()
        {
            validate_non_empty("example", example)?;
        }
        validate_non_empty("thoughts", &note.thoughts)?;
        validate_non_empty("target_reader", &note.target_reader)?;
        if let Some(title) = note.title.as_deref() {
            validate_non_empty("title", title)?;
            let count = title.chars().count();
            if count > MAX_TITLE_CHARS {
                return Err(format!(
                    "title must contain at most {MAX_TITLE_CHARS} characters; got {count}"
                ));
            }
        }
        let points = note.sections.as_deref().unwrap_or(&[]);
        let legacy_points = &note.core_points;
        if points.is_empty() && legacy_points.is_empty() {
            return Err("core_points must contain at least one item".into());
        }
        if note.actions.is_empty() {
            return Err("actions must contain at least one item".into());
        }
        let core_points = if let Some(points) = note.sections.as_ref() {
            points
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    render_point(
                        index,
                        &point.number,
                        &point.title,
                        &point.content,
                        point.extension.as_deref(),
                        point.example.as_deref(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?
        } else {
            note.core_points
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    render_point(
                        index,
                        &point.number,
                        &point.title,
                        &point.content,
                        point.extension.as_deref(),
                        point.example.as_deref(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        /*
        let core_points = note.core_points.iter().enumerate()
            .enumerate()
            .map(|(index, point)| {
                validate_non_empty("core_points[].title", &point.title)?;
                validate_non_empty("core_points[].content", &point.content)?;
                let number = point
                    .number
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .map(str::trim)
                    .unwrap_or("");
                Ok(RenderCorePoint {
                    number: if number.is_empty() {
                        format!("{:02}", index + 1)
                    } else {
                        number.to_owned()
                    },
                    title: point.title.trim().to_owned(),
                    content_html: escape_multiline(&point.content),
                    extension_html: point
                        .extension
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(escape_multiline),
                })
            }).collect::<Result<Vec<_>, String>>()?; */
        let actions = note
            .actions
            .iter()
            .map(|action| {
                validate_non_empty("actions[].text", &action.text)?;
                Ok(RenderAction {
                    text_html: escape_multiline(&action.text),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let context = RenderContext {
            book_name: note.book_name.trim(),
            author: note.author.trim(),
            why_read_html: escape_multiline(&note.why_read),
            summary_html: escape_multiline(&note.summary),
            core_points: core_points.clone(),
            example_html: escape_multiline(note.example.as_deref().unwrap_or("")),
            thoughts_html: escape_multiline(&note.thoughts),
            target_reader_html: escape_multiline(&note.target_reader),
            actions,
            insights: core_points.clone(),
            concepts: core_points.clone(),
            principles: core_points.clone(),
            problem_html: escape_multiline(&note.why_read),
            business_thoughts_html: escape_multiline(&note.thoughts),
            architecture_html: escape_multiline(&note.summary),
            investment_thesis_html: escape_multiline(&note.summary),
            risk_html: escape_multiline(note.example.as_deref().unwrap_or("")),
            book_meta: note.book_meta.as_ref().and_then(|meta| {
                let rendered = RenderBookMeta {
                    platform: meta
                        .platform
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or("微信读书")
                        .to_owned(),
                    rating: meta
                        .rating
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_owned),
                    rating_count: meta
                        .rating_count
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_owned),
                    want_to_read: meta
                        .want_to_read
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_owned),
                    reading_count: meta
                        .reading_count
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_owned),
                    url: meta
                        .url
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_owned),
                };

                (rendered.rating.is_some()
                    || rendered.want_to_read.is_some()
                    || rendered.reading_count.is_some())
                .then_some(rendered)
            }),
        };
        let template_name = match note.style {
            NoteStyle::Reading => "reading",
            NoteStyle::Business => "business",
            NoteStyle::Tech => "tech",
            NoteStyle::Invest => "invest",
        };
        let html = self
            .handlebars
            .render(template_name, &context)
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
    why_read_html: String,
    summary_html: String,
    core_points: Vec<RenderCorePoint>,
    example_html: String,
    thoughts_html: String,
    target_reader_html: String,
    actions: Vec<RenderAction>,
    insights: Vec<RenderCorePoint>,
    concepts: Vec<RenderCorePoint>,
    principles: Vec<RenderCorePoint>,
    problem_html: String,
    business_thoughts_html: String,
    architecture_html: String,
    investment_thesis_html: String,
    risk_html: String,
    book_meta: Option<RenderBookMeta>,
}
#[derive(Serialize)]
struct RenderBookMeta {
    platform: String,
    rating: Option<String>,
    rating_count: Option<String>,
    want_to_read: Option<String>,
    reading_count: Option<String>,
    url: Option<String>,
}
#[derive(Serialize, Clone)]
struct RenderCorePoint {
    number: String,
    title: String,
    content_html: String,
    extension_html: Option<String>,
    example_html: Option<String>,
    definition_html: String,
    practice_html: String,
    case_html: Option<String>,
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

fn render_point(
    index: usize,
    number: &Option<String>,
    title: &str,
    content: &str,
    extension: Option<&str>,
    example: Option<&str>,
) -> Result<RenderCorePoint, String> {
    validate_non_empty("core_points[].title", title)?;
    validate_non_empty("core_points[].content", content)?;
    let number = number
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .map(str::trim)
        .unwrap_or("");
    Ok(RenderCorePoint {
        number: if number.is_empty() {
            format!("{:02}", index + 1)
        } else {
            number.to_owned()
        },
        title: title.trim().to_owned(),
        content_html: escape_multiline(content),
        extension_html: extension
            .filter(|v| !v.trim().is_empty())
            .map(escape_multiline),
        example_html: example
            .filter(|v| !v.trim().is_empty())
            .map(escape_multiline),
        definition_html: escape_multiline(content),
        practice_html: extension
            .filter(|v| !v.trim().is_empty())
            .map(escape_multiline)
            .unwrap_or_default(),
        case_html: example
            .filter(|v| !v.trim().is_empty())
            .map(escape_multiline),
    })
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
            title: None,
            style: NoteStyle::Reading,
            category: None,
            tags: vec![],
            book_meta: None,
            book_name: "系统之美".into(),
            author: "德内拉·梅多斯".into(),
            why_read: "理解系统如何运作。".into(),
            summary: "结构决定行为。".into(),
            core_points: vec![CorePointInput {
                number: Some("01".into()),
                title: "系统思维".into(),
                content: "关注结构，而不只是事件。".into(),
                extension: Some("应用到工作复盘。".into()),
                example: None,
            }],
            sections: None,
            example: Some("书中的反馈回路案例。".into()),
            thoughts: "先看整体，\n再看局部。".into(),
            target_reader: "希望改善思考方式的人。".into(),
            actions: vec![ActionInput {
                text: "画出反馈回路".into(),
            }],
        }
    }
    #[test]
    fn renders_v2_sections() {
        let rendered = BookNoteRenderer::new().unwrap().render(&note()).unwrap();
        assert!(rendered.html.contains("为什么读这本书"));
        assert!(rendered.html.contains("01 · 系统思维"));
        assert!(rendered.html.contains("延伸思考"));
        assert!(rendered.html.contains("✓"));
    }

    #[test]
    fn omits_empty_book_case_section() {
        let mut input = note();
        input.example = None;
        let rendered = BookNoteRenderer::new().unwrap().render(&input).unwrap();
        assert!(!rendered.html.contains("书中案例"));
    }

    #[test]
    fn omits_empty_book_meta_section() {
        let mut input = note();
        input.book_meta = Some(BookMetaInput {
            platform: Some("微信读书".into()),
            rating: None,
            rating_count: None,
            want_to_read: None,
            reading_count: None,
            url: None,
        });
        let rendered = BookNoteRenderer::new().unwrap().render(&input).unwrap();
        assert!(!rendered.html.contains("阅读参考"));
    }

    #[test]
    fn escapes_untrusted_text() {
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
        input.title = Some("书".repeat(33));
        assert!(renderer.render(&input).unwrap_err().contains("at most 32"));
    }
}
