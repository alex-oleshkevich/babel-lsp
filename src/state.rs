use dashmap::DashMap;
use ropey::Rope;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use tower_lsp_server::ls_types::Uri;

pub struct DocumentState {
    pub rope: Rope,
    pub version: i32,
}

pub struct WorkspaceState {
    pub documents: DashMap<Uri, DocumentState>,
    doc_locks: DashMap<Uri, Arc<Mutex<()>>>,
    utf8_encoding: AtomicBool,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            doc_locks: DashMap::new(),
            utf8_encoding: AtomicBool::new(false),
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
}
