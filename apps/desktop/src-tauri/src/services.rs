//! Explicit operations for optional providers; no service runs during startup.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use selenite_core::vault::Vault;
use selenite_plugin_host::{
    Catalog, CatalogClient, InstalledPlugin, PluginConfig, PluginRun, SignedDocument,
    pro::{ProConfig, verify_entitlement},
};
use serde_json::{Value, json};
use tauri::State;

use crate::{CommandResult, SharedState, commands::session};

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

#[tauri::command]
pub(crate) async fn read_sync_source(
    path: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let source = if let Some(metadata) = path.strip_prefix(".selenite/") {
            ensure!(
                (metadata.starts_with("schemas/") || metadata.starts_with("views/"))
                    && metadata.ends_with(".toml")
                    || metadata.starts_with("templates/") && metadata.ends_with(".md"),
                "Source is outside the sync metadata scope"
            );
            session
                .vault()
                .read_metadata_bounded(metadata, selenite_sync::MAX_SOURCE_BYTES)?
        } else {
            selenite_core::validate_page_path(std::path::Path::new(&path))?;
            match session
                .vault()
                .read_source_bounded(&path, selenite_sync::MAX_SOURCE_BYTES)
            {
                Ok(source) => Some(source),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        };
        Ok(json!({"source":source}))
    })
    .await
}

#[tauri::command]
pub(crate) async fn sync_resolve(
    plan: selenite_sync::SyncPlan,
    path: String,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_sync::SyncReport> {
    session(state, move |session| {
        let config = selenite_sync::SyncConfig::load(session.vault())?;
        ensure!(
            config.include_metadata == plan.include_metadata,
            "Sync scope changed; review a fresh plan"
        );
        let provider = selenite_sync::configured_provider(&config)?;
        selenite_sync::resolve_conflict(session.vault(), provider.as_ref(), &plan, &path, &source)
    })
    .await
}

#[tauri::command]
pub(crate) async fn sync_plan(
    state: State<'_, SharedState>,
) -> CommandResult<selenite_sync::SyncPlan> {
    session(state, |session| {
        let config = selenite_sync::SyncConfig::load(session.vault())?;
        let provider = selenite_sync::configured_provider(&config)?;
        selenite_sync::plan(session.vault(), provider.as_ref(), config.include_metadata)
    })
    .await
}

#[tauri::command]
pub(crate) async fn sync_run(
    plan: selenite_sync::SyncPlan,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_sync::SyncReport> {
    session(state, move |session| {
        let config = selenite_sync::SyncConfig::load(session.vault())?;
        ensure!(
            config.include_metadata == plan.include_metadata,
            "Sync scope changed; review a fresh plan"
        );
        let provider = selenite_sync::configured_provider(&config)?;
        selenite_sync::run(session.vault(), provider.as_ref(), &plan)
    })
    .await
}

fn plugin_config(vault: &Vault) -> Result<PluginConfig> {
    parse_plugin_config(
        vault
            .read_metadata_bounded("plugins.toml", 1024 * 1024)?
            .as_deref(),
    )
}

#[tauri::command]
pub(crate) async fn list_installed_plugins(
    state: State<'_, SharedState>,
) -> CommandResult<Vec<InstalledPlugin>> {
    session(state, |session| {
        selenite_plugin_host::list_installed(session.vault())
    })
    .await
}

#[tauri::command]
pub(crate) async fn plugin_catalog(state: State<'_, SharedState>) -> CommandResult<Catalog> {
    let shared = Arc::clone(state.inner());
    session(state, move |session| {
        let config = plugin_config(session.vault())?;
        let catalog = CatalogClient::new(&config)?.fetch_catalog(now()?)?;
        let result = catalog.catalog().clone();
        *shared
            .catalog
            .lock()
            .map_err(|_| anyhow::anyhow!("Catalog lock failed"))? =
            Some((session.root().to_owned(), catalog));
        Ok(result)
    })
    .await
}

#[tauri::command]
pub(crate) async fn install_plugin(
    id: String,
    state: State<'_, SharedState>,
) -> CommandResult<InstalledPlugin> {
    let shared = Arc::clone(state.inner());
    session(state, move |session| {
        let config = plugin_config(session.vault())?;
        let guard = shared
            .catalog
            .lock()
            .map_err(|_| anyhow::anyhow!("Catalog lock failed"))?;
        let (root, catalog) = guard
            .as_ref()
            .context("Fetch and review the signed catalog first")?;
        ensure!(
            root == session.root(),
            "Catalog belongs to another vault; fetch it again"
        );
        CatalogClient::new(&config)?.install(session.vault(), catalog, &id)
    })
    .await
}

#[tauri::command]
pub(crate) async fn prepare_plugin_run(
    id: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let source = session
            .vault()
            .read_metadata_bounded("plugins.toml", 1024 * 1024)?;
        let config = parse_plugin_config(source.as_deref())?;
        ensure!(config.enabled, "Plugins are disabled");
        let (plugin, _, _) =
            selenite_plugin_host::load_installed(session.vault(), &id, &config.limits)?;
        let granted = config.grants.get(&id).cloned().unwrap_or_default();
        let effective: Vec<_> = plugin
            .capabilities
            .intersection(&granted)
            .copied()
            .collect();
        Ok(json!({"plugin":plugin,"configSource":source,"effectiveGrants":effective}))
    })
    .await
}

fn parse_plugin_config(source: Option<&str>) -> Result<PluginConfig> {
    let config = source.map(toml::from_str).transpose()?.unwrap_or_default();
    PluginConfig::validate(&config)?;
    Ok(config)
}

fn reviewed_plugin_config(vault: &Vault, expected: Option<&str>) -> Result<PluginConfig> {
    let source = vault.read_metadata_bounded("plugins.toml", 1024 * 1024)?;
    ensure!(
        source.as_deref() == expected,
        "Plugin configuration changed; review its grants again"
    );
    parse_plugin_config(source.as_deref())
}

#[tauri::command]
pub(crate) async fn run_plugin(
    id: String,
    input: Value,
    expected_plugin: InstalledPlugin,
    expected_config: Option<String>,
    state: State<'_, SharedState>,
) -> CommandResult<PluginRun> {
    session(state, move |session| {
        ensure!(
            expected_plugin.id == id,
            "Reviewed plugin ID does not match"
        );
        let config = reviewed_plugin_config(session.vault(), expected_config.as_deref())?;
        selenite_plugin_host::run_installed_reviewed(
            session.vault(),
            &config,
            &expected_plugin,
            input,
        )
    })
    .await
}

fn pro_config(vault: &Vault) -> Result<ProConfig> {
    let config = vault
        .read_metadata_bounded("pro.toml", 64 * 1024)?
        .map(|source| toml::from_str(&source))
        .transpose()?
        .unwrap_or_default();
    ProConfig::validate(&config)?;
    Ok(config)
}

fn entitlement_status(vault: &Vault) -> Result<Value> {
    let Some(source) = vault.read_metadata_bounded("entitlement.json", 64 * 1024)? else {
        return Ok(
            json!({"active":false,"capabilities":[],"subject":null,"expiresAt":null,"message":"Community edition. No optional entitlement installed."}),
        );
    };
    let verified = (|| {
        let document: SignedDocument = serde_json::from_str(&source)?;
        verify_entitlement(&pro_config(vault)?, &document, now()?)
    })();
    Ok(match verified {
        Ok(entitlement) => {
            json!({"active":true,"capabilities":entitlement.capabilities,"subject":entitlement.subject,"expiresAt":entitlement.expires_at,"message":"Signed entitlement is valid. Community features remain available."})
        }
        Err(error) => {
            json!({"active":false,"capabilities":[],"subject":null,"expiresAt":null,"message":format!("{error:#}")})
        }
    })
}

#[tauri::command]
pub(crate) async fn pro_status(state: State<'_, SharedState>) -> CommandResult<Value> {
    session(state, |session| entitlement_status(session.vault())).await
}

#[tauri::command]
pub(crate) async fn install_entitlement(
    document: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        ensure!(document.len() <= 64 * 1024, "Entitlement exceeds 64 KiB");
        let signed: SignedDocument = serde_json::from_str(&document)?;
        verify_entitlement(&pro_config(session.vault())?, &signed, now()?)?;
        let expected = session
            .vault()
            .read_metadata_bounded("entitlement.json", 64 * 1024)?;
        session
            .vault()
            .write_metadata("entitlement.json", expected.as_deref(), &document)?;
        entitlement_status(session.vault())
    })
    .await
}

#[tauri::command]
pub(crate) async fn recover_renames(state: State<'_, SharedState>) -> CommandResult<Value> {
    session(state, |session| {
        let reports = session.vault().recover_pending_renames()?;
        Ok(json!({"recovered":reports.len()}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::reviewed_plugin_config;
    use selenite_core::vault::Vault;
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn plugin_confirmation_rejects_configuration_changes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap()
            .join(".local")
            .join("tests")
            .join(format!(
                "plugin-review-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let vault = Vault::open(root).unwrap();
        assert!(!reviewed_plugin_config(&vault, None).unwrap().enabled);
        let source = "enabled = true\n";
        vault.write_metadata("plugins.toml", None, source).unwrap();
        assert!(reviewed_plugin_config(&vault, None).is_err());
        assert!(
            reviewed_plugin_config(&vault, Some(source))
                .unwrap()
                .enabled
        );
        let changed = "enabled = true\n[grants]\nwriter = ['vault_write']\n";
        vault
            .write_metadata("plugins.toml", Some(source), changed)
            .unwrap();
        assert!(reviewed_plugin_config(&vault, Some(source)).is_err());
        assert!(reviewed_plugin_config(&vault, Some(changed)).is_ok());
    }
}
