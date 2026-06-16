use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use notify::{RecommendedWatcher, Watcher};
use ropey::Rope;
use tokio::sync::{mpsc, watch};
use tower_lsp_server::{Client, LanguageServer, jsonrpc::Result, ls_types::*};
use walkdir::WalkDir;

use crate::catalog::index::CatalogIndex;
use crate::catalog::loader::{load_po_file, load_po_from_str, locale_domain_from_po_path};
use crate::config::{Config, discover_locale_dirs, resolve_config};
use crate::features::{code_action, code_lens, completion, definition, diagnostics, document_link, document_symbol, hardcoded, hover, inlay_hint, pybabel, references, rename};
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

        // REQ-CAT-09: detect whether the client supports dynamic file-watcher registration.
        let client_watches = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|f| f.dynamic_registration)
            .unwrap_or(false);
        self.state.set_client_watches_files(client_watches);

        let hint_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.inlay_hint.as_ref())
            .and_then(|h| h.refresh_support)
            .unwrap_or(false);
        self.state.set_inlay_hint_refresh_support(hint_refresh);

        let lens_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.code_lens.as_ref())
            .and_then(|cl| cl.refresh_support)
            .unwrap_or(false);
        self.state.set_code_lens_refresh_support(lens_refresh);

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
                // REQ-CPL-02: quote characters trigger msgid completion.
                completion_provider: Some(completion_provider_options()),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions::default(),
                ))),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: pybabel::COMMANDS.iter().map(|s| s.to_string()).collect(),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
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

        let client_for_rebuild = self.client.clone();
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
                publish_diagnostics_after_rebuild(&state_for_rebuild, &client_for_rebuild).await;
            }
        });

        // Workspace scan — populates catalog_files, then triggers the initial rebuild.
        let client = self.client.clone();
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

                // REQ-CAT-09: register watcher with client or fall back to notify.
                let locale_dirs = {
                    let cfg = state.config.read().await;
                    discover_locale_dirs(&root, &cfg)
                };
                if state.client_watches_files() {
                    register_lsp_watcher(&client).await;
                } else {
                    start_notify_watcher(&state, &locale_dirs);
                }

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

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let enc = if self.state.is_utf8_encoding() {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        };

        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let rope = doc.rope.clone();
        drop(doc);

        let text = rope.to_string();
        let index = self.state.catalog_index.read().await;
        let config = self.state.config.read().await;
        let items = extract_and_complete(&rope, &text, &uri, pos, enc, &index, &config);

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let index = self.state.catalog_index.read().await;

        // Catalog file (.po/.pot): resolve by line number from loaded entries.
        if is_catalog_uri(&uri) {
            if let Some(path) = uri.to_file_path() {
                let file_entries = index.entries_for_file(&path);
                return Ok(hover::hover_catalog(&file_entries, pos, &index));
            }
            return Ok(None);
        }

        // Source file: extract calls, find the one under the cursor.
        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let rope = doc.rope.clone();
        drop(doc);
        let text = rope.to_string();
        let config = self.state.config.read().await;
        let calls = extract_calls(&text, &uri, &config);
        Ok(hover::hover_source(&calls, pos, &index))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        if is_catalog_uri(&uri) {
            return Ok(None); // definition from .po goes nowhere meaningful
        }
        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let rope = doc.rope.clone();
        drop(doc);
        let text = rope.to_string();
        let config = self.state.config.read().await;
        let calls = extract_calls(&text, &uri, &config);
        drop(config);
        let index = self.state.catalog_index.read().await;
        Ok(definition::goto_definition(&calls, pos, &index))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        // Resolve the key from the cursor position.
        let key = if is_catalog_uri(&uri) {
            let index = self.state.catalog_index.read().await;
            if let Some(path) = uri.to_file_path() {
                let file_entries = index.entries_for_file(&path);
                let cursor_line = pos.line + 1;
                file_entries
                    .iter()
                    .find(|e| e.line == cursor_line)
                    .map(|e| e.key())
            } else {
                None
            }
        } else {
            let Some(doc) = self.state.documents.get(&uri) else {
                return Ok(None);
            };
            let rope = doc.rope.clone();
            drop(doc);
            let text = rope.to_string();
            let config = self.state.config.read().await;
            let calls = extract_calls(&text, &uri, &config);
            drop(config);
            calls
                .iter()
                .find(|c| {
                    c.msgid.is_some()
                        && c.msgid_range.is_some_and(|r| crate::util::pos_in_range(pos, r))
                })
                .map(|c| crate::catalog::index::CatalogKey {
                    msgid: c.msgid.clone().unwrap_or_default(),
                    msgctxt: c.msgctxt.clone(),
                })
        };

        let Some(key) = key else { return Ok(None) };

        // Collect call sites: open documents.
        let config = self.state.config.read().await;
        let mut call_sites: Vec<(Uri, Vec<crate::extract::types::TranslationCall>)> = Vec::new();

        for entry in self.state.documents.iter() {
            let doc_uri = entry.key().clone();
            if is_catalog_uri(&doc_uri) {
                continue;
            }
            let text = entry.value().rope.to_string();
            let calls = extract_calls(&text, &doc_uri, &config);
            if !calls.is_empty() {
                call_sites.push((doc_uri, calls));
            }
        }

        // Workspace scan (REQ-NAV-06).
        if let Some(root) = self.state.workspace_root.get() {
            let open_uris: std::collections::HashSet<String> = call_sites
                .iter()
                .map(|(u, _)| u.to_string())
                .collect();
            let jinja_exts: Vec<String> = config.jinja_extensions.clone();
            let root = root.clone();
            let extra = config
                .extra_keywords
                .iter()
                .filter_map(|kw| {
                    crate::extract::types::TranslationFunc::from_name(kw)
                        .map(|f| (kw.clone(), f))
                })
                .collect::<std::collections::HashMap<_, _>>();
            drop(config);

            // Walk on a blocking thread to avoid holding async executor.
            let scanned = tokio::task::spawn_blocking(move || {
                scan_workspace_for_calls(&root, &jinja_exts, &extra, &open_uris)
            })
            .await
            .unwrap_or_default();
            call_sites.extend(scanned);
        } else {
            drop(config);
        }

        let index = self.state.catalog_index.read().await;
        let locs = references::find_references(&key, &index, call_sites);
        Ok(if locs.is_empty() { None } else { Some(locs) })
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = params.text_document.uri;
        if !is_catalog_uri(&uri) {
            return Ok(None);
        }
        // Use the open buffer if available; otherwise return None.
        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let text = doc.rope.to_string();
        drop(doc);

        let workspace_root = self.state.workspace_root.get().map(|p| p.as_path());
        let catalog_dir = uri
            .to_file_path()
            .map(|p| p.parent().map(|d| d.to_path_buf()).unwrap_or_default())
            .unwrap_or_default();

        let links = document_link::document_links(&text, &catalog_dir, workspace_root);
        Ok(if links.is_empty() { None } else { Some(links) })
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        if is_catalog_uri(&uri) {
            return Ok(None);
        }

        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let text = doc.rope.to_string();
        drop(doc);

        let config = self.state.config.read().await;
        let Some(locale) = config.inlay_hint_locale.clone() else { return Ok(None) };
        let calls = extract_calls(&text, &uri, &config);
        drop(config);

        let index = self.state.catalog_index.read().await;
        let hints = inlay_hint::inlay_hints(&calls, &index, Some(&locale), params.range);
        Ok(if hints.is_empty() { None } else { Some(hints) })
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        if !is_catalog_uri(&uri) {
            return Ok(None);
        }
        let Some(path) = uri.to_file_path() else { return Ok(None) };
        let index = self.state.catalog_index.read().await;
        let entries = index.entries_for_file(&path);
        let symbols = document_symbol::document_symbols(&entries);
        Ok(if symbols.is_empty() {
            None
        } else {
            Some(DocumentSymbolResponse::Nested(symbols))
        })
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let index = self.state.catalog_index.read().await;
        let symbols = document_symbol::workspace_symbols(&index, &params.query);
        Ok(if symbols.is_empty() {
            None
        } else {
            Some(WorkspaceSymbolResponse::Nested(symbols))
        })
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.clone();
        let is_catalog = is_catalog_uri(&uri);
        let is_config = is_config_uri(&uri);
        let has_hardcoded_diag = params.context.diagnostics.iter().any(|d| {
            matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string")
        });

        if !is_catalog && !is_config && !has_hardcoded_diag {
            return Ok(None);
        }

        // Handle hardcoded-string extract actions for Python source files.
        // Placed before has_locale_dirs to avoid an unnecessary config read.
        if has_hardcoded_diag && !is_catalog && !is_config {
            let config = self.state.config.read().await;
            if !config.detect_hardcoded_strings {
                return Ok(None);
            }
            let keyword: String = config
                .extra_keywords
                .iter()
                .find(|kw| {
                    crate::extract::types::TranslationFunc::from_name(kw)
                        .map(|f| !f.has_domain() && !f.has_context() && !f.has_plural())
                        .unwrap_or(false)
                })
                .cloned()
                .unwrap_or_else(|| "_".to_string());
            drop(config);

            let source = {
                let Some(doc) = self.state.documents.get(&uri) else { return Ok(None) };
                doc.rope.to_string()
            };

            let index = self.state.catalog_index.read().await;
            let pot_path = index.pot_file_path().map(|p| p.to_path_buf());
            let po_paths: Vec<PathBuf> =
                index.po_file_paths().into_iter().map(|p| p.to_path_buf()).collect();

            let pot_info: Option<(Uri, String)> = pot_path.as_ref().and_then(|path| {
                let pot_uri = Uri::from_file_path(path)?;
                let content = self
                    .state
                    .documents
                    .get(&pot_uri)
                    .map(|d| d.rope.to_string())
                    .or_else(|| std::fs::read_to_string(path).ok())?;
                Some((pot_uri, content))
            });

            let locale_po_contents: Vec<(Uri, String)> = po_paths
                .iter()
                .filter_map(|path| {
                    let po_uri = Uri::from_file_path(path)?;
                    let content = self
                        .state
                        .documents
                        .get(&po_uri)
                        .map(|d| d.rope.to_string())
                        .or_else(|| std::fs::read_to_string(path).ok())?;
                    Some((po_uri, content))
                })
                .collect();

            let locale_po_refs: Vec<(&Uri, &str)> =
                locale_po_contents.iter().map(|(u, c)| (u, c.as_str())).collect();

            let actions = hardcoded::code_actions_for_hardcoded(
                &params.context.diagnostics,
                &source,
                &uri,
                &keyword,
                pot_info.as_ref().map(|(u, c)| (u, c.as_str())),
                &index,
                &locale_po_refs,
            );
            drop(index);

            if actions.is_empty() {
                return Ok(None);
            }
            return Ok(Some(
                actions.into_iter().map(CodeActionOrCommand::CodeAction).collect(),
            ));
        }

        let has_locale_dirs = {
            let config = self.state.config.read().await;
            self.state
                .workspace_root
                .get()
                .map(|root| !discover_locale_dirs(root, &config).is_empty())
                .unwrap_or(false)
        };

        if is_catalog {
            let Some(doc) = self.state.documents.get(&uri) else {
                return Ok(None);
            };
            let content = doc.rope.to_string();
            drop(doc);

            let Some(path) = uri.to_file_path() else { return Ok(None) };
            let index = self.state.catalog_index.read().await;
            let entries = index.entries_for_file(&path);
            let mut actions =
                code_action::code_actions_for_po(&params, &content, &entries, &uri);
            actions.extend(code_action::command_actions_for_po(has_locale_dirs));
            if actions.is_empty() {
                return Ok(None);
            }
            return Ok(Some(
                actions.into_iter().map(CodeActionOrCommand::CodeAction).collect(),
            ));
        }

        if is_config {
            let actions = code_action::command_actions_for_config(has_locale_dirs);
            if actions.is_empty() {
                return Ok(None);
            }
            return Ok(Some(
                actions.into_iter().map(CodeActionOrCommand::CodeAction).collect(),
            ));
        }

        Ok(None)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;
        let Some(doc) = self.state.documents.get(&uri) else {
            return Ok(None);
        };
        let text = doc.rope.to_string();
        drop(doc);

        let lenses = if is_catalog_uri(&uri) {
            let Some(path) = uri.to_file_path() else { return Ok(None) };
            let index = self.state.catalog_index.read().await;
            let entries = index.entries_for_file(&path);
            code_lens::code_lenses_catalog(&text, &entries)
        } else {
            let config = self.state.config.read().await;
            let calls = extract_calls(&text, &uri, &config);
            drop(config);
            code_lens::code_lenses_source(&calls)
        };

        Ok(if lenses.is_empty() { None } else { Some(lenses) })
    }

    async fn code_lens_resolve(&self, lens: CodeLens) -> Result<CodeLens> {
        let Some(ref data_val) = lens.data else {
            return Ok(lens);
        };
        let Ok(data) = serde_json::from_value::<code_lens::LensData>(data_val.clone()) else {
            return Ok(lens);
        };

        let index = self.state.catalog_index.read().await;

        let all_source_calls = if matches!(data.kind, code_lens::LensKind::Source) {
            let open_docs: Vec<(Uri, String)> = self
                .state
                .documents
                .iter()
                .filter_map(|entry| {
                    let uri = entry.key().clone();
                    if is_catalog_uri(&uri) {
                        return None;
                    }
                    Some((uri, entry.value().rope.to_string()))
                })
                .collect();
            let config = self.state.config.read().await;
            open_docs
                .iter()
                .flat_map(|(uri, text)| extract_calls(text, uri, &config))
                .collect()
        } else {
            vec![]
        };

        Ok(code_lens::resolve_lens(lens, &index, &all_source_calls))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let pos = params.position;

        if is_catalog_uri(uri) {
            let Some(path) = uri.to_file_path() else { return Ok(None) };
            let Some(doc) = self.state.documents.get(uri) else { return Ok(None) };
            let content = doc.rope.to_string();
            drop(doc);
            let lines: Vec<&str> = content.lines().collect();
            let index = self.state.catalog_index.read().await;
            let entries = index.entries_for_file(&path);
            let result = rename::prepare_rename_catalog(&entries, &lines, pos);
            drop(index);
            Ok(result.map(|(range, _)| PrepareRenameResponse::Range(range)))
        } else {
            let Some(doc) = self.state.documents.get(uri) else { return Ok(None) };
            let content = doc.rope.to_string();
            drop(doc);
            let config = self.state.config.read().await;
            let calls = extract_calls(&content, uri, &config);
            drop(config);
            let result = rename::prepare_rename_source(&calls, pos);
            Ok(result.map(|(range, _)| PrepareRenameResponse::Range(range)))
        }
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = &params.new_name;

        // Resolve the CatalogKey from the cursor position.
        let key = if is_catalog_uri(uri) {
            let Some(path) = uri.to_file_path() else { return Ok(None) };
            let Some(doc) = self.state.documents.get(uri) else { return Ok(None) };
            let content = doc.rope.to_string();
            drop(doc);
            let lines: Vec<&str> = content.lines().collect();
            let index = self.state.catalog_index.read().await;
            let entries = index.entries_for_file(&path);
            let key = rename::prepare_rename_catalog(&entries, &lines, pos).map(|(_, k)| k);
            drop(index);
            key
        } else {
            let Some(doc) = self.state.documents.get(uri) else { return Ok(None) };
            let content = doc.rope.to_string();
            drop(doc);
            let config = self.state.config.read().await;
            let calls = extract_calls(&content, uri, &config);
            drop(config);
            rename::prepare_rename_source(&calls, pos).map(|(_, k)| k)
        };

        let Some(key) = key else { return Ok(None) };

        let index = self.state.catalog_index.read().await;

        // Collect buffer content for every catalog file that touches the key.
        let catalog_entries: Vec<_> = {
            let mut v: Vec<_> = index.lookup(&key).into_iter().cloned().collect();
            if let Some(pot) = index.lookup_pot(&key) {
                v.push(pot.clone());
            }
            v
        };

        let mut catalog_bufs: std::collections::HashMap<std::path::PathBuf, String> =
            std::collections::HashMap::new();
        for entry in &catalog_entries {
            if catalog_bufs.contains_key(&entry.file_path) {
                continue;
            }
            let content = Uri::from_file_path(&entry.file_path)
                .as_ref()
                .and_then(|cu| self.state.documents.get(cu))
                .map(|d| d.rope.to_string());
            let content = if let Some(c) = content {
                c
            } else {
                let path = entry.file_path.clone();
                tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or_default()
            };
            catalog_bufs.insert(entry.file_path.clone(), content);
        }

        // Collect source call sites (open buffers + workspace scan).
        let config = self.state.config.read().await;
        let mut call_sites: Vec<(Uri, Vec<crate::extract::types::TranslationCall>)> = Vec::new();
        for entry in self.state.documents.iter() {
            let doc_uri = entry.key().clone();
            if is_catalog_uri(&doc_uri) { continue; }
            let text = entry.value().rope.to_string();
            let calls = extract_calls(&text, &doc_uri, &config);
            if !calls.is_empty() {
                call_sites.push((doc_uri, calls));
            }
        }
        if let Some(root) = self.state.workspace_root.get() {
            let open_uris: std::collections::HashSet<String> =
                call_sites.iter().map(|(u, _)| u.to_string()).collect();
            let jinja_exts = config.jinja_extensions.clone();
            let root = root.clone();
            let extra = config
                .extra_keywords
                .iter()
                .filter_map(|kw| {
                    crate::extract::types::TranslationFunc::from_name(kw)
                        .map(|f| (kw.clone(), f))
                })
                .collect::<std::collections::HashMap<_, _>>();
            drop(config);
            let scanned = tokio::task::spawn_blocking(move || {
                scan_workspace_for_calls(&root, &jinja_exts, &extra, &open_uris)
            })
            .await
            .unwrap_or_default();
            call_sites.extend(scanned);
        } else {
            drop(config);
        }

        match rename::build_rename_edit(&key, new_name, &index, &catalog_bufs, &call_sites) {
            Ok(edit) => Ok(Some(edit)),
            Err(msg) => Err(tower_lsp_server::jsonrpc::Error {
                code: tower_lsp_server::jsonrpc::ErrorCode::InvalidParams,
                message: msg.into(),
                data: None,
            }),
        }
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<LSPAny>> {
        let op = match params.command.as_str() {
            "babel-lsp.extract" => pybabel::PybabelOp::Extract,
            "babel-lsp.update" => pybabel::PybabelOp::Update,
            "babel-lsp.compile" => pybabel::PybabelOp::Compile,
            other => {
                return Err(tower_lsp_server::jsonrpc::Error {
                    code: tower_lsp_server::jsonrpc::ErrorCode::MethodNotFound,
                    message: format!("unknown command: {other}").into(),
                    data: None,
                });
            }
        };

        let locale = params
            .arguments
            .first()
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let Some(root) = self.state.workspace_root.get().cloned() else {
            return Ok(None);
        };

        let config = self.state.config.read().await.clone();
        let locale_dirs = discover_locale_dirs(&root, &config);

        let opts = pybabel::RunOptions {
            pybabel_path: config.pybabel_path,
            locale_dirs,
            domains: config.domains,
            locale,
            workspace_root: root,
        };

        let client = self.client.clone();
        let state = Arc::clone(&self.state);

        let result = tokio::task::spawn_blocking(move || pybabel::run_pybabel(op, &opts))
            .await
            .unwrap_or(pybabel::RunResult::Failure {
                exit_code: -1,
                stderr: "task failed".into(),
            });

        match result {
            pybabel::RunResult::Success => {
                state.trigger_rebuild();
            }
            pybabel::RunResult::Failure { stderr, .. } => {
                client.show_message(MessageType::ERROR, stderr).await;
            }
            pybabel::RunResult::NotFound => {
                client
                    .show_message(
                        MessageType::ERROR,
                        "pybabel not found — install Babel or set `pybabel_path` in config",
                    )
                    .await;
            }
        }

        Ok(None)
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // REQ-CAT-09: client-side watcher events for .po/.pot files.
        let mut needs_rebuild = false;
        for event in &params.changes {
            if !is_catalog_uri(&event.uri) {
                continue;
            }
            if handle_catalog_file_event(&event.uri, event.typ, &self.state).await {
                needs_rebuild = true;
            }
        }
        if needs_rebuild {
            self.state.trigger_rebuild();
        }
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
        // REQ-ARCH-10: every opened file receives an immediate publish.
        let config = self.state.config.read().await;
        let index = self.state.catalog_index.read().await;
        let diags = if is_catalog_uri(&uri) {
            let path = uri.to_file_path().unwrap_or_default();
            let file_entries = index.entries_for_file(&path);
            diagnostics::check_catalog(&file_entries, &uri, &index)
        } else {
            let calls = extract_calls(&params.text_document.text, &uri, &config);
            let mut diags = diagnostics::check_source(&calls, &index);
            if config.detect_hardcoded_strings {
                let extra = extra_keywords(&config);
                diags.extend(hardcoded::check_source(
                    params.text_document.text.as_bytes(),
                    &uri,
                    &extra,
                ));
            }
            diags
        };
        let filtered = diagnostics::apply_diag_filter(diags, &config.diagnostics);
        drop(index);
        drop(config);
        self.client.publish_diagnostics(uri, filtered, None).await;
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
        } else if let Some(doc) = self.state.documents.get(&uri) {
            let text = doc.rope.to_string();
            drop(doc);
            let config = self.state.config.read().await;
            let index = self.state.catalog_index.read().await;
            let calls = extract_calls(&text, &uri, &config);
            let mut diags = diagnostics::check_source(&calls, &index);
            if config.detect_hardcoded_strings {
                let extra = extra_keywords(&config);
                diags.extend(hardcoded::check_source(text.as_bytes(), &uri, &extra));
            }
            let filtered = diagnostics::apply_diag_filter(diags, &config.diagnostics);
            drop(index);
            drop(config);
            self.client.publish_diagnostics(uri, filtered, None).await;
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
        } else if let Some(doc) = self.state.documents.get(&uri) {
            // Re-publish source diagnostics: the saved text may differ from the last
            // did_change state (e.g. formatter ran on save, or client sends full text
            // only on save), so stale diagnostics would otherwise linger.
            let text = doc.rope.to_string();
            drop(doc);
            let config = self.state.config.read().await;
            let index = self.state.catalog_index.read().await;
            let calls = extract_calls(&text, &uri, &config);
            let mut diags = diagnostics::check_source(&calls, &index);
            if config.detect_hardcoded_strings {
                let extra = extra_keywords(&config);
                diags.extend(hardcoded::check_source(text.as_bytes(), &uri, &extra));
            }
            let filtered = diagnostics::apply_diag_filter(diags, &config.diagnostics);
            drop(index);
            drop(config);
            self.client.publish_diagnostics(uri, filtered, None).await;
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

/// Returns true if `uri` points to `babel.cfg` or `pyproject.toml` — files where
/// the "Extract messages" command action is anchored (REQ-CMD-03).
fn is_config_uri(uri: &Uri) -> bool {
    uri.to_file_path()
        .as_deref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|n| n == "babel.cfg" || n == "pyproject.toml")
        .unwrap_or(false)
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

/// Publish diagnostics for all catalog files and open source files.
///
/// Called immediately after [`rebuild_catalog_index`] so every diagnostic
/// consumer sees a consistent view of the new index.
async fn publish_diagnostics_after_rebuild(state: &Arc<WorkspaceState>, client: &Client) {
    let config = state.config.read().await.clone();
    let catalog_files = state.catalog_files.read().await.clone();
    let open_sources: Vec<(Uri, String)> = state
        .documents
        .iter()
        .filter_map(|entry| {
            let uri = entry.key().clone();
            if is_catalog_uri(&uri) { return None; }
            Some((uri, entry.value().rope.to_string()))
        })
        .collect();

    // Collect all source calls for project checks (open buffers + disk scan).
    let all_calls = collect_all_source_calls(state, &open_sources, &config).await;

    let index = state.catalog_index.read().await;

    // Catalog + project diagnostics merged by URI.
    // Pre-insert every catalog URI with an empty Vec so that files with no
    // new findings still get published (clearing previously-shown diagnostics).
    let mut catalog_diags: HashMap<Uri, Vec<Diagnostic>> = HashMap::new();
    for disk_path in &catalog_files {
        if let Some(uri) = Uri::from_file_path(disk_path) {
            catalog_diags.insert(uri, vec![]);
        }
    }
    for disk_path in &catalog_files {
        let Some(uri) = Uri::from_file_path(disk_path) else { continue };
        let file_entries = index.entries_for_file(disk_path);
        catalog_diags
            .entry(uri.clone())
            .or_default()
            .extend(diagnostics::check_catalog(&file_entries, &uri, &index));
    }
    for (uri, pdiags) in diagnostics::check_project(&index, &all_calls) {
        catalog_diags.entry(uri).or_default().extend(pdiags);
    }
    for (uri, diags) in catalog_diags {
        let filtered = diagnostics::apply_diag_filter(diags, &config.diagnostics);
        client.publish_diagnostics(uri, filtered, None).await;
    }

    // Source-side diagnostics: re-check all open source files.
    for (uri, text) in &open_sources {
        let calls = extract_calls(text, uri, &config);
        let mut diags = diagnostics::check_source(&calls, &index);
        if config.detect_hardcoded_strings {
            let extra = extra_keywords(&config);
            diags.extend(hardcoded::check_source(text.as_bytes(), uri, &extra));
        }
        let filtered = diagnostics::apply_diag_filter(diags, &config.diagnostics);
        client.publish_diagnostics(uri.clone(), filtered, None).await;
    }

    // REQ-HINT-05: tell the client to re-request inlay hints for all open files.
    if state.inlay_hint_refresh_support() {
        let _ = client.inlay_hint_refresh().await;
    }

    // REQ-LENS-07: tell the client to re-request code lenses after a catalog rebuild.
    if state.code_lens_refresh_support() {
        let _ = client.code_lens_refresh().await;
    }
}

/// Collect all source-file translation calls for project-level checks.
///
/// Combines open-buffer calls with a disk scan of workspace source files that
/// are not currently open in the editor.
async fn collect_all_source_calls(
    state: &Arc<WorkspaceState>,
    open_sources: &[(Uri, String)],
    config: &Config,
) -> Vec<crate::extract::types::TranslationCall> {
    let mut calls: Vec<crate::extract::types::TranslationCall> = open_sources
        .iter()
        .flat_map(|(uri, text)| extract_calls(text, uri, config))
        .collect();

    let Some(root) = state.workspace_root.get().cloned() else {
        return calls;
    };

    let skip_uris: std::collections::HashSet<String> =
        open_sources.iter().map(|(uri, _)| uri.to_string()).collect();
    let extra: std::collections::HashMap<String, crate::extract::types::TranslationFunc> = config
        .extra_keywords
        .iter()
        .filter_map(|kw| {
            crate::extract::types::TranslationFunc::from_name(kw).map(|f| (kw.clone(), f))
        })
        .collect();
    let jinja_exts = config.jinja_extensions.clone();

    let disk = tokio::task::spawn_blocking(move || {
        scan_workspace_for_calls(&root, &jinja_exts, &extra, &skip_uris)
    })
    .await
    .unwrap_or_default();

    for (_, file_calls) in disk {
        calls.extend(file_calls);
    }

    calls
}

// ── Source file dispatch helpers ──────────────────────────────────────────────

fn extra_keywords(
    config: &Config,
) -> std::collections::HashMap<String, crate::extract::types::TranslationFunc> {
    config
        .extra_keywords
        .iter()
        .filter_map(|kw| {
            crate::extract::types::TranslationFunc::from_name(kw).map(|f| (kw.clone(), f))
        })
        .collect()
}

/// Extract translation calls from a source file, picking the right extractor
/// based on the file extension. Returns an empty vec for unknown file types.
fn extract_calls(
    text: &str,
    uri: &Uri,
    config: &Config,
) -> Vec<crate::extract::types::TranslationCall> {
    let ext = uri
        .to_file_path()
        .and_then(|p| p.extension().and_then(|e| e.to_str()).map(str::to_owned))
        .unwrap_or_default();

    let extra = config
        .extra_keywords
        .iter()
        .filter_map(|kw| {
            crate::extract::types::TranslationFunc::from_name(kw)
                .map(|f| (kw.clone(), f))
        })
        .collect::<std::collections::HashMap<_, _>>();

    if ext == "py" {
        crate::extract::python::extract(text.as_bytes(), &extra)
    } else if config
        .jinja_extensions
        .iter()
        .any(|je| je.trim_start_matches('.') == ext)
    {
        crate::extract::jinja::extract(text.as_bytes(), &extra)
    } else {
        vec![]
    }
}

fn extract_and_complete(
    rope: &ropey::Rope,
    text: &str,
    uri: &Uri,
    pos: Position,
    enc: PositionEncoding,
    index: &CatalogIndex,
    config: &Config,
) -> Vec<CompletionItem> {
    let calls = extract_calls(text, uri, config);
    if calls.is_empty() {
        return vec![];
    }
    completion::complete(rope, &calls, pos, enc, index)
}

// ── External change handling (REQ-CAT-09/10) ─────────────────────────────────

/// Register `workspace/didChangeWatchedFiles` with the client for *.po and *.pot.
async fn register_lsp_watcher(client: &Client) {
    let opts = DidChangeWatchedFilesRegistrationOptions {
        watchers: vec![
            FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.po".to_string()),
                kind: None,
            },
            FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.pot".to_string()),
                kind: None,
            },
        ],
    };
    let registration = Registration {
        id: "babel-lsp-catalog-watcher".to_string(),
        method: "workspace/didChangeWatchedFiles".to_string(),
        register_options: Some(serde_json::to_value(opts).expect("serializable")),
    };
    if let Err(e) = client.register_capability(vec![registration]).await {
        tracing::warn!("failed to register file watcher: {e}");
    }
}

/// Update `catalog_files` for a single LSP file event.
///
/// Returns `true` when the change warrants a rebuild; `false` when the event
/// should be ignored (e.g. the file is open in the editor — REQ-CAT-10).
async fn handle_catalog_file_event(
    uri: &Uri,
    change_type: FileChangeType,
    state: &Arc<WorkspaceState>,
) -> bool {
    // REQ-CAT-10: if the file is open, the buffer is authoritative — skip watcher events.
    if state.documents.contains_key(uri) {
        return false;
    }
    let Some(path) = uri.to_file_path().map(|p| p.into_owned()) else {
        return false;
    };

    if change_type == FileChangeType::CREATED {
        state.catalog_files.write().await.push(path);
        return true;
    }
    if change_type == FileChangeType::DELETED {
        state.catalog_files.write().await.retain(|p| p != &path);
        return true;
    }
    // CHANGED — the file still exists; rebuild will re-read it from disk.
    true
}

/// Start a native `notify` watcher over `locale_dirs` when the client doesn't
/// support dynamic registration (REQ-CAT-09 fallback).
fn start_notify_watcher(state: &Arc<WorkspaceState>, locale_dirs: &[PathBuf]) {
    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Event>();

    let watcher_result = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        notify::Config::default(),
    );

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("failed to create notify watcher: {e}");
            return;
        }
    };

    for dir in locale_dirs {
        if let Err(e) = watcher.watch(dir, notify::RecursiveMode::Recursive) {
            tracing::warn!("failed to watch {}: {e}", dir.display());
        }
    }

    // Keep the watcher alive for the server's lifetime.
    state.set_notify_watcher(watcher);

    let state = Arc::clone(state);
    tokio::task::spawn(async move {
        while let Some(event) = rx.recv().await {
            handle_notify_event(event, &state).await;
        }
    });
}

/// Process a single notify event: update catalog_files and trigger rebuild.
async fn handle_notify_event(event: notify::Event, state: &Arc<WorkspaceState>) {
    let mut needs_rebuild = false;
    for path in &event.paths {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "po" && ext != "pot" {
            continue;
        }

        // REQ-CAT-10: ignore events for files open in the editor.
        if is_open_in_editor(path, state) {
            continue;
        }

        if event.kind.is_create() {
            state.catalog_files.write().await.push(path.clone());
            needs_rebuild = true;
        } else if event.kind.is_remove() {
            state.catalog_files.write().await.retain(|p| p != path);
            needs_rebuild = true;
        } else if event.kind.is_modify() {
            // Rename events (Modify(Name(_))) arrive here with [old, new] paths.
            // Use existence to distinguish: missing path was renamed away (delete),
            // path not yet tracked was renamed in (create). Pure content changes
            // are already tracked — just rebuild.
            if !path.exists() {
                state.catalog_files.write().await.retain(|p| p != path);
            } else {
                let mut files = state.catalog_files.write().await;
                if !files.contains(path) {
                    files.push(path.clone());
                }
            }
            needs_rebuild = true;
        }
    }
    if needs_rebuild {
        state.trigger_rebuild();
    }
}

/// Returns true if `path` is currently open in the editor (has an overlay buffer).
fn is_open_in_editor(path: &Path, state: &Arc<WorkspaceState>) -> bool {
    state.documents.iter().any(|entry| {
        entry
            .key()
            .to_file_path()
            .map(|p| p.as_ref() == path)
            .unwrap_or(false)
    })
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

/// REQ-NAV-06: walk from `root`, collect source files, skip noise directories.
fn scan_workspace_for_calls(
    root: &std::path::Path,
    jinja_exts: &[String],
    extra: &std::collections::HashMap<String, crate::extract::types::TranslationFunc>,
    skip_uris: &std::collections::HashSet<String>,
) -> Vec<(Uri, Vec<crate::extract::types::TranslationCall>)> {
    const PRUNE: &[&str] = &[
        ".git", "target", ".venv", "venv", "__pycache__", ".mypy_cache", ".pytest_cache",
    ];

    let mut results = Vec::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            !e.file_type().is_dir()
                || e.file_name()
                    .to_str()
                    .map(|n| !PRUNE.contains(&n))
                    .unwrap_or(true)
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        let is_py = ext == "py";
        let is_jinja = jinja_exts
            .iter()
            .any(|je| je.trim_start_matches('.') == ext);

        if !is_py && !is_jinja {
            continue;
        }

        let Some(uri) = Uri::from_file_path(path) else { continue };
        if skip_uris.contains(&uri.to_string()) {
            continue; // already covered by an open document
        }

        let Ok(bytes) = std::fs::read(path) else { continue };
        let calls = if is_py {
            crate::extract::python::extract(&bytes, extra)
        } else {
            crate::extract::jinja::extract(&bytes, extra)
        };

        if !calls.is_empty() {
            results.push((uri, calls));
        }
    }
    results
}

fn completion_provider_options() -> CompletionOptions {
    CompletionOptions {
        trigger_characters: Some(vec!["\"".to_string(), "'".to_string()]),
        ..CompletionOptions::default()
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

    // ── REQ-CAT-09 / REQ-CAT-10 (handle_catalog_file_event) ─────────────────

    #[tokio::test]
    async fn file_event_created_adds_to_catalog_files() {
        // REQ-CAT-09: CREATED event registers the new file.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        let uri = make_uri(&po);
        let added = handle_catalog_file_event(&uri, FileChangeType::CREATED, &state).await;

        assert!(added);
        assert!(state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn file_event_deleted_removes_from_catalog_files() {
        // REQ-CAT-09: DELETED event unregisters the file.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        let uri = make_uri(&po);
        let changed = handle_catalog_file_event(&uri, FileChangeType::DELETED, &state).await;

        assert!(changed);
        assert!(!state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn file_event_changed_triggers_rebuild() {
        // REQ-CAT-09: CHANGED event returns true so the caller triggers a rebuild.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        let uri = make_uri(&po);
        let changed = handle_catalog_file_event(&uri, FileChangeType::CHANGED, &state).await;

        assert!(changed);
        // catalog_files is unchanged; rebuild will re-read the file.
        assert!(state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn file_event_ignored_when_file_is_open() {
        // REQ-CAT-10: watcher events for open files are silently dropped.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        // Simulate file open in editor.
        state.documents.insert(
            make_uri(&po),
            DocumentState {
                rope: Rope::from_str(""),
                version: 1,
            },
        );

        let uri = make_uri(&po);
        let changed = handle_catalog_file_event(&uri, FileChangeType::CHANGED, &state).await;

        assert!(!changed, "event for an open file must be ignored");
    }

    #[tokio::test]
    async fn notify_event_create_adds_file() {
        // REQ-CAT-09 fallback: notify Create event adds path to catalog_files.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        let event = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![po.clone()],
            attrs: Default::default(),
        };
        handle_notify_event(event, &state).await;

        assert!(state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn notify_event_remove_drops_file() {
        // REQ-CAT-09 fallback: notify Remove event drops path from catalog_files.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        let event = notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![po.clone()],
            attrs: Default::default(),
        };
        handle_notify_event(event, &state).await;

        assert!(!state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn notify_event_modify_triggers_rebuild_for_non_open_file() {
        // Content change to a tracked, non-open file must trigger rebuild.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![po.clone()];

        let (tx, mut rx) = watch::channel(());
        state.set_rebuild_trigger(tx);

        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![po.clone()],
            attrs: Default::default(),
        };
        handle_notify_event(event, &state).await;

        // Must not timeout — rebuild was triggered.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.changed())
                .await
                .is_ok(),
            "rebuild must be triggered for a modified non-open file"
        );
        // File still tracked (content-change, not removed).
        assert!(state.catalog_files.read().await.contains(&po));
    }

    #[tokio::test]
    async fn notify_event_rename_removes_old_path_and_adds_new_path() {
        // Rename shows up as Modify(Name) with [old, new] paths.
        // Old path removed (doesn't exist), new path added.
        let dir = TempDir::new().unwrap();
        let old_po = dir.path().join("de/LC_MESSAGES/old.po");
        let new_po = write_po(&dir, "de/LC_MESSAGES/new.po", b"");

        let state = Arc::new(WorkspaceState::new());
        *state.catalog_files.write().await = vec![old_po.clone()];

        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![old_po.clone(), new_po.clone()],
            attrs: Default::default(),
        };
        handle_notify_event(event, &state).await;

        let files = state.catalog_files.read().await;
        assert!(
            !files.contains(&old_po),
            "old path must be removed after rename"
        );
        assert!(
            files.contains(&new_po),
            "new path must be added after rename"
        );
    }

    #[tokio::test]
    async fn notify_event_ignored_for_open_file() {
        // REQ-CAT-10: notify Modify events for open files are dropped.
        let dir = TempDir::new().unwrap();
        let po = write_po(&dir, "de/LC_MESSAGES/messages.po", b"");

        let state = Arc::new(WorkspaceState::new());
        // File open in editor.
        state.documents.insert(
            make_uri(&po),
            DocumentState {
                rope: Rope::from_str(""),
                version: 1,
            },
        );

        let (tx, mut rx) = watch::channel(());
        state.set_rebuild_trigger(tx);

        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![po.clone()],
            attrs: Default::default(),
        };
        handle_notify_event(event, &state).await;

        // rebuild must NOT have been triggered — timeout means no send occurred.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), rx.changed())
                .await
                .is_err(),
            "rebuild should not be triggered for an open file"
        );
    }

    #[test]
    fn is_open_in_editor_true_when_document_present() {
        let dir = TempDir::new().unwrap();
        let po = dir.path().join("messages.po");
        std::fs::write(&po, b"").unwrap();

        let state = Arc::new(WorkspaceState::new());
        state.documents.insert(
            make_uri(&po),
            DocumentState {
                rope: Rope::from_str(""),
                version: 1,
            },
        );
        assert!(is_open_in_editor(&po, &state));
    }

    #[test]
    fn is_open_in_editor_false_when_no_document() {
        let dir = TempDir::new().unwrap();
        let po = dir.path().join("messages.po");
        let state = Arc::new(WorkspaceState::new());
        assert!(!is_open_in_editor(&po, &state));
    }

    // ── REQ-CPL-02 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_02_advertises_quote_trigger_characters() {
        let opts = completion_provider_options();
        let triggers = opts.trigger_characters.as_deref().unwrap_or_default();
        assert!(triggers.contains(&"\"".to_string()), "double-quote trigger missing");
        assert!(triggers.contains(&"'".to_string()), "single-quote trigger missing");
    }

    // ── REQ-CMD-03: is_config_uri ─────────────────────────────────────────────

    #[test]
    fn req_cmd_03_is_config_uri_detects_babel_cfg_and_pyproject() {
        let babel_cfg = Uri::from_file_path("/project/babel.cfg").unwrap();
        let pyproject = Uri::from_file_path("/project/pyproject.toml").unwrap();
        let unrelated = Uri::from_file_path("/project/setup.py").unwrap();
        assert!(is_config_uri(&babel_cfg));
        assert!(is_config_uri(&pyproject));
        assert!(!is_config_uri(&unrelated));
        // Catalog files are not config files.
        assert!(!is_config_uri(&Uri::from_file_path("/locale/de/messages.po").unwrap()));
    }
}
