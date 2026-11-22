//! The licence notice (ADR 0024): before an account's first use of the
//! interface, a summary of the Hippocratic License 2.1 and its original
//! text; the person opens the text, ticks "I certify that I have read",
//! and Enter asks `pkexec resilience-admin record-licence SHA256` (no
//! password for the active local session) to append the record. Nothing
//! else of the interface can be reached until then; declining logs out.

use std::time::Duration;

use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use resilience_core::admin::AdminConfig;
use resilience_core::licence::{self, LicenceSettings};
use resilience_core::proc::run_full_env;

use super::{App, Effect, Level, Modal, Pending, View, scroll_key};

/// Which page of the notice is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicencePage {
    Summary,
    Full,
}

impl App {
    /// The settings, when this system shows the notice at all.
    fn licence_settings(&self) -> Option<&LicenceSettings> {
        self.sys.licence.as_ref().filter(|l| l.prompt)
    }

    /// Whether this account has yet to certify it read this licence text.
    pub fn licence_needed(&self) -> bool {
        let Some(l) = self.licence_settings() else {
            return false;
        };
        let record = std::fs::read_to_string(&l.record).unwrap_or_default();
        !licence::has_read(&record, &self.who.user, &l.sha256)
    }

    /// The notice, opened on top of the main menu at startup if needed.
    pub fn licence_view(&self) -> Option<View> {
        if !self.licence_needed() {
            return None;
        }
        let l = self.licence_settings()?;
        let text = std::fs::read_to_string(&l.file).unwrap_or_else(|e| {
            format!(
                "The licence text could not be read ({}: {e}).\n\nThe licence is in the \
                 file LICENSE of Resilience's source code.",
                l.file.display()
            )
        });
        Some(View::Licence {
            page: LicencePage::Summary,
            text,
            scroll: 0,
            full_seen: false,
            ticked: false,
        })
    }

    /// Every key while the notice is on top: nothing else is reachable.
    pub(super) fn licence_key(&mut self, key: KeyEvent) -> Effect {
        let Some(View::Licence {
            page,
            scroll,
            full_seen,
            ticked,
            ..
        }) = self.stack.last_mut()
        else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                *page = match page {
                    LicencePage::Summary => LicencePage::Full,
                    LicencePage::Full => LicencePage::Summary,
                };
                if *page == LicencePage::Full {
                    *full_seen = true;
                }
                *scroll = 0;
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                if *full_seen {
                    *ticked = !*ticked;
                } else {
                    self.notify(
                        Level::Warn,
                        "Read the original text first: press Tab to open it.",
                    );
                }
            }
            KeyCode::Enter => {
                if *ticked {
                    self.record_licence();
                } else if *full_seen {
                    self.notify(
                        Level::Warn,
                        "Tick the box first (Space) to certify that you read the licence.",
                    );
                } else {
                    self.notify(
                        Level::Warn,
                        "Read the original text first: press Tab to open it.",
                    );
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => self.confirm_decline_licence(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => scroll_key(scroll, key),
        }
        Effect::None
    }

    fn confirm_decline_licence(&mut self) {
        self.modal = Some(Modal::Confirm {
            title: "LOG OUT".into(),
            body: "Without confirming that you have read the licence, the Resilience \
                   interface does not open. Log out now? You will be asked again at the \
                   next login."
                .into(),
            confirm: "Log out".into(),
            action: Pending::DeclineLicence,
            yes_focused: false,
        });
    }

    /// `pkexec <helper> record-licence SHA256`, then check the record.
    fn record_licence(&mut self) {
        let Some(l) = self.licence_settings().cloned() else {
            return;
        };
        let result = std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
            .and_then(|cfg| {
                let pkexec = cfg.pkexec.to_string_lossy().into_owned();
                let helper = cfg.helper.to_string_lossy().into_owned();
                let shell = cfg.session_shell.to_string_lossy().into_owned();
                match run_full_env(
                    &[&pkexec, &helper, "record-licence", &l.sha256],
                    &[("SHELL", &shell)],
                    Duration::from_secs(15),
                ) {
                    Some(o) if o.success() => Ok(()),
                    Some(o) => Err(o
                        .stderr
                        .lines()
                        .next()
                        .unwrap_or("the record was refused")
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(200)
                        .collect()),
                    None => Err("the system helper did not run or took too long".into()),
                }
            });
        match result {
            Ok(()) if !self.licence_needed() => {
                self.stack.retain(|v| !matches!(v, View::Licence { .. }));
                tracing::info!(user = %self.who.user, "licence reading recorded");
                self.notify(
                    Level::Info,
                    format!("Thank you. Your reading of the {} is recorded.", l.title()),
                );
            }
            other => {
                let why = other
                    .err()
                    .unwrap_or_else(|| "the record does not show it".to_string());
                self.modal = Some(Modal::Message {
                    title: "NOT RECORDED".into(),
                    body: format!(
                        "Your reading of the licence could not be recorded: {why}.\n\n\
                         Press Enter to try again, or q to log out."
                    ),
                    error: true,
                });
            }
        }
    }
}
