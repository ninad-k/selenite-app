//! Filesystem boundary for a vault. Markdown files remain authoritative.

use std::{
    fs, io,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "vault must be a directory",
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Sorted vault-relative paths. Never follows symbolic links or cache folders.
    pub fn markdown_paths(&self) -> io::Result<Vec<PathBuf>> {
        let mut pending = vec![PathBuf::new()];
        let mut pages = Vec::new();
        while let Some(relative) = pending.pop() {
            for entry in fs::read_dir(self.root.join(&relative))? {
                let entry = entry?;
                let kind = entry.file_type()?;
                let name = entry.file_name();
                if name == ".selenite" || name == ".git" || kind.is_symlink() {
                    continue;
                }
                let path = relative.join(name);
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() && is_markdown(&path) {
                    pages.push(path);
                }
            }
        }
        pages.sort();
        Ok(pages)
    }

    pub fn read_source(&self, path: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(self.resolve_existing(path.as_ref())?)
    }

    /// Exclusively create a Markdown page and any missing ordinary directories.
    /// Returns false for an existing regular page, without changing its bytes.
    pub fn create_page(&self, path: impl AsRef<Path>, source: &str) -> io::Result<bool> {
        let path = path.as_ref();
        crate::validate_page_path(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        let mut directory = self.root.clone();
        for component in path.parent().unwrap_or(Path::new("")).components() {
            directory.push(component);
            match fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let metadata = fs::symlink_metadata(&directory)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || !directory.canonicalize()?.starts_with(&self.root)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "page ancestor is not a regular vault directory",
                ));
            }
        }
        let destination = self.root.join(path);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(mut file) => {
                file.write_all(source.as_bytes())?;
                file.sync_all()?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(destination)?;
                if metadata.is_file() && !metadata.file_type().is_symlink() {
                    Ok(false)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "page destination is not a regular file",
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Resolve a relative path without following links, including linked ancestors.
    pub fn resolve_existing(&self, path: &Path) -> io::Result<PathBuf> {
        validate_relative(path)?;
        let mut current = self.root.clone();
        for component in path.components() {
            current.push(component);
            if fs::symlink_metadata(&current)?.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "symbolic links are not vault pages",
                ));
            }
        }
        let resolved = current.canonicalize()?;
        if !resolved.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path escapes vault",
            ));
        }
        Ok(resolved)
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("md"))
}

pub fn validate_relative(path: &Path) -> io::Result<()> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "paths must be UTF-8"))?;
    if text.is_empty()
        || text.contains(':')
        || text.contains('\0')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || text
            .split(['/', '\\'])
            .any(|c| c == ".." || c == "." || c.is_empty() || c == ".selenite" || c == ".git")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a vault-relative content path",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let path = root.join(".local").join("tests").join(format!(
            "vault-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn enumerates_markdown_deterministically_without_cache() {
        let root = scratch();
        fs::create_dir_all(root.join("Journal")).unwrap();
        fs::create_dir_all(root.join(".selenite")).unwrap();
        fs::write(root.join("Journal").join("today.md"), "hello").unwrap();
        fs::write(root.join("a.MD"), "hello").unwrap();
        fs::write(root.join(".selenite").join("ignored.md"), "cache").unwrap();
        fs::write(root.join("asset.txt"), "asset").unwrap();
        let vault = Vault::open(root).unwrap();
        assert_eq!(
            vault.markdown_paths().unwrap(),
            vec![
                PathBuf::from("Journal").join("today.md"),
                PathBuf::from("a.MD")
            ]
        );
        assert_eq!(
            vault
                .read_source(Path::new("Journal").join("today.md"))
                .unwrap(),
            "hello"
        );
    }

    #[test]
    fn rejects_escape_and_metadata_paths() {
        for path in [
            "",
            "../secret.md",
            "C:\\secret.md",
            "/secret.md",
            ".selenite/index.md",
            "a/../../secret.md",
            "a\\..\\secret.md",
        ] {
            assert!(validate_relative(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn creates_nested_page_exclusively_without_overwriting() {
        let root = scratch();
        let vault = Vault::open(&root).unwrap();
        let path = Path::new("Nested").join("Page.md");
        assert!(vault.create_page(&path, "original\r\n").unwrap());
        assert!(!vault.create_page(&path, "replacement").unwrap());
        assert_eq!(vault.read_source(path).unwrap(), "original\r\n");
        for invalid in ["../outside.md", ".selenite/cache.md", "asset.txt"] {
            assert!(vault.create_page(invalid, "content").is_err());
        }
        fs::write(root.join("File.md"), "not a directory").unwrap();
        assert!(
            vault
                .create_page(Path::new("File.md").join("Child.md"), "content")
                .is_err()
        );
    }
}
