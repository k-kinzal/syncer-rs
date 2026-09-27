//! HTTP(S) transport. Auth comes only from local enrollment, never from policy data.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use std::{io::Read, time::Duration};
use syncer_extension_sdk::{MAX_BYTES, Request, decode, encode, string};
fn response(mut response: Response) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "HTTP request failed with status {}",
        response.status()
    );
    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut bytes = vec![];
    (&mut response)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "HTTP response exceeds 16 MiB limit"
    );
    Ok(json!({"data":encode(&bytes),"revision":etag}))
}
pub fn dispatch(request: Request) -> Result<Value> {
    if request.method == "manifest" {
        return Ok(
            json!({"abi_version":1,"name":"http","version":env!("CARGO_PKG_VERSION"),"methods":["fetch","publish","report"],"schemes":["http","https"],"kinds":[],"targets":[]}),
        );
    }
    let p = &request.params;
    let uri = url::Url::parse(string(p, "uri")?)?;
    ensure!(
        matches!(uri.scheme(), "http" | "https")
            && uri.username().is_empty()
            && uri.password().is_none(),
        "expected HTTP(S) URL without embedded credentials"
    );
    // Redirects are deliberately surfaced: credentials and report ciphertext go only to the enrolled endpoint.
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let builder = match request.method.as_str() {
        "fetch" => client.get(uri.clone()),
        "publish" => client.put(uri.clone()).body(decode(&p["data"])?),
        "report" => client
            .post(uri.clone())
            .header("Content-Type", "application/octet-stream")
            .body(decode(&p["data"])?),
        _ => bail!("unsupported HTTP method"),
    };
    let mut builder = if let Some(token) = p["auth"]["access_token"].as_str() {
        ensure!(
            uri.scheme() == "https"
                || matches!(uri.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
            "credentials require HTTPS"
        );
        builder.bearer_auth(token)
    } else {
        builder
    };
    if let Some(revision) = p["revision"].as_str() {
        builder = builder.header("If-Match", revision);
    }
    response(builder.send().context("HTTP transport failed")?)
}
syncer_extension_sdk::export_extension!(dispatch);
