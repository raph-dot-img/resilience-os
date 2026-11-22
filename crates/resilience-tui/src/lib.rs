//! The Resilience terminal interface as a library: the ratatui renderer of
//! `resilience-shell` (used by the `resilience` binary, and by
//! `resilience-gui` for screens it does not draw natively yet, ADR 0026).

pub use resilience_shell::{app, help};
pub mod keys;
pub mod theme;
pub mod ui;

use std::io::{self, BufRead, Write};
use std::process::ExitStatus;

use resilience_core::launch;

use crate::app::RunRequest;

/// Runs `req` in the foreground on the current terminal: clears it, prints
/// the banner, runs the program (with its polkit agent if any), and waits
/// for Enter when asked to or when the program failed.
pub fn run_foreground(req: &RunRequest) -> io::Result<ExitStatus> {
    let mut out = io::stdout();
    // Clear the screen so the program starts on a clean console.
    let _ = write!(out, "\x1b[2J\x1b[H");
    if let Some(b) = &req.banner {
        let _ = writeln!(out, "{b}\n");
    }
    let _ = out.flush();
    let extra: Vec<&std::ffi::OsStr> = req.extra_args.iter().map(|p| p.as_os_str()).collect();
    let result = match &req.polkit_agent {
        Some(agent) => launch::run_with_polkit_agent(agent, &req.argv, &req.env),
        None => launch::run_foreground_env(&req.argv, &extra, &req.env, req.clean_env),
    };
    let failed = !matches!(&result, Ok(st) if st.success());
    if req.pause_after || failed {
        match &result {
            Ok(st) if !st.success() => println!("\n[{} exited with {st}]", req.label),
            Err(e) => println!("\n[{} could not start: {e}]", req.label),
            _ => {}
        }
        print!("\nPress Enter to return to Resilience...");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    result
}

#[cfg(test)]
mod tests;
