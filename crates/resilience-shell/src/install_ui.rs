//! Installing Resilience on this computer (ADR 0022), live system only.
//!
//! The Install screen gathers the answers (disk, encryption, keyboard
//! layout, accounts); nothing is changed until the person has read the
//! summary and typed the disk's name. Then `resilience-install run …` runs
//! in the foreground through pkexec: it checks everything again and asks
//! for the passphrase and passwords itself, on the console.

use std::time::Duration;

use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use crate::list::ListState;
use resilience_core::admin::AdminConfig;
use resilience_core::install::{
    self, Disk, Firmware, INSTALL_CONFIG_PATH, InstallConfig, LSBLK_COLUMNS, MAX_ACCOUNTS,
    NewAccount, Request,
};
use resilience_core::media::parse_pairs;
use resilience_core::mode::Mode;
use resilience_core::proc::run_full;

use super::{App, Effect, Modal, RunRequest, View, clamp_selection, is_activate, is_back, nav};

pub const INSTALL_LABEL: &str = "resilience-install";

/// Everything chosen so far on the Install screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallForm {
    pub cfg: InstallConfig,
    pub firmware: Firmware,
    pub disk: Option<Disk>,
    pub encrypt: bool,
    pub keymap: String,
    pub accounts: Vec<NewAccount>,
}

/// One line of the Install screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallRow {
    Disk,
    Encrypt,
    Keyboard,
    Account(usize),
    AddAccount,
    Install,
}

impl InstallForm {
    pub fn rows(&self) -> Vec<InstallRow> {
        let mut v = vec![InstallRow::Disk, InstallRow::Encrypt, InstallRow::Keyboard];
        v.extend((0..self.accounts.len()).map(InstallRow::Account));
        if self.accounts.len() < MAX_ACCOUNTS {
            v.push(InstallRow::AddAccount);
        }
        v.push(InstallRow::Install);
        v
    }

    /// The request for resilience-install, once a disk is chosen.
    pub fn request(&self) -> Option<Request> {
        let d = self.disk.as_ref()?;
        Some(Request {
            disk: d.path.clone(),
            size: d.size,
            model: d.model.clone(),
            firmware: self.firmware,
            encrypt: self.encrypt,
            keymap: self.keymap.clone(),
            accounts: self.accounts.clone(),
        })
    }
}

/// What a text dialog of the installer is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallInput {
    Name(usize),
    FullName(usize),
    /// Type the disk's name to erase it.
    Confirm,
}

fn next_mode(m: Mode) -> Mode {
    match m {
        Mode::UltraStable => Mode::Stable,
        Mode::Stable => Mode::Advanced,
        Mode::Advanced => Mode::UltraStable,
    }
}

impl App {
    /// The installer exists on live systems only.
    pub(super) fn install_present(&self) -> bool {
        self.paths.install_config.exists()
    }

    fn install_config(&self) -> Result<InstallConfig, String> {
        let text = std::fs::read_to_string(&self.paths.install_config)
            .map_err(|e| format!("{}: {e}", self.paths.install_config.display()))?;
        InstallConfig::parse(&text).map_err(|e| format!("{INSTALL_CONFIG_PATH}: {e}"))
    }

    pub(super) fn open_install(&mut self) -> Option<View> {
        let cfg = match self.install_config() {
            Ok(c) => c,
            Err(e) => {
                self.modal = Some(Modal::Message {
                    title: "INSTALLER UNAVAILABLE".into(),
                    body: e,
                    error: true,
                });
                return None;
            }
        };
        let firmware = if self.paths.efi.exists() {
            Firmware::Uefi
        } else {
            Firmware::Bios
        };
        let current = self
            .sys
            .keyboard
            .effective(self.settings.keyboard.as_deref());
        let keymap = if cfg.keymaps.iter().any(|k| k == current) {
            current.to_string()
        } else {
            cfg.keymaps.first().cloned().unwrap_or_else(|| "us".into())
        };
        Some(View::Install {
            form: Box::new(InstallForm {
                cfg,
                firmware,
                disk: None,
                encrypt: true,
                keymap,
                accounts: install::default_accounts(),
            }),
            list: ListState::default().with_selected(Some(0)),
        })
    }

    /// Whole disks as resilience-install will see them.
    pub(super) fn load_disks(&self, cfg: &InstallConfig) -> (Vec<Disk>, Option<String>) {
        let lsblk = cfg.lsblk.to_string_lossy();
        match run_full(
            &[lsblk.as_ref(), "-P", "-b", "-o", LSBLK_COLUMNS],
            Duration::from_secs(10),
        ) {
            Some(o) if o.success() => (install::disks(&parse_pairs(&o.stdout)), None),
            Some(o) => (
                Vec::new(),
                Some(format!("lsblk failed: {}", o.stderr.trim())),
            ),
            None => (Vec::new(), Some("lsblk could not be run".into())),
        }
    }

    fn install_form_mut(&mut self) -> Option<&mut InstallForm> {
        self.stack.iter_mut().rev().find_map(|v| match v {
            View::Install { form, .. } => Some(form.as_mut()),
            _ => None,
        })
    }

    pub fn install_form(&self) -> Option<&InstallForm> {
        self.stack.iter().rev().find_map(|v| match v {
            View::Install { form, .. } => Some(form.as_ref()),
            _ => None,
        })
    }

    fn install_message(&mut self, title: &str, body: impl Into<String>, error: bool) {
        self.modal = Some(Modal::Message {
            title: title.into(),
            body: body.into(),
            error,
        });
    }

    fn ask_install(&mut self, field: InstallInput, value: String) {
        self.modal = Some(Modal::InstallText {
            field,
            value,
            error: None,
        });
    }

    /// Keys of the three installer views.
    pub(super) fn install_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        // A view opened on top of this one.
        let mut child: Option<View> = None;
        let form_cfg = self.install_form().map(|f| f.cfg.clone());
        match &mut view {
            View::Install { form, list } => {
                let rows = form.rows();
                let row = list.selected().and_then(|i| rows.get(i).copied());
                match (key.code, row) {
                    (KeyCode::Char('m'), Some(InstallRow::Account(i))) => {
                        form.accounts[i].mode = next_mode(form.accounts[i].mode);
                    }
                    (KeyCode::Char('f'), Some(InstallRow::Account(i))) => {
                        let v = form.accounts[i].description.clone();
                        self.ask_install(InstallInput::FullName(i), v);
                    }
                    (KeyCode::Char('d') | KeyCode::Delete, Some(InstallRow::Account(i))) => {
                        if form.accounts.len() > 1 {
                            form.accounts.remove(i);
                            clamp_selection(list, form.rows().len());
                        } else {
                            self.install_message(
                                "ACCOUNTS",
                                "At least one account is needed.",
                                true,
                            );
                        }
                    }
                    (KeyCode::Char('a'), _) if form.accounts.len() < MAX_ACCOUNTS => {
                        self.ask_install(InstallInput::Name(form.accounts.len()), String::new());
                    }
                    _ if is_activate(key) => match row {
                        Some(InstallRow::Disk) => {
                            let (disks, error) = self.load_disks(&form.cfg);
                            let sel = disks.iter().position(|d| d.problem.is_none()).unwrap_or(0);
                            child = Some(View::InstallDisks {
                                disks,
                                error,
                                list: ListState::default().with_selected(Some(sel)),
                            });
                        }
                        Some(InstallRow::Encrypt) => form.encrypt = !form.encrypt,
                        Some(InstallRow::Keyboard) => {
                            let sel = form.cfg.keymaps.iter().position(|k| *k == form.keymap);
                            child = Some(View::InstallLayouts {
                                list: ListState::default().with_selected(sel.or(Some(0))),
                            });
                        }
                        Some(InstallRow::Account(i)) => {
                            let v = form.accounts[i].name.clone();
                            self.ask_install(InstallInput::Name(i), v);
                        }
                        Some(InstallRow::AddAccount) => {
                            self.ask_install(
                                InstallInput::Name(form.accounts.len()),
                                String::new(),
                            );
                        }
                        Some(InstallRow::Install) => self.review_install(form),
                        None => {}
                    },
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, rows.len(), key),
                }
            }
            View::InstallDisks { disks, error, list } => {
                if is_activate(key) {
                    if let Some(d) = list.selected().and_then(|i| disks.get(i)).cloned() {
                        match &d.problem {
                            Some(why) => self.install_message(
                                "THIS DISK CANNOT BE USED",
                                format!("{}\n{}\n\n{why}.", d.path, d.describe()),
                                true,
                            ),
                            None => {
                                if let Some(form) = self.install_form_mut() {
                                    form.disk = Some(d);
                                }
                                return (None, Effect::None);
                            }
                        }
                    }
                } else if key.code == KeyCode::Char('r') {
                    if let Some(cfg) = &form_cfg {
                        (*disks, *error) = self.load_disks(cfg);
                    }
                    clamp_selection(list, disks.len());
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, disks.len(), key);
                }
            }
            View::InstallLayouts { list } => {
                let keymaps = form_cfg.map(|c| c.keymaps).unwrap_or_default();
                if is_activate(key) {
                    if let Some(k) = list.selected().and_then(|i| keymaps.get(i)).cloned() {
                        if let Some(form) = self.install_form_mut() {
                            form.keymap = k;
                        }
                        return (None, Effect::None);
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, keymaps.len(), key);
                }
            }
            _ => {}
        }
        match child {
            Some(c) => {
                self.stack.push(view);
                (Some(c), Effect::None)
            }
            None => (Some(view), Effect::None),
        }
    }

    /// "Install": everything must be ready; then the summary and the typed
    /// confirmation.
    fn review_install(&mut self, form: &InstallForm) {
        let Some(disk) = &form.disk else {
            self.install_message(
                "CHOOSE A DISK",
                "Choose the disk to install on first (the first line).",
                true,
            );
            return;
        };
        if let Some(why) = &disk.problem {
            self.install_message("THIS DISK CANNOT BE USED", why.clone(), true);
            return;
        }
        if let Err(e) = install::check_accounts(&form.accounts, &Default::default()) {
            self.install_message("ACCOUNTS", e.to_string(), true);
            return;
        }
        self.ask_install(InstallInput::Confirm, String::new());
    }

    /// The text the confirmation shows: exactly what will happen.
    pub fn install_summary(&self) -> String {
        let Some(form) = self.install_form() else {
            return String::new();
        };
        let Some(d) = &form.disk else {
            return String::new();
        };
        let mut s = format!(
            "EVERYTHING ON THIS DISK WILL BE ERASED:\n  {}  {}\n",
            d.path,
            d.describe()
        );
        if d.parts.is_empty() {
            s.push_str("  (no partitions found on it)\n");
        } else {
            s.push_str("  It holds now:\n");
            for p in d.parts.iter().take(6) {
                let what = [p.fstype.as_str(), p.label.as_str()]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ");
                s.push_str(&format!(
                    "  - {} {} {}\n",
                    p.path,
                    install::human_size(p.size),
                    if what.is_empty() { "(unknown)" } else { &what }
                ));
            }
        }
        if d.parts.len() > 6 {
            s.push_str(&format!("  - ... and {} more\n", d.parts.len() - 6));
        }
        s.push_str(&format!(
            "\nResilience {} for {}, encryption {}, keyboard {}.\nAccounts:\n",
            form.cfg.edition,
            form.firmware.label(),
            if form.encrypt { "ON" } else { "off" },
            self.sys.keyboard.label(&form.keymap)
        ));
        for a in &form.accounts {
            s.push_str(&format!("  - {} ({})\n", a.name, a.mode.label()));
        }
        if !install::has_administrator(&form.accounts) {
            s.push_str(
                "\nWARNING: no Advanced account. Nobody will be able to change modes, \
                 install packs or update this computer.\n",
            );
        }
        s.push_str(if form.encrypt {
            "\nYou will type the passphrase and the passwords yourself in the next \
             steps; they are never shown."
        } else {
            "\nYou will type the passwords yourself in the next steps; they are never \
             shown."
        });
        s
    }

    pub(super) fn install_text_key(
        &mut self,
        field: InstallInput,
        mut value: String,
        error: Option<String>,
        key: KeyEvent,
    ) -> Effect {
        let max = match field {
            InstallInput::Name(_) => 32,
            InstallInput::FullName(_) => install::MAX_DESCRIPTION,
            InstallInput::Confirm => 32,
        };
        let mut error = error;
        match key.code {
            KeyCode::Esc => return Effect::None,
            KeyCode::Enter => match self.install_submit(field, value.trim()) {
                Ok(effect) => return effect,
                Err(e) => error = Some(e),
            },
            KeyCode::Backspace => {
                value.pop();
                error = None;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if value.chars().count() < max {
                    value.push(c);
                }
                error = None;
            }
            _ => {}
        }
        self.modal = Some(Modal::InstallText {
            field,
            value,
            error,
        });
        Effect::None
    }

    fn install_submit(&mut self, field: InstallInput, value: &str) -> Result<Effect, String> {
        let Some(form) = self.install_form_mut() else {
            return Ok(Effect::None);
        };
        match field {
            InstallInput::Name(i) => {
                let mut accounts = form.accounts.clone();
                let account = NewAccount {
                    name: value.to_string(),
                    description: accounts
                        .get(i)
                        .map(|a| a.description.clone())
                        .unwrap_or_default(),
                    mode: accounts.get(i).map_or(Mode::Stable, |a| a.mode),
                };
                if i < accounts.len() {
                    accounts[i] = account;
                } else {
                    accounts.push(account);
                }
                install::check_accounts(&accounts, &Default::default())
                    .map_err(|e| e.to_string())?;
                let added = i >= form.accounts.len();
                form.accounts = accounts;
                if added {
                    // A new account: its full name next (may stay empty).
                    self.ask_install(InstallInput::FullName(i), String::new());
                }
                Ok(Effect::None)
            }
            InstallInput::FullName(i) => {
                if !install::valid_description(value) {
                    return Err(format!(
                        "At most {} characters, without : , or \\",
                        install::MAX_DESCRIPTION
                    ));
                }
                if let Some(a) = form.accounts.get_mut(i) {
                    a.description = value.to_string();
                }
                Ok(Effect::None)
            }
            InstallInput::Confirm => {
                let Some(req) = form.request() else {
                    return Ok(Effect::None);
                };
                let name = req
                    .disk
                    .strip_prefix("/dev/")
                    .unwrap_or(&req.disk)
                    .to_string();
                if value != name {
                    return Err(format!(
                        "Type {name} exactly to erase it, or Esc to go back."
                    ));
                }
                let installer = form.cfg.installer.to_string_lossy().into_owned();
                self.run_install(&installer, &req)
            }
        }
    }

    /// `pkexec resilience-install run …` in the foreground.
    fn run_install(&mut self, installer: &str, req: &Request) -> Result<Effect, String> {
        let admin = AdminConfig::read(&self.paths.admin_config)?;
        let mut argv = vec![
            admin.pkexec.to_string_lossy().into_owned(),
            installer.to_string(),
        ];
        argv.extend(install::to_args(req));
        tracing::warn!(disk = %req.disk, "installation requested");
        Ok(Effect::Run(RunRequest {
            label: INSTALL_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                admin.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "INSTALLING RESILIENCE on {}. Follow the questions below; Ctrl-C stops \
                 (the disk is then left unusable until you install again).",
                req.disk
            )),
        }))
    }
}
