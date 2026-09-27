use anyhow::{Context as _, Result, ensure};
use std::path::{Path, PathBuf};
use syncer_core::{Extensions, extension::Installed, storage};

/// An explicit local trust decision; policy files cannot enable this mode.
pub struct Development {
    directory: Option<PathBuf>,
}

impl Development {
    pub fn new(enabled: bool) -> Result<Self> {
        let directory = if enabled {
            let executable = std::env::current_exe()?.canonicalize()?;
            Some(
                executable
                    .parent()
                    .context("executable has no parent directory")?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(Self { directory })
    }

    pub async fn load(&self, installed: &[Installed]) -> Result<Extensions> {
        let mut fallbacks = Vec::new();
        if let Some(directory) = &self.directory {
            // Only known official build outputs, never arbitrary libraries or cwd.
            for (name, basename) in [
                ("http", "http"),
                ("git", "git"),
                ("google-drive", "google_drive"),
                ("json", "json"),
                ("structured", "structured"),
                ("claude", "claude"),
            ] {
                fallbacks.push((
                    name,
                    directory.join(format!(
                        "{}syncer_extension_{basename}{}",
                        std::env::consts::DLL_PREFIX,
                        std::env::consts::DLL_SUFFIX,
                    )),
                ));
            }
        }
        Extensions::load_with_fallbacks(installed, &fallbacks).await
    }

    pub fn check_target(&self, path: &Path) -> Result<()> {
        if let Some(directory) = &self.directory {
            ensure!(
                !storage::is_within(path, directory),
                "policies cannot write inside the development extension directory"
            );
        }
        Ok(())
    }
}
