use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use dashmap::DashMap;
use notify::RecommendedWatcher;
use ropey::Rope;
use tokio::sync::{Mutex as AsyncMutex, RwLock, watch};
use tower_lsp_server::ls_types::Uri;

use crate::catalog::index::CatalogIndex;
use crate::config::Config;

pub struct DocumentState {
    pub rope: Rope,
    pub version: i32,
}

#[allow(dead_code)]
pub struct WorkspaceState {
    pub documents: DashMap<Uri, DocumentState>,
    doc_locks: DashMap<Uri, Arc<AsyncMutex<()>>>,
    utf8_encoding: AtomicBool,
    /// Whether the client supports `workspace/didChangeWatchedFiles` dynamic registration.
    pub client_watches_files: AtomicBool,
    /// Whether the client supports `workspace/inlayHint/refresh` (REQ-HINT-05).
    inlay_hint_refresh_support: AtomicBool,
    pub workspace_root: OnceLock<PathBuf>,
    pub config: RwLock<Config>,
    pub catalog_files: RwLock<Vec<PathBuf>>,
    pub catalog_index: RwLock<CatalogIndex>,
    rebuild_tx: OnceLock<watch::Sender<()>>,
    /// Keeps the native filesystem watcher alive for the server's lifetime.
    notify_watcher: OnceLock<Mutex<RecommendedWatcher>>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            doc_locks: DashMap::new(),
            utf8_encoding: AtomicBool::new(false),
            client_watches_files: AtomicBool::new(false),
            inlay_hint_refresh_support: AtomicBool::new(false),
            workspace_root: OnceLock::new(),
            config: RwLock::new(Config::default()),
            catalog_files: RwLock::new(vec![]),
            catalog_index: RwLock::new(CatalogIndex::default()),
            rebuild_tx: OnceLock::new(),
            notify_watcher: OnceLock::new(),
        }
    }

    pub fn doc_lock(&self, uri: &Uri) -> Arc<AsyncMutex<()>> {
        self.doc_locks
            .entry(uri.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub fn set_utf8_encoding(&self, utf8: bool) {
        self.utf8_encoding.store(utf8, Ordering::Relaxed);
    }

    pub fn is_utf8_encoding(&self) -> bool {
        self.utf8_encoding.load(Ordering::Relaxed)
    }

    /// Wire up the channel used by the debounced rebuild task.
    pub fn set_rebuild_trigger(&self, tx: watch::Sender<()>) {
        let _ = self.rebuild_tx.set(tx);
    }

    /// Signal that the catalog index should be rebuilt.
    ///
    /// No-op if [`set_rebuild_trigger`] has not been called yet.
    pub fn trigger_rebuild(&self) {
        if let Some(tx) = self.rebuild_tx.get() {
            let _ = tx.send(());
        }
    }

    /// Record whether the client supports dynamic `workspace/didChangeWatchedFiles` registration.
    pub fn set_client_watches_files(&self, val: bool) {
        self.client_watches_files.store(val, Ordering::Relaxed);
    }

    /// Returns true if the client supports dynamic `workspace/didChangeWatchedFiles` registration.
    pub fn client_watches_files(&self) -> bool {
        self.client_watches_files.load(Ordering::Relaxed)
    }

    /// Store the native filesystem watcher so it stays alive for the server's lifetime.
    pub fn set_notify_watcher(&self, watcher: RecommendedWatcher) {
        let _ = self.notify_watcher.set(Mutex::new(watcher));
    }

    /// Record whether the client supports `workspace/inlayHint/refresh` (REQ-HINT-05).
    pub fn set_inlay_hint_refresh_support(&self, val: bool) {
        self.inlay_hint_refresh_support.store(val, Ordering::Relaxed);
    }

    /// Returns true if the client supports `workspace/inlayHint/refresh`.
    pub fn inlay_hint_refresh_support(&self) -> bool {
        self.inlay_hint_refresh_support.load(Ordering::Relaxed)
    }
}
