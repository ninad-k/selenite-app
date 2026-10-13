#[allow(dead_code)]
mod common;

use std::{
    fs,
    path::{Path, PathBuf},
};

use selenite_index::Index;

#[test]
fn searches_titles_and_bodies_after_rebuild() {
    let root = common::scratch("search-fields");
    common::write(&root, "Ownership.md", "A note about borrowing.");
    common::write(
        &root,
        Path::new("Projects").join("Rust.md"),
        "Ownership keeps data safe.",
    );
    common::write(
        &root,
        "Elsewhere.md",
        "---\nsecret: ownership\n---\nUnrelated body.",
    );
    let mut index = Index::open(&root).unwrap();
    assert!(!index.is_ready().unwrap());
    assert!(index.search("ownership", 10).is_err());
    index.rebuild().unwrap();
    assert!(index.is_ready().unwrap());
    let hits = index.search("ownership", 10).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter()
            .any(|hit| hit.path == Path::new("Ownership.md") && hit.title == "Ownership")
    );
    assert!(
        hits.iter()
            .any(|hit| hit.path == Path::new("Projects").join("Rust.md") && hit.title == "Rust")
    );
    assert!(
        hits.iter()
            .all(|hit| hit.score.is_finite() && hit.score > 0.0)
    );
    assert!(index.search("absentword", 10).unwrap().is_empty());
}

#[test]
fn blank_queries_limits_and_syntax_errors_are_explicit() {
    let root = common::scratch("search-input");
    common::write(&root, "Note.md", "a searchable word");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    assert!(index.search(" \n\t ", 10).unwrap().is_empty());
    assert!(index.search("word", 0).unwrap().is_empty());
    assert!(index.search("\"unclosed phrase", 10).is_err());
    assert!(index.search("unknown_field:word", 10).is_err());
}

#[test]
fn equal_score_cutoff_is_stable_across_rebuild_and_reopen() {
    let root = common::scratch("search-stability");
    for number in (0..30).rev() {
        common::write(
            &root,
            format!("Note{number:02}.md"),
            "identical searchable content",
        );
    }
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    let first = index.search("searchable", 7).unwrap();
    assert_eq!(
        first.iter().map(|hit| hit.path.clone()).collect::<Vec<_>>(),
        (0..7)
            .map(|number| PathBuf::from(format!("Note{number:02}.md")))
            .collect::<Vec<_>>()
    );
    assert!(first.windows(2).all(|pair| pair[0].score == pair[1].score));
    index.rebuild().unwrap();
    assert_eq!(index.search("searchable", 7).unwrap(), first);
    drop(index);
    let reopened = Index::open(&root).unwrap();
    assert!(reopened.is_ready().unwrap());
    assert_eq!(reopened.search("searchable", 7).unwrap(), first);
    // Opening another read-only handle must not require an exclusive writer lock.
    let second_reader = Index::open(&root).unwrap();
    assert_eq!(second_reader.search("searchable", 7).unwrap(), first);
}

#[test]
fn rebuild_replaces_changed_body_and_omits_retained_non_markdown_backup() {
    let root = common::scratch("search-refresh");
    common::write(&root, "Changed.md", "oldphrase");
    common::write(&root, "Gone.md", "oldphrase");
    let mut index = Index::open(&root).unwrap();
    index.rebuild().unwrap();
    assert_eq!(index.search("oldphrase", 10).unwrap().len(), 2);
    common::write(&root, "Changed.md", "newphrase");
    let retained = common::retain_as_backup(&root, "Gone.md");
    index.rebuild().unwrap();
    assert!(index.search("oldphrase", 10).unwrap().is_empty());
    let updated = index.search("newphrase", 10).unwrap();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].path, Path::new("Changed.md"));
    assert_eq!(fs::read_to_string(retained).unwrap(), "oldphrase");
}
