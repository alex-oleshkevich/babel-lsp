use std::{path::{Path, PathBuf}, sync::Arc};

use ropey::Rope;
use tower_lsp_server::{
    Client, LanguageServer,
    jsonrpc::Result,
    ls_types::*,
};
use walkdir::WalkDir;

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

        let state = Arc::clone(&self.state);
        tokio::task::spawn(async move {
            if let Some(root) = state.workspace_root.get().cloned() {
                let indicators = state.config.indicators();
                let jinja_exts = state.config.jinja_extensions.clone();

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
        let uri = params.text_document.uri;
        let lock = self.state.doc_lock(&uri);
        let _guard = lock.lock().await;

        let doc = DocumentState {
            rope: Rope::from_str(&params.text_document.text),
            version: params.text_document.version,
        };
        self.state.documents.insert(uri, doc);
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
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let lock = self.state.doc_lock(&uri);
        let _guard = lock.lock().await;

        self.state.documents.remove(&uri);
    }
}

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
            || jinja_extensions.iter().any(|je| je.trim_start_matches('.') == ext))
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
    indicators.iter().filter(|ind| !ind.is_empty()).any(|ind| {
        bytes.windows(ind.len()).any(|w| w == ind.as_bytes())
    })
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
    use tempfile::TempDir;
    use tower_lsp_server::ls_types::Range;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
        Range { start: pos(sl, sc), end: pos(el, ec) }
    }

    fn change(r: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent { range: r, range_length: None, text: text.to_string() }
    }

    fn write_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn apply_full_sync_replaces_rope() {
        let mut rope = Rope::from_str("old content");
        apply_change(&mut rope, change(None, "new content"), PositionEncoding::Utf16);
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
        apply_change(&mut rope, change(Some(range(0, 1, 0, 1)), "X"), PositionEncoding::Utf16);
        apply_change(&mut rope, change(Some(range(0, 3, 0, 3)), "Y"), PositionEncoding::Utf16);
        assert_eq!(rope.to_string(), "aXbYc\n");
    }

    #[test]
    fn scan_finds_catalog_files() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "messages.po", b"msgid \"\"\n");
        write_file(&dir, "messages.pot", b"msgid \"\"\n");
        write_file(&dir, "views.py", b"x = 1\n");

        let indicators = vec!["_(".to_string()];
        let (src, catalogs) = scan_workspace(dir.path(),&indicators, &[]);

        assert_eq!(src, 0);
        assert_eq!(catalogs.len(), 2);
    }

    #[test]
    fn scan_includes_source_with_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "views.py", b"msg = _(\"Hello\")\n");

        let indicators = vec!["_(".to_string()];
        let (src, _) = scan_workspace(dir.path(),&indicators, &[]);

        assert_eq!(src, 1);
    }

    #[test]
    fn scan_excludes_source_without_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "views.py", b"x = 42\n");

        let indicators = vec!["_(".to_string()];
        let (src, _) = scan_workspace(dir.path(),&indicators, &[]);

        assert_eq!(src, 0);
    }

    #[test]
    fn scan_includes_jinja_with_indicator() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "template.html", b"<p>{% trans %}Hello{% endtrans %}</p>");

        let indicators = vec!["{% trans".to_string()];
        let jinja_exts = vec![".html".to_string()];
        let (src, _) = scan_workspace(dir.path(),&indicators, &jinja_exts);

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
}
