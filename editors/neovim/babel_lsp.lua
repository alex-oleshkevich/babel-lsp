-- Add to your init.lua (requires Neovim 0.11+ with built-in lsp config)
-- or place in after/plugin/babel_lsp.lua

vim.lsp.config('babel_lsp', {
  cmd = { 'babel-lsp', 'lsp', '--stdio' },
  filetypes = { 'python', 'jinja', 'htmldjango', 'html', 'po' },
  root_markers = { 'pyproject.toml', '.git' },
})
vim.lsp.enable('babel_lsp')
