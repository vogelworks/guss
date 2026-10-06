//! Terminal UI helpers: section headers and automation-safe confirmation prompts.

use crate::error::{Result, bail};
use colored::Colorize;
use std::io::{self, IsTerminal, Write};

/// The 80-char rule the old shell scripts and `cli` printed between sections.
pub fn section() {
    println!("{}", "_".repeat(80));
}

/// Print a boxed heading (replaces the ad-hoc `echo "____"` blocks in the scripts).
pub fn heading(title: &str) {
    section();
    println!("  {}", title.bold());
    section();
}

pub fn info(msg: &str) {
    println!("{}", msg);
}

pub fn good(msg: &str) {
    println!("{}", msg.green());
}

pub fn warn(msg: &str) {
    eprintln!("{}", msg.yellow());
}

/// Automation-safe yes/no prompt.
///
/// - `assume_yes` (the `--yes` flag): return `true` without prompting.
/// - Interactive TTY: prompt and read `y`/`n`.
/// - Non-interactive (piped/CI) without `--yes`: return `false` so the caller
///   aborts rather than silently proceeding on a destructive step.
pub fn confirm(prompt: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        println!("{prompt} [auto-yes]");
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{prompt} (y/n) ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// Confirm a step that must not proceed without approval; errors out otherwise.
/// Convenience wrapper so destructive callers can `require_confirm(...)?;`.
pub fn require_confirm(prompt: &str, assume_yes: bool) -> Result<()> {
    if confirm(prompt, assume_yes)? {
        Ok(())
    } else {
        bail!("aborted (not confirmed — pass --yes for non-interactive use)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assume_yes_confirms_without_a_tty() {
        // --yes must short-circuit to true regardless of terminal state.
        assert!(confirm("proceed?", true).unwrap());
        assert!(require_confirm("proceed?", true).is_ok());
    }

    #[test]
    fn non_tty_without_yes_declines() {
        // Under `cargo test`, stdin is not a terminal, so confirm must decline
        // (false) and require_confirm must error rather than block on input.
        assert!(!confirm("proceed?", false).unwrap());
        assert!(require_confirm("proceed?", false).is_err());
    }
}
