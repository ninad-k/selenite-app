mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use selenite_index::Index;

const WELCOME: &str = include_str!("../../../tests/fixtures/vault/Welcome.md");
const RUST: &str = include_str!("../../../tests/fixtures/vault/Projects/Rust.md");
const UNICODE: &str = include_str!("../../../tests/fixtures/vault/Unicode.md");

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    pages: Vec<(PathBuf, String, String)>,
    properties: BTreeMap<PathBuf, BTreeMap<String, String>>,
    rust_backlinks: Vec<BacklinkSnapshot>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BacklinkSnapshot {
    source: PathBuf,
    target: String,
    resolved: Option<PathBuf>,
    heading: Option<String>,
    alias: Option<String>,
    embedded: bool,
}

fn snapshot(index: &Index) -> Snapshot {
    let mut pages: Vec<_> = index
        .pages()
        .unwrap()
        .into_iter()
        .map(|page| (page.path, page.title, page.body))
        .collect();
    pages.sort();
    let properties = pages
        .iter()
        .map(|(path, _, _)| (path.clone(), index.properties(path).unwrap()))
        .collect();
    let mut rust_backlinks: Vec<_> = index
        .backlinks(&Path::new("Projects").join("Rust.md"))
        .unwrap()
        .into_iter()
        .map(|link| BacklinkSnapshot {
            source: link.source,
            target: link.target,
            resolved: link.resolved,
            heading: link.heading,
            alias: link.alias,
            embedded: link.embedded,
        })
        .collect();
    rust_backlinks.sort();
    Snapshot {
        pages,
        properties,
        rust_backlinks,
    }
}

#[test]
fn rebuild_is_complete_deterministic_and_preserves_authoritative_sources() {
    let root = common::scratch("rebuild");
    common::write(&root, "Welcome.md", WELCOME);
    common::write(&root, Path::new("Projects").join("Rust.md"), RUST);
    common::write(&root, "Unicode.md", UNICODE);
    let mut index = Index::open(&root).unwrap();
    let first = index.rebuild().unwrap();
    assert_eq!(first.pages, 3);
    assert_eq!(first.links, 6);
    assert_eq!(first.properties, 5);
    assert!(index.cache_dir().starts_with(root.join(".selenite")));
    let before = snapshot(&index);
    let second = index.rebuild().unwrap();
    assert_eq!(
        (second.pages, second.links, second.properties),
        (first.pages, first.links, first.properties)
    );
    assert_eq!(snapshot(&index), before);
    assert_eq!(before.rust_backlinks.len(), 2);
    assert_eq!(before.rust_backlinks[0].source, Path::new("Unicode.md"));
    assert_eq!(
        before.rust_backlinks[1].heading.as_deref(),
        Some("Ownership")
    );
    assert_eq!(
        before.rust_backlinks[1].alias.as_deref(),
        Some("Rust notes")
    );
    assert_eq!(
        fs::read_to_string(root.join("Welcome.md")).unwrap(),
        WELCOME
    );
    assert_eq!(
        fs::read_to_string(root.join("Projects").join("Rust.md")).unwrap(),
        RUST
    );
    assert_eq!(
        fs::read_to_string(root.join("Unicode.md")).unwrap(),
        UNICODE
    );
    drop(index);
    let reopened = Index::open(&root).unwrap();
    assert_eq!(snapshot(&reopened), before);
}

#[test]
fn rebuild_replaces_changed_links_properties_and_body() {
    let root = common::scratch("update");
    common::write(
        &root,
        "Source.md",
        "---\nstatus: draft\nobsolete: true\n---\nBefore [[Old]]",
    );
    common::write(&root, "Old.md", "Old target");
    common::write(&root, "New.md", "New target");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    assert_eq!(index.backlinks(Path::new("Old.md")).unwrap().len(), 1);
    common::write(
        &root,
        "Source.md",
        "---\nstatus: published\n---\nAfter ![[New#^block|shown]]",
    );
    let stats = index.rebuild().unwrap();
    assert_eq!((stats.pages, stats.links, stats.properties), (3, 1, 1));
    assert!(index.backlinks(Path::new("Old.md")).unwrap().is_empty());
    let links = index.backlinks(Path::new("New.md")).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source, Path::new("Source.md"));
    assert_eq!(links[0].target, "New");
    assert_eq!(links[0].resolved.as_deref(), Some(Path::new("New.md")));
    assert_eq!(links[0].heading.as_deref(), Some("^block"));
    assert_eq!(links[0].alias.as_deref(), Some("shown"));
    assert!(links[0].embedded);
    let properties = index.properties(Path::new("Source.md")).unwrap();
    assert_eq!(properties.len(), 1);
    assert_eq!(properties["status"].trim(), "published");
    let source = index
        .pages()
        .unwrap()
        .into_iter()
        .find(|page| page.path == Path::new("Source.md"))
        .unwrap();
    assert_eq!(source.body, "After ![[New#^block|shown]]");
}

#[test]
fn rebuild_removes_rows_for_notes_no_longer_on_disk() {
    let root = common::scratch("remove");
    let nested = Path::new("Nested").join("Gone.md");
    common::write(&root, &nested, "---\nstatus: gone\n---\n[[Survivor]]");
    common::write(&root, "Survivor.md", "[[Nested/Gone]]");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    assert_eq!(index.backlinks(Path::new("Survivor.md")).unwrap().len(), 1);
    let preserved = common::retain_as_backup(&root, &nested);
    let stats = index.rebuild().unwrap();
    assert_eq!((stats.pages, stats.links, stats.properties), (1, 1, 0));
    assert_eq!(index.pages().unwrap()[0].path, Path::new("Survivor.md"));
    assert!(index.properties(&nested).unwrap().is_empty());
    assert!(
        index
            .backlinks(Path::new("Survivor.md"))
            .unwrap()
            .is_empty()
    );
    assert!(index.backlinks(&nested).unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(preserved).unwrap(),
        "---\nstatus: gone\n---\n[[Survivor]]"
    );
}

#[test]
fn malformed_yaml_reports_filename_and_preserves_previous_cache() {
    let root = common::scratch("invalid");
    common::write(&root, "Good.md", "---\nstatus: sound\n---\nCached body");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let before = snapshot(&index);
    common::write(&root, "Good.md", "Changed on disk");
    common::write(&root, "Broken.md", "---\nstatus: [unfinished\n---\nInvalid");
    let error = index.rebuild().unwrap_err();
    assert!(format!("{error:#}").contains("Broken.md"), "{error:#}");
    assert_eq!(snapshot(&index), before);
    assert_eq!(
        fs::read_to_string(root.join("Good.md")).unwrap(),
        "Changed on disk"
    );
}

#[test]
fn ignores_cache_git_and_non_markdown_files_but_indexes_nested_unicode() {
    let root = common::scratch("scope");
    let nested = Path::new("Projects").join("Café.MD");
    common::write(&root, &nested, "# Café\n\nこんにちは [[Root]]");
    common::write(&root, "Root.md", "[[Projects/Café]]");
    common::write(
        &root,
        Path::new(".selenite").join("schemas").join("ignored.md"),
        "[[Root]]",
    );
    common::write(&root, Path::new(".git").join("ignored.md"), "[[Root]]");
    common::write(&root, "asset.txt", "[[Root]]");
    let mut index = Index::open(&root).unwrap();
    let stats = index.rebuild().unwrap();
    assert_eq!((stats.pages, stats.links), (2, 2));
    let pages = index.pages().unwrap();
    let cafe = pages.iter().find(|page| page.path == nested).unwrap();
    assert_eq!(cafe.title, "Café");
    assert!(cafe.body.contains("こんにちは"));
    assert_eq!(
        index.backlinks(&nested).unwrap()[0].source,
        Path::new("Root.md")
    );
    assert_eq!(index.backlinks(Path::new("Root.md")).unwrap().len(), 1);
}

#[test]
fn explicit_rebuild_recovers_non_sqlite_cache_and_retains_original_bytes() {
    let root = common::scratch("corrupt-cache");
    let source = "---\nstatus: original\n---\n# Source of truth\n";
    let corrupt = "not a SQLite database; preserve these bytes";
    common::write(&root, "Authoritative.md", source);
    common::write(
        &root,
        Path::new(".selenite").join("cache").join("index.sqlite"),
        corrupt,
    );

    let (index, stats) = Index::rebuild_from_disk(&root).unwrap();
    assert_eq!((stats.pages, stats.links, stats.properties), (1, 0, 1));
    assert_eq!(index.pages().unwrap()[0].body, "# Source of truth\n");
    assert_eq!(
        fs::read_to_string(root.join("Authoritative.md")).unwrap(),
        source
    );
    let retained: Vec<_> = fs::read_dir(root.join(".selenite"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path != index.cache_dir())
        .collect();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        fs::read_to_string(retained[0].join("index.sqlite")).unwrap(),
        corrupt
    );
}

#[test]
fn explicit_rebuild_recovers_incompatible_link_schema_from_markdown() {
    let root = common::scratch("old-schema");
    let source = "[[Target#Heading|label]]";
    common::write(&root, "Source.md", source);
    common::write(&root, "Target.md", "Target body");
    let cache = root.join(".selenite").join("cache");
    fs::create_dir_all(&cache).unwrap();
    let incompatible = rusqlite::Connection::open(cache.join("index.sqlite")).unwrap();
    incompatible.execute_batch("CREATE TABLE links (legacy TEXT); INSERT INTO links(legacy) VALUES ('old cached value');").unwrap();
    drop(incompatible);

    let (index, stats) = Index::rebuild_from_disk(&root).unwrap();
    assert_eq!((stats.pages, stats.links), (2, 1));
    let links = index.backlinks(Path::new("Target.md")).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source, Path::new("Source.md"));
    assert_eq!(links[0].heading.as_deref(), Some("Heading"));
    assert_eq!(links[0].alias.as_deref(), Some("label"));
    assert_eq!(fs::read_to_string(root.join("Source.md")).unwrap(), source);
    let retained: Vec<_> = fs::read_dir(root.join(".selenite"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path != index.cache_dir())
        .collect();
    assert_eq!(retained.len(), 1);
    let previous = rusqlite::Connection::open_with_flags(
        retained[0].join("index.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        previous
            .query_row("SELECT legacy FROM links", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "old cached value"
    );
}

#[test]
fn malformed_source_prevents_recovery_from_touching_corrupt_cache() {
    let root = common::scratch("corrupt-and-invalid");
    let source = "---\nstatus: [unfinished\n---\nInvalid";
    let corrupt = "invalid SQLite bytes that must remain exactly here";
    common::write(&root, "Broken.md", source);
    let cache = root.join(".selenite").join("cache");
    common::write(
        &root,
        Path::new(".selenite").join("cache").join("index.sqlite"),
        corrupt,
    );

    let error = Index::rebuild_from_disk(&root)
        .err()
        .expect("malformed source must reject rebuild");
    assert!(format!("{error:#}").contains("Broken.md"), "{error:#}");
    assert_eq!(
        fs::read_to_string(cache.join("index.sqlite")).unwrap(),
        corrupt
    );
    assert_eq!(fs::read_to_string(root.join("Broken.md")).unwrap(), source);
    assert_eq!(fs::read_dir(root.join(".selenite")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn does_not_follow_symlink_pages_or_directories() {
    use std::os::unix::fs::symlink;

    let root = common::scratch("symlink");
    let vault = root.join("vault");
    fs::create_dir_all(&vault).unwrap();
    common::write(
        &root,
        Path::new("outside").join("Target.md"),
        "Outside vault",
    );
    common::write(&vault, "Inside.md", "Inside vault");
    symlink(
        root.join("outside").join("Target.md"),
        vault.join("Linked.md"),
    )
    .unwrap();
    symlink(root.join("outside"), vault.join("LinkedDirectory")).unwrap();
    let mut index = Index::open(&vault).unwrap();
    let stats = index.rebuild().unwrap();
    assert_eq!(stats.pages, 1);
    assert_eq!(index.pages().unwrap()[0].path, Path::new("Inside.md"));
}
