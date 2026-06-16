use ropey::{Rope, RopeSlice};
use tower_lsp_server::ls_types::{Position, Range};

#[derive(Clone, Copy)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// Returns true when `pos` is within `range` (inclusive start, exclusive end).
pub fn pos_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character >= range.end.character {
        return false;
    }
    true
}

pub fn char_offset_to_lsp_pos(rope: &Rope, char_offset: usize, enc: PositionEncoding) -> Position {
    let char_offset = char_offset.min(rope.len_chars());
    let line = rope.char_to_line(char_offset);
    let line_start = rope.line_to_char(line);
    let col_chars = char_offset - line_start;
    let line_slice = rope.line(line);
    let character: usize = match enc {
        PositionEncoding::Utf8 => line_slice.chars().take(col_chars).map(|c| c.len_utf8()).sum(),
        PositionEncoding::Utf16 => line_slice.chars().take(col_chars).map(|c| c.len_utf16()).sum(),
    };
    Position {
        line: line as u32,
        character: character as u32,
    }
}

pub fn lsp_pos_to_char_offset(rope: &Rope, pos: Position, enc: PositionEncoding) -> usize {
    let line = (pos.line as usize).min(rope.len_lines().saturating_sub(1));
    let character = pos.character as usize;
    let line_start = rope.line_to_char(line);
    let line_slice = rope.line(line);
    let col = match enc {
        PositionEncoding::Utf8 => line_slice.byte_to_char(character.min(line_slice.len_bytes())),
        PositionEncoding::Utf16 => utf16_col_to_char(line_slice, character),
    };
    line_start + col
}

fn utf16_col_to_char(slice: RopeSlice, utf16_col: usize) -> usize {
    let mut count = 0usize;
    for (i, ch) in slice.chars().enumerate() {
        if count >= utf16_col {
            return i;
        }
        count += ch.len_utf16();
    }
    slice.len_chars()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn utf16_ascii_offset() {
        let r = rope("hello\nworld\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(1, 3), PositionEncoding::Utf16),
            9
        );
    }

    #[test]
    fn utf8_ascii_offset() {
        let r = rope("hello\nworld\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(1, 3), PositionEncoding::Utf8),
            9
        );
    }

    #[test]
    fn utf16_multibyte_bmp() {
        // 'é' (U+00E9) is 1 UTF-16 unit; col 3 lands on it, col 4 is past it
        let r = rope("café\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(0, 4), PositionEncoding::Utf16),
            4
        );
    }

    #[test]
    fn utf8_multibyte_bmp() {
        // 'é' (U+00E9) is 2 UTF-8 bytes; byte 5 is past it
        let r = rope("café\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(0, 5), PositionEncoding::Utf8),
            4
        );
    }

    #[test]
    fn utf16_surrogate_pair() {
        // 😀 (U+1F600) occupies 2 UTF-16 code units; col 3 is 'b'
        let r = rope("a😀b\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(0, 3), PositionEncoding::Utf16),
            2
        );
    }

    #[test]
    fn utf16_col_past_end_clamps() {
        let r = rope("hi\n");
        assert_eq!(
            lsp_pos_to_char_offset(&r, pos(0, 99), PositionEncoding::Utf16),
            3
        );
    }

    #[test]
    fn line_past_end_clamps() {
        let r = rope("hi\n");
        // line 99 doesn't exist; should clamp without panicking
        let _ = lsp_pos_to_char_offset(&r, pos(99, 0), PositionEncoding::Utf16);
    }

    #[test]
    fn utf8_byte_past_end_clamps() {
        let r = rope("hi\n");
        // byte 99 past line end; should clamp without panicking
        let _ = lsp_pos_to_char_offset(&r, pos(0, 99), PositionEncoding::Utf8);
    }
}
