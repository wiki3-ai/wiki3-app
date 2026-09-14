//! Small, dependency-free helpers shared across modules.

/// Collapse a user-supplied string to `None` when it carries no information.
///
/// A cleared text field arrives as `Some("")`, and a field the user typed a
/// space into arrives as `Some("   ")`. To a human those both mean "not set",
/// so reducing them to `None` at the edge keeps "absent" a single concept
/// instead of a family of near-misses that each behave slightly differently.
///
/// The value itself is returned untouched — trimming happens only for the
/// emptiness test, so meaningful surrounding whitespace survives.
pub fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::non_blank;

    #[test]
    fn non_blank_collapses_absent_empty_and_whitespace() {
        assert_eq!(non_blank(None), None);
        assert_eq!(non_blank(Some(String::new())), None);
        assert_eq!(non_blank(Some("   ".to_string())), None);
        assert_eq!(non_blank(Some("\t\n  ".to_string())), None);
    }

    #[test]
    fn non_blank_preserves_the_original_spelling() {
        assert_eq!(
            non_blank(Some("  padded  ".to_string())),
            Some("  padded  ".to_string())
        );
        assert_eq!(non_blank(Some("x".to_string())), Some("x".to_string()));
    }
}
