use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use ropey::Rope;
use tokio::sync::watch;
use tower_lsp_server::{Client, LanguageServer, jsonrpc::Result, ls_types::*};
use walkdir::WalkDir;

use crate::catalog::index::CatalogIndex;
use crate::catalog::loader::{load_po_file, load_po_from_str, locale_domain_from_po_path};
use crate::config::resolve_config;
use crate::state::{DocumentState, WorkspaceState};
use crate::util::{PositionEncoding, lsp_pos_to_char_offset};

pub struct Backend {
    client: Client,
    state: Arc<WorkspaceState>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(WorkspaceState::new()),
        }
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let utf8 = params
            .capabilities
            .general
            .as_ref()
            .and_then(|g| g.position_encodings.as_ref())
            .map(|encs| encs.contains(&PositionEncodingKind::UTF8))
            .unwrap_or(false);

        self.state.set_utf8_encoding(utf8);

        let root_uri = params
            .workspace_folders
            .as_deref()
            .and_then(|f| f.first())
            .map(|f| f.uri.clone());

        if let Some(uri) = root_uri {
            if let Some(path) = uri.to_file_path() {
                let _ = self.state.workspace_root.set(path.into_owned());
            }
        }

        let encoding = if utf8 {
            PositionEncodingKind::UTF8
        } else {
            PositionEncodingKind::UTF16
        };

        Ok(InitializeResult {
            offset_encoding: None,
            server_info: Some(ServerInfo {
                name: "babel-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(encoding),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        will_save: None,
                        will_save_wait_until: None,
                    },
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "babel-lsp initialized")
            .await;

        // Spin up the debounced catalog-rebuild task (REQ-CAT-08).
        let (rebuild_tx, mut rebuild_rx) = watch::channel(());
        self.state.set_rebuild_trigger(rebuild_tx);

        let state_for_rebuild = Arc::clone(&self.state);
        tokio::task::spawn(async move {
            loop {
                if rebuild_rx.changed().await.is_err() {
                    break; // sender dropped — server is shutting down
                }
                // Absorb burst: keep resetting the 300 ms window until quiet.
                while tokio::time::timeout(Duration::from_millis(300), rebuild_rx.changed())
                    .await
                    .is_ok()
                {}
                rebuild_catalog_index(&state_for_rebuild).await;
            }
        });

        // Workspace scan — populates catalog_files, then triggers the initial rebuild.
        let state = Arc::clone(&self.state);
        tokio::task::spawn(async move {
            if let Some(root) = state.workspace_root.get().cloned() {
                let resolved = tokio::task::spawn_blocking({
                    let root = root.clone();
                    move || resolve_config(&root)
                })
                .await
                .unwrap_or_default();
                *state.config.write().await = resolved;

                let indicators = state.config.read().await.indicators();
                let jinja_exts = state.config.read().await.jinja_extensions.clone();

                match tokio::task::spawn_blocking(move || {
                    scan_workspace(root.as_path(), &indicators, &jinja_exts)
                })
                .await
                {
                    Ok((source_count, catalog_files)) => {
                        tracing::info!(
                            source_files = source_count,
                            catalog_files = catalog_files.len(),
                            "workspace scan complete"
                        );
                        *state.catalog_files.write().await = catalog_files;
                        state.trigger_rebuild(); // REQ-CAT-08: build initial index
                    }
                    Err(e) => tracing::error!("workspace scan panicked: {e}"),
                }
            }
        });
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        {
            let lock = self.state.doc_lock(&uri);
            let _guard = lock.lock().await;
            let doc = DocumentState {
                rope: Rope::from_str(&params.text_document.text),
                version: params.text_document.version,
            };
            self.state.documents.insert(uri.clone(), doc);
        }
        // REQ-ARCH-10: every opened file receives a publish (e2e "server saw this" signal)
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let lock = self.state.doc_lock(&uri);
        let _guard = lock.lock().await;

        let enc = if self.state.is_utf8_encoding() {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        };

        if let Some(mut doc) = self.state.documents.get_mut(&uri) {
            doc.version = params.text_document.version;
            for change in params.content_changes {
                apply_change(&mut doc.rope, change, enc);
            }
        }

        if is_catalog_uri(&uri) {
            self.state.trigger_rebuild();
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let lock = self.state.doc_lock(&uri);
        let _guard = lock.lock().await;

        if let Some(text) = params.text {
            if let Some(mut doc) = self.state.documents.get_mut(&uri) {
                doc.rope = Rope::from_str(&text);
            }
        }

        if is_catalog_uri(&uri) {
            self.state.trigger_rebuild();
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let lock = self.state.doc_lock(&uri);
        let _guard = lock.lock().await;

        self.state.documents.remove(&uri);

        // Revert to disk: removing the buffer overlay means the next rebuild
        // will re-read the saved file instead.
        if is_catalog_uri(&uri) {
            self.state.trigger_rebuild();
        }
    }
}

// ── Catalog rebuild ───────────────────────────────────────────────────────────

/// Returns true if `uri` points to a `.po` or `.pot` catalog file.
fn is_catalog_uri(uri: &Uri) -> bool {
    matches!(
        uri.to_file_path()
            .as_deref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str()),
        Some("po" | "pot")
    )
}

/// Rebuild the catalog index from all known catalog files.
///
/// For each file that is currently open in the editor (REQ-CAT-07), the buffer
/// text is parsed instead of the on-disk file. If buffer parsing fails, the
/// on-disk version is used as a fallback (P3).
async fn rebuild_catalog_index(state: &Arc<WorkspaceState>) {
    let catalog_files = state.catalog_files.read().await.clone();
    let mut all_entries = vec![];

    for disk_path in &catalog_files {
        let Some((locale, domain)) = locale_domain_from_po_path(disk_path) else {
            continue;
        };

        // Check for an open buffer that overlays this file (REQ-CAT-07).
        let buffer_text = state.documents.iter().find_map(|entry| {
            let doc_path = entry.key().to_file_path()?;
            (doc_path.as_ref() == disk_path.as_path()).then(|| entry.value().rope.to_string())
        });

        let path_owned = disk_path.clone();
        let entries = if let Some(text) = buffer_text {
            let path_buf = path_owned.clone();
            let locale_c = locale.clone();
            let domain_c = domain.clone();
            let result = tokio::task::spawn_blocking(move || {
                load_po_from_str(&text, &path_buf, &locale_c, &domain_c)
            })
            .await;
            match result {
                Ok(Ok(e)) => e,
                _ => {
                    // Buffer failed to parse — fall back to disk (P3).
                    tokio::task::spawn_blocking(move || load_po_file(&path_owned, &locale, &domain))
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .unwrap_or_default()
                }
            }
        } else {
            tokio::task::spawn_blocking(move || load_po_file(&path_owned, &locale, &domain))
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default()
        };

        all_entries.extend(entries);
    }

    let entry_count = all_entries.len();
    *state.catalog_index.write().await = CatalogIndex::build(all_entries);
    tracing::info!(entries = entry_count, "catalog index rebuilt");
}

// ── Workspace scan ────────────────────────────────────────────────────────────

fn scan_workspace(
    root: &Path,
    indicators: &[String],
    jinja_extensions: &[String],
) -> (usize, Vec<PathBuf>) {
    let mut source_count = 0usize;
    let mut catalog_files = vec![];

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if ext == "po" || ext == "pot" {
            catalog_files.push(path.to_path_buf());
        } else if (ext == "py"
            || jinja_extensions
                .iter()
                .any(|je| je.trim_start_matches('.') == ext))
            && has_indicator(path, indicators)
        {
            source_count += 1;
        }
    }

    (source_count, catalog_files)
}

fn has_indicator(path: &Path, indicators: &[String]) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    indicators
        .iter()
        .filter(|ind| !ind.is_empty())
        .any(|ind| bytes.windows(ind.len()).any(|w| w == ind.as_bytes()))
}

fn apply_change(rope: &mut Rope, change: TextDocumentContentChangeEvent, enc: PositionEncoding) {
    match change.range {
        Some(range) => {
            let start = lsp_pos_to_char_offset(rope, range.start, enc);
            let end = lsp_pos_to_char_offset(rope, range.end, enc);
            rope.remove(start..end);
            if !change.text.is_empty() {
                rope.insert(start, &change.text);
            }
        }
        None => {
            *rope = Rope::from_str(&change.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::index::CatalogKey;
    use tempfile::TempDir;
    use tower_lsp_server::ls_types::{Range, Uri};

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
        Range {
            start: pos(sl, sc),
            end: pos(el, ec),
        }
    }

    fn change(r: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: r,
            range_length: None,
            text: text.to_string(),
        }
    }

    fn write_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn apply_full_sync_replaces_rope() {
        let mut rope = Rope::from_str("old content");
        apply_change(
            &mut rope,
            change(None, "new content"),
            PositionEncoding::Utf16,
        );
        assert_eq!(rope.to_string(), "new content");
    }

    #[test]
    fn apply_incremental_insert() {
        let mut rope = Rope::from_str("hello world\n");
        apply_change(
            &mut rope,
            change(Some(range(0, 5, 0, 5)), " beautiful"),
            PositionEncoding::Utf16,
        );
        assert_eq!(rope.to_string(), "hello beautiful world\n");
    }

    #[test]
    fn apply_incremental_delete() {
        let mut rope = Rope::from_str("hello world\n");
        apply_change(
            &mut rope,
            change(Some(range(0, 5, 0, 11)), ""),
            PositionEncoding::Utf16,
        );
        assert_eq!(rope.to_string(), "hello\n");
    }

    #[test]
    fn apply_incremental_replace() {
        let mut rope = Rope::from_str("foo bar\n");
        apply_change(
            &mut rope,
            change(Some(range(0, 4, 0, 7)), "baz"),
            PositionEncoding::Utf16,
        );
        assert_eq!(rope.to_string(), "foo baz\n");
    }

    #[test]
    fn apply_multiple_changes_in_order() {
        let mut rope = Rope::from_str("abc\n");
        apply_change(
            &mut rope,
            change(Some(range(0, 1, 0, 1)), "X"),
            PositionEncoding::Utf16,
        );
        apply_change(
            &mut rope,
            change(Some(range(0, 3, 0, 3)), "Y"),
            PositionEncoding::Utf16,
        );
        assert_eq!(rope.to_string(), "aXbYc\n");
    }

    #[test]
    fn scan_finds_catalog_files() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "messages.po", b"msgid \"\"\n");
        write_file(&dir, "messages.pot", b"msgid \"\"\n");
        write_file(&dir, "views.py", b"x = 1\n");

        let indicators = vec!["_(".to_string()];
        let (src, catalogs) = scan_workspace(dir.path(), &indicators, &[]);

        assert_eq!(src, 0);
        assert_eq!(catalogs.len(), 2);
    }

    #[test]
    fn scan_includes_source_with_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "views.py", b"msg = _(\"Hello\")\n");

        let indicators = vec!["_(".to_string()];
        let (src, _) = scan_workspace(dir.path(), &indicators, &[]);

        assert_eq!(src, 1);
    }

    #[test]
    fn scan_excludes_source_without_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "views.py", b"x = 42\n");

        let indicators = vec!["_(".to_string()];
        let (src, _) = scan_workspace(dir.path(), &indicators, &[]);

        assert_eq!(src, 0);
    }

    #[test]
    fn scan_includes_jinja_with_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "template.html",
            b"<p>{% trans %}Hello{% endtrans %}</p>",
        );

        let indicators = vec!["{% trans".to_string()];
        let jinja_exts = vec![".html".to_string()];
        let (src, _) = scan_workspace(dir.path(), &indicators, &jinja_exts);

        assert_eq!(src, 1);
    }

    #[test]
    fn has_indicator_detects_substring() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "f.py", b"result = gettext('hi')\n");

        assert!(has_indicator(&path, &["gettext".to_string()]));
        assert!(!has_indicator(&path, &["ngettext".to_string()]));
    }

    #[test]
    fn has_indicator_returns_false_on_unreadable() {
        assert!(!has_indicator(
            std::path::Path::new("/nonexistent/file.py"),
            &["_(".to_string()]
        ));
    }

    // ── REQ-CAT-07 / REQ-CAT-08 ──────────────────────────────────────────────

    const DISK_PO: &str = concat!(
        "msgid \"\"\n",
        "msgstr \"\"\n",
        "\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
        "\n",
        "msgid \"Disk Msg\"\n",
        "msgstr \"Datenträger\"\n",
    );

    const BUFFER_PO: &str = concat!(
        "msgid \"\"\n",
        "msgstr \"\"\n",
        "\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
        "\n",
        "msgid \"Buffer Msg\"\n",
        "msgstr \"Puffer\"\n",
    );

    fn make_uri(path: &std::path::Path) -> Uri {
        Uri::from_file_path(path).unwrap()
    }

    fn write_po(dir: &TempDir, rel: &str, content: &[u8]) -> PathBuf {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn rebuild_uses_disk_when_no_buffer_open() {
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", DISK_PO.as_bytes());

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po];

        rebuild_catalog_index(&state).await;

        let index = state.catalog_index.read().await;
        assert!(!index.lookup(&CatalogKey::new("Disk Msg")).is_empty());
        assert!(index.lookup(&CatalogKey::new("Buffer Msg")).is_empty());
    }

    #[tokio::test]
    async fn rebuild_overlays_open_buffer_content() {
        // REQ-CAT-07: buffer beats disk when the file is open.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", DISK_PO.as_bytes());

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        state.documents.insert(
            make_uri(&po),
            DocumentState {
                rope: Rope::from_str(BUFFER_PO),
                version: 1,
            },
        );

        rebuild_catalog_index(&state).await;

        let index = state.catalog_index.read().await;
        assert!(!index.lookup(&CatalogKey::new("Buffer Msg")).is_empty());
        assert!(index.lookup(&CatalogKey::new("Disk Msg")).is_empty());
    }

    #[tokio::test]
    async fn rebuild_reverts_to_disk_after_buffer_removed() {
        // Simulates didClose: once the buffer is gone, the next rebuild reads disk.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", DISK_PO.as_bytes());

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        state.documents.insert(
            make_uri(&po),
            DocumentState {
                rope: Rope::from_str(BUFFER_PO),
                version: 1,
            },
        );
        rebuild_catalog_index(&state).await;
        assert!(!state
            .catalog_index
            .read()
            .await
            .lookup(&CatalogKey::new("Buffer Msg"))
            .is_empty());

        state.documents.remove(&make_uri(&po));
        rebuild_catalog_index(&state).await;
        let index = state.catalog_index.read().await;
        assert!(!index.lookup(&CatalogKey::new("Disk Msg")).is_empty());
        assert!(index.lookup(&CatalogKey::new("Buffer Msg")).is_empty());
    }

    #[test]
    fn trigger_rebuild_before_channel_set_is_noop() {
        // Must not panic when the channel hasn't been wired up yet.
        let state = WorkspaceState::new();
        state.trigger_rebuild(); // should be silent no-op
    }

    #[test]
    fn is_catalog_uri_detects_po_and_pot() {
        let po = Uri::from_file_path("/locale/de/LC_MESSAGES/messages.po").unwrap();
        let pot = Uri::from_file_path("/locale/messages.pot").unwrap();
        let py = Uri::from_file_path("/views.py").unwrap();
        assert!(is_catalog_uri(&po));
        assert!(is_catalog_uri(&pot));
        assert!(!is_catalog_uri(&py));
    }
}
