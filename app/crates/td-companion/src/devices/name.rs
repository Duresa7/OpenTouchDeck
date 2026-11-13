//! The rule for a Device name, with the words each screen uses for a broken one. The Device and
//! `td_link::identity::valid_name` hold the same rule; this says which part a name breaks.
use td_link::net::wire::name_char_allowed;

/// The most UTF-8 bytes a Device keeps in its name.
pub const NAME_BYTES: usize = 48;

/// Why a Device would refuse a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameProblem {
    /// Empty, or only spaces.
    Empty,
    /// More than [`NAME_BYTES`] bytes.
    TooLong,
    /// A control, format or filler character: invisible, so the name could pass for another.
    Special,
}

/// What, if anything, is wrong with `name`.
pub fn name_problem(name: &str) -> Option<NameProblem> {
    if !name.chars().any(|c| !c.is_whitespace()) {
        Some(NameProblem::Empty)
    } else if name.len() > NAME_BYTES {
        Some(NameProblem::TooLong)
    } else if !name.chars().all(name_char_allowed) {
        Some(NameProblem::Special)
    } else {
        None
    }
}

const SPECIAL: &str = "Remove tabs, line breaks and other special characters.";

impl NameProblem {
    /// In Settings, where the Device already has a name.
    pub fn settings_text(self) -> &'static str {
        match self {
            Self::Empty => "Give the Device a name.",
            Self::TooLong => "That name is too long for the Device.",
            Self::Special => SPECIAL,
        }
    }
    /// In setup, where the person names it for the first time.
    pub fn setup_text(self) -> String {
        match self {
            Self::Empty => "Enter a name.".into(),
            Self::TooLong => format!("Names can be up to {NAME_BYTES} characters."),
            Self::Special => SPECIAL.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_break_the_rule_the_way_the_device_does() {
        assert_eq!(name_problem(""), Some(NameProblem::Empty));
        assert_eq!(name_problem("   "), Some(NameProblem::Empty));
        assert_eq!(name_problem(&"a".repeat(48)), None);
        assert_eq!(name_problem(&"a".repeat(49)), Some(NameProblem::TooLong));
        // 16 three-byte characters are 48 bytes; one more is too many.
        assert_eq!(name_problem(&"€".repeat(16)), None);
        assert_eq!(name_problem(&"€".repeat(17)), Some(NameProblem::TooLong));
        for hidden in [
            "tab\there",
            "zero\u{200b}width",
            "bidi\u{202e}",
            "\u{3164}filler",
        ] {
            assert_eq!(
                name_problem(hidden),
                Some(NameProblem::Special),
                "{hidden:?}"
            );
        }
        assert_eq!(name_problem("Desk left"), None);
    }

    #[test]
    fn the_rule_matches_what_the_link_accepts() {
        for name in ["", " ", "Desk", "a\u{0}", &"x".repeat(49), "Café ☕"] {
            assert_eq!(
                name_problem(name).is_none(),
                td_link::identity::valid_name(name),
                "{name:?}"
            );
        }
    }

    #[test]
    fn each_screen_words_the_problem_its_own_way() {
        assert_eq!(
            NameProblem::Empty.settings_text(),
            "Give the Device a name."
        );
        assert_eq!(NameProblem::Empty.setup_text(), "Enter a name.");
        assert_eq!(
            NameProblem::TooLong.setup_text(),
            "Names can be up to 48 characters."
        );
    }
}
