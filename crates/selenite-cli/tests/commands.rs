use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository.join(".local").join("tests").join(format!(
        "cli vault café {}-{}-{}",
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

fn cli(arguments: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_selenite"))
        .args(arguments)
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn command_sequence_handles_spaces_unicode_backlinks_and_external_edits() {
    let root = scratch();
    let target = Path::new("Projects").join("Café notes.md");
    let target_source = "---\nstatus: original\n---\n# Café\n\noriginalterm";
    let reader_source = "[[Projects/Café notes#^anchor|read this]]";
    write(&root, &target, target_source);
    write(&root, "Reader.md", reader_source);
    let rebuilt = success(cli(&["rebuild".as_ref(), root.as_os_str()]));
    assert!(rebuilt.starts_with("rebuilt\tpages=2\tlinks=1\tproperties=1\telapsed_ms="));
    let found = success(cli(&[
        "search".as_ref(),
        root.as_os_str(),
        "originalterm".as_ref(),
    ]));
    assert!(found.contains("\tProjects/Café notes.md\tCafé notes\n"));
    let backlinks = success(cli(&[
        "backlinks".as_ref(),
        root.as_os_str(),
        "Café notes".as_ref(),
    ]));
    assert_eq!(backlinks, "Reader.md\t#^anchor\n");
    assert_eq!(
        success(cli(&[
            "backlinks".as_ref(),
            root.as_os_str(),
            "Projects/Café notes.md".as_ref()
        ])),
        backlinks
    );
    assert_eq!(
        fs::read_to_string(root.join(&target)).unwrap(),
        target_source
    );
    assert_eq!(
        fs::read_to_string(root.join("Reader.md")).unwrap(),
        reader_source
    );
    write(&root, &target, "changedterm");
    assert!(
        success(cli(&[
            "search".as_ref(),
            root.as_os_str(),
            "originalterm".as_ref()
        ]))
        .is_empty()
    );
    assert!(
        success(cli(&[
            "search".as_ref(),
            root.as_os_str(),
            "changedterm".as_ref()
        ]))
        .contains("Projects/Café notes.md")
    );
    assert_eq!(
        fs::read_to_string(root.join(&target)).unwrap(),
        "changedterm"
    );
}

#[test]
fn search_creates_cache_and_rebuild_recovers_corruption() {
    let root = scratch();
    write(&root, "Note.md", "autocacheterm");
    let found = success(cli(&[
        "search".as_ref(),
        root.as_os_str(),
        "autocacheterm".as_ref(),
    ]));
    assert!(found.contains("\tNote.md\tNote\n"));
    assert!(
        root.join(".selenite")
            .join("cache")
            .join("index.sqlite")
            .is_file()
    );
    // The child process has exited, so no database handle remains open.
    fs::write(
        root.join(".selenite").join("cache").join("index.sqlite"),
        "corrupt disposable cache",
    )
    .unwrap();
    assert!(success(cli(&["rebuild".as_ref(), root.as_os_str()])).contains("pages=1"));
    assert_eq!(
        success(cli(&[
            "search".as_ref(),
            root.as_os_str(),
            "autocacheterm".as_ref()
        ])),
        found
    );
    assert_eq!(
        fs::read_to_string(root.join("Note.md")).unwrap(),
        "autocacheterm"
    );
}

#[test]
fn explicit_root_path_disambiguates_backlinks_without_guessing_bare_names() {
    let root = scratch();
    write(&root, "Topic.md", "root");
    write(&root, Path::new("Other").join("Topic.md"), "nested");
    write(&root, "Reader.md", "[[./Topic]] [[Other/Topic]]");
    let ambiguous = cli(&["backlinks".as_ref(), root.as_os_str(), "Topic".as_ref()]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("matches 2 pages"));
    assert_eq!(
        success(cli(&[
            "backlinks".as_ref(),
            root.as_os_str(),
            "Topic.md".as_ref()
        ])),
        "Reader.md\n"
    );
    assert_eq!(
        success(cli(&[
            "backlinks".as_ref(),
            root.as_os_str(),
            "Other/Topic".as_ref()
        ])),
        "Reader.md\n"
    );
    for invalid in ["Missing", "../Topic.md", ""] {
        let output = cli(&["backlinks".as_ref(), root.as_os_str(), invalid.as_ref()]);
        assert!(!output.status.success(), "{invalid}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_usage_bad_query_and_missing_vault_have_correct_exit_codes() {
    assert!(success(cli(&["--help".as_ref()])).contains("selenite rebuild <vault>"));
    assert!(success(cli(&["-h".as_ref()])).contains("selenite backlinks <vault> <page>"));
    for arguments in [
        vec![],
        vec!["unknown"],
        vec!["rebuild"],
        vec!["search", "vault"],
        vec!["rebuild", "vault", "extra"],
    ] {
        let arguments: Vec<_> = arguments.iter().map(OsStr::new).collect();
        let output = cli(&arguments);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
    let root = scratch();
    write(&root, "Note.md", "a word");
    let bad_query = cli(&["search".as_ref(), root.as_os_str(), "\"unclosed".as_ref()]);
    assert!(!bad_query.status.success());
    assert!(bad_query.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bad_query.stderr).contains("invalid search query"));
    let missing = cli(&["rebuild".as_ref(), root.join("missing").as_os_str()]);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
}
