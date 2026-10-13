mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use selenite_index::{Index, IndexedLink, IndexedPage};

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    pages: Vec<IndexedPage>,
    properties: BTreeMap<PathBuf, BTreeMap<String, String>>,
    backlinks: BTreeMap<PathBuf, Vec<IndexedLink>>,
    search_paths: Vec<PathBuf>,
}

fn search_paths(index: &Index, query: &str) -> Vec<PathBuf> {
    index
        .search(query, 100)
        .unwrap()
        .into_iter()
        .map(|hit| hit.path)
        .collect()
}

fn snapshot(index: &Index, query: &str) -> Snapshot {
    let pages = index.pages().unwrap();
    let properties = pages
        .iter()
        .map(|page| (page.path.clone(), index.properties(&page.path).unwrap()))
        .collect();
    let backlinks = pages
        .iter()
        .map(|page| (page.path.clone(), index.backlinks(&page.path).unwrap()))
        .collect();
    Snapshot {
        pages,
        properties,
        backlinks,
        search_paths: search_paths(index, query),
    }
}

#[test]
fn unchanged_refresh_reports_no_work_and_preserves_results() {
    let root = common::scratch("incremental-noop");
    common::write(
        &root,
        "Source.md",
        "---\r\nstatus: draft\r\n---\r\nCafé unchangedword [[Target]]\r\n",
    );
    common::write(&root, "Target.md", "unchangedword target");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let before = snapshot(&index, "unchangedword");

    for _ in 0..2 {
        let stats = index.refresh().unwrap();
        assert_eq!((stats.upserted, stats.removed, stats.unchanged), (0, 0, 2));
        assert!(index.is_ready().unwrap());
        assert_eq!(snapshot(&index, "unchangedword"), before);
    }
}

#[test]
fn changed_note_replaces_properties_links_and_search_without_duplicates() {
    let root = common::scratch("incremental-update");
    common::write(
        &root,
        "Source.md",
        "---\nstatus: draft\nobsolete: true\n---\noldtoken [[Old]]",
    );
    common::write(&root, "Old.md", "Previous destination");
    common::write(&root, "New.md", "Current destination");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let reader = Index::open(&root).unwrap();
    assert_eq!(search_paths(&reader, "oldtoken"), [Path::new("Source.md")]);

    let source = "---\nstatus: published\n---\nnewtoken ![[New#^anchor|visible]]";
    common::write(&root, "Source.md", source);
    let stats = index.refresh().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (1, 0, 2));
    assert!(index.backlinks(Path::new("Old.md")).unwrap().is_empty());
    let links = index.backlinks(Path::new("New.md")).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source, Path::new("Source.md"));
    assert_eq!(links[0].resolved.as_deref(), Some(Path::new("New.md")));
    assert_eq!(links[0].heading.as_deref(), Some("^anchor"));
    assert_eq!(links[0].alias.as_deref(), Some("visible"));
    assert!(links[0].embedded);
    let properties = index.properties(Path::new("Source.md")).unwrap();
    assert_eq!(properties.len(), 1);
    assert_eq!(properties["status"].trim(), "published");
    assert!(search_paths(&index, "oldtoken").is_empty());
    assert_eq!(search_paths(&index, "newtoken"), [Path::new("Source.md")]);
    assert!(search_paths(&reader, "oldtoken").is_empty());
    assert_eq!(search_paths(&reader, "newtoken"), [Path::new("Source.md")]);
    assert_eq!(index.pages().unwrap().len(), 3);
    assert_eq!(fs::read_to_string(root.join("Source.md")).unwrap(), source);

    index.refresh().unwrap();
    assert_eq!(search_paths(&index, "newtoken"), [Path::new("Source.md")]);
    assert_eq!(index.backlinks(Path::new("New.md")).unwrap(), links);
}

#[test]
fn adding_and_removing_duplicate_basename_updates_unchanged_incoming_links() {
    let root = common::scratch("incremental-ambiguity");
    let original = Path::new("Projects").join("Topic.md");
    let duplicate = Path::new("Archive").join("Topic.md");
    common::write(&root, &original, "Original destination");
    common::write(&root, "Source.md", "[[Topic]]");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let original_links = index.backlinks(&original).unwrap();
    assert_eq!(original_links.len(), 1);

    common::write(&root, &duplicate, "duplicatetoken");
    let stats = index.refresh().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (1, 0, 2));
    assert!(index.backlinks(&original).unwrap().is_empty());
    assert!(index.backlinks(&duplicate).unwrap().is_empty());
    assert_eq!(
        search_paths(&index, "duplicatetoken"),
        [duplicate.as_path()]
    );

    let retained = common::retain_as_backup(&root, &duplicate);
    let stats = index.refresh().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (0, 1, 2));
    assert_eq!(index.backlinks(&original).unwrap(), original_links);
    assert!(index.backlinks(&duplicate).unwrap().is_empty());
    assert!(search_paths(&index, "duplicatetoken").is_empty());
    assert_eq!(index.pages().unwrap().len(), 2);
    assert_eq!(fs::read_to_string(retained).unwrap(), "duplicatetoken");
    assert_eq!(
        fs::read_to_string(root.join("Source.md")).unwrap(),
        "[[Topic]]"
    );
}

#[test]
fn moving_nested_directory_replaces_paths_and_resolves_links_from_disk() {
    let root = common::scratch("incremental-directory-move");
    let old_directory = Path::new("Projects").join("Old");
    let new_directory = Path::new("Projects").join("New");
    let old_source = old_directory.join("Source.md");
    let old_target = old_directory.join("Target.md");
    let new_source = new_directory.join("Source.md");
    let new_target = new_directory.join("Target.md");
    common::write(&root, &old_source, "movingtoken [[./Target]]");
    common::write(
        &root,
        &old_target,
        "---\nstatus: moving\n---\ndestinationtoken",
    );
    common::write(&root, "Ref.md", "[[Target]] [[Projects/Old/Target]]");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    assert_eq!(index.backlinks(&old_target).unwrap().len(), 3);

    let from = root.join(&old_directory);
    let to = root.join(&new_directory);
    assert!(from.canonicalize().unwrap().starts_with(&root));
    assert!(
        to.parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(&root)
    );
    assert!(!to.exists());
    fs::rename(from, to).unwrap();

    let stats = index.refresh().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (2, 2, 1));
    assert!(index.backlinks(&old_target).unwrap().is_empty());
    assert!(index.properties(&old_target).unwrap().is_empty());
    assert_eq!(
        index.properties(&new_target).unwrap()["status"].trim(),
        "moving"
    );
    let links = index.backlinks(&new_target).unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].source, new_source);
    assert_eq!(links[0].target, "./Target");
    assert_eq!(links[1].source, Path::new("Ref.md"));
    assert_eq!(links[1].target, "Target");
    assert!(
        links
            .iter()
            .all(|link| link.resolved.as_ref() == Some(&new_target))
    );
    assert_eq!(search_paths(&index, "movingtoken"), [new_source.as_path()]);
    assert_eq!(
        search_paths(&index, "destinationtoken"),
        [new_target.as_path()]
    );
    let pages = index.pages().unwrap();
    assert_eq!(pages.len(), 3);
    assert!(
        pages
            .iter()
            .all(|page| page.path != old_source && page.path != old_target)
    );
    assert_eq!(
        fs::read_to_string(root.join(new_source)).unwrap(),
        "movingtoken [[./Target]]"
    );
    assert_eq!(
        fs::read_to_string(root.join("Ref.md")).unwrap(),
        "[[Target]] [[Projects/Old/Target]]"
    );
}

#[test]
fn malformed_changed_source_preserves_previous_snapshot_and_correction_recovers() {
    let root = common::scratch("incremental-invalid");
    common::write(
        &root,
        "Good.md",
        "---\nstatus: original\n---\nstabletoken [[Target]]",
    );
    common::write(&root, "Target.md", "Original destination");
    common::write(&root, "Other.md", "Changed destination");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let before = snapshot(&index, "stabletoken");

    common::write(
        &root,
        "Good.md",
        "---\nstatus: corrected\n---\nreplacementtoken [[Other]]",
    );
    common::write(
        &root,
        "ZBroken.md",
        "---\nstatus: [unfinished\n---\ninvalidtoken",
    );
    let error = index.refresh().unwrap_err();
    assert!(format!("{error:#}").contains("ZBroken.md"), "{error:#}");
    assert!(index.is_ready().unwrap());
    assert_eq!(snapshot(&index, "stabletoken"), before);
    assert!(search_paths(&index, "replacementtoken").is_empty());
    assert!(search_paths(&index, "invalidtoken").is_empty());

    common::write(&root, "ZBroken.md", "correctedtoken");
    let stats = index.refresh().unwrap();
    assert_eq!((stats.upserted, stats.removed, stats.unchanged), (2, 0, 2));
    assert!(search_paths(&index, "stabletoken").is_empty());
    assert_eq!(
        search_paths(&index, "replacementtoken"),
        [Path::new("Good.md")]
    );
    assert_eq!(
        search_paths(&index, "correctedtoken"),
        [Path::new("ZBroken.md")]
    );
    assert!(index.backlinks(Path::new("Target.md")).unwrap().is_empty());
    let links = index.backlinks(Path::new("Other.md")).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source, Path::new("Good.md"));
    assert_eq!(
        index.properties(Path::new("Good.md")).unwrap()["status"].trim(),
        "corrected"
    );
    let after = snapshot(&index, "replacementtoken");
    drop(index);
    let reopened = Index::open(&root).unwrap();
    assert_eq!(snapshot(&reopened, "replacementtoken"), after);
}
