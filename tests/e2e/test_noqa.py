"""E2E tests for # noqa inline suppression."""
from lsprotocol import types
from pytest_lsp import LanguageClient


async def test_noqa_bare_suppresses_all_diagnostics(
    shopfront_client: LanguageClient,
    shopfront,
):
    """Bare `# noqa` on a line suppresses all diagnostics for that line."""
    src = shopfront / "app" / "views.py"
    text = 'from flask_babel import _\n\n_("UnknownMsgId")  # noqa\n'
    shopfront_client.text_document_did_open(
        types.DidOpenTextDocumentParams(
            text_document=types.TextDocumentItem(
                uri=src.as_uri(),
                language_id="python",
                version=1,
                text=text,
            )
        )
    )
    await shopfront_client.wait_for_notification("textDocument/publishDiagnostics")
    diags = shopfront_client.diagnostics.get(src.as_uri(), [])
    codes = [
        d.code for d in diags if d.range.start.line == 2
    ]
    assert "msg/unknown-id" not in codes, (
        f"msg/unknown-id should be suppressed by bare # noqa, got: {codes}"
    )


async def test_noqa_specific_code_suppresses_matching_diagnostic(
    shopfront_client: LanguageClient,
    shopfront,
):
    """`# noqa: msg/unknown-id` suppresses only that code."""
    src = shopfront / "app" / "views.py"
    text = 'from flask_babel import _\n\n_("UnknownMsgId")  # noqa: msg/unknown-id\n'
    shopfront_client.text_document_did_open(
        types.DidOpenTextDocumentParams(
            text_document=types.TextDocumentItem(
                uri=src.as_uri(),
                language_id="python",
                version=1,
                text=text,
            )
        )
    )
    await shopfront_client.wait_for_notification("textDocument/publishDiagnostics")
    diags = shopfront_client.diagnostics.get(src.as_uri(), [])
    codes = [
        d.code for d in diags if d.range.start.line == 2
    ]
    assert "msg/unknown-id" not in codes, (
        f"msg/unknown-id should be suppressed by # noqa: msg/unknown-id, got: {codes}"
    )


async def test_noqa_specific_code_does_not_suppress_other_diagnostics(
    shopfront_client: LanguageClient,
    shopfront,
):
    """`# noqa: msg/fstring-in-call` must not suppress msg/unknown-id."""
    src = shopfront / "app" / "views.py"
    text = 'from flask_babel import _\n\n_("UnknownMsgId")  # noqa: msg/fstring-in-call\n'
    shopfront_client.text_document_did_open(
        types.DidOpenTextDocumentParams(
            text_document=types.TextDocumentItem(
                uri=src.as_uri(),
                language_id="python",
                version=1,
                text=text,
            )
        )
    )
    await shopfront_client.wait_for_notification("textDocument/publishDiagnostics")
    diags = shopfront_client.diagnostics.get(src.as_uri(), [])
    codes = [
        d.code for d in diags if d.range.start.line == 2
    ]
    assert "msg/unknown-id" in codes, (
        f"msg/unknown-id should NOT be suppressed by # noqa: msg/fstring-in-call, got: {codes}"
    )
