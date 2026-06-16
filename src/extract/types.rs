use std::collections::HashMap;

use tower_lsp_server::ls_types::Range;

/// Why a `TranslationCall`'s first argument could not be resolved to a string literal.
///
/// Used by the diagnostic "shape trio" (REQ-DIAG-06).  Only one reason fires per
/// call — the most specific that matches.
#[derive(Clone, Debug, PartialEq)]
pub enum UnresolvedReason {
    /// The argument is an f-string (`f"…"`).  gettext looks up the *interpolated*
    /// text, never the template, so extraction always misses these.
    FString,
    /// The argument is a `str.format(…)` call or a `%` formatting expression —
    /// formatting runs before the catalog lookup.
    FormatBeforeCall,
    /// The argument is a name, attribute, function call, or other non-literal.
    /// `pybabel extract` cannot read it statically.
    NonConstant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TranslationFunc {
    Gettext,
    NGettext,
    PGettext,
    NPGettext,
    DGettext,
    DNGettext,
    DPGettext,
    DNPGettext,
}

impl TranslationFunc {
    pub fn has_domain(&self) -> bool {
        matches!(
            self,
            Self::DGettext | Self::DNGettext | Self::DPGettext | Self::DNPGettext
        )
    }

    pub fn has_context(&self) -> bool {
        matches!(
            self,
            Self::PGettext | Self::NPGettext | Self::DPGettext | Self::DNPGettext
        )
    }

    pub fn has_plural(&self) -> bool {
        matches!(
            self,
            Self::NGettext | Self::NPGettext | Self::DNGettext | Self::DNPGettext
        )
    }

    /// Map a well-known callee name to its variant.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "_" | "gettext" | "gettext_lazy" | "ugettext" | "ugettext_lazy" => Some(Self::Gettext),
            "ngettext" | "ngettext_lazy" | "ungettext" | "ungettext_lazy" => Some(Self::NGettext),
            "pgettext" | "pgettext_lazy" => Some(Self::PGettext),
            "npgettext" => Some(Self::NPGettext),
            "dgettext" => Some(Self::DGettext),
            "dngettext" => Some(Self::DNGettext),
            "dpgettext" => Some(Self::DPGettext),
            "dnpgettext" => Some(Self::DNPGettext),
            _ => None,
        }
    }

    /// Resolve a name against built-ins first, then the extra_keywords table.
    pub fn resolve(name: &str, extra: &HashMap<String, TranslationFunc>) -> Option<Self> {
        Self::from_name(name).or_else(|| extra.get(name).cloned())
    }
}

#[derive(Clone, Debug)]
pub struct TranslationCall {
    pub func: TranslationFunc,
    /// `None` when the first argument is not a resolvable string literal.
    pub msgid: Option<String>,
    pub msgid_plural: Option<String>,
    pub msgctxt: Option<String>,
    pub domain: Option<String>,
    /// The whole call expression.
    pub range: Range,
    /// The msgid string literal alone (anchor for hover/goto/rename).
    pub msgid_range: Option<Range>,
    /// Why the msgid argument could not be resolved (shape trio, REQ-DIAG-06).
    /// `None` when `msgid` is `Some` or when the call has no argument at all.
    pub unresolved_reason: Option<UnresolvedReason>,
    /// Source range of the unresolved first argument (for diagnostic squiggles).
    /// `None` when `unresolved_reason` is `None`.
    pub unresolved_arg_range: Option<Range>,
    /// `true` when the msgid was resolved from an implicit string concatenation
    /// (`"a" "b"`) — triggers `msg/implicit-concat`.
    pub is_implicit_concat: bool,
}
