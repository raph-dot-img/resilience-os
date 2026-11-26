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

/// Environment variable holding the graphical map viewer's absolute path
/// (set by the session; ADR 0026 amendment 1).
pub const MAPS_GUI_VAR: &str = "RESILIENCE_MAPS_GUI";

/// What a run ended with: the exit code, and for a native viewer the last
/// line it wrote on stderr (its reason when it refused every file).
pub type Outcome = Result<(i32, Option<String>), String>;

pub async fn in_terminal(req: RunRequest) -> Outcome {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let native = std::env::var_os(MAPS_GUI_VAR)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .and_then(|viewer| native_map(&req, &viewer));
        let r = match native {
            Some((viewer, args)) => run_native(&viewer, &args),
            None => run_blocking(&req).map(|c| (c, None)),
        };
        let _ = tx.send(r);
    });
    rx.await
        .unwrap_or_else(|_| Err("the runner thread stopped".into()))
}

/// The map viewer's run (`resilience-maps view ARGS...`) as the graphical
/// viewer's: the same arguments after `view`, the same files. Anything
/// else (or a request with environment changes) stays in the terminal.
pub fn native_map(req: &RunRequest, viewer: &std::path::Path) -> Option<(PathBuf, Vec<String>)> {
    let (prog, rest) = req.argv.split_first()?;
    let is_maps = std::path::Path::new(prog)
        .file_name()
        .is_some_and(|n| n == "resilience-maps");
    if !is_maps || rest.first().map(String::as_str) != Some("view") || !req.env.is_empty() {
        return None;
    }
    let mut args: Vec<String> = rest[1..].to_vec();
    for p in &req.extra_args {
        args.push(p.to_str()?.to_string());
    }
    Some((viewer.to_path_buf(), args))
}

/// Run the native viewer as a Wayland client of the same compositor (it
/// shows on top of the GUI), wait for it, keep the end of its stderr.
fn run_native(viewer: &std::path::Path, args: &[String]) -> Outcome {
    use std::io::Read;
    let mut child = Command::new(viewer)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start the map viewer: {e}"))?;
    // At most 16 KB of messages are kept; the rest is read and dropped.
    let mut kept = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let mut buf = [0u8; 4096];
        while let Ok(n) = err.read(&mut buf) {
            if n == 0 {
                break;
            }
            if kept.len() < 16 * 1024 {
                kept.extend_from_slice(&buf[..n]);
            }
        }
    }
    let st = child.wait().map_err(|e| format!("the map viewer: {e}"))?;
    let last = String::from_utf8_lossy(&kept)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.chars().filter(|c| !c.is_control()).take(200).collect());
    Ok((st.code().unwrap_or(1), last))
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
pub fn to_status(r: Outcome) -> io::Result<ExitStatus> {
    match r {
        Ok((code, _)) => Ok(ExitStatus::from_raw((code & 0xff) << 8)),
        Err(e) => Err(io::Error::other(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_round_trip() {
        assert!(to_status(Ok((0, None))).is_ok_and(|s| s.success()));
        assert_eq!(
            to_status(Ok((3, None))).ok().and_then(|s| s.code()),
            Some(3)
        );
        assert!(to_status(Err("x".into())).is_err());
    }

    fn req(argv: &[&str]) -> RunRequest {
        RunRequest {
            label: "Map".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            extra_args: vec![],
            env: vec![],
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: None,
        }
    }

    #[test]
    fn only_the_map_viewer_goes_native() {
        let v = std::path::Path::new("/nix/store/x/bin/resilience-maps-gui");
        let r = req(&[
            "/nix/store/y/bin/resilience-maps",
            "view",
            "--pack",
            "/a.mbtiles",
        ]);
        assert_eq!(
            native_map(&r, v),
            Some((v.to_path_buf(), vec!["--pack".into(), "/a.mbtiles".into()]))
        );
        let mut extra = r.clone();
        extra.extra_args.push("/b.mbtiles".into());
        assert_eq!(native_map(&extra, v).map(|x| x.1.len()), Some(3));
        // `info`, other programs, environment changes: the terminal.
        assert_eq!(
            native_map(&req(&["/x/resilience-maps", "info", "/a"]), v),
            None
        );
        assert_eq!(native_map(&req(&["/x/vim", "view"]), v), None);
        assert_eq!(
            native_map(&req(&["/x/resilience-maps-evil", "view"]), v),
            None
        );
        let mut env = r;
        env.env.push(("X".into(), "1".into()));
        assert_eq!(native_map(&env, v), None);
        assert_eq!(native_map(&req(&[]), v), None);
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
