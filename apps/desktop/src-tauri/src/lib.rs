//! Native shell. Vault source access stays in the Rust session boundary.

pub mod vault;

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

use tauri::{State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use vault::{OpenedVault, PreviewPage, VaultSession};

struct AppState {
    session: Mutex<Option<VaultSession>>,
    expected: Mutex<Option<(String, usize, String)>>,
    startup: Option<PathBuf>,
    preload: Mutex<Option<JoinHandle<CommandResult<PreparedVault>>>>,
    started: Instant,
    report: Option<PathBuf>,
}

type SharedState = Arc<AppState>;
type CommandResult<T> = Result<T, String>;
type PreparedVault = (VaultSession, OpenedVault);

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn load_vault(state: &AppState, root: PathBuf) -> CommandResult<OpenedVault> {
    install_vault(state, prepare_vault(root)?)
}

fn prepare_vault(root: PathBuf) -> CommandResult<PreparedVault> {
    VaultSession::open(&root, &today()).map_err(|error| format!("{error:#}"))
}

fn install_vault(state: &AppState, (session, opened): PreparedVault) -> CommandResult<OpenedVault> {
    *state.expected.lock().map_err(|error| error.to_string())? = Some((
        opened.today.clone(),
        opened.files.len(),
        opened.page.title.clone(),
    ));
    *state.session.lock().map_err(|error| error.to_string())? = Some(session);
    Ok(opened)
}

#[tauri::command]
async fn startup_vault(state: State<'_, SharedState>) -> CommandResult<Option<OpenedVault>> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let preload = shared
            .preload
            .lock()
            .map_err(|error| error.to_string())?
            .take();
        if let Some(preload) = preload {
            let prepared = preload
                .join()
                .map_err(|_| "Vault startup worker failed".to_owned())??;
            return install_vault(&shared, prepared).map(Some);
        }
        shared
            .startup
            .clone()
            .map(|root| load_vault(&shared, root))
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn choose_vault(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> CommandResult<Option<OpenedVault>> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let Some(folder) = app
            .dialog()
            .file()
            .set_title("Open a Selenite vault")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let root = folder.into_path().map_err(|error| error.to_string())?;
        load_vault(&shared, root).map(Some)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn read_page(path: String, state: State<'_, SharedState>) -> CommandResult<PreviewPage> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let guard = shared.session.lock().map_err(|error| error.to_string())?;
        guard
            .as_ref()
            .ok_or("Open a vault first")?
            .read_page(&path)
            .map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn open_today(state: State<'_, SharedState>) -> CommandResult<OpenedVault> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let root = shared
            .session
            .lock()
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("Open a vault first")?
            .root()
            .to_owned();
        load_vault(&shared, root)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// An opt-in native smoke-test hook. It writes only to a path supplied at launch.
#[tauri::command]
fn frontend_ready(
    path: String,
    file_count: usize,
    title: String,
    state: State<'_, SharedState>,
) -> CommandResult<()> {
    let Some(report) = &state.report else {
        return Ok(());
    };
    let expected = state.expected.lock().map_err(|error| error.to_string())?;
    if expected.as_ref() != Some(&(path.clone(), file_count, title.clone())) {
        return Err("Rendered page does not match the opened vault".to_owned());
    }
    let result = serde_json::json!({
        "pagePath": path, "fileCount": file_count, "title": title,
        "readyElapsedMs": state.started.elapsed().as_secs_f64() * 1000.0,
        "processId": std::process::id(), "surface": "native-tauri-webview"
    });
    if let Some(parent) = report.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = fs::File::create_new(report).map_err(|error| error.to_string())?;
    serde_json::to_writer(file, &result).map_err(|error| error.to_string())
}

#[tauri::command]
fn smoke_exit(app: tauri::AppHandle, state: State<'_, SharedState>) -> CommandResult<()> {
    if state.report.is_none() {
        return Err("Smoke mode is disabled".to_owned());
    }
    app.exit(0);
    Ok(())
}

pub fn run() {
    let started = Instant::now();
    let startup = std::env::var_os("SELENITE_VAULT")
        .or_else(|| std::env::args_os().nth(1))
        .map(PathBuf::from);
    // Read the authoritative files while the native WebView initializes. A
    // failed worker spawn falls back to ordinary loading in startup_vault.
    let preload = startup.clone().and_then(|root| {
        thread::Builder::new()
            .name("selenite-vault-startup".to_owned())
            .spawn(move || prepare_vault(root))
            .ok()
    });
    let state = Arc::new(AppState {
        session: Mutex::new(None),
        expected: Mutex::new(None),
        startup,
        preload: Mutex::new(preload),
        started,
        report: std::env::var_os("SELENITE_SMOKE_REPORT").map(PathBuf::from),
    });
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            startup_vault,
            choose_vault,
            read_page,
            open_today,
            frontend_ready,
            smoke_exit
        ])
        .setup(|app| {
            let mut window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Selenite")
                    .inner_size(1180.0, 800.0)
                    .min_inner_size(760.0, 520.0);
            if let Some(directory) = std::env::var_os("SELENITE_WEBVIEW_DATA_DIR") {
                let directory = PathBuf::from(directory);
                fs::create_dir_all(&directory)?;
                window = window.data_directory(directory);
            }
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Selenite desktop");
}
