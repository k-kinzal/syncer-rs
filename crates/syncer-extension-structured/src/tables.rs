use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
use syncer_language::Rule;

pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let delimiter = if rule.kind == "tsv" { b'\t' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(content.as_bytes());
    let headers = reader.headers()?.clone();
    ensure!(
        !headers.is_empty() && headers.iter().all(|h| !h.is_empty()),
        "CSV/TSV requires a nonempty header row"
    );
    ensure!(
        headers.iter().collect::<BTreeSet<_>>().len() == headers.len(),
        "CSV/TSV header names must be unique"
    );
    let header_end = reader.position().byte() as usize;
    let mut records = Vec::new();
    let mut ranges = Vec::new();
    let mut record = csv::StringRecord::new();
    while reader.read_record(&mut record)? {
        let start = record.position().context("CSV row has no position")?.byte() as usize;
        ranges.push(start..reader.position().byte() as usize);
        records.push(Value::Object(
            headers
                .iter()
                .zip(record.iter())
                .map(|(k, v)| (k.into(), Value::String(v.into())))
                .collect(),
        ));
    }
    let mut expected = Value::Array(records.clone());
    let edit = syncer_document::apply(&mut expected, rule, action)?;
    if edit.replacement.is_none() {
        return Ok((content.into(), edit.compliant));
    }
    let rows = expected
        .as_array()
        .context("CSV document must be an array of row objects")?;
    let mut output = content[..header_end].to_string();
    for (index, row) in rows.iter().enumerate() {
        let row = row.as_object().context("CSV row must be an object")?;
        ensure!(
            row.len() == headers.len() && headers.iter().all(|k| row.contains_key(k)),
            "CSV row must retain every header field; set a cell to an empty string instead of removing it"
        );
        let cells = headers
            .iter()
            .map(|key| row[key].as_str().context("CSV/TSV cells must be strings"))
            .collect::<Result<Vec<_>>>()?;
        if index < records.len() && records[index].as_object() == Some(row) {
            output.push_str(&content[ranges[index].clone()]);
        } else {
            if !output.ends_with('\n') && !output.is_empty() {
                output.push('\n');
            }
            let terminator = if content.contains("\r\n") {
                csv::Terminator::CRLF
            } else {
                csv::Terminator::Any(b'\n')
            };
            let mut writer = csv::WriterBuilder::new()
                .delimiter(delimiter)
                .terminator(terminator)
                .from_writer(Vec::new());
            writer.write_record(cells)?;
            output.push_str(std::str::from_utf8(&writer.into_inner()?)?);
        }
    }
    // The parser enforces row widths and correct quoting after every edit.
    let mut checked = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(output.as_bytes());
    let checked_rows = checked
        .records()
        .map(|r| {
            Ok(Value::Object(
                headers
                    .iter()
                    .zip(r?.iter())
                    .map(|(k, v)| (k.into(), Value::String(v.into())))
                    .collect(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        Value::Array(checked_rows) == expected,
        "CSV/TSV edit did not round-trip"
    );
    Ok((output, edit.compliant))
}
