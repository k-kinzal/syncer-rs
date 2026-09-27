use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Write};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Format {
    #[default]
    Human,
    Json,
    Jsonl,
    Yaml,
    Table,
    #[value(alias = "tsv")]
    Text,
}

pub enum View {
    Message,
    Sources,
    Extensions,
    Fetch,
    Plan { dry_run: bool },
    Reporters,
    ReportSummary,
}

pub struct Output {
    format: Format,
    query: Option<jmespath::Expression<'static>>,
    quiet: bool,
}

impl Output {
    pub fn new(format: Format, query: Option<&str>, quiet: bool) -> Result<Self> {
        Ok(Self {
            format,
            query: query
                .map(jmespath::compile)
                .transpose()
                .context("invalid --query (JMESPath)")?,
            quiet,
        })
    }

    /// Evaluate and render before performing the requested mutation. Output
    /// selection never changes the plan, compliance checks or command exit code.
    pub fn prepare(&self, view: View, data: impl Serialize) -> Result<String> {
        let data = serde_json::to_value(data)?;
        let data = if let Some(query) = &self.query {
            let selected = query
                .search(&data)
                .context("cannot evaluate --query (JMESPath)")?;
            serde_json::to_value(selected.as_ref())?
        } else {
            data
        };
        if self.quiet {
            return Ok(String::new());
        }
        let text = match self.format {
            Format::Json => serde_json::to_string_pretty(&data)?,
            Format::Jsonl => serde_json::to_string(&data)?,
            Format::Yaml => serde_norway::to_string(&data)?,
            Format::Table => generic(&data, true),
            Format::Text => generic(&data, false),
            Format::Human if self.query.is_some() => generic(&data, true),
            Format::Human => human(view, &data),
        };
        Ok(if text.ends_with('\n') || text.is_empty() {
            text
        } else {
            text + "\n"
        })
    }

    pub fn print(&self, view: View, data: impl Serialize) -> Result<()> {
        write(&self.prepare(view, data)?)
    }

    pub fn notice(&self, message: &str) {
        if !self.quiet {
            eprintln!("{}", safe(message));
        }
    }
}

pub fn write(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::BrokenPipe {
                StdoutClosed.into()
            } else {
                error.into()
            }
        })
}

#[derive(Debug)]
struct StdoutClosed;
impl std::fmt::Display for StdoutClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stdout reader closed the pipe")
    }
}
impl std::error::Error for StdoutClosed {}

pub fn broken_pipe(error: &anyhow::Error) -> bool {
    // A broken network/source pipe is an operation failure, not a closed stdout.
    error.is::<StdoutClosed>()
}

// Keep names, paths and source text from injecting terminal control sequences.
fn safe(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn cell(value: &Value) -> String {
    match value {
        Value::String(s) => safe(s),
        other => other.to_string(),
    }
}

fn table(headers: &[String], rows: &[Vec<String>], with_headers: bool) -> String {
    if rows.is_empty() {
        return if with_headers {
            "No results.".into()
        } else {
            String::new()
        };
    }
    if !with_headers {
        return rows
            .iter()
            .map(|row| row.join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    let mut widths: Vec<_> = headers.iter().map(|s| s.width()).collect();
    for row in rows {
        for (width, value) in widths.iter_mut().zip(row) {
            *width = (*width).max(value.width());
        }
    }
    let line = |row: &[String]| {
        row.iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (s, width))| {
                if i + 1 == widths.len() {
                    s.clone()
                } else {
                    format!("{s}{}", " ".repeat(width - s.width() + 2))
                }
            })
            .collect::<String>()
    };
    let mut lines = vec![
        line(headers),
        widths
            .iter()
            .map(|n| "-".repeat(*n))
            .collect::<Vec<_>>()
            .join("  "),
    ];
    lines.extend(rows.iter().map(|row| line(row)));
    lines.join("\n")
}

fn generic(value: &Value, with_headers: bool) -> String {
    match value {
        Value::Array(values) if values.is_empty() => {
            if with_headers {
                "No results.".into()
            } else {
                String::new()
            }
        }
        Value::Array(values) if values.iter().all(Value::is_object) => {
            let keys: BTreeSet<_> = values
                .iter()
                .flat_map(|v| v.as_object().unwrap().keys())
                .collect();
            let headers = keys
                .iter()
                .map(|k| safe(k).to_uppercase())
                .collect::<Vec<_>>();
            let rows = values
                .iter()
                .map(|v| keys.iter().map(|k| cell(&v[*k])).collect())
                .collect::<Vec<_>>();
            table(&headers, &rows, with_headers)
        }
        Value::Array(values) if values.iter().all(Value::is_array) => {
            let width = values
                .iter()
                .map(|v| v.as_array().unwrap().len())
                .max()
                .unwrap_or(0);
            let headers = (1..=width)
                .map(|i| format!("COLUMN{i}"))
                .collect::<Vec<_>>();
            let rows = values
                .iter()
                .map(|v| (0..width).map(|i| cell(&v[i])).collect())
                .collect::<Vec<_>>();
            table(&headers, &rows, with_headers)
        }
        Value::Array(values) => values.iter().map(cell).collect::<Vec<_>>().join("\n"),
        Value::Object(values) if with_headers => {
            let rows = values
                .iter()
                .map(|(key, v)| vec![safe(key), cell(v)])
                .collect::<Vec<_>>();
            table(&["FIELD".into(), "VALUE".into()], &rows, true)
        }
        Value::Object(values) => values.values().map(cell).collect::<Vec<_>>().join("\t"),
        scalar => cell(scalar),
    }
}

fn records(data: &Value, headers: &[&str], fields: &[&str], empty: &str) -> String {
    let Some(values) = data.as_array().filter(|v| !v.is_empty()) else {
        return empty.into();
    };
    let rows = values
        .iter()
        .map(|v| {
            fields
                .iter()
                .map(|field| cell(v.pointer(field).unwrap_or(&Value::Null)))
                .collect()
        })
        .collect::<Vec<_>>();
    table(
        &headers.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
        &rows,
        true,
    )
}

fn human(view: View, data: &Value) -> String {
    match view {
        View::Message => data["message"]
            .as_str()
            .map(safe)
            .unwrap_or_else(|| generic(data, true)),
        View::Sources => records(
            data,
            &["NAME", "PRIORITY", "SOURCE"],
            &["/name", "/priority", "/endpoint/uri"],
            "No sources enrolled. Use syncer add NAME PATH_OR_URL.",
        ),
        View::Fetch => records(
            data,
            &["NAME", "SHA256"],
            &["/name", "/sha256"],
            "No sources to fetch.",
        ),
        View::Reporters => records(
            data,
            &["POLICY", "DESTINATION", "RECIPIENT"],
            &["/policy", "/endpoint/uri", "/recipient"],
            "No reporting configured.",
        ),
        View::Extensions => {
            let rows = data.as_array().into_iter().flatten().map(|m| {
                let mut capabilities = Vec::new();
                for (field, label) in [("schemes", "sources"), ("kinds", "formats"), ("targets", "targets")] {
                    let values = m[field].as_array().into_iter().flatten().map(cell).collect::<Vec<_>>();
                    if !values.is_empty() { capabilities.push(format!("{label}: {}", values.join(", "))); }
                }
                json!({"name":m["name"],"version":m["version"],"capabilities":capabilities.join("; ")})
            }).collect::<Vec<_>>();
            records(
                &json!(rows),
                &["NAME", "VERSION", "CAPABILITIES"],
                &["/name", "/version", "/capabilities"],
                "No extensions enabled. Use syncer extension install LIBRARY or --dev-extensions for a local build.",
            )
        }
        View::ReportSummary => format!(
            "{}\n\n{}",
            cell(&data["scope"]),
            records(
                &data["rules"],
                &[
                    "POLICY",
                    "RULE",
                    "OBSERVED",
                    "COMPLIANT",
                    "VIOLATING",
                    "MAX DAYS TO RESOLVE"
                ],
                &[
                    "/policy",
                    "/id",
                    "/observed_devices",
                    "/compliant_devices",
                    "/violating_devices",
                    "/max_observed_resolution_days"
                ],
                "No reports observed."
            )
        ),
        View::Plan { dry_run } => {
            let count = data["changed_files"].as_u64().unwrap_or(0);
            let compliant = data["compliant"].as_bool().unwrap_or(false);
            let mut lines = vec![if !compliant {
                "Unresolved violations; no files changed.".into()
            } else if count == 0 {
                "No changes needed. All rules are satisfied.".into()
            } else if dry_run {
                format!("Dry run: {count} file(s) would change. No files written.")
            } else {
                format!("Applied changes to {count} file(s).")
            }];
            for file in data["files"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "  {}{}",
                    cell(&file["path"]),
                    if file["sensitive"] == true {
                        " (sensitive; diff hidden)"
                    } else if file["binary"] == true {
                        " (binary)"
                    } else {
                        ""
                    }
                ));
                if let Some(diff) = file["diff"].as_str() {
                    lines.extend(diff.lines().map(safe));
                }
            }
            let rows = data["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|rule| {
                    let status = if rule["after"] != true {
                        "VIOLATION"
                    } else if rule["before"] == true {
                        "OK"
                    } else if !compliant || rule["after"] != true {
                        "VIOLATION"
                    } else if dry_run {
                        "WILL REPAIR"
                    } else {
                        "REPAIRED"
                    };
                    json!({"policy":rule["policy"],"id":rule["id"],"status":status})
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                lines.push(String::new());
                lines.push(records(
                    &json!(rows),
                    &["POLICY", "RULE", "STATUS"],
                    &["/policy", "/id", "/status"],
                    "",
                ));
            }
            lines.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_broken_pipes_are_not_treated_as_successful_output() {
        assert!(!broken_pipe(
            &std::io::Error::new(std::io::ErrorKind::BrokenPipe, "source transport failed").into()
        ));
        assert!(broken_pipe(
            &anyhow::Error::new(StdoutClosed).context("output")
        ));
    }

    #[test]
    fn display_escapes_control_characters_but_json_preserves_the_data() {
        let data = json!(["a\tb", "c\nd", "\u{1b}[31m", "日本語"]);
        let text = Output::new(Format::Text, None, false)
            .unwrap()
            .prepare(View::Message, &data)
            .unwrap();
        assert!(!text.contains('\u{1b}') && !text.contains('\t'));
        assert_eq!(text.lines().count(), 4);
        assert!(text.contains("a\\tb") && text.contains("c\\nd") && text.contains("日本語"));
        let machine = Output::new(Format::Json, None, false)
            .unwrap()
            .prepare(View::Message, &data)
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&machine).unwrap(), data);
    }
}
