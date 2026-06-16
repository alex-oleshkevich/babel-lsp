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
        let line_prefix_bytes = raw_line
            .find("#:")
            .unwrap_or(0);
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
    rest: &str,           // text after the `#:` on the line
    line_idx: u32,        // 0-based line index in the document
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
        let token_end = token_start + token.len();
        char_offset = token_end + 1; // +1 for the space separator

        // REQ-NAV-08: split on the last colon to separate path and line.
        let colon_pos = match token.rfind(':') {
            Some(p) => p,
            None => continue, // no colon → skip (REQ-NAV-08)
        };
        let path_part = &token[..colon_pos];
        let line_part = &token[colon_pos + 1..];

        // Skip tokens without a numeric line (REQ-NAV-08).
        let target_line: u32 = match line_part.parse::<u32>() {
            Ok(n) if n > 0 => n - 1, // 1-based → 0-based
            _ => continue,
        };

        let resolved = resolve_path(path_part, catalog_dir, workspace_root);
        let target_uri = match Uri::from_file_path(&resolved) {
            Some(u) => u,
            None => continue,
        };

        let range = Range {
            start: Position { line: line_idx, character: token_start as u32 },
            end: Position { line: line_idx, character: token_end as u32 },
        };

        let mut target = target_uri.to_string();
        // Append the 0-based line as a URI fragment so editors can jump to it.
        target.push_str(&format!("#L{}", target_line + 1));

        // `DocumentLink.target` is a Uri, not a String — build the jump target
        // as a plain URI without a fragment (fragments are not standard in
        // LSP document link targets; line navigation is via LocationLink).
        links.push(DocumentLink {
            range,
            target: Some(target_uri),
            tooltip: None,
            data: None,
        });
    }
}

/// Resolve a relative path: try catalog_dir first, workspace_root as fallback.
fn resolve_path(path_str: &str, catalog_dir: &Path, workspace_root: Option<&Path>) -> PathBuf {
    let candidate = catalog_dir.join(path_str);
    if candidate.exists() {
        return candidate;
    }
    if let Some(root) = workspace_root {
        let root_candidate = root.join(path_str);
        if root_candidate.exists() {
            return root_candidate;
        }
        return root_candidate;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn catalog_dir() -> PathBuf {
        PathBuf::from("/locale/de/LC_MESSAGES")
    }

    fn links_for(content: &str) -> Vec<DocumentLink> {
        document_links(content, &catalog_dir(), Some(Path::new("/workspace")))
    }

    // ── REQ-NAV-07 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_07_hash_colon_comments_become_links() {
        let content = "#: app/views.py:42\nmsgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let links = links_for(content);
        assert_eq!(links.len(), 1);
        assert!(
            links[0].target.as_ref().unwrap().to_string().contains("views.py"),
            "target should point to views.py"
        );
        // range covers the token on line 0
        assert_eq!(links[0].range.start.line, 0);
    }

    #[test]
    fn req_nav_07_multiple_tokens_on_one_line() {
        let content = "#: app/views.py:42 app/templates/checkout.html:8\n";
        let links = links_for(content);
        assert_eq!(links.len(), 2);
        assert!(links[0].target.as_ref().unwrap().to_string().contains("views.py"));
        assert!(links[1].target.as_ref().unwrap().to_string().contains("checkout.html"));
    }

    // ── REQ-NAV-08 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_08_parses_last_colon_split_and_converts_line() {
        // 1-based line 42 → 0-based 41 for the jump position.
        let content = "#: app/views.py:42\n";
        let links = links_for(content);
        assert_eq!(links.len(), 1);
        // The link range starts right after `#: ` at column 3.
        assert_eq!(links[0].range.start.character, 3);
        // token is `app/views.py:42` (16 chars), end = 3 + 16 = 19.
        assert_eq!(links[0].range.end.character, 3 + "app/views.py:42".len() as u32);
    }

    #[test]
    fn req_nav_08_skips_token_without_numeric_line() {
        let content = "#: app/views.py app/other.py:10\n";
        let links = links_for(content);
        // First token has no colon → skipped; second has :10 → included.
        assert_eq!(links.len(), 1);
        assert!(links[0].target.as_ref().unwrap().to_string().contains("other.py"));
    }

    #[test]
    fn req_nav_08_skips_non_reference_lines() {
        let content = "# translator comment\nmsgid \"Checkout\"\n#: app/views.py:1\n";
        let links = links_for(content);
        // Only the `#:` line produces links; `#` alone does not.
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].range.start.line, 2);
    }

    // ── REQ-NAV-09 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_09_source_to_catalog_link_is_optional() {
        // document_links only covers catalog→source direction.
        // A .po file with no `#:` comments returns empty — that's fine.
        let content = "msgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let links = links_for(content);
        assert!(links.is_empty());
    }
}
