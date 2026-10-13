#[allow(dead_code)]
mod common;

use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use selenite_index::{Index, VaultWatcher};

#[test]
fn flush_reconciles_bursts_new_notes_and_retained_removals() {
    let root = common::scratch("watcher-flush");
    common::write(&root, "Existing.md", "initialword");
    let watcher = VaultWatcher::start(&root, Duration::from_secs(30)).unwrap();
    let reader = Index::open(&root).unwrap();
    for number in 0..20 {
        common::write(&root, "Existing.md", &format!("revision{number}"));
    }
    common::write(&root, "New.md", "finalword");
    let stats = watcher.flush().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (2, 0, 0));
    assert!(reader.search("initialword", 10).unwrap().is_empty());
    assert_eq!(reader.search("revision19", 10).unwrap().len(), 1);
    assert_eq!(reader.search("finalword", 10).unwrap().len(), 1);
    let backup = common::retain_as_backup(&root, "New.md");
    let stats = watcher.flush().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (0, 1, 1));
    assert!(reader.search("finalword", 10).unwrap().is_empty());
    assert_eq!(fs::read_to_string(backup).unwrap(), "finalword");
    assert!(watcher.last_error().is_none());
}

#[test]
fn flush_reconciles_directory_rename_without_touching_markdown() {
    let root = common::scratch("watcher-directory");
    common::write(&root, Path::new("Before").join("Note.md"), "portableword");
    common::write(&root, "Reader.md", "[[Before/Note]]");
    let watcher = VaultWatcher::start(&root, Duration::from_secs(30)).unwrap();
    let source = root.join("Before");
    let destination = root.join("After");
    assert!(source.canonicalize().unwrap().starts_with(&root));
    assert!(
        destination
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(&root)
    );
    assert!(!destination.exists());
    fs::rename(source, &destination).unwrap();
    let stats = watcher.flush().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (1, 1, 1));
    let reader = Index::open(&root).unwrap();
    assert_eq!(
        reader.search("portableword", 10).unwrap()[0].path,
        Path::new("After").join("Note.md")
    );
    assert!(
        reader
            .backlinks(&Path::new("Before").join("Note.md"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(root.join("Reader.md")).unwrap(),
        "[[Before/Note]]"
    );
}

#[test]
fn malformed_yaml_is_visible_and_later_edits_retry_successfully() {
    let root = common::scratch("watcher-error");
    common::write(&root, "Note.md", "beforeword");
    let watcher = VaultWatcher::start(&root, Duration::from_secs(30)).unwrap();
    common::write(&root, "Note.md", "---\nstatus: [broken\n---\nafterword");
    assert!(watcher.flush().is_err());
    assert!(watcher.last_error().unwrap().contains("Note.md"));
    let reader = Index::open(&root).unwrap();
    assert_eq!(reader.search("beforeword", 10).unwrap().len(), 1);
    common::write(&root, "Note.md", "afterword");
    assert_eq!(watcher.flush().unwrap().upserted, 1);
    assert!(watcher.last_error().is_none());
    assert!(reader.search("beforeword", 10).unwrap().is_empty());
    assert_eq!(reader.search("afterword", 10).unwrap().len(), 1);
}

#[test]
fn native_event_updates_search_without_explicit_flush() {
    let root = common::scratch("watcher-native");
    common::write(&root, "Note.md", "beforeword");
    let watcher = VaultWatcher::start(&root, Duration::from_millis(60)).unwrap();
    let reader = Index::open(&root).unwrap();
    common::write(&root, "Note.md", "nativeword");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if reader.search("nativeword", 10).unwrap().len() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native watcher did not update: {:?}",
            watcher.last_error()
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert!(reader.search("beforeword", 10).unwrap().is_empty());
}
