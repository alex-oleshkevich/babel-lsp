use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use dashmap::DashMap;
use ropey::Rope;
use tokio::sync::{Mutex, RwLock, watch};
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
    doc_locks: DashMap<Uri, Arc<Mutex<()>>>,
    utf8_encoding: AtomicBool,
    pub workspace_root: OnceLock<PathBuf>,
    pub config: RwLock<Config>,
    pub catalog_files: RwLock<Vec<PathBuf>>,
    pub catalog_index: RwLock<CatalogIndex>,
    rebuild_tx: OnceLock<watch::Sender<()>>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            doc_locks: DashMap::new(),
            utf8_encoding: AtomicBool::new(false),
            workspace_root: OnceLock::new(),
            config: RwLock::new(Config::default()),
            catalog_files: RwLock::new(vec![]),
            catalog_index: RwLock::new(CatalogIndex::default()),
            rebuild_tx: OnceLock::new(),
        }
    }

    pub fn doc_lock(&self, uri: &Uri) -> Arc<Mutex<()>> {
        self.doc_locks
            .entry(uri.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
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
}
