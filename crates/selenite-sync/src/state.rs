use crate::{
    MAX_REMOTE_VERSIONS, RemoteVersion, SyncProvider, apply_content, content_path, digest,
    logical_path, read_content, validate_source,
};
use anyhow::{Context, Result, ensure};
use selenite_core::vault::Vault;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncActionKind {
    Upload,
    Download,
    Conflict,
    Track,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncAction {
    pub path: String,
    pub kind: SyncActionKind,
    pub revisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncIssue {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPlan {
    pub id: String,
    pub provider: String,
    pub include_metadata: bool,
    pub actions: Vec<SyncAction>,
    pub issues: Vec<SyncIssue>,
    pub unchanged: usize,
    snapshot: Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    local: BTreeMap<String, String>,
    remote: Vec<RemoteVersion>,
    baseline: Option<String>,
    pending: BTreeMap<String, PendingMerge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingMerge {
    format: u32,
    path: String,
    revision: String,
    parents: Vec<String>,
    complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: Vec<String>,
    pub issues: Vec<SyncIssue>,
    pub unchanged: usize,
    pub resolved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselineEntry {
    hash: String,
    revisions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    format: u32,
    entries: BTreeMap<String, BaselineEntry>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            format: 1,
            entries: BTreeMap::new(),
        }
    }
}

fn baseline(source: Option<&str>) -> Result<Baseline> {
    let baseline: Baseline = source
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    ensure!(baseline.format == 1, "unsupported sync baseline format");
    for (path, entry) in &baseline.entries {
        content_path(path)?;
        ensure!(
            crate::valid_hash(&entry.hash)
                && entry
                    .revisions
                    .iter()
                    .all(|revision| crate::valid_hash(revision)),
            "invalid sync baseline checksum"
        );
    }
    Ok(baseline)
}

fn state_path(provider: &dyn SyncProvider) -> PathBuf {
    Path::new("sync")
        .join("state")
        .join(format!("{}.json", digest(provider.identity())))
}

fn pending_directory(provider: &dyn SyncProvider) -> PathBuf {
    Path::new("sync")
        .join("pending")
        .join(digest(provider.identity()))
}

fn pending_path(provider: &dyn SyncProvider, path: &str) -> PathBuf {
    pending_directory(provider).join(format!("{}.json", digest(path)))
}

fn pending_merges(
    vault: &Vault,
    provider: &dyn SyncProvider,
    include_metadata: bool,
) -> Result<BTreeMap<String, PendingMerge>> {
    let mut pending = BTreeMap::new();
    for path in vault.metadata_paths(pending_directory(provider))? {
        let source = vault
            .read_metadata_bounded(&path, 64 * 1024)?
            .context("pending merge disappeared")?;
        let merge: PendingMerge =
            serde_json::from_str(&source).context("invalid pending merge record")?;
        let (_, metadata) = content_path(&merge.path)?;
        ensure!(
            merge.format == 1
                && crate::valid_hash(&merge.revision)
                && merge.parents.iter().all(|parent| crate::valid_hash(parent))
                && path == pending_path(provider, &merge.path),
            "invalid pending merge identity"
        );
        if !merge.complete && (include_metadata || !metadata) {
            pending.insert(merge.path.clone(), merge);
        }
    }
    Ok(pending)
}

// The separate sync lock serializes the complete network transaction without
// holding the core write lock while transports block or other writers edit.
fn lock(vault: &Vault) -> Result<File> {
    let relative = Path::new("sync").join("run.lock");
    if vault.read_metadata(&relative)?.is_none() {
        match vault.write_metadata(&relative, None, "") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut path = vault.root().to_owned();
    for part in [".selenite", "sync", "run.lock"] {
        path.push(part);
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "symbolic link in sync lock path"
        );
    }
    ensure!(
        fs::symlink_metadata(&path)?.is_file(),
        "sync lock must be an ordinary file"
    );
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    file.lock()?;
    Ok(file)
}

/// Read local sources, immutable remote versions, and the durable baseline.
/// Planning performs no remote writes and no page or configuration writes.
pub fn plan(
    vault: &Vault,
    provider: &dyn SyncProvider,
    include_metadata: bool,
) -> Result<SyncPlan> {
    let _guard = lock(vault)?;
    build_plan(vault, provider, include_metadata)
}

fn build_plan(
    vault: &Vault,
    provider: &dyn SyncProvider,
    include_metadata: bool,
) -> Result<SyncPlan> {
    let mut local = BTreeMap::new();
    {
        let _guard = vault.lock_shared()?;
        for path in vault.markdown_paths()? {
            let path = logical_path(&path);
            let source =
                read_content(vault, &path)?.context("page disappeared during sync snapshot")?;
            validate_source(&path, &source)?;
            local.insert(path, source);
        }
        if include_metadata {
            for prefix in ["schemas", "views", "templates"] {
                for path in vault.metadata_paths(prefix)? {
                    let logical = format!(".selenite/{}", logical_path(&path));
                    if content_path(&logical).is_err() {
                        continue;
                    }
                    let source = vault
                        .read_metadata_bounded(&path, crate::MAX_SOURCE_BYTES)?
                        .context("metadata disappeared during sync snapshot")?;
                    validate_source(&logical, &source)?;
                    local.insert(logical, source);
                }
            }
        }
    }
    let mut versions = BTreeMap::new();
    for version in provider.versions()? {
        version.validate()?;
        if !include_metadata && content_path(&version.path)?.1 {
            continue;
        }
        if let Some(previous) = versions.insert(version.revision.clone(), version.clone()) {
            ensure!(previous == version, "remote revision collision");
        }
        ensure!(
            versions.len() <= MAX_REMOTE_VERSIONS,
            "remote version count exceeds limit"
        );
    }
    let raw = vault.read_metadata_bounded(state_path(provider), 64 * 1024 * 1024)?;
    let baseline = baseline(raw.as_deref())?;
    let pending = pending_merges(vault, provider, include_metadata)?;
    let mut paths: BTreeSet<String> = local.keys().cloned().collect();
    paths.extend(pending.keys().cloned());
    paths.extend(versions.values().map(|version| version.path.clone()));
    paths.extend(
        baseline
            .entries
            .keys()
            .filter(|path| include_metadata || !path.starts_with(".selenite/"))
            .cloned(),
    );
    let mut actions = Vec::new();
    let mut issues = Vec::new();
    let mut unchanged = 0;
    let mut folded = BTreeMap::<String, Vec<String>>::new();
    for path in &paths {
        folded
            .entry(path.to_lowercase())
            .or_default()
            .push(path.clone());
    }
    for path in paths {
        if folded[&path.to_lowercase()].len() > 1 {
            issues.push(SyncIssue { path, reason: "Paths differ only by case; resolve the portable filename collision before syncing.".into() });
            continue;
        }
        let all: Vec<_> = versions
            .values()
            .filter(|version| version.path == path)
            .collect();
        let parents: BTreeSet<_> = all
            .iter()
            .flat_map(|version| version.parents.iter())
            .collect();
        let heads: Vec<_> = all
            .iter()
            .filter(|version| !parents.contains(&version.revision))
            .copied()
            .collect();
        let previous = baseline.entries.get(&path);
        let missing_history = all.iter().any(|version| {
            version.parents.iter().any(|parent| {
                versions
                    .get(parent)
                    .is_none_or(|ancestor| ancestor.path != path)
            })
        }) || pending.get(&path).is_some_and(|merge| {
            merge.parents.iter().any(|parent| {
                versions
                    .get(parent)
                    .is_none_or(|ancestor| ancestor.path != path)
            })
        }) || previous.is_some_and(|entry| {
            entry
                .revisions
                .iter()
                .any(|revision| !versions.contains_key(revision))
        });
        if missing_history {
            issues.push(SyncIssue {
                path,
                reason: "Remote history is missing or moved; deletion/rename is never propagated."
                    .into(),
            });
            continue;
        }
        let source = local.get(&path);
        let revisions: Vec<_> = heads.iter().map(|head| head.revision.clone()).collect();
        if heads.len() > 1 || pending.contains_key(&path) {
            actions.push(SyncAction {
                path,
                kind: SyncActionKind::Conflict,
                revisions,
            });
            continue;
        }
        let remote = heads.first().copied();
        let kind = match (source, remote, previous) {
            (None, None, _) => {
                unchanged += 1;
                continue;
            }
            (None, Some(_), Some(_)) => {
                issues.push(SyncIssue {
                    path,
                    reason: "Local file is missing or renamed; remote bytes were retained.".into(),
                });
                continue;
            }
            (Some(_), None, Some(_)) => {
                issues.push(SyncIssue {
                    path,
                    reason: "Remote file is missing or renamed; local bytes were retained.".into(),
                });
                continue;
            }
            (None, Some(_), None) => SyncActionKind::Download,
            (Some(_), None, None) => SyncActionKind::Upload,
            (Some(local), Some(remote), _) if local == &remote.source => {
                let entry = BaselineEntry {
                    hash: digest(local),
                    revisions: revisions.clone(),
                };
                if previous == Some(&entry) {
                    unchanged += 1;
                    continue;
                }
                SyncActionKind::Track
            }
            (Some(local), Some(_), Some(previous)) if digest(local) == previous.hash => {
                SyncActionKind::Download
            }
            (Some(_), Some(remote), Some(previous)) if digest(&remote.source) == previous.hash => {
                SyncActionKind::Upload
            }
            (Some(_), Some(_), _) => SyncActionKind::Conflict,
        };
        actions.push(SyncAction {
            path,
            kind,
            revisions,
        });
    }
    let snapshot = Snapshot {
        local,
        remote: versions.into_values().collect(),
        baseline: raw,
        pending,
    };
    let provider = provider.identity();
    let id = digest(serde_json::to_vec(&(
        &provider,
        include_metadata,
        &actions,
        &issues,
        &snapshot,
    ))?);
    Ok(SyncPlan {
        id,
        provider,
        include_metadata,
        actions,
        issues,
        unchanged,
        snapshot,
    })
}

/// Recompute the complete review and require equality, including every source,
/// action and baseline. Forged or stale IPC plans cannot publish any changes.
/// After interruption, create a fresh plan; immutable publication and per-action
/// baseline checkpoints make replay convergent without deleting either side.
pub fn run(vault: &Vault, provider: &dyn SyncProvider, reviewed: &SyncPlan) -> Result<SyncReport> {
    let _guard = lock(vault)?;
    let current = build_plan(vault, provider, reviewed.include_metadata)?;
    ensure!(
        &current == reviewed,
        "sync plan is stale or modified; review a fresh plan"
    );
    let journal = Path::new("sync")
        .join("journals")
        .join(format!("{}.json", current.id));
    let saved = serde_json::to_string_pretty(&current)?;
    if let Some(existing) = vault.read_metadata(&journal)? {
        ensure!(existing == saved, "sync journal collision");
    } else {
        vault.write_metadata(&journal, None, &saved)?;
    }
    let mut baseline_raw = current.snapshot.baseline.clone();
    let mut state = baseline(baseline_raw.as_deref())?;
    let mut report = SyncReport {
        issues: current.issues.clone(),
        unchanged: current.unchanged,
        ..SyncReport::default()
    };
    for action in &current.actions {
        let expected = current.snapshot.local.get(&action.path).map(String::as_str);
        ensure!(
            read_content(vault, &action.path)?.as_deref() == expected,
            "local source changed during sync: {}",
            action.path
        );
        let heads: Vec<_> = action
            .revisions
            .iter()
            .map(|revision| {
                current
                    .snapshot
                    .remote
                    .iter()
                    .find(|version| &version.revision == revision)
                    .expect("validated plan references a known revision")
            })
            .collect();
        let (hash, revisions) = match action.kind {
            SyncActionKind::Upload => {
                let source = expected.context("upload has no local source")?;
                let version = RemoteVersion::new(&action.path, action.revisions.clone(), source)?;
                provider.publish(&version).with_context(|| {
                    format!(
                        "upload failed for {}; completed earlier actions are checkpointed",
                        action.path
                    )
                })?;
                report.uploaded += 1;
                (digest(source), vec![version.revision])
            }
            SyncActionKind::Download => {
                let head = heads.first().context("download has no remote head")?;
                apply_content(vault, &action.path, expected, &head.source)?;
                report.downloaded += 1;
                (digest(&head.source), action.revisions.clone())
            }
            SyncActionKind::Track => (
                digest(expected.context("tracked source missing")?),
                action.revisions.clone(),
            ),
            SyncActionKind::Conflict => {
                if let Some(source) = expected {
                    report.conflicts.push(conflict_copy(
                        vault,
                        &RemoteVersion::new(&action.path, Vec::new(), source)?,
                    )?);
                }
                for head in heads {
                    if expected == Some(head.source.as_str()) {
                        continue;
                    }
                    let path = conflict_copy(vault, head)?;
                    report.conflicts.push(path);
                }
                continue;
            }
        };
        state
            .entries
            .insert(action.path.clone(), BaselineEntry { hash, revisions });
        let next = serde_json::to_string_pretty(&state)?;
        vault.write_metadata(state_path(provider), baseline_raw.as_deref(), &next)?;
        baseline_raw = Some(next);
    }
    let completed = Path::new("sync")
        .join("journals")
        .join(format!("{}.complete.json", current.id));
    let existing = vault.read_metadata(&completed)?;
    vault.write_metadata(
        completed,
        existing.as_deref(),
        &serde_json::to_string_pretty(&report)?,
    )?;
    Ok(report)
}

/// Explicitly merge a reviewed conflict. All reviewed remote heads become
/// parents of a new immutable version; future concurrent heads remain visible.
/// Pending intent prevents ordinary sync from overwriting the locally retained
/// merge after transport failure. Retry by obtaining a fresh reviewed plan.
pub fn resolve_conflict(
    vault: &Vault,
    provider: &dyn SyncProvider,
    reviewed: &SyncPlan,
    path: &str,
    merged_source: &str,
) -> Result<SyncReport> {
    validate_source(path, merged_source)?;
    let _guard = lock(vault)?;
    let current = build_plan(vault, provider, reviewed.include_metadata)?;
    ensure!(
        &current == reviewed,
        "sync plan is stale or modified; review a fresh plan"
    );
    let action = current
        .actions
        .iter()
        .find(|action| action.path == path && action.kind == SyncActionKind::Conflict)
        .context("selected path is not a reviewed conflict")?;
    let expected = current.snapshot.local.get(path).map(String::as_str);
    ensure!(
        read_content(vault, path)?.as_deref() == expected,
        "local source changed during conflict resolution: {path}"
    );
    let heads: Vec<_> = action
        .revisions
        .iter()
        .map(|revision| {
            current
                .snapshot
                .remote
                .iter()
                .find(|version| &version.revision == revision)
                .expect("validated conflict revision")
        })
        .collect();
    // A lost upload acknowledgement or interrupted checkpoint can already have
    // published this exact pending merge. Finish it without another upload.
    let published = current.snapshot.pending.get(path).and_then(|pending| {
        heads.first().copied().filter(|head| {
            heads.len() == 1 && head.revision == pending.revision && head.source == merged_source
        })
    });
    let version = if let Some(published) = published {
        published.clone()
    } else {
        RemoteVersion::new(path, action.revisions.clone(), merged_source)?
    };
    let journal_id = digest(serde_json::to_vec(&(&current.id, path, &version))?);
    let journal = Path::new("sync")
        .join("resolutions")
        .join(format!("{journal_id}.json"));
    let intent =
        serde_json::to_string_pretty(&serde_json::json!({"review":current,"merge":version}))?;
    if let Some(existing) = vault.read_metadata(&journal)? {
        ensure!(existing == intent, "merge journal collision");
    } else {
        vault.write_metadata(&journal, None, &intent)?;
    }
    let mut report = SyncReport {
        issues: current.issues.clone(),
        ..SyncReport::default()
    };
    if let Some(source) = expected {
        report.conflicts.push(conflict_copy(
            vault,
            &RemoteVersion::new(path, Vec::new(), source)?,
        )?);
    }
    for head in &heads {
        if expected != Some(head.source.as_str()) {
            report.conflicts.push(conflict_copy(vault, head)?);
        }
    }
    let pending_file = pending_path(provider, path);
    let previous = vault.read_metadata_bounded(&pending_file, 64 * 1024)?;
    let mut pending = PendingMerge {
        format: 1,
        path: path.to_owned(),
        revision: version.revision.clone(),
        parents: version.parents.clone(),
        complete: false,
    };
    let pending_source = serde_json::to_string_pretty(&pending)?;
    vault.write_metadata(&pending_file, previous.as_deref(), &pending_source)?;
    if expected != Some(merged_source) {
        apply_content(vault, path, expected, merged_source)?;
    }
    ensure!(
        read_content(vault, path)?.as_deref() == Some(merged_source),
        "local merged source changed before publication: {path}"
    );
    if published.is_none() {
        provider.publish(&version).with_context(|| format!("merge upload failed; merged source remains at {path}; review a fresh plan and resolve again"))?;
        report.uploaded = 1;
    }
    let mut baseline = baseline(current.snapshot.baseline.as_deref())?;
    baseline.entries.insert(
        path.to_owned(),
        BaselineEntry {
            hash: digest(merged_source),
            revisions: vec![version.revision.clone()],
        },
    );
    vault.write_metadata(
        state_path(provider),
        current.snapshot.baseline.as_deref(),
        &serde_json::to_string_pretty(&baseline)?,
    )?;
    pending.complete = true;
    vault.write_metadata(
        &pending_file,
        Some(&pending_source),
        &serde_json::to_string_pretty(&pending)?,
    )?;
    report.resolved.push(path.to_owned());
    let completed = Path::new("sync")
        .join("resolutions")
        .join(format!("{journal_id}.complete.json"));
    let previous = vault.read_metadata(&completed)?;
    vault.write_metadata(
        completed,
        previous.as_deref(),
        &serde_json::to_string_pretty(&report)?,
    )?;
    Ok(report)
}

fn conflict_copy(vault: &Vault, remote: &RemoteVersion) -> Result<String> {
    let path = format!(
        "Conflicts/sync-{}-{}.md",
        &digest(&remote.path)[..16],
        remote.revision
    );
    let source = if content_path(&remote.path)?.1 {
        let longest = remote
            .source
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or_default();
        let fence = "`".repeat(longest.max(2) + 1);
        format!(
            "# Sync conflict: {}\n\nRemote revision: {}\n\n{fence}\n{}\n{fence}\n",
            remote.path, remote.revision, remote.source
        )
    } else {
        remote.source.clone()
    };
    if let Some(existing) = read_content(vault, &path)? {
        ensure!(
            existing == source,
            "conflict copy was externally changed: {path}"
        );
    } else {
        apply_content(vault, &path, None, &source)?;
    }
    Ok(path)
}
