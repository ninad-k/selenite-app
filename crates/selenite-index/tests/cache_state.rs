#[allow(dead_code)]
mod common;

use common::{scratch, write};
use selenite_index::Index;

#[test]
fn interrupted_cross_cache_update_cannot_be_queried() {
    let root = scratch("dirty");
    write(&root, "Note.md", "originalword");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let connection = rusqlite::Connection::open(index.cache_dir().join("index.sqlite")).unwrap();
    connection
        .execute("UPDATE cache_state SET ready = 0", [])
        .unwrap();
    assert!(!index.is_ready().unwrap());
    assert!(index.search("originalword", 10).is_err());
    assert!(index.pages().is_err());
    index.rebuild().unwrap();
    assert!(index.is_ready().unwrap());
    assert_eq!(index.search("originalword", 10).unwrap().len(), 1);
}

#[test]
fn existing_reader_observes_another_handles_committed_rebuild() {
    let root = scratch("readers");
    write(&root, "Note.md", "beforeword");
    let mut writer = Index::open(&root).unwrap();
    writer.rebuild().unwrap();
    let reader = Index::open(&root).unwrap();
    assert_eq!(reader.search("beforeword", 10).unwrap().len(), 1);
    write(&root, "Note.md", "afterword");
    writer.rebuild().unwrap();
    assert!(reader.search("beforeword", 10).unwrap().is_empty());
    assert_eq!(reader.search("afterword", 10).unwrap().len(), 1);
}

#[test]
fn broken_search_metadata_recovers_from_sources() {
    let root = scratch("bad-search");
    write(&root, "Note.md", "recoverableword");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let metadata = index.cache_dir().join("search").join("meta.json");
    drop(index);
    std::fs::write(metadata, "not json").unwrap();
    let (index, _) = Index::rebuild_from_disk(&root).unwrap();
    assert_eq!(index.search("recoverableword", 10).unwrap().len(), 1);
}
