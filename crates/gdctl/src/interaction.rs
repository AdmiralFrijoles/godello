//! Asking the user yes or no questions.
//!
//! Every command must be usable without prompts. When the non interactive flag
//! is set, a question takes the safe default and never waits for input. This keeps
//! that logic in one place so commands just call confirm with a sensible default.

use std::io::{self, Write};

/// Decides yes or no questions, prompting only in interactive mode.
pub struct Interaction {
    /// When true, never prompt. Return the default for each question.
    yes: bool,
}

impl Interaction {
    pub fn new(yes: bool) -> Self {
        Interaction { yes }
    }

    /// Ask a yes or no question. In non interactive mode the default is returned
    /// at once. Otherwise the user is prompted and a blank or unrecognized answer
    /// falls back to the default.
    pub fn confirm(&self, question: &str, default_yes: bool) -> bool {
        if self.yes {
            return default_yes;
        }
        let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
        print!("{question} {hint} ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(_) => interpret_response(&line, default_yes),
            // No input available, for example a closed stdin, takes the default.
            Err(_) => default_yes,
        }
    }
}

/// Read a yes or no answer. A blank or unrecognized answer takes the default.
fn interpret_response(input: &str, default_yes: bool) -> bool {
    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_and_no_words_are_read() {
        assert!(interpret_response("y", false));
        assert!(interpret_response("yes", false));
        assert!(interpret_response("YES\n", false));
        assert!(!interpret_response("n", true));
        assert!(!interpret_response("no", true));
        assert!(!interpret_response("  No  ", true));
    }

    #[test]
    fn blank_and_unknown_take_the_default() {
        assert!(interpret_response("", true));
        assert!(!interpret_response("", false));
        assert!(interpret_response("maybe", true));
        assert!(!interpret_response("maybe", false));
    }

    #[test]
    fn non_interactive_returns_the_default_without_input() {
        let interaction = Interaction::new(true);
        assert!(interaction.confirm("install?", true));
        assert!(!interaction.confirm("reset?", false));
    }
}
