"""E2E lifecycle tests — REQ-ARCH-01, REQ-ARCH-09, REQ-ARCH-10, REQ-ARCH-11."""
from lsprotocol import types
from pytest_lsp import LanguageClient


async def test_initialize_incremental_sync(client: LanguageClient):
    sync = client.server_capabilities.text_document_sync
    if isinstance(sync, types.TextDocumentSyncOptions):
        assert sync.change == types.TextDocumentSyncKind.Incremental
    else:
        assert sync == types.TextDocumentSyncKind.Incremental


async def test_initialize_utf8_encoding(client: LanguageClient):
    assert client.server_capabilities.position_encoding == types.PositionEncodingKind.Utf8


async def test_did_open_triggers_publish_diagnostics(
    shopfront_client: LanguageClient,
    shopfront,
):
    """REQ-ARCH-10: every opened file receives a publishDiagnostics notification."""
    views = shopfront / "app" / "views.py"
    shopfront_client.text_document_did_open(
        types.DidOpenTextDocumentParams(
            text_document=types.TextDocumentItem(
                uri=views.as_uri(),
                language_id="python",
                version=1,
                text=views.read_text(),
            )
        )
    )
    await shopfront_client.wait_for_notification("textDocument/publishDiagnostics")
    assert views.as_uri() in shopfront_client.diagnostics


async def test_did_open_non_ascii_file(
    shopfront_client: LanguageClient,
    shopfront,
):
    """Server handles files with non-ASCII content without crashing (E17)."""
    views = shopfront / "app" / "views.py"
    non_ascii_text = views.read_text() + '\n_("日本語テスト")\n'
    shopfront_client.text_document_did_open(
        types.DidOpenTextDocumentParams(
            text_document=types.TextDocumentItem(
                uri=views.as_uri(),
                language_id="python",
                version=1,
                text=non_ascii_text,
            )
        )
    )
    await shopfront_client.wait_for_notification("textDocument/publishDiagnostics")
    assert views.as_uri() in shopfront_client.diagnostics
