//! "Community network" (ADR 0025): find the computers that share
//! resources nearby, read their library and pages, get packs from them,
//! and share this computer's resources.
//!
//! - **Community network**: this computer's sharing (on: Enter stops it),
//!   "Share this computer's resources", "Type the address of a computer",
//!   then every computer heard sharing (from the account's `shares.json`,
//!   written by its messaging program). Names are claims: `"name"?`.
//! - **A computer**: its front page, library and pages (read through
//!   `resilience-library open-remote`, a loopback relay and the restricted
//!   text browser), and its packs (`peer-list`, then `change --from-peer`:
//!   the store checks every pack against this system's catalog).
//! - **Share**: the network, the name, the services; the warning; then
//!   `pkexec <helper> set-share`, with an administrator's password.
//!
//! The interface never talks to the network itself: `resilience-library`
//! does, as the user, with the address checked again there.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use crate::list::ListState;
use resilience_core::admin::AdminConfig;
use resilience_core::packs::{self, PackConfig, PackState};
use resilience_core::proc::run_full;
use resilience_core::share::{self, HeardShare, Peer, Service, ShareState};
use std::time::Duration;

use super::{
    App, Effect, Level, Modal, Pending, RunRequest, View, clamp_selection, is_activate, is_back,
    nav,
};

pub const NEARBY_LABEL: &str = "Nearby computer";
pub const PEER_PACKS_LABEL: &str = "Packs from a nearby computer";
pub const SET_SHARE_LABEL: &str = "Sharing";
pub const PEER_FILE_LABEL: &str = "A file from a nearby computer";
pub const HOTSPOT_LABEL: &str = "Hotspot";

/// The lists are read again this often while the screen is shown.
const RELOAD_TICKS: u64 = 8;
/// Rows of the share form: network, name, four services, "Share now".
pub const SHARE_ROWS: usize = 7;

/// One line of the Community network screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommunityRow {
    /// This computer shares now; Enter stops it.
    Sharing(ShareState),
    /// Share this computer's resources.
    Share,
    /// Type the address of a computer.
    Address,
    /// This computer as a Wi-Fi network (on: what it is).
    Hotspot { on: Option<String> },
    /// The mesh network (ADR 0021), from here too.
    Mesh { on: bool },
    /// A computer heard sharing, and how to reach it from here (`None`:
    /// none of its addresses is on this computer's networks).
    Machine {
        heard: HeardShare,
        peer: Option<Peer>,
    },
}

/// What can be done with a nearby computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAction {
    Index,
    Library,
    Pages,
    Files,
    Packs,
}

impl PeerAction {
    pub fn name(self) -> &'static str {
        match self {
            PeerAction::Index => "Its front page (what it shares)",
            PeerAction::Library => "Read its library (encyclopedias, guides...)",
            PeerAction::Pages => "Read its pages (notices, news)",
            PeerAction::Files => "Files in its shared folder (copy one into Received)",
            PeerAction::Packs => "Get packs from it (checked before installing)",
        }
    }

    fn what(self) -> Option<&'static str> {
        match self {
            PeerAction::Index => Some("index"),
            PeerAction::Library => Some("library"),
            PeerAction::Pages => Some("pages"),
            PeerAction::Files | PeerAction::Packs => None,
        }
    }
}

/// A catalog pack a nearby computer offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPack {
    pub id: String,
    pub title: String,
    pub size: u64,
    pub installed: bool,
}

/// The choices before sharing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareForm {
    /// Network interfaces that are up (not the loopback).
    pub interfaces: Vec<String>,
    pub iface: usize,
    pub name: String,
    /// In the order of `Service::ALL`.
    pub services: [bool; 4],
}

impl ShareForm {
    pub fn chosen(&self) -> Vec<Service> {
        Service::ALL
            .into_iter()
            .zip(self.services)
            .filter_map(|(s, on)| on.then_some(s))
            .collect()
    }
}

/// What is typed in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareInput {
    Address,
    Name,
}

pub fn service_label(s: Service) -> &'static str {
    match s {
        Service::Packs => "Packs (encyclopedias, maps... others can download them)",
        Service::Library => "Library (others read your packs in their browser)",
        Service::Files => "Shared folder (/var/lib/resilience/shared/files)",
        Service::Pages => "Pages (/var/lib/resilience/shared/pages)",
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl App {
    /// The helper's configuration, if this system offers sharing.
    fn share_config(&self) -> Option<AdminConfig> {
        std::fs::read_to_string(&self.paths.admin_config)
            .ok()
            .and_then(|t| AdminConfig::parse(&t).ok())
            .filter(|c| c.share.is_some())
    }

    pub(crate) fn share_offered(&self) -> bool {
        self.share_config().is_some()
    }

    /// What this computer shares now, if anything.
    pub(crate) fn share_state(&self) -> Option<ShareState> {
        std::fs::read_to_string(&self.paths.share_state)
            .ok()
            .and_then(|t| ShareState::parse(&t))
    }

    /// The line shown at the top of every screen while sharing (ADR 0025 §3).
    pub fn sharing_banner(&self) -> Option<String> {
        let s = self.share_state()?;
        Some(format!(
            "SHARING {} on {} as \"{}\". Stop: Communications > Community network.",
            s.services.join(", "),
            s.interface,
            s.name
        ))
    }

    fn routes(&self) -> (String, String) {
        (
            std::fs::read_to_string(&self.paths.route4).unwrap_or_default(),
            std::fs::read_to_string(&self.paths.route6).unwrap_or_default(),
        )
    }

    pub fn community_rows(&self) -> Vec<CommunityRow> {
        let mut rows = Vec::new();
        match self.share_state() {
            Some(s) => rows.push(CommunityRow::Sharing(s)),
            None => rows.push(CommunityRow::Share),
        }
        rows.push(CommunityRow::Address);
        let admin = std::fs::read_to_string(&self.paths.admin_config)
            .ok()
            .and_then(|t| AdminConfig::parse(&t).ok());
        if let Some(h) = admin.as_ref().and_then(|c| c.hotspot.as_ref()) {
            let on = std::fs::read_to_string(&self.paths.hotspot_state)
                .ok()
                .and_then(|t| {
                    t.lines().find_map(|l| {
                        l.strip_prefix("device = \"")?
                            .strip_suffix('"')
                            .map(String::from)
                    })
                })
                .filter(|d| share::valid_interface(d))
                .map(|d| format!("\"{}\" on {d}", h.ssid));
            rows.push(CommunityRow::Hotspot { on });
        }
        if admin.as_ref().is_some_and(|c| c.mesh_unit.is_some()) {
            rows.push(CommunityRow::Mesh {
                on: self.paths.mesh_devices.exists(),
            });
        }
        let text =
            std::fs::read_to_string(self.chat_store().root.join("shares.json")).unwrap_or_default();
        let (r4, r6) = self.routes();
        for heard in share::parse_heard(&text, now()) {
            let peer = share::peer_for(&heard, &r4, &r6);
            rows.push(CommunityRow::Machine { heard, peer });
        }
        rows
    }

    pub(super) fn open_community(&self) -> View {
        View::Community {
            rows: self.community_rows(),
            list: ListState::default().with_selected(Some(0)),
        }
    }

    fn reload_community(&mut self) {
        let rows = self.community_rows();
        for v in &mut self.stack {
            if let View::Community { rows: r, list } = v {
                *r = rows.clone();
                clamp_selection(list, r.len());
            }
        }
    }

    /// From the main loop's tick: keep a visible Community screen current.
    pub(super) fn community_tick(&mut self) {
        if self.tick.is_multiple_of(RELOAD_TICKS)
            && matches!(self.stack.last(), Some(View::Community { .. }))
        {
            self.reload_community();
        }
    }

    fn peer_menu(&self, name: String, peer: Peer, services: &[Service]) -> View {
        let mut actions = vec![PeerAction::Index];
        for (s, a) in [
            (Service::Library, PeerAction::Library),
            (Service::Pages, PeerAction::Pages),
            (Service::Files, PeerAction::Files),
            (Service::Packs, PeerAction::Packs),
        ] {
            if services.contains(&s) {
                actions.push(a);
            }
        }
        View::PeerMenu {
            name,
            peer,
            actions,
            list: ListState::default().with_selected(Some(0)),
        }
    }

    /// The catalog packs `peer` offers (asked with `resilience-library
    /// peer-list`, which checks the address again and talks to it).
    fn peer_packs_view(&mut self, peer: &Peer) -> Option<View> {
        let cfg = PackConfig::load(&self.paths.packs_config).ok()?;
        let inv = packs::inventory(&cfg);
        let addr = peer.display();
        let out = run_full(
            &[self.sys.tools.library.as_str(), "peer-list", addr.as_str()],
            Duration::from_secs(25),
        );
        let out = match out {
            Some(o) if o.success() => o.stdout,
            Some(o) => {
                let why = o
                    .stderr
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("no answer")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect::<String>();
                self.notify(Level::Error, format!("No list of packs: {why}"));
                return None;
            }
            None => {
                self.notify(Level::Error, "The computer did not answer in time.");
                return None;
            }
        };
        let rows: Vec<PeerPack> = out
            .lines()
            .filter_map(|l| {
                let id = l.split('\t').next()?;
                let i = cfg.packs.iter().position(|p| p.id == id)?;
                let p = &cfg.packs[i];
                Some(PeerPack {
                    id: p.id.clone(),
                    title: p.title.clone(),
                    size: p.size,
                    installed: inv.states.get(i) == Some(&PackState::Installed),
                })
            })
            .collect();
        let ticked = vec![false; rows.len()];
        let sel = (!rows.is_empty()).then_some(0);
        Some(View::PeerPacks {
            peer: peer.clone(),
            rows,
            ticked,
            list: ListState::default().with_selected(sel),
        })
    }

    /// The files of `peer`'s shared folder (`resilience-library
    /// peer-files`, which checks the address again and talks to it).
    fn peer_files_view(&mut self, peer: &Peer) -> Option<View> {
        let addr = peer.display();
        let out = run_full(
            &[self.sys.tools.library.as_str(), "peer-files", addr.as_str()],
            Duration::from_secs(25),
        );
        let text = match out {
            Some(o) if o.success() => o.stdout,
            Some(o) => {
                let why: String = o
                    .stderr
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("no answer")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect();
                self.notify(Level::Error, format!("No list of files: {why}"));
                return None;
            }
            None => {
                self.notify(Level::Error, "The computer did not answer in time.");
                return None;
            }
        };
        let rows: Vec<share::SharedFile> = text
            .lines()
            .filter_map(|l| {
                let (name, size) = l.rsplit_once('\t')?;
                let size = size.parse().ok()?;
                share::valid_shared_name(name).then(|| share::SharedFile {
                    name: name.to_string(),
                    size,
                })
            })
            .take(share::MAX_FOLDER_FILES)
            .collect();
        let sel = (!rows.is_empty()).then_some(0);
        Some(View::PeerFiles {
            peer: peer.clone(),
            rows,
            list: ListState::default().with_selected(sel),
        })
    }

    /// `resilience-library peer-get ADDRESS NAME`.
    pub(super) fn peer_get_effect(&self, peer: &str, name: &str) -> Effect {
        tracing::info!(peer, "file from a nearby computer requested");
        Effect::Run(RunRequest {
            label: PEER_FILE_LABEL.into(),
            argv: vec![
                self.sys.tools.library.clone(),
                "peer-get".into(),
                peer.to_string(),
                name.to_string(),
            ],
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: None,
        })
    }

    /// In a shared folder (`files` or `pages`): how many files everyone
    /// may read (they are served) and how many are private (not served).
    /// Hidden files are never served and not counted.
    pub fn shared_counts(&self, sub: &str) -> (usize, usize) {
        use std::os::unix::fs::PermissionsExt;
        let mut out = (0, 0);
        let Ok(d) = std::fs::read_dir(self.paths.shared_dir.join(sub)) else {
            return out;
        };
        for e in d.flatten().take(10_000) {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            match std::fs::symlink_metadata(e.path()) {
                Ok(m) if m.is_file() && m.permissions().mode() & 0o004 != 0 => out.0 += 1,
                Ok(_) => out.1 += 1,
                Err(_) => {}
            }
        }
        out
    }

    /// Network interfaces that are up, not the loopback.
    fn up_interfaces(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.paths.sys_net)
            .map(|d| {
                d.flatten()
                    .filter_map(|e| e.file_name().to_str().map(String::from))
                    .filter(|n| share::valid_interface(n))
                    .filter(|n| {
                        let dir = self.paths.sys_net.join(n);
                        let flags = std::fs::read_to_string(dir.join("flags")).unwrap_or_default();
                        let kind = std::fs::read_to_string(dir.join("type")).unwrap_or_default();
                        share::interface_facts(&flags, &kind).is_some_and(|f| f.up && !f.loopback)
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    fn share_form_view(&mut self) -> Option<View> {
        let interfaces = self.up_interfaces();
        if interfaces.is_empty() {
            self.modal = Some(Modal::Message {
                title: "SHARE".into(),
                body: "This computer is on no network now. Join a Wi-Fi network, plug a \
                       cable, or turn the mesh network on first."
                    .into(),
                error: true,
            });
            return None;
        }
        Some(View::ShareSetup {
            form: ShareForm {
                interfaces,
                iface: 0,
                name: "Resilience".into(),
                services: [true, true, false, false],
            },
            list: ListState::default().with_selected(Some(0)),
        })
    }

    fn confirm_share(&mut self, form: &ShareForm) {
        let services = form.chosen();
        if services.is_empty() {
            self.notify(Level::Warn, "Tick at least one thing to share.");
            return;
        }
        let Some(iface) = form.interfaces.get(form.iface) else {
            return;
        };
        let mut args = vec![
            "on".to_string(),
            iface.clone(),
            "--name".into(),
            form.name.clone(),
        ];
        args.extend(services.iter().map(|s| s.id().to_string()));
        let list = services
            .iter()
            .map(|s| format!("- {}", service_label(*s)))
            .collect::<Vec<_>>()
            .join("\n");
        self.modal = Some(Modal::Confirm {
            title: "SHARE ON THIS NETWORK".into(),
            body: format!(
                "Share on {iface} as \"{}\":\n{list}\n\nEverything you share can be read by \
                 ANYONE on this network. Share only what you would put on a public notice \
                 board.\n\nIt stops when you stop it, when this network changes, or when \
                 the computer restarts. An administrator's password is needed.",
                form.name
            ),
            confirm: "Share".into(),
            action: Pending::SetShare { args },
            yes_focused: false,
        });
    }

    /// `pkexec <helper> set-share ...` with an administrator's password.
    pub(super) fn set_share_effect(&mut self, args: &[String]) -> Effect {
        let Some(cfg) = self.share_config() else {
            self.notify(Level::Error, "Sharing is not offered on this computer.");
            return Effect::None;
        };
        let mut argv = vec![
            cfg.pkexec.to_string_lossy().into_owned(),
            cfg.helper.to_string_lossy().into_owned(),
            "set-share".into(),
        ];
        argv.extend(args.iter().cloned());
        tracing::warn!(?args, "sharing change requested");
        Effect::Run(RunRequest {
            label: SET_SHARE_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(
                "SHARING. An administrator must now type their password. To cancel, press \
                 Ctrl-C while the password is asked."
                    .into(),
            ),
        })
    }

    /// `pkexec <helper> set-hotspot on|off`, in the foreground: the
    /// helper shows the password, which stays on screen until a key.
    pub(super) fn set_hotspot_effect(&mut self, on: bool) -> Effect {
        let Some(cfg) = self.share_config().filter(|c| c.hotspot.is_some()) else {
            self.notify(Level::Error, "This computer offers no hotspot.");
            return Effect::None;
        };
        tracing::warn!(on, "hotspot change requested");
        Effect::Run(RunRequest {
            label: HOTSPOT_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-hotspot".into(),
                if on { "on" } else { "off" }.into(),
            ],
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(
                "HOTSPOT. An administrator must now type their password. To cancel, press \
                 Ctrl-C while the password is asked."
                    .into(),
            ),
        })
    }

    /// `resilience-library change --from-peer ADDRESS --get ID...`.
    pub(super) fn peer_packs_effect(&self, peer: &str, get: &[String]) -> Effect {
        let mut argv = vec![
            self.sys.tools.library.clone(),
            "change".into(),
            "--from-peer".into(),
            peer.to_string(),
        ];
        for id in get {
            argv.extend(["--get".to_string(), id.clone()]);
        }
        tracing::warn!(peer, ?get, "packs from a nearby computer requested");
        Effect::Run(RunRequest {
            label: PEER_PACKS_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(
                "PACKS FROM A NEARBY COMPUTER. Each pack is checked against this system's \
                 catalog before it is installed. An administrator must now type their \
                 password; Ctrl-C stops, and an unfinished pack continues next time."
                    .into(),
            ),
        })
    }

    fn open_remote_effect(&self, peer: &Peer, what: &str) -> Effect {
        Effect::Run(RunRequest {
            label: NEARBY_LABEL.into(),
            argv: vec![
                self.sys.tools.library.clone(),
                "open-remote".into(),
                peer.display(),
                what.into(),
            ],
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: false,
            banner: None,
        })
    }

    /// After a program started from these screens exits.
    pub(super) fn share_after_run(&mut self, label: &str) {
        if label == HOTSPOT_LABEL {
            self.reload_community();
        }
        if label == SET_SHARE_LABEL {
            while matches!(self.view(), View::ShareSetup { .. }) {
                self.stack.pop();
            }
            self.reload_community();
        }
        if label == PEER_PACKS_LABEL {
            while matches!(self.view(), View::PeerPacks { .. }) {
                self.stack.pop();
            }
        }
    }

    /// Keys of the Community network screens.
    pub(super) fn community_key(
        &mut self,
        mut view: View,
        key: KeyEvent,
    ) -> (Option<View>, Effect) {
        let mut next: Option<View> = None;
        let mut effect = Effect::None;
        match &mut view {
            View::Community { rows, list } => match key.code {
                KeyCode::Char('r') => {
                    *rows = self.community_rows();
                    clamp_selection(list, rows.len());
                }
                KeyCode::Char('a') => self.ask_share(ShareInput::Address, String::new()),
                _ if is_activate(key) => match list.selected().and_then(|i| rows.get(i)).cloned() {
                    Some(CommunityRow::Sharing(s)) => {
                        self.modal = Some(Modal::Confirm {
                            title: "STOP SHARING".into(),
                            body: format!(
                                "This computer shares {} on {} as \"{}\".\n\nStop sharing? \
                                     An administrator's password is needed.",
                                s.services.join(", "),
                                s.interface,
                                s.name
                            ),
                            confirm: "Stop sharing".into(),
                            action: Pending::SetShare {
                                args: vec!["off".into()],
                            },
                            yes_focused: false,
                        });
                    }
                    Some(CommunityRow::Share) => next = self.share_form_view(),
                    Some(CommunityRow::Address) => {
                        self.ask_share(ShareInput::Address, String::new())
                    }
                    Some(CommunityRow::Mesh { .. }) => self.open_mesh(),
                    Some(CommunityRow::Hotspot { on }) => self.confirm_hotspot(on),
                    Some(CommunityRow::Machine { heard, peer }) => match peer {
                        Some(p) => {
                            next = Some(self.peer_menu(
                                format!("\"{}\"?", heard.name),
                                p,
                                &heard.services,
                            ))
                        }
                        None => {
                            self.modal = Some(Modal::Message {
                                title: "NOT REACHABLE".into(),
                                body: format!(
                                    "\"{}\" was heard, but none of its addresses is on a \
                                         network this computer is connected to. Join the same \
                                         Wi-Fi network or mesh, then try again.",
                                    heard.name
                                ),
                                error: true,
                            });
                        }
                    },
                    None => {}
                },
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, rows.len(), key),
            },
            View::PeerMenu {
                peer,
                actions,
                list,
                ..
            } => {
                if is_activate(key) {
                    match list.selected().and_then(|i| actions.get(i)).copied() {
                        Some(PeerAction::Packs) => {
                            let p = peer.clone();
                            next = self.peer_packs_view(&p);
                        }
                        Some(PeerAction::Files) => {
                            let p = peer.clone();
                            next = self.peer_files_view(&p);
                        }
                        Some(a) => {
                            if let Some(w) = a.what() {
                                effect = self.open_remote_effect(peer, w);
                            }
                        }
                        None => {}
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, actions.len(), key);
                }
            }
            View::PeerPacks {
                peer,
                rows,
                ticked,
                list,
            } => match key.code {
                KeyCode::Char(' ') => {
                    if let Some(i) = list.selected()
                        && let Some(r) = rows.get(i)
                    {
                        if r.installed {
                            self.notify(Level::Info, "This pack is already installed.");
                        } else {
                            ticked[i] = !ticked[i];
                        }
                    }
                }
                KeyCode::Char('d') | KeyCode::Enter => {
                    let get: Vec<String> = rows
                        .iter()
                        .zip(ticked.iter())
                        .filter(|(_, t)| **t)
                        .map(|(r, _)| r.id.clone())
                        .collect();
                    if get.is_empty() {
                        self.notify(Level::Warn, "Tick packs with Space first.");
                    } else {
                        let size: u64 = rows
                            .iter()
                            .zip(ticked.iter())
                            .filter(|(_, t)| **t)
                            .map(|(r, _)| r.size)
                            .sum();
                        self.modal = Some(Modal::Confirm {
                            title: "GET PACKS".into(),
                            body: format!(
                                "Get {} pack(s), {} in all, from the computer at {}?\n\nEach \
                                 pack is checked against this system's catalog before it is \
                                 installed: a pack that does not match is refused. An \
                                 administrator's password is needed.",
                                get.len(),
                                resilience_core::human_bytes(size),
                                peer.display()
                            ),
                            confirm: "Get".into(),
                            action: Pending::PeerPacks {
                                peer: peer.display(),
                                get,
                            },
                            yes_focused: false,
                        });
                    }
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, rows.len(), key),
            },
            View::PeerFiles { peer, rows, list } => {
                if is_activate(key) {
                    if let Some(f) = list.selected().and_then(|i| rows.get(i)) {
                        if f.size > share::MAX_FOLDER_FILE {
                            self.notify(Level::Warn, "This file is too large to copy.");
                        } else {
                            self.modal = Some(Modal::Confirm {
                                title: "COPY A FILE".into(),
                                body: format!(
                                    "Copy '{}' ({}) from the computer at {} into your \
                                     Received folder?\n\nIt is not checked: it is as \
                                     trustworthy as whoever runs that computer. Nothing is \
                                     replaced, and it is never run.",
                                    resilience_core::messages::clean_line(&f.name, 80),
                                    resilience_core::human_bytes(f.size),
                                    peer.display()
                                ),
                                confirm: "Copy".into(),
                                action: Pending::PeerGet {
                                    peer: peer.display(),
                                    name: f.name.clone(),
                                },
                                yes_focused: false,
                            });
                        }
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, rows.len(), key);
                }
            }
            View::ShareSetup { form, list } => {
                let row = list.selected().unwrap_or(0);
                if key.code == KeyCode::Char(' ') || is_activate(key) {
                    match row {
                        0 => form.iface = (form.iface + 1) % form.interfaces.len().max(1),
                        1 => {
                            let n = form.name.clone();
                            self.ask_share(ShareInput::Name, n);
                        }
                        2..=5 => form.services[row - 2] = !form.services[row - 2],
                        _ => {
                            let f = form.clone();
                            self.confirm_share(&f);
                        }
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, SHARE_ROWS, key);
                }
            }
            _ => {}
        }
        match next {
            Some(n) => {
                self.stack.push(view);
                (Some(n), effect)
            }
            None => (Some(view), effect),
        }
    }

    fn confirm_hotspot(&mut self, on: Option<String>) {
        let (body, confirm, turn_on) = match on {
            Some(what) => (
                format!(
                    "The hotspot {what} is ON.\n\nTurn it off? The people on it lose the \
                     connection. An administrator's password is needed."
                ),
                "Turn off",
                false,
            ),
            None => (
                "Make this computer a Wi-Fi network that phones and other computers can \
                 join, when there is no Wi-Fi router.\n\n- A new password is shown once; \
                 give it to the people who should join.\n- It gives no access to the \
                 Internet through this computer.\n- This computer's Wi-Fi cannot join \
                 another network meanwhile (a cable still works).\n- Share something on it \
                 from this screen, or there is nothing to read on it.\n\nAn administrator's \
                 password is needed. It stops when turned off or at restart."
                    .to_string(),
                "Turn on",
                true,
            ),
        };
        self.modal = Some(Modal::Confirm {
            title: "HOTSPOT".into(),
            body,
            confirm: confirm.into(),
            action: Pending::SetHotspot { on: turn_on },
            yes_focused: false,
        });
    }

    fn ask_share(&mut self, kind: ShareInput, value: String) {
        self.modal = Some(Modal::ShareInput {
            kind,
            value,
            error: None,
        });
    }

    pub(super) fn share_input_key(
        &mut self,
        kind: ShareInput,
        mut value: String,
        error: Option<String>,
        key: KeyEvent,
    ) -> Effect {
        let mut error = error;
        match key.code {
            KeyCode::Esc => return Effect::None,
            KeyCode::Enter => match self.share_submit(kind, value.trim()) {
                Ok(()) => return Effect::None,
                Err(e) => error = Some(e),
            },
            KeyCode::Backspace => {
                value.pop();
                error = None;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if value.chars().count() < 64 && !c.is_control() {
                    value.push(c);
                }
                error = None;
            }
            _ => {}
        }
        self.modal = Some(Modal::ShareInput { kind, value, error });
        Effect::None
    }

    fn share_submit(&mut self, kind: ShareInput, value: &str) -> Result<(), String> {
        match kind {
            ShareInput::Name => {
                if !share::valid_name(value) {
                    return Err("1 to 32 letters, digits, spaces or dashes, starting and \
                                ending with a letter or digit."
                        .into());
                }
                if let Some(View::ShareSetup { form, .. }) = self.stack.last_mut() {
                    form.name = value.to_string();
                }
                Ok(())
            }
            ShareInput::Address => {
                let p = Peer::parse(value).ok_or(
                    "Not the address of a computer on a local network (for example \
                     192.168.1.20, or fe80::1%bat0 on the mesh).",
                )?;
                let (r4, r6) = self.routes();
                if !share::on_link(&p, &r4, &r6) {
                    return Err("This address is not on a network this computer is \
                                connected to."
                        .into());
                }
                let menu = self.peer_menu(p.display(), p, &Service::ALL);
                self.stack.push(menu);
                Ok(())
            }
        }
    }
}
