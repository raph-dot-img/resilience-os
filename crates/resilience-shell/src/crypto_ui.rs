//! Security & cryptography screens (ADR 0015).
//!
//! The interface only chooses what to do, on which file and for whom; the
//! unprivileged `resilience-crypto` then does it in the foreground. age,
//! minisign and pinentry ask for passphrases on the terminal themselves, so
//! no secret ever passes through the interface. The only secrets shown here
//! are random passwords generated on request, which are never stored.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use crate::list::ListState;
use resilience_core::crypto::{self, Contact, KeyStore};
use resilience_core::openpgp::{self, PgpKey};
use resilience_core::printable;
use resilience_core::proc::run_full;

use super::{App, Builtin, Effect, Level, MediaPurpose, Modal, Pending, RunRequest, View};

pub const CRYPTO_LABEL: &str = "Cryptography";

/// What a file is being chosen for (Find & view files as a file picker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Encrypt,
    Decrypt,
    ReadMessage,
    Sign,
    Verify,
    Checksum,
    Compare,
    CheckList,
    Inspect,
    AddContact,
    /// A file to send in a local message (ADR 0020).
    SendFile,
}

impl Pick {
    pub fn title(self) -> &'static str {
        match self {
            Pick::Encrypt => "CHOOSE THE FILE TO ENCRYPT",
            Pick::Decrypt => "CHOOSE THE ENCRYPTED FILE (.age, .gpg, .asc)",
            Pick::ReadMessage => "CHOOSE THE ENCRYPTED MESSAGE",
            Pick::Sign => "CHOOSE THE FILE TO SIGN",
            Pick::Verify => "CHOOSE THE SIGNED FILE (or its .minisig, .sig, .asc)",
            Pick::Checksum | Pick::Compare => "CHOOSE THE FILE TO CHECK",
            Pick::CheckList => "CHOOSE THE CHECKSUM LIST (e.g. SHA256SUMS)",
            Pick::Inspect => "CHOOSE THE FILE TO INSPECT",
            Pick::AddContact => "CHOOSE THE PUBLIC KEY FILE (Resilience or OpenPGP)",
            Pick::SendFile => "CHOOSE THE FILE TO SEND (at most 900 KB)",
        }
    }

    /// Search text typed for the user when the picker opens.
    fn query(self) -> &'static str {
        match self {
            Pick::CheckList => "sum",
            _ => "",
        }
    }
}

/// Entries of the small menus (messages, checksums, random).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    WriteMessage,
    ReadMessage,
    ShowSums,
    CompareSum,
    CheckList,
    PassphraseEn,
    PassphraseFr,
    Password,
    HexKey,
}

pub const MESSAGE_CHOICES: [Choice; 2] = [Choice::WriteMessage, Choice::ReadMessage];
pub const SUM_CHOICES: [Choice; 3] = [Choice::ShowSums, Choice::CompareSum, Choice::CheckList];
pub const RANDOM_CHOICES: [Choice; 4] = [
    Choice::PassphraseEn,
    Choice::PassphraseFr,
    Choice::Password,
    Choice::HexKey,
];

impl Choice {
    pub fn name(self) -> &'static str {
        match self {
            Choice::WriteMessage => "Write an encrypted message",
            Choice::ReadMessage => "Read an encrypted message",
            Choice::ShowSums => "Show a file's checksums",
            Choice::CompareSum => "Compare a file with the checksum you were given",
            Choice::CheckList => "Check the files of a checksum list (SHA256SUMS...)",
            Choice::PassphraseEn => "Passphrase: 6 English words",
            Choice::PassphraseFr => "Phrase de passe : 6 mots français",
            Choice::Password => "Password: 24 characters",
            Choice::HexKey => "Random key: 64 hexadecimal digits (256 bits)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Choice::WriteMessage => {
                "Type a message; it is saved encrypted in your Messages folder as text that \
                 can be copied, printed or sent. Only the people you choose can read it."
            }
            Choice::ReadMessage => {
                "Shows an encrypted message sent to you (or encrypted with a passphrase). It \
                 is shown only, never saved decrypted."
            }
            Choice::ShowSums => {
                "Computes the SHA-256 and SHA-512 checksums: they identify the exact content \
                 of a file."
            }
            Choice::CompareSum => {
                "Paste or type the checksum published with a file (for example on a download \
                 page) to check that your copy is complete and unchanged."
            }
            Choice::CheckList => {
                "Checks every file named in a list of checksums, like the SHA256SUMS files \
                 published with software and disk images."
            }
            Choice::PassphraseEn => {
                "Words chosen at random by xkcdpass from the EFF word list: easy to remember \
                 and to type, very hard to guess."
            }
            Choice::PassphraseFr => {
                "Mots tirés au hasard par xkcdpass dans une liste française (sans accents, \
                 pour les taper facilement)."
            }
            Choice::Password => {
                "24 random letters, digits, + and / from openssl (144 bits): for a password \
                 manager or a Wi-Fi network."
            }
            Choice::HexKey => "256 random bits from openssl, as hexadecimal.",
        }
    }
}

/// A row of the "encrypt for" checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    Passphrase,
    Me,
    Contact(String),
    /// An OpenPGP key of the GnuPG keyring (can encrypt).
    OpenPgp {
        fingerprint: String,
        name: String,
    },
}

impl Recipient {
    /// Only one group per file: a passphrase, age keys, or OpenPGP keys.
    fn group(&self) -> u8 {
        match self {
            Recipient::Passphrase => 0,
            Recipient::Me | Recipient::Contact(_) => 1,
            Recipient::OpenPgp { .. } => 2,
        }
    }
}

/// What is being encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptTarget {
    File(PathBuf),
    Message,
}

/// A row of the Keys & contacts screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRow {
    Encryption,
    Signing,
    /// The user has no OpenPGP key yet (Enter or g creates one).
    NoPgpKey,
    Contact(Contact),
    Pgp(PgpKey),
}

impl App {
    fn keystore(&self) -> KeyStore {
        KeyStore::for_home(&self.home)
    }

    /// GnuPG's folder for this account.
    fn gnupg_home(&self) -> PathBuf {
        std::env::var_os("GNUPGHOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".gnupg"))
    }

    /// The OpenPGP keys of the keyring (own keys marked). Nothing is created
    /// when the user has never used GnuPG.
    pub(super) fn pgp_keys(&self) -> Vec<PgpKey> {
        let dir = self.gnupg_home();
        if !dir.is_dir() {
            return Vec::new();
        }
        let d = dir.to_string_lossy();
        let list = |what: &str| {
            run_full(
                &[
                    &self.sys.tools.gpg,
                    "--batch",
                    "--no-tty",
                    "--homedir",
                    &d,
                    "--with-colons",
                    "--fingerprint",
                    "--fingerprint",
                    what,
                ],
                Duration::from_secs(10),
            )
            .map(|o| openpgp::parse_keys(&o.stdout))
            .unwrap_or_default()
        };
        let secret: Vec<String> = list("--list-secret-keys")
            .into_iter()
            .map(|k| k.fingerprint)
            .collect();
        let mut keys = list("--list-keys");
        for k in &mut keys {
            k.secret = secret.contains(&k.fingerprint);
        }
        // Own keys first.
        keys.sort_by_key(|k| !k.secret);
        keys
    }

    /// Open one of the Security entries.
    pub(super) fn open_crypto(&mut self, b: Builtin) -> (Option<View>, Effect) {
        let choices = |title: &'static str, items: &[Choice]| View::Choices {
            title,
            items: items.to_vec(),
            list: ListState::default().with_selected(Some(0)),
        };
        let v = match b {
            Builtin::Encrypt => self.open_picker(Pick::Encrypt),
            Builtin::Decrypt => self.open_picker(Pick::Decrypt),
            Builtin::Sign => {
                if !self.keystore().has_signing_key() {
                    self.no_key_message("signing");
                    return (None, Effect::None);
                }
                self.open_picker(Pick::Sign)
            }
            Builtin::Verify => self.open_picker(Pick::Verify),
            Builtin::Inspect => self.open_picker(Pick::Inspect),
            Builtin::Messages => choices("ENCRYPTED MESSAGES", &MESSAGE_CHOICES),
            Builtin::Checksums => choices("CHECKSUMS & FILE INTEGRITY", &SUM_CHOICES),
            Builtin::Random => choices("RANDOM PASSWORDS & KEYS", &RANDOM_CHOICES),
            Builtin::Keys => View::Keys {
                rows: self.key_rows(),
                list: ListState::default().with_selected(Some(0)),
            },
            _ => return (None, Effect::None),
        };
        (Some(v), Effect::None)
    }

    fn no_key_message(&mut self, kind: &str) {
        self.modal = Some(Modal::Message {
            title: "NO KEY YET".into(),
            body: format!(
                "You have no {kind} key yet. Create your keys first: Security → Keys & \
                 contacts, then Enter on \"Your {kind} key\"."
            ),
            error: false,
        });
    }

    pub(super) fn open_picker(&mut self, pick: Pick) -> View {
        let mut v = self.open_files();
        if let View::Files {
            idx,
            query,
            list,
            pick: p,
        } = &mut v
        {
            *p = Some(pick);
            *query = pick.query().to_string();
            let n = resilience_core::files::search(idx, query).len();
            list.select((n > 0).then_some(0));
        }
        v
    }

    /// A file was chosen in the picker.
    pub(super) fn picked(&mut self, pick: Pick, path: &Path) -> Effect {
        let file = || Some(path.to_path_buf());
        match pick {
            Pick::Encrypt => {
                self.open_recipients(EncryptTarget::File(path.to_path_buf()));
                Effect::None
            }
            Pick::Decrypt => self.crypto_run("Decrypt a file", &["decrypt"], file()),
            Pick::ReadMessage => self.crypto_run("Read a message", &["read-message"], file()),
            Pick::Sign => self.crypto_run("Sign a file", &["sign"], file()),
            Pick::Verify => self.crypto_run("Check a signature", &["verify"], file()),
            Pick::Checksum => self.crypto_run("Checksums", &["checksum"], file()),
            Pick::CheckList => self.crypto_run("Check a list", &["check-list"], file()),
            Pick::Inspect => self.crypto_run("Inspect a file", &["inspect"], file()),
            Pick::AddContact => self.crypto_run("Add a contact", &["add-contact"], file()),
            Pick::SendFile => {
                self.chat_file_picked(path);
                Effect::None
            }
            Pick::Compare => {
                self.modal = Some(Modal::ExpectedSum {
                    file: path.to_path_buf(),
                    value: String::new(),
                    error: None,
                });
                Effect::None
            }
        }
    }

    /// Run `resilience-crypto ARGS [FILE]` in the foreground.
    pub(super) fn crypto_run(&self, what: &str, args: &[&str], file: Option<PathBuf>) -> Effect {
        let mut argv = vec![self.sys.tools.crypto.clone()];
        argv.extend(args.iter().map(|a| a.to_string()));
        Effect::Run(RunRequest {
            label: CRYPTO_LABEL.into(),
            argv,
            extra_args: file.into_iter().collect(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(format!("{}: press Ctrl-C to cancel.", what.to_uppercase())),
        })
    }

    pub(super) fn choose(&mut self, c: Choice) -> (Option<View>, Effect) {
        match c {
            Choice::WriteMessage => {
                self.open_recipients(EncryptTarget::Message);
                (None, Effect::None)
            }
            Choice::ReadMessage => (Some(self.open_picker(Pick::ReadMessage)), Effect::None),
            Choice::ShowSums => (Some(self.open_picker(Pick::Checksum)), Effect::None),
            Choice::CompareSum => (Some(self.open_picker(Pick::Compare)), Effect::None),
            Choice::CheckList => (Some(self.open_picker(Pick::CheckList)), Effect::None),
            Choice::PassphraseEn | Choice::PassphraseFr | Choice::Password | Choice::HexKey => {
                self.show_random(c);
                (None, Effect::None)
            }
        }
    }

    // ----- random ---------------------------------------------------------------

    fn show_random(&mut self, c: Choice) {
        let t = &self.sys.tools;
        let argv: Vec<&str> = match c {
            Choice::PassphraseEn => vec![&t.xkcdpass, "-V", "-w", "eff-long", "-n", "6"],
            // Unaccented words only: the console keyboard may lack accents.
            Choice::PassphraseFr => vec![
                &t.xkcdpass,
                "-V",
                "-w",
                "fr-freelang",
                "-n",
                "6",
                "-v",
                "[a-z]",
                "--min",
                "4",
                "--max",
                "9",
            ],
            Choice::Password => vec![&t.openssl, "rand", "-base64", "18"],
            Choice::HexKey => vec![&t.openssl, "rand", "-hex", "32"],
            _ => return,
        };
        let out = run_full(&argv, Duration::from_secs(10)).filter(|o| o.success());
        let result = out.and_then(|o| match c {
            Choice::PassphraseEn | Choice::PassphraseFr => crypto::parse_xkcdpass(&o.stdout),
            Choice::Password => Some((o.stdout.trim().to_string(), Some(144))),
            _ => Some((crypto::grouped(o.stdout.trim()), Some(256))),
        });
        // Never logged and never in the notification history.
        self.modal = Some(match result {
            Some((secret, bits)) if !secret.is_empty() => Modal::Message {
                title: "RANDOM".into(),
                body: format!(
                    "{}\n\n{}\n\nStrength: about {} bits{}. Nothing is saved: write it down \
                     or remember it now. Choose the entry again for another one.",
                    c.name(),
                    secret,
                    bits.map(|b| b.to_string()).unwrap_or_else(|| "?".into()),
                    if bits.is_some_and(|b| b >= 70) {
                        " (out of reach of guessing)"
                    } else {
                        ""
                    }
                ),
                error: false,
            },
            _ => Modal::Message {
                title: "RANDOM".into(),
                body: "The random generator could not be run.".into(),
                error: true,
            },
        });
    }

    // ----- recipients -----------------------------------------------------------

    fn open_recipients(&mut self, target: EncryptTarget) {
        let ks = self.keystore();
        let mut rows = vec![Recipient::Passphrase];
        if ks.own().encryption.is_some() {
            rows.push(Recipient::Me);
        }
        rows.extend(
            ks.contacts()
                .into_iter()
                .filter(|c| c.age.is_some())
                .map(|c| Recipient::Contact(c.name)),
        );
        rows.extend(
            self.pgp_keys()
                .into_iter()
                .filter(|k| k.can_encrypt && !k.unusable)
                .map(|k| Recipient::OpenPgp {
                    name: format!("{}{}", k.uid, if k.secret { " (me)" } else { "" }),
                    fingerprint: k.fingerprint,
                }),
        );
        let mut ticked = vec![false; rows.len()];
        ticked[0] = true;
        self.modal = Some(Modal::Recipients {
            target,
            rows,
            ticked,
            sel: 0,
            error: None,
        });
    }

    pub(super) fn recipients_key(
        &mut self,
        target: EncryptTarget,
        rows: Vec<Recipient>,
        mut ticked: Vec<bool>,
        mut sel: usize,
        key: KeyEvent,
    ) -> Effect {
        let mut error = None;
        match key.code {
            KeyCode::Esc => return Effect::None,
            KeyCode::Up | KeyCode::Char('k') => sel = sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => sel = (sel + 1).min(rows.len() - 1),
            KeyCode::Char(' ') => {
                ticked[sel] = !ticked[sel];
                // A passphrase, age keys and OpenPGP keys cannot be mixed
                // (different programs and formats): ticking one group
                // unticks the others.
                if ticked[sel] {
                    let g = rows[sel].group();
                    for (r, t) in rows.iter().zip(ticked.iter_mut()) {
                        if r.group() != g {
                            *t = false;
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let mut args: Vec<String> = Vec::new();
                for (r, _) in rows.iter().zip(&ticked).filter(|(_, t)| **t) {
                    match r {
                        Recipient::Passphrase => args.push("--passphrase".into()),
                        Recipient::Me => args.push("--me".into()),
                        Recipient::Contact(n) => args.extend(["--to".into(), n.clone()]),
                        Recipient::OpenPgp { fingerprint, .. } => {
                            args.extend(["--to-pgp".into(), fingerprint.clone()])
                        }
                    }
                }
                if args.is_empty() {
                    error = Some("Tick at least one line with Space.".to_string());
                } else {
                    let (what, op, file) = match &target {
                        EncryptTarget::File(f) => ("Encrypt a file", "encrypt", Some(f.clone())),
                        EncryptTarget::Message => ("Write a message", "write-message", None),
                    };
                    let mut all = vec![op];
                    all.extend(args.iter().map(String::as_str));
                    return self.crypto_run(what, &all, file);
                }
            }
            _ => {}
        }
        self.modal = Some(Modal::Recipients {
            target,
            rows,
            ticked,
            sel,
            error,
        });
        Effect::None
    }

    pub(super) fn expected_sum_key(
        &mut self,
        file: PathBuf,
        mut value: String,
        key: KeyEvent,
    ) -> Effect {
        match key.code {
            KeyCode::Esc => return Effect::None,
            KeyCode::Enter => match crypto::parse_expected(&value) {
                Ok(_) => {
                    return self.crypto_run(
                        "Compare with a checksum",
                        &["compare", value.trim()],
                        Some(file),
                    );
                }
                Err(e) => {
                    self.modal = Some(Modal::ExpectedSum {
                        file,
                        value,
                        error: Some(e),
                    });
                    return Effect::None;
                }
            },
            KeyCode::Backspace => {
                value.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && value.chars().count() < 300
                    && !c.is_control() =>
            {
                value.push(c);
            }
            _ => {}
        }
        self.modal = Some(Modal::ExpectedSum {
            file,
            value,
            error: None,
        });
        Effect::None
    }

    // ----- keys & contacts ------------------------------------------------------

    pub(super) fn key_rows(&self) -> Vec<KeyRow> {
        let mut rows = vec![KeyRow::Encryption, KeyRow::Signing];
        let pgp = self.pgp_keys();
        if !pgp.iter().any(|k| k.secret) {
            rows.push(KeyRow::NoPgpKey);
        }
        rows.extend(self.keystore().contacts().into_iter().map(KeyRow::Contact));
        rows.extend(pgp.into_iter().map(KeyRow::Pgp));
        rows
    }

    pub(super) fn reload_keys(&mut self) {
        let rows = self.key_rows();
        for v in self.stack.iter_mut() {
            if let View::Keys { rows: r, list } = v {
                *r = rows.clone();
                super::clamp_selection(list, r.len());
            }
        }
    }

    /// One line of the Keys & contacts list.
    pub fn key_row_text(&self, r: &KeyRow) -> String {
        let ks = self.keystore();
        match r {
            KeyRow::Encryption => match ks.own().encryption {
                Some(k) => format!(
                    "Your encryption key   age, ends with {}",
                    crypto::age_short(&k)
                ),
                None if ks.has_encryption_key() => {
                    "Your encryption key   (public part missing)".into()
                }
                None => "Your encryption key   not created yet: Enter creates your keys".into(),
            },
            KeyRow::Signing => match ks
                .own()
                .signing
                .as_deref()
                .and_then(crypto::minisign_key_id)
            {
                Some(id) => format!("Your signing key      minisign, number {id}"),
                None if ks.has_signing_key() => {
                    "Your signing key      (public part missing)".into()
                }
                None => "Your signing key      not created yet: Enter creates your keys".into(),
            },
            KeyRow::Contact(c) => {
                let mut parts = Vec::new();
                if let Some(k) = &c.age {
                    parts.push(format!("encrypt {}", crypto::age_short(k)));
                }
                if let Some(id) = c.minisign.as_deref().and_then(crypto::minisign_key_id) {
                    parts.push(format!("signs {id}"));
                }
                format!("{:<22}{}", printable(&c.name), parts.join("  "))
            }
            KeyRow::NoPgpKey => {
                "Your OpenPGP key      none (optional, for GnuPG users): g creates one".into()
            }
            KeyRow::Pgp(k) => {
                let name: String = k.uid.chars().take(21).collect();
                let short = &k.fingerprint[k.fingerprint.len().saturating_sub(8)..];
                let label = if k.secret {
                    "Your OpenPGP key   "
                } else {
                    "OpenPGP key        "
                };
                format!(
                    "{:<22}{label}...{} {}{}",
                    name,
                    &short[..4.min(short.len())],
                    &short[4.min(short.len())..],
                    if k.unusable {
                        "  (revoked/expired)"
                    } else {
                        ""
                    }
                )
            }
        }
    }

    /// Details of a row, as a page (public keys only: nothing secret).
    fn key_details(&self, r: &KeyRow) -> View {
        let ks = self.keystore();
        let own = ks.own();
        let text = match r {
            KeyRow::Encryption | KeyRow::Signing => {
                let mut s = String::from(
                    "YOUR PUBLIC KEYS\n\nThese can be given to anyone: they contain no \
                     secret. Your private keys stay in ~/.local/share/resilience/keys, \
                     encrypted with your passphrases.\n\n",
                );
                if let Some(k) = &own.encryption {
                    s.push_str(&format!(
                        "Encryption key (age): people encrypt files for you with it.\n  {k}\n  \
                         ends with {}\n\n",
                        crypto::age_short(k)
                    ));
                }
                if let Some(k) = &own.signing {
                    s.push_str(&format!(
                        "Signing key (minisign): people check your signatures with it.\n  {k}\n  \
                         key number {}\n\n",
                        crypto::minisign_key_id(k).unwrap_or_default()
                    ));
                }
                s.push_str(
                    "To give them, press e on the list to save them on a USB stick, or read \
                     them aloud. Never give your passphrases or the files in the keys folder.",
                );
                s
            }
            KeyRow::Contact(c) => {
                let mut s = format!("CONTACT: {}\n\n", printable(&c.name));
                if let Some(a) = &c.added {
                    s.push_str(&format!("Added: {a}\n\n"));
                }
                if let Some(k) = &c.age {
                    s.push_str(&format!(
                        "Encryption key (age): you can encrypt files and messages for them.\n  \
                         {k}\n  ends with {}\n\n",
                        crypto::age_short(k)
                    ));
                }
                if let Some(k) = &c.minisign {
                    s.push_str(&format!(
                        "Signing key (minisign): you can check their signatures.\n  {k}\n  key \
                         number {}\n\n",
                        crypto::minisign_key_id(k).unwrap_or_default()
                    ));
                }
                s.push_str(
                    "Check these with the person directly (in person or by phone) before \
                     relying on them. To remove this contact, press Delete on the list.",
                );
                s
            }
            KeyRow::NoPgpKey => "YOUR OPENPGP KEY\n\nYou have none. You only need one to \
                 exchange with people who use GnuPG or other OpenPGP programs. Press g on the \
                 list to create one; others' OpenPGP keys can be added with a."
                .to_string(),
            KeyRow::Pgp(k) => {
                let date = |t: Option<i64>| {
                    t.map(|t| {
                        resilience_core::admin::rfc3339_utc(t)
                            .chars()
                            .take(10)
                            .collect()
                    })
                    .unwrap_or_else(|| "never".to_string())
                };
                format!(
                    "{}OPENPGP KEY (GnuPG)\n\n{}\n\nFingerprint:\n  {}\n\nCreated: {}\nExpires: \
                     {}\nCan encrypt: {}{}\n\n{}",
                    if k.secret { "YOUR " } else { "" },
                    printable(&k.uid),
                    openpgp::grouped_fingerprint(&k.fingerprint),
                    date(k.created),
                    date(k.expires),
                    if k.can_encrypt {
                        "yes"
                    } else {
                        "no (signing only)"
                    },
                    if k.unusable {
                        "\nWARNING: this key is revoked or expired."
                    } else {
                        ""
                    },
                    if k.secret {
                        "Its secret part is in ~/.gnupg, protected by your passphrase. Press \
                         e on the list to save the public part on a USB stick."
                    } else {
                        "Compare this fingerprint with the one its owner gives you (in \
                         person, by phone, or published by a software project). Press Delete \
                         on the list to remove it."
                    }
                )
            }
        };
        View::Pager {
            title: "KEY DETAILS".into(),
            text,
            scroll: 0,
        }
    }

    /// Keys of the Keys & contacts screen. Returns the view to push and an
    /// effect; `None` view keeps this screen.
    pub(super) fn keys_key(
        &mut self,
        rows: &[KeyRow],
        sel: Option<usize>,
        key: KeyEvent,
    ) -> (Option<View>, Effect) {
        let row = sel.and_then(|i| rows.get(i));
        let ks = self.keystore();
        let complete = ks.has_encryption_key() && ks.has_signing_key();
        match key.code {
            KeyCode::Enter => match row {
                Some(KeyRow::Encryption | KeyRow::Signing) if !complete => (
                    None,
                    self.crypto_run("Create your keys", &["new-keys"], None),
                ),
                Some(KeyRow::NoPgpKey) => (
                    None,
                    self.crypto_run("Create your OpenPGP key", &["new-openpgp-key"], None),
                ),
                Some(r) => (Some(self.key_details(r)), Effect::None),
                None => (None, Effect::None),
            },
            KeyCode::Char('n') if !complete => (
                None,
                self.crypto_run("Create your keys", &["new-keys"], None),
            ),
            KeyCode::Char('g') if rows.contains(&KeyRow::NoPgpKey) => (
                None,
                self.crypto_run("Create your OpenPGP key", &["new-openpgp-key"], None),
            ),
            KeyCode::Char('a') => (Some(self.open_picker(Pick::AddContact)), Effect::None),
            KeyCode::Char('t') => (
                None,
                self.crypto_run("Add a contact", &["add-contact"], None),
            ),
            KeyCode::Char('e') => {
                let has_pgp = rows.iter().any(|r| matches!(r, KeyRow::Pgp(k) if k.secret));
                if ks.own() == crypto::OwnKeys::default() && !has_pgp {
                    self.no_key_message("encryption");
                    return (None, Effect::None);
                }
                let (volumes, error) = self.load_volumes();
                let sel = (!volumes.is_empty()).then_some(0);
                (
                    Some(View::Media {
                        volumes,
                        error,
                        list: ListState::default().with_selected(sel),
                        purpose: MediaPurpose::ExportKeys,
                    }),
                    Effect::None,
                )
            }
            KeyCode::Delete | KeyCode::Char('x') => {
                match row {
                    Some(KeyRow::Contact(c)) => self.confirm_remove_contact(&c.name),
                    Some(KeyRow::Pgp(k)) if k.secret => {
                        self.modal = Some(Modal::Message {
                            title: "YOUR OWN KEY".into(),
                            body: "Your own OpenPGP key is not deleted from here: without \
                                   it, nothing encrypted for it can be opened again."
                                .into(),
                            error: false,
                        });
                    }
                    Some(KeyRow::Pgp(k)) => {
                        self.modal = Some(Modal::Confirm {
                            title: "REMOVE AN OPENPGP KEY".into(),
                            body: format!(
                                "Remove the OpenPGP key of {} from your keyring?\n\nYou will \
                                 no longer be able to encrypt for it or check its signatures \
                                 until you add it again.",
                                printable(&k.uid)
                            ),
                            confirm: "Remove".into(),
                            action: Pending::RemovePgpKey {
                                fingerprint: k.fingerprint.clone(),
                            },
                            yes_focused: false,
                        });
                    }
                    _ => {}
                }
                (None, Effect::None)
            }
            _ => (None, Effect::None),
        }
    }

    fn confirm_remove_contact(&mut self, name: &str) {
        self.modal = Some(Modal::Confirm {
            title: "REMOVE A CONTACT".into(),
            body: format!(
                "Remove {} and their public keys from your contacts?\n\nYou will no longer \
                 be able to encrypt for them or check their signatures, until you add their \
                 keys again. Files already encrypted are not affected.",
                printable(name)
            ),
            confirm: "Remove".into(),
            action: Pending::RemoveContact {
                name: name.to_string(),
            },
            yes_focused: false,
        });
    }

    pub(super) fn remove_contact(&mut self, name: &str) {
        match self.keystore().remove_contact(name) {
            Ok(()) => self.notify(
                Level::Info,
                format!("{} was removed from your contacts.", printable(name)),
            ),
            Err(e) => self.notify(
                Level::Error,
                format!("Cannot remove {}: {e}", printable(name)),
            ),
        }
        self.reload_keys();
    }

    /// Remove a public (not own) key from the GnuPG keyring.
    pub(super) fn remove_pgp_key(&mut self, fingerprint: &str) {
        let is_own = self
            .pgp_keys()
            .iter()
            .any(|k| k.fingerprint == fingerprint && k.secret);
        if is_own || !openpgp::is_fingerprint(fingerprint) {
            return;
        }
        let d = self.gnupg_home().to_string_lossy().into_owned();
        let out = run_full(
            &[
                &self.sys.tools.gpg,
                "--batch",
                "--no-tty",
                "--yes",
                "--homedir",
                &d,
                "--delete-keys",
                fingerprint,
            ],
            Duration::from_secs(10),
        );
        match out {
            Some(o) if o.success() => self.notify(Level::Info, "The OpenPGP key was removed."),
            Some(o) => self.notify(
                Level::Error,
                format!("The key was not removed: {}", printable(o.stderr.trim())),
            ),
            None => self.notify(Level::Error, "gpg did not answer."),
        }
        self.reload_keys();
    }

    /// Export the public keys to the chosen stick.
    pub(super) fn export_to(&self, mount: &Path) -> Effect {
        self.crypto_run(
            "Export your public keys",
            &["export-keys"],
            Some(mount.to_path_buf()),
        )
    }
}
