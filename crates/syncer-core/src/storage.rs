//! Filesystem primitives shared by enrollment and target transactions.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub fn reject_symlinks(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) => {
                ensure!(
                    !meta.file_type().is_symlink(),
                    "symlink paths are not allowed: {}",
                    prefix.display()
                );
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        meta.file_attributes() & 0x400 == 0,
                        "Windows reparse points are not allowed: {}",
                        prefix.display()
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn normalize(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "path must be absolute: {}",
        path.display()
    );
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                ensure!(out.pop(), "path escapes filesystem root");
            }
            Component::CurDir => {}
            _ => out.push(component),
        }
    }
    Ok(out)
}
pub fn checked_path(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = normalize(path)?;
    reject_symlinks(&path)?;
    ensure!(
        roots
            .iter()
            .any(|r| is_within(&path, r) && !same_path(&path, r)),
        "target {} is outside locally approved roots; enroll with --allow-root",
        path.display()
    );
    Ok(path)
}
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    read_optional_limit(path, syncer_extension_sdk::MAX_BYTES)
}
pub fn read_optional_limit(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    reject_symlinks(path)?;
    match File::open(path) {
        Ok(mut f) => {
            let meta = f.metadata()?;
            ensure!(
                meta.is_file(),
                "target is not a regular file: {}",
                path.display()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(
                    meta.nlink() == 1,
                    "hard-linked files are not supported: {}",
                    path.display()
                );
            }
            ensure!(
                meta.len() <= limit as u64,
                "file too large: {}",
                path.display()
            );
            let mut bytes = vec![];
            (&mut f).take((limit + 1) as u64).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= limit, "file too large");
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}
pub fn private_dir(path: &Path) -> Result<()> {
    reject_symlinks(path)?;
    if !path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(path)?;
    }
    Ok(())
}
pub fn atomic_write(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> Result<()> {
    reject_symlinks(path)?;
    let parent = path.parent().context("path has no parent")?;
    private_dir(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    if let Some(permissions) = permissions {
        tmp.as_file().set_permissions(permissions)?;
    }
    tmp.as_file().sync_all()?;
    reject_symlinks(path)?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub struct Lock(File);
impl Lock {
    pub fn acquire(state: &Path) -> Result<Self> {
        private_dir(state)?;
        let path = state.join("apply.lock");
        reject_symlinks(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive()
            .context("another syncer process is active for this state directory")?;
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub fn verify_unchanged(path: &Path, before: &Option<Vec<u8>>) -> Result<()> {
    if &read_optional(path)? != before {
        bail!(
            "file changed after planning: {}; run apply again",
            path.display()
        );
    }
    Ok(())
}

/// Case-insensitive Windows path comparison also normalizes verbatim disk prefixes.
pub fn is_within(path: &Path, root: &Path) -> bool {
    if path
        .ancestors()
        .any(|p| same_file::is_same_file(p, root).unwrap_or(false))
    {
        return true;
    }
    #[cfg(windows)]
    {
        let path = windows_key(path);
        let root = windows_key(root);
        path == root || path.starts_with(&(root.trim_end_matches('\\').to_string() + "\\"))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}
pub fn same_path(path: &Path, root: &Path) -> bool {
    if same_file::is_same_file(path, root).unwrap_or(false) {
        return true;
    }
    #[cfg(windows)]
    {
        windows_key(path) == windows_key(root)
    }
    #[cfg(not(windows))]
    {
        path == root
    }
}
#[cfg(windows)]
fn windows_key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('/', "\\")
        .to_lowercase()
}
