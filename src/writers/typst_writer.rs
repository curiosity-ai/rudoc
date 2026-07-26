use crate::ir::doc::{Block, DocIR, Inline};

/// Render DocIR to Typst source (.typ).
pub fn render(doc: &DocIR, paper: &str, font: &str) -> String {
    let mut out = String::new();

    // Document setup. `paper` and `font` reach us from CLI flags, which may in
    // turn be fed by a caller wrapping rudoc — escape them as string literals
    // so they cannot terminate the string and inject Typst code.
    out.push_str(&format!(
        "#set page(paper: \"{}\")\n#set text(font: \"{}\", size: 11pt)\n#set par(justify: true)\n\n",
        escape_typ_str(paper),
        escape_typ_str(font)
    ));

    // Metadata
    if let Some(title) = &doc.metadata.title {
        out.push_str(&format!(
            "#align(center, text(size: 18pt, weight: \"bold\")[{}])\n\n",
            escape_typ(title)
        ));
    }
    if let Some(author) = &doc.metadata.author {
        out.push_str(&format!(
            "#align(center)[{}]\n\n",
            escape_typ(author)
        ));
    }
    if let Some(date) = &doc.metadata.date {
        out.push_str(&format!(
            "#align(center)[{}]\n\n",
            escape_typ(date)
        ));
    }

    for block in &doc.blocks {
        render_block(block, &mut out, 0);
        out.push('\n');
    }
    out
}

fn render_block(block: &Block, out: &mut String, depth: usize) {
    match block {
        Block::Heading(level, inlines) => {
            let prefix = match level {
                1 => "= ",
                2 => "== ",
                3 => "=== ",
                4 => "==== ",
                5 => "===== ",
                _ => "====== ",
            };
            out.push_str(prefix);
            render_inlines(inlines, out);
            out.push('\n');
        }
        Block::Para(inlines) => {
            render_inlines(inlines, out);
            out.push('\n');
        }
        Block::CodeBlock { lang, code } => {
            let lang_str = lang.as_deref().unwrap_or("");
            out.push_str(&format!("```{}\n{}\n```\n", lang_str, code));
        }
        Block::BlockQuote(blocks) => {
            out.push_str("#block(stroke: (left: 2pt + gray), inset: (left: 8pt, y: 4pt))[\n");
            for b in blocks { render_block(b, out, depth + 1); }
            out.push_str("]\n");
        }
        Block::List { ordered, start, items, .. } => {
            for (i, item) in items.iter().enumerate() {
                if *ordered {
                    out.push_str(&format!("{}. ", i as u64 + start));
                } else {
                    out.push_str("- ");
                }
                for (bi, block) in item.iter().enumerate() {
                    if bi == 0 {
                        match block {
                            Block::Para(inlines) => {
                                render_inlines(inlines, out);
                                out.push('\n');
                            }
                            other => render_block(other, out, depth + 1),
                        }
                    } else {
                        render_block(block, out, depth + 1);
                    }
                }
            }
        }
        Block::Table { head, rows } => {
            let cols = head.len().max(rows.first().map(|r| r.len()).unwrap_or(1));
            out.push_str(&format!(
                "#table(\n  columns: {},\n  table.header(",
                cols
            ));
            for cell in head {
                out.push_str("[*");
                render_inlines(cell, out);
                out.push_str("*], ");
            }
            out.push_str("),\n");
            for row in rows {
                for cell in row {
                    out.push_str("  [");
                    render_inlines(cell, out);
                    out.push_str("],\n");
                }
            }
            out.push_str(")\n");
        }
        Block::HorizontalRule => out.push_str("#line(length: 100%)\n"),
        // Raw content is only passed through verbatim when it is already Typst.
        // Anything else (notably `format == "html"`, which the Markdown reader
        // emits for inline HTML) would be injected straight into the source we
        // hand to `typst compile`, so it is dropped instead.
        Block::RawBlock { format, content } if format == "typst" => out.push_str(content),
        Block::RawBlock { .. } => {}
    }
}

fn render_inlines(inlines: &[Inline], out: &mut String) {
    for il in inlines { render_inline(il, out); }
}

fn render_inline(il: &Inline, out: &mut String) {
    match il {
        Inline::Text(t) => out.push_str(&escape_typ(t)),
        Inline::Emph(inner) => {
            out.push_str("_");
            render_inlines(inner, out);
            out.push('_');
        }
        Inline::Strong(inner) => {
            out.push_str("*");
            render_inlines(inner, out);
            out.push('*');
        }
        Inline::Strikethrough(inner) => {
            out.push_str("#strike[");
            render_inlines(inner, out);
            out.push(']');
        }
        Inline::Code(s) => {
            out.push('`');
            out.push_str(s);
            out.push('`');
        }
        Inline::Link { url, content, .. } => {
            out.push_str(&format!("#link(\"{}\")[", escape_typ_str(url)));
            render_inlines(content, out);
            out.push(']');
        }
        Inline::Image { src, alt } => {
            let mut alt_text = String::new();
            for il in alt { crate::ir::doc::inline_to_text(il, &mut alt_text); }
            out.push_str(&format!("#image(\"{}\", alt: \"{}\")",
                escape_typ_str(src), escape_typ_str(&alt_text)));
        }
        Inline::LineBreak => out.push_str("\\\n"),
        Inline::SoftBreak => out.push(' '),
        // See the RawBlock arm: non-Typst raw content is dropped rather than
        // injected into the source.
        Inline::RawInline { format, content } if format == "typst" => out.push_str(content),
        Inline::RawInline { .. } => {}
    }
}

/// Escape Typst special characters in running text (markup context).
fn escape_typ(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('@', "\\@")
        .replace('$', "\\$")
}

/// Escape a value being interpolated *inside a Typst string literal*, e.g.
/// `#link("HERE")`. This is a different context from running text: only the
/// backslash and the double quote are special, but both must be escaped or the
/// value can close the string and continue in code position.
fn escape_typ_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _    => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::doc::{Block, DocIR, Inline};

    #[test]
    fn string_context_escapes_the_quote() {
        assert_eq!(escape_typ_str(r#"a") + eval("1+1"#), r#"a\") + eval(\"1+1"#);
        assert_eq!(escape_typ_str(r"back\slash"), r"back\\slash");
    }

    #[test]
    fn link_url_cannot_escape_into_code_position() {
        let mut d = DocIR::new();
        d.blocks.push(Block::Para(vec![Inline::Link {
            url: r#"x") + read("/etc/passwd"#.into(),
            title: String::new(),
            content: vec![Inline::Text("t".into())],
        }]));
        let out = render(&d, "a4", "Arial");
        // Every quote from the URL must arrive escaped, so the whole payload
        // stays inside the string literal instead of continuing as code.
        assert!(
            out.contains(r##"#link("x\") + read(\"/etc/passwd")[t]"##),
            "link was not escaped as expected: {out}"
        );
    }

    #[test]
    fn paper_and_font_are_escaped() {
        let out = render(&DocIR::new(), r#"a4") #read("/etc/passwd"#, "Arial");
        assert!(out.starts_with(r#"#set page(paper: "a4\") #read(\"/etc/passwd")"#), "{out}");
    }

    #[test]
    fn foreign_raw_content_is_not_injected() {
        let mut d = DocIR::new();
        d.blocks.push(Block::RawBlock {
            format: "html".into(),
            content: r#"#read("/etc/passwd")"#.into(),
        });
        d.blocks.push(Block::Para(vec![Inline::RawInline {
            format: "html".into(),
            content: r#"#eval("1+1")"#.into(),
        }]));
        let out = render(&d, "a4", "Arial");
        assert!(!out.contains("#read("), "raw html injected into typst source: {out}");
        assert!(!out.contains("#eval("));
    }

    #[test]
    fn genuine_typst_raw_still_passes_through() {
        let mut d = DocIR::new();
        d.blocks.push(Block::RawBlock {
            format: "typst".into(),
            content: "#pagebreak()".into(),
        });
        assert!(render(&d, "a4", "Arial").contains("#pagebreak()"));
    }
}
