#[allow(dead_code)]
mod common;

use selenite_core::vault::Vault;
use selenite_index::Index;
use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

#[test]
fn reconciliation_waits_for_cooperating_multifile_writer() {
    let root = common::scratch("rename-snapshot");
    let vault = Vault::open(&root).unwrap();
    vault.create_page("Old.md", "a snapshotword").unwrap();
    vault.create_page("Reader.md", "[[Old]]").unwrap();
    let (mut index, _) = Index::rebuild_from_disk(&root).unwrap();
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".selenite/write.lock"))
        .unwrap();
    writer.lock().unwrap();
    fs::rename(root.join("Old.md"), root.join("New.md")).unwrap();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = index
            .refresh()
            .and_then(|_| index.backlinks(Path::new("New.md")));
        result_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
    fs::write(root.join("Reader.md"), "[[New]]").unwrap();
    drop(writer);
    let links = result_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source, Path::new("Reader.md"));
    worker.join().unwrap();
}
