//! PDF writer with two strategies:
//!  1. Subprocess: call the `typst` CLI if it is on PATH (best quality)
//!  2. Built-in:   `printpdf` with embedded Helvetica (feature = "pdf")
//!  3. Fallback:   helpful error with instructions
//!
//! Strategy 1 executes an external program, so it is selectable via
//! `--pdf-engine` and announces itself on stderr rather than running silently.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::ir::doc::DocIR;
use crate::writers::typst_writer;

/// Which backend produces the PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfEngine {
    /// Use `typst` when it is available, otherwise the built-in renderer.
    Auto,
    /// Require the external `typst` binary; fail if it is missing.
    Typst,
    /// Never spawn a subprocess; use the built-in renderer only.
    Builtin,
}

impl PdfEngine {
    pub fn from_name(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "auto" => PdfEngine::Auto,
            "typst" => PdfEngine::Typst,
            "builtin" | "internal" | "printpdf" => PdfEngine::Builtin,
            other => bail!(
                "Unknown --pdf-engine '{}'. Valid values: auto, typst, builtin.",
                other
            ),
        })
    }
}

pub fn render(
    doc: &DocIR,
    paper: &str,
    font: &str,
    engine: PdfEngine,
    quiet: bool,
) -> Result<Vec<u8>> {
    // Strategy 1: typst CLI subprocess (zero extra binary size)
    if engine != PdfEngine::Builtin {
        let required = engine == PdfEngine::Typst;
        match find_typst() {
            None if required => bail!(
                "--pdf-engine=typst was requested, but no `typst` binary was found on PATH.\n\
                 Install it from https://typst.app, or use --pdf-engine=builtin."
            ),
            None => {}
            Some(program) => {
                let typ_source = typst_writer::render(doc, paper, font);
                match run_typst(&program, &typ_source, quiet) {
                    Ok(Some(bytes)) => return Ok(bytes),
                    Ok(None) if required => bail!(
                        "`typst` at {} did not produce a PDF for the generated document.",
                        program.display()
                    ),
                    Err(e) if required => return Err(e),
                    Ok(None) => {
                        if !quiet {
                            eprintln!("[rudoc] warning: typst produced no output; \
                                       falling back to the built-in renderer");
                        }
                    }
                    Err(e) => {
                        if !quiet {
                            eprintln!("[rudoc] warning: typst engine error ({e}); \
                                       falling back to the built-in renderer");
                        }
                    }
                }
            }
        }
    }

    // Strategy 2: built-in printpdf (requires feature = "pdf")
    #[cfg(feature = "pdf")]
    {
        return render_printpdf(doc, paper);
    }

    // Strategy 3: helpful error
    #[cfg(not(feature = "pdf"))]
    bail!(
        "PDF output requires either:\n\
         • Install typst on your PATH: https://typst.app (recommended, best quality)\n\
         • Rebuild rudoc with PDF built-in: cargo build --features pdf\n\
         • Convert to .typ first then run typst manually: rudoc input.md output.typ"
    )
}

/// Compile `typ_source` with an already-resolved `typst` binary.
///
/// `Ok(None)` means typst ran but produced no PDF; the caller decides whether
/// that is fatal.
fn run_typst(program: &Path, typ_source: &str, quiet: bool) -> Result<Option<Vec<u8>>> {
    let dir = create_private_dir()?;
    let result = compile_in(&dir, program, typ_source, quiet);
    // Clean up whatever happened, including on the error paths.
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn compile_in(
    dir: &Path,
    program: &Path,
    typ_source: &str,
    quiet: bool,
) -> Result<Option<Vec<u8>>> {
    let typ_path = dir.join("input.typ");
    let pdf_path = dir.join("output.pdf");

    std::fs::write(&typ_path, typ_source)?;

    if !quiet {
        eprintln!("[rudoc] PDF via external engine: {}", program.display());
    }

    // `--root` pins typst's file-access sandbox to our private directory, so a
    // converted document cannot reach files elsewhere on the machine even if it
    // manages to get something evaluated.
    let status = std::process::Command::new(program)
        .arg("compile")
        .arg("--root")
        .arg(dir)
        .arg(&typ_path)
        .arg(&pdf_path)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Ok(None);
    }
    Ok(std::fs::read(&pdf_path).ok())
}

/// Create a private, exclusively-owned scratch directory under the system temp
/// directory, and return its path.
///
/// Writing `rudoc_<pid>_<n>.typ` directly into the shared temp directory is not
/// safe: the names are predictable and `fs::write` follows symlinks, so a local
/// attacker can pre-create the path as a link to a file they want truncated and
/// overwritten — and can swap the output PDF between typst writing it and rudoc
/// reading it back, making rudoc emit a file of their choosing.
///
/// `mkdir(2)` fails when the final component already exists and does not follow
/// a symlink there, which makes directory creation the atomic step that defeats
/// both races. On Unix the directory is created 0700 so nothing can be planted
/// inside it afterwards.
fn create_private_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir();

    for attempt in 0..64u32 {
        let path = base.join(format!(
            "rudoc-{}-{:016x}",
            std::process::id(),
            scratch_token(attempt)
        ));

        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }

        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }

    bail!("Could not create a private temporary directory in {}", base.display())
}

/// Name material for the scratch directory. Not cryptographic — it only has to
/// avoid collisions; `mkdir` failing on an existing path is what provides the
/// security property.
fn scratch_token(attempt: u32) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    nanos
        ^ (CTR.fetch_add(1, Ordering::Relaxed) << 32)
        ^ ((attempt as u64) << 48)
}

/// Locate the `typst` binary by walking PATH ourselves.
///
/// `Command::new("typst")` hands resolution to the platform, and on Windows the
/// process-creation search order includes the current directory — so converting
/// documents inside an untrusted folder could execute a `typst.exe` dropped
/// alongside them. Resolving to an absolute path here keeps the lookup to
/// absolute PATH entries. Only `.exe` is considered on Windows, which avoids the
/// separate argument-escaping hazards of `.bat`/`.cmd` wrappers.
fn find_typst() -> Option<PathBuf> {
    find_typst_in(&std::env::var_os("PATH")?)
}

fn find_typst_in(path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    let name = if cfg!(windows) { "typst.exe" } else { "typst" };

    for dir in std::env::split_paths(path_var) {
        // An empty PATH entry means "current directory" on POSIX; skip it, and
        // skip any other relative entry, for the same reason.
        if dir.as_os_str().is_empty() || !dir.is_absolute() {
            continue;
        }
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Pure-Rust PDF using printpdf with Helvetica built-in font.
#[cfg(feature = "pdf")]
fn render_printpdf(doc: &DocIR, paper: &str) -> Result<Vec<u8>> {
    use printpdf::*;

    let (width_mm, height_mm): (f64, f64) = match paper {
        "a3"        => (297.0, 420.0),
        "a5"        => (148.0, 210.0),
        "us-letter" => (215.9, 279.4),
        "us-legal"  => (215.9, 355.6),
        _           => (210.0, 297.0), // a4 default
    };

    let (doc_pdf, page1, layer1) = PdfDocument::new(
        doc.metadata.title.as_deref().unwrap_or("Document"),
        Mm(width_mm),
        Mm(height_mm),
        "Layer 1",
    );

    let font      = doc_pdf.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc_pdf.add_builtin_font(BuiltinFont::HelveticaBold)?;

    let margin_mm   = 20.0_f64;
    let line_height = 6.0_f64;
    let text_width  = width_mm - 2.0 * margin_mm;
    let mut cursor_y = height_mm - margin_mm;

    let mut pages: Vec<(PdfPageIndex, PdfLayerIndex)> = vec![(page1, layer1)];
    let mut page_idx = 0usize;

    macro_rules! ensure_space {
        ($needed:expr) => {
            if cursor_y < margin_mm + $needed {
                let (np, nl) = doc_pdf.add_page(Mm(width_mm), Mm(height_mm), "Layer 1");
                pages.push((np, nl));
                page_idx += 1;
                cursor_y = height_mm - margin_mm;
            }
        };
    }

    macro_rules! layer {
        () => { doc_pdf.get_page(pages[page_idx].0).get_layer(pages[page_idx].1) };
    }

    for block in &doc.blocks {
        match block {
            crate::ir::doc::Block::Heading(level, inlines) => {
                let size = match level { 1 => 18.0, 2 => 14.0, 3 => 12.0, _ => 11.0 };
                cursor_y -= if *level <= 2 { line_height } else { line_height * 0.5 };
                ensure_space!(size * 0.5 + line_height);
                let mut text = String::new();
                for il in inlines { crate::ir::doc::inline_to_text(il, &mut text); }
                layer!().use_text(&text, size, Mm(margin_mm), Mm(cursor_y), &font_bold);
                cursor_y -= line_height * (size / 11.0);
            }
            crate::ir::doc::Block::Para(inlines) => {
                let mut text = String::new();
                for il in inlines { crate::ir::doc::inline_to_text(il, &mut text); }
                let chars_per_line = (text_width / 2.2) as usize;
                for line in &word_wrap(&text, chars_per_line) {
                    ensure_space!(line_height * 2.0);
                    layer!().use_text(line, 11.0, Mm(margin_mm), Mm(cursor_y), &font);
                    cursor_y -= line_height;
                }
                cursor_y -= line_height * 0.5;
            }
            crate::ir::doc::Block::CodeBlock { code, .. } => {
                let font_mono = doc_pdf.add_builtin_font(BuiltinFont::Courier)?;
                for line in code.lines() {
                    ensure_space!(line_height);
                    layer!().use_text(line, 9.0, Mm(margin_mm + 4.0), Mm(cursor_y), &font_mono);
                    cursor_y -= line_height * 0.9;
                }
                cursor_y -= line_height * 0.5;
            }
            crate::ir::doc::Block::List { ordered, start, items, .. } => {
                for (i, item) in items.iter().enumerate() {
                    let bullet = if *ordered { format!("{}.", i as u64 + start) } else { "•".to_string() };
                    let text = item.iter().find_map(|b| {
                        if let crate::ir::doc::Block::Para(ils) = b {
                            let mut s = String::new();
                            for il in ils { crate::ir::doc::inline_to_text(il, &mut s); }
                            Some(s)
                        } else { None }
                    }).unwrap_or_default();
                    ensure_space!(line_height);
                    layer!().use_text(&bullet, 11.0, Mm(margin_mm),       Mm(cursor_y), &font);
                    layer!().use_text(&text,   11.0, Mm(margin_mm + 6.0), Mm(cursor_y), &font);
                    cursor_y -= line_height;
                }
                cursor_y -= line_height * 0.5;
            }
            crate::ir::doc::Block::HorizontalRule => {
                cursor_y -= line_height;
                ensure_space!(line_height);
                let line = Line {
                    points: vec![
                        (Point::new(Mm(margin_mm), Mm(cursor_y + line_height / 2.0)), false),
                        (Point::new(Mm(width_mm - margin_mm), Mm(cursor_y + line_height / 2.0)), false),
                    ],
                    is_closed: false, has_fill: false, has_stroke: true, is_clipping_path: false,
                };
                layer!().add_shape(line);
                cursor_y -= line_height;
            }
            other => {
                let mut text = String::new();
                crate::ir::doc::block_to_text_pub(other, &mut text);
                for line in text.lines() {
                    ensure_space!(line_height);
                    layer!().use_text(line, 11.0, Mm(margin_mm), Mm(cursor_y), &font);
                    cursor_y -= line_height;
                }
            }
        }
    }

    let mut buf = Vec::new();
    doc_pdf.save(&mut std::io::BufWriter::new(&mut buf))?;
    Ok(buf)
}

fn word_wrap(text: &str, chars_per_line: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut current = String::new();
    for word in words {
        if current.len() + word.len() + 1 > chars_per_line && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() { current.push(' '); }
        current.push_str(word);
    }
    if !current.is_empty() { lines.push(current); }
    if lines.is_empty() { lines.push(String::new()); }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_names_parse() {
        assert_eq!(PdfEngine::from_name("auto").unwrap(), PdfEngine::Auto);
        assert_eq!(PdfEngine::from_name("TYPST").unwrap(), PdfEngine::Typst);
        assert_eq!(PdfEngine::from_name("builtin").unwrap(), PdfEngine::Builtin);
        assert!(PdfEngine::from_name("latex").is_err());
    }

    #[test]
    fn scratch_dir_is_private_unique_and_removable() {
        let a = create_private_dir().unwrap();
        let b = create_private_dir().unwrap();
        assert_ne!(a, b, "scratch directory names must not collide");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "scratch dir must not be group/world accessible");
        }

        // Re-creating an existing path must fail rather than reuse it, which is
        // what stops a pre-planted symlink from being followed.
        let mut builder = std::fs::DirBuilder::new();
        assert!(builder.recursive(false).create(&a).is_err());

        std::fs::remove_dir_all(&a).unwrap();
        std::fs::remove_dir_all(&b).unwrap();
    }

    #[test]
    fn typst_lookup_ignores_relative_path_entries() {
        use std::ffi::OsStr;

        // "." and the empty entry are what would let a binary dropped in the
        // working directory be executed. Neither may be searched, even when a
        // real `typst` sits right there.
        let dir = create_private_dir().unwrap();
        let planted = dir.join(if cfg!(windows) { "typst.exe" } else { "typst" });
        std::fs::write(&planted, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert!(find_typst_in(OsStr::new(".:")).is_none());
        assert!(find_typst_in(OsStr::new("")).is_none());
        assert!(find_typst_in(OsStr::new("relative/dir")).is_none());

        // An absolute entry pointing at the same binary is still honoured.
        assert_eq!(find_typst_in(dir.as_os_str()).as_deref(), Some(planted.as_path()));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
