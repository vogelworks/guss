//! `guss` task: interactive shell for guss commands.
//!
//! Provides a nested REPL with TUI where you can navigate through commands and execute them.
//! Outputs scroll in the main area; input is fixed at the bottom.

use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use guss_common::Result;
use guss_common::history::History;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::io;
use std::process::Command;

#[derive(Args)]
pub struct GussArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub initial_cmd: Vec<String>,
}

struct AppState {
    context: Vec<String>,
    output_lines: Vec<String>,
    scroll_pos: usize,
    input: String,
    running: bool,
    history: History,
    /// The `guss` clap tree, so navigation and help match what `guss` accepts.
    cli: clap::Command,
}

impl AppState {
    fn new(cli: clap::Command) -> Self {
        Self {
            context: vec![],
            output_lines: vec![],
            scroll_pos: 0,
            input: String::new(),
            running: true,
            history: History::new(),
            cli,
        }
    }

    fn context_key(&self) -> String {
        if self.context.is_empty() {
            "root".to_string()
        } else {
            self.context.join(":")
        }
    }

    fn add_to_history(&mut self, cmd: String) {
        let key = self.context_key();
        self.history.add(&key, cmd);
    }

    fn history_up(&mut self) {
        if let Some(prev) = self.history.prev(&self.context_key()) {
            self.input = prev;
        }
    }

    fn history_down(&mut self) {
        let key = self.context_key();
        let navigating = self.history.current(&key).is_some();
        match self.history.next(&key) {
            Some(next) => self.input = next,
            None if navigating => self.input.clear(),
            None => {}
        }
    }

    /// (name, description) of the subcommands available in the current context.
    fn subcommands(&self) -> Vec<(String, String)> {
        let mut cmd = &self.cli;
        for name in &self.context {
            match cmd.find_subcommand(name) {
                Some(sub) => cmd = sub,
                None => return vec![],
            }
        }
        cmd.get_subcommands()
            .map(|c| (c.get_name().to_string(), c.get_about().map(|a| a.to_string()).unwrap_or_default()))
            .collect()
    }

    /// `..` / `.. <cmd>`: go up one level, then into `cmd` if it exists there.
    /// Returns `cmd` back when it doesn't, leaving you at the parent.
    fn go_back<'a>(&mut self, cmd: Option<&'a str>) -> Option<&'a str> {
        self.context.pop();
        let cmd = cmd?;
        if self.is_valid_command(cmd) {
            self.context.push(cmd.to_string());
            None
        } else {
            Some(cmd)
        }
    }

    /// Whether `cmd` is a subcommand of the current context.
    fn is_valid_command(&self, cmd: &str) -> bool {
        self.subcommands().iter().any(|(name, _)| name == cmd)
    }

    fn scroll_up(&mut self) {
        self.scroll_pos = self.scroll_pos.saturating_sub(3);
    }

    fn scroll_down(&mut self) {
        if !self.output_lines.is_empty() {
            self.scroll_pos = (self.scroll_pos + 3).min(self.output_lines.len().saturating_sub(1));
        }
    }

    fn add_output(&mut self, line: String) {
        self.output_lines.push(line);
    }
}

/// `cli` is the top-level clap command, so the shell's command list and help
/// always match what `guss` actually accepts.
pub fn run(args: GussArgs, cli: clap::Command) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new(cli);

    // Handle initial command or show help
    if args.initial_cmd.len() == 1 && state.is_valid_command(&args.initial_cmd[0]) {
        state.context = args.initial_cmd;
        show_context_help(&mut state);
    } else if !args.initial_cmd.is_empty() {
        execute_and_capture(&mut state, &args.initial_cmd)?;
    } else {
        show_help(&mut state);
    }

    // Run TUI loop
    let result = tui_loop(&mut terminal, &mut state);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn tui_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &mut AppState) -> Result<()> {
    let tick_rate = std::time::Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();

    loop {
        terminal.draw(|f| ui(f, state))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => {
                        state.input.clear();
                        continue;
                    }
                    KeyCode::Enter => {
                        if !state.input.trim().is_empty() {
                            handle_input(state)?;
                            state.input.clear();
                        }
                    }
                    KeyCode::Backspace => {
                        state.input.pop();
                    }
                    KeyCode::Char(c) => {
                        state.input.push(c);
                    }
                    KeyCode::Up => state.scroll_up(),
                    KeyCode::Down => state.scroll_down(),
                    KeyCode::Left => state.history_up(),
                    KeyCode::Right => state.history_down(),
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }

        if !state.running {
            break;
        }
    }

    Ok(())
}

fn ui(f: &mut ratatui::Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)].as_ref())
        .split(f.size());

    // Output area with scrolling
    let output_text: Vec<Line> = state.output_lines.iter().map(|l| Line::from(l.as_str())).collect();
    let output = Paragraph::new(output_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} (↑↓ scroll, ← → history) ", state.context.join(" > "))),
        )
        .wrap(Wrap { trim: true })
        .scroll((state.scroll_pos as u16, 0));
    f.render_widget(output, chunks[0]);

    // Input area
    let input_text = format!("> {}", state.input);
    let input = Paragraph::new(input_text).block(Block::default().borders(Borders::ALL).title(" INPUT "));
    f.render_widget(input, chunks[1]);
}

fn handle_input(state: &mut AppState) -> Result<()> {
    let line = state.input.trim().to_string();

    match line.as_str() {
        "exit" | "quit" => {
            state.running = false;
            return Ok(());
        }
        "help" | "?" => {
            show_help(state);
            state.add_to_history(line);
        }
        input if input.split_whitespace().next() == Some("..") => {
            let missing = state.go_back(input.split_whitespace().nth(1));
            if state.context.is_empty() {
                show_help(state);
            } else {
                show_context_help(state);
            }
            if let Some(cmd) = missing {
                state.output_lines.insert(0, format!("no command '{cmd}' here"));
            }
            state.add_to_history(line);
        }
        cmd if state.is_valid_command(cmd) => {
            state.context.push(cmd.to_string());
            show_context_help(state);
            state.add_to_history(line);
        }
        _ => {
            let mut args = state.context.clone();
            args.extend(line.split_whitespace().map(|s| s.to_string()));
            execute_and_capture(state, &args)?;
            state.add_to_history(line);
        }
    }

    Ok(())
}

fn show_help(state: &mut AppState) {
    state.output_lines.clear();
    state.add_output("Available commands:".to_string());
    let lines: Vec<String> = state.subcommands().iter().map(|(name, about)| format!("  {name:<12} {about}")).collect();
    for line in lines {
        state.add_output(line);
    }
    state.add_output("".to_string());
    state.add_output("Navigation:".to_string());
    state.add_output("  <command>    Enter a command's context".to_string());
    state.add_output("  ..           Go back to parent context".to_string());
    state.add_output("  .. <cmd>     Go back, then into <cmd>".to_string());
    state.add_output("  help / ?     Show this help".to_string());
    state.add_output("  exit / quit  Exit the shell".to_string());
}

fn show_context_help(state: &mut AppState) {
    state.output_lines.clear();

    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            state.add_output("Error: could not get executable path".to_string());
            return;
        }
    };

    let mut cmd = Command::new(&exe_path);
    cmd.args(&state.context).arg("--help");

    match cmd.output() {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let help_text = if stdout.is_empty() { &stderr } else { &stdout };

            if help_text.is_empty() {
                state.add_output("(no help text available)".to_string());
            } else {
                for line in help_text.lines() {
                    state.add_output(line.to_string());
                }
            }
        }
        Err(e) => {
            state.add_output(format!("Error running help: {}", e));
        }
    }
}

fn execute_and_capture(state: &mut AppState, args: &[String]) -> Result<()> {
    // Build, deploy, and docs open in separate terminal; others show in TUI
    let is_separate_window = args.first().map(|s| s.as_str()) == Some("build")
        || args.first().map(|s| s.as_str()) == Some("deploy")
        || args.first().map(|s| s.as_str()) == Some("docs");

    if is_separate_window {
        execute_in_separate_window(args)?;
        state.add_output("Opened in separate window...".to_string());
    } else {
        execute_with_output(state, args)?;
    }
    Ok(())
}

fn execute_in_separate_window(args: &[String]) -> Result<()> {
    let exe_path = std::env::current_exe()?;
    guss_common::terminal::spawn_in_terminal(&exe_path, args)
}

fn execute_with_output(state: &mut AppState, args: &[String]) -> Result<()> {
    use std::io::BufRead;
    use std::process::Stdio;
    use std::sync::mpsc;

    state.output_lines.clear();
    state.add_output(format!("$ guss {}", args.join(" ")));
    state.add_output("".to_string());

    let exe_path = std::env::current_exe()?;
    let mut cmd = Command::new(exe_path);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    match cmd.spawn() {
        Ok(mut child) => {
            let (tx, rx) = mpsc::channel();

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let tx1 = tx.clone();
            std::thread::spawn(move || {
                if let Some(stdout) = stdout {
                    let reader = std::io::BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            let _ = tx1.send(line);
                        }
                    }
                }
            });

            let tx2 = tx.clone();
            std::thread::spawn(move || {
                if let Some(stderr) = stderr {
                    let reader = std::io::BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            let _ = tx2.send(line);
                        }
                    }
                }
            });

            drop(tx);
            for line in rx {
                state.add_output(line);
            }

            let _ = child.wait();
        }
        Err(e) => {
            state.add_output(format!("Error executing command: {}", e));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        let leaf = |n: &'static str| clap::Command::new(n);
        let cli = clap::Command::new("guss")
            .subcommand(leaf("net"))
            .subcommand(leaf("deploy").subcommand(leaf("tools")).subcommand(leaf("logs")));
        AppState::new(cli)
    }

    #[test]
    fn dot_dot_navigation() {
        let mut s = state();
        s.context = vec!["deploy".into(), "tools".into()];

        assert_eq!(s.go_back(Some("logs")), None);
        assert_eq!(s.context, ["deploy", "logs"], "sibling at the parent level");

        assert_eq!(s.go_back(Some("net")), Some("net"));
        assert_eq!(s.context, ["deploy"], "unknown at that level: stay at parent");

        assert_eq!(s.go_back(Some("net")), None);
        assert_eq!(s.context, ["net"]);

        s.go_back(None);
        s.go_back(None);
        assert!(s.context.is_empty(), ".. at root is a no-op");
    }

    #[test]
    fn commands_are_relative_to_context() {
        let mut s = state();
        assert!(s.is_valid_command("deploy") && !s.is_valid_command("tools"));
        s.context = vec!["deploy".into()];
        assert!(s.is_valid_command("tools") && !s.is_valid_command("net"));
    }
}
