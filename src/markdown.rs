//! Renders Markdown into the HTML shape Proton Mail's composer produces, so
//! drafts look hand-written in Proton: the composer's default font, one `<div>`
//! per paragraph, a blank `<div><br></div>` line between blocks, and the
//! composer's own margin/blockquote rules inlined. Raw HTML in the input is
//! escaped, never passed through, so callers cannot introduce other styling.

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html::push_html};

const FONT_STYLE: &str = "font-family: Arial, sans-serif; font-size: 14px;";
const BLANK_LINE: &str = "<div><br></div>\n";
const BLOCKQUOTE_STYLE: &str = "margin: 0; padding: 0 0 0 1rem; border-left: 4px solid #e5e5e5;";
const NO_MARGIN: &str = "margin: 0;";

pub fn markdown_to_html(markdown: &str) -> String {
    let mut sibling_blocks = vec![0usize];
    let events = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH)
        .flat_map(|event| restyle(event, &mut sibling_blocks));
    let mut body = String::new();
    push_html(&mut body, events);
    format!("<div style=\"{FONT_STYLE}\">\n{body}</div>\n")
}

fn markup(html: impl Into<String>) -> Event<'static> {
    Event::Html(CowStr::from(html.into()))
}

fn starts_block(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Rule
            | Event::Start(
                Tag::Paragraph
                    | Tag::Heading { .. }
                    | Tag::BlockQuote(_)
                    | Tag::CodeBlock(_)
                    | Tag::List(_)
                    | Tag::HtmlBlock
            )
    )
}

/// `sibling_blocks` holds, for each open container (document, blockquote, list
/// item), how many blocks it has seen; every block after the first is preceded
/// by a blank line, which replaces the browser margins zeroed out below.
fn restyle<'a>(event: Event<'a>, sibling_blocks: &mut Vec<usize>) -> Vec<Event<'a>> {
    let follows_sibling = starts_block(&event)
        && sibling_blocks.last_mut().is_some_and(|count| {
            *count += 1;
            *count > 1
        });

    match &event {
        Event::Start(Tag::BlockQuote(_) | Tag::Item) => sibling_blocks.push(0),
        Event::End(TagEnd::BlockQuote(_) | TagEnd::Item) => {
            sibling_blocks.pop();
        }
        _ => {}
    }

    let styled = match event {
        Event::Start(Tag::Paragraph | Tag::HtmlBlock) => markup("<div>"),
        Event::End(TagEnd::Paragraph | TagEnd::HtmlBlock) => markup("</div>\n"),
        Event::Start(Tag::Heading { level, .. }) => {
            markup(format!("<{level} style=\"{NO_MARGIN}\">"))
        }
        Event::Start(Tag::BlockQuote(_)) => {
            markup(format!("<blockquote style=\"{BLOCKQUOTE_STYLE}\">\n"))
        }
        Event::Start(Tag::List(None)) => markup(format!("<ul style=\"{NO_MARGIN}\">\n")),
        Event::Start(Tag::List(Some(1))) => markup(format!("<ol style=\"{NO_MARGIN}\">\n")),
        Event::Start(Tag::List(Some(start))) => {
            markup(format!("<ol start=\"{start}\" style=\"{NO_MARGIN}\">\n"))
        }
        Event::Start(Tag::CodeBlock(_)) => markup(format!("<pre style=\"{NO_MARGIN}\"><code>")),
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::SoftBreak => Event::HardBreak,
        other => other,
    };

    if follows_sibling {
        vec![markup(BLANK_LINE), styled]
    } else {
        vec![styled]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner(markdown: &str) -> String {
        let html = markdown_to_html(markdown);
        html.strip_prefix(&format!("<div style=\"{FONT_STYLE}\">\n"))
            .and_then(|rest| rest.strip_suffix("</div>\n"))
            .unwrap_or_else(|| panic!("missing Proton font wrapper: {html}"))
            .to_owned()
    }

    #[test]
    fn wraps_body_in_proton_default_font() {
        assert_eq!(
            markdown_to_html("Hello"),
            "<div style=\"font-family: Arial, sans-serif; font-size: 14px;\">\n<div>Hello</div>\n</div>\n"
        );
    }

    #[test]
    fn paragraphs_are_divs_separated_by_a_blank_line() {
        assert_eq!(
            inner("First\n\nSecond"),
            "<div>First</div>\n<div><br></div>\n<div>Second</div>\n"
        );
    }

    #[test]
    fn single_newlines_are_kept_as_line_breaks() {
        assert_eq!(inner("Best,\nChase"), "<div>Best,<br />\nChase</div>\n");
    }

    #[test]
    fn inline_formatting() {
        assert_eq!(
            inner("**bold** *italic* ~~gone~~ `code` [link](https://example.com)"),
            "<div><strong>bold</strong> <em>italic</em> <del>gone</del> <code>code</code> \
             <a href=\"https://example.com\">link</a></div>\n"
        );
    }

    #[test]
    fn lists_have_no_margin_and_tight_items() {
        assert_eq!(
            inner("Items:\n\n- one\n- two\n  1. nested"),
            "<div>Items:</div>\n<div><br></div>\n<ul style=\"margin: 0;\">\n<li>one</li>\n\
             <li>two<ol style=\"margin: 0;\">\n<li>nested</li>\n</ol>\n</li>\n</ul>\n"
        );
    }

    #[test]
    fn ordered_list_keeps_custom_start() {
        assert!(inner("3. three\n4. four").starts_with("<ol start=\"3\" style=\"margin: 0;\">"));
    }

    #[test]
    fn blockquote_uses_composer_style_and_separates_inner_paragraphs() {
        assert_eq!(
            inner("> one\n>\n> two"),
            "<blockquote style=\"margin: 0; padding: 0 0 0 1rem; border-left: 4px solid #e5e5e5;\">\n\
             <div>one</div>\n<div><br></div>\n<div>two</div>\n</blockquote>\n"
        );
    }

    #[test]
    fn headings_and_code_blocks_have_no_margin() {
        assert_eq!(
            inner("# Title\n\n```\nlet x = 1 < 2;\n```"),
            "<h1 style=\"margin: 0;\">Title</h1>\n<div><br></div>\n\
             <pre style=\"margin: 0;\"><code>let x = 1 &lt; 2;\n</code></pre>\n"
        );
    }

    #[test]
    fn raw_html_is_escaped() {
        let inline = inner("Hi <span style=\"color: red\">there</span>");
        assert!(!inline.contains("<span"), "inline HTML leaked: {inline}");
        assert!(inline.contains("&lt;span style=\"color: red\"&gt;there&lt;/span&gt;"));

        let block = inner("<table style=\"font-size: 40px\"><tr><td>x</td></tr></table>");
        assert!(!block.contains("<table"), "block HTML leaked: {block}");
        assert!(block.starts_with("<div>&lt;table"));
    }
}
