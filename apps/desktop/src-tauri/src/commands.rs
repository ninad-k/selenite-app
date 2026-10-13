//! Application commands compose the file, query and optional service boundaries.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use selenite_ai::{AiConfig, HttpProvider};
use selenite_core::{LinkResolver, Page};
use selenite_db::{Cell, DatabaseStore};
use selenite_index::{Index, path_key};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::State;

use crate::{
    CommandResult, SharedState, today,
    vault::{OpenedVault, PreviewPage, VaultSession},
};

pub(super) async fn session<T: Send + 'static>(
    state: State<'_, SharedState>,
    operation: impl FnOnce(&VaultSession) -> Result<T> + Send + 'static,
) -> CommandResult<T> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let current = shared
            .session
            .lock()
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("Open a vault first")?
            .clone();
        operation(&current).map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn preview_markdown(
    path: String,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<PreviewPage> {
    session(state, move |session| session.preview(&path, &source)).await
}

#[tauri::command]
pub(crate) async fn save_page(
    path: String,
    expected_source: String,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<PreviewPage> {
    session(state, move |session| {
        let preview = session.preview(&path, &source)?;
        session
            .vault()
            .write_page(&path, &expected_source, &source)?;
        Ok(preview)
    })
    .await
}

#[tauri::command]
pub(crate) async fn create_page(
    path: String,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<PreviewPage> {
    session(state, move |session| {
        let preview = session.preview(&path, &source)?;
        ensure!(
            session.vault().create_page(&path, &source)?,
            "A page already exists at this path"
        );
        Ok(preview)
    })
    .await
}

#[tauri::command]
pub(crate) async fn refresh_vault(state: State<'_, SharedState>) -> CommandResult<OpenedVault> {
    session(state, move |session| session.today(&today())).await
}

#[tauri::command]
pub(crate) async fn search_pages(
    query: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let mut index = match Index::open(session.root()) {
            Ok(index) => index,
            Err(_) => Index::rebuild_from_disk(session.root())?.0,
        };
        index.refresh()?;
        Ok(json!(
            index
                .search(&query, 100)?
                .into_iter()
                .map(|hit| json!({"path":path_key(&hit.path),"title":hit.title,"score":hit.score}))
                .collect::<Vec<_>>()
        ))
    })
    .await
}

fn pages(session: &VaultSession) -> Result<Vec<Page>> {
    let _source_guard = session.vault().lock_shared()?;
    session
        .vault()
        .markdown_paths()?
        .into_iter()
        .map(|path| {
            let source = session.vault().read_source(&path)?;
            Ok(Page::parse(path, &source)?)
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn page_links(
    path: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state,move |session| {
        let pages = pages(session)?;
        let resolver = LinkResolver::new(pages.iter().map(|page|&page.path))?;
        let titles: BTreeMap<_,_> = pages.iter().map(|page|(&page.path,&page.title)).collect();
        let target = Path::new(&path);
        ensure!(titles.contains_key(&target.to_owned()), "Page does not exist");
        let mut backlinks = Vec::new();
        let mut outgoing = Vec::new();
        let mut edges = std::collections::BTreeSet::new();
        for page in &pages {
            for link in &page.links {
                let Ok(destination) = resolver.resolve(&page.path,&link.target) else { continue; };
                if destination == target { backlinks.push(json!({"path":path_key(&page.path),"title":page.title})); }
                if page.path == target { outgoing.push(json!({"path":path_key(&destination),"title":titles.get(&destination),"fragment":link.heading})); }
                edges.insert((path_key(&page.path),path_key(&destination)));
            }
        }
        backlinks.dedup();
        Ok(json!({"backlinks":backlinks,"outgoing":outgoing,"graph":{
            "nodes":pages.iter().map(|page|json!({"path":path_key(&page.path),"title":page.title})).collect::<Vec<_>>(),
            "edges":edges.into_iter().map(|(source,target)|json!({"source":source,"target":target})).collect::<Vec<_>>()
        }}))
    }).await
}

#[derive(Deserialize)]
pub(crate) struct ExpectedSource {
    path: String,
    source: String,
}

#[derive(Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpectedChange {
    path: String,
    source: String,
    updated_source: String,
}

fn rename_changes(pages: &[Page], plan: &selenite_core::RenamePlan) -> Result<Vec<ExpectedChange>> {
    plan.pages
        .iter()
        .map(|page| {
            let old_path = if page.path == plan.to {
                &plan.from
            } else {
                &page.path
            };
            let previous = pages
                .iter()
                .find(|previous| previous.path == *old_path)
                .context("rename plan source missing")?;
            Ok(ExpectedChange {
                path: path_key(old_path),
                source: previous.source.clone(),
                updated_source: page.source.clone(),
            })
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn rename_preview(
    from: String,
    to: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state,move |session| {
        let pages = pages(session)?;
        let plan = DatabaseStore::new(session.vault().clone()).plan_rename(&pages,Path::new(&from),Path::new(&to))?;
        let changes = rename_changes(&pages,&plan)?;
        Ok(json!({"from":from,"to":to,"changes":changes,"expectedSources":pages.iter().map(|page|json!({"path":path_key(&page.path),"source":page.source})).collect::<Vec<_>>()}))
    }).await
}

#[tauri::command]
pub(crate) async fn apply_rename(
    from: String,
    to: String,
    expected_sources: Vec<ExpectedSource>,
    expected_changes: Vec<ExpectedChange>,
    state: State<'_, SharedState>,
) -> CommandResult<OpenedVault> {
    session(state, move |session| {
        let pages = expected_sources
            .into_iter()
            .map(|item| Page::parse(item.path, &item.source))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let reviewed = DatabaseStore::new(session.vault().clone()).plan_rename_reviewed(
            &pages,
            Path::new(&from),
            Path::new(&to),
        )?;
        let plan = reviewed.plan;
        ensure!(
            rename_changes(&pages, &plan)? == expected_changes,
            "Rename rules changed; review a fresh preview"
        );
        session.vault().apply_rename_with_metadata_snapshot(
            Path::new(&from),
            Path::new(&to),
            &pages,
            &plan.pages,
            &reviewed.schema_snapshot,
        )?;
        session.snapshot(&to, &today())
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_databases(
    state: State<'_, SharedState>,
) -> CommandResult<Vec<selenite_db::DatabaseSchema>> {
    session(state, |session| {
        DatabaseStore::new(session.vault().clone()).list_schemas()
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_views(
    state: State<'_, SharedState>,
) -> CommandResult<Vec<selenite_db::SavedView>> {
    session(state, |session| {
        DatabaseStore::new(session.vault().clone()).list_views()
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_view(
    id: String,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_db::QueryResult> {
    session(state, move |session| {
        DatabaseStore::new(session.vault().clone()).query_saved(&id)
    })
    .await
}

#[tauri::command]
pub(crate) async fn install_gtd_templates(
    state: State<'_, SharedState>,
) -> CommandResult<Vec<String>> {
    session(state, |session| {
        DatabaseStore::new(session.vault().clone()).install_gtd_templates()
    })
    .await
}

#[tauri::command]
pub(crate) async fn update_database_row(
    database: String,
    path: String,
    expected_source: String,
    changes: BTreeMap<String, Cell>,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_db::Row> {
    session(state, move |session| {
        let store = DatabaseStore::new(session.vault().clone());
        let schema = store
            .load_schema(&database)?
            .context("Database does not exist")?;
        store.update_row(&schema, &path, &expected_source, &changes)
    })
    .await
}

#[tauri::command]
pub(crate) async fn create_database_row(
    database: String,
    path: String,
    body: String,
    cells: BTreeMap<String, Cell>,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_db::Row> {
    session(state, move |session| {
        let store = DatabaseStore::new(session.vault().clone());
        let schema = store
            .load_schema(&database)?
            .context("Database does not exist")?;
        store.create_row(&schema, &path, &body, &cells)
    })
    .await
}

fn config_path(service: &str) -> Result<String> {
    ensure!(
        matches!(service, "ai" | "sync" | "plugins" | "pro"),
        "Unknown service"
    );
    Ok(format!("{service}.toml"))
}

#[tauri::command]
pub(crate) async fn read_definition(
    kind: String,
    id: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let store = DatabaseStore::new(session.vault().clone());
        let source = match kind.as_str() {
            "schema" => store
                .load_schema_document(&id)?
                .map(|document| document.source),
            "view" => store
                .load_view_document(&id)?
                .map(|document| document.source),
            _ => anyhow::bail!("Unknown definition kind"),
        };
        Ok(json!({"source":source}))
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_definition(
    kind: String,
    id: String,
    expected_source: Option<String>,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let store = DatabaseStore::new(session.vault().clone());
        let (directory, normalized) = match kind.as_str() {
            "schema" => {
                let schema: selenite_db::DatabaseSchema = toml::from_str(&source)?;
                ensure!(schema.id == id, "Schema ID must match the selected ID");
                store.save_schema(&schema, expected_source.as_deref())?;
                (
                    "schemas",
                    store
                        .load_schema_document(&id)?
                        .context("saved schema missing")?
                        .source,
                )
            }
            "view" => {
                let view: selenite_db::SavedView = toml::from_str(&source)?;
                ensure!(view.id == id, "View ID must match the selected ID");
                store.save_view(&view, expected_source.as_deref())?;
                (
                    "views",
                    store
                        .load_view_document(&id)?
                        .context("saved view missing")?
                        .source,
                )
            }
            _ => anyhow::bail!("Unknown definition kind"),
        };
        Ok(json!({"path":format!(".selenite/{directory}/{id}.toml"),"source":normalized}))
    })
    .await
}

#[tauri::command]
pub(crate) async fn read_service_config(
    service: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state,move |session| {
        let path = config_path(&service)?;
        Ok(json!({"path":format!(".selenite/{path}"),"source":session.vault().read_metadata_bounded(path,64*1024)?}))
    }).await
}

#[tauri::command]
pub(crate) async fn save_service_config(
    service: String,
    expected_source: Option<String>,
    source: String,
    state: State<'_, SharedState>,
) -> CommandResult<Value> {
    session(state, move |session| {
        let path = config_path(&service)?;
        ensure!(
            source.len() <= 64 * 1024,
            "Service configuration exceeds 64 KiB"
        );
        let _: toml::Table = toml::from_str(&source).context("Invalid TOML")?;
        if service == "ai" {
            toml::from_str::<AiConfig>(&source)?.validate()?;
        }
        if service == "sync" {
            toml::from_str::<selenite_sync::SyncConfig>(&source)?.validate()?;
        }
        if service == "plugins" {
            toml::from_str::<selenite_plugin_host::PluginConfig>(&source)?.validate()?;
        }
        if service == "pro" {
            toml::from_str::<selenite_plugin_host::pro::ProConfig>(&source)?.validate()?;
        }
        session
            .vault()
            .write_metadata(&path, expected_source.as_deref(), &source)?;
        Ok(json!({"path":format!(".selenite/{path}"),"source":source}))
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_index(
    expected_config: String,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_ai::EmbeddingStats> {
    session(state, move |session| {
        let config = reviewed_ai_config(session.vault(), &expected_config)?;
        let provider = HttpProvider::new(config.clone())?;
        selenite_ai::rebuild_embeddings(session.vault(), &config, &provider)
    })
    .await
}

#[tauri::command]
pub(crate) async fn ask_vault(
    question: String,
    expected_config: String,
    state: State<'_, SharedState>,
) -> CommandResult<selenite_ai::Answer> {
    session(state, move |session| {
        let config = reviewed_ai_config(session.vault(), &expected_config)?;
        let provider = HttpProvider::new(config.clone())?;
        selenite_ai::ask(session.vault(), &config, &provider, &question)
    })
    .await
}

fn reviewed_ai_config(vault: &selenite_core::vault::Vault, expected: &str) -> Result<AiConfig> {
    let source = vault
        .read_metadata_bounded("ai.toml", 64 * 1024)?
        .context("Configure the AI provider first")?;
    ensure!(
        source == expected,
        "AI configuration changed; review the provider and scope again"
    );
    let config: AiConfig = toml::from_str(&source)?;
    config.validate()?;
    Ok(config)
}
