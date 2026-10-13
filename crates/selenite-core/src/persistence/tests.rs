use super::*;
use crate::LinkResolver;
use std::sync::{Arc, Barrier};

fn scratch() -> PathBuf {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository
        .join(".local")
        .join("tests")
        .join(format!("persistence-{}", revision()));
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

#[test]
fn bounded_reads_reject_oversize_without_changing_files() {
    let vault = Vault::open(scratch()).unwrap();
    assert!(vault.create_page("Unicode.md", "café").unwrap());
    assert_eq!(vault.read_source_bounded("Unicode.md", 5).unwrap(), "café");
    assert!(vault.read_source_bounded("Unicode.md", 4).is_err());
    assert!(vault.read_source_bounded("../outside.md", 100).is_err());
    assert_eq!(vault.read_metadata_bounded("absent.toml", 1).unwrap(), None);
    vault
        .write_metadata("settings.toml", None, "enabled = true")
        .unwrap();
    assert!(vault.read_metadata_bounded("settings.toml", 5).is_err());
    assert_eq!(
        vault
            .read_metadata_bounded("settings.toml", 100)
            .unwrap()
            .as_deref(),
        Some("enabled = true")
    );
    assert!(vault.read_metadata_bounded("write.lock", 100).is_err());
}

fn pages(vault: &Vault) -> Vec<Page> {
    vault
        .markdown_paths()
        .unwrap()
        .into_iter()
        .map(|path| Page::parse(&path, &vault.read_source(&path).unwrap()).unwrap())
        .collect()
}

fn sample() -> Vault {
    let vault = Vault::open(scratch()).unwrap();
    vault
        .create_page(
            "Old.md",
            "---\nstatus: draft\n---\n# café\r\n[[./Other#^x|other]]\r\n",
        )
        .unwrap();
    vault.create_page("Other.md", "unchanged\r\n").unwrap();
    vault
        .create_page(
            Path::new("Nested").join("Reader.md"),
            "α ![[Old#^café|label]]\r\n`[[Old]]`\r\n",
        )
        .unwrap();
    vault
}

#[test]
fn writes_compare_exact_bytes_and_retain_immutable_history() {
    let vault = Vault::open(scratch()).unwrap();
    let original = "---\r\nstatus: draft\r\n---\r\nα original\r\n";
    let next = "# changed\n";
    assert!(vault.create_page("Note.md", original).unwrap());
    let report = vault.write_page("Note.md", original, next).unwrap();
    assert_eq!(vault.read_source("Note.md").unwrap(), next);
    assert_eq!(
        fs::read_to_string(vault.root().join(report.history)).unwrap(),
        original
    );
    assert_eq!(report.path, Path::new("Note.md"));
    assert!(!report.revision.is_empty());
    assert_eq!(
        vault
            .write_page("Note.md", original, "lost update")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(
        vault
            .write_page("Note.md", next, "---\nx: [\n---\n")
            .is_err()
    );
    assert_eq!(vault.read_source("Note.md").unwrap(), next);
    assert!(!vault.create_page("Note.md", "---\nx: [\n---\n").unwrap());
}

#[test]
fn staging_failure_preserves_original_page() {
    let root = scratch();
    fs::write(root.join("Note.md"), "original").unwrap();
    fs::create_dir(root.join(".selenite")).unwrap();
    fs::write(root.join(".selenite").join("history"), "blocked directory").unwrap();
    let vault = Vault::open(root).unwrap();
    assert!(
        vault
            .write_page("Note.md", "original", "replacement")
            .is_err()
    );
    assert_eq!(vault.read_source("Note.md").unwrap(), "original");
    assert!(vault.create_page("New.md", "whole page").is_err());
    assert!(!vault.root().join("New.md").exists());
}

#[test]
fn concurrent_compare_and_swap_has_exactly_one_winner() {
    let vault = Vault::open(scratch()).unwrap();
    vault.create_page("Note.md", "original").unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for content in ["first", "second"] {
        let vault = vault.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            vault.write_page("Note.md", "original", content)
        }));
    }
    barrier.wait();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(matches!(
        vault.read_source("Note.md").unwrap().as_str(),
        "first" | "second"
    ));
}

#[test]
fn metadata_compare_and_swap_and_discovery_are_separate_from_pages() {
    let vault = Vault::open(scratch()).unwrap();
    let schema = Path::new("schemas").join("tasks.toml");
    assert_eq!(vault.read_metadata(&schema).unwrap(), None);
    assert!(vault.metadata_paths("schemas").unwrap().is_empty());
    vault
        .write_metadata(&schema, None, "version = 1\r\n")
        .unwrap();
    assert_eq!(
        vault.read_metadata(&schema).unwrap().as_deref(),
        Some("version = 1\r\n")
    );
    assert_eq!(
        vault
            .write_metadata(&schema, None, "version = 2")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        vault
            .write_metadata(&schema, Some("version = 1\n"), "version = 2")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    vault
        .write_metadata(&schema, Some("version = 1\r\n"), "version = 2")
        .unwrap();
    vault
        .write_metadata(
            Path::new("schemas").join("nested").join("z.toml"),
            None,
            "other",
        )
        .unwrap();
    let mut expected = vec![schema, Path::new("schemas").join("nested").join("z.toml")];
    expected.sort();
    assert_eq!(vault.metadata_paths("schemas").unwrap(), expected);
    assert!(vault.markdown_paths().unwrap().is_empty());
    assert_eq!(
        vault
            .write_metadata("missing.toml", Some("missing"), "new")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn persistence_paths_are_portable_and_internal_metadata_is_reserved() {
    let vault = Vault::open(scratch()).unwrap();
    for path in [
        "../secret.md",
        "bad?.md",
        "CON.md",
        "com1.md",
        "LPT².md",
        "folder./note.md",
        "folder /note.md",
        "AUX/note.md",
        "bad<name.md",
        "bad\nname.md",
        ".SELENITE/note.md",
        ".GIT/note.md",
    ] {
        assert!(vault.create_page(path, "content").is_err(), "{path}");
        assert!(vault.read_metadata(path).is_err(), "{path}");
    }
    for path in [
        "write.lock",
        "WRITE.LOCK",
        "history/revision.md",
        "transactions/rename.yaml",
        "../outside.toml",
        "/outside.toml",
    ] {
        assert!(vault.read_metadata(path).is_err(), "{path}");
        assert!(vault.write_metadata(path, None, "bad").is_err(), "{path}");
    }
    assert!(vault.markdown_paths().unwrap().is_empty());
}

#[test]
fn reviewed_rename_preserves_source_bytes_and_link_identities() {
    let vault = sample();
    let before = pages(&vault);
    let target = Path::new("Moved").join("New.md");
    let report = vault
        .apply_rename(Path::new("Old.md"), &target, &before)
        .unwrap();
    assert!(!vault.root().join("Old.md").exists());
    assert_eq!(
        fs::read_to_string(vault.root().join(&report.journal).join("source-archive.md")).unwrap(),
        before
            .iter()
            .find(|page| page.path == Path::new("Old.md"))
            .unwrap()
            .source
    );
    assert_eq!(vault.read_source("Other.md").unwrap(), "unchanged\r\n");
    let reader = vault
        .read_source(Path::new("Nested").join("Reader.md"))
        .unwrap();
    assert!(reader.starts_with("α ![[New#^café|label]]\r\n"));
    assert!(reader.ends_with("`[[Old]]`\r\n"));
    let after = pages(&vault);
    let resolver = LinkResolver::new(after.iter().map(|page| &page.path)).unwrap();
    let moved = after.iter().find(|page| page.path == target).unwrap();
    let referring = after
        .iter()
        .find(|page| page.path == Path::new("Nested").join("Reader.md"))
        .unwrap();
    assert_eq!(
        resolver
            .resolve(&referring.path, &referring.links[0].target)
            .unwrap(),
        target
    );
    assert_eq!(
        resolver
            .resolve(&moved.path, &moved.links[0].target)
            .unwrap(),
        Path::new("Other.md")
    );
    assert!(
        vault
            .root()
            .join(&report.journal)
            .join("committed")
            .is_file()
    );
    assert!(vault.recover_pending_renames().unwrap().is_empty());
}

#[test]
fn stale_snapshots_and_destination_collisions_never_publish() {
    let vault = sample();
    let before = pages(&vault);
    fs::write(vault.root().join("Other.md"), "external edit").unwrap();
    assert_eq!(
        vault
            .apply_rename(Path::new("Old.md"), Path::new("New.md"), &before)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(vault.root().join("Old.md").is_file());
    assert!(!vault.root().join("New.md").exists());
    let before = pages(&vault);
    assert!(
        vault
            .apply_rename(Path::new("Old.md"), Path::new("Other.md"), &before)
            .is_err()
    );
    assert_eq!(vault.read_source("Other.md").unwrap(), "external edit");
    vault.create_page("Extra.md", "extra").unwrap();
    assert!(
        vault
            .apply_rename(Path::new("Old.md"), Path::new("New.md"), &before)
            .is_err()
    );
    assert!(vault.root().join("Old.md").is_file());
}

#[test]
fn case_only_rename_preserves_content_and_updates_filename() {
    let vault = Vault::open(scratch()).unwrap();
    vault.create_page("Page.md", "exact\r\n").unwrap();
    vault
        .apply_rename(Path::new("Page.md"), Path::new("page.md"), &pages(&vault))
        .unwrap();
    assert_eq!(
        vault.markdown_paths().unwrap(),
        vec![PathBuf::from("page.md")]
    );
    assert_eq!(vault.read_source("page.md").unwrap(), "exact\r\n");
}

fn interrupt_after_archival(vault: &Vault) -> Journal {
    let _guard = exclusive(vault).unwrap();
    let journal = prepare_rename(
        vault,
        Path::new("Old.md"),
        Path::new("New.md"),
        &pages(vault),
        &[],
        None,
    )
    .unwrap();
    let source = vault.resolve_existing(Path::new("Old.md")).unwrap();
    let archive = journal.directory.join("source-archive.md");
    assert!(source.starts_with(vault.root()) && archive.starts_with(vault.root()));
    fs::rename(source, archive).unwrap();
    journal
}

#[test]
fn interrupted_rename_rolls_forward_only_when_explicitly_requested() {
    let vault = sample();
    let journal = interrupt_after_archival(&vault);
    assert!(!vault.root().join("Old.md").exists());
    assert!(!vault.root().join("New.md").exists());
    let reopened = Vault::open(vault.root()).unwrap();
    assert!(!reopened.root().join("New.md").exists());
    let reports = reopened.recover_pending_renames().unwrap();
    assert_eq!(reports.len(), 1);
    for edit in &journal.edits {
        assert_eq!(reopened.read_source(&edit.target).unwrap(), edit.after);
    }
    assert!(journal.directory.join("source-archive.md").is_file());
    assert!(reopened.recover_pending_renames().unwrap().is_empty());
}

#[test]
fn recovery_refuses_external_edits_and_retains_all_snapshots() {
    let vault = sample();
    let journal = interrupt_after_archival(&vault);
    fs::write(
        vault.root().join("Other.md"),
        "external change to an unchanged page",
    )
    .unwrap();
    assert_eq!(
        vault.recover_pending_renames().unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        vault.read_source("Other.md").unwrap(),
        "external change to an unchanged page"
    );
    assert!(!vault.root().join("New.md").exists());
    assert!(!journal.directory.join("committed").exists());
    for (number, edit) in journal.edits.iter().enumerate() {
        assert_eq!(
            fs::read_to_string(journal.directory.join(format!("{number}-before.md"))).unwrap(),
            edit.before
        );
        assert_eq!(
            fs::read_to_string(journal.directory.join(format!("{number}-after.md"))).unwrap(),
            edit.after
        );
    }
}

fn overlay_fixture() -> (Vault, Vec<Page>, Page) {
    let vault = Vault::open(scratch()).unwrap();
    vault.create_page("Old.md", "original target\r\n").unwrap();
    vault
        .create_page("Reader.md", "---\nrelation: Old\n---\n[[Old]]\r\n")
        .unwrap();
    let expected = pages(&vault);
    let plan = plan_rename(&expected, Path::new("Old.md"), Path::new("New.md")).unwrap();
    let reader = plan
        .pages
        .iter()
        .find(|page| page.path == Path::new("Reader.md"))
        .unwrap();
    let overlay = Page::parse(
        "Reader.md",
        &format!("---\nrelation: New\n---\n{}", reader.body),
    )
    .unwrap();
    (vault, expected, overlay)
}

#[test]
fn frontmatter_overlay_commits_with_body_links_in_one_rename() {
    let (vault, expected, overlay) = overlay_fixture();
    let report = vault
        .apply_rename_with_edits(
            Path::new("Old.md"),
            Path::new("New.md"),
            &expected,
            std::slice::from_ref(&overlay),
        )
        .unwrap();
    assert_eq!(vault.read_source("Reader.md").unwrap(), overlay.source);
    assert_eq!(vault.read_source("New.md").unwrap(), "original target\r\n");
    assert_eq!(report.updated.len(), 2);
    assert!(
        vault
            .root()
            .join(report.journal)
            .join("committed")
            .is_file()
    );
}

#[test]
fn rename_overlays_reject_body_changes_duplicates_and_nonexistent_paths() {
    let (vault, expected, overlay) = overlay_fixture();
    for overlays in [
        vec![Page::parse("Reader.md", "changed body").unwrap()],
        vec![overlay.clone(), overlay.clone()],
        vec![Page::parse("Old.md", "original target\r\n").unwrap()],
        vec![Page::parse("Added.md", "not an existing page").unwrap()],
    ] {
        assert!(
            vault
                .apply_rename_with_edits(
                    Path::new("Old.md"),
                    Path::new("New.md"),
                    &expected,
                    &overlays
                )
                .is_err()
        );
        for page in &expected {
            assert_eq!(vault.read_source(&page.path).unwrap(), page.source);
        }
        assert!(!vault.root().join("New.md").exists());
    }
}

#[test]
fn interrupted_rename_recovers_frontmatter_overlay_and_markdown_together() {
    let (vault, expected, overlay) = overlay_fixture();
    let journal = {
        let _guard = exclusive(&vault).unwrap();
        let journal = prepare_rename(
            &vault,
            Path::new("Old.md"),
            Path::new("New.md"),
            &expected,
            std::slice::from_ref(&overlay),
            None,
        )
        .unwrap();
        let source = vault.resolve_existing(Path::new("Old.md")).unwrap();
        let archived = journal.directory.join("source-archive.md");
        assert!(source.starts_with(vault.root()) && archived.starts_with(vault.root()));
        fs::rename(source, archived).unwrap();
        journal
    };
    let recovered = vault.recover_pending_renames().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(vault.read_source("Reader.md").unwrap(), overlay.source);
    assert_eq!(vault.read_source("New.md").unwrap(), "original target\r\n");
    assert!(journal.directory.join("source-archive.md").is_file());
}

#[test]
fn rename_rejects_invalid_metadata_snapshot_without_publishing() {
    let (vault, expected, overlay) = overlay_fixture();
    for snapshot in [
        MetadataSnapshot {
            prefix: PathBuf::from("../outside"),
            files: BTreeMap::new(),
        },
        MetadataSnapshot {
            prefix: PathBuf::from("schemas"),
            files: BTreeMap::from([(PathBuf::from("settings.toml"), String::new())]),
        },
        MetadataSnapshot {
            prefix: PathBuf::from("history"),
            files: BTreeMap::new(),
        },
    ] {
        assert!(
            vault
                .apply_rename_with_metadata_snapshot(
                    Path::new("Old.md"),
                    Path::new("New.md"),
                    &expected,
                    std::slice::from_ref(&overlay),
                    &snapshot,
                )
                .is_err()
        );
        for page in &expected {
            assert_eq!(vault.read_source(&page.path).unwrap(), page.source);
        }
        assert!(!vault.root().join("New.md").exists());
        assert!(!vault.root().join(".selenite").join("transactions").exists());
    }
}

#[test]
fn recovery_rechecks_retained_metadata_snapshot_before_publication() {
    let (vault, expected, overlay) = overlay_fixture();
    let schema = "kind = 'relation'\r\n";
    vault
        .write_metadata("schemas/links.toml", None, schema)
        .unwrap();
    let journal = {
        let _guard = exclusive(&vault).unwrap();
        let snapshot = vault.metadata_snapshot("schemas").unwrap();
        let journal = prepare_rename(
            &vault,
            Path::new("Old.md"),
            Path::new("New.md"),
            &expected,
            std::slice::from_ref(&overlay),
            Some(&snapshot),
        )
        .unwrap();
        let source = vault.resolve_existing(Path::new("Old.md")).unwrap();
        let archive = journal.directory.join("source-archive.md");
        assert!(source.starts_with(vault.root()) && archive.starts_with(vault.root()));
        fs::rename(source, archive).unwrap();
        journal
    };
    vault
        .write_metadata("schemas/links.toml", Some(schema), "kind = 'text'\n")
        .unwrap();
    assert_eq!(
        vault.recover_pending_renames().unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let reader = expected
        .iter()
        .find(|page| page.path == Path::new("Reader.md"))
        .unwrap();
    assert_eq!(vault.read_source("Reader.md").unwrap(), reader.source);
    assert!(!vault.root().join("New.md").exists());
    assert!(!journal.directory.join("committed").exists());
    vault
        .write_metadata("schemas/links.toml", Some("kind = 'text'\n"), schema)
        .unwrap();
    assert_eq!(vault.recover_pending_renames().unwrap().len(), 1);
    assert_eq!(vault.read_source("Reader.md").unwrap(), overlay.source);
    assert_eq!(vault.read_source("New.md").unwrap(), "original target\r\n");
}

#[cfg(unix)]
#[test]
fn symlinked_content_and_metadata_paths_never_escape_vault() {
    use std::os::unix::fs::symlink;
    let root = scratch();
    let outside = root.join("outside");
    let inside = root.join("vault");
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&inside).unwrap();
    fs::write(outside.join("Note.md"), "outside bytes").unwrap();
    symlink(&outside, inside.join("Linked")).unwrap();
    let vault = Vault::open(&inside).unwrap();
    assert!(vault.create_page("Linked/New.md", "bad").is_err());
    assert!(
        vault
            .write_page("Linked/Note.md", "outside bytes", "bad")
            .is_err()
    );
    vault
        .write_metadata("schemas/good.toml", None, "safe")
        .unwrap();
    symlink(
        &outside,
        inside.join(".selenite").join("schemas").join("linked"),
    )
    .unwrap();
    assert!(vault.read_metadata("schemas/linked/Note.md").is_err());
    assert!(
        vault
            .write_metadata("schemas/linked/Note.md", Some("outside bytes"), "bad")
            .is_err()
    );
    assert_eq!(
        vault.metadata_paths("schemas").unwrap(),
        vec![PathBuf::from("schemas/good.toml")]
    );
    assert_eq!(
        fs::read_to_string(outside.join("Note.md")).unwrap(),
        "outside bytes"
    );
    assert!(!outside.join("New.md").exists());
}
