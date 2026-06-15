use std::path::PathBuf;

// ── CatalogKey ────────────────────────────────────────────────────────────────

/// The lookup key for every catalog feature: msgid + optional msgctxt.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CatalogKey {
    pub msgid: String,
    pub msgctxt: Option<String>,
}

#[allow(dead_code)]
impl CatalogKey {
    pub fn new(msgid: impl Into<String>) -> Self {
        Self { msgid: msgid.into(), msgctxt: None }
    }

    pub fn with_ctx(msgid: impl Into<String>, msgctxt: impl Into<String>) -> Self {
        Self { msgid: msgid.into(), msgctxt: Some(msgctxt.into()) }
    }
}

// ── CatalogEntry ──────────────────────────────────────────────────────────────

/// One parsed message from a `.po` or `.pot` catalog.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EntryFlags {
    pub fuzzy: bool,
    pub obsolete: bool,
}

/// One parsed catalog entry, carrying everything features need.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub locale: String,
    pub domain: String,
    pub msgid: String,
    pub msgctxt: Option<String>,
    pub msgid_plural: Option<String>,
    pub msgstr: Vec<String>,
    pub flags: EntryFlags,
    pub file_path: PathBuf,
    pub line: u32,
}

#[allow(dead_code)]
impl CatalogEntry {
    pub fn key(&self) -> CatalogKey {
        CatalogKey {
            msgid: self.msgid.clone(),
            msgctxt: self.msgctxt.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_key_equality() {
        let a = CatalogKey::new("Checkout");
        let b = CatalogKey::new("Checkout");
        assert_eq!(a, b);
    }

    #[test]
    fn catalog_key_distinguishes_context() {
        let a = CatalogKey::with_ctx("Save", "button");
        let b = CatalogKey::with_ctx("Save", "menu");
        assert_ne!(a, b);
    }

    #[test]
    fn catalog_key_no_context_ne_with_context() {
        let a = CatalogKey::new("Save");
        let b = CatalogKey::with_ctx("Save", "button");
        assert_ne!(a, b);
    }

    #[test]
    fn entry_key_round_trips() {
        let entry = CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: "Checkout".into(),
            msgctxt: Some("ecommerce".into()),
            msgid_plural: None,
            msgstr: vec!["Kasse".into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: "/locale/de/LC_MESSAGES/messages.po".into(),
            line: 10,
        };
        let key = entry.key();
        assert_eq!(key.msgid, "Checkout");
        assert_eq!(key.msgctxt.as_deref(), Some("ecommerce"));
    }
}
