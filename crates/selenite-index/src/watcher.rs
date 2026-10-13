//! One debounced reconciliation worker for a vault, with bounded event buffering.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{Index, RefreshStats};

type ErrorState = Arc<Mutex<WatchErrors>>;

#[derive(Default)]
struct WatchErrors {
    native: Option<String>,
    indexing: Option<String>,
}

impl WatchErrors {
    fn message(&self) -> Option<String> {
        match (&self.native, &self.indexing) {
            (Some(native), Some(indexing)) => Some(format!("{native}; indexing: {indexing}")),
            (Some(message), None) | (None, Some(message)) => Some(message.clone()),
            (None, None) => None,
        }
    }
}

enum Command {
    Changed,
    Flush(SyncSender<Result<RefreshStats, String>>),
    Stop,
}

/// Registers native observation before reconciliation, then owns a single
/// indexing worker. Dropping the handle stops observation and joins the worker.
pub struct VaultWatcher {
    watcher: Option<RecommendedWatcher>,
    commands: SyncSender<Command>,
    worker: Option<JoinHandle<()>>,
    last_error: ErrorState,
}

impl VaultWatcher {
    pub fn start(root: impl AsRef<Path>, debounce: Duration) -> Result<Self> {
        let root = selenite_core::vault::Vault::open(root)?.root().to_owned();
        let (commands, receiver) = mpsc::sync_channel(16);
        let last_error: ErrorState = Arc::new(Mutex::new(WatchErrors::default()));
        let event_commands = commands.clone();
        let event_errors = Arc::clone(&last_error);
        let event_root = root.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            observe(event, &event_root, &event_commands, &event_errors);
        })
        .context("create vault watcher")?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .context("watch vault recursively")?;
        let mut index = match Index::open(&root) {
            Ok(index) => index,
            Err(_) => Index::rebuild_from_disk(&root)?.0,
        };
        index.refresh()?;
        let worker_errors = Arc::clone(&last_error);
        let worker = thread::Builder::new()
            .name("selenite-index-watcher".to_owned())
            .spawn(move || run(index, receiver, debounce, worker_errors))
            .context("start vault indexing worker")?;
        Ok(Self {
            watcher: Some(watcher),
            commands,
            worker: Some(worker),
            last_error,
        })
    }

    /// Reconcile synchronously even if the operating system delivered no event.
    pub fn flush(&self) -> Result<RefreshStats> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.commands
            .send(Command::Flush(sender))
            .context("indexing worker stopped")?;
        receiver
            .recv()
            .context("indexing worker stopped before flush completed")?
            .map_err(anyhow::Error::msg)
    }

    /// Native observation errors remain visible for this handle's lifetime.
    /// A successful refresh clears only the most recent indexing error.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .message()
    }
}

impl Drop for VaultWatcher {
    fn drop(&mut self) {
        drop(self.watcher.take());
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(mut index: Index, receiver: Receiver<Command>, debounce: Duration, errors: ErrorState) {
    let mut deadline: Option<Instant> = None;
    loop {
        let command = match deadline {
            Some(deadline) => {
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::Changed) => {
                deadline = Some(
                    Instant::now()
                        .checked_add(debounce)
                        .unwrap_or_else(Instant::now),
                )
            }
            Ok(Command::Flush(reply)) => {
                deadline = None;
                let _ = reply.send(refresh(&mut index, &errors));
            }
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                deadline = None;
                let _ = refresh(&mut index, &errors);
            }
        }
    }
}

fn refresh(index: &mut Index, errors: &ErrorState) -> Result<RefreshStats, String> {
    match index.refresh() {
        Ok(stats) => {
            set_error(errors, None);
            Ok(stats)
        }
        Err(error) => {
            let message = format!("{error:#}");
            set_error(errors, Some(message.clone()));
            Err(message)
        }
    }
}

fn set_error(errors: &ErrorState, error: Option<String>) {
    errors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .indexing = error;
}

fn observe(
    event: notify::Result<Event>,
    root: &Path,
    commands: &SyncSender<Command>,
    errors: &ErrorState,
) {
    match event {
        Ok(event) if relevant(&event, root) => {
            // A queued change or flush already forces full reconciliation;
            // duplicate OS events can safely coalesce when the queue fills.
            let _ = commands.try_send(Command::Changed);
        }
        Ok(_) => {}
        Err(error) => {
            errors
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .native = Some(format!("file watcher: {error}"));
            // Native errors can mean dropped observations; reconcile the disk
            // even though the operating system could not describe a change.
            let _ = commands.try_send(Command::Changed);
        }
    }
}

fn relevant(event: &Event, root: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            let relative: PathBuf = path.strip_prefix(root).unwrap_or(path).to_owned();
            !relative
                .components()
                .any(|part| part.as_os_str() == ".selenite" || part.as_os_str() == ".git")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_error_forces_reconciliation_and_survives_indexing_success() {
        let errors = Arc::new(Mutex::new(WatchErrors::default()));
        let (commands, receiver) = mpsc::sync_channel(1);
        observe(
            Err(notify::Error::generic("observation overflow")),
            Path::new("vault"),
            &commands,
            &errors,
        );
        assert!(matches!(receiver.try_recv(), Ok(Command::Changed)));
        set_error(&errors, Some("temporary parse failure".to_owned()));
        assert!(
            errors
                .lock()
                .unwrap()
                .message()
                .unwrap()
                .contains("temporary parse failure")
        );
        set_error(&errors, None);
        let message = errors.lock().unwrap().message().unwrap();
        assert!(message.contains("observation overflow"));
        assert!(!message.contains("temporary parse failure"));
    }
}
