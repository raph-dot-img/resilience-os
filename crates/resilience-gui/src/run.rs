//! Programs started from the GUI run in a terminal window (foot) on top of
//! it: the same foreground run as in the TUI (banner, polkit text agent,
//! "Press Enter" pause), so secrets are typed into the programs themselves
//! exactly as on the console (ADR 0015, 0026).
//!
//! The request goes to `resilience-gui run FILE` inside foot through a 0600
//! file in `$XDG_RUNTIME_DIR` (requests carry no secrets).

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};

use resilience_shell::app::RunRequest;

/// Environment variable holding foot's absolute path (set by the session).
pub const TERMINAL_VAR: &str = "RESILIENCE_TERMINAL";

pub async fn in_terminal(req: RunRequest) -> Result<i32, String> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_blocking(&req));
    });
    rx.await
        .unwrap_or_else(|_| Err("the runner thread stopped".into()))
}

fn run_blocking(req: &RunRequest) -> Result<i32, String> {
    let foot = std::env::var_os(TERMINAL_VAR)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("{TERMINAL_VAR} is not set to an absolute path"))?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let file =
        write_request(&runtime_dir(), req).map_err(|e| format!("cannot write the request: {e}"))?;
    let status = Command::new(&foot)
        .arg("--app-id=resilience-run")
        .arg(format!("--title={}", req.label))
        .arg("--")
        .arg(&me)
        .arg("run")
        .arg(&file)
        .status();
    let _ = fs::remove_file(&file);
    match status {
        Ok(st) => Ok(st.code().unwrap_or(1)),
        Err(e) => Err(format!("cannot start the terminal: {e}")),
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
}

fn write_request(dir: &std::path::Path, req: &RunRequest) -> io::Result<PathBuf> {
    let json = serde_json::to_vec(req).map_err(io::Error::other)?;
    for n in 0..100u32 {
        let path = dir.join(format!("resilience-run-{}-{n}.json", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(&json)?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free request file name"))
}

/// `resilience-gui run FILE`: inside the terminal window.
pub fn run_file(path: &str) -> ExitCode {
    let req: Result<RunRequest, String> = fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
    let _ = fs::remove_file(path);
    let req = match req {
        Ok(r) => r,
        Err(e) => {
            eprintln!("resilience-gui: bad run request: {e}");
            return ExitCode::from(2);
        }
    };
    match resilience_tui::run_foreground(&req) {
        Ok(st) => ExitCode::from(st.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(_) => ExitCode::from(127),
    }
}

/// The terminal's exit code as the status `App::after_run` expects.
pub fn to_status(r: Result<i32, String>) -> io::Result<ExitStatus> {
    match r {
        Ok(code) => Ok(ExitStatus::from_raw((code & 0xff) << 8)),
        Err(e) => Err(io::Error::other(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_round_trip() {
        assert!(to_status(Ok(0)).is_ok_and(|s| s.success()));
        assert_eq!(to_status(Ok(3)).ok().and_then(|s| s.code()), Some(3));
        assert!(to_status(Err("x".into())).is_err());
    }

    #[test]
    fn request_file_is_private_and_parses() {
        let req = RunRequest {
            label: "Editor".into(),
            argv: vec!["/bin/true".into()],
            extra_args: vec![],
            env: vec![("SHELL".into(), "/x".into())],
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: None,
        };
        let dir = std::env::temp_dir().join(format!("resilience-gui-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        let p = write_request(&dir, &req).expect("write");
        let mode = fs::metadata(&p).expect("meta").permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600);
        let back: RunRequest = serde_json::from_slice(&fs::read(&p).expect("read")).expect("parse");
        assert_eq!(back, req);
        fs::remove_file(p).expect("rm");
        fs::remove_dir(dir).expect("rmdir");
    }
}
