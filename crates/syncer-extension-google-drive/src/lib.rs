//! Google Drive blob transport with OAuth access-token and refresh-token credentials.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{io::Read, time::Duration};
use syncer_extension_sdk::{MAX_BYTES, Request, decode, encode, string};
fn file_id(uri: &str) -> Result<String> {
    let u = url::Url::parse(uri)?;
    let id = if u.scheme() == "drive" {
        u.host_str()
            .context("drive:// requires file or folder ID")?
            .to_owned()
    } else {
        ensure!(
            u.scheme() == "https"
                && matches!(u.host_str(), Some("drive.google.com" | "docs.google.com")),
            "expected a Google Drive HTTPS URL"
        );
        let parts: Vec<_> = u.path_segments().context("invalid Drive path")?.collect();
        if let Some(i) = parts.iter().position(|x| *x == "d" || *x == "folders") {
            parts.get(i + 1).context("missing Drive ID")?.to_string()
        } else {
            u.query_pairs()
                .find(|(k, _)| k == "id")
                .context("URL has no Drive file ID")?
                .1
                .into_owned()
        }
    };
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "invalid Drive file ID"
    );
    Ok(id)
}
fn token(client: &Client, p: &Value) -> Result<String> {
    if let Some(token) = p["auth"]["access_token"].as_str() {
        return Ok(token.into());
    }
    let auth = &p["auth"];
    let refresh = string(auth, "refresh_token")
        .context("Drive requires locally configured access_token or refresh_token credentials")?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", string(auth, "client_id")?),
    ];
    if let Some(secret) = auth["client_secret"].as_str() {
        form.push(("client_secret", secret));
    }
    let response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&form)
        .send()?;
    ensure!(
        response.status().is_success(),
        "Google OAuth refresh failed ({})",
        response.status()
    );
    let body: Value = response.json()?;
    Ok(string(&body, "access_token")?.into())
}
pub fn dispatch(request: Request) -> Result<Value> {
    if request.method == "manifest" {
        return Ok(
            json!({"abi_version":1,"name":"google-drive","version":env!("CARGO_PKG_VERSION"),"methods":["fetch","publish","report"],"schemes":["drive"],"kinds":[],"targets":[]}),
        );
    }
    let p = &request.params;
    let id = file_id(string(p, "uri")?)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let token = token(&client, p)?;
    let builder = match request.method.as_str() {
        "fetch" => client
            .get(format!("https://www.googleapis.com/drive/v3/files/{id}"))
            .query(&[("alt", "media"), ("supportsAllDrives", "true")]),
        "publish" => client
            .patch(format!(
                "https://www.googleapis.com/upload/drive/v3/files/{id}"
            ))
            .query(&[("uploadType", "media"), ("supportsAllDrives", "true")])
            .header("Content-Type", "application/octet-stream")
            .body(decode(&p["data"])?),
        "report" => {
            let data = decode(&p["data"])?;
            ensure!(
                data.starts_with(b"age-encryption.org/v1\n"),
                "Drive reports must be age encrypted"
            );
            let name = format!("{}.age", uuid::Uuid::new_v4());
            let boundary = format!("syncer-{}", uuid::Uuid::new_v4());
            let metadata = serde_json::to_string(
                &json!({"name":name,"parents":[id],"mimeType":"application/octet-stream"}),
            )?;
            let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend(data);
            body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
            client
                .post("https://www.googleapis.com/upload/drive/v3/files")
                .query(&[("uploadType", "multipart"), ("supportsAllDrives", "true")])
                .header(
                    "Content-Type",
                    format!("multipart/related; boundary={boundary}"),
                )
                .body(body)
        }
        _ => bail!("unsupported Drive method"),
    };
    let builder = if let Some(revision) = p["revision"].as_str() {
        builder.header("If-Match", revision)
    } else {
        builder
    };
    let mut response = builder.bearer_auth(token).send()?;
    ensure!(
        response.status().is_success(),
        "Google Drive request failed ({})",
        response.status()
    );
    let revision = response
        .headers()
        .get("etag")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let mut data = vec![];
    (&mut response)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= MAX_BYTES, "Drive content exceeds 16 MiB");
    Ok(json!({"data":encode(&data),"revision":revision}))
}
syncer_extension_sdk::export_extension!(dispatch);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_links() {
        assert_eq!(
            file_id("https://drive.google.com/file/d/ABC-123/view").unwrap(),
            "ABC-123"
        );
        assert_eq!(file_id("drive://ABC-123").unwrap(), "ABC-123");
        assert!(file_id("https://evil.test/file/d/x/view").is_err());
    }
}
