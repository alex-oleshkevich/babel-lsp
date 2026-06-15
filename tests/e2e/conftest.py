import os
import pathlib
import shutil

import pytest
import pytest_lsp
from lsprotocol import types
from pytest_lsp import ClientServerConfig, LanguageClient

_ROOT = pathlib.Path(__file__).parent.parent.parent
_FIXTURES = pathlib.Path(__file__).parent / "fixtures"
_DEFAULT_BIN = str(_ROOT / "target" / "debug" / "babel-lsp")
SERVER_CMD = [os.environ.get("BABEL_LSP_BIN", _DEFAULT_BIN), "lsp"]


def _capabilities() -> types.ClientCapabilities:
    return types.ClientCapabilities(
        general=types.GeneralClientCapabilities(
            position_encodings=[types.PositionEncodingKind.Utf8],
        ),
        workspace=types.WorkspaceClientCapabilities(
            workspace_folders=True,
        ),
    )


@pytest.fixture
def shopfront(tmp_path):
    dst = tmp_path / "shopfront"
    shutil.copytree(_FIXTURES / "clean_shopfront", dst)
    return dst


@pytest_lsp.fixture(config=ClientServerConfig(server_command=SERVER_CMD))
async def client(lsp_client: LanguageClient):
    init = await lsp_client.initialize_session(
        types.InitializeParams(capabilities=_capabilities())
    )
    lsp_client.server_capabilities = init.capabilities
    yield
    await lsp_client.shutdown_session()


@pytest_lsp.fixture(config=ClientServerConfig(server_command=SERVER_CMD))
async def shopfront_client(lsp_client: LanguageClient, shopfront):
    init = await lsp_client.initialize_session(
        types.InitializeParams(
            capabilities=_capabilities(),
            workspace_folders=[
                types.WorkspaceFolder(uri=shopfront.as_uri(), name="shopfront"),
            ],
        )
    )
    lsp_client.server_capabilities = init.capabilities
    yield
    await lsp_client.shutdown_session()
