//! Application state and input handling.
//!
//! The [`App`] owns all state; [`App::handle_key`] is a pure state transition
//! that returns an [`Effect`] for the main loop to execute (running a program
//! needs the terminal to be released, which only the main loop can do). This
//! keeps all navigation logic testable without a terminal.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::input::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::list::ListState;
use resilience_core::admin::{self, AccountRow, AdminConfig, ModeState};
use resilience_core::backup::{self, BackupFile};
use resilience_core::catalog::{Availability, Catalog, Category};
use resilience_core::clock;
use resilience_core::config::{SystemConfig, ThemeName, UserSettings};
use resilience_core::handbook;
use resilience_core::human_bytes;
use resilience_core::keyboard;
use resilience_core::manpage;
use resilience_core::media::{self, Ejected, Volume};
use resilience_core::mode::{Detected, Mode};
use resilience_core::network::{self, NetworkState, ProbeConfig, ProbeReport};
use resilience_core::notes;
use resilience_core::packs::PackKind;
use resilience_core::proc::{run_full, run_full_env};
use resilience_core::recovery;
use resilience_core::status::{self, Snapshot, SystemState};
use resilience_core::wifi::{self, Network, Overview};

#[path = "chat_ui.rs"]
mod chat_ui;
#[path = "crypto_ui.rs"]
mod crypto_ui;
#[path = "files_ui.rs"]
mod files_ui;
#[path = "install_ui.rs"]
mod install_ui;
#[path = "library.rs"]
mod library;
#[path = "licence_ui.rs"]
mod licence_ui;
#[path = "share_ui.rs"]
mod share_ui;
#[path = "update_ui.rs"]
mod update_ui;
pub use chat_ui::now_secs as chat_now;
pub use chat_ui::{ChatInput, ChatRow, NearbyRow};
pub use crypto_ui::CRYPTO_LABEL;
pub use crypto_ui::{Choice, EncryptTarget, KeyRow, Pick, Recipient};
pub use files_ui::Table;
pub use install_ui::INSTALL_LABEL;
pub use install_ui::{InstallForm, InstallInput, InstallRow};
pub use library::{ItemPlace, stick_items};
pub use library::{LibItem, PackRow, PacksScreen};
pub use licence_ui::LicencePage;
pub use share_ui::SET_SHARE_LABEL as SET_SHARE_LABEL_FOR_TESTS;
pub use share_ui::{
    CommunityRow, PeerAction, PeerPack, SHARE_ROWS, ShareForm, ShareInput, service_label,
};
pub use update_ui::update_rows as update_ui_rows;
pub use update_ui::{UPDATE_APPLY_LABEL, UPDATE_CHECK_LABEL, UpdateRow};

const FAST_REFRESH: Duration = Duration::from_secs(5);
const SLOW_REFRESH: Duration = Duration::from_secs(30);
const TOAST_DURATION: Duration = Duration::from_secs(6);
const MAX_NOTICES: usize = 100;
/// Label of the run request that changes a security mode (see `after_run`).
pub const SET_MODE_LABEL: &str = "Change security mode";
pub const SET_TIME_LABEL: &str = "Set the clock";
pub const SET_DEFAULT_GENERATION_LABEL: &str = "Default generation";
/// Labels of the backup tool runs (see `after_run`).
pub const BACKUP_LABEL: &str = "Backup of your files";
pub const CHECK_LABEL: &str = "Check a backup";
pub const RESTORE_LABEL: &str = "Restore a backup";
/// Labels of the offline library runs (see `after_run`).
pub const LIBRARY_LABEL: &str = "Offline library";
pub const PACKS_LABEL: &str = "Resource packs";

/// System files the interface reads. Real paths by default; tests point
/// them into a temporary directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub admin_config: PathBuf,
    pub group_file: PathBuf,
    pub profiles: PathBuf,
    /// The Nix store; system generations must be closures in it.
    pub store: PathBuf,
    pub booted_system: PathBuf,
    pub running_system: PathBuf,
    /// Catalog of resource packs (ADR 0013); absent when packs are disabled.
    pub packs_config: PathBuf,
    /// Written by the machine's Reticulum service (ADR 0018); absent when
    /// local messaging is not enabled on this computer.
    pub comms_client: PathBuf,
    /// Written by the mesh service while the mesh is on (ADR 0021).
    pub mesh_devices: PathBuf,
    /// Written by the share unit while sharing (ADR 0025).
    pub share_state: PathBuf,
    /// The kernel's routes, to tell which addresses are on this
    /// computer's networks.
    pub route4: PathBuf,
    pub route6: PathBuf,
    /// Network interfaces.
    pub sys_net: PathBuf,
    /// The shared folders (`files`, `pages`) of the community network.
    pub shared_dir: PathBuf,
    /// Written by the hotspot unit while it is on (no secret in it).
    pub hotspot_state: PathBuf,
    /// The installer's configuration: only on live systems (ADR 0022).
    pub install_config: PathBuf,
    /// Present when the computer started with UEFI.
    pub efi: PathBuf,
}

impl Default for Paths {
    fn default() -> Paths {
        Paths {
            admin_config: admin::ADMIN_CONFIG_PATH.into(),
            group_file: "/etc/group".into(),
            profiles: recovery::PROFILES_DIR.into(),
            store: "/nix/store".into(),
            booted_system: "/run/booted-system".into(),
            running_system: "/run/current-system".into(),
            packs_config: resilience_core::packs::PACKS_CONFIG_PATH.into(),
            comms_client: "/run/resilience-reticulum/client/config".into(),
            mesh_devices: "/run/resilience-mesh/devices".into(),
            share_state: "/run/resilience-share/state.toml".into(),
            route4: "/proc/net/route".into(),
            route6: "/proc/net/ipv6_route".into(),
            sys_net: "/sys/class/net".into(),
            shared_dir: "/var/lib/resilience/shared".into(),
            hotspot_state: "/run/resilience-share/hotspot".into(),
            install_config: resilience_core::install::INSTALL_CONFIG_PATH.into(),
            efi: "/sys/firmware/efi".into(),
        }
    }
}

/// Why the list of removable volumes was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPurpose {
    Backups,
    /// Copy resource packs from it (from the Resource packs screen).
    InstallPacks,
    /// Read packs on it without installing them (from the Offline library).
    ReadPacks,
    /// Save the user's public keys on it (from Keys & contacts).
    ExportKeys,
}

/// Entries of the Recovery section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryItem {
    Health,
    Generations,
    /// Only when the system can take updates (ADR 0023).
    Update,
    Report,
    Emergency,
}

impl RecoveryItem {
    pub const ALL: [RecoveryItem; 5] = [
        RecoveryItem::Health,
        RecoveryItem::Generations,
        RecoveryItem::Update,
        RecoveryItem::Report,
        RecoveryItem::Emergency,
    ];

    pub fn name(self) -> &'static str {
        match self {
            RecoveryItem::Health => "System health & recent failures",
            RecoveryItem::Generations => "System generations (roll back)",
            RecoveryItem::Update => "Update from a USB stick",
            RecoveryItem::Report => "Export a diagnostic report",
            RecoveryItem::Emergency => "Emergency guide",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            RecoveryItem::Health => {
                "Overall state, services that failed, and recent error messages from this boot."
            }
            RecoveryItem::Generations => {
                "Every earlier version of the system is kept. Shows them, how to start one \
                 from the boot menu, and makes one the default (administrator password). \
                 Your personal files are not affected by a system rollback."
            }
            RecoveryItem::Update => {
                "Installs a newer Resilience from an update on a USB stick, after checking \
                 that it is intact and signed by a Resilience release key (administrator \
                 password). It starts at the next restart; the current version stays in \
                 the boot menu. Your files are not affected."
            }
            RecoveryItem::Report => {
                "Saves a plain-text report (status, failures, generations, interface log) in \
                 your Diagnostics folder, to show to someone who can help. Contains no passwords."
            }
            RecoveryItem::Emergency => "What to do when something is wrong: step-by-step, offline.",
        }
    }
}

/// Functions built into the interface (as opposed to catalog programs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Handbook,
    Files,
    Manual,
    Library,
    Packs,
    Map,
    MapFiles,
    Chat,
    /// The Wi-Fi mesh network (ADR 0021).
    Mesh,
    /// Sharing with nearby computers (ADR 0025).
    Community,
    Notes,
    Network,
    Wifi,
    Encrypt,
    Decrypt,
    Messages,
    Sign,
    Verify,
    Checksums,
    Inspect,
    Random,
    Keys,
    SystemStatus,
    Notifications,
    Settings,
    Clock,
    Users,
    Recovery,
    Backup,
    /// Install Resilience on this computer (live system only, ADR 0022).
    Install,
    Help,
    Shell,
    Logout,
    Reboot,
    PowerOff,
}

impl Builtin {
    pub const ALL: [Builtin; 35] = [
        Builtin::Handbook,
        Builtin::Files,
        Builtin::Manual,
        Builtin::Library,
        Builtin::Packs,
        Builtin::Map,
        Builtin::MapFiles,
        Builtin::Chat,
        Builtin::Mesh,
        Builtin::Community,
        Builtin::Notes,
        Builtin::Network,
        Builtin::Wifi,
        Builtin::Encrypt,
        Builtin::Decrypt,
        Builtin::Messages,
        Builtin::Sign,
        Builtin::Verify,
        Builtin::Checksums,
        Builtin::Inspect,
        Builtin::Random,
        Builtin::Keys,
        Builtin::SystemStatus,
        Builtin::Recovery,
        Builtin::Backup,
        Builtin::Notifications,
        Builtin::Settings,
        Builtin::Clock,
        Builtin::Users,
        Builtin::Install,
        Builtin::Help,
        Builtin::Shell,
        Builtin::Logout,
        Builtin::Reboot,
        Builtin::PowerOff,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Handbook => "Resilience Handbook",
            Builtin::Manual => "Manual pages (man)",
            Builtin::Files => "Find & view files",
            Builtin::Library => "Offline library",
            Builtin::Packs => "Resource packs: choose what to get",
            Builtin::Map => "Map",
            Builtin::MapFiles => "Maps on this computer & USB sticks",
            Builtin::Chat => "Local messages",
            Builtin::Mesh => "Mesh network (Wi-Fi without a router)",
            Builtin::Community => "Community network: share with computers nearby",
            Builtin::Notes => "Notes",
            Builtin::Network => "Network status & diagnostics",
            Builtin::Wifi => "Wi-Fi networks",
            Builtin::Encrypt => "Encrypt a file",
            Builtin::Decrypt => "Decrypt a file",
            Builtin::Messages => "Encrypted messages",
            Builtin::Sign => "Sign a file",
            Builtin::Verify => "Check a signature",
            Builtin::Checksums => "Checksums: is a file intact?",
            Builtin::Inspect => "What is this file?",
            Builtin::Random => "Random passwords & keys",
            Builtin::Keys => "Keys & contacts",
            Builtin::SystemStatus => "System status",
            Builtin::Notifications => "Notification history",
            Builtin::Settings => "Settings",
            Builtin::Clock => "Date & time",
            Builtin::Users => "Users & security modes",
            Builtin::Recovery => "Recovery & diagnostics",
            Builtin::Backup => "Backups & USB sticks",
            Builtin::Install => "Install Resilience on this computer",
            Builtin::Help => "Help & keyboard shortcuts",
            Builtin::Shell => "Terminal shell",
            Builtin::Logout => "Log out",
            Builtin::Reboot => "Restart computer",
            Builtin::PowerOff => "Power off",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Builtin::Handbook => {
                "The built-in user guide: first steps, security modes, recovery. Works fully offline."
            }
            Builtin::Files => {
                "Find a file by part of its name, in your home folder and on open USB \
                 sticks, and read it here: text, PDF, CSV tables, archive contents. Shows \
                 a file's size and checksum. Nothing is ever changed or run."
            }
            Builtin::Manual => {
                "The reference manual of every command and file of this system. Type a \
                 command name (for example ls), or a few words to search the manuals."
            }
            Builtin::Library => {
                "Read the resource packs installed on this computer (encyclopedias, \
                 dictionaries, health and practical guides), with full-text search. \
                 Works without any network."
            }
            Builtin::Packs => {
                "See every resource pack available, its size and language, and tick the \
                 ones you want: download them when online, or copy them from a USB stick. \
                 Installing or removing needs an administrator's password."
            }
            Builtin::Map => {
                "The offline map: the whole world at overview detail, and streets, \
                 places and points of interest wherever a map pack is installed. Arrows \
                 move, + and - zoom, ? shows every key, q comes back here."
            }
            Builtin::MapFiles => {
                "See which maps this computer has (built in, installed, or on an open USB \
                 stick), what each one covers and how old its data is, and open the map \
                 on one of them. Map packs are added in Resource packs."
            }
            Builtin::Chat => {
                "Write to people nearby, without the Internet: computers on the same \
                 local network find each other. Each message shows whether it is still \
                 waiting, sent, or delivered; a message for someone who is away waits \
                 and is sent when they are back."
            }
            Builtin::Mesh => {
                "When there is no Wi-Fi router, computers can reach each other directly \
                 by Wi-Fi, each one relaying for the others, so that local messages \
                 still get through. Turn it on here; turn it off to use ordinary Wi-Fi \
                 again."
            }
            Builtin::Community => {
                "Find the computers nearby that share their library, packs or pages, \
                 read them and get packs from them (checked before installing). An \
                 administrator can share this computer's resources the same way."
            }
            Builtin::Notes => {
                "Write and keep personal text notes. Notes are private files in your Notes folder."
            }
            Builtin::Network => {
                "Shows whether this computer has a network link, a route, and (on request) Internet access."
            }
            Builtin::Wifi => {
                "See nearby Wi-Fi networks and join one. Networks you join are saved for \
                 your account only; their passwords are kept by the system, not shown."
            }
            Builtin::Encrypt => {
                "Make an encrypted copy of a file (age), with a passphrase or for the keys \
                 of chosen people. The copy is saved next to the original as NAME.age."
            }
            Builtin::Decrypt => {
                "Open a file encrypted with age, with its passphrase or your key. The result \
                 goes into your Decrypted folder; nothing is overwritten."
            }
            Builtin::Messages => {
                "Write a message only chosen people can read, as text you can copy, print \
                 or send; read messages sent to you without saving them."
            }
            Builtin::Sign => {
                "Sign a file with your key (minisign), so others can check it comes from \
                 you and was not changed. Saves NAME.minisig next to it."
            }
            Builtin::Verify => {
                "Check a file's signature (NAME.minisig) against the keys of your contacts: \
                 who signed it, and whether it was changed since."
            }
            Builtin::Checksums => {
                "Compute SHA-256/SHA-512 checksums, compare a file with a published \
                 checksum, or check every file of a SHA256SUMS list."
            }
            Builtin::Inspect => {
                "Tells what kind of file this is: encrypted (age, OpenPGP, LUKS, ZIP...), \
                 a signature, a public or PRIVATE key, a checksum list, or plain."
            }
            Builtin::Random => {
                "Strong passphrases and passwords, made by the system's random generator. \
                 Shown once, never saved."
            }
            Builtin::Keys => {
                "Create your keys, give your public keys to others, and keep your \
                 contacts' public keys to encrypt for them and check their signatures."
            }
            Builtin::SystemStatus => {
                "Detailed health report: services, storage, memory, power, system generation."
            }
            Builtin::Notifications => "Messages the interface has shown during this session.",
            Builtin::Settings => {
                "Choose the colour theme and the keyboard layout. Your security mode is shown here too."
            }
            Builtin::Clock => {
                "Shows the date, time and time zone, and sets the clock when it is wrong \
                 (there is no network time). Setting it asks for an administrator password."
            }
            Builtin::Users => {
                "See each account's security mode. An administrator can change a mode here; \
                 every change asks for an administrator password and is recorded."
            }
            Builtin::Recovery => {
                "When something is wrong: system health, earlier system versions, a report \
                 to share, and the emergency guide."
            }
            Builtin::Backup => {
                "Save your files, encrypted with a passphrase, to a USB stick; check a backup; \
                 restore one into a new folder. Also ejects sticks safely."
            }
            Builtin::Install => {
                "Copy Resilience onto this computer's disk, so that your files, messages \
                 and settings are kept. Choose the disk, encryption, keyboard and accounts; \
                 nothing is changed before you confirm by typing the disk's name. The \
                 whole disk is erased."
            }
            Builtin::Help => "How to move around the interface and what each key does.",
            Builtin::Shell => "An unprivileged command-line shell. Type 'exit' to return here.",
            Builtin::Logout => "End this session.",
            Builtin::Reboot => "Restart the computer. Unsaved work in open programs is lost.",
            Builtin::PowerOff => "Shut the computer down safely.",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Builtin::Handbook | Builtin::Manual | Builtin::Library | Builtin::Packs => {
                Category::Knowledge
            }
            Builtin::Map | Builtin::MapFiles => Category::Maps,
            Builtin::Chat | Builtin::Mesh | Builtin::Community => Category::Communications,
            Builtin::Notes | Builtin::Files => Category::Productivity,
            Builtin::Network | Builtin::Wifi => Category::Networking,
            Builtin::Encrypt
            | Builtin::Decrypt
            | Builtin::Messages
            | Builtin::Sign
            | Builtin::Verify
            | Builtin::Checksums
            | Builtin::Inspect
            | Builtin::Random
            | Builtin::Keys => Category::Security,
            _ => Category::System,
        }
    }

    /// Entries that exist only on a system with resource packs.
    pub fn needs_packs(self) -> bool {
        matches!(
            self,
            Builtin::Library | Builtin::Packs | Builtin::Map | Builtin::MapFiles
        )
    }

    pub fn min_mode(self) -> Mode {
        match self {
            Builtin::Shell => Mode::Stable,
            _ => Mode::UltraStable,
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Builtin::Handbook => "help guide manual documentation docs",
            Builtin::Files => {
                "file files find search document documents pdf csv spreadsheet table \
                 archive zip tar checksum sha256 view open read"
            }
            Builtin::Manual => "man manual manpage manpages reference command commands options",
            Builtin::Library => {
                "library kiwix zim wikipedia encyclopedia dictionary wiktionary read offline \
                 medicine health travel wikivoyage articles"
            }
            Builtin::Packs => {
                "packs resource download install usb kiwix zim wikipedia encyclopedia \
                 dictionary content get add remove"
            }
            Builtin::Map => {
                "map maps world atlas country city town street road place where navigation \
                 geography openstreetmap osm"
            }
            Builtin::MapFiles => "map maps pack packs mbtiles usb stick coverage region",
            Builtin::Chat => {
                "message messages chat talk write contact contacts nearby local network \
                 mesh reticulum lxmf address offline communication"
            }
            Builtin::Mesh => {
                "mesh batman ad hoc adhoc wifi wi-fi router relay neighbourhood neighborhood \
                 network without router disaster"
            }
            Builtin::Community => {
                "community share sharing nearby neighbour neighbor library packs pages \
                 shelter school server network hotspot download"
            }
            Builtin::Notes => "write text memo journal",
            Builtin::Network => "wifi ethernet internet offline online dns",
            Builtin::Wifi => {
                "wifi wi-fi wireless wlan hotspot ssid password connect join network internet"
            }
            Builtin::Encrypt => {
                "encrypt encryption age secret protect password passphrase crypto file"
            }
            Builtin::Decrypt => "decrypt decryption age open encrypted secret crypto file",
            Builtin::Messages => "message messages encrypted secret letter text armor age crypto",
            Builtin::Sign => "sign signature minisign authenticate crypto",
            Builtin::Verify => {
                "verify signature check minisign authentic authenticity tampered crypto"
            }
            Builtin::Checksums => {
                "checksum checksums hash sha256 sha512 sha256sums md5 integrity verify \
                 compare intact corrupt download iso"
            }
            Builtin::Inspect => "inspect format identify encrypted gpg pgp openpgp luks zip what",
            Builtin::Random => "random password passphrase generate diceware key pin",
            Builtin::Keys => "keys key contact contacts public private export import age minisign",
            Builtin::SystemStatus => "health battery disk storage memory",
            Builtin::Settings => {
                "theme colour color preferences keyboard layout azerty qwerty clavier"
            }
            Builtin::Clock => "clock time date hour timezone zone ntp calendar rtc wrong",
            Builtin::Users => {
                "account accounts mode modes administrator admin permissions privileges sudo"
            }
            Builtin::Recovery => {
                "rescue repair rollback generation failure failed diagnostics report emergency broken"
            }
            Builtin::Backup => {
                "backup backups restore usb stick key removable media drive disk eject copy save sd card"
            }
            Builtin::Install => {
                "install installer installation disk hard drive ssd setup keep permanent"
            }
            Builtin::Shell => "terminal command bash console",
            Builtin::PowerOff => "shutdown halt",
            Builtin::Reboot => "reboot",
            _ => "",
        }
    }
}

pub fn category_description(c: Category) -> &'static str {
    match c {
        Category::Knowledge => "Offline guides, manuals and reference material.",
        Category::Maps => "Offline maps, places and navigation.",
        Category::Communications => "Messaging over local networks and the Internet.",
        Category::Networking => "Network status, diagnostics and configuration.",
        Category::Radio => "Receive-only radio and SDR tools.",
        Category::Security => "Encryption, signatures and integrity checks.",
        Category::Productivity => "Editors, notes, files, calculators and calendars.",
        Category::Entertainment => "Games and leisure that work offline.",
        Category::System => "Status, settings, power and session management.",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    App(usize),
    Builtin(Builtin),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Home,
    Category,
    Reader,
    Notes,
    Network,
    Settings,
    Search,
    Users,
    Recovery,
    Backup,
    Library,
    Maps,
    Chat,
    Files,
    Crypto,
    Install,
    Community,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchHit {
    Category(Category),
    Entry(Entry),
    Page(handbook::Page),
}

#[derive(Debug)]
pub enum View {
    Home {
        list: ListState,
    },
    /// The installer's answers (ADR 0022).
    Install {
        form: Box<InstallForm>,
        list: ListState,
    },
    /// Whole disks to install on.
    InstallDisks {
        disks: Vec<resilience_core::install::Disk>,
        error: Option<String>,
        list: ListState,
    },
    /// Keyboard layouts the installed system may use.
    InstallLayouts {
        list: ListState,
    },
    Category {
        cat: Category,
        list: ListState,
    },
    Handbook {
        pages: Vec<handbook::Page>,
        list: ListState,
        error: Option<String>,
    },
    Pager {
        title: String,
        text: String,
        scroll: usize,
    },
    Notes {
        notes: Vec<notes::Note>,
        list: ListState,
    },
    Network {
        scroll: usize,
    },
    Status {
        scroll: usize,
    },
    Settings {
        list: ListState,
    },
    Search {
        query: String,
        hits: Vec<SearchHit>,
        list: ListState,
    },
    Users {
        rows: Vec<AccountRow>,
        error: Option<String>,
        list: ListState,
    },
    ChooseMode {
        row: AccountRow,
        list: ListState,
    },
    Recovery {
        list: ListState,
    },
    /// System generations, newest first; Enter makes one the default.
    Generations {
        gens: Vec<recovery::Generation>,
        list: ListState,
    },
    /// The licence notice (ADR 0024), on top of everything until recorded.
    Licence {
        page: LicencePage,
        text: String,
        scroll: usize,
        /// The original text was opened.
        full_seen: bool,
        /// "I certify that I have read" is ticked.
        ticked: bool,
    },
    /// Removable volumes and the updates on them (ADR 0023).
    Updates {
        rows: Vec<UpdateRow>,
        error: Option<String>,
        list: ListState,
    },
    /// Removable volumes, opened for `purpose`.
    Media {
        volumes: Vec<Volume>,
        error: Option<String>,
        list: ListState,
        purpose: MediaPurpose,
    },
    /// File search: the index is built when the view opens. With `pick`,
    /// Enter chooses the file for a cryptography operation instead.
    Files {
        idx: Box<resilience_core::files::Index>,
        query: String,
        list: ListState,
        pick: Option<Pick>,
    },
    /// A small menu (messages, checksums, random).
    Choices {
        title: &'static str,
        items: Vec<Choice>,
        list: ListState,
    },
    /// The user's keys and their contacts.
    Keys {
        rows: Vec<KeyRow>,
        list: ListState,
    },
    /// A CSV/TSV file.
    Table(Box<Table>),
    /// Suggested selections of packs, over the packs screen.
    PackSets {
        list: ListState,
    },
    /// Installed resource packs of one kind; Enter reads one (ZIM) or
    /// opens the map on it (map packs).
    Library {
        kind: PackKind,
        items: Vec<LibItem>,
        list: ListState,
    },
    /// Community network (ADR 0025): this computer's sharing, then the
    /// computers heard sharing.
    Community {
        rows: Vec<CommunityRow>,
        list: ListState,
    },
    /// What can be done with one nearby computer.
    PeerMenu {
        name: String,
        peer: resilience_core::share::Peer,
        actions: Vec<PeerAction>,
        list: ListState,
    },
    /// The catalog packs a nearby computer offers; Space ticks.
    PeerPacks {
        peer: resilience_core::share::Peer,
        rows: Vec<PeerPack>,
        ticked: Vec<bool>,
        list: ListState,
    },
    /// The files of a nearby computer's shared folder.
    PeerFiles {
        peer: resilience_core::share::Peer,
        rows: Vec<resilience_core::share::SharedFile>,
        list: ListState,
    },
    /// The choices before sharing this computer's resources.
    ShareSetup {
        form: ShareForm,
        list: ListState,
    },
    /// Local messages: one line per person written to or heard from.
    Chats {
        rows: Vec<ChatRow>,
        status: resilience_core::messages::Status,
        list: ListState,
    },
    /// People heard on the local network.
    Nearby {
        rows: Vec<NearbyRow>,
        list: ListState,
    },
    /// The messages exchanged with one address.
    Chat {
        peer: String,
        name: resilience_core::messages::Name,
        records: Vec<resilience_core::messages::Record>,
        list: ListState,
    },
    /// The catalog of resource packs with the user's ticks.
    Packs {
        s: Box<PacksScreen>,
        list: ListState,
    },
    /// Nearby Wi-Fi networks (NetworkManager, as the user).
    Wifi {
        overview: Option<Overview>,
        error: Option<String>,
        list: ListState,
    },
    /// Backups on one mounted volume; row 0 is "make a new backup".
    Backups {
        volume: Volume,
        mount: PathBuf,
        files: Vec<BackupFile>,
        list: ListState,
    },
    Help {
        topic: HelpTopic,
        scroll: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Logout,
    Reboot,
    PowerOff,
    /// Turn the mesh network on or off (ADR 0021).
    SetMesh {
        on: bool,
    },
    /// `set-share` with these arguments (ADR 0025).
    SetShare {
        args: Vec<String>,
    },
    /// Get packs from a nearby computer (ADR 0025).
    PeerPacks {
        peer: String,
        get: Vec<String>,
    },
    /// Turn the hotspot on or off (ADR 0025 §6).
    SetHotspot {
        on: bool,
    },
    /// Copy a file of a nearby computer's shared folder into ~/Received.
    PeerGet {
        peer: String,
        name: String,
    },
    SetMode {
        user: String,
        to: Mode,
    },
    /// Seconds since the epoch, validated by `clock::check_bounds`.
    SetTime {
        utc: i64,
    },
    /// Make this generation the default at every boot.
    SetDefaultGeneration {
        number: u64,
    },
    /// Log out without certifying having read the licence (ADR 0024).
    DeclineLicence,
    /// Install the update in this bundle folder (ADR 0023).
    ApplyUpdate {
        dir: PathBuf,
    },
    Backup {
        mount: PathBuf,
    },
    /// Join an open (unencrypted) network after a warning.
    JoinWifi {
        net: Network,
    },
    ForgetWifi {
        name: String,
        uuid: String,
    },
    Restore {
        file: PathBuf,
    },
    /// Install (`get`, pack ids) and remove (`remove`, file names) packs,
    /// downloading or copying from the stick mounted at `from`.
    ChangePacks {
        get: Vec<String>,
        remove: Vec<String>,
        from: Option<PathBuf>,
    },
    RemoveContact {
        name: String,
    },
    RemovePgpKey {
        fingerprint: String,
    },
}

#[derive(Debug)]
pub enum Modal {
    Confirm {
        title: String,
        body: String,
        confirm: String,
        action: Pending,
        yes_focused: bool,
    },
    Message {
        title: String,
        body: String,
        error: bool,
    },
    NewNote {
        value: String,
        error: Option<String>,
    },
    /// Type a Wi-Fi password (shown as stars). `uuid` is set when the
    /// profile already exists (its saved password was refused).
    WifiPassword {
        net: Network,
        uuid: Option<String>,
        value: String,
        error: Option<String>,
    },
    /// A manual page name, or words to search the manuals for.
    ManPage {
        value: String,
        error: Option<String>,
    },
    /// Words to search for in one pack.
    PackSearch { item: LibItem, value: String },
    /// Community network: an address, or the name to share under.
    ShareInput {
        kind: ShareInput,
        value: String,
        error: Option<String>,
    },
    /// Local messages: an address, a message or a contact's name.
    ChatInput {
        kind: ChatInput,
        value: String,
        error: Option<String>,
    },
    /// Who to encrypt for: Space ticks, Enter goes on.
    Recipients {
        target: EncryptTarget,
        rows: Vec<Recipient>,
        ticked: Vec<bool>,
        sel: usize,
        error: Option<String>,
    },
    /// The checksum a file should have.
    ExpectedSum {
        file: PathBuf,
        value: String,
        error: Option<String>,
    },
    /// Type a new local date and time.
    SetTime {
        value: String,
        zone: String,
        error: Option<String>,
    },
    /// The installer: an account name, a full name, or the disk's name.
    InstallText {
        field: InstallInput,
        value: String,
        error: Option<String>,
    },
}

/// A program to run in the foreground with the terminal released.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunRequest {
    pub label: String,
    pub argv: Vec<String>,
    pub extra_args: Vec<PathBuf>,
    /// Environment overrides for the child (most runs: none).
    pub env: Vec<(String, String)>,
    /// For pkexec commands: polkit text agent to register first.
    pub polkit_agent: Option<String>,
    /// Start from an empty environment (only TERM, LANG, HOME, USER, a fixed
    /// PATH, then `env`), e.g. for `man`, whose pager and formatter read many
    /// variables.
    pub clean_env: bool,
    pub pause_after: bool,
    pub banner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Run(RunRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub level: Level,
    pub text: String,
    pub at: SystemTime,
}

/// Everything the interface knows about the machine.
pub struct Probe {
    pub config: ProbeConfig,
    pub report: Option<ProbeReport>,
    pub running: Option<(mpsc::Receiver<ProbeReport>, Instant)>,
}

pub struct App {
    pub sys: SystemConfig,
    pub catalog: Catalog,
    pub who: Detected,
    pub home: PathBuf,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
    pub paths: Paths,
    pub theme: ThemeName,
    pub snapshot: Snapshot,
    pub sys_state: SystemState,
    pub failed_units: Vec<String>,
    pub net: NetworkState,
    pub probe: Probe,
    pub stack: Vec<View>,
    pub modal: Option<Modal>,
    pub notices: Vec<Notice>,
    pub toast: Option<(usize, Instant)>,
    /// The Wi-Fi connection attempt running in the background, if any.
    pub wifi_job: Option<WifiJob>,
    pub tick: u64,
    last_fast: Instant,
    last_slow: Instant,
}

/// A Wi-Fi connection attempt (nmcli in a thread, up to about a minute).
pub struct WifiJob {
    pub ssid: String,
    pub uuid: String,
    /// The profile was created for this attempt: remove it if it fails.
    pub new_profile: bool,
    /// A password was typed for this attempt.
    pub with_password: bool,
    pub started: Instant,
    rx: mpsc::Receiver<Result<(), String>>,
}

/// Inputs gathered by `main` before the interface starts.
pub struct Startup {
    pub sys: SystemConfig,
    pub catalog: Catalog,
    pub who: Detected,
    pub home: PathBuf,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
    pub paths: Paths,
    pub warnings: Vec<String>,
}

/// Gather configuration. Problems become warnings shown inside the
/// interface: a broken catalog must not prevent the interface from starting.
impl Startup {
    pub fn load() -> Startup {
        let mut warnings = Vec::new();
        let sys = match SystemConfig::load() {
            Ok((c, path, found)) => {
                if !found {
                    warnings.push(format!(
                        "{} not found; using built-in defaults",
                        path.display()
                    ));
                }
                c
            }
            Err(e) => {
                warnings.push(format!("{e}; using built-in defaults"));
                SystemConfig::default()
            }
        };
        let catalog = match Catalog::load(&sys.catalog) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("application catalog unavailable: {e}"));
                Catalog::default()
            }
        };
        let home = resilience_core::config::home_dir();
        let settings_path = UserSettings::path(&resilience_core::config::config_home(&home));
        let (settings, warn) = UserSettings::load(&settings_path);
        warnings.extend(warn);
        Startup {
            sys,
            catalog,
            who: resilience_core::mode::detect(),
            home,
            settings,
            settings_path,
            paths: Paths::default(),
            warnings,
        }
    }
}

impl App {
    pub fn new(s: Startup) -> App {
        let (probe_config, invalid) = s.sys.probe.to_probe_config();
        let mut app = App {
            theme: s.settings.theme,
            snapshot: status::snapshot(&s.home),
            sys_state: SystemState::Unknown("not checked yet".into()),
            failed_units: Vec::new(),
            net: network::passive_state(),
            probe: Probe {
                config: probe_config,
                report: None,
                running: None,
            },
            stack: vec![View::Home {
                list: ListState::default().with_selected(Some(0)),
            }],
            modal: None,
            notices: Vec::new(),
            toast: None,
            wifi_job: None,
            tick: 0,
            last_fast: Instant::now(),
            last_slow: Instant::now(),
            sys: s.sys,
            catalog: s.catalog,
            who: s.who,
            home: s.home,
            settings: s.settings,
            settings_path: s.settings_path,
            paths: s.paths,
        };
        app.refresh_slow();
        if let Some(v) = app.licence_view() {
            app.stack.push(v);
        }
        for w in s.warnings {
            app.notify(Level::Warn, w);
        }
        for t in invalid {
            app.notify(Level::Warn, format!("ignored invalid probe target '{t}'"));
        }
        app
    }

    pub fn mode(&self) -> Mode {
        self.who.mode
    }

    // ----- periodic work ---------------------------------------------------

    pub fn refresh_fast(&mut self) {
        self.snapshot = status::snapshot(&self.home);
        self.net = network::passive_state();
        if matches!(self.view(), View::Wifi { .. }) {
            self.reload_wifi();
        }
        self.last_fast = Instant::now();
    }

    pub fn refresh_slow(&mut self) {
        self.sys_state = status::system_state();
        self.failed_units = status::failed_units();
        self.last_slow = Instant::now();
    }

    /// Called roughly four times per second by the main loop.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.last_fast.elapsed() >= FAST_REFRESH {
            self.refresh_fast();
        }
        if self.last_slow.elapsed() >= SLOW_REFRESH {
            self.refresh_slow();
        }
        self.poll_wifi_job();
        self.chat_tick();
        self.community_tick();
        if let Some((rx, _)) = &self.probe.running {
            match rx.try_recv() {
                Ok(report) => {
                    self.probe.running = None;
                    let ok = report.internet.is_ok();
                    self.probe.report = Some(report);
                    if ok {
                        self.notify(Level::Info, "Internet probe succeeded.");
                    } else {
                        self.notify(
                            Level::Warn,
                            "Internet is not reachable (details in Network status).",
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.probe.running = None;
                    self.notify(Level::Error, "Network probe stopped unexpectedly.");
                }
            }
        }
        if self
            .toast
            .is_some_and(|(_, t)| t.elapsed() >= TOAST_DURATION)
        {
            self.toast = None;
        }
    }

    pub fn start_probe(&mut self) {
        if self.probe.running.is_some() {
            return;
        }
        self.net = network::passive_state();
        let (tx, rx) = mpsc::channel();
        let cfg = self.probe.config.clone();
        let level = self.net.level;
        thread::spawn(move || {
            let _ = tx.send(network::probe(&cfg, level));
        });
        self.probe.running = Some((rx, Instant::now()));
        tracing::info!(?level, "network probe started");
    }

    pub fn notify(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        match level {
            Level::Info => tracing::info!(%text, "notice"),
            Level::Warn => tracing::warn!(%text, "notice"),
            Level::Error => tracing::error!(%text, "notice"),
        }
        self.notices.push(Notice {
            level,
            text,
            at: SystemTime::now(),
        });
        if self.notices.len() > MAX_NOTICES {
            self.notices.remove(0);
        }
        self.toast = Some((self.notices.len() - 1, Instant::now()));
    }

    pub fn current_toast(&self) -> Option<&Notice> {
        self.toast.and_then(|(i, _)| self.notices.get(i))
    }

    // ----- menu model -------------------------------------------------------

    /// Entries offered in a category for the current mode: built-ins first.
    pub fn entries(&self, cat: Category) -> Vec<Entry> {
        let mode = self.mode();
        let packs = self.paths.packs_config.exists();
        Builtin::ALL
            .iter()
            .filter(|b| b.category() == cat && mode.allows(b.min_mode()))
            .filter(|b| packs || !b.needs_packs())
            .filter(|b| self.builtin_present(**b))
            .map(|b| Entry::Builtin(*b))
            .chain(
                self.catalog
                    .in_category(cat, mode)
                    .map(|(i, _)| Entry::App(i)),
            )
            .collect()
    }

    /// Categories with at least one entry. Empty categories are not shown,
    /// so the menu never contains an item that does nothing.
    pub fn home_categories(&self) -> Vec<Category> {
        Category::ALL
            .into_iter()
            .filter(|c| !self.entries(*c).is_empty())
            .collect()
    }

    pub fn entry_name(&self, e: Entry) -> String {
        match e {
            Entry::App(i) => self.catalog.apps[i].name.clone(),
            Entry::Builtin(b) => b.name().to_string(),
        }
    }

    pub fn entry_ready(&self, e: Entry) -> bool {
        match e {
            Entry::App(i) => matches!(
                self.catalog.apps[i].availability(self.mode()),
                Availability::Ready(_)
            ),
            Entry::Builtin(_) => true,
        }
    }

    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mode = self.mode();
        let mut hits: Vec<SearchHit> = self
            .home_categories()
            .into_iter()
            .filter(|c| c.title().to_lowercase().contains(&q))
            .map(SearchHit::Category)
            .collect();
        let packs = self.paths.packs_config.exists();
        hits.extend(
            Builtin::ALL
                .iter()
                .filter(|b| mode.allows(b.min_mode()))
                .filter(|b| packs || !b.needs_packs())
                .filter(|b| self.builtin_present(**b))
                .filter(|b| {
                    [b.name(), b.description(), b.keywords()]
                        .iter()
                        .any(|s| s.to_lowercase().contains(&q))
                })
                .map(|b| SearchHit::Entry(Entry::Builtin(*b))),
        );
        hits.extend(
            self.catalog
                .apps
                .iter()
                .enumerate()
                .filter(|(_, a)| mode.allows(a.min_mode) && a.matches(&q))
                .map(|(i, _)| SearchHit::Entry(Entry::App(i))),
        );
        if let Ok(pages) = handbook::list(&self.sys.handbook_dir) {
            hits.extend(
                pages
                    .into_iter()
                    .filter(|p| p.title.to_lowercase().contains(&q))
                    .map(SearchHit::Page),
            );
        }
        // Names that start with the query first, then names that contain
        // it, then matches in descriptions only ("recovery" opens Recovery
        // & diagnostics, not the Handbook whose description mentions it).
        let rank = |h: &SearchHit| {
            let name = match h {
                SearchHit::Category(c) => c.title().to_lowercase(),
                SearchHit::Entry(Entry::Builtin(b)) => b.name().to_lowercase(),
                SearchHit::Entry(Entry::App(i)) => self.catalog.apps[*i].name.to_lowercase(),
                SearchHit::Page(p) => p.title.to_lowercase(),
            };
            if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else {
                2
            }
        };
        hits.sort_by_key(|h| rank(h));
        hits
    }

    // ----- navigation -------------------------------------------------------

    /// The selection of the current view, for the views a pointer can
    /// choose rows in (the GUI, ADR 0026).
    pub fn list_mut(&mut self) -> Option<&mut ListState> {
        match self.stack.last_mut()? {
            View::Home { list }
            | View::Category { list, .. }
            | View::Search { list, .. }
            | View::Handbook { list, .. }
            | View::Files { list, .. }
            | View::Library { list, .. }
            | View::Chats { list, .. }
            | View::Nearby { list, .. }
            | View::Chat { list, .. }
            | View::Wifi { list, .. }
            | View::Recovery { list }
            | View::Users { list, .. }
            | View::Media { list, .. }
            | View::Generations { list, .. }
            | View::Notes { list, .. }
            | View::Updates { list, .. }
            | View::Backups { list, .. }
            | View::Community { list, .. }
            | View::PeerMenu { list, .. }
            | View::ChooseMode { list, .. }
            | View::Choices { list, .. }
            | View::Packs { list, .. }
            | View::PackSets { list }
            | View::PeerPacks { list, .. }
            | View::PeerFiles { list, .. }
            | View::ShareSetup { list, .. }
            | View::Keys { list, .. }
            | View::Settings { list } => Some(list),
            _ => None,
        }
    }

    /// The selected row of the current view (see [`App::list_mut`]).
    pub fn selected_row(&self) -> Option<usize> {
        match self.view() {
            View::Home { list }
            | View::Category { list, .. }
            | View::Search { list, .. }
            | View::Handbook { list, .. }
            | View::Files { list, .. }
            | View::Library { list, .. }
            | View::Chats { list, .. }
            | View::Nearby { list, .. }
            | View::Chat { list, .. }
            | View::Wifi { list, .. }
            | View::Recovery { list }
            | View::Users { list, .. }
            | View::Media { list, .. }
            | View::Generations { list, .. }
            | View::Notes { list, .. }
            | View::Updates { list, .. }
            | View::Backups { list, .. }
            | View::Community { list, .. }
            | View::PeerMenu { list, .. }
            | View::ChooseMode { list, .. }
            | View::Choices { list, .. }
            | View::Packs { list, .. }
            | View::PackSets { list }
            | View::PeerPacks { list, .. }
            | View::PeerFiles { list, .. }
            | View::ShareSetup { list, .. }
            | View::Keys { list, .. }
            | View::Settings { list } => list.selected(),
            _ => None,
        }
    }

    /// The scroll position of a text page (the renderer clamps it to the
    /// lines it can show, as the TUI's does).
    pub fn scroll_mut(&mut self) -> Option<&mut usize> {
        match self.stack.last_mut()? {
            View::Pager { scroll, .. }
            | View::Status { scroll }
            | View::Network { scroll }
            | View::Help { scroll, .. } => Some(scroll),
            _ => None,
        }
    }

    /// Selects row `i` of the current view and activates it, exactly as
    /// moving there and pressing Enter would.
    pub fn choose_row(&mut self, i: usize) -> Effect {
        if self.modal.is_some() {
            return Effect::None;
        }
        match self.list_mut() {
            Some(l) => l.select(Some(i)),
            None => return Effect::None,
        }
        self.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    }

    /// Back to the main menu, as repeated Esc would (never past the
    /// licence notice, which takes Esc itself).
    pub fn go_home(&mut self) {
        while self.stack.len() > 1 && self.modal.is_none() {
            if matches!(self.view(), View::Licence { .. }) {
                return;
            }
            let n = self.stack.len();
            self.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            if self.stack.len() >= n {
                return;
            }
        }
    }

    /// Opens Search (if needed) with `query` typed in it.
    pub fn set_search(&mut self, query: &str) {
        if self.modal.is_some() || matches!(self.view(), View::Licence { .. }) {
            return;
        }
        let q: String = query.chars().take(64).collect();
        let hits = self.search(&q);
        let list = ListState::default().with_selected((!hits.is_empty()).then_some(0));
        let v = View::Search {
            query: q,
            hits,
            list,
        };
        match self.stack.last_mut() {
            Some(top @ View::Search { .. }) => *top = v,
            _ => self.push(v),
        }
    }

    pub fn view(&self) -> &View {
        self.stack.last().expect("view stack is never empty")
    }

    fn push(&mut self, v: View) {
        self.stack.push(v);
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn help_topic(&self) -> HelpTopic {
        match self.view() {
            View::Home { .. } => HelpTopic::Home,
            View::Category { .. } => HelpTopic::Category,
            View::Handbook { .. }
            | View::Pager { .. }
            | View::Status { .. }
            | View::Help { .. }
            | View::Licence { .. } => HelpTopic::Reader,
            View::Notes { .. } => HelpTopic::Notes,
            View::Chats { .. } | View::Nearby { .. } | View::Chat { .. } => HelpTopic::Chat,
            View::Community { .. }
            | View::PeerMenu { .. }
            | View::PeerPacks { .. }
            | View::PeerFiles { .. }
            | View::ShareSetup { .. } => HelpTopic::Community,
            View::Network { .. } => HelpTopic::Network,
            View::Settings { .. } => HelpTopic::Settings,
            View::Users { .. } | View::ChooseMode { .. } => HelpTopic::Users,
            View::Install { .. } | View::InstallDisks { .. } | View::InstallLayouts { .. } => {
                HelpTopic::Install
            }
            View::Recovery { .. } | View::Generations { .. } | View::Updates { .. } => {
                HelpTopic::Recovery
            }
            View::Media {
                purpose: MediaPurpose::ExportKeys,
                ..
            } => HelpTopic::Crypto,
            View::Media { .. } | View::Backups { .. } => HelpTopic::Backup,
            View::Wifi { .. } => HelpTopic::Network,
            View::Files { pick: None, .. } | View::Table(_) => HelpTopic::Files,
            View::Files { .. } | View::Choices { .. } | View::Keys { .. } => HelpTopic::Crypto,
            View::Library {
                kind: PackKind::Map,
                ..
            } => HelpTopic::Maps,
            View::Library { .. } | View::Packs { .. } | View::PackSets { .. } => HelpTopic::Library,
            View::Search { .. } => HelpTopic::Search,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.kind != KeyEventKind::Press {
            return Effect::None;
        }
        if let Some(modal) = self.modal.take() {
            return self.handle_modal(modal, key);
        }
        if matches!(self.view(), View::Licence { .. }) {
            return self.licence_key(key);
        }
        let typing = matches!(self.view(), View::Search { .. } | View::Files { .. });
        match key.code {
            KeyCode::F(1) => {
                let topic = self.help_topic();
                if !matches!(self.view(), View::Help { .. }) {
                    self.push(View::Help { topic, scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::Char('?') if !typing => {
                let topic = self.help_topic();
                if !matches!(self.view(), View::Help { .. }) {
                    self.push(View::Help { topic, scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::F(5) => {
                self.refresh_fast();
                self.refresh_slow();
                if !matches!(self.view(), View::Status { .. }) {
                    self.push(View::Status { scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::Esc => {
                self.back();
                return Effect::None;
            }
            KeyCode::Char('/') if !typing => {
                self.push(View::Search {
                    query: String::new(),
                    hits: Vec::new(),
                    list: ListState::default(),
                });
                return Effect::None;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl-L: the main loop redraws every frame anyway; refresh data.
                self.refresh_fast();
                return Effect::None;
            }
            // Ctrl-C must never drop the user out of the interface.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Effect::None;
            }
            _ => {}
        }

        let view = self.stack.pop().expect("view stack is never empty");
        let (view, effect) = self.handle_view_key(view, key);
        if let Some(v) = view {
            self.stack.push(v);
        }
        if self.stack.is_empty() {
            self.stack.push(View::Home {
                list: ListState::default().with_selected(Some(0)),
            });
        }
        effect
    }

    /// Handle a key for the top view. The view has been popped from the
    /// stack: returning `Some(view)` keeps it on top, `None` means it was
    /// either closed or already pushed back underneath a newly opened view.
    fn handle_view_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        if matches!(
            view,
            View::Chats { .. } | View::Nearby { .. } | View::Chat { .. }
        ) {
            return self.chat_key(view, key);
        }
        if matches!(
            view,
            View::Install { .. } | View::InstallDisks { .. } | View::InstallLayouts { .. }
        ) {
            return self.install_key(view, key);
        }
        if matches!(view, View::Updates { .. }) {
            return self.updates_key(view, key);
        }
        if matches!(
            view,
            View::Community { .. }
                | View::PeerMenu { .. }
                | View::PeerPacks { .. }
                | View::PeerFiles { .. }
                | View::ShareSetup { .. }
        ) {
            return self.community_key(view, key);
        }
        let mut next: Option<View> = None;
        let mut effect = Effect::None;
        match &mut view {
            // Handled by chat_key and install_key above.
            View::Chats { .. } | View::Nearby { .. } | View::Chat { .. } => {}
            View::Install { .. } | View::InstallDisks { .. } | View::InstallLayouts { .. } => {}
            View::Updates { .. } => {}
            View::Community { .. }
            | View::PeerMenu { .. }
            | View::PeerPacks { .. }
            | View::PeerFiles { .. }
            | View::ShareSetup { .. } => {}
            // Handled by licence_key in handle_key.
            View::Licence { .. } => {}
            View::Home { list } => {
                let cats = self.home_categories();
                if let Some(i) = digit(key) {
                    if i < cats.len() {
                        list.select(Some(i));
                        next = Some(category_view(cats[i]));
                    }
                } else if is_activate(key) {
                    if let Some(c) = list.selected().and_then(|i| cats.get(i)) {
                        next = Some(category_view(*c));
                    }
                } else if key.code == KeyCode::Char('q') {
                    self.confirm(Pending::Logout);
                } else {
                    nav(list, cats.len(), key);
                }
            }
            View::Category { cat, list } => {
                let entries = self.entries(*cat);
                let chosen = if let Some(i) = digit(key) {
                    (i < entries.len()).then(|| {
                        list.select(Some(i));
                        entries[i]
                    })
                } else if is_activate(key) {
                    list.selected().and_then(|i| entries.get(i).copied())
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, entries.len(), key);
                    None
                };
                if let Some(e) = chosen {
                    let (v, eff) = self.activate(e);
                    next = v;
                    effect = eff;
                }
            }
            View::Handbook { pages, list, .. } => {
                if is_activate(key) {
                    if let Some(p) = list.selected().and_then(|i| pages.get(i)).cloned() {
                        next = Some(self.open_page(&p));
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, pages.len(), key);
                }
            }
            View::Pager { scroll, .. } | View::Status { scroll } | View::Help { scroll, .. } => {
                if is_back(key) {
                    return (None, Effect::None);
                }
                scroll_key(scroll, key);
            }
            View::Network { scroll } => match key.code {
                KeyCode::Char('p') => self.start_probe(),
                KeyCode::Char('r') => {
                    self.refresh_fast();
                    self.notify(Level::Info, "Network status refreshed.");
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => scroll_key(scroll, key),
            },
            View::Notes {
                notes: list_notes,
                list,
            } => match key.code {
                KeyCode::Char('n') => {
                    self.modal = Some(Modal::NewNote {
                        value: String::new(),
                        error: None,
                    })
                }
                KeyCode::Char('r') => {
                    *list_notes = notes::list(&notes::notes_dir(&self.home));
                    clamp_selection(list, list_notes.len());
                }
                _ if is_activate(key) => {
                    if let Some(n) = list.selected().and_then(|i| list_notes.get(i)) {
                        effect = self.edit_note(n.path.clone());
                    }
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, list_notes.len(), key),
            },
            View::Settings { list } => {
                let themes = ThemeName::ALL.len();
                if is_activate(key) {
                    match list.selected() {
                        Some(i) if i < themes => self.set_theme(ThemeName::ALL[i]),
                        Some(i) => {
                            if let Some(l) = self.sys.keyboard.layouts.get(i - themes) {
                                let id = l.id.clone();
                                self.set_layout(&id);
                            }
                        }
                        None => {}
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, themes + self.sys.keyboard.layouts.len(), key);
                }
            }
            View::Users { rows, error, list } => {
                if is_activate(key) {
                    if let Some(r) = list.selected().and_then(|i| rows.get(i)).cloned() {
                        next = self.choose_mode(r);
                    }
                } else if key.code == KeyCode::Char('r') {
                    let (r, e) = self.load_accounts();
                    *rows = r;
                    *error = e;
                    clamp_selection(list, rows.len());
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, rows.len(), key);
                }
            }
            View::ChooseMode { row, list } => {
                if is_activate(key) {
                    if let Some(m) = list.selected().and_then(|i| MODES.get(i)) {
                        self.confirm_set_mode(row, *m);
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, MODES.len(), key);
                }
            }
            View::Recovery { list } => {
                let items = self.recovery_items();
                let chosen = if let Some(i) = digit(key) {
                    (i < items.len()).then(|| {
                        list.select(Some(i));
                        items[i]
                    })
                } else if is_activate(key) {
                    list.selected().and_then(|i| items.get(i).copied())
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, items.len(), key);
                    None
                };
                if let Some(item) = chosen {
                    next = self.open_recovery(item);
                }
            }
            View::Media {
                volumes,
                error,
                list,
                purpose,
            } => {
                let selected = list.selected().and_then(|i| volumes.get(i)).cloned();
                if *purpose != MediaPurpose::Backups && is_activate(key) {
                    if let Some(v) = selected
                        && let Some(mount) = self.mount_volume(&v)
                    {
                        if *purpose == MediaPurpose::ExportKeys {
                            // The list stays below; after_run closes it.
                            return (Some(view), self.export_to(&mount));
                        }
                        if *purpose == MediaPurpose::InstallPacks {
                            self.packs_from_mount(&mount);
                        } else {
                            self.notify(Level::Info, format!("{} is open.", v.display_name()));
                            // This list closes; the library below shows the stick.
                            self.reload_packs();
                        }
                        return (None, Effect::None);
                    }
                    (*volumes, *error) = self.load_volumes();
                    clamp_selection(list, volumes.len());
                    return (Some(view), Effect::None);
                }
                match key.code {
                    KeyCode::Char('r') => {
                        (*volumes, *error) = self.load_volumes();
                        clamp_selection(list, volumes.len());
                    }
                    KeyCode::Char('e') => {
                        if let Some(v) = selected {
                            self.eject_volume(&v);
                            (*volumes, *error) = self.load_volumes();
                            clamp_selection(list, volumes.len());
                        }
                    }
                    _ if is_activate(key) => {
                        if let Some(v) = selected {
                            next = self.open_volume(v);
                            (*volumes, *error) = self.load_volumes();
                            clamp_selection(list, volumes.len());
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, volumes.len(), key),
                }
            }
            View::Generations { gens, list } => match key.code {
                KeyCode::Char('r') => {
                    *gens = self.load_generations();
                    clamp_selection(list, gens.len());
                }
                _ if is_activate(key) => {
                    if let Some(g) = list.selected().and_then(|i| gens.get(i)).cloned() {
                        self.confirm_default_generation(&g);
                    }
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, gens.len(), key),
            },
            View::Wifi {
                overview,
                error,
                list,
            } => {
                let n = overview.as_ref().map_or(0, |o| o.networks.len());
                let selected = list
                    .selected()
                    .and_then(|i| overview.as_ref()?.networks.get(i))
                    .cloned();
                match key.code {
                    KeyCode::Char('r') => {
                        self.request_wifi_scan();
                        self.notify(Level::Info, "Looking for Wi-Fi networks...");
                        (*overview, *error) = self.load_wifi();
                    }
                    KeyCode::Char('w') => {
                        match wifi::radio_on(&self.sys.tools.nmcli) {
                            Ok(()) => {
                                self.notify(Level::Info, "Wi-Fi switched on.");
                                self.request_wifi_scan();
                            }
                            Err(e) => self.wifi_refused("Wi-Fi could not be switched on", &e),
                        }
                        (*overview, *error) = self.load_wifi();
                    }
                    KeyCode::Char('d') => {
                        if let Some(net) = selected {
                            self.disconnect_wifi(&net);
                            (*overview, *error) = self.load_wifi();
                        }
                    }
                    KeyCode::Char('f') => {
                        if let Some(net) = selected {
                            self.confirm_forget_wifi(&net);
                        }
                    }
                    _ if is_activate(key) => {
                        if let Some(net) = selected {
                            self.join_wifi(&net);
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, n, key),
                }
                let n = overview.as_ref().map_or(0, |o| o.networks.len());
                if list.selected().is_none() && n > 0 {
                    list.select(Some(0));
                }
                clamp_selection(list, n);
            }
            View::Backups {
                volume,
                mount,
                files,
                list,
            } => {
                let sel = list.selected().unwrap_or(0);
                match key.code {
                    KeyCode::Char('r') => {
                        *files = backup::list_backups(mount);
                        clamp_selection(list, files.len() + 1);
                    }
                    KeyCode::Char('c') => {
                        if let Some(f) = sel.checked_sub(1).and_then(|i| files.get(i)) {
                            effect = self.backup_tool_effect(CHECK_LABEL, "check", &f.path);
                        }
                    }
                    KeyCode::Char('e') => {
                        let v = volume.clone();
                        self.eject_volume(&v);
                        self.reload_volumes();
                        return (None, Effect::None);
                    }
                    _ if is_activate(key) => match sel.checked_sub(1) {
                        None => self.confirm_backup(volume, mount),
                        Some(i) => {
                            if let Some(f) = files.get(i) {
                                self.confirm_restore(f);
                            }
                        }
                    },
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, files.len() + 1, key),
                }
            }
            View::Library { kind, items, list } => {
                let kind = *kind;
                let selected = list.selected().and_then(|i| items.get(i)).cloned();
                match key.code {
                    KeyCode::Char('s') if kind == PackKind::Zim => {
                        if let Some(item) = selected {
                            self.modal = Some(Modal::PackSearch {
                                item,
                                value: String::new(),
                            });
                        }
                    }
                    KeyCode::Char('p') => next = self.open_packs(),
                    KeyCode::Char('u') => next = Some(self.choose_reading_stick()),
                    KeyCode::Char('c') => {
                        if let Some(item) = selected {
                            effect = self.check_pack_effect(&item);
                        }
                    }
                    KeyCode::Char('r') => self.reload_packs_in(kind, items, list),
                    _ if is_activate(key) => {
                        if let Some(item) = selected {
                            effect = match kind {
                                PackKind::Zim => self.open_pack_effect(&item, None),
                                PackKind::Map => self.open_map_effect(Some(&item)),
                            };
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, items.len(), key),
                }
            }
            View::Packs { s, list } => {
                let rows = s.rows();
                let row = list.selected().and_then(|i| rows.get(i)).copied();
                match key.code {
                    KeyCode::Char(' ') => {
                        if let Some(r) = row {
                            s.toggle(r);
                        }
                    }
                    KeyCode::Char('d') => self.confirm_download(s),
                    KeyCode::Char('s') => {
                        next = (!s.cfg.sets.is_empty()).then(|| View::PackSets {
                            list: ListState::default().with_selected(Some(0)),
                        });
                        if next.is_none() {
                            self.notify(Level::Info, "No suggested sets on this system.");
                        }
                    }
                    KeyCode::Char('u') => next = self.choose_pack_stick(),
                    KeyCode::Char('f') => {
                        s.next_language();
                        list.select(Some(0));
                        library::fix_selection(list, &s.rows());
                    }
                    KeyCode::Char('r') => {
                        s.refresh();
                        library::fix_selection(list, &s.rows());
                    }
                    KeyCode::Enter => {
                        if let Some(r) = row {
                            next = self.pack_details(s, r);
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => library::nav_rows(list, &rows, key),
                }
            }
            View::Files {
                idx,
                query,
                list,
                pick,
            } => {
                let hits: Vec<(PathBuf, String)> = resilience_core::files::search(idx, query)
                    .iter()
                    .map(|e| (e.path.clone(), e.shown.clone()))
                    .collect();
                let selected = list.selected().and_then(|i| hits.get(i)).cloned();
                match key.code {
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if query.chars().count() < 64 {
                            query.push(c);
                        }
                        let n = resilience_core::files::search(idx, query).len();
                        list.select((n > 0).then_some(0));
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        let n = resilience_core::files::search(idx, query).len();
                        list.select((n > 0).then_some(0));
                    }
                    KeyCode::Enter => {
                        if let Some((p, shown)) = selected {
                            if let Some(pk) = *pick {
                                // The picker closes; the operation runs.
                                return (None, self.picked(pk, &p));
                            }
                            let (v, e) = self.view_file(&p, &shown);
                            next = v;
                            effect = e;
                        }
                    }
                    KeyCode::F(3) => {
                        if let Some((p, shown)) = selected {
                            effect = self.file_details(&p, &shown);
                        }
                    }
                    KeyCode::F(2) => {
                        **idx = resilience_core::files::index(&self.file_roots());
                        let n = resilience_core::files::search(idx, query).len();
                        clamp_selection(list, n);
                        self.notify(Level::Info, format!("{} files indexed.", idx.entries.len()));
                    }
                    _ => nav(list, hits.len(), key),
                }
            }
            View::Table(t) => {
                let rows = t.rows.len().saturating_sub(1);
                let cols = t.widths.len();
                match key.code {
                    KeyCode::Left => t.col = t.col.saturating_sub(1),
                    KeyCode::Right => t.col = (t.col + 1).min(cols.saturating_sub(1)),
                    KeyCode::Up | KeyCode::Char('k') => t.row = t.row.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        t.row = (t.row + 1).min(rows.saturating_sub(1))
                    }
                    KeyCode::PageUp => t.row = t.row.saturating_sub(15),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        t.row = (t.row + 15).min(rows.saturating_sub(1))
                    }
                    KeyCode::Home | KeyCode::Char('g') => t.row = 0,
                    KeyCode::End | KeyCode::Char('G') => t.row = rows.saturating_sub(1),
                    KeyCode::Backspace => return (None, Effect::None),
                    _ => {}
                }
            }
            View::Choices { items, list, .. } => {
                if is_activate(key) {
                    if let Some(c) = list.selected().and_then(|i| items.get(i)).copied() {
                        let (v, e) = self.choose(c);
                        next = v;
                        effect = e;
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, items.len(), key);
                }
            }
            View::Keys { rows, list } => {
                if matches!(
                    key.code,
                    KeyCode::Up | KeyCode::Down | KeyCode::Char('k' | 'j')
                ) || matches!(
                    key.code,
                    KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown
                ) {
                    nav(list, rows.len(), key);
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    let (v, e) = self.keys_key(rows, list.selected(), key);
                    next = v;
                    effect = e;
                }
            }
            View::PackSets { list } => {
                let n = self.pack_sets_len();
                if is_activate(key) {
                    if let Some(i) = list.selected() {
                        self.apply_pack_set(i);
                        return (None, Effect::None);
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, n, key);
                }
            }
            View::Search { query, hits, list } => match key.code {
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if query.chars().count() < 64 {
                        query.push(c);
                    }
                    *hits = self.search(query);
                    list.select((!hits.is_empty()).then_some(0));
                }
                KeyCode::Backspace => {
                    query.pop();
                    *hits = self.search(query);
                    list.select((!hits.is_empty()).then_some(0));
                }
                KeyCode::Enter => {
                    if let Some(h) = list.selected().and_then(|i| hits.get(i)).cloned() {
                        match h {
                            SearchHit::Category(c) => next = Some(category_view(c)),
                            SearchHit::Entry(e) => {
                                let (v, eff) = self.activate(e);
                                next = v;
                                effect = eff;
                            }
                            SearchHit::Page(p) => next = Some(self.open_page(&p)),
                        }
                    }
                }
                _ => nav(list, hits.len(), key),
            },
        }
        if let Some(n) = next {
            // Keep the current view below the newly opened one.
            self.stack.push(view);
            self.stack.push(n);
            return (None, effect);
        }
        (Some(view), effect)
    }

    /// Open an entry: returns a view to push and/or an effect to run.
    pub fn activate(&mut self, e: Entry) -> (Option<View>, Effect) {
        match e {
            Entry::App(i) => {
                let mode = self.mode();
                let app = &self.catalog.apps[i];
                match (app.availability(mode), app.command_for(mode)) {
                    (Availability::Ready(_), Some(argv)) => {
                        let req = RunRequest {
                            label: app.name.clone(),
                            argv: argv.to_vec(),
                            extra_args: Vec::new(),
                            env: Vec::new(),
                            polkit_agent: None,
                            clean_env: false,
                            pause_after: app.pause_after_exit,
                            banner: None,
                        };
                        (None, Effect::Run(req))
                    }
                    (Availability::Missing(what), _) => {
                        let body = format!(
                            "'{}' cannot start because its program is not available:\n\n  {what}\n\n\
                             The rest of the system is unaffected. If this persists, report it \
                             to your administrator.",
                            app.name
                        );
                        self.notify(Level::Error, format!("{} is not available", app.name));
                        self.modal = Some(Modal::Message {
                            title: "PROGRAM UNAVAILABLE".into(),
                            body,
                            error: true,
                        });
                        (None, Effect::None)
                    }
                    (Availability::Ready(_), None) => (None, Effect::None),
                }
            }
            Entry::Builtin(b) => self.activate_builtin(b),
        }
    }

    fn activate_builtin(&mut self, b: Builtin) -> (Option<View>, Effect) {
        let none = (None, Effect::None);
        if !self.mode().allows(b.min_mode()) {
            // Not reachable through the menu; defence in depth for search.
            self.notify(
                Level::Error,
                format!("{} requires {} mode", b.name(), b.min_mode()),
            );
            return none;
        }
        match b {
            Builtin::Handbook => {
                let (pages, error) = match handbook::list(&self.sys.handbook_dir) {
                    Ok(p) => (p, None),
                    Err(e) => (
                        Vec::new(),
                        Some(format!("{}: {e}", self.sys.handbook_dir.display())),
                    ),
                };
                let sel = (!pages.is_empty()).then_some(0);
                (
                    Some(View::Handbook {
                        pages,
                        list: ListState::default().with_selected(sel),
                        error,
                    }),
                    Effect::None,
                )
            }
            Builtin::Notes => {
                let n = notes::list(&notes::notes_dir(&self.home));
                let sel = (!n.is_empty()).then_some(0);
                (
                    Some(View::Notes {
                        notes: n,
                        list: ListState::default().with_selected(sel),
                    }),
                    Effect::None,
                )
            }
            Builtin::Network => {
                self.refresh_fast();
                (Some(View::Network { scroll: 0 }), Effect::None)
            }
            Builtin::SystemStatus => {
                self.refresh_fast();
                self.refresh_slow();
                (Some(View::Status { scroll: 0 }), Effect::None)
            }
            Builtin::Notifications => {
                let text = if self.notices.is_empty() {
                    "No notifications in this session.".to_string()
                } else {
                    self.notices
                        .iter()
                        .rev()
                        .map(|n| {
                            format!(
                                "{} {} {}",
                                crate::fmt::clock(n.at),
                                level_tag(n.level),
                                n.text
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                (
                    Some(View::Pager {
                        title: "NOTIFICATION HISTORY".into(),
                        text,
                        scroll: 0,
                    }),
                    Effect::None,
                )
            }
            Builtin::Settings => {
                let i = ThemeName::ALL
                    .iter()
                    .position(|t| *t == self.settings.theme);
                (
                    Some(View::Settings {
                        list: ListState::default().with_selected(i),
                    }),
                    Effect::None,
                )
            }
            Builtin::Clock => {
                self.open_set_time();
                none
            }
            Builtin::Wifi => {
                self.request_wifi_scan();
                let (overview, error) = self.load_wifi();
                let sel = overview.as_ref().is_some_and(|o| !o.networks.is_empty());
                (
                    Some(View::Wifi {
                        overview,
                        error,
                        list: ListState::default().with_selected(sel.then_some(0)),
                    }),
                    Effect::None,
                )
            }
            Builtin::Users => {
                let (rows, error) = self.load_accounts();
                let sel = (!rows.is_empty()).then_some(0);
                (
                    Some(View::Users {
                        rows,
                        error,
                        list: ListState::default().with_selected(sel),
                    }),
                    Effect::None,
                )
            }
            Builtin::Recovery => (
                Some(View::Recovery {
                    list: ListState::default().with_selected(Some(0)),
                }),
                Effect::None,
            ),
            Builtin::Manual => {
                self.modal = Some(Modal::ManPage {
                    value: String::new(),
                    error: None,
                });
                none
            }
            Builtin::Encrypt
            | Builtin::Decrypt
            | Builtin::Messages
            | Builtin::Sign
            | Builtin::Verify
            | Builtin::Checksums
            | Builtin::Inspect
            | Builtin::Random
            | Builtin::Keys => self.open_crypto(b),
            Builtin::Library => (self.open_library(), Effect::None),
            Builtin::Files => (Some(self.open_files()), Effect::None),
            Builtin::Packs => (self.open_packs(), Effect::None),
            Builtin::Map => (None, self.open_map_effect(None)),
            Builtin::Chat => (self.open_chats(), Effect::None),
            Builtin::Mesh => {
                self.open_mesh();
                (None, Effect::None)
            }
            Builtin::Community => (Some(self.open_community()), Effect::None),
            Builtin::Install => (self.open_install(), Effect::None),
            Builtin::MapFiles => (self.open_map_files(), Effect::None),
            Builtin::Backup => {
                let (volumes, error) = self.load_volumes();
                let sel = (!volumes.is_empty()).then_some(0);
                (
                    Some(View::Media {
                        volumes,
                        error,
                        list: ListState::default().with_selected(sel),
                        purpose: MediaPurpose::Backups,
                    }),
                    Effect::None,
                )
            }
            Builtin::Help => (
                Some(View::Help {
                    topic: HelpTopic::Home,
                    scroll: 0,
                }),
                Effect::None,
            ),
            Builtin::Shell => {
                let req = RunRequest {
                    label: "Terminal shell".into(),
                    argv: self.sys.shell.clone(),
                    extra_args: Vec::new(),
                    env: Vec::new(),
                    polkit_agent: None,
                    clean_env: false,
                    pause_after: false,
                    banner: Some(format!(
                        "RESILIENCE SHELL — {} mode, unprivileged. Type 'exit' to return to the interface.",
                        self.mode()
                    )),
                };
                (None, Effect::Run(req))
            }
            Builtin::Logout => {
                self.confirm(Pending::Logout);
                none
            }
            Builtin::Reboot => {
                self.confirm(Pending::Reboot);
                none
            }
            Builtin::PowerOff => {
                self.confirm(Pending::PowerOff);
                none
            }
        }
    }

    fn open_page(&mut self, p: &handbook::Page) -> View {
        match handbook::read(&p.path) {
            Ok(text) => View::Pager {
                title: p.title.to_uppercase(),
                text,
                scroll: 0,
            },
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("cannot open {}: {e}", p.path.display()),
                );
                View::Pager {
                    title: "ERROR".into(),
                    text: format!("This page could not be read:\n\n{e}"),
                    scroll: 0,
                }
            }
        }
    }

    fn edit_note(&self, path: PathBuf) -> Effect {
        let argv = if self.mode() == Mode::UltraStable {
            &self.sys.restricted_editor
        } else {
            &self.sys.editor
        };
        Effect::Run(RunRequest {
            label: "Notes editor".into(),
            argv: argv.clone(),
            extra_args: vec![path],
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: false,
            banner: None,
        })
    }

    /// Load a keyboard layout for the consoles and save it as this
    /// account's choice. The kernel lets only privileged programs change
    /// the console keymap: `pkexec <helper> set-keymap ID` (ADR 0017; no
    /// password for the active local session, listed layouts only).
    fn set_layout(&mut self, id: &str) {
        let label = self.sys.keyboard.label(id).to_string();
        match self.load_layout(id) {
            Ok(()) => {
                self.settings.keyboard = Some(id.to_string());
                match self.settings.save(&self.settings_path) {
                    Ok(()) => self.notify(
                        Level::Info,
                        format!(
                            "Keyboard layout set to {label}. It applies to every console until \
                             someone logs out; login prompts always use the system's layout."
                        ),
                    ),
                    Err(e) => self.notify(
                        Level::Error,
                        format!("Keyboard layout applied but not saved: {e}"),
                    ),
                }
            }
            Err(e) => self.notify(
                Level::Error,
                format!("The keyboard layout {label} could not be loaded: {e}"),
            ),
        }
    }

    fn load_layout(&self, id: &str) -> Result<(), String> {
        if self.sys.keyboard.find(id).is_none() || !keyboard::is_valid_id(id) {
            return Err("it is not one of the layouts of this system".into());
        }
        let cfg = std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))?;
        let pkexec = cfg.pkexec.to_string_lossy();
        let helper = cfg.helper.to_string_lossy();
        let shell = cfg.session_shell.to_string_lossy();
        match run_full_env(
            &[pkexec.as_ref(), helper.as_ref(), "set-keymap", id],
            &[("SHELL", shell.as_ref())],
            Duration::from_secs(15),
        ) {
            Some(o) if o.success() => Ok(()),
            Some(o) => Err(o
                .stderr
                .lines()
                .next()
                .unwrap_or("the keyboard layout was refused")
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect()),
            None => Err("the system helper did not run or took too long".into()),
        }
    }

    /// At the start of a session: load the layout this account chose, if it
    /// differs from the system's (which the login prompt already loaded).
    pub fn apply_saved_layout(&mut self) {
        let Some(saved) = self.settings.keyboard.clone() else {
            return;
        };
        let id = self.sys.keyboard.effective(Some(&saved)).to_string();
        if id != saved {
            self.notify(
                Level::Warn,
                format!("Your keyboard layout '{saved}' is no longer offered; using {id}."),
            );
        }
        if id == self.sys.keyboard.default {
            return;
        }
        if let Err(e) = self.load_layout(&id) {
            let label = self.sys.keyboard.label(&id).to_string();
            self.notify(
                Level::Error,
                format!("Your keyboard layout {label} could not be loaded: {e}"),
            );
        }
    }

    fn set_theme(&mut self, t: ThemeName) {
        self.settings.theme = t;
        self.theme = t;
        match self.settings.save(&self.settings_path) {
            Ok(()) => self.notify(Level::Info, format!("Theme set to {}.", t.label())),
            Err(e) => self.notify(Level::Error, format!("Theme applied but not saved: {e}")),
        }
    }

    fn confirm(&mut self, action: Pending) {
        let (title, body, confirm) = match &action {
            Pending::SetMode { .. }
            | Pending::SetTime { .. }
            | Pending::SetDefaultGeneration { .. }
            | Pending::ApplyUpdate { .. }
            | Pending::DeclineLicence
            | Pending::JoinWifi { .. }
            | Pending::ForgetWifi { .. }
            | Pending::Backup { .. }
            | Pending::Restore { .. }
            | Pending::ChangePacks { .. }
            | Pending::RemoveContact { .. }
            | Pending::RemovePgpKey { .. }
            | Pending::SetMesh { .. }
            | Pending::SetShare { .. }
            | Pending::PeerPacks { .. }
            | Pending::PeerGet { .. }
            | Pending::SetHotspot { .. } => return,
            Pending::Logout => (
                "LOG OUT",
                "End this session? Programs you started from here will be closed.",
                "Log out",
            ),
            Pending::Reboot => (
                "RESTART",
                "Restart the computer now? Unsaved work in open programs will be lost.",
                "Restart",
            ),
            Pending::PowerOff => (
                "POWER OFF",
                "Shut the computer down now? Unsaved work in open programs will be lost.",
                "Power off",
            ),
        };
        self.modal = Some(Modal::Confirm {
            title: title.into(),
            body: body.into(),
            confirm: confirm.into(),
            action,
            yes_focused: false, // the safe choice is the default
        });
    }

    fn handle_modal(&mut self, modal: Modal, key: KeyEvent) -> Effect {
        match modal {
            Modal::Message { .. } => {
                if !matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                    self.modal = Some(modal);
                }
                Effect::None
            }
            Modal::Confirm {
                title,
                body,
                confirm,
                action,
                mut yes_focused,
            } => {
                let decided = match key.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                        yes_focused = !yes_focused;
                        None
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
                    KeyCode::Enter => Some(yes_focused),
                    _ => None,
                };
                match decided {
                    None => {
                        self.modal = Some(Modal::Confirm {
                            title,
                            body,
                            confirm,
                            action,
                            yes_focused,
                        });
                        Effect::None
                    }
                    Some(false) => Effect::None,
                    Some(true) => self.execute(action),
                }
            }
            Modal::NewNote { mut value, error } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter => match notes::create(&notes::notes_dir(&self.home), &value) {
                    Ok(path) => {
                        self.notify(Level::Info, format!("Created note {}", path.display()));
                        self.reload_notes();
                        self.edit_note(path)
                    }
                    Err(e) => {
                        self.modal = Some(Modal::NewNote {
                            value,
                            error: Some(e.to_string()),
                        });
                        Effect::None
                    }
                },
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::NewNote { value, error: None });
                    Effect::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if value.chars().count() < 64 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::NewNote { value, error: None });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::NewNote { value, error });
                    Effect::None
                }
            },
            Modal::WifiPassword {
                net,
                uuid,
                mut value,
                error,
            } => match key.code {
                KeyCode::Esc => {
                    wifi::wipe(value);
                    Effect::None
                }
                KeyCode::Enter => {
                    self.submit_wifi_password(net, uuid, value);
                    Effect::None
                }
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::WifiPassword {
                        net,
                        uuid,
                        value,
                        error: None,
                    });
                    Effect::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if value.len() < 64 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::WifiPassword {
                        net,
                        uuid,
                        value,
                        error: None,
                    });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::WifiPassword {
                        net,
                        uuid,
                        value,
                        error,
                    });
                    Effect::None
                }
            },
            Modal::ManPage { mut value, error } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter => self.submit_man_page(value),
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::ManPage { value, error: None });
                    Effect::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if value.chars().count() < 80 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::ManPage { value, error: None });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::ManPage { value, error });
                    Effect::None
                }
            },
            Modal::Recipients {
                target,
                rows,
                ticked,
                sel,
                ..
            } => self.recipients_key(target, rows, ticked, sel, key),
            Modal::ExpectedSum { file, value, .. } => self.expected_sum_key(file, value, key),
            Modal::ChatInput { kind, value, error } => self.chat_input_key(kind, value, error, key),
            Modal::ShareInput { kind, value, error } => {
                self.share_input_key(kind, value, error, key)
            }
            Modal::InstallText {
                field,
                value,
                error,
            } => self.install_text_key(field, value, error, key),
            Modal::PackSearch { item, mut value } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter if !value.trim().is_empty() => {
                    self.open_pack_effect(&item, Some(value.trim()))
                }
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::PackSearch { item, value });
                    Effect::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if value.chars().count() < 80 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::PackSearch { item, value });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::PackSearch { item, value });
                    Effect::None
                }
            },
            Modal::SetTime {
                mut value,
                zone,
                error,
            } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter => {
                    self.submit_set_time(value, zone);
                    Effect::None
                }
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::SetTime {
                        value,
                        zone,
                        error: None,
                    });
                    Effect::None
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && (c.is_ascii_digit() || "-: ".contains(c)) =>
                {
                    if value.len() < 19 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::SetTime {
                        value,
                        zone,
                        error: None,
                    });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::SetTime { value, zone, error });
                    Effect::None
                }
            },
        }
    }

    fn execute(&mut self, action: Pending) -> Effect {
        let verb = match action {
            Pending::Logout | Pending::DeclineLicence => return Effect::Quit,
            Pending::SetMode { user, to } => return self.set_mode_effect(&user, to),
            Pending::SetTime { utc } => return self.set_time_effect(utc),
            Pending::SetDefaultGeneration { number } => {
                return self.set_default_generation_effect(number);
            }
            Pending::ApplyUpdate { dir } => return self.apply_update_effect(&dir),
            Pending::JoinWifi { net } => {
                self.create_and_join(&net, None);
                return Effect::None;
            }
            Pending::SetMesh { on } => {
                self.set_mesh(on);
                return Effect::None;
            }
            Pending::SetShare { args } => return self.set_share_effect(&args),
            Pending::PeerPacks { peer, get } => return self.peer_packs_effect(&peer, &get),
            Pending::PeerGet { peer, name } => return self.peer_get_effect(&peer, &name),
            Pending::SetHotspot { on } => return self.set_hotspot_effect(on),
            Pending::ForgetWifi { name, uuid } => {
                self.forget_wifi(&name, &uuid);
                return Effect::None;
            }
            Pending::Backup { mount } => {
                return self.backup_tool_effect(BACKUP_LABEL, "create", &mount);
            }
            Pending::Restore { file } => {
                return self.backup_tool_effect(RESTORE_LABEL, "restore", &file);
            }
            Pending::ChangePacks { get, remove, from } => {
                return self.change_packs_effect(&get, &remove, from.as_deref());
            }
            Pending::RemoveContact { name } => {
                self.remove_contact(&name);
                return Effect::None;
            }
            Pending::RemovePgpKey { fingerprint } => {
                self.remove_pgp_key(&fingerprint);
                return Effect::None;
            }
            Pending::Reboot => "reboot",
            Pending::PowerOff => "poweroff",
        };
        tracing::warn!(verb, "power action requested");
        // --no-ask-password: never let polkit start a password agent on top
        // of the TUI. Authorisation for local sessions is decided by logind.
        match run_full(
            &["systemctl", "--no-ask-password", verb],
            Duration::from_secs(10),
        ) {
            Some(o) if o.success() => {
                self.notify(Level::Info, format!("{verb} in progress..."));
            }
            Some(o) => {
                let msg = o.stderr.trim().to_string();
                self.notify(Level::Error, format!("{verb} refused: {msg}"));
                self.modal = Some(Modal::Message {
                    title: "ACTION REFUSED".into(),
                    body: format!("The system refused to {verb}:\n\n{msg}"),
                    error: true,
                });
            }
            None => {
                self.notify(Level::Error, format!("{verb}: systemctl did not answer"));
                self.modal = Some(Modal::Message {
                    title: "ACTION FAILED".into(),
                    body: "systemctl could not be run or did not answer in time.".into(),
                    error: true,
                });
            }
        }
        Effect::None
    }

    fn reload_notes(&mut self) {
        let dir = notes::notes_dir(&self.home);
        for v in self.stack.iter_mut() {
            if let View::Notes { notes: n, list } = v {
                *n = notes::list(&dir);
                clamp_selection(list, n.len());
            }
        }
    }

    /// Called by the main loop after a program started from the TUI exits.
    pub fn after_run(&mut self, label: &str, result: &std::io::Result<std::process::ExitStatus>) {
        match result {
            Ok(st) if st.success() => {}
            Ok(st) => self.notify(Level::Warn, format!("{label} exited with {st}")),
            Err(e) => self.notify(Level::Error, format!("{label} could not start: {e}")),
        }
        if label == SET_MODE_LABEL {
            // Back to the account list, which now shows the result.
            while matches!(self.view(), View::ChooseMode { .. }) {
                self.stack.pop();
            }
            self.reload_accounts();
        }
        if [BACKUP_LABEL, CHECK_LABEL, RESTORE_LABEL].contains(&label) {
            self.reload_backups();
        }
        self.share_after_run(label);
        if label == PACKS_LABEL {
            // Back to the packs screen (from the stick list), refreshed.
            while matches!(
                self.view(),
                View::Media {
                    purpose: MediaPurpose::InstallPacks,
                    ..
                }
            ) {
                self.stack.pop();
            }
            self.reload_packs();
        }
        if label == crypto_ui::CRYPTO_LABEL {
            while matches!(
                self.view(),
                View::Media {
                    purpose: MediaPurpose::ExportKeys,
                    ..
                }
            ) {
                self.stack.pop();
            }
            self.reload_keys();
        }
        if label == UPDATE_CHECK_LABEL || label == UPDATE_APPLY_LABEL {
            self.reload_updates();
        }
        if label == SET_DEFAULT_GENERATION_LABEL || label == UPDATE_APPLY_LABEL {
            let fresh = self.load_generations();
            for v in &mut self.stack {
                if let View::Generations { gens, list } = v {
                    *gens = fresh.clone();
                    clamp_selection(list, gens.len());
                }
            }
        }
        self.reload_notes();
        self.refresh_fast();
    }

    // ----- users and security modes ----------------------------------------

    /// Managed accounts, from the root-owned helper configuration, the
    /// recorded mode changes and the live group database. Read-only.
    pub fn load_accounts(&self) -> (Vec<AccountRow>, Option<String>) {
        let cfg = match AdminConfig::read(&self.paths.admin_config) {
            Ok(c) => c,
            Err(e) => return (Vec::new(), Some(e)),
        };
        let state = match std::fs::read_to_string(&cfg.state_file) {
            Ok(t) => match ModeState::parse(&t) {
                Ok(s) => s,
                Err(e) => {
                    return (
                        Vec::new(),
                        Some(format!("{} is damaged: {e}", cfg.state_file.display())),
                    );
                }
            },
            Err(_) => ModeState::default(),
        };
        let groups = std::fs::read_to_string(&self.paths.group_file).unwrap_or_default();
        (admin::overview(&cfg, &state, &groups), None)
    }

    fn reload_accounts(&mut self) {
        let (rows, error) = self.load_accounts();
        for v in self.stack.iter_mut() {
            if let View::Users {
                rows: r,
                error: e,
                list,
            } = v
            {
                *r = rows.clone();
                *e = error.clone();
                clamp_selection(list, r.len());
            }
        }
    }

    fn choose_mode(&mut self, row: AccountRow) -> Option<View> {
        if !self.has_administrator("Changing a security mode") {
            return None;
        }
        let i = MODES.iter().position(|m| *m == row.effective);
        Some(View::ChooseMode {
            row,
            list: ListState::default().with_selected(i),
        })
    }

    /// False (with an explanation shown) when no account could type an
    /// administrator password, so an action is not offered at all.
    fn has_administrator(&mut self, what: &str) -> bool {
        let (rows, _) = self.load_accounts();
        if admin::administrators(&rows) > 0 {
            return true;
        }
        self.modal = Some(Modal::Message {
            title: "NO ADMINISTRATOR".into(),
            body: format!(
                "{what} needs the password of an Advanced (administrator) account, \
                 and this system has none.\n\n\
                 On the live system this is intentional. On an installed system, \
                 see the handbook page 'Security modes'."
            ),
            error: true,
        });
        false
    }

    /// Explain the consequences before anything happens. The checks here
    /// are for clear messages only; the helper repeats all of them.
    fn confirm_set_mode(&mut self, row: &AccountRow, to: Mode) {
        let cfg = AdminConfig::read(&self.paths.admin_config).ok();
        let state = cfg
            .as_ref()
            .and_then(|c| std::fs::read_to_string(&c.state_file).ok())
            .and_then(|t| ModeState::parse(&t).ok())
            .unwrap_or_default();
        if let Some(c) = &cfg
            && let Err(refusal) = admin::plan_set_mode(c, &state, &row.name, to)
        {
            self.modal = Some(Modal::Message {
                title: "NOT POSSIBLE".into(),
                body: format!("{}.", capitalise(&refusal.to_string())),
                error: true,
            });
            return;
        }
        let user = &row.name;
        let is_self = *user == self.who.user;
        let mut body = format!("Change '{user}' from {} to {to} mode?\n\n", row.effective);
        body.push_str(match to {
            Mode::Advanced => {
                "The account will be able to change the whole system with sudo and its \
                 password. Mistakes in this mode can damage the installation."
            }
            Mode::Stable => {
                "The account gets an unprivileged shell and can use the Nix package \
                 tools, but cannot change the system."
            }
            Mode::UltraStable => {
                "The account will only have the guided interface: no shell, no \
                 administration, no software installation."
            }
        });
        body.push_str("\n\n");
        if to < row.effective {
            body.push_str(&format!(
                "Rights are removed immediately: all sessions of '{user}' will be closed{}.",
                if is_self { ", including this one" } else { "" }
            ));
        } else {
            body.push_str(&format!("New rights apply at the next login of '{user}'."));
        }
        body.push_str(
            "\n\nAn administrator must type their password next. The change is \
             recorded in the audit log and kept after restarts.",
        );
        self.modal = Some(Modal::Confirm {
            title: "CHANGE SECURITY MODE".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::SetMode {
                user: user.clone(),
                to,
            },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> set-mode <user> <mode>`. The user
    /// name comes from the root-owned configuration and is validated again.
    fn set_mode_effect(&mut self, user: &str, to: Mode) -> Effect {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => {
                self.notify(Level::Error, format!("mode change unavailable: {e}"));
                return Effect::None;
            }
        };
        if !admin::valid_user_name(user) || !cfg.users.contains_key(user) {
            return Effect::None;
        }
        tracing::warn!(
            user,
            mode = admin::mode_id(to),
            "security mode change requested"
        );
        Effect::Run(RunRequest {
            label: SET_MODE_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-mode".into(),
                user.into(),
                admin::mode_id(to).into(),
            ],
            extra_args: Vec::new(),
            // pkexec refuses to run unless $SHELL is listed in /etc/shells;
            // Ultra-Stable sessions deliberately set it to nologin.
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "CHANGE SECURITY MODE: '{user}' -> {to}. An administrator must now type \
                 their password. Press Ctrl-C to cancel."
            )),
        })
    }

    // ----- date and time ------------------------------------------------------

    fn open_set_time(&mut self) {
        if !self.has_administrator("Setting the clock") {
            return;
        }
        self.modal = Some(Modal::SetTime {
            value: crate::fmt::date_time(SystemTime::now()),
            zone: clock::system_zone(),
            error: None,
        });
    }

    /// Validate the typed local time; on success ask for confirmation.
    /// The helper checks format and bounds again on the UTC value.
    fn submit_set_time(&mut self, value: String, zone: String) {
        let parsed = clock::parse_local(&value).and_then(|c| {
            clock::local_to_unix(c)
                .ok_or(clock::TimeRefusal::NoSuchDate)
                .and_then(clock::check_bounds)
        });
        let utc = match parsed {
            Ok(t) => t,
            Err(e) => {
                self.modal = Some(Modal::SetTime {
                    value,
                    zone,
                    error: Some(capitalise(&e.to_string())),
                });
                return;
            }
        };
        let now = crate::fmt::date_time(SystemTime::now());
        let new = crate::fmt::date_time(SystemTime::UNIX_EPOCH + Duration::from_secs(utc as u64));
        let body = format!(
            "Set the clock to {new} ({zone})?\n\n\
             The clock now shows {now}. In UTC the new time is {}.\n\n\
             An administrator must type their password next. The change is \
             recorded in the audit log, and the hardware clock keeps it after \
             a restart.",
            admin::rfc3339_utc(utc)
        );
        self.modal = Some(Modal::Confirm {
            title: "SET THE CLOCK".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::SetTime { utc },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> set-time <YYYY-MM-DDTHH:MM:SSZ>`.
    fn set_time_effect(&mut self, utc: i64) -> Effect {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("setting the clock is unavailable: {e}"),
                );
                return Effect::None;
            }
        };
        let Ok(utc) = clock::check_bounds(utc) else {
            return Effect::None;
        };
        let at = admin::rfc3339_utc(utc);
        tracing::warn!(to = %at, "clock change requested");
        Effect::Run(RunRequest {
            label: SET_TIME_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-time".into(),
                at.clone(),
            ],
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "SET THE CLOCK to {at}. An administrator must now type their \
                 password. Press Ctrl-C to cancel."
            )),
        })
    }

    // ----- Wi-Fi ----------------------------------------------------------------

    pub fn load_wifi(&self) -> (Option<Overview>, Option<String>) {
        let nmcli = &self.sys.tools.nmcli;
        if !wifi::available(nmcli) {
            return (
                None,
                Some("NetworkManager is not installed on this system.".into()),
            );
        }
        match wifi::overview(nmcli) {
            Ok(o) => (Some(o), None),
            Err(e) => (None, Some(e)),
        }
    }

    fn reload_wifi(&mut self) {
        let (o, e) = self.load_wifi();
        for v in self.stack.iter_mut() {
            if let View::Wifi {
                overview,
                error,
                list,
            } = v
            {
                *overview = o.clone();
                *error = e.clone();
                let n = overview.as_ref().map_or(0, |o| o.networks.len());
                if list.selected().is_none() && n > 0 {
                    list.select(Some(0));
                }
                clamp_selection(list, n);
            }
        }
    }

    /// Scanning takes a few seconds; results show up at the next refresh.
    fn request_wifi_scan(&self) {
        let nmcli = self.sys.tools.nmcli.clone();
        if !wifi::available(&nmcli) {
            return;
        }
        thread::spawn(move || {
            if let Err(e) = wifi::request_scan(&nmcli) {
                tracing::info!(error = %e, "wifi scan request");
            }
        });
    }

    fn wifi_refused(&mut self, what: &str, e: &str) {
        let hint = if e.contains("nsufficient privileges") || e.contains("Not authorized") {
            "\n\nYour account is not allowed to do this. An administrator can do it, \
             or change your security mode."
        } else {
            ""
        };
        self.notify(Level::Error, format!("{what}: {e}"));
        self.modal = Some(Modal::Message {
            title: "WI-FI".into(),
            body: format!("{what}:\n\n{e}{hint}"),
            error: true,
        });
    }

    fn join_wifi(&mut self, net: &Network) {
        if let Some(j) = &self.wifi_job {
            self.notify(Level::Warn, format!("Still connecting to '{}'...", j.ssid));
            return;
        }
        if net.in_use {
            self.modal = Some(Modal::Message {
                title: "WI-FI".into(),
                body: format!(
                    "You are connected to '{}'. Press d to disconnect.",
                    net.ssid
                ),
                error: false,
            });
            return;
        }
        if let Some(p) = &net.saved {
            // NetworkManager has the password; if it is refused, the
            // password dialog opens (see poll_wifi_job).
            let (ssid, uuid) = (net.ssid.clone(), p.uuid.clone());
            self.start_wifi_job(ssid, uuid, false, None);
            return;
        }
        if let Some(problem) = net.problem() {
            self.modal = Some(Modal::Message {
                title: "NOT POSSIBLE".into(),
                body: format!("'{}' cannot be joined here: {problem}.", net.ssid),
                error: true,
            });
            return;
        }
        if net.security == wifi::Security::Open {
            self.modal = Some(Modal::Confirm {
                title: "OPEN NETWORK".into(),
                body: format!(
                    "'{}' has no password and no encryption. People nearby can see \
                     what you send unless the program itself encrypts it.\n\n\
                     Join it anyway?",
                    net.ssid
                ),
                confirm: "Join".into(),
                action: Pending::JoinWifi { net: net.clone() },
                yes_focused: false,
            });
            return;
        }
        self.modal = Some(Modal::WifiPassword {
            net: net.clone(),
            uuid: None,
            value: String::new(),
            error: None,
        });
    }

    /// The password dialog was confirmed. The password is checked, then
    /// moved into the connection thread, which wipes it after use.
    fn submit_wifi_password(&mut self, net: Network, uuid: Option<String>, value: String) {
        if let Err(e) = wifi::check_password(&value) {
            self.modal = Some(Modal::WifiPassword {
                net,
                uuid,
                value,
                error: Some(format!("{}.", capitalise(e))),
            });
            return;
        }
        match uuid {
            Some(u) => self.start_wifi_job(net.ssid.clone(), u, false, Some(value)),
            None => self.create_and_join(&net, Some(value)),
        }
    }

    /// Create a profile for this account only (no secret), then connect.
    fn create_and_join(&mut self, net: &Network, password: Option<String>) {
        let nmcli = self.sys.tools.nmcli.clone();
        let created = wifi::new_uuid()
            .map_err(|e| e.to_string())
            .and_then(|uuid| {
                let argv = wifi::add_profile_argv(&nmcli, net, &uuid, &self.who.user)?;
                wifi::add_profile(&argv)?;
                Ok(uuid)
            });
        match created {
            Ok(uuid) => {
                tracing::info!(ssid = %net.ssid, "wifi profile created");
                self.start_wifi_job(net.ssid.clone(), uuid, true, password);
            }
            Err(e) => {
                if let Some(p) = password {
                    wifi::wipe(p);
                }
                self.wifi_refused(&format!("'{}' could not be set up", net.ssid), &e);
            }
        }
    }

    fn start_wifi_job(
        &mut self,
        ssid: String,
        uuid: String,
        new_profile: bool,
        password: Option<String>,
    ) {
        let nmcli = self.sys.tools.nmcli.clone();
        let with_password = password.is_some();
        let (tx, rx) = mpsc::channel();
        let id = uuid.clone();
        thread::spawn(move || {
            let _ = tx.send(wifi::connect(&nmcli, &id, password));
        });
        self.notify(Level::Info, format!("Connecting to '{ssid}'..."));
        self.wifi_job = Some(WifiJob {
            ssid,
            uuid,
            new_profile,
            with_password,
            started: Instant::now(),
            rx,
        });
    }

    fn poll_wifi_job(&mut self) {
        let Some(job) = &self.wifi_job else { return };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("the connection attempt stopped".into()),
        };
        let job = self.wifi_job.take().expect("checked above");
        match result {
            Ok(()) => {
                tracing::info!(ssid = %job.ssid, "wifi connected");
                self.notify(Level::Info, format!("Connected to '{}'.", job.ssid));
            }
            Err(e) => {
                tracing::info!(ssid = %job.ssid, error = %e, "wifi connection failed");
                if job.new_profile {
                    // A failed first attempt leaves nothing behind.
                    if let Err(f) = wifi::forget(&self.sys.tools.nmcli, &job.uuid) {
                        self.notify(
                            Level::Warn,
                            format!("the new network could not be removed: {f}"),
                        );
                    }
                }
                let password_problem = wifi::needs_password(&e);
                if password_problem && !job.new_profile && !job.with_password {
                    // A saved network whose password was refused or lost.
                    let net = self
                        .wifi_networks()
                        .into_iter()
                        .find(|n| n.ssid == job.ssid);
                    if let Some(net) = net {
                        self.modal = Some(Modal::WifiPassword {
                            net,
                            uuid: Some(job.uuid),
                            value: String::new(),
                            error: Some(
                                "The saved password was not accepted. Type it again.".into(),
                            ),
                        });
                        self.reload_wifi();
                        return;
                    }
                }
                let mut body = format!("Could not connect to '{}':\n\n{e}", job.ssid);
                if let Some(hint) = wifi::failure_hint(&e, job.with_password) {
                    body.push_str("\n\n");
                    body.push_str(hint);
                }
                if job.new_profile {
                    body.push_str(" Nothing was saved.");
                }
                self.notify(Level::Warn, format!("Not connected to '{}'.", job.ssid));
                self.modal = Some(Modal::Message {
                    title: "NOT CONNECTED".into(),
                    body,
                    error: true,
                });
            }
        }
        self.reload_wifi();
    }

    fn wifi_networks(&self) -> Vec<Network> {
        self.stack
            .iter()
            .find_map(|v| match v {
                View::Wifi {
                    overview: Some(o), ..
                } => Some(o.networks.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn disconnect_wifi(&mut self, net: &Network) {
        match &net.saved {
            Some(p) if p.active => match wifi::disconnect(&self.sys.tools.nmcli, p) {
                Ok(()) => self.notify(Level::Info, format!("Disconnected from '{}'.", net.ssid)),
                Err(e) => self.wifi_refused("Could not disconnect", &e),
            },
            _ if net.in_use => self.wifi_refused(
                "Could not disconnect",
                "this connection was not set up under the network's name",
            ),
            _ => self.notify(Level::Info, format!("'{}' is not connected.", net.ssid)),
        }
    }

    fn confirm_forget_wifi(&mut self, net: &Network) {
        let Some(p) = &net.saved else {
            self.notify(Level::Info, format!("'{}' is not saved.", net.ssid));
            return;
        };
        self.modal = Some(Modal::Confirm {
            title: "FORGET NETWORK".into(),
            body: format!(
                "Forget '{}'? The saved password is deleted{}. You will need the \
                 password to join it again.",
                net.ssid,
                if p.active {
                    " and you are disconnected"
                } else {
                    ""
                }
            ),
            confirm: "Forget".into(),
            action: Pending::ForgetWifi {
                name: net.ssid.clone(),
                uuid: p.uuid.clone(),
            },
            yes_focused: false,
        });
    }

    fn forget_wifi(&mut self, name: &str, uuid: &str) {
        match wifi::forget(&self.sys.tools.nmcli, uuid) {
            Ok(()) => self.notify(Level::Info, format!("'{name}' was forgotten.")),
            Err(e) => self.wifi_refused(&format!("'{name}' could not be forgotten"), &e),
        }
        self.reload_wifi();
    }

    // ----- removable media and backups ---------------------------------------

    /// Removable volumes, through lsblk. Internal disks are never listed.
    pub fn load_volumes(&self) -> (Vec<Volume>, Option<String>) {
        match media::list(&self.sys.tools.lsblk) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(e)),
        }
    }

    fn reload_volumes(&mut self) {
        let (v, e) = self.load_volumes();
        for view in self.stack.iter_mut() {
            if let View::Media {
                volumes,
                error,
                list,
                ..
            } = view
            {
                *volumes = v.clone();
                *error = e.clone();
                clamp_selection(list, volumes.len());
            }
        }
    }

    fn reload_backups(&mut self) {
        for view in self.stack.iter_mut() {
            if let View::Backups {
                mount, files, list, ..
            } = view
            {
                *files = backup::list_backups(mount);
                clamp_selection(list, files.len() + 1);
            }
        }
    }

    /// Open one manual page in `less` (secure mode), or show the pages whose
    /// description matches the words.
    fn submit_man_page(&mut self, value: String) -> Effect {
        let req = match manpage::parse(&value) {
            Ok(r) => r,
            Err(e) => {
                self.modal = Some(Modal::ManPage {
                    value,
                    error: Some(e),
                });
                return Effect::None;
            }
        };
        let man = self.sys.tools.man.clone();
        let program = match &req {
            manpage::Request::Page { .. } => man.clone(),
            manpage::Request::Search(_) => self.sys.tools.apropos.clone(),
        };
        let mut argv: Vec<String> = vec![program];
        argv.extend(req.args());
        match &req {
            manpage::Request::Page { .. } => {
                let mut probe: Vec<&str> = vec![man.as_str(), "-w"];
                probe.extend(argv[1..].iter().map(String::as_str));
                if !run_full(&probe, Duration::from_secs(10)).is_some_and(|o| o.success()) {
                    self.modal = Some(Modal::ManPage {
                        value,
                        error: Some(
                            "No manual page with that name. Type a few words to search instead."
                                .into(),
                        ),
                    });
                    return Effect::None;
                }
                Effect::Run(RunRequest {
                    label: "Manual pages".into(),
                    argv,
                    extra_args: Vec::new(),
                    env: manpage::environment(&self.sys.tools.less),
                    polkit_agent: None,
                    clean_env: true,
                    pause_after: false,
                    banner: None,
                })
            }
            manpage::Request::Search(words) => {
                let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                let text = run_full(&refs, Duration::from_secs(15))
                    .filter(|o| o.success() && !o.stdout.trim().is_empty())
                    .map(|o| {
                        format!(
                            "Manual pages about \"{}\". Open one with Manual pages and its \
                             name (the section in brackets can come first, e.g. \"5 passwd\").\n\n{}",
                            words.join(" "),
                            o.stdout
                        )
                    })
                    .unwrap_or_else(|| format!("No manual page mentions \"{}\".", words.join(" ")));
                self.push(View::Pager {
                    title: "MANUAL PAGES".into(),
                    text,
                    scroll: 0,
                });
                Effect::None
            }
        }
    }

    /// Mount (if needed) a volume; explains any problem in a message.
    fn mount_volume(&mut self, v: &Volume) -> Option<PathBuf> {
        if let Some(p) = &v.problem {
            self.modal = Some(Modal::Message {
                title: "CANNOT USE THIS DEVICE".into(),
                body: format!("'{}' cannot be used here: {p}.", v.display_name()),
                error: true,
            });
            return None;
        }
        match media::mount(&self.sys.tools.lsblk, &self.sys.tools.udisksctl, v) {
            Ok(m) => Some(m),
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("cannot open {}: {e}", v.display_name()),
                );
                self.modal = Some(Modal::Message {
                    title: "CANNOT OPEN THE STICK".into(),
                    body: format!("'{}' could not be opened:\n\n{e}", v.display_name()),
                    error: true,
                });
                None
            }
        }
    }

    fn reload_packs_in(&mut self, kind: PackKind, items: &mut Vec<LibItem>, list: &mut ListState) {
        if let Ok(c) = resilience_core::packs::PackConfig::load(&self.paths.packs_config) {
            *items = self.all_library_items(&c, kind);
            clamp_selection(list, items.len());
        }
    }

    /// Mount (if needed) and show the backups on a volume.
    fn open_volume(&mut self, v: Volume) -> Option<View> {
        if let Some(p) = &v.problem {
            self.modal = Some(Modal::Message {
                title: "CANNOT USE THIS DEVICE".into(),
                body: format!("'{}' cannot be used here: {p}.", v.display_name()),
                error: true,
            });
            return None;
        }
        match media::mount(&self.sys.tools.lsblk, &self.sys.tools.udisksctl, &v) {
            Ok(mount) => {
                if v.mountpoint.is_none() {
                    self.notify(
                        Level::Info,
                        format!("{} is ready ({})", v.display_name(), mount.display()),
                    );
                }
                let files = backup::list_backups(&mount);
                Some(View::Backups {
                    volume: Volume {
                        mountpoint: Some(mount.clone()),
                        ..v
                    },
                    mount,
                    files,
                    list: ListState::default().with_selected(Some(0)),
                })
            }
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("cannot open {}: {e}", v.display_name()),
                );
                self.modal = Some(Modal::Message {
                    title: "CANNOT OPEN THE STICK".into(),
                    body: format!("'{}' could not be opened:\n\n{e}", v.display_name()),
                    error: true,
                });
                None
            }
        }
    }

    fn eject_volume(&mut self, v: &Volume) {
        let name = v.display_name();
        match media::eject(&self.sys.tools.udisksctl, v) {
            Ok(how) => {
                let extra = match how {
                    Ejected::PoweredOff => String::new(),
                    Ejected::Unmounted(why) => {
                        format!("\n\n(It could not be switched off: {why}.)")
                    }
                };
                self.notify(Level::Info, format!("{name} can be unplugged."));
                self.modal = Some(Modal::Message {
                    title: "SAFE TO UNPLUG".into(),
                    body: format!(
                        "'{name}' can be unplugged now: everything has been written to it.{extra}"
                    ),
                    error: false,
                });
            }
            Err(e) => {
                self.notify(Level::Error, format!("cannot eject {name}: {e}"));
                self.modal = Some(Modal::Message {
                    title: "CANNOT EJECT".into(),
                    body: format!("'{name}' could not be ejected: {e}\n\nDo not unplug it yet."),
                    error: true,
                });
            }
        }
    }

    fn confirm_backup(&mut self, volume: &Volume, mount: &std::path::Path) {
        if volume.read_only {
            self.modal = Some(Modal::Message {
                title: "READ-ONLY STICK".into(),
                body: format!(
                    "'{}' is write-protected, so no backup can be saved on it. Backups on it \
                     can still be checked and restored.",
                    volume.display_name()
                ),
                error: true,
            });
            return;
        }
        let estimate = backup::estimate_size(&self.home);
        let free = status::storage(mount).map(|s| s.available);
        let mut body = format!(
            "Save your home folder (about {}) to '{}'{}?\n\n\
             The backup is encrypted with a passphrase you choose next. Without it nobody \
             can open the backup, not even you. Your files are not changed.",
            human_bytes(estimate),
            volume.display_name(),
            free.map(|f| format!(" ({} free)", human_bytes(f)))
                .unwrap_or_default()
        );
        if free.is_some_and(|f| f < estimate) {
            body.push_str(
                "\n\n[!!] The stick may be too small. Compression often helps; if it is \
                 still too small, the backup stops and nothing is kept.",
            );
        }
        self.modal = Some(Modal::Confirm {
            title: "NEW BACKUP".into(),
            body,
            confirm: "Back up".into(),
            action: Pending::Backup {
                mount: mount.to_path_buf(),
            },
            yes_focused: false,
        });
    }

    fn confirm_restore(&mut self, f: &BackupFile) {
        let mut body = format!(
            "Restore the backup of {}?\n\n\
             Its files go into a new folder, {}/backup-{}. Nothing that exists is \
             changed. You will need this backup's passphrase.",
            backup::human_stamp(&f.stamp),
            self.home.join(backup::RESTORED_DIR).display(),
            f.stamp
        );
        if !f.has_checksum {
            body.push_str(
                "\n\nThis copy has no checksum file; damage will still be detected while restoring.",
            );
        }
        self.modal = Some(Modal::Confirm {
            title: "RESTORE A BACKUP".into(),
            body,
            confirm: "Restore".into(),
            action: Pending::Restore {
                file: f.path.clone(),
            },
            yes_focused: false,
        });
    }

    /// Run `resilience-backup <op> <path>` in the foreground: age asks for
    /// the passphrase on the terminal itself.
    fn backup_tool_effect(&self, label: &str, op: &str, path: &std::path::Path) -> Effect {
        Effect::Run(RunRequest {
            label: label.into(),
            argv: vec![self.sys.tools.backup.clone(), op.into()],
            extra_args: vec![path.to_path_buf()],
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(format!("{}: press Ctrl-C to cancel.", label.to_uppercase())),
        })
    }

    // ----- recovery ----------------------------------------------------------

    fn open_recovery(&mut self, item: RecoveryItem) -> Option<View> {
        match item {
            RecoveryItem::Health => {
                self.refresh_slow();
                Some(View::Pager {
                    title: "SYSTEM HEALTH & RECENT FAILURES".into(),
                    text: self.health_text(),
                    scroll: 0,
                })
            }
            RecoveryItem::Generations => {
                let gens = self.load_generations();
                let mut list = ListState::default();
                list.select((!gens.is_empty()).then_some(0));
                Some(View::Generations { gens, list })
            }
            RecoveryItem::Update => self.open_updates(),
            RecoveryItem::Report => {
                self.export_report();
                None
            }
            RecoveryItem::Emergency => {
                let pages = handbook::list(&self.sys.handbook_dir).unwrap_or_default();
                match pages.iter().find(|p| {
                    p.path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().contains("recovery"))
                }) {
                    Some(p) => Some(self.open_page(&p.clone())),
                    None => Some(View::Pager {
                        title: "EMERGENCY GUIDE".into(),
                        text: EMERGENCY_FALLBACK.into(),
                        scroll: 0,
                    }),
                }
            }
        }
    }

    pub fn health_text(&self) -> String {
        let mut t = String::new();
        let state = match &self.sys_state {
            SystemState::Running => "running: everything started correctly".to_string(),
            SystemState::Degraded => "DEGRADED: some services failed (listed below)".to_string(),
            other => format!("{other:?}"),
        };
        t.push_str(&format!("Overall state: {state}\n\n"));
        if self.failed_units.is_empty() {
            t.push_str("Failed services: none\n");
        } else {
            t.push_str(&format!("Failed services ({}):\n", self.failed_units.len()));
            for f in recovery::failure_details(&self.failed_units) {
                t.push_str(&format!(
                    "  {}\n    {}\n    result: {}   since: {}\n",
                    f.unit, f.description, f.result, f.since
                ));
            }
            t.push_str(
                "\nA failed service is often harmless for daily use. If something you need \
                 does not work, restart the computer; if it still fails, start the previous \
                 system generation from the boot menu (see 'System generations').\n",
            );
        }
        let errors = recovery::recent_errors(30);
        t.push_str("\nRecent error messages (this boot):\n");
        if errors.is_empty() {
            t.push_str("  none that this account may read\n");
        } else {
            for e in &errors {
                t.push_str(&format!("  {e}\n"));
            }
        }
        if self.mode() != Mode::Advanced {
            t.push_str(
                "\nOnly administrators can read every system message. This list shows the \
                 messages your account is allowed to see.\n",
            );
        }
        t
    }

    /// The Recovery entries this system offers.
    pub fn recovery_items(&self) -> Vec<RecoveryItem> {
        RecoveryItem::ALL
            .into_iter()
            .filter(|i| *i != RecoveryItem::Update || self.update_settings().is_some())
            .collect()
    }

    pub fn load_generations(&self) -> Vec<recovery::Generation> {
        recovery::generations(
            &self.paths.profiles,
            &self.paths.booted_system,
            &self.paths.running_system,
        )
    }

    /// Explain what making `g` the default means, then ask. The checks
    /// here only give clear messages; the helper repeats all of them.
    fn confirm_default_generation(&mut self, g: &recovery::Generation) {
        let gens = self.load_generations();
        if let Err(r) = recovery::plan_set_default(&gens, &g.number.to_string(), &self.paths.store)
        {
            self.modal = Some(Modal::Message {
                title: "NOT POSSIBLE".into(),
                body: format!("{}.", capitalise(&r.to_string())),
                error: !matches!(r, recovery::DefaultRefusal::AlreadyDefault(_)),
            });
            return;
        }
        if !self.has_administrator("Changing the default generation") {
            return;
        }
        let newest = gens.iter().map(|g| g.number).max().unwrap_or(g.number);
        let mut body = format!(
            "Start generation {} ({}, created {}) at every start of the computer?\n\n",
            g.number,
            g.nixos_version.as_deref().unwrap_or("unknown version"),
            g.created
                .map(crate::fmt::date_time)
                .unwrap_or_else(|| "?".into()),
        );
        if g.number < newest {
            body.push_str(
                "This goes back to older system software and older system settings. \
                 Your personal files are not touched. Newer generations stay in the \
                 boot menu and can be made the default again.\n\n",
            );
        }
        body.push_str(
            "Nothing changes until the next restart. An administrator must type \
             their password next; the change is recorded in the audit log.",
        );
        self.modal = Some(Modal::Confirm {
            title: "DEFAULT GENERATION".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::SetDefaultGeneration { number: g.number },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> set-default-generation <N>`.
    fn set_default_generation_effect(&mut self, number: u64) -> Effect {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("changing the default generation is unavailable: {e}"),
                );
                return Effect::None;
            }
        };
        tracing::warn!(generation = number, "default generation change requested");
        Effect::Run(RunRequest {
            label: SET_DEFAULT_GENERATION_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-default-generation".into(),
                number.to_string(),
            ],
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "DEFAULT GENERATION {number}. An administrator must now type their \
                 password. To cancel, press Ctrl-C while the password is asked."
            )),
        })
    }

    pub fn generations_text(&self) -> String {
        let gens = recovery::generations(
            &self.paths.profiles,
            &self.paths.booted_system,
            &self.paths.running_system,
        );
        let mut t = String::from(
            "Every change to the system creates a new generation; earlier ones are kept \
             and can be started from the boot menu. Starting an earlier generation changes \
             only the system software and its settings, never your personal files.\n\n",
        );
        if gens.is_empty() {
            t.push_str(
                "No generations are recorded on this machine. This is normal on the live \
                 system, which always starts the same way.\n",
            );
        } else {
            t.push_str(&format!(
                "{:>4}  {:<16}  {:<24}  {}\n",
                "#", "CREATED", "VERSION", "STATE"
            ));
            for g in &gens {
                let mut marks = Vec::new();
                if g.booted {
                    marks.push("started now");
                }
                if g.running && !g.booted {
                    marks.push("active");
                }
                if g.default {
                    marks.push("default at next boot");
                }
                t.push_str(&format!(
                    "{:>4}  {:<16}  {:<24}  {}\n",
                    g.number,
                    g.created
                        .map(crate::fmt::date_time)
                        .unwrap_or_else(|| "?".into()),
                    g.nixos_version.as_deref().unwrap_or("?"),
                    marks.join(", ")
                ));
                if let Some(k) = &g.kernel {
                    t.push_str(&format!("{:>4}  kernel {k}\n", ""));
                }
            }
        }
        t.push_str(
            "\nTo start an earlier generation:\n\
             \x20 1. Restart the computer (System > Restart computer).\n\
             \x20 2. When the boot menu appears, use the arrow keys to choose an older entry\n\
             \x20    (on some menus it is under 'NixOS - All configurations').\n\
             \x20 3. Press Enter. Only this start uses the older generation: the next\n\
             \x20    restart returns to the newest one, unless an administrator makes\n\
             \x20    the older generation permanent.\n",
        );
        t
    }

    pub fn diagnostic_report(&self) -> String {
        let now = admin::now_rfc3339();
        let mut t = format!(
            "RESILIENCE OS DIAGNOSTIC REPORT\n\
             Created: {now} (UTC)\n\
             Contains: system status, failed services, generations, network link state\n\
             (no addresses), and the last lines of the interface log. No passwords, no\n\
             file names from your home directory.\n\n"
        );
        let s = &self.snapshot;
        t.push_str(&format!(
            "Resilience: {} (edition {})\nOS: {}\nKernel: {}\nGeneration: {}\nUser: {} ({} mode)\nUptime: {}\n",
            resilience_core::VERSION,
            self.sys.edition,
            s.os_name,
            s.kernel,
            s.system_generation.as_deref().unwrap_or("?"),
            self.who.user,
            self.mode(),
            s.uptime.map(status::format_uptime).unwrap_or_else(|| "?".into()),
        ));
        if let Some(m) = &s.memory {
            t.push_str(&format!(
                "Memory: {} available of {}\n",
                resilience_core::human_bytes(m.available_kib * 1024),
                resilience_core::human_bytes(m.total_kib * 1024)
            ));
        }
        for (label, d) in [("system", &s.root), ("home", &s.home)] {
            if let Some(d) = d {
                t.push_str(&format!(
                    "Storage ({label}): {} free of {}\n",
                    resilience_core::human_bytes(d.available),
                    resilience_core::human_bytes(d.total)
                ));
            }
        }
        t.push_str(&format!("Network: {:?}\n", self.net.level));
        for i in &self.net.interfaces {
            t.push_str(&format!(
                "  {} {:?} {}\n",
                i.name,
                i.kind,
                if i.is_connected() {
                    "link up"
                } else {
                    "no link"
                }
            ));
        }
        t.push_str("\n--- Health ---\n");
        t.push_str(&self.health_text());
        t.push_str("\n--- Generations ---\n");
        t.push_str(&self.generations_text());
        t.push_str("\n--- Interface log (last 50 lines) ---\n");
        let log = resilience_core::config::state_home(&self.home)
            .join("resilience")
            .join("tui.log");
        for l in recovery::tail(&log, 50) {
            t.push_str(&l);
            t.push('\n');
        }
        t
    }

    fn export_report(&mut self) {
        let name = recovery::report_file_name(&admin::now_rfc3339());
        let dir = recovery::reports_dir(&self.home);
        match recovery::write_report(&dir, &name, &self.diagnostic_report()) {
            Ok(path) => {
                self.notify(Level::Info, format!("Report saved: {}", path.display()));
                self.modal = Some(Modal::Message {
                    title: "REPORT SAVED".into(),
                    body: format!(
                        "The diagnostic report was saved as:\n\n  {}\n\n\
                         It is a plain text file. Copy it to a USB stick or show it to \
                         someone who can help. It contains no passwords.",
                        path.display()
                    ),
                    error: false,
                });
            }
            Err(e) => {
                self.notify(Level::Error, format!("report not saved: {e}"));
                self.modal = Some(Modal::Message {
                    title: "REPORT NOT SAVED".into(),
                    body: format!(
                        "The report could not be written to {}:\n\n{e}",
                        dir.display()
                    ),
                    error: true,
                });
            }
        }
    }
}

/// The modes offered by the mode chooser, least privileged first.
pub const MODES: [Mode; 3] = [Mode::UltraStable, Mode::Stable, Mode::Advanced];

const EMERGENCY_FALLBACK: &str = "The handbook is not available.\n\n\
1. Restart the computer.\n\
2. In the boot menu, choose an earlier entry and press Enter.\n\
3. If the interface does not start, press Ctrl-Alt-F2 and log in with an\n\
   administrator account.\n\
4. Your files are in your home folder; they are not changed by these steps.";

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

pub fn level_tag(l: Level) -> &'static str {
    match l {
        Level::Info => "[--]",
        Level::Warn => "[!!]",
        Level::Error => "[XX]",
    }
}

fn category_view(cat: Category) -> View {
    View::Category {
        cat,
        list: ListState::default().with_selected(Some(0)),
    }
}

fn digit(key: KeyEvent) -> Option<usize> {
    match key.code {
        KeyCode::Char(c @ '1'..='9') => Some(c as usize - '1' as usize),
        _ => None,
    }
}

fn is_activate(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
    ) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_back(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h')
    )
}

fn clamp_selection(list: &mut ListState, len: usize) {
    if len == 0 {
        list.select(None);
    } else {
        list.select(Some(list.selected().unwrap_or(0).min(len - 1)));
    }
}

/// Up/Down/Home/End/PgUp/PgDn (and j/k) list navigation with wrap-around.
fn nav(list: &mut ListState, len: usize, key: KeyEvent) {
    if len == 0 {
        list.select(None);
        return;
    }
    let cur = list.selected().unwrap_or(0).min(len - 1);
    let next = match key.code {
        KeyCode::Up | KeyCode::Char('k') => (cur + len - 1) % len,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => (cur + 1) % len,
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => len - 1,
        KeyCode::PageUp => cur.saturating_sub(10),
        KeyCode::PageDown => (cur + 10).min(len - 1),
        _ => cur,
    };
    list.select(Some(next));
}

/// Scroll keys for read-only text. Rendering clamps the upper bound.
fn scroll_key(scroll: &mut usize, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
        KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
        KeyCode::PageDown | KeyCode::Char(' ') => *scroll += 10,
        KeyCode::Home | KeyCode::Char('g') => *scroll = 0,
        KeyCode::End | KeyCode::Char('G') => *scroll = usize::MAX / 2,
        _ => {}
    }
}
