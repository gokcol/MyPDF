//! Editable clipboard text with a plain-text alternative for apps without HTML support.
//! PDF paint instructions are already resolved by the viewer; this module only
//! serializes the selected text and its styles, never document markup or URLs.
use crate::viewer::{PageText, TextLine, TextSpan, TextStyle};
use std::{fmt::Write, ops::Range};

#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardText {
    pub text: String,
    /// An HTML fragment. The platform clipboard supplies its own UTF-8 wrapper.
    pub html: String,
}

/// Copy exactly this UTF-8 range, including partial styled runs and ligatures.
pub fn from_page(page: &PageText, range: Range<usize>) -> Option<ClipboardText> {
    let text = page.text.get(range.clone())?;
    if text.trim().is_empty() {
        return None;
    }
    let mut html = String::from("<div>");
    let mut start = range.start;
    let mut previous: Option<(&TextLine, f32)> = None;
    let selected_lines: Vec<_> = text.split('\n').collect();
    let geometry: Vec<_> =
        selected_lines
            .iter()
            .scan(start, |offset, line| {
                let at = *offset;
                *offset += line.len() + 1;
                let i = page.lines.partition_point(|l| l.range.end <= at);
                Some(page.lines.get(i).filter(|l| {
                    l.range.start <= at && l.left.is_finite() && l.baseline.is_finite()
                }))
            })
            .collect();
    // Preserve relative indentation without copying the PDF's page margin.
    let left_edge = geometry
        .iter()
        .flatten()
        .map(|l| l.left)
        .reduce(f32::min)
        .unwrap_or(0.);
    for (line, position) in selected_lines.into_iter().zip(geometry) {
        let end = start + line.len();
        let first = page.spans.partition_point(|s| s.range.end <= start);
        let style_at_start = page.spans.get(first).filter(|s| s.range.start <= start);
        let default = TextStyle::default();
        let base = style_at_start.map_or(&default, |s| &s.style);
        let height = page.spans[first..]
            .iter()
            .take_while(|s| s.range.start < end)
            .fold(font_size(base.size), |size, s| {
                size.max(font_size(s.style.size))
            })
            * 1.2;
        let indent = position.map_or(0., |l| (l.left - left_edge).clamp(0., 720.));
        let gap = match (previous, position) {
            (Some((before, before_height)), Some(now)) => {
                (now.baseline - before.baseline - (height + before_height) / 2.).clamp(0., 720.)
            }
            _ => 0.,
        };
        let _ = write!(
            html,
            "<p style=\"margin:0pt;margin-top:{gap:.2}pt;margin-left:{indent:.2}pt;line-height:{height:.2}pt;mso-line-height-rule:exactly;white-space:pre-wrap\">"
        );
        if line.is_empty() {
            html.push_str("<br>");
        } else {
            write_runs(&mut html, &page.text, start..end, &page.spans, base);
        }
        html.push_str("</p>");
        previous = position.map(|p| (p, height));
        start = end + 1;
    }
    html.push_str("</div>");
    Some(ClipboardText {
        text: text.to_owned(),
        html,
    })
}

/// A text object may have a known uniform style but no indexed glyph selection.
pub fn from_styled_text(text: &str, style: &TextStyle) -> Option<ClipboardText> {
    from_page(
        &PageText {
            text: text.to_owned(),
            spans: vec![TextSpan {
                range: 0..text.len(),
                style: style.clone(),
            }],
            ..Default::default()
        },
        0..text.len(),
    )
}

fn write_runs(
    html: &mut String,
    text: &str,
    range: Range<usize>,
    spans: &[TextSpan],
    base: &TextStyle,
) {
    let first = spans.partition_point(|s| s.range.end <= range.start);
    let mut cursor = range.start;
    let mut inherited = base;
    for span in spans[first..]
        .iter()
        .take_while(|s| s.range.start < range.end)
    {
        let start = cursor.max(span.range.start);
        let end = range.end.min(span.range.end);
        if start >= end || text.get(start..end).is_none() {
            continue;
        }
        if cursor < start {
            write_run(html, &text[cursor..start], inherited);
        }
        write_run(html, &text[start..end], &span.style);
        inherited = &span.style;
        cursor = end;
    }
    if cursor < range.end {
        write_run(html, &text[cursor..range.end], inherited);
    }
}

fn font_size(size: f32) -> f32 {
    if size.is_finite() && size > 0. {
        size.clamp(1., 512.)
    } else {
        12.
    }
}

fn write_run(html: &mut String, text: &str, style: &TextStyle) {
    let family = if style.family.trim().is_empty() {
        "sans-serif"
    } else {
        &style.family
    };
    // Escape as a CSS string first, then as an HTML attribute. A PDF font name
    // must never be able to introduce HTML, CSS declarations or remote content.
    let mut css_family = String::new();
    for c in family.chars().take(256) {
        if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
            css_family.push(c);
        } else {
            let _ = write!(css_family, "\\{:x} ", c as u32);
        }
    }
    let css_family = if matches!(family, "sans-serif" | "serif" | "monospace") {
        css_family
    } else {
        format!("'{css_family}'")
    };
    let [r, g, b] = style.color;
    let size = font_size(style.size);
    let weight = if style.bold { 700 } else { 400 };
    let slant = if style.italic { "italic" } else { "normal" };
    let _ = write!(
        html,
        "<span style=\"font-family:{css_family};font-size:{size:.2}pt;font-weight:{weight};font-style:{slant};color:#{r:02x}{g:02x}{b:02x};mso-spacerun:yes\">"
    );
    for c in text.chars() {
        match c {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            '\t' => html.push_str("<span style=\"mso-tab-count:1\">&#9;</span>"),
            '\r' => (),
            c if c.is_control() => html.push_str("&#xfffd;"),
            c => html.push(c),
        }
    }
    html.push_str("</span>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_unicode_selection_retains_mixed_styles_and_exact_plain_text() {
        let text = "prefix café & bold Ω suffix";
        let bold = text.find("bold").unwrap();
        let page = PageText {
            text: text.into(),
            spans: vec![
                TextSpan {
                    range: 0..bold,
                    style: TextStyle {
                        family: "Aptos".into(),
                        size: 11.,
                        ..Default::default()
                    },
                },
                TextSpan {
                    range: bold..text.len(),
                    style: TextStyle {
                        family: "Georgia".into(),
                        size: 14.,
                        bold: true,
                        italic: true,
                        color: [20, 50, 200],
                    },
                },
            ],
            ..Default::default()
        };
        let copied = from_page(&page, 7..text.find(" suffix").unwrap()).unwrap();
        assert_eq!(copied.text, "café & bold Ω");
        assert!(copied.html.contains("café &amp; "));
        assert!(
            copied
                .html
                .contains("font-family:'Aptos';font-size:11.00pt;font-weight:400")
        );
        assert!(copied.html.contains("font-family:'Georgia';font-size:14.00pt;font-weight:700;font-style:italic;color:#1432c8"));
        assert!(!copied.html.contains("prefix"));
        assert!(!copied.html.contains("suffix"));
        assert!(
            from_page(&page, 11..12).is_none(),
            "UTF-8 boundaries are required"
        );
    }

    #[test]
    fn paragraphs_keep_relative_indentation_and_vertical_gaps() {
        let page = PageText {
            text: "first\nsecond\nthird".into(),
            lines: vec![
                TextLine {
                    range: 0..5,
                    left: 50.,
                    baseline: 70.,
                },
                TextLine {
                    range: 6..12,
                    left: 50.,
                    baseline: 84.4,
                },
                TextLine {
                    range: 13..18,
                    left: 68.,
                    baseline: 113.2,
                },
            ],
            ..Default::default()
        };
        let copied = from_page(&page, 0..page.text.len()).unwrap();
        assert_eq!(copied.text, page.text);
        assert_eq!(copied.html.matches("<p ").count(), 3);
        assert!(
            copied
                .html
                .contains("margin-top:14.40pt;margin-left:18.00pt")
        );
        assert!(!copied.html.contains("margin-left:50.00pt"));
    }

    #[test]
    fn markup_font_names_and_bad_metrics_cannot_escape_into_html() {
        let copied = from_styled_text(
            "<img src='https://example.test'>\tİstanbul  🦀",
            &TextStyle {
                family: "Evil\"';background:url(https://example.test);<x>".into(),
                size: f32::NAN,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!copied.html.contains("<img"));
        assert!(!copied.html.contains("<x>"));
        assert!(!copied.html.contains("background:url("));
        assert!(copied.html.contains("&lt;img"));
        assert!(copied.html.contains("font-size:12.00pt"));
        assert!(copied.html.contains("İstanbul  🦀"));
        assert!(copied.html.contains("mso-tab-count:1"));
    }

    #[test]
    fn fallback_font_is_a_generic_css_family() {
        let copied = from_styled_text("Plain", &TextStyle::default()).unwrap();
        assert!(copied.html.contains("font-family:sans-serif;"));
        assert!(!copied.html.contains("font-family:'sans-serif'"));
    }
}
