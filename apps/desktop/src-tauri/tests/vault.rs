use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use selenite_desktop::vault::VaultSession;

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository.join(".local").join("tests").join(format!(
        "desktop-vault-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write(root: &Path, relative: impl AsRef<Path>, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert!(
        path.parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(root)
    );
    fs::write(path, source).unwrap();
}

#[test]
fn preview_resolves_wikilinks_and_block_embeds_without_rendering_code_or_html() {
    let root = scratch();
    write(
        &root,
        "Target.md",
        "# Target\n\nA quoted fact. ^fact\n\n<script>bad()</script>\n",
    );
    write(
        &root,
        "Source.md",
        "[[Target|<img src=x>]]\n\n![[Target#^fact]]\n\n`[[Target]]`\n",
    );
    let (session, _) = VaultSession::open(&root, "2026-10-13").unwrap();
    let preview = session.read_page("Source.md").unwrap();
    assert!(
        preview.html.contains("#page=Target.md&amp;fragment=")
            || preview.html.contains("#page=Target.md&fragment=")
    );
    assert!(preview.html.contains("&lt;img src=x&gt;"));
    assert!(preview.html.contains("embedded-note"));
    assert!(preview.html.contains("A quoted fact."));
    assert!(preview.html.contains("<code>[[Target]]</code>"));
    assert!(!preview.html.contains("<script>"));
}

#[test]
fn creates_default_today_and_never_overwrites_existing_note() {
    let root = scratch();
    let (session, opened) = VaultSession::open(&root, "2026-10-13").unwrap();
    assert!(opened.created);
    assert_eq!(opened.today, "Journal/2026-10-13.md");
    assert_eq!(opened.page.title, "2026-10-13");
    assert!(opened.page.source.contains("date: \"2026-10-13\""));
    assert!(opened.page.html.contains("<h1>2026-10-13</h1>"));
    let path = root.join("Journal").join("2026-10-13.md");
    fs::write(&path, "# Personal note\r\n\r\nPreserve exactly.\r\n").unwrap();
    write(
        &root,
        Path::new(".selenite").join("templates").join("daily.md"),
        "---\nbroken: [\n---",
    );
    let reopened = session.today("2026-10-13").unwrap();
    assert!(!reopened.created);
    assert_eq!(
        reopened.page.source,
        "# Personal note\r\n\r\nPreserve exactly.\r\n"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), reopened.page.source);
}

#[test]
fn uses_custom_template_with_date_and_title_substitutions() {
    let root = scratch();
    write(
        &root,
        Path::new(".selenite").join("templates").join("daily.md"),
        "---\nkind: diary\ndate: '{{date}}'\n---\n# {{title}}\n\nCustom entry.",
    );
    let (_, opened) = VaultSession::open(&root, "2024-02-29").unwrap();
    assert_eq!(
        opened.page.source,
        "---\nkind: diary\ndate: '2024-02-29'\n---\n# 2024-02-29\n\nCustom entry."
    );
    assert_eq!(opened.files, ["Journal/2024-02-29.md"]);
}

#[test]
fn rejects_invalid_dates_and_invalid_templates_without_creating_note() {
    let root = scratch();
    for invalid in [
        "2025-02-29",
        "2026-13-01",
        "2026-1-03",
        "../escape",
        "2026-10-13-extra",
    ] {
        assert!(VaultSession::open(&root, invalid).is_err(), "{invalid}");
    }
    assert!(!root.join("Journal").exists());
    write(
        &root,
        Path::new(".selenite").join("templates").join("daily.md"),
        "---\nkind: [unfinished\n---\ncontent",
    );
    let error = VaultSession::open(&root, "2026-10-13")
        .err()
        .expect("bad YAML must reject creation");
    assert!(format!("{error:#}").contains("template"));
    assert!(!root.join("Journal").join("2026-10-13.md").exists());
}

#[test]
fn preview_escapes_raw_html_omits_images_and_rejects_dangerous_links() {
    let root = scratch();
    let (session, _) = VaultSession::open(&root, "2026-10-13").unwrap();
    write(
        &root,
        "Unsafe.md",
        "<script>alert('x')</script>\n\n<img src=\"https://remote.test/pixel\">\n\n![alt text](https://remote.test/image.png)\n\n[unsafe](javascript:alert) [data](data:text/html,test) [encoded](jav&#x61;script:alert) [safe](https://example.com)\n",
    );
    let page = session.read_page("Unsafe.md").unwrap();
    assert!(!page.html.contains("<script"));
    assert!(!page.html.contains("<img"));
    assert!(page.html.contains("&lt;script&gt;"));
    assert!(page.html.contains("alt text"));
    assert!(!page.html.contains("href=\"javascript:"));
    assert!(!page.html.contains("href=\"data:"));
    assert!(page.html.contains("href=\"https://example.com\""));
    assert!(!page.html.contains("image.png"));
    for invalid in [
        "../outside.md",
        "/outside.md",
        "C:\\outside.md",
        ".selenite/templates/daily.md",
        "asset.txt",
    ] {
        assert!(session.read_page(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn lists_nested_markdown_pages_without_cache_entries() {
    let root = scratch();
    for number in 0..50 {
        write(
            &root,
            Path::new("Nested").join(format!("Note{number:02}.md")),
            "# Existing",
        );
    }
    write(
        &root,
        Path::new(".selenite").join("ignored.md"),
        "Cache metadata",
    );
    let (_, opened) = VaultSession::open(&root, "2026-10-13").unwrap();
    assert_eq!(opened.files.len(), 51);
    assert!(opened.files.contains(&"Nested/Note49.md".to_owned()));
    assert!(
        opened
            .files
            .iter()
            .all(|path| !path.contains('\\') && !path.starts_with(".selenite"))
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlink_journal_and_custom_template() {
    use std::os::unix::fs::symlink;
    let root = scratch();
    let vault = root.join("vault");
    let outside = root.join("outside");
    fs::create_dir_all(&vault).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, vault.join("Journal")).unwrap();
    assert!(VaultSession::open(&vault, "2026-10-13").is_err());
    assert!(!outside.join("2026-10-13.md").exists());
    let second = root.join("second");
    fs::create_dir_all(second.join(".selenite").join("templates")).unwrap();
    fs::write(outside.join("daily.md"), "# External template").unwrap();
    symlink(
        outside.join("daily.md"),
        second.join(".selenite").join("templates").join("daily.md"),
    )
    .unwrap();
    assert!(VaultSession::open(&second, "2026-10-13").is_err());
    assert!(!second.join("Journal").exists());
}

#[test]
fn failed_embed_does_not_block_containing_note() {
    let root = scratch();
    let (session, _) = VaultSession::open(&root, "2026-10-13").unwrap();
    write(&root, "Broken.md", "---\ninvalid: [\n---\n");
    write(&root, "Large.md", &"x".repeat(1024 * 1024 + 1));
    let source = "# Keep editing\n\n![[Broken]]\n\n![[Large]]\n";
    write(&root, "Reader.md", source);
    let page = session.read_page("Reader.md").unwrap();
    assert_eq!(page.source, source);
    assert!(page.html.contains("#page=Broken.md"));
    assert!(page.html.contains("#page=Large.md"));
    assert!(session.preview("Reader.md", source).is_ok());
}

#[test]
fn renamed_today_snapshot_does_not_recreate_original_or_read_template() {
    let root = scratch();
    let date = "2026-10-13";
    let (session, opened) = VaultSession::open(&root, date).unwrap();
    let from = Path::new(&opened.today);
    let to = Path::new("Archive.md");
    let page = selenite_core::Page::parse(from, &opened.page.source).unwrap();
    session.vault().apply_rename(from, to, &[page]).unwrap();
    write(
        &root,
        ".selenite/templates/daily.md",
        "---\nbroken: [\n---\n",
    );
    let snapshot = session.snapshot("Archive.md", date).unwrap();
    assert_eq!(snapshot.page.path, "Archive.md");
    assert!(!snapshot.created);
    assert!(!root.join(from).exists());
    assert_eq!(snapshot.files, ["Archive.md"]);
}
