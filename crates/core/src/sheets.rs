//! Spreadsheets: `sheet.read` turns a CSV, Excel (.xlsx, .xlsm, .xls), or OpenDocument (.ods)
//! sheet into rows keyed by the header row, and `sheet.write` writes rows out as an Excel file
//! (or CSV by extension). Both are pure Rust; nothing is borrowed from the Mac.

use serde_json::{json, Value};
use std::path::Path;

use crate::tools::{safe_file_name, Tool, ToolContext, ToolDescription, ToolError};

/// Rows a read keeps at most, so a huge sheet does not swamp a model's context.
pub const MAX_ROWS: usize = 5_000;

/// `sheet.read`: one sheet as rows of `{ header: value }`.
pub struct SheetRead;

fn cell_to_json(c: &calamine::Data) -> Value {
    use calamine::Data;
    match c {
        Data::Empty => Value::Null,
        Data::String(s) => Value::String(s.clone()),
        Data::Float(f) => json!(f),
        Data::Int(i) => json!(i),
        Data::Bool(b) => Value::Bool(*b),
        Data::DateTime(d) => d.as_datetime().map(|t| Value::String(t.format("%Y-%m-%d %H:%M:%S").to_string())).unwrap_or_else(|| json!(d.as_f64())),
        Data::DateTimeIso(s) | Data::DurationIso(s) => Value::String(s.clone()),
        Data::Error(e) => Value::String(format!("#{e:?}")),
    }
}

/// Headers from the first row: a blank header becomes `Column N`; a repeated one gets ` (2)`.
fn headers_of(first: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, v) in first.iter().enumerate() {
        let base = match v {
            Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
            Value::Null => format!("Column {}", i + 1),
            other => other.to_string().trim_matches('"').to_string(),
        };
        let mut name = base.clone();
        let mut n = 2;
        while out.contains(&name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        out.push(name);
    }
    out
}

fn rows_from(table: Vec<Vec<Value>>, max_rows: usize) -> (Vec<String>, Vec<Value>, bool) {
    let mut it = table.into_iter();
    let Some(first) = it.next() else { return (Vec::new(), Vec::new(), false) };
    let headers = headers_of(&first);
    let mut rows = Vec::new();
    let mut truncated = false;
    for r in it {
        if r.iter().all(Value::is_null) {
            continue;
        }
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut row = serde_json::Map::new();
        for (i, h) in headers.iter().enumerate() {
            row.insert(h.clone(), r.get(i).cloned().unwrap_or(Value::Null));
        }
        rows.push(Value::Object(row));
    }
    (headers, rows, truncated)
}

/// Read a CSV file as a table of JSON values (numbers stay numbers, everything else text).
pub fn read_csv(path: &Path) -> Result<Vec<Vec<Value>>, String> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_path(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("{}: {e}", path.display()))?;
        out.push(
            rec.iter()
                .map(|s| {
                    let t = s.trim();
                    if t.is_empty() {
                        Value::Null
                    } else if let Ok(i) = t.parse::<i64>() {
                        json!(i)
                    } else if let Ok(f) = t.parse::<f64>() {
                        json!(f)
                    } else {
                        Value::String(s.to_string())
                    }
                })
                .collect(),
        );
    }
    Ok(out)
}

/// Read one sheet of a workbook (the named one, else the first) as a table.
pub fn read_workbook(path: &Path, sheet: Option<&str>) -> Result<(String, Vec<String>, Vec<Vec<Value>>), String> {
    use calamine::Reader;
    let mut wb = calamine::open_workbook_auto(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let names: Vec<String> = wb.sheet_names().to_vec();
    let name = match sheet {
        Some(s) => names.iter().find(|n| n.eq_ignore_ascii_case(s)).cloned().ok_or_else(|| format!("{} has no sheet called “{s}”; it has {}", path.display(), names.join(", ")))?,
        None => names.first().cloned().ok_or_else(|| format!("{} has no sheets", path.display()))?,
    };
    let range = wb.worksheet_range(&name).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: Vec<Vec<Value>> = range.rows().map(|r| r.iter().map(cell_to_json).collect()).collect();
    Ok((name, names, table))
}

#[async_trait::async_trait]
impl Tool for SheetRead {
    fn id(&self) -> &str {
        "sheet.read"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Read a spreadsheet (.csv, .xlsx, .xlsm, .xls, .ods) as rows keyed by its header row. One sheet at a time: the named one, else the first.".into(),
            returns: "`{ path, sheet, sheets: [names], headers: [...], rows: [{ header: value }], count, truncated }`; point a `table` output at `.rows`, or hand `.rows` to a model step.".into(),
            parameters: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "The spreadsheet file." },
                    "sheet": { "type": "string", "description": "A sheet name (workbooks only); the first sheet when left out." },
                    "maxRows": { "type": "integer", "description": "Keep at most this many rows (default 5000)." }
                }
            }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs.get("path").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("path is required".into()))?;
        let path = ctx.check_read(raw)?;
        let max_rows = inputs.get("maxRows").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(MAX_ROWS).clamp(1, 50_000);
        let sheet = inputs.get("sheet").and_then(Value::as_str).map(String::from);
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
        let p = path.clone();
        let (name, names, table) = tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>, Vec<Vec<Value>>), String> {
            if ext == "csv" || ext == "tsv" || ext == "txt" {
                Ok(("csv".into(), vec!["csv".into()], read_csv(&p)?))
            } else {
                read_workbook(&p, sheet.as_deref())
            }
        })
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?
        .map_err(ToolError::Failed)?;
        let (headers, rows, truncated) = rows_from(table, max_rows);
        Ok(json!({ "path": path.to_string_lossy(), "sheet": name, "sheets": names, "headers": headers, "count": rows.len(), "rows": rows, "truncated": truncated }))
    }
}

/// `sheet.write`: rows out as an Excel file, or CSV when the name ends in .csv.
pub struct SheetWrite;

/// The column order: the given one, else every key in the order first seen.
fn columns_of(rows: &[Value], given: Option<&Vec<Value>>) -> Vec<String> {
    if let Some(g) = given {
        return g.iter().filter_map(Value::as_str).map(String::from).collect();
    }
    let mut cols: Vec<String> = Vec::new();
    for r in rows {
        if let Some(o) = r.as_object() {
            for k in o.keys() {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    cols
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn write_xlsx(path: &Path, sheet: &str, columns: &[String], rows: &[Value]) -> Result<(), String> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name(sheet).map_err(|e| e.to_string())?;
    let bold = rust_xlsxwriter::Format::new().set_bold();
    for (c, name) in columns.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, name, &bold).map_err(|e| e.to_string())?;
    }
    for (r, row) in rows.iter().enumerate() {
        let r = (r + 1) as u32;
        for (c, name) in columns.iter().enumerate() {
            let c = c as u16;
            match row.get(name) {
                Some(Value::Number(n)) => {
                    ws.write_number(r, c, n.as_f64().unwrap_or(0.0)).map_err(|e| e.to_string())?;
                }
                Some(Value::Bool(b)) => {
                    ws.write_boolean(r, c, *b).map_err(|e| e.to_string())?;
                }
                Some(v) if !v.is_null() => {
                    ws.write_string(r, c, cell_text(v)).map_err(|e| e.to_string())?;
                }
                _ => {}
            }
        }
    }
    ws.autofit();
    wb.save(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_csv(path: &Path, columns: &[String], rows: &[Value]) -> Result<(), String> {
    let mut w = csv::Writer::from_path(path).map_err(|e| format!("{}: {e}", path.display()))?;
    w.write_record(columns).map_err(|e| e.to_string())?;
    for row in rows {
        let rec: Vec<String> = columns.iter().map(|c| row.get(c).map(cell_text).unwrap_or_default()).collect();
        w.write_record(&rec).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())
}

#[async_trait::async_trait]
impl Tool for SheetWrite {
    fn id(&self) -> &str {
        "sheet.write"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn has_side_effects(&self) -> bool {
        true
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write rows as a spreadsheet: an Excel file (.xlsx), or CSV when the name ends in .csv. Give a full path, or a directory plus a file name. Parent folders are created.".into(),
            returns: "`{ path, sheet, columns, rows, count }`; point a `file` output at the step for the file and a `table` output at `.rows` to show it.".into(),
            parameters: json!({
                "type": "object",
                "required": ["rows"],
                "properties": {
                    "rows": { "type": "array", "description": "One object per row; keys are the column names." },
                    "columns": { "type": "array", "items": { "type": "string" }, "description": "Column order; every key in order of first appearance when left out." },
                    "path": { "type": "string", "description": "Absolute path of the file to write (.xlsx or .csv)." },
                    "dir": { "type": "string", "description": "Directory to write into (with name)." },
                    "name": { "type": "string", "description": "File name, ending in .xlsx or .csv (with dir)." },
                    "sheet": { "type": "string", "description": "The sheet's name inside an Excel file (default Sheet1)." },
                    "overwrite": { "type": "boolean", "description": "Replace an existing file (default false)." }
                }
            }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let rows = inputs.get("rows").and_then(Value::as_array).cloned().ok_or_else(|| ToolError::InvalidInput("rows (array of objects) is required".into()))?;
        let raw = match (inputs.get("path").and_then(Value::as_str), inputs.get("dir").and_then(Value::as_str)) {
            (Some(p), _) => p.to_string(),
            (None, Some(d)) => format!("{}/{}", d.trim_end_matches('/'), safe_file_name(inputs.get("name").and_then(Value::as_str).unwrap_or("table.xlsx"))),
            _ => return Err(ToolError::InvalidInput("path, or dir and name, is required".into())),
        };
        let path = ctx.check_write(&raw)?;
        if path.exists() && !inputs.get("overwrite").and_then(Value::as_bool).unwrap_or(false) {
            return Err(ToolError::Failed(format!("{} already exists (set overwrite to replace it)", path.display())));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| ToolError::Failed(format!("{}: {e}", parent.display())))?;
        }
        let columns = columns_of(&rows, inputs.get("columns").and_then(Value::as_array));
        let sheet = inputs.get("sheet").and_then(Value::as_str).unwrap_or("Sheet1").chars().take(31).collect::<String>();
        let is_csv = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("csv"));
        let (p, cols, rs, sh) = (path.clone(), columns.clone(), rows.clone(), sheet.clone());
        tokio::task::spawn_blocking(move || if is_csv { write_csv(&p, &cols, &rs) } else { write_xlsx(&p, &sh, &cols, &rs) })
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?
            .map_err(ToolError::Failed)?;
        Ok(json!({ "path": path.to_string_lossy(), "sheet": if is_csv { "csv".to_string() } else { sheet }, "columns": columns, "count": rows.len(), "rows": rows }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{FsPolicy, KeepAccess, NetPolicy};

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext { run_id: "t".into(), artifact_dir: dir.join("art"), fs: FsPolicy::open(), net: NetPolicy::none(), keeps: KeepAccess::default(), models: None, resolutions: Default::default() }
    }

    #[tokio::test]
    async fn rows_go_out_as_excel_and_csv_and_come_back_the_same() {
        let dir = std::env::temp_dir().join(format!("stillroom-sheets-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let rows = json!([{ "Name": "Ada", "Age": 36, "Done": true }, { "Name": "Grace", "Age": 45.5, "Done": false }, { "Name": "Linus", "Age": null, "Note": "no age" }]);
        let out = SheetWrite.call(&ctx(&dir), json!({ "dir": dir.to_string_lossy(), "name": "people.xlsx", "rows": rows, "sheet": "People" })).await.unwrap();
        assert_eq!(out["columns"], json!(["Name", "Age", "Done", "Note"]));
        assert_eq!(out["count"], 3);
        let back = SheetRead.call(&ctx(&dir), json!({ "path": out["path"] })).await.unwrap();
        assert_eq!(back["sheet"], "People");
        assert_eq!(back["sheets"], json!(["People"]));
        assert_eq!(back["headers"], json!(["Name", "Age", "Done", "Note"]));
        assert_eq!(back["rows"][0], json!({ "Name": "Ada", "Age": 36.0, "Done": true, "Note": null }));
        assert_eq!(back["rows"][1]["Age"], 45.5);
        assert_eq!(back["rows"][2], json!({ "Name": "Linus", "Age": null, "Done": null, "Note": "no age" }));
        assert_eq!(back["truncated"], false);
        // CSV by extension, with an explicit column order; numbers come back as numbers.
        let out = SheetWrite.call(&ctx(&dir), json!({ "path": dir.join("people.csv").to_string_lossy(), "rows": rows, "columns": ["Age", "Name"] })).await.unwrap();
        assert_eq!(out["sheet"], "csv");
        let back = SheetRead.call(&ctx(&dir), json!({ "path": out["path"], "maxRows": 2 })).await.unwrap();
        assert_eq!(back["headers"], json!(["Age", "Name"]));
        assert_eq!(back["rows"], json!([{ "Age": 36, "Name": "Ada" }, { "Age": 45.5, "Name": "Grace" }]));
        assert_eq!(back["truncated"], true);
        // A second write refuses without overwrite.
        let err = SheetWrite.call(&ctx(&dir), json!({ "path": dir.join("people.csv").to_string_lossy(), "rows": rows })).await.unwrap_err();
        assert!(err.to_string().contains("already exists"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn headers_are_named_when_blank_and_made_unique_when_repeated() {
        assert_eq!(headers_of(&[json!("A"), Value::Null, json!("A"), json!(3)]), vec!["A", "Column 2", "A (2)", "3"]);
    }
}
