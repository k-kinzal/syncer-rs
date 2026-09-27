use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use syncer_core::{Extensions, extension::Installed, storage};
use syncer_extension_sdk::{decode, encode};
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub extensions: Vec<Installed>,
    #[serde(default)]
    pub reports: Vec<Reporter>,
    #[serde(default)]
    pub installation_secret: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub uri: String,
    pub extension: Option<String>,
    #[serde(default)]
    pub auth_env: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    #[serde(default)]
    pub asset: bool,
    pub name: String,
    pub endpoint: Endpoint,
    pub priority: i32,
    pub roots: Vec<PathBuf>,
    pub sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reporter {
    pub policy: String,
    pub endpoint: Endpoint,
    pub recipient: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cache {
    pub data: String,
    pub revision: Option<String>,
    pub fetched_at: u64,
    pub digest: String,
}
impl Config {
    pub fn read(state: &Path) -> Result<Self> {
        match storage::read_optional(&state.join("config.json"))? {
            Some(data) => {
                Ok(serde_json::from_slice(&data).context("invalid local enrollment config")?)
            }
            None => Ok(Self::default()),
        }
    }
    pub fn save(&self, state: &Path) -> Result<()> {
        storage::atomic_write(
            &state.join("config.json"),
            &serde_json::to_vec_pretty(self)?,
            None,
        )
    }
}
impl Endpoint {
    pub fn local_path(&self) -> Result<Option<PathBuf>> {
        if self.extension.is_some() {
            return Ok(None);
        }
        if self.uri.starts_with("file://") {
            return Ok(Some(
                url::Url::parse(&self.uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file URL"))?,
            ));
        }
        if !self.uri.contains("://") {
            return Ok(Some(PathBuf::from(&self.uri)));
        }
        Ok(None)
    }
    pub fn new(
        uri: &str,
        extension: Option<String>,
        credentials: &[String],
        project: &Path,
    ) -> Result<Self> {
        let mut auth_env = BTreeMap::new();
        for pair in credentials {
            let (key, value) = pair
                .split_once('=')
                .context("credential must be key=ENV_VAR_NAME, never a secret value")?;
            ensure!(
                !key.is_empty()
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && !value.is_empty(),
                "invalid credential environment variable"
            );
            auth_env.insert(key.into(), value.into());
        }
        let uri = if !uri.contains("://") {
            let p = Path::new(uri);
            storage::normalize(&if p.is_absolute() {
                p.into()
            } else {
                project.join(p)
            })?
            .to_string_lossy()
            .into_owned()
        } else {
            uri.into()
        };
        Ok(Self {
            uri,
            extension,
            auth_env,
        })
    }
    pub async fn call(
        &self,
        extensions: &Extensions,
        method: &str,
        data: Option<&[u8]>,
        revision: Option<&str>,
    ) -> Result<Value> {
        if self.extension.is_none()
            && (!self.uri.contains("://") || self.uri.starts_with("file://"))
        {
            let path = if self.uri.starts_with("file://") {
                url::Url::parse(&self.uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file URL"))?
            } else {
                PathBuf::from(&self.uri)
            };
            return match method {
                "fetch" => {
                    let bytes =
                        storage::read_optional(&path)?.context("local source does not exist")?;
                    Ok(json!({"data":encode(&bytes),"revision":syncer_core::digest(&bytes)}))
                }
                "publish" => {
                    if let Some(revision) = revision {
                        let current =
                            storage::read_optional(&path)?.context("local source was removed")?;
                        ensure!(
                            syncer_core::digest(&current) == revision,
                            "source changed since last fetch; refresh before pushing"
                        );
                    }
                    storage::atomic_write(
                        &path,
                        data.context("missing publish data")?,
                        std::fs::metadata(&path).ok().map(|m| m.permissions()),
                    )?;
                    Ok(json!({}))
                }
                "report" => {
                    let data = data.context("missing encrypted report")?;
                    storage::private_dir(&path)?;
                    let out = path.join(format!("{}.age", uuid::Uuid::new_v4()));
                    storage::atomic_write(&out, data, None)?;
                    Ok(json!({"path":out}))
                }
                _ => anyhow::bail!("unsupported local source method"),
            };
        }
        let mut auth = serde_json::Map::new();
        for (key, var) in &self.auth_env {
            auth.insert(
                key.clone(),
                Value::String(std::env::var(var).with_context(|| {
                    format!("credential environment variable {var} is not set")
                })?),
            );
        }
        let params =
            json!({"uri":self.uri,"auth":auth,"data":data.map(encode),"revision":revision});
        if let Some(name) = &self.extension {
            extensions.named(name, method, params).await
        } else {
            let scheme = self
                .uri
                .split_once("://")
                .context("source requires a URI scheme")?
                .0;
            extensions
                .capability("scheme", scheme, method, params)
                .await
        }
    }
}
pub async fn fetch(
    source: &Source,
    extensions: &Extensions,
    state: &Path,
    offline: bool,
) -> Result<Cache> {
    let cache = if offline {
        let bytes = storage::read_optional_limit(
            &state.join("cache").join(format!("{}.json", source.name)),
            syncer_extension_sdk::MAX_BYTES * 2,
        )?
        .context("no offline cache; run syncer fetch first")?;
        serde_json::from_slice::<Cache>(&bytes)?
    } else {
        let response = source
            .endpoint
            .call(extensions, "fetch", None, None)
            .await?;
        let bytes = decode(&response["data"])?;
        Cache {
            data: if source.asset {
                encode(&bytes)
            } else {
                String::from_utf8(bytes.clone()).context("policy must be UTF-8")?
            },
            revision: response["revision"].as_str().map(str::to_owned),
            digest: syncer_core::digest(&bytes),
            fetched_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        }
    };
    ensure!(
        syncer_core::digest(&cache_bytes(source, &cache)?) == cache.digest,
        "cache digest mismatch"
    );
    if let Some(pin) = &source.sha256 {
        ensure!(
            &cache.digest == pin,
            "source {} does not match pinned SHA-256",
            source.name
        );
    }
    if !source.asset {
        syncer_language::parse(&cache.data).with_context(|| format!("source {}", source.name))?;
    }
    Ok(cache)
}
pub fn save_cache(source: &Source, cache: &Cache, state: &Path) -> Result<()> {
    storage::atomic_write(
        &state.join("cache").join(format!("{}.json", source.name)),
        &serde_json::to_vec(cache)?,
        None,
    )
}

pub fn cache_bytes(source: &Source, cache: &Cache) -> Result<Vec<u8>> {
    if source.asset {
        decode(&Value::String(cache.data.clone()))
    } else {
        Ok(cache.data.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_hosts_do_not_implicitly_select_a_service_extension() {
        let project = std::env::current_dir().unwrap();
        for uri in [
            "https://drive.google.com/file/d/example/view",
            "https://docs.google.com/document/d/example/edit",
            "https://config.example.com/policy.hcl",
        ] {
            let endpoint = Endpoint::new(uri, None, &[], &project).unwrap();
            assert!(endpoint.extension.is_none());
            let explicit = Endpoint::new(uri, Some("custom-source".into()), &[], &project).unwrap();
            assert_eq!(explicit.extension.as_deref(), Some("custom-source"));
        }
    }
}
