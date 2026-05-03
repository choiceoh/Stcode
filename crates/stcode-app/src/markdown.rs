use gpui::{AppContext, Context, Entity};

use crate::app_state::{ChatItem, MessageSegment, SessionUiState, Speaker};
use crate::selectable_text::{InlineKind, InlineSpan, SelectableText};
use crate::{MainView, theme};

/// 세션의 마지막 agent message에 markdown 파싱한 segments를 채운다.
/// 이미 채워졌거나 ``` 코드블록이 없으면 no-op.
pub(crate) fn finalize_last_agent_message_markdown(
    s: &mut SessionUiState,
    cx: &mut Context<MainView>,
) {
    let Some(item) = s.messages.iter_mut().rev().find(|m| {
        matches!(
            m,
            ChatItem::Message {
                who: Speaker::Agent,
                ..
            }
        )
    }) else {
        return;
    };
    let ChatItem::Message { text, segments, .. } = item else {
        return;
    };
    if segments.is_some() {
        return;
    }
    // raw text 추출 — read는 immut borrow, scope 끝내고 cx.new로 mut.
    let raw = text.read(cx).content().to_string();
    if !has_markdown_markers(&raw) {
        return;
    }
    let parsed = parse_markdown_segments(&raw, cx);
    *segments = Some(parsed);
}

/// 마크다운 marker가 하나라도 있으면 true. 파싱 비용 절약용 빠른 체크.
fn has_markdown_markers(raw: &str) -> bool {
    if raw.contains("```") || raw.contains('`') || raw.contains("**") || raw.contains("](") {
        return true;
    }
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("### ")
            || t.starts_with("- ")
            || t.starts_with("* ")
        {
            return true;
        }
    }
    false
}

/// 구조적 파싱 결과 — entity 만들기 전 단계. 단위 테스트 가능.
#[derive(Debug, PartialEq, Eq)]
enum RawSegment {
    Paragraph(String),
    Heading {
        level: u8,
        body: String,
    },
    ListItem(String),
    Code {
        body: String,
        language: Option<String>,
    },
}

/// `raw`를 RawSegment로 자른다 (entity 없음 → 테스트 가능).
/// 절차:
/// 1. ``` fence 토글 — fence 안은 통째로 Code.
/// 2. non-code 본문은 line 단위:
///    - `# `/`## `/`### ` → Heading
///    - `- `/`* ` (들여쓰기 0~3) → ListItem
///    - 빈 줄 → paragraph 끊기
///    - 그 외 → paragraph buffer 누적
/// inline (코드/볼드) 는 다음 단계.
fn parse_markdown_raw(raw: &str) -> Vec<RawSegment> {
    let mut segments: Vec<RawSegment> = Vec::new();
    let mut text_buf = String::new();
    let mut code_buf = String::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;

    let flush_text = |buf: &mut String, segs: &mut Vec<RawSegment>| {
        if buf.trim().is_empty() {
            buf.clear();
            return;
        }
        let text = buf.trim_end_matches('\n').to_string();
        segs.push(RawSegment::Paragraph(text));
        buf.clear();
    };

    let push_code = |buf: &mut String, lang: &mut Option<String>, segs: &mut Vec<RawSegment>| {
        let body = buf.trim_end_matches('\n').to_string();
        segs.push(RawSegment::Code {
            body,
            language: lang.take(),
        });
        buf.clear();
    };

    for line in raw.split_inclusive('\n') {
        let stripped_nl = line.trim_end_matches('\n').trim_end_matches('\r');

        if let Some(rest) = stripped_nl.strip_prefix("```") {
            if in_code {
                push_code(&mut code_buf, &mut code_lang, &mut segments);
                in_code = false;
            } else {
                flush_text(&mut text_buf, &mut segments);
                let lang = rest.trim().to_string();
                code_lang = if lang.is_empty() { None } else { Some(lang) };
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_buf.push_str(line);
            continue;
        }

        if let Some(level) = heading_level(stripped_nl) {
            flush_text(&mut text_buf, &mut segments);
            let body = stripped_nl[(level as usize + 1).min(stripped_nl.len())..]
                .trim_start()
                .to_string();
            segments.push(RawSegment::Heading { level, body });
            continue;
        }

        if let Some(item) = list_item_body(stripped_nl) {
            flush_text(&mut text_buf, &mut segments);
            segments.push(RawSegment::ListItem(item));
            continue;
        }

        if stripped_nl.trim().is_empty() {
            flush_text(&mut text_buf, &mut segments);
            continue;
        }

        text_buf.push_str(line);
    }

    if in_code {
        push_code(&mut code_buf, &mut code_lang, &mut segments);
    } else {
        flush_text(&mut text_buf, &mut segments);
    }

    segments
}

#[derive(Debug, PartialEq, Eq)]
struct InlineParse {
    text: String,
    spans: Vec<InlineSpan>,
}

fn parse_inline_markdown(raw: &str) -> InlineParse {
    let mut text = String::with_capacity(raw.len());
    let mut spans = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if let Some(after_tick) = rest.strip_prefix('`') {
            if let Some(end) = after_tick.find('`') {
                let body = &after_tick[..end];
                if !body.is_empty() {
                    let start = text.len();
                    text.push_str(body);
                    spans.push(InlineSpan {
                        range: start..text.len(),
                        kind: InlineKind::Code,
                    });
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        if let Some(after_open) = rest.strip_prefix("**") {
            if let Some(end) = after_open.find("**") {
                let body = &after_open[..end];
                if !body.is_empty() {
                    let start = text.len();
                    text.push_str(body);
                    spans.push(InlineSpan {
                        range: start..text.len(),
                        kind: InlineKind::Bold,
                    });
                    i += 2 + end + 2;
                    continue;
                }
            }
        }
        if let Some(after_bracket) = rest.strip_prefix('[') {
            if let Some(label_end) = after_bracket.find("](") {
                let label = &after_bracket[..label_end];
                let after_url_open = &after_bracket[label_end + 2..];
                if let Some(url_end) = after_url_open.find(')') {
                    let url = &after_url_open[..url_end];
                    if !label.is_empty() && !url.is_empty() {
                        let start = text.len();
                        text.push_str(label);
                        text.push_str(" (");
                        text.push_str(url);
                        text.push(')');
                        spans.push(InlineSpan {
                            range: start..text.len(),
                            kind: InlineKind::Link,
                        });
                        i += 1 + label_end + 2 + url_end + 1;
                        continue;
                    }
                }
            }
        }

        let ch = rest.chars().next().expect("non-empty rest");
        text.push(ch);
        i += ch.len_utf8();
    }
    InlineParse { text, spans }
}

fn selectable_from_markdown_inline(
    raw: String,
    color: u32,
    cx: &mut Context<MainView>,
) -> Entity<SelectableText> {
    let parsed = parse_inline_markdown(&raw);
    if parsed.spans.is_empty() && parsed.text == raw {
        cx.new(|cx| SelectableText::new(raw, color, cx))
    } else {
        cx.new(|cx| SelectableText::new_inline(parsed.text, color, parsed.spans, cx))
    }
}

/// RawSegment 들에서 SelectableText entity를 만들어 MessageSegment 로 변환.
fn parse_markdown_segments(raw: &str, cx: &mut Context<MainView>) -> Vec<MessageSegment> {
    parse_markdown_raw(raw)
        .into_iter()
        .map(|raw_seg| match raw_seg {
            RawSegment::Paragraph(text) => {
                let entity = selectable_from_markdown_inline(text, theme::TOKENS.fg, cx);
                MessageSegment::Paragraph(entity)
            }
            RawSegment::Heading { level, body } => {
                let entity = selectable_from_markdown_inline(body, theme::TOKENS.fg, cx);
                MessageSegment::Heading {
                    level,
                    body: entity,
                }
            }
            RawSegment::ListItem(body) => {
                let entity = selectable_from_markdown_inline(body, theme::TOKENS.fg, cx);
                MessageSegment::ListItem { body: entity }
            }
            RawSegment::Code { body, language } => {
                let entity = cx.new(|cx| SelectableText::new(body, theme::TOKENS.fg, cx));
                MessageSegment::Code {
                    body: entity,
                    language,
                }
            }
        })
        .collect()
}

/// `# `/`## `/`### ` 만 인식. 더 깊은 헤딩은 무시.
fn heading_level(line: &str) -> Option<u8> {
    if line.starts_with("### ") {
        Some(3)
    } else if line.starts_with("## ") {
        Some(2)
    } else if line.starts_with("# ") {
        Some(1)
    } else {
        None
    }
}

/// 들여쓰기 0~3 + `- ` 또는 `* ` 로 시작하면 list item.
fn list_item_body(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some(rest.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return Some(rest.to_string());
    }
    None
}

#[cfg(test)]
mod markdown_tests {
    use super::*;

    #[test]
    fn inline_code_strips_markers_and_records_span() {
        let parsed = parse_inline_markdown("use `cargo test` now");

        assert_eq!(parsed.text, "use cargo test now");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 4..14,
                kind: InlineKind::Code,
            }]
        );
    }

    #[test]
    fn inline_bold_strips_markers_and_records_span() {
        let parsed = parse_inline_markdown("this is **important**");

        assert_eq!(parsed.text, "this is important");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 8..17,
                kind: InlineKind::Bold,
            }]
        );
    }

    #[test]
    fn inline_link_keeps_url_visible_and_records_span() {
        let parsed = parse_inline_markdown("see [docs](https://example.test)");

        assert_eq!(parsed.text, "see docs (https://example.test)");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 4..31,
                kind: InlineKind::Link,
            }]
        );
    }

    #[test]
    fn unmatched_inline_markers_stay_literal() {
        let parsed = parse_inline_markdown("bad `code and **bold");

        assert_eq!(parsed.text, "bad `code and **bold");
        assert!(parsed.spans.is_empty());
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_markdown_raw(""), vec![]);
    }

    #[test]
    fn plain_paragraph() {
        let segs = parse_markdown_raw("hello world");
        assert_eq!(segs, vec![RawSegment::Paragraph("hello world".into())]);
    }

    #[test]
    fn fenced_code_with_lang() {
        let segs = parse_markdown_raw("```python\nprint('hi')\n```");
        assert_eq!(
            segs,
            vec![RawSegment::Code {
                body: "print('hi')".into(),
                language: Some("python".into())
            }]
        );
    }

    #[test]
    fn heading_levels() {
        let segs = parse_markdown_raw("# h1\n## h2\n### h3");
        assert_eq!(
            segs,
            vec![
                RawSegment::Heading {
                    level: 1,
                    body: "h1".into()
                },
                RawSegment::Heading {
                    level: 2,
                    body: "h2".into()
                },
                RawSegment::Heading {
                    level: 3,
                    body: "h3".into()
                },
            ]
        );
    }

    #[test]
    fn list_dash_and_star() {
        let segs = parse_markdown_raw("- one\n* two\n  - nested");
        assert_eq!(
            segs,
            vec![
                RawSegment::ListItem("one".into()),
                RawSegment::ListItem("two".into()),
                RawSegment::ListItem("nested".into()),
            ]
        );
    }

    #[test]
    fn paragraph_break_on_blank_line() {
        let segs = parse_markdown_raw("para 1\n\npara 2");
        assert_eq!(
            segs,
            vec![
                RawSegment::Paragraph("para 1".into()),
                RawSegment::Paragraph("para 2".into()),
            ]
        );
    }

    #[test]
    fn mixed_kitchen_sink() {
        let raw = "# 안녕\n\n첫 단락.\n\n- 항목 1\n- 항목 2\n\n```rs\nfn main() {}\n```\n\n끝.";
        let segs = parse_markdown_raw(raw);
        assert_eq!(
            segs,
            vec![
                RawSegment::Heading {
                    level: 1,
                    body: "안녕".into()
                },
                RawSegment::Paragraph("첫 단락.".into()),
                RawSegment::ListItem("항목 1".into()),
                RawSegment::ListItem("항목 2".into()),
                RawSegment::Code {
                    body: "fn main() {}".into(),
                    language: Some("rs".into())
                },
                RawSegment::Paragraph("끝.".into()),
            ]
        );
    }

    #[test]
    fn unclosed_code_block_treated_as_code() {
        let segs = parse_markdown_raw("```\nhello\nworld");
        assert_eq!(
            segs,
            vec![RawSegment::Code {
                body: "hello\nworld".into(),
                language: None
            }]
        );
    }
}
