use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Keep every generated artifact under the repository and retain it for review.
pub fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = repository.join(".local").join("tests").join(format!(
        "index-{label}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

pub fn write(root: &Path, relative: impl AsRef<Path>, source: &str) {
    let relative = relative.as_ref();
    assert!(
        relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    );
    let destination = root.join(relative);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    assert!(
        destination
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(root)
    );
    fs::write(destination, source).unwrap();
}

/// Simulate removal from the Markdown set without deleting the original file.
pub fn retain_as_backup(root: &Path, relative: impl AsRef<Path>) -> PathBuf {
    let source = root.join(relative);
    let destination = source.with_extension("bak");
    assert!(source.canonicalize().unwrap().starts_with(root));
    assert!(
        destination
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(root)
    );
    assert!(!destination.exists());
    fs::rename(source, &destination).unwrap();
    destination
}
