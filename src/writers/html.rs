use crate::ir::doc::{Block, DocIR, Inline};

/// Rendering options that affect the safety of the emitted HTML.
#[derive(Debug, Clone, Copy)]
pub struct HtmlOptions {
    /// Emit a full document with `<head>` and styles.
    pub standalone: bool,
    /// Pass raw HTML from the source document through verbatim.
    ///
    /// Off by default: input documents are frequently untrusted, and raw
    /// passthrough turns a `<script>` in a Markdown file into script execution
    /// in whoever opens the output.
    pub allow_raw_html: bool,
}

pub fn render(doc: &DocIR, opts: HtmlOptions) -> String {
    let standalone = opts.standalone;
    let mut body = String::new();
    for block in &doc.blocks {
        render_block(block, &mut body, opts);
        body.push('\n');
    }

    if !standalone {
        return body;
    }

    let title = doc.metadata.title.as_deref().unwrap_or("Document");
    format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
  body {{ max-width: 860px; margin: 2rem auto; font-family: system-ui, sans-serif;
          line-height: 1.6; color: #222; padding: 0 1rem; }}
  pre {{ background: #f5f5f5; padding: 1rem; border-radius: 4px; overflow-x: auto; }}
  code {{ font-family: "Fira Code", "Cascadia Code", monospace; font-size: 0.9em; }}
  blockquote {{ border-left: 4px solid #ccc; margin: 0; padding-left: 1rem; color: #555; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ border: 1px solid #ddd; padding: 0.5rem 0.75rem; text-align: left; }}
  th {{ background: #f0f0f0; }}
  img {{ max-width: 100%; }}
  hr {{ border: none; border-top: 1px solid #eee; }}
</style>
</head>
<body>
{body}
</body>
</html>
"#,
        lang = doc.metadata.lang.as_deref().unwrap_or("en"),
        title = escape_html(title),
        body = body,
    )
}

fn render_block(block: &Block, out: &mut String, opts: HtmlOptions) {
    match block {
        Block::Heading(level, inlines) => {
            out.push_str(&format!("<h{}>", level));
            render_inlines(inlines, out, opts);
            out.push_str(&format!("</h{}>\n", level));
        }
        Block::Para(inlines) => {
            out.push_str("<p>");
            render_inlines(inlines, out, opts);
            out.push_str("</p>\n");
        }
        Block::CodeBlock { lang, code } => {
            let class = lang
                .as_deref()
                .map(|l| format!(" class=\"language-{}\"", l))
                .unwrap_or_default();
            out.push_str(&format!("<pre><code{}>{}</code></pre>\n",
                class, escape_html(code)));
        }
        Block::BlockQuote(blocks) => {
            out.push_str("<blockquote>\n");
            for b in blocks { render_block(b, out, opts); }
            out.push_str("</blockquote>\n");
        }
        Block::List { ordered, start, items, .. } => {
            let tag = if *ordered {
                if *start != 1 {
                    format!("<ol start=\"{}\">", start)
                } else {
                    "<ol>".to_string()
                }
            } else {
                "<ul>".to_string()
            };
            let close = if *ordered { "</ol>" } else { "</ul>" };
            out.push_str(&tag);
            out.push('\n');
            for item in items {
                out.push_str("<li>");
                // Unwrap single para items (tight lists)
                if item.len() == 1 {
                    if let Block::Para(inlines) = &item[0] {
                        render_inlines(inlines, out, opts);
                        out.push_str("</li>\n");
                        continue;
                    }
                }
                out.push('\n');
                for b in item { render_block(b, out, opts); }
                out.push_str("</li>\n");
            }
            out.push_str(close);
            out.push('\n');
        }
        Block::Table { head, rows } => {
            out.push_str("<table>\n");
            if !head.is_empty() {
                out.push_str("<thead><tr>");
                for cell in head {
                    out.push_str("<th>");
                    render_inlines(cell, out, opts);
                    out.push_str("</th>");
                }
                out.push_str("</tr></thead>\n");
            }
            out.push_str("<tbody>\n");
            for row in rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    render_inlines(cell, out, opts);
                    out.push_str("</td>");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody></table>\n");
        }
        Block::HorizontalRule => out.push_str("<hr>\n"),
        Block::RawBlock { format, content } if format == "html" => {
            if opts.allow_raw_html {
                out.push_str(content);
            } else {
                // Escaped, not dropped: the author still sees their markup, but
                // it is inert.
                out.push_str(&escape_html(content));
            }
        }
        Block::RawBlock { content, .. } => {
            // Comment-wrapped, so "-->" in the content must not be able to
            // close the comment early and escape into markup.
            out.push_str(&format!("<!-- {} -->\n", escape_comment(content)));
        }
    }
}

/// Neutralise anything that could terminate an HTML comment.
fn escape_comment(s: &str) -> String {
    s.replace("--", "\u{2013}\u{2013}").replace('>', "&gt;")
}

fn render_inlines(inlines: &[Inline], out: &mut String, opts: HtmlOptions) {
    for il in inlines {
        render_inline(il, out, opts);
    }
}

fn render_inline(il: &Inline, out: &mut String, opts: HtmlOptions) {
    match il {
        Inline::Text(t) => out.push_str(&escape_html(t)),
        Inline::Emph(inner) => {
            out.push_str("<em>");
            render_inlines(inner, out, opts);
            out.push_str("</em>");
        }
        Inline::Strong(inner) => {
            out.push_str("<strong>");
            render_inlines(inner, out, opts);
            out.push_str("</strong>");
        }
        Inline::Strikethrough(inner) => {
            out.push_str("<s>");
            render_inlines(inner, out, opts);
            out.push_str("</s>");
        }
        Inline::Code(s) => {
            out.push_str("<code>");
            out.push_str(&escape_html(s));
            out.push_str("</code>");
        }
        Inline::Link { url, title, content } => {
            if title.is_empty() {
                out.push_str(&format!("<a href=\"{}\">", escape_attr(&sanitize_url(url))));
            } else {
                out.push_str(&format!("<a href=\"{}\" title=\"{}\">",
                    escape_attr(&sanitize_url(url)), escape_html(title)));
            }
            render_inlines(content, out, opts);
            out.push_str("</a>");
        }
        Inline::Image { src, alt } => {
            let mut alt_text = String::new();
            render_inlines_plain(alt, &mut alt_text);
            out.push_str(&format!("<img src=\"{}\" alt=\"{}\">",
                escape_attr(&sanitize_url(src)), escape_html(&alt_text)));
        }
        Inline::LineBreak => out.push_str("<br>\n"),
        Inline::SoftBreak => out.push('\n'),
        Inline::RawInline { format, content } if format == "html" => {
            if opts.allow_raw_html {
                out.push_str(content);
            } else {
                out.push_str(&escape_html(content));
            }
        }
        Inline::RawInline { .. } => {}
    }
}

fn render_inlines_plain(inlines: &[Inline], out: &mut String) {
    for il in inlines {
        crate::ir::doc::inline_to_text(il, out);
    }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Reduce a URL taken from document content to a scheme we are willing to emit
/// into an `href`/`src`.
///
/// Escaping alone is not enough here: `javascript:alert(1)` contains no
/// character that attribute escaping touches, yet it executes on click. Only a
/// known-good set of schemes is allowed through; anything else becomes an inert
/// fragment. Scheme-relative and relative URLs carry no scheme and are kept.
pub fn sanitize_url(url: &str) -> String {
    const ALLOWED: [&str; 7] = ["http", "https", "mailto", "ftp", "ftps", "tel", "irc"];

    // Strip characters browsers ignore when resolving the scheme, so that
    // "java\tscript:" and " javascript:" cannot slip past the comparison.
    let normalized: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();

    // A scheme runs up to the first ':', and only if no '/', '?' or '#' comes
    // first — otherwise the colon belongs to a path or query.
    let scheme = normalized
        .find(':')
        .filter(|&i| !normalized[..i].contains(['/', '?', '#']))
        .map(|i| &normalized[..i]);

    match scheme {
        // No scheme: relative URL or bare fragment, nothing to execute.
        None => url.to_string(),
        Some(s) => {
            // A single letter is a Windows drive prefix ("C:\docs\a.png"),
            // not a URL scheme.
            if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) {
                return url.to_string();
            }
            if ALLOWED.contains(&s) {
                return url.to_string();
            }
            // Inline images are common and inert; other data: payloads are not.
            if s == "data" && normalized.starts_with("data:image/") && !normalized.contains("svg") {
                return url.to_string();
            }
            "#".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::doc::{Block, DocIR, Inline};

    fn doc_with(inline: Inline) -> DocIR {
        let mut d = DocIR::new();
        d.blocks.push(Block::Para(vec![inline]));
        d
    }

    fn opts(allow_raw_html: bool) -> HtmlOptions {
        HtmlOptions { standalone: false, allow_raw_html }
    }

    #[test]
    fn executable_url_schemes_are_neutralised() {
        for hostile in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            " javascript:alert(1)",
            "java\tscript:alert(1)",
            "vbscript:msgbox",
            "data:text/html,<script>alert(1)</script>",
            "data:image/svg+xml,<svg onload=alert(1)>",
        ] {
            assert_eq!(sanitize_url(hostile), "#", "should have been blocked: {hostile}");
        }
    }

    #[test]
    fn ordinary_urls_survive() {
        for benign in [
            "https://example.com/a?b=1#c",
            "http://example.com",
            "mailto:a@b.c",
            "tel:+15551234",
            "../relative/path.png",
            "#anchor",
            "/absolute/path",
            "C:\\docs\\image.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ] {
            assert_eq!(sanitize_url(benign), benign, "should have been allowed: {benign}");
        }
    }

    #[test]
    fn raw_html_is_inert_unless_opted_in() {
        let d = doc_with(Inline::RawInline {
            format: "html".into(),
            content: "<script>alert(1)</script>".into(),
        });

        let safe = render(&d, opts(false));
        assert!(!safe.contains("<script>"), "raw HTML leaked by default: {safe}");
        assert!(safe.contains("&lt;script&gt;"));

        let unsafe_out = render(&d, opts(true));
        assert!(unsafe_out.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn attribute_escaping_covers_quote_and_angle_brackets() {
        let escaped = escape_attr("\" onerror='x' <b>");
        assert!(!escaped.contains('"'));
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('\''));
    }

    #[test]
    fn non_html_raw_block_cannot_close_its_comment() {
        let mut d = DocIR::new();
        d.blocks.push(Block::RawBlock {
            format: "latex".into(),
            content: "--><script>alert(1)</script>".into(),
        });
        let out = render(&d, opts(false));
        assert!(!out.contains("-->\u{3c}script"), "comment was closed early: {out}");
        assert!(!out.contains("<script>"));
    }
}
