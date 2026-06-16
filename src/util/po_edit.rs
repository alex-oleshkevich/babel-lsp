use tower_lsp_server::ls_types::{Position, Range};

/// Positional span for one non-obsolete PO entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoEntrySpan {
    /// First line of this entry (flags line if present, else msgid). 0-based.
    pub entry_start_line: u32,
    /// The `#,` flags comment line, 0-based.
    pub flags_line: Option<u32>,
    /// The `msgid` line, 0-based.
    pub msgid_line: u32,
    /// The `msgid_plural` line, if present, 0-based.
    pub msgid_plural_line: Option<u32>,
    /// First `msgstr` / `msgstr[0]` line, 0-based.
    pub msgstr_start_line: u32,
    /// Last line of the msgstr block (including string continuations), 0-based.
    pub msgstr_end_line: u32,
    /// Number of `msgstr` keyword lines (1 for singular, N for plural).
    pub msgstr_count: usize,
}

/// Parse all non-obsolete PO entries in `content` into spans.
pub fn parse_entry_spans(content: &str) -> Vec<PoEntrySpan> {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut spans = Vec::new();
    let mut i = 0usize;

    while i < n {
        // Obsolete entries start with `#~` — skip them.
        if !lines[i].starts_with("msgid \"") {
            i += 1;
            continue;
        }

        let msgid_line = i as u32;
        let flags_line = find_flags_before(&lines, i);
        let entry_start_line = flags_line.unwrap_or(msgid_line);

        // Advance past msgid keyword and its continuation lines.
        i += 1;
        while i < n && lines[i].starts_with('"') {
            i += 1;
        }

        // Optional msgid_plural.
        let mut msgid_plural_line = None;
        if i < n && lines[i].starts_with("msgid_plural \"") {
            msgid_plural_line = Some(i as u32);
            i += 1;
            while i < n && lines[i].starts_with('"') {
                i += 1;
            }
        }

        // Require at least one msgstr keyword.
        if i >= n || !lines[i].starts_with("msgstr") {
            continue;
        }

        let msgstr_start_line = i as u32;
        let mut msgstr_end_line = i as u32;
        let mut msgstr_count = 0usize;

        while i < n && lines[i].starts_with("msgstr") {
            msgstr_count += 1;
            msgstr_end_line = i as u32;
            i += 1;
            while i < n && lines[i].starts_with('"') {
                msgstr_end_line = i as u32;
                i += 1;
            }
        }

        spans.push(PoEntrySpan {
            entry_start_line,
            flags_line,
            msgid_line,
            msgid_plural_line,
            msgstr_start_line,
            msgstr_end_line,
            msgstr_count,
        });
    }

    spans
}

/// Find the span whose entry bounds contain `line` (0-based), if any.
pub fn span_at_line(spans: &[PoEntrySpan], line: u32) -> Option<&PoEntrySpan> {
    spans.iter().find(|s| s.entry_start_line <= line && line <= s.msgstr_end_line)
}

/// LSP range covering the entire msgstr block (for replacement edits).
///
/// The range runs from column 0 of `msgstr_start_line` to end-of-content on
/// `msgstr_end_line`, so new text can be substituted in place and the trailing
/// newline is preserved.
pub fn msgstr_replace_range(span: &PoEntrySpan, lines: &[&str]) -> Range {
    let end_char = lines
        .get(span.msgstr_end_line as usize)
        .map(|l| l.len() as u32)
        .unwrap_or(0);
    Range {
        start: Position { line: span.msgstr_start_line, character: 0 },
        end: Position { line: span.msgstr_end_line, character: end_char },
    }
}

/// LSP range for the `#,` flags line (content only, no trailing newline).
pub fn flags_line_range(span: &PoEntrySpan, lines: &[&str]) -> Option<Range> {
    let fl = span.flags_line?;
    let end_char = lines.get(fl as usize).map(|l| l.len() as u32).unwrap_or(0);
    Some(Range {
        start: Position { line: fl, character: 0 },
        end: Position { line: fl, character: end_char },
    })
}

/// Escape a raw string value for use inside PO `"..."` delimiters.
pub fn escape_po(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Scan backwards from `msgid_idx` to find the nearest `#,` flags line.
///
/// Stops at blank lines, non-comment lines, or the start of the file.
/// Other comment types (`# `, `#.`, `#:`, `#|`) are skipped over.
fn find_flags_before(lines: &[&str], msgid_idx: usize) -> Option<u32> {
    let mut j = msgid_idx.checked_sub(1)?;
    loop {
        let line = lines[j];
        if line.trim().is_empty() {
            return None;
        }
        if line.starts_with("#,") {
            return Some(j as u32);
        }
        if line.starts_with('#') {
            if j == 0 { return None; }
            j -= 1;
        } else {
            return None;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_entry_spans ────────────────────────────────────────────────────

    #[test]
    fn simple_singular_entry() {
        let content = "msgid \"Hello\"\nmsgstr \"Hallo\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.msgid_line, 0);
        assert_eq!(s.msgstr_start_line, 1);
        assert_eq!(s.msgstr_end_line, 1);
        assert_eq!(s.msgstr_count, 1);
        assert!(s.flags_line.is_none());
        assert!(s.msgid_plural_line.is_none());
    }

    #[test]
    fn entry_with_flags() {
        let content = "#, fuzzy\nmsgid \"Save\"\nmsgstr \"\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.flags_line, Some(0));
        assert_eq!(s.msgid_line, 1);
        assert_eq!(s.entry_start_line, 0);
    }

    #[test]
    fn entry_with_comments_before_flags() {
        let content = "# Translator comment\n#. Extracted\n#: file.py:10\n#, python-format\nmsgid \"%(n)s item\"\nmsgstr \"\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.flags_line, Some(3));
        assert_eq!(s.msgid_line, 4);
    }

    #[test]
    fn plural_entry() {
        let content = "msgid \"%(n)d item\"\nmsgid_plural \"%(n)d items\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.msgid_line, 0);
        assert_eq!(s.msgid_plural_line, Some(1));
        assert_eq!(s.msgstr_start_line, 2);
        assert_eq!(s.msgstr_end_line, 3);
        assert_eq!(s.msgstr_count, 2);
    }

    #[test]
    fn multiline_msgid_and_msgstr() {
        let content = "msgid \"\"\n\"Part 1 \"\n\"Part 2\"\nmsgstr \"\"\n\"Übersetzung\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.msgid_line, 0);
        assert_eq!(s.msgstr_start_line, 3);
        assert_eq!(s.msgstr_end_line, 4);
    }

    #[test]
    fn two_entries_separated_by_blank() {
        let content = "msgid \"A\"\nmsgstr \"X\"\n\nmsgid \"B\"\nmsgstr \"Y\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].msgid_line, 0);
        assert_eq!(spans[1].msgid_line, 3);
    }

    #[test]
    fn obsolete_entries_are_skipped() {
        let content = "#~ msgid \"Old\"\n#~ msgstr \"Alt\"\n\nmsgid \"New\"\nmsgstr \"Neu\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].msgid_line, 3);
    }

    #[test]
    fn blank_between_comment_and_msgid_breaks_flags() {
        // A blank line between #, fuzzy and msgid means no flags.
        let content = "#, fuzzy\n\nmsgid \"A\"\nmsgstr \"\"\n";
        let spans = parse_entry_spans(content);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].flags_line.is_none());
    }

    // ── span_at_line ─────────────────────────────────────────────────────────

    #[test]
    fn span_at_line_finds_span_at_msgid() {
        let content = "msgid \"A\"\nmsgstr \"X\"\n";
        let spans = parse_entry_spans(content);
        let s = span_at_line(&spans, 0);
        assert!(s.is_some());
    }

    #[test]
    fn span_at_line_finds_span_at_msgstr() {
        let content = "msgid \"A\"\nmsgstr \"X\"\n";
        let spans = parse_entry_spans(content);
        assert!(span_at_line(&spans, 1).is_some());
    }

    #[test]
    fn span_at_line_finds_span_at_flags() {
        let content = "#, fuzzy\nmsgid \"A\"\nmsgstr \"\"\n";
        let spans = parse_entry_spans(content);
        assert!(span_at_line(&spans, 0).is_some());
    }

    #[test]
    fn span_at_line_returns_none_for_blank_between_entries() {
        let content = "msgid \"A\"\nmsgstr \"X\"\n\nmsgid \"B\"\nmsgstr \"Y\"\n";
        let spans = parse_entry_spans(content);
        assert!(span_at_line(&spans, 2).is_none()); // blank line
    }

    // ── escape_po ────────────────────────────────────────────────────────────

    #[test]
    fn escape_po_plain_ascii() {
        assert_eq!(escape_po("Hello"), "Hello");
    }

    #[test]
    fn escape_po_backslash_and_quote() {
        assert_eq!(escape_po("a\\b\"c"), "a\\\\b\\\"c");
    }

    #[test]
    fn escape_po_newline_and_tab() {
        assert_eq!(escape_po("a\nb\tc"), "a\\nb\\tc");
    }

    // ── msgstr_replace_range ─────────────────────────────────────────────────

    #[test]
    fn msgstr_replace_range_singular() {
        let content = "msgid \"A\"\nmsgstr \"\"\n";
        let lines: Vec<&str> = content.lines().collect();
        let spans = parse_entry_spans(content);
        let r = msgstr_replace_range(&spans[0], &lines);
        assert_eq!(r.start.line, 1);
        assert_eq!(r.start.character, 0);
        assert_eq!(r.end.line, 1);
        assert_eq!(r.end.character, "msgstr \"\"".len() as u32);
    }
}
