//! Claude file target aliases, resolved using explicit host context.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::Path;
use syncer_extension_sdk::{Request, string};
pub fn dispatch(request: Request) -> Result<Value> {
    match request.method.as_str() {
        "manifest" => Ok(
            json!({"abi_version":1,"name":"claude","version":env!("CARGO_PKG_VERSION"),"methods":["resolve"],"targets":["claude"],"schemes":[],"kinds":[]}),
        ),
        "resolve" => {
            let p = &request.params;
            let target = string(p, "target")?;
            let home = Path::new(string(p, "home")?);
            let project = Path::new(string(p, "project")?);
            let path = match target {
                "claude://user/settings" => home.join(".claude/settings.json"),
                "claude://user/instructions" => home.join(".claude/CLAUDE.md"),
                "claude://project/settings" => project.join(".claude/settings.json"),
                "claude://project/local" => project.join(".claude/settings.local.json"),
                "claude://project/instructions" => project.join("CLAUDE.md"),
                _ => bail!(
                    "unknown Claude target; use user/settings, user/instructions, project/settings, project/local or project/instructions"
                ),
            };
            Ok(json!({"path":path}))
        }
        _ => bail!("unsupported Claude method"),
    }
}
syncer_extension_sdk::export_extension!(dispatch);
