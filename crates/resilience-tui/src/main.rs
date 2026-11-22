//! `resilience` — the Resilience OS terminal interface.
//!
//! ```text
//! resilience            start the interactive interface (needs a terminal)
//! resilience status     print a machine-readable status summary
//! resilience apps       list catalog entries and their availability
//! resilience --version  print the version
//! ```

use std::fs;
use std::io::{self, IsTerminal};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use resilience_core::catalog::Availability;
use resilience_core::config;
use resilience_core::{mode, network, status};

use resilience_tui::app::{App, Effect, RunRequest, Startup};
use resilience_tui::{keys, ui};

const USAGE: &str = "\
Usage: resilience [COMMAND]

Commands:
  (none)      Start the interactive Resilience interface
  status      Print a status summary (key: value lines)
  apps        List application catalog entries and their availability
  --version   Print the version
  --help      Print this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_interactive(),
        Some("status") => cli_status(),
        Some("apps") => cli_apps(),
        Some("-V" | "--version" | "version") => {
            println!("resilience {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("resilience: unknown command '{other}'\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn cli_status() -> ExitCode {
    let s = Startup::load();
    let snap = status::snapshot(&s.home);
    let net = network::passive_state();
    let state = status::system_state();
    let mode = s.who.mode;
    let ready = s
        .catalog
        .apps
        .iter()
        .filter(|a| {
            mode.allows(a.min_mode) && matches!(a.availability(mode), Availability::Ready(_))
        })
        .count();
    let visible = s
        .catalog
        .apps
        .iter()
        .filter(|a| mode.allows(a.min_mode))
        .count();
    println!("version: {}", resilience_core::VERSION);
    println!("edition: {}", s.sys.edition);
    println!("user: {}", s.who.user);
    println!(
        "mode: {}",
        match mode {
            mode::Mode::UltraStable => "ultra-stable",
            mode::Mode::Stable => "stable",
            mode::Mode::Advanced => "advanced",
        }
    );
    println!("system: {}", state.label().to_lowercase());
    println!("network: {}", net.level.label().to_lowercase());
    println!(
        "gateway: {}",
        net.gateway_v4
            .map(|(i, g)| format!("{g} via {i}"))
            .unwrap_or_else(|| "none".into())
    );
    println!("os: {}", snap.os_name);
    println!("apps: {visible} visible, {ready} ready");
    for w in &s.warnings {
        println!("warning: {w}");
    }
    ExitCode::SUCCESS
}

fn cli_apps() -> ExitCode {
    let s = Startup::load();
    let mode = s.who.mode;
    for a in &s.catalog.apps {
        let avail = if !mode.allows(a.min_mode) {
            format!("requires-{}", a.min_mode.label().to_lowercase())
        } else {
            match a.availability(mode) {
                Availability::Ready(_) => "ready".to_string(),
                Availability::Missing(p) => format!("unavailable({p})"),
            }
        };
        println!("{}\t{:?}\t{:?}\t{avail}", a.id, a.category, a.min_mode);
    }
    for w in &s.warnings {
        eprintln!("warning: {w}");
    }
    ExitCode::SUCCESS
}

/// Log to `$XDG_STATE_HOME/resilience/tui.log` (never to the terminal, which
/// belongs to the interface). Logging failures are not fatal.
fn init_logging(home: &Path) {
    let dir = config::state_home(home).join("resilience");
    if fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .is_err()
    {
        return;
    }
    let path = dir.join("tui.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::rename(&path, dir.join("tui.log.old"));
    }
    let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let filter = if std::env::var_os("RESILIENCE_DEBUG").is_some() {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_max_level(filter)
        .try_init();
}

fn run_interactive() -> ExitCode {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!(
            "resilience: the interactive interface needs a terminal (try 'resilience status')"
        );
        return ExitCode::from(2);
    }
    let s = Startup::load();
    init_logging(&s.home);
    tracing::info!(version = resilience_core::VERSION, user = %s.who.user, mode = %s.who.mode, "interface starting");

    // Log panics before ratatui's hook restores the terminal.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(%info, "interface panicked");
        previous(info);
    }));

    let mut app = App::new(s);
    // Before the screen is taken: loadkeys may print to the console.
    app.apply_saved_layout();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    match result {
        Ok(()) => {
            tracing::info!("interface exited normally");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(error = %e, "interface failed");
            eprintln!("resilience: {e}");
            ExitCode::FAILURE
        }
    }
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(k) => match app.handle_key(keys::from_crossterm(k)) {
                    Effect::None => {}
                    Effect::Quit => return Ok(()),
                    Effect::Run(req) => run_program(terminal, app, &req)?,
                },
                Event::Resize(..) => {}
                _ => {}
            }
        } else {
            app.on_tick();
        }
    }
}

/// Release the terminal, run a program in the foreground, take it back.
fn run_program(terminal: &mut DefaultTerminal, app: &mut App, req: &RunRequest) -> io::Result<()> {
    ratatui::restore();
    let result = resilience_tui::run_foreground(req);
    app.after_run(&req.label, &result);
    enable_raw_mode()?;
    // Clear without `Terminal::clear`: it asks the terminal for the cursor
    // position, and keys typed at that moment (Esc right after quitting a
    // program) mix with the answer, so crossterm gave up after 2 s and the
    // interface stopped. Resetting both buffers makes the next frame draw
    // every cell, as `clear` would.
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        Clear(ClearType::All),
        MoveTo(0, 0)
    )?;
    terminal.swap_buffers();
    terminal.swap_buffers();
    Ok(())
}
