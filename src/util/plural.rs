/// Parse `nplurals=N` from a full PO header msgstr value.
///
/// The header msgstr contains `\n`-separated fields like:
/// `Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n != 1);\n`
pub fn parse_nplurals(header_msgstr: &str) -> Option<u32> {
    for line in header_msgstr.split('\n') {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Plural-Forms:") {
            for part in rest.split(';') {
                let part = part.trim();
                if let Some(n) = part.strip_prefix("nplurals=") {
                    return n.trim().parse().ok();
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nplurals_from_full_header() {
        let header =
            "Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n != 1);\n";
        assert_eq!(parse_nplurals(header), Some(2));
    }

    #[test]
    fn parses_nplurals_3() {
        let header = "Plural-Forms: nplurals=3; plural=...;\n";
        assert_eq!(parse_nplurals(header), Some(3));
    }

    #[test]
    fn returns_none_when_no_plural_forms() {
        let header = "Content-Type: text/plain; charset=UTF-8\n";
        assert_eq!(parse_nplurals(header), None);
    }

    #[test]
    fn returns_none_for_empty_string() {
        assert_eq!(parse_nplurals(""), None);
    }
}
