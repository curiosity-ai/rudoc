use anyhow::{bail, Context, Result};
use calamine::{open_workbook_auto_from_rs, DataType, Reader};
use std::io::Cursor;

use crate::ir::table::{CellValue, Sheet, TableIR};

/// Ceiling on the total decompressed size a workbook may declare.
const MAX_TOTAL_DECOMPRESSED: u64 = 512 * 1024 * 1024;

pub fn parse(bytes: &[u8], target_sheet: Option<&str>) -> Result<TableIR> {
    reject_oversized_archive(bytes)?;

    let cursor = Cursor::new(bytes);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .context("Failed to open Excel file")?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::new();

    for name in &sheet_names {
        if let Some(filter) = target_sheet {
            if name != filter { continue; }
        }
        // worksheet_range returns Option<Result<Range, Error>>
        let range = match workbook.worksheet_range(name) {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                eprintln!("[rudoc] warning: sheet '{}' error: {}", name, e);
                continue;
            }
            None => continue,
        };

        let mut headers: Vec<String> = Vec::new();
        let mut rows: Vec<Vec<CellValue>> = Vec::new();
        let mut first_row = true;

        for row in range.rows() {
            let cells: Vec<CellValue> = row.iter().map(data_to_cell).collect();
            if first_row {
                headers = cells.iter().map(|c| c.as_str()).collect();
                first_row = false;
            } else {
                rows.push(cells);
            }
        }

        sheets.push(Sheet { name: name.clone(), headers, rows });
    }

    if sheets.is_empty() {
        anyhow::bail!("No sheets found in workbook");
    }
    Ok(TableIR { sheets })
}

/// Reject workbooks whose declared decompressed size is implausible, before
/// handing the bytes to a parser that would expand them.
///
/// This reads the sizes recorded in the ZIP central directory, so it stops the
/// cheap "small file, enormous expansion" case. It is a guard, not a proof: an
/// archive may understate its entry sizes, and the real defence there is
/// running conversions under a memory limit.
fn reject_oversized_archive(bytes: &[u8]) -> Result<()> {
    let mut archive = match zip::ZipArchive::new(Cursor::new(bytes)) {
        Ok(a) => a,
        // Not a ZIP at all (legacy .xls, for instance) — nothing to check here.
        Err(_) => return Ok(()),
    };

    let mut total: u64 = 0;
    for i in 0..archive.len() {
        // `by_index_raw` reads the entry header without setting up decompression.
        let entry = archive.by_index_raw(i)?;
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_DECOMPRESSED {
            bail!(
                "Workbook declares more than {} MiB of decompressed content — refusing to parse.",
                MAX_TOTAL_DECOMPRESSED / (1024 * 1024)
            );
        }
    }
    Ok(())
}

fn data_to_cell(d: &DataType) -> CellValue {
    match d {
        DataType::Empty => CellValue::Empty,
        DataType::String(s) => CellValue::Str(s.clone()),
        DataType::Float(f) => CellValue::Num(*f),
        DataType::Int(i) => CellValue::Num(*i as f64),
        DataType::Bool(b) => CellValue::Bool(*b),
        DataType::Error(_) => CellValue::Empty,
        _ => CellValue::Empty,
    }
}
