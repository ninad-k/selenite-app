use anyhow::{Result, bail};
use selenite_core::vault::Vault;
use selenite_sync::{
    RemoteVersion, SyncActionKind, SyncConfig, SyncProvider, plan, resolve_conflict, run,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Memory {
    versions: Mutex<BTreeMap<String, RemoteVersion>>,
    writes: AtomicUsize,
    fail_second: AtomicBool,
    concurrent: Mutex<Option<RemoteVersion>>,
    fail_after_publish: AtomicBool,
}

impl Memory {
    fn add(&self, version: RemoteVersion) {
        self.versions
            .lock()
            .unwrap()
            .insert(version.revision.clone(), version);
    }
    fn one(&self) -> RemoteVersion {
        self.versions
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone()
    }
}

impl SyncProvider for Memory {
    fn name(&self) -> &str {
        "Memory"
    }
    fn identity(&self) -> String {
        "memory-test".into()
    }
    fn versions(&self) -> Result<Vec<RemoteVersion>> {
        Ok(self.versions.lock().unwrap().values().cloned().collect())
    }
    fn publish(&self, version: &RemoteVersion) -> Result<()> {
        let number = self.writes.fetch_add(1, Ordering::SeqCst);
        if number == 1 && self.fail_second.load(Ordering::SeqCst) {
            bail!("simulated interrupted sync");
        }
        if let Some(concurrent) = self.concurrent.lock().unwrap().take() {
            self.add(concurrent);
        }
        self.add(version.clone());
        if self.fail_after_publish.load(Ordering::SeqCst) {
            bail!("simulated lost upload acknowledgement");
        }
        Ok(())
    }
}

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = repository.join(".local").join("tests").join(format!(
        "sync-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn vault() -> Vault {
    Vault::open(scratch()).unwrap()
}

#[test]
fn dry_run_then_upload_download_and_three_way_changes() {
    let first = vault();
    let second = vault();
    let remote = Memory::default();
    first.create_page("Note.md", "α original\r\n").unwrap();
    let review = plan(&first, &remote, false).unwrap();
    assert_eq!(review.actions[0].kind, SyncActionKind::Upload);
    assert_eq!(remote.writes.load(Ordering::SeqCst), 0);
    assert_eq!(run(&first, &remote, &review).unwrap().uploaded, 1);
    assert_eq!(
        run(&second, &remote, &plan(&second, &remote, false).unwrap())
            .unwrap()
            .downloaded,
        1
    );
    assert_eq!(second.read_source("Note.md").unwrap(), "α original\r\n");
    first
        .write_page("Note.md", "α original\r\n", "first changed")
        .unwrap();
    run(&first, &remote, &plan(&first, &remote, false).unwrap()).unwrap();
    let review = plan(&second, &remote, false).unwrap();
    assert_eq!(review.actions[0].kind, SyncActionKind::Download);
    run(&second, &remote, &review).unwrap();
    assert_eq!(second.read_source("Note.md").unwrap(), "first changed");
    assert!(plan(&second, &remote, false).unwrap().actions.is_empty());
    assert_eq!(remote.versions.lock().unwrap().len(), 2);
}

#[test]
fn simultaneous_edits_preserve_both_sides_as_markdown() {
    let local = vault();
    let remote = Memory::default();
    local.create_page("Note.md", "base").unwrap();
    run(&local, &remote, &plan(&local, &remote, false).unwrap()).unwrap();
    let base = remote.one();
    local
        .write_page("Note.md", "base", "local change\r\n")
        .unwrap();
    remote.add(RemoteVersion::new("Note.md", vec![base.revision], "remote change\n").unwrap());
    let review = plan(&local, &remote, false).unwrap();
    assert_eq!(review.actions[0].kind, SyncActionKind::Conflict);
    let report = run(&local, &remote, &review).unwrap();
    assert_eq!(report.conflicts.len(), 2);
    let copies: Vec<_> = report
        .conflicts
        .iter()
        .map(|path| local.read_source(path).unwrap())
        .collect();
    assert!(copies.contains(&"local change\r\n".to_owned()));
    assert!(copies.contains(&"remote change\n".to_owned()));
    assert_eq!(local.read_source("Note.md").unwrap(), "local change\r\n");
}

#[test]
fn concurrent_remote_create_produces_heads_without_overwrite() {
    let local = vault();
    let remote = Memory::default();
    local.create_page("Note.md", "local").unwrap();
    *remote.concurrent.lock().unwrap() =
        Some(RemoteVersion::new("Note.md", vec![], "racing remote").unwrap());
    run(&local, &remote, &plan(&local, &remote, false).unwrap()).unwrap();
    assert_eq!(remote.versions.lock().unwrap().len(), 2);
    let review = plan(&local, &remote, false).unwrap();
    assert_eq!(review.actions[0].kind, SyncActionKind::Conflict);
    let report = run(&local, &remote, &review).unwrap();
    assert_eq!(report.conflicts.len(), 2);
    assert_eq!(local.read_source("Note.md").unwrap(), "local");
}

#[test]
fn manipulated_and_stale_plans_fail_before_publishing() {
    let local = vault();
    let remote = Memory::default();
    local.create_page("Note.md", "original").unwrap();
    let review = plan(&local, &remote, false).unwrap();
    let mut forged = serde_json::to_value(&review).unwrap();
    forged["actions"][0]["path"] = "../outside.md".into();
    let forged = serde_json::from_value(forged).unwrap();
    assert!(run(&local, &remote, &forged).is_err());
    local.write_page("Note.md", "original", "changed").unwrap();
    assert!(run(&local, &remote, &review).is_err());
    assert_eq!(remote.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn partial_sync_replans_from_durable_checkpoints() {
    let local = vault();
    let remote = Memory::default();
    local.create_page("A.md", "a").unwrap();
    local.create_page("B.md", "b").unwrap();
    remote.fail_second.store(true, Ordering::SeqCst);
    let review = plan(&local, &remote, false).unwrap();
    assert!(run(&local, &remote, &review).is_err());
    assert_eq!(remote.versions.lock().unwrap().len(), 1);
    remote.fail_second.store(false, Ordering::SeqCst);
    let reopened = Vault::open(local.root()).unwrap();
    let next = plan(&reopened, &remote, false).unwrap();
    assert_eq!(next.actions.len(), 1);
    assert_eq!(next.actions[0].path, "B.md");
    run(&reopened, &remote, &next).unwrap();
    assert_eq!(remote.versions.lock().unwrap().len(), 2);
    assert!(local.metadata_paths("sync/journals").unwrap().len() >= 3);
}

#[test]
fn missing_remote_and_local_rename_are_reported_never_deleted() {
    let local = vault();
    let remote = Memory::default();
    local.create_page("Note.md", "preserved").unwrap();
    run(&local, &remote, &plan(&local, &remote, false).unwrap()).unwrap();
    let saved = remote.one();
    remote.versions.lock().unwrap().clear();
    let review = plan(&local, &remote, false).unwrap();
    assert_eq!(review.issues.len(), 1);
    assert!(review.actions.is_empty());
    assert_eq!(local.read_source("Note.md").unwrap(), "preserved");
    remote.add(saved);
    let original = local.root().join("Note.md");
    let retained = local.root().join("Note.bak");
    assert!(original.starts_with(local.root()) && retained.starts_with(local.root()));
    fs::rename(original, retained).unwrap();
    let review = plan(&local, &remote, false).unwrap();
    assert_eq!(review.issues.len(), 1);
    assert!(review.actions.is_empty());
    assert_eq!(remote.versions.lock().unwrap().len(), 1);
}

#[test]
fn metadata_scope_is_opt_in_and_never_syncs_credentials_or_cache() {
    let local = vault();
    let remote = Memory::default();
    local
        .write_metadata("schemas/tasks.toml", None, "version = 1")
        .unwrap();
    local
        .write_metadata("views/board.toml", None, "layout = 'board'")
        .unwrap();
    local
        .write_metadata("templates/daily.md", None, "# {{date}}\n")
        .unwrap();
    local
        .write_metadata("sync.toml", None, "enabled = false")
        .unwrap();
    local
        .write_metadata("cache/secret.md", None, "must not sync")
        .unwrap();
    assert!(plan(&local, &remote, false).unwrap().actions.is_empty());
    let review = plan(&local, &remote, true).unwrap();
    assert_eq!(review.actions.len(), 3);
    run(&local, &remote, &review).unwrap();
    assert!(
        remote
            .versions
            .lock()
            .unwrap()
            .values()
            .all(|version| !version.source.contains("must not sync")
                && !version.path.ends_with("sync.toml"))
    );
}

#[test]
fn invalid_remote_paths_and_sources_are_rejected() {
    for path in [
        "../outside.md",
        "A//B.md",
        "/Note.md",
        "A\\B.md",
        ".selenite/sync.toml",
        ".selenite/history/a.md",
        ".selenite/cache/a.md",
        ".selenite/schemas/../sync.toml",
        "CON.md",
    ] {
        assert!(RemoteVersion::new(path, vec![], "safe").is_err(), "{path}");
    }
    assert!(RemoteVersion::new("Note.md", vec![], "---\nx: [\n---\n").is_err());
    let mut invalid = RemoteVersion::new("Note.md", vec![], "safe").unwrap();
    invalid.path = "../outside.md".into();
    let remote = Memory::default();
    remote.add(invalid);
    assert!(plan(&vault(), &remote, false).is_err());
    assert!(!SyncConfig::default().enabled);
    assert!(SyncConfig::default().validate().is_ok());
}

fn conflict_fixture() -> (Vault, Memory, RemoteVersion) {
    let local = vault();
    let remote = Memory::default();
    local.create_page("Note.md", "base").unwrap();
    run(&local, &remote, &plan(&local, &remote, false).unwrap()).unwrap();
    let base = remote.one();
    local.write_page("Note.md", "base", "local change").unwrap();
    remote
        .add(RemoteVersion::new("Note.md", vec![base.revision.clone()], "remote change").unwrap());
    (local, remote, base)
}

#[test]
fn reviewed_merge_joins_all_heads_and_converges_on_another_device() {
    let first = vault();
    let second = vault();
    let remote = Memory::default();
    first.create_page("Note.md", "base").unwrap();
    run(&first, &remote, &plan(&first, &remote, false).unwrap()).unwrap();
    run(&second, &remote, &plan(&second, &remote, false).unwrap()).unwrap();
    let base = remote.one();
    let left = RemoteVersion::new("Note.md", vec![base.revision.clone()], "left").unwrap();
    let right = RemoteVersion::new("Note.md", vec![base.revision], "right").unwrap();
    remote.add(left.clone());
    remote.add(right.clone());
    let review = plan(&first, &remote, false).unwrap();
    let report = resolve_conflict(
        &first,
        &remote,
        &review,
        "Note.md",
        "# Combined\r\nleft and right\r\n",
    )
    .unwrap();
    assert_eq!(report.resolved, vec!["Note.md"]);
    assert_eq!(report.uploaded, 1);
    assert_eq!(report.conflicts.len(), 3);
    let versions = remote.versions().unwrap();
    let merged = versions
        .iter()
        .find(|version| version.source.starts_with("# Combined"))
        .unwrap();
    let mut parents = vec![left.revision, right.revision];
    parents.sort();
    assert_eq!(merged.parents, parents);
    assert_eq!(first.read_source("Note.md").unwrap(), merged.source);
    let next = plan(&second, &remote, false).unwrap();
    assert_eq!(next.actions[0].kind, SyncActionKind::Download);
    run(&second, &remote, &next).unwrap();
    assert_eq!(second.read_source("Note.md").unwrap(), merged.source);
    assert!(plan(&second, &remote, false).unwrap().actions.is_empty());
    assert!(
        !plan(&first, &remote, false)
            .unwrap()
            .actions
            .iter()
            .any(|action| action.path == "Note.md")
    );
    for path in report.conflicts {
        assert!(first.root().join(path).is_file());
    }
}

#[test]
fn merge_rejects_invalid_sources_nonconflicts_and_stale_reviews() {
    let (local, remote, base) = conflict_fixture();
    let review = plan(&local, &remote, false).unwrap();
    let count = remote.writes.load(Ordering::SeqCst);
    assert!(resolve_conflict(&local, &remote, &review, "Note.md", "---\nx: [\n---\n").is_err());
    assert!(resolve_conflict(&local, &remote, &review, "../outside.md", "bad").is_err());
    assert!(resolve_conflict(&local, &remote, &review, "Other.md", "not conflicted").is_err());
    assert_eq!(local.read_source("Note.md").unwrap(), "local change");
    local
        .write_page("Note.md", "local change", "external local edit")
        .unwrap();
    assert!(resolve_conflict(&local, &remote, &review, "Note.md", "merged").is_err());
    let next = plan(&local, &remote, false).unwrap();
    remote.add(RemoteVersion::new("Note.md", vec![base.revision], "new remote head").unwrap());
    assert!(resolve_conflict(&local, &remote, &next, "Note.md", "merged").is_err());
    assert_eq!(local.read_source("Note.md").unwrap(), "external local edit");
    assert_eq!(remote.writes.load(Ordering::SeqCst), count);
    assert!(local.metadata_paths("sync/resolutions").unwrap().is_empty());
}

#[test]
fn failed_merge_preserves_local_choice_and_fresh_review_can_retry() {
    let (local, remote, _) = conflict_fixture();
    remote.fail_second.store(true, Ordering::SeqCst);
    let review = plan(&local, &remote, false).unwrap();
    // Choosing the original baseline is still an explicit choice. Without the
    // pending record, ordinary three-way sync would download the remote edit.
    let error = resolve_conflict(&local, &remote, &review, "Note.md", "base").unwrap_err();
    assert!(error.to_string().contains("merged source remains"));
    assert_eq!(local.read_source("Note.md").unwrap(), "base");
    let reopened = Vault::open(local.root()).unwrap();
    let next = plan(&reopened, &remote, false).unwrap();
    assert_eq!(
        next.actions
            .iter()
            .find(|action| action.path == "Note.md")
            .unwrap()
            .kind,
        SyncActionKind::Conflict
    );
    assert!(resolve_conflict(&reopened, &remote, &review, "Note.md", "base").is_err());
    remote.fail_second.store(false, Ordering::SeqCst);
    assert_eq!(
        resolve_conflict(&reopened, &remote, &next, "Note.md", "base")
            .unwrap()
            .uploaded,
        1
    );
    assert!(
        !plan(&reopened, &remote, false)
            .unwrap()
            .actions
            .iter()
            .any(|action| action.path == "Note.md")
    );
    assert!(reopened.metadata_paths("sync/resolutions").unwrap().len() >= 3);
}

#[test]
fn lost_merge_acknowledgement_retries_without_another_upload() {
    let (local, remote, _) = conflict_fixture();
    remote.fail_after_publish.store(true, Ordering::SeqCst);
    let review = plan(&local, &remote, false).unwrap();
    assert!(resolve_conflict(&local, &remote, &review, "Note.md", "merged").is_err());
    let writes = remote.writes.load(Ordering::SeqCst);
    remote.fail_after_publish.store(false, Ordering::SeqCst);
    let next = plan(&local, &remote, false).unwrap();
    let report = resolve_conflict(&local, &remote, &next, "Note.md", "merged").unwrap();
    assert_eq!(report.uploaded, 0);
    assert_eq!(report.resolved, vec!["Note.md"]);
    assert_eq!(remote.writes.load(Ordering::SeqCst), writes);
    assert!(
        !plan(&local, &remote, false)
            .unwrap()
            .actions
            .iter()
            .any(|action| action.path == "Note.md")
    );
}

#[test]
fn new_head_during_merge_publication_remains_an_independent_conflict() {
    let (local, remote, base) = conflict_fixture();
    let review = plan(&local, &remote, false).unwrap();
    *remote.concurrent.lock().unwrap() =
        Some(RemoteVersion::new("Note.md", vec![base.revision], "late remote change").unwrap());
    resolve_conflict(
        &local,
        &remote,
        &review,
        "Note.md",
        "merged reviewed changes",
    )
    .unwrap();
    let next = plan(&local, &remote, false).unwrap();
    let action = next
        .actions
        .iter()
        .find(|action| action.path == "Note.md")
        .unwrap();
    assert_eq!(action.kind, SyncActionKind::Conflict);
    assert_eq!(action.revisions.len(), 2);
    assert!(
        remote
            .versions()
            .unwrap()
            .iter()
            .any(|version| version.source == "late remote change")
    );
    assert_eq!(
        local.read_source("Note.md").unwrap(),
        "merged reviewed changes"
    );
}

#[test]
fn metadata_merges_validate_toml_and_preserve_both_originals() {
    let local = vault();
    let remote = Memory::default();
    let path = ".selenite/schemas/tasks.toml";
    local
        .write_metadata("schemas/tasks.toml", None, "version = 1\n")
        .unwrap();
    remote.add(RemoteVersion::new(path, vec![], "version = 2\n").unwrap());
    let review = plan(&local, &remote, true).unwrap();
    assert!(resolve_conflict(&local, &remote, &review, path, "version = [").is_err());
    assert_eq!(
        local.read_metadata("schemas/tasks.toml").unwrap().unwrap(),
        "version = 1\n"
    );
    let report = resolve_conflict(&local, &remote, &review, path, "version = 3\n").unwrap();
    assert_eq!(report.conflicts.len(), 2);
    assert_eq!(
        local.read_metadata("schemas/tasks.toml").unwrap().unwrap(),
        "version = 3\n"
    );
    let copies: Vec<_> = report
        .conflicts
        .iter()
        .map(|path| local.read_source(path).unwrap())
        .collect();
    assert!(copies.iter().any(|copy| copy.contains("version = 1")));
    assert!(copies.iter().any(|copy| copy.contains("version = 2")));
}
