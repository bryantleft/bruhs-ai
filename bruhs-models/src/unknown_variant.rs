//! The error every closed string enum's `FromStr` returns.

/// A string that is not one of an enum's wire values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} {value:?}")]
pub struct UnknownVariant {
    /// What was being parsed, e.g. `"permission mode"`.
    pub kind: &'static str,
    /// The rejected input.
    pub value: String,
}

impl UnknownVariant {
    pub(crate) fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}
