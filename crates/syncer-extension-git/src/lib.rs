//! Read a single policy file from a Git repository. No checkout, hooks or repository commands run.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use syncer_extension_sdk::{MAX_BYTES, Request, encode, string};
fn git(args: &[&str], dir: &std::path::Path) -> Result<Vec<u8>> {
    let out = tempfile::NamedTempFile::new()?;
    let err = tempfile::NamedTempFile::new()?;
    let mut child = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "credential.interactive=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .stdin(Stdio::null())
        .stdout(out.reopen()?)
        .stderr(err.reopen()?)
        .spawn()
        .context("git executable is required")?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git operation timed out after 60 seconds");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    ensure!(
        status.success(),
        "Git command failed ({status}); check repository access and ref"
    );
    let mut data = vec![];
    out.reopen()?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= MAX_BYTES, "Git output exceeds 16 MiB");
    Ok(data)
}
pub fn dispatch(request: Request) -> Result<Value> {
    if request.method == "manifest" {
        return Ok(
            json!({"abi_version":1,"name":"git","version":env!("CARGO_PKG_VERSION"),"methods":["fetch"],"schemes":["git+https","git+ssh"],"kinds":[],"targets":[]}),
        );
    }
    ensure!(
        request.method == "fetch",
        "Git extension is read-only; publish policies with your usual Git workflow"
    );
    let raw = string(&request.params, "uri")?
        .strip_prefix("git+")
        .context("expected git+https or git+ssh URI")?;
    let mut uri = url::Url::parse(raw)?;
    ensure!(
        matches!(uri.scheme(), "https" | "ssh") && uri.password().is_none(),
        "unsupported Git protocol or embedded password"
    );
    ensure!(
        uri.scheme() == "ssh" || uri.username().is_empty(),
        "HTTPS Git URLs must not contain credentials"
    );
    let path = uri
        .fragment()
        .context("Git URL requires #path/to/policy.hcl")?
        .to_owned();
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains('\\')
            && path.split('/').all(|x| !matches!(x, ".." | "." | ""))
            && !path.contains(':')
            && !path.contains('%'),
        "Git policy path must be a plain relative path"
    );
    let reference = uri
        .query_pairs()
        .find(|(k, _)| k == "ref")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| "HEAD".into());
    ensure!(
        !reference.starts_with('-')
            && reference
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_/ .".contains(&c))
            && !reference.contains(' '),
        "invalid Git ref"
    );
    uri.set_fragment(None);
    uri.set_query(None);
    let temp = tempfile::tempdir()?;
    git(&["init", "--bare", "."], temp.path())?;
    git(
        &["fetch", "--depth=1", "--no-tags", uri.as_str(), &reference],
        temp.path(),
    )?;
    let revision = String::from_utf8(git(&["rev-parse", "FETCH_HEAD"], temp.path())?)?
        .trim()
        .to_owned();
    let object = format!("{revision}:{path}");
    let data = git(&["show", &object], temp.path())?;
    Ok(json!({"data":encode(&data),"revision":revision}))
}
syncer_extension_sdk::export_extension!(dispatch);
