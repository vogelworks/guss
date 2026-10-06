//! Interactive refine prompt for searchable output (docs, logs).
//!
//! Input goes through rustyline, so ← → edit the line, ↑ ↓ walk this session's
//! history, and Ctrl-R searches it. Empty input, `exit`, Ctrl-C or Ctrl-D quits.

use crate::error::Result;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

/// Prompt for terms until the user quits, handing each one to `display`.
pub fn refine(mut display: impl FnMut(&str)) -> Result<()> {
    let mut rl = DefaultEditor::new()?;
    loop {
        let term = match rl.readline("refine > ") {
            Ok(line) => line.trim().to_string(),
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        if term.is_empty() || term == "exit" {
            break;
        }
        rl.add_history_entry(&term)?;
        display(&term);
    }
    Ok(())
}

/// Print `content`, then filter its lines case-insensitively by each refine term.
pub fn show(content: &str) -> Result<()> {
    println!("{content}\n");
    let lines: Vec<&str> = content.lines().collect();
    println!("{} lines. Type a term to filter (↑ ↓ history, empty or 'exit' to quit):\n", lines.len());

    refine(|term| {
        let needle = term.to_lowercase();
        let matches: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|line| line.to_lowercase().contains(&needle))
            .collect();

        println!();
        if matches.is_empty() {
            println!("  (no matches for '{term}')\n");
        } else {
            println!("  {} matches for '{term}':", matches.len());
            for line in matches {
                println!("    {line}");
            }
            println!();
        }
    })
}
