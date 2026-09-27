use anyhow::{Context, Result, bail, ensure};
use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::Arc,
};
use syncer_extension_sdk::{ABI_VERSION, Manifest, Request, Response};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub path: PathBuf,
    pub sha256: String,
}
#[async_trait::async_trait]
pub trait Extension: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value>;
}
struct Native {
    _library: Library,
    invoke: unsafe extern "C" fn(*const c_char) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_char),
    mutex: std::sync::Mutex<()>,
}
impl Native {
    fn call(&self, request: Request) -> Result<Value> {
        let _lock = self
            .mutex
            .lock()
            .map_err(|_| anyhow::anyhow!("extension lock poisoned"))?;
        let input = CString::new(serde_json::to_vec(&request)?)?;
        // SAFETY: function symbols are checked at load, library lives with Native. Buffers use the allocating library's free function.
        unsafe {
            let output = (self.invoke)(input.as_ptr());
            ensure!(!output.is_null(), "extension returned null");
            let bytes = CStr::from_ptr(output).to_bytes().to_vec();
            (self.free)(output);
            ensure!(
                bytes.len() <= syncer_extension_sdk::MAX_BYTES * 3,
                "extension response too large"
            );
            let response: Response =
                serde_json::from_slice(&bytes).context("invalid extension response")?;
            if let Some(error) = response.error {
                bail!("{error}");
            }
            response.result.context("extension returned no result")
        }
    }
}
struct NativeAsync(Arc<Native>);
#[async_trait::async_trait]
impl Extension for NativeAsync {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let native = self.0.clone();
        let request = Request {
            method: method.into(),
            params,
        };
        tokio::task::spawn_blocking(move || native.call(request)).await?
    }
}
#[derive(Default)]
pub struct Extensions {
    entries: BTreeMap<String, (Manifest, Arc<dyn Extension>)>,
}
impl Extensions {
    pub async fn load(installed: &[Installed]) -> Result<Self> {
        let mut out = Self::default();
        for install in installed {
            crate::storage::reject_symlinks(&install.path)?;
            ensure!(
                crate::digest(&std::fs::read(&install.path)?) == install.sha256,
                "extension digest changed: {}",
                install.path.display()
            );
            // SAFETY: native extensions are explicitly installed trusted code. Rust ABI never crosses this boundary.
            let native = unsafe {
                let library = Library::new(&install.path).context("load native extension")?;
                let abi = *library
                    .get::<unsafe extern "C" fn() -> u32>(b"syncer_extension_abi_version\0")?;
                ensure!(abi() == ABI_VERSION, "incompatible extension ABI");
                Native {
                    invoke: *library.get(b"syncer_extension_invoke\0")?,
                    free: *library.get(b"syncer_extension_free\0")?,
                    _library: library,
                    mutex: std::sync::Mutex::new(()),
                }
            };
            let ext: Arc<dyn Extension> = Arc::new(NativeAsync(Arc::new(native)));
            let manifest: Manifest =
                serde_json::from_value(ext.call("manifest", json!({})).await?)?;
            ensure!(
                manifest.abi_version == ABI_VERSION,
                "incompatible manifest ABI"
            );
            ensure!(
                syncer_language::valid_name(&manifest.name),
                "invalid extension name"
            );
            ensure!(
                !out.entries.contains_key(&manifest.name),
                "duplicate extension {}",
                manifest.name
            );
            for (other, _) in out.entries.values() {
                ensure!(
                    !manifest.schemes.iter().any(|x| other.schemes.contains(x))
                        && !manifest.kinds.iter().any(|x| other.kinds.contains(x))
                        && !manifest.targets.iter().any(|x| other.targets.contains(x)),
                    "extension capabilities conflict: {} and {}",
                    manifest.name,
                    other.name
                );
            }
            ensure!(
                !manifest.schemes.iter().any(|x| x == "file")
                    && !manifest
                        .kinds
                        .iter()
                        .any(|x| matches!(x.as_str(), "file" | "text" | "regex")),
                "extension cannot replace built-in capabilities"
            );
            out.entries.insert(manifest.name.clone(), (manifest, ext));
        }
        Ok(out)
    }
    pub fn manifests(&self) -> Vec<&Manifest> {
        self.entries.values().map(|x| &x.0).collect()
    }
    pub async fn named(&self, name: &str, method: &str, params: Value) -> Result<Value> {
        let (manifest, ext) = self.entries.get(name).with_context(|| {
            format!("extension '{name}' is not installed; use syncer extension install")
        })?;
        ensure!(
            manifest.methods.iter().any(|m| m == method),
            "extension {name} does not support {method}"
        );
        ext.call(method, params)
            .await
            .with_context(|| format!("extension {name}, method {method}"))
    }
    pub async fn capability(
        &self,
        category: &str,
        name: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let (manifest, _) = self
            .entries
            .values()
            .find(|(m, _)| {
                match category {
                    "scheme" => &m.schemes,
                    "kind" => &m.kinds,
                    _ => &m.targets,
                }
                .iter()
                .any(|x| x == name)
            })
            .with_context(|| format!("no installed extension for {category} '{name}'"))?;
        self.named(&manifest.name, method, params).await
    }
}
