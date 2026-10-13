use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use selenite_core::vault::Vault;
use selenite_plugin_host::{
    CatalogClient, PluginConfig, SignedDocument,
    pro::{ProConfig, verify_entitlement},
};
use serde_json::json;

pub enum Command {
    Recover(PathBuf),
    SyncPlan(PathBuf),
    SyncRun {
        root: PathBuf,
        plan: PathBuf,
    },
    Plugins(PathBuf),
    Catalog(PathBuf),
    Install {
        root: PathBuf,
        id: String,
    },
    Run {
        root: PathBuf,
        id: String,
        input: String,
    },
    ProStatus(PathBuf),
}

pub fn parse(args: &[OsString]) -> Result<Option<Command>> {
    let text = |value: &OsString| {
        value
            .to_str()
            .map(str::to_owned)
            .context("service argument must be Unicode")
    };
    Ok(match args {
        [command, root] if command == "recover-renames" => Some(Command::Recover(root.into())),
        [command, root] if command == "sync-plan" => Some(Command::SyncPlan(root.into())),
        [command, root, plan] if command == "sync-run" => Some(Command::SyncRun {
            root: root.into(),
            plan: plan.into(),
        }),
        [command, root] if command == "plugins" => Some(Command::Plugins(root.into())),
        [command, root] if command == "catalog" => Some(Command::Catalog(root.into())),
        [command, root, id] if command == "install-plugin" => Some(Command::Install {
            root: root.into(),
            id: text(id)?,
        }),
        [command, root, id, input] if command == "run-plugin" => Some(Command::Run {
            root: root.into(),
            id: text(id)?,
            input: text(input)?,
        }),
        [command, root] if command == "pro-status" => Some(Command::ProStatus(root.into())),
        _ => None,
    })
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
fn config(vault: &Vault) -> Result<PluginConfig> {
    Ok(vault
        .read_metadata_bounded("plugins.toml", 1024 * 1024)?
        .map(|source| toml::from_str(&source))
        .transpose()?
        .unwrap_or_default())
}

pub fn execute(command: Command, output: &mut impl Write) -> Result<()> {
    let value = match command {
        Command::Recover(root) => {
            let vault = Vault::open(root)?;
            let reports = vault.recover_pending_renames()?;
            json!({"recovered":reports.len(),"renames":reports.iter().map(|report|json!({"from":report.from,"to":report.to,"journal":report.journal})).collect::<Vec<_>>()})
        }
        Command::SyncPlan(root) => {
            let vault = Vault::open(root)?;
            let config = selenite_sync::SyncConfig::load(&vault)?;
            let provider = selenite_sync::configured_provider(&config)?;
            serde_json::to_value(selenite_sync::plan(
                &vault,
                provider.as_ref(),
                config.include_metadata,
            )?)?
        }
        Command::SyncRun { root, plan } => {
            let mut source = Vec::new();
            File::open(plan)?
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut source)?;
            ensure!(
                source.len() <= 64 * 1024 * 1024,
                "reviewed plan exceeds 64 MiB"
            );
            let plan: selenite_sync::SyncPlan = serde_json::from_slice(&source)?;
            let vault = Vault::open(root)?;
            let config = selenite_sync::SyncConfig::load(&vault)?;
            ensure!(
                config.include_metadata == plan.include_metadata,
                "sync scope changed; create a fresh plan"
            );
            let provider = selenite_sync::configured_provider(&config)?;
            serde_json::to_value(selenite_sync::run(&vault, provider.as_ref(), &plan)?)?
        }
        Command::Plugins(root) => {
            serde_json::to_value(selenite_plugin_host::list_installed(&Vault::open(root)?)?)?
        }
        Command::Catalog(root) => {
            let vault = Vault::open(root)?;
            serde_json::to_value(
                CatalogClient::new(&config(&vault)?)?
                    .fetch_catalog(now()?)?
                    .catalog(),
            )?
        }
        Command::Install { root, id } => {
            let vault = Vault::open(root)?;
            let client = CatalogClient::new(&config(&vault)?)?;
            let catalog = client.fetch_catalog(now()?)?;
            serde_json::to_value(client.install(&vault, &catalog, &id)?)?
        }
        Command::Run { root, id, input } => {
            ensure!(input.len() <= 2 * 1024 * 1024, "plugin input exceeds 2 MiB");
            let vault = Vault::open(root)?;
            serde_json::to_value(selenite_plugin_host::run_installed(
                &vault,
                &config(&vault)?,
                &id,
                serde_json::from_str(&input)?,
            )?)?
        }
        Command::ProStatus(root) => {
            let vault = Vault::open(root)?;
            if let Some(source) = vault.read_metadata_bounded("entitlement.json", 64 * 1024)? {
                let document: SignedDocument = serde_json::from_str(&source)?;
                let config: ProConfig = vault
                    .read_metadata_bounded("pro.toml", 64 * 1024)?
                    .map(|source| toml::from_str(&source))
                    .transpose()?
                    .unwrap_or_default();
                let entitlement = verify_entitlement(&config, &document, now()?)?;
                json!({"active":true,"entitlement":entitlement})
            } else {
                json!({"active":false,"edition":"community","capabilities":[]})
            }
        }
    };
    super::json_output(output, &value)
}
