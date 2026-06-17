use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::*;

/// REQ-NAV-07/08: parse `#:` reference comments in a `.po`/`.pot` buffer and
/// return one `DocumentLink` per `path:line` token.
///
/// Paths are resolved relative to `catalog_dir`, with `workspace_root` as a
/// fallback.  The server does not pre-check whether the target file exists
/// (REQ-NAV-09 — that is the editor's concern).
pub fn document_links(
    content: &str,
    catalog_dir: &Path,
    workspace_root: Option<&Path>,
) -> Vec<DocumentLink> {
    let mut links = Vec::new();

    for (lineno, raw_line) in content.lines().enumerate() {
        let rest = match raw_line.trim_start().strip_prefix("#:") {
            Some(r) => r,
            None => continue,
        };

        // Byte offset of the `#:` on this line (needed to compute column).
        let line_prefix_bytes = raw_line.find("#:").unwrap_or(0);
        // Start of the token list: skip `#:` then optional space.
        let tokens_start = line_prefix_bytes + 2; // skip `#:`

        parse_reference_line(
            rest,
            lineno as u32,
            tokens_start,
            catalog_dir,
            workspace_root,
            &mut links,
        );
    }

    links
}

fn parse_reference_line(
    rest: &str,              // text after the `#:` on the line
    line_idx: u32,           // 0-based line index in the document
    tokens_col_start: usize, // byte column where token area starts
    catalog_dir: &Path,
    workspace_root: Option<&Path>,
    links: &mut Vec<DocumentLink>,
) {
    let mut col = tokens_col_start;

    // Skip a leading space after `#:` if present.
    let mut chars = rest.chars().peekable();
    if chars.peek() == Some(&' ') {
        chars.next();
        col += 1;
    }

    // Rebuild the remaining text for splitting.
    let remaining: String = chars.collect();
    let mut char_offset = col;

    for token in remaining.split_ascii_whitespace() {
        let token_start = char_offset;
        // Use char count (not byte length) for correct UTF-16 column positions.
        let token_char_len = token.chars().count();
        let token_end = token_start + token_char_len;
        char_offset = token_end + 1; // +1 for the space separator

        // REQ-NAV-08: split on the last colon to separate path and line.
        let colon_pos = match token.rfind(':') {
            Some(p) => p,
            None => continue, // no colon → skip (REQ-NAV-08)
        };
        let path_part = &token[..colon_pos];
        let line_part = &token[colon_pos + 1..];

        // Skip tokens without a numeric line (REQ-NAV-08).
        let _target_line: u32 = match line_part.parse::<u32>() {
            Ok(n) if n > 0 => n - 1, // 1-based → 0-based
            _ => continue,
        };

        let resolved = match resolve_path(path_part, catalog_dir, workspace_root) {
            Some(p) => p,
            None => continue, // file does not exist — skip link
        };
        let target_uri = match Uri::from_file_path(&resolved) {
            Some(u) => u,
            None => continue,
        };

        let range = Range {
            start: Position {
                line: line_idx,
                character: token_start as u32,
            },
            end: Position {
                line: line_idx,
                character: token_end as u32,
            },
        };

        links.push(DocumentLink {
            range,
            target: Some(target_uri),
            tooltip: None,
            data: None,
        });
    }
}

/// Resolve a relative path: try catalog_dir first, workspace_root as fallback.
///
/// Returns `None` when no existing file is found for `path_str`, so callers
/// do not emit links to non-existent files (REQ-NAV-09).
fn resolve_path(
    path_str: &str,
    catalog_dir: &Path,
    workspace_root: Option<&Path>,
) -> Option<PathBuf> {
    let candidate = catalog_dir.join(path_str);
    if candidate.exists() {
        return Some(candidate);
    }
    if let Some(root) = workspace_root {
        let root_candidate = root.join(path_str);
        if root_candidate.exists() {
            return Some(root_candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Create a temp directory with the given files (relative paths).  Returns
    /// the TempDir so it stays alive for the duration of the test.
    fn make_tree(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for rel in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "").unwrap();
        }
        dir
    }

    fn links_for_tree(content: &str, catalog_dir: &Path, workspace: &Path) -> Vec<DocumentLink> {
        document_links(content, catalog_dir, Some(workspace))
    }

    // ── REQ-NAV-07 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_07_hash_colon_comments_become_links() {
        let tree = make_tree(&["app/views.py"]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "#: app/views.py:42\nmsgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert_eq!(links.len(), 1);
        assert!(
            links[0]
                .target
                .as_ref()
                .unwrap()
                .to_string()
                .contains("views.py"),
            "target should point to views.py"
        );
        assert_eq!(links[0].range.start.line, 0);
    }

    #[test]
    fn req_nav_07_multiple_tokens_on_one_line() {
        let tree = make_tree(&["app/views.py", "app/templates/checkout.html"]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "#: app/views.py:42 app/templates/checkout.html:8\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert_eq!(links.len(), 2);
        assert!(
            links[0]
                .target
                .as_ref()
                .unwrap()
                .to_string()
                .contains("views.py")
        );
        assert!(
            links[1]
                .target
                .as_ref()
                .unwrap()
                .to_string()
                .contains("checkout.html")
        );
    }

    // ── REQ-NAV-08 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_08_parses_last_colon_split_and_converts_line() {
        let tree = make_tree(&["app/views.py"]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "#: app/views.py:42\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert_eq!(links.len(), 1);
        // The link range starts right after `#: ` at column 3.
        assert_eq!(links[0].range.start.character, 3);
        // token is `app/views.py:42` (15 chars), end = 3 + 15 = 18.
        assert_eq!(
            links[0].range.end.character,
            3 + "app/views.py:42".chars().count() as u32
        );
    }

    #[test]
    fn req_nav_08_skips_token_without_numeric_line() {
        let tree = make_tree(&["app/other.py"]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        // "app/views.py" has no colon → skipped; "app/other.py:10" is included.
        let content = "#: app/views.py app/other.py:10\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert_eq!(links.len(), 1);
        assert!(
            links[0]
                .target
                .as_ref()
                .unwrap()
                .to_string()
                .contains("other.py")
        );
    }

    #[test]
    fn req_nav_08_skips_non_reference_lines() {
        let tree = make_tree(&["app/views.py"]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "# translator comment\nmsgid \"Checkout\"\n#: app/views.py:1\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].range.start.line, 2);
    }

    // ── REQ-NAV-09 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_09_source_to_catalog_link_is_optional() {
        let tree = make_tree(&[]);
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "msgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert!(links.is_empty());
    }

    #[test]
    fn non_existent_path_produces_no_link() {
        // When the referenced source file does not exist on disk, no link is emitted.
        let tree = make_tree(&[]); // no files created
        let workspace = tree.path();
        let catalog_dir = workspace.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        let content = "#: app/views.py:42\n";
        let links = links_for_tree(content, &catalog_dir, workspace);
        assert!(
            links.is_empty(),
            "non-existent referenced file should produce no link"
        );
    }
}
