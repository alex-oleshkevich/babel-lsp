use std::sync::Arc;

use ropey::Rope;
use tower_lsp_server::{
    Client, LanguageServer,
    jsonrpc::Result,
    ls_types::*,
};

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

    #[test]
    fn apply_full_sync_replaces_rope() {
        let mut rope = Rope::from_str("old content");
        apply_change(&mut rope, change(None, "new content"), PositionEncoding::Utf16);
        assert_eq!(rope.to_string(), "new content");
    }

    #[test]
    fn apply_incremental_insert() {
        let mut rope = Rope::from_str("hello world\n");
        // insert " beautiful" after "hello"
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
        // delete " world"
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
        // First: insert 'X' at col 1 → "aXbc\n"
        apply_change(&mut rope, change(Some(range(0, 1, 0, 1)), "X"), PositionEncoding::Utf16);
        // Second: insert 'Y' at col 3 in the updated rope ("aXbc\n") → "aXbYc\n"
        apply_change(&mut rope, change(Some(range(0, 3, 0, 3)), "Y"), PositionEncoding::Utf16);
        assert_eq!(rope.to_string(), "aXbYc\n");
    }
}
