//! DOCX writer using docx-rs.
//! Writes directly to an in-memory buffer — no temp files.
use anyhow::Result;
use docx_rs::*;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::ir::doc::{Block, DocIR, Inline};

pub fn render(doc: &DocIR) -> Result<Vec<u8>> {
    build_docx_bytes(doc)
}

fn build_docx_bytes(doc: &DocIR) -> Result<Vec<u8>> {
    let mut docx = Docx::new();

    // Title from metadata
    if let Some(title) = &doc.metadata.title {
        let para = Paragraph::new()
            .add_run(Run::new().add_text(title).bold())
            .style("Title");
        docx = docx.add_paragraph(para);
    }

    let mut num_id_counter = 1usize;
    for block in &doc.blocks {
        (docx, num_id_counter) = add_block(docx, block, num_id_counter)?;
    }

    // Write directly to an in-memory cursor — no temp file needed
    let mut buf: Vec<u8> = Vec::new();
    docx.build()
        .pack(Cursor::new(&mut buf))
        .map_err(|e| anyhow::anyhow!("DOCX pack: {}", e))?;
    Ok(buf)
}

fn add_block(docx: Docx, block: &Block, num_id: usize) -> Result<(Docx, usize)> {
    let mut num_id = num_id;
    let docx = match block {
        Block::Heading(level, inlines) => {
            let style = match level {
                1 => "Heading1", 2 => "Heading2", 3 => "Heading3",
                4 => "Heading4", 5 => "Heading5", _ => "Heading6",
            };
            let mut para = Paragraph::new().style(style);
            para = add_inlines_to_para(para, inlines);
            docx.add_paragraph(para)
        }
        Block::Para(inlines) => {
            let mut para = Paragraph::new();
            para = add_inlines_to_para(para, inlines);
            docx.add_paragraph(para)
        }
        Block::CodeBlock { code, .. } => {
            let mut d = docx;
            for line in code.lines() {
                let para = Paragraph::new().add_run(
                    Run::new().add_text(line).fonts(RunFonts::new().ascii("Courier New")),
                );
                d = d.add_paragraph(para);
            }
            d
        }
        Block::BlockQuote(blocks) => {
            let mut d = docx;
            for b in blocks {
                (d, num_id) = add_block(d, b, num_id)?;
            }
            d
        }
        Block::List { ordered, start, items, .. } => {
            let abstract_num = AbstractNumbering::new(num_id).add_level(Level::new(
                0,
                Start::new((*start).max(1) as usize),
                NumberFormat::new(if *ordered { "decimal" } else { "bullet" }),
                LevelText::new(if *ordered { "%1." } else { "•" }),
                LevelJc::new("left"),
            ));
            let mut d = docx
                .add_abstract_numbering(abstract_num)
                .add_numbering(Numbering::new(num_id, num_id));

            for item in items {
                let inlines = match item.first() {
                    Some(Block::Para(ils)) => ils.clone(),
                    Some(Block::Heading(_, ils)) => ils.clone(),
                    _ => vec![],
                };
                let mut para = Paragraph::new()
                    .numbering(NumberingId::new(num_id), IndentLevel::new(0));
                para = add_inlines_to_para(para, &inlines);
                d = d.add_paragraph(para);
                for sub in item.iter().skip(1) {
                    (d, num_id) = add_block(d, sub, num_id + 1)?;
                    num_id -= 1;
                }
            }
            num_id += 1;
            d
        }
        Block::Table { head, rows } => {
            let mut table = Table::new(vec![]);
            let header_cells: Vec<TableCell> = head.iter().map(|cell_inlines| {
                let text: String = cell_inlines.iter().map(|i| {
                    let mut s = String::new();
                    crate::ir::doc::inline_to_text(i, &mut s);
                    s
                }).collect();
                TableCell::new().add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text(&text).bold())
                )
            }).collect();
            table = table.add_row(TableRow::new(header_cells));

            for row in rows {
                let cells: Vec<TableCell> = row.iter().map(|cell_inlines| {
                    let mut para = Paragraph::new();
                    para = add_inlines_to_para(para, cell_inlines);
                    TableCell::new().add_paragraph(para)
                }).collect();
                table = table.add_row(TableRow::new(cells));
            }
            docx.add_table(table)
        }
        Block::HorizontalRule => {
            docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("─".repeat(40)))
            )
        }
        Block::RawBlock { content, .. } => {
            docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(content)))
        }
    };
    Ok((docx, num_id))
}

fn add_inlines_to_para(mut para: Paragraph, inlines: &[Inline]) -> Paragraph {
    for il in inlines { para = add_inline_to_para(para, il); }
    para
}

fn add_inline_to_para(para: Paragraph, il: &Inline) -> Paragraph {
    match il {
        Inline::Text(t) => para.add_run(Run::new().add_text(t)),
        Inline::Strong(inner) => {
            let mut t = String::new();
            for i in inner { crate::ir::doc::inline_to_text(i, &mut t); }
            para.add_run(Run::new().add_text(&t).bold())
        }
        Inline::Emph(inner) => {
            let mut t = String::new();
            for i in inner { crate::ir::doc::inline_to_text(i, &mut t); }
            para.add_run(Run::new().add_text(&t).italic())
        }
        Inline::Strikethrough(inner) => {
            let mut t = String::new();
            for i in inner { crate::ir::doc::inline_to_text(i, &mut t); }
            para.add_run(Run::new().add_text(format!("~~{}~~", t)))
        }
        Inline::Code(s) => {
            para.add_run(
                Run::new().add_text(s).fonts(RunFonts::new().ascii("Courier New"))
            )
        }
        Inline::Link { url, content, .. } => {
            let mut t = String::new();
            for i in content { crate::ir::doc::inline_to_text(i, &mut t); }
            let hl = Hyperlink::new(url, HyperlinkType::External)
                .add_run(Run::new().add_text(&t));
            para.add_hyperlink(hl)
        }
        Inline::Image { src, alt } => {
            let mut alt_text = String::new();
            for i in alt { crate::ir::doc::inline_to_text(i, &mut alt_text); }

            if let Some(pic_data) = try_embed_image(src) {
                let (w_emu, h_emu) = read_image_dims(&pic_data).unwrap_or((2743200, 1828800));
                let pic = Pic::new(&pic_data).size(w_emu, h_emu);
                para.add_run(Run::new().add_image(pic))
            } else {
                para.add_run(Run::new().add_text(format!("[Image: {}]", alt_text)))
            }
        }
        Inline::LineBreak => para.add_run(Run::new().add_break(BreakType::TextWrapping)),
        Inline::SoftBreak => para.add_run(Run::new().add_text(" ")),
        Inline::RawInline { content, .. } => para.add_run(Run::new().add_text(content)),
    }
}

/// Largest image we are willing to read off disk and embed.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Resolve an image reference from document content into a path we are willing
/// to read.
///
/// Image sources come from the document being converted, which is routinely
/// untrusted. Reading whatever path it names turns a format conversion into
/// file exfiltration: `![](/etc/passwd)` or `![](../../.ssh/id_rsa)` would be
/// read and embedded into the output `.docx` for the attacker to collect.
/// References are therefore resolved relative to the current directory and must
/// stay within it.
fn resolve_image_path(src: &str) -> Option<PathBuf> {
    if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
        return None;
    }
    let path = Path::new(src);
    if path.is_absolute() {
        return None;
    }
    let base = std::env::current_dir().ok()?.canonicalize().ok()?;
    // canonicalize() resolves both ".." and symlinks, so neither can be used to
    // walk out of `base` behind the containment check.
    let resolved = base.join(path).canonicalize().ok()?;
    if !resolved.starts_with(&base) || !resolved.is_file() {
        return None;
    }
    Some(resolved)
}

fn try_embed_image(src: &str) -> Option<Vec<u8>> {
    let path = resolve_image_path(src)?;

    // Read through a single handle rather than checking then re-opening, so the
    // file cannot be swapped between the check and the read.
    let file = std::fs::File::open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 || len > MAX_IMAGE_BYTES {
        return None;
    }
    let mut data = Vec::with_capacity(len as usize);
    file.take(MAX_IMAGE_BYTES).read_to_end(&mut data).ok()?;

    validate_image(&data)?;

    Some(data)
}

/// Largest edge length and decoded footprint we are willing to process.
const MAX_IMAGE_DIMENSION: u32 = 20_000;
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

/// Confirm the bytes really decode as a supported image.
///
/// Two separate problems need this. Sniffing the magic number is not enough for
/// confidentiality — a file that merely *starts* with a PNG signature is not an
/// image, and embedding it would still hand over its contents. It is also not
/// enough for robustness: docx-rs decodes the buffer with an `expect`, so any
/// malformed input reaching `Pic::new` aborts the whole conversion. Decoding
/// here, against the same `image` version docx-rs uses, rules out both — and the
/// limits stop a small file that declares enormous dimensions from being
/// expanded into gigabytes of pixels.
fn validate_image(data: &[u8]) -> Option<()> {
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);

    let mut reader = image::io::Reader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits);
    reader.format()?;
    reader.decode().ok()?;
    Some(())
}

fn read_image_dims(data: &[u8]) -> Option<(u32, u32)> {
    let reader = image::io::Reader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let (w_px, h_px) = reader.into_dimensions().ok()?;
    let w_emu = w_px * 9525;
    let h_emu = h_px * 9525;
    let max_w: u32 = 5486400;
    if w_emu > max_w {
        let scale = max_w as f64 / w_emu as f64;
        Some((max_w, (h_emu as f64 * scale) as u32))
    } else {
        Some((w_emu, h_emu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_and_traversing_paths_are_refused() {
        for hostile in [
            "/etc/passwd",
            "../../../../etc/passwd",
            "../Cargo.toml",
            "http://example.com/x.png",
            "data:image/png;base64,AAAA",
        ] {
            assert!(resolve_image_path(hostile).is_none(), "should be refused: {hostile}");
        }
    }

    #[test]
    fn paths_inside_the_working_directory_resolve() {
        // Tests run with the crate root as the working directory.
        assert!(resolve_image_path("Cargo.toml").is_some());
        assert!(resolve_image_path("does-not-exist.png").is_none());
    }

    #[test]
    fn non_images_are_not_embedded() {
        // Right magic number, invalid body: must be rejected rather than
        // embedded, and rather than reaching docx-rs, which would panic.
        let mut fake_png = b"\x89PNG\r\n\x1a\n".to_vec();
        fake_png.extend_from_slice(b"SECRET-CONTENT");
        assert!(validate_image(&fake_png).is_none());
        assert!(validate_image(b"plain text, not an image").is_none());
    }
}
