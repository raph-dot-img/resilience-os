//! TUI behaviour tests: navigation state machine and rendering at 80x24,
//! using ratatui's in-memory TestBackend (no real terminal needed).

use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use resilience_core::admin;
use resilience_core::catalog::{Catalog, Category};
use resilience_core::clock;
use resilience_core::config::{SystemConfig, ThemeName, ToolPaths, UserSettings};
use resilience_core::mode::{Detected, Mode};
use resilience_core::packs::PackKind;
use resilience_shell::input::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{
    App, BACKUP_LABEL, Builtin, CHECK_LABEL, CRYPTO_LABEL, Effect, EncryptTarget, Entry,
    INSTALL_LABEL, InstallForm, InstallInput, InstallRow, ItemPlace, MAKE_MAP_LABEL, MediaPurpose,
    Modal, PACKS_LABEL, PackRow, Paths, Pending, Pick, RESTORE_LABEL, Recipient, SearchHit,
    Startup, UPDATE_APPLY_LABEL, UPDATE_CHECK_LABEL, View, stick_items,
};
use crate::ui;

const CATALOG: &str = r#"
[[apps]]
id = "editor"
name = "Text editor"
category = "productivity"
description = "Edit text files"
command = ["/bin/sh", "-c", "true"]
restricted_command = ["/bin/sh", "-c", "restricted"]

[[apps]]
id = "monitor"
name = "Process monitor"
category = "system"
description = "Advanced-only tool"
command = ["/bin/sh"]
min_mode = "advanced"

[[apps]]
id = "ghost"
name = "Ghost game"
category = "entertainment"
description = "Program is missing"
command = ["/nonexistent/ghost"]
"#;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("resilience-tui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn app(mode: Mode, tag: &str) -> App {
    let home = tmp(tag);
    let hb = home.join("handbook");
    std::fs::create_dir_all(&hb).expect("mkdir");
    std::fs::write(
        hb.join("10-first.md"),
        "# First steps\nWelcome to Resilience.\n",
    )
    .expect("w");
    let sys = SystemConfig {
        handbook_dir: hb,
        edition: "test".into(),
        tools: fake_media_tools(&home),
        ..SystemConfig::default()
    };
    App::new(Startup {
        sys,
        catalog: Catalog::parse(CATALOG, Path::new("test")).expect("catalog"),
        who: Detected {
            mode,
            user: "tester".into(),
            groups: vec![],
        },
        settings_path: UserSettings::path(&home.join(".config")),
        paths: test_paths(&home),
        home,
        settings: UserSettings::default(),
        warnings: vec![],
    })
}

/// A fake system: helper configuration with three accounts (demo logs in
/// automatically, bob is Stable, carol is the only administrator), the group
/// database matching it, and no system generations.
fn test_paths(home: &Path) -> Paths {
    let sys = home.join("sys");
    std::fs::create_dir_all(sys.join("profiles")).expect("mkdir");
    std::fs::write(
        sys.join("admin.toml"),
        format!(
            r#"
state_file = "{0}/modes.toml"
audit_log = "{0}/audit.log"
lock_file = "{0}/lock"
gpasswd = "/bin/gpasswd"
loginctl = "/bin/loginctl"
timedatectl = "/bin/timedatectl"
nix_env = "/bin/nix-env"
helper = "/store/bin/resilience-admin"
pkexec = "/run/wrappers/bin/pkexec"
pkttyagent = "/store/bin/pkttyagent"
session_shell = "/run/current-system/sw/bin/resilience-session"
autologin_user = "demo"
[users.demo]
declared_mode = "ultra-stable"
[users.bob]
declared_mode = "stable"
[users.carol]
declared_mode = "advanced"
description = "Carol the admin"
"#,
            sys.display()
        ),
    )
    .expect("write admin.toml");
    std::fs::write(
        sys.join("group"),
        "wheel:x:1:carol\nresilience-stable:x:990:bob,carol\nnetworkmanager:x:57:bob,carol\n",
    )
    .expect("write group");
    write_packs(&sys);
    Paths {
        admin_config: sys.join("admin.toml"),
        group_file: sys.join("group"),
        profiles: sys.join("profiles"),
        store: sys.join("store"),
        booted_system: sys.join("booted"),
        running_system: sys.join("current"),
        packs_config: sys.join("packs.toml"),
        // Absent: local messaging is off unless a test creates the file.
        comms_client: sys.join("comms-client-config"),
        // Absent: the mesh is off.
        mesh_devices: sys.join("mesh-devices"),
        // Absent: not sharing.
        share_state: sys.join("share-state.toml"),
        route4: sys.join("route"),
        route6: sys.join("ipv6_route"),
        sys_net: sys.join("net"),
        shared_dir: sys.join("shared"),
        hotspot_state: sys.join("hotspot"),
        // Absent: not a live system (no installer).
        install_config: sys.join("install.toml"),
        mapmaker_config: sys.join("mapmaker.toml"),
        efi: sys.join("efi"),
    }
}

/// A catalog of three packs in `sys/packs.toml`: "wiki-en" (installed),
/// "medicine-en" (half downloaded) and "wiki-fr" (absent), plus an older
/// version of wiki-en still on disk.
fn write_packs(sys: &Path) {
    let dir = sys.join("packs");
    std::fs::create_dir_all(dir.join(".staging")).expect("mkdir");
    let entry = |id: &str, title: &str, cat: &str, lang: &str, file: &str, size: u64| {
        format!(
            "[[pack]]\nid = \"{id}\"\ntitle = \"{title}\"\ndescription = \"About {id}\"\n\
             category = \"{cat}\"\nlanguage = \"{lang}\"\nfile = \"{file}\"\nsize = {size}\n\
             sha256 = \"{}\"\nurls = [\"https://example.org/{file}\"]\nlicense = \"CC-BY-SA-4.0\"\n\
             source = \"Test\"\n",
            "a".repeat(64)
        )
    };
    let text = format!(
        "dir = \"{}\"\nstore_user = \"resilience-packs\"\nstore = \"/store/bin/resilience-pack-store\"\n\
         pkexec = \"/run/wrappers/bin/pkexec\"\npkttyagent = \"/store/bin/pkttyagent\"\n\
         curl = \"/store/bin/curl\"\nsha256sum = \"/store/bin/sha256sum\"\n\
         kiwix_serve = \"/store/bin/kiwix-serve\"\nbrowser = [\"/store/bin/lynx\"]\n\
         session_shell = \"/run/current-system/sw/bin/resilience-session\"\n{}{}{}",
        dir.display(),
        entry(
            "wiki-en",
            "Wikipedia (English)",
            "encyclopedia",
            "en",
            "wiki_en_2026-09.zim",
            10
        ),
        entry(
            "medicine-en",
            "Medicine (English)",
            "health",
            "en",
            "med_en_2026-10.zim",
            4000
        ),
        entry(
            "wiki-fr",
            "Wikipedia (French)",
            "encyclopedia",
            "fr",
            "wiki_fr_2026-10.zim",
            3000
        ),
    ) + &format!(
        "[[set]]\nid = \"starter\"\ntitle = \"Starter set\"\ndescription = \"A start\"\n\
         packs = [\"wiki-en\", \"wiki-fr\", \"medicine-en\"]\n\
         [[builtin]]\ntitle = \"System documentation (built in)\"\ndescription = \"Docs\"\n\
         category = \"documentation\"\nlanguage = \"en\"\npath = \"{}\"\nlicense = \"MIT\"\n\
         source = \"Test\"\n",
        sys.join("docs.zim").display()
    );
    std::fs::write(sys.join("docs.zim"), b"docs").expect("w");
    std::fs::write(sys.join("packs.toml"), text).expect("write packs.toml");
    std::fs::write(dir.join("wiki_en_2026-09.zim"), b"0123456789").expect("w");
    std::fs::write(dir.join("wiki_en_2026-03.zim"), b"old").expect("w");
    std::fs::write(
        dir.join(".staging/med_en_2026-10.zim.part"),
        vec![0u8; 1000],
    )
    .expect("w");
}

/// Fake `lsblk` and `udisksctl`: an internal disk, a USB stick (mounted
/// once `udisksctl mount` ran) and an encrypted SD card. udisksctl records
/// its arguments in `sys/udisks.log`; `sys/busy` makes unmount fail.
fn fake_media_tools(home: &Path) -> ToolPaths {
    use std::os::unix::fs::PermissionsExt;
    let sys = home.join("sys");
    std::fs::create_dir_all(&sys).expect("mkdir");
    let d = sys.display();
    let lsblk = format!(
        r#"#!/bin/sh
if [ -f {d}/mounted ]; then MP="/run/media/tester/STICK"; AV=1000000; else MP=""; AV=""; fi
cat <<LSBLK
NAME="vdb" PATH="/dev/vdb" PKNAME="" TYPE="disk" RM="0" HOTPLUG="0" RO="0" SIZE="67108864" LABEL="INTERNAL" FSTYPE="ext4" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
NAME="sda" PATH="/dev/sda" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="0" SIZE="134217728" LABEL="" FSTYPE="" MOUNTPOINT="" FSAVAIL="" TRAN="usb" MODEL="Stick"
NAME="sda1" PATH="/dev/sda1" PKNAME="sda" TYPE="part" RM="0" HOTPLUG="0" RO="0" SIZE="134217728" LABEL="STICK" FSTYPE="vfat" MOUNTPOINT="$MP" FSAVAIL="$AV" TRAN="" MODEL=""
NAME="mmcblk0" PATH="/dev/mmcblk0" PKNAME="" TYPE="disk" RM="0" HOTPLUG="1" RO="0" SIZE="32000000000" LABEL="SECRET" FSTYPE="crypto_LUKS" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
LSBLK
"#
    );
    let udisksctl = format!(
        r#"#!/bin/sh
echo "$@" >> {d}/udisks.log
case "$1" in
  mount) touch {d}/mounted; echo "Mounted /dev/sda1 at /run/media/tester/STICK" ;;
  unmount)
    if [ -f {d}/busy ]; then
      echo "Error unmounting /dev/sda1: GDBus.Error:org.freedesktop.UDisks2.Error.DeviceBusy: Error unmounting /dev/sda1: target is busy" >&2
      exit 1
    fi
    rm -f {d}/mounted ;;
esac
"#
    );
    // Fake nmcli: one Wi-Fi device, a saved network "Home" (not active), a
    // network managed by an administrator ("Shared", its profile cannot be
    // deleted by this account), and a few visible networks. Every call is
    // logged to sys/nmcli.log; profiles live in sys/nm-profiles.
    std::fs::write(
        sys.join("nm-profiles"),
        "Home:0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10:802-11-wireless:no\n\
         Shared:5f0c1a52-1b6f-4a39-9d1a-2a4c9d2c8e01:802-11-wireless:no\n\
         Wired:6a1c1a52-1b6f-4a39-9d1a-2a4c9d2c8e02:802-3-ethernet:yes\n",
    )
    .expect("write profiles");
    let nmcli = format!(
        r#"#!/bin/sh
echo "$@" >> {d}/nmcli.log
P={d}/nm-profiles
case "$*" in
  "-t -f TYPE device") echo ethernet; [ -f {d}/no-wifi ] || echo wifi ;;
  "-t radio wifi") if [ -f {d}/radio-off ]; then echo disabled; else echo enabled; fi ;;
  "radio wifi on") rm -f {d}/radio-off ;;
  "-t -f NAME,UUID,TYPE,ACTIVE connection show") cat $P ;;
  "-t -f IN-USE,SSID,SECURITY,SIGNAL device wifi list --rescan no")
    if grep -q '^Cafe:.*:yes$' $P; then C='*'; else C=''; fi
    cat <<NM
:Home:WPA2:70
$C:Cafe:WPA2:55
:Cafe:WPA2:30
:Library:--:40
:Corp:WPA2 802.1X:80
:Old:WEP:20
:Shared:WPA2:35
NM
    ;;
  "device wifi rescan") ;;
  "connection add "*)
    if [ -f {d}/deny-add ]; then echo "Error: Failed to add 'x' connection: Insufficient privileges" >&2; exit 1; fi
    shift 7; name="$1"; shift 4; uuid="$1"
    echo "$name:$uuid:802-11-wireless:no" >> $P ;;
  "connection delete uuid 5f0c1a52-1b6f-4a39-9d1a-2a4c9d2c8e01")
    echo "Error: Connection deletion failed: Insufficient privileges" >&2; exit 1 ;;
  "connection delete uuid "*) grep -v ":$4:" $P > $P.new; mv $P.new $P ;;
  "connection down uuid "*) sed -i "s/:$4:802-11-wireless:yes/:$4:802-11-wireless:no/" $P ;;
  "--wait 45 connection up uuid "*" passwd-file /dev/stdin")
    # The secret arrives on stdin only; keep it for the test to check.
    cat > {d}/nm-stdin
    if [ "$(cat {d}/nm-stdin)" != "802-11-wireless-security.psk:right-password" ]; then
      echo "Error: Connection activation failed: Secrets were required, but not provided." >&2; exit 4
    fi
    sed -i "s/:$6:802-11-wireless:no/:$6:802-11-wireless:yes/" $P ;;
  "--wait 45 connection up uuid "*)
    if [ -f {d}/forgot-secret ] || [ "$6" = 0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10 -a -f {d}/home-refused ]; then
      echo "Error: Connection activation failed: Secrets were required, but not provided." >&2; exit 4
    fi
    sed -i "s/:$6:802-11-wireless:no/:$6:802-11-wireless:yes/" $P ;;
  *) echo "Error: unexpected arguments: $*" >&2; exit 2 ;;
esac
"#
    );
    // Fake man: knows only ls; -k finds it by its description.
    let man = r#"#!/bin/sh
case "$1" in
  -w) [ "$2" = ls ] && echo /share/man/man1/ls.1.gz || exit 16 ;;
  --and) echo "ls (1)               - list directory contents" ;;
  *) exit 3 ;;
esac
"#
    .to_string();
    for (name, text) in [
        ("lsblk", lsblk),
        ("udisksctl", udisksctl),
        ("nmcli", nmcli),
        ("man", man),
        (
            "pdftotext",
            "#!/bin/sh\necho \"PDF TEXT of $6\"\n".to_string(),
        ),
        (
            "xkcdpass",
            "#!/bin/sh\necho 'With the current options, your word list contains 7227 words.'\n\
             echo 'A 6 word password from this list will have roughly 76 (12.82 * 6) bits of entropy,'\n\
             echo\necho \"graceful encore snooze reoccur factor $5\"\n"
                .to_string(),
        ),
        (
            // Fake gpg: one OpenPGP contact (Bob), no own key; deletions logged.
            "gpg",
            format!(
                "#!/bin/sh\necho \"$@\" >> {d}/gpg.log\n\
                 case \"$*\" in\n\
                 *--list-secret-keys*) ;;\n\
                 *--list-keys*) cat <<K\n\
                 pub:-:255:22:B0B0B0B0B0B0B0B0:1794734011:::-:::scESC:::::ed25519:::0:\n\
                 fpr:::::::::0123456789ABCDEF0123456789ABCDEFB0B0B0B0:\n\
                 uid:-::::1794734011::AA::Bob Builder <bob@example.org>::::::::::0:\n\
                 K\n;;\n\
                 esac\n"
            ),
        ),
        (
            "openssl",
            "#!/bin/sh\n[ \"$2\" = -hex ] && echo 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff || echo 'q2Fz+R0b/x9kLm3nT7uVwYzA'\n"
                .to_string(),
        ),
        (
            "bsdtar",
            "#!/bin/sh\n[ \"$1\" = -tvf ] && echo \"-rw-r--r-- 0 a a 5 Jan 1 2026 inside.txt\"\n"
                .to_string(),
        ),
    ] {
        let p = sys.join(name);
        std::fs::write(&p, text).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    ToolPaths {
        lsblk: sys.join("lsblk").display().to_string(),
        udisksctl: sys.join("udisksctl").display().to_string(),
        backup: "/store/bin/resilience-backup".into(),
        nmcli: sys.join("nmcli").display().to_string(),
        man: sys.join("man").display().to_string(),
        apropos: sys.join("man").display().to_string(),
        less: "/store/bin/less".into(),
        pdftotext: sys.join("pdftotext").display().to_string(),
        bsdtar: sys.join("bsdtar").display().to_string(),
        crypto: "/store/bin/resilience-crypto".into(),
        xkcdpass: sys.join("xkcdpass").display().to_string(),
        openssl: sys.join("openssl").display().to_string(),
        gpg: sys.join("gpg").display().to_string(),
        ..ToolPaths::default()
    }
}

fn udisks_log(a: &App) -> String {
    std::fs::read_to_string(a.home.join("sys/udisks.log")).unwrap_or_default()
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn press(app: &mut App, c: KeyCode) -> Effect {
    app.handle_key(key(c))
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    t.draw(|f| ui::draw(f, app)).expect("draw");
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn home_renders_at_80x24() {
    let mut a = app(Mode::UltraStable, "home");
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("MAIN MENU"), "{screen}");
    assert!(screen.contains("STATUS"));
    assert!(screen.contains("ULTRA-STABLE"));
    assert!(screen.contains("[1] KNOWLEDGE & DOCUMENTATION"));
    assert!(screen.contains("F1 Help"));
    // Maps come with the resource packs of the system.
    assert!(screen.contains("[2] MAPS & NAVIGATION"), "{screen}");
    // Empty categories (radio...) are not offered.
    assert!(!screen.contains("RADIO"));
}

#[test]
fn renders_every_view_without_panicking_at_several_sizes() {
    let mut a = app(Mode::Advanced, "sizes");
    let opens: [&[KeyCode]; 6] = [
        &[],
        &[KeyCode::F(1)],
        &[KeyCode::F(5)],
        &[KeyCode::Char('/'), KeyCode::Char('n')],
        &[KeyCode::Char('1')],
        &[KeyCode::Char('1'), KeyCode::Enter],
    ];
    for seq in opens {
        for c in seq {
            press(&mut a, *c);
        }
        for (w, h) in [(80, 24), (40, 12), (120, 40), (20, 5), (200, 60)] {
            let s = render(&mut a, w, h);
            if w < 40 || h < 12 {
                assert!(s.contains("Terminal"), "small terminal notice at {w}x{h}");
            }
        }
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

#[test]
fn ultra_stable_has_no_shell_or_admin_entries() {
    let a = app(Mode::UltraStable, "ultra");
    let sys = a.entries(Category::System);
    assert!(!sys.contains(&Entry::Builtin(Builtin::Shell)));
    assert!(
        sys.iter().all(|e| !matches!(e, Entry::App(_))),
        "advanced-only app hidden"
    );
    assert!(
        a.search("shell").is_empty()
            || a.search("terminal").iter().all(|h| !matches!(
                h,
                crate::app::SearchHit::Entry(Entry::Builtin(Builtin::Shell))
            ))
    );
}

#[test]
fn stable_and_advanced_get_more() {
    let s = app(Mode::Stable, "stable");
    assert!(
        s.entries(Category::System)
            .contains(&Entry::Builtin(Builtin::Shell))
    );
    assert!(
        !s.entries(Category::System)
            .iter()
            .any(|e| matches!(e, Entry::App(_)))
    );
    let adv = app(Mode::Advanced, "adv");
    assert!(
        adv.entries(Category::System)
            .iter()
            .any(|e| matches!(e, Entry::App(_)))
    );
}

#[test]
fn ultra_stable_uses_restricted_command() {
    let mut a = app(Mode::UltraStable, "restricted");
    let cats = a.home_categories();
    let idx = cats
        .iter()
        .position(|c| *c == Category::Productivity)
        .expect("productivity");
    press(&mut a, KeyCode::Char(char::from(b'1' + idx as u8)));
    // Entries: [Notes builtin, Files builtin, Text editor app]
    let eff = press(&mut a, KeyCode::Char('3'));
    match eff {
        Effect::Run(r) => assert_eq!(r.argv, vec!["/bin/sh", "-c", "restricted"]),
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn missing_program_shows_error_not_crash() {
    let mut a = app(Mode::UltraStable, "missing");
    let cats = a.home_categories();
    let idx = cats
        .iter()
        .position(|c| *c == Category::Entertainment)
        .expect("entertainment");
    press(&mut a, KeyCode::Char(char::from(b'1' + idx as u8)));
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(a.modal, Some(Modal::Message { error: true, .. })));
    assert!(render(&mut a, 80, 24).contains("PROGRAM UNAVAILABLE"));
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
}

#[test]
fn power_off_requires_confirmation_and_defaults_to_cancel() {
    let mut a = app(Mode::UltraStable, "power");
    let (_, eff) = a.activate(Entry::Builtin(Builtin::PowerOff));
    assert_eq!(eff, Effect::None);
    assert!(matches!(
        a.modal,
        Some(Modal::Confirm {
            yes_focused: false,
            ..
        })
    ));
    // Enter on the default button cancels: nothing happens.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
}

#[test]
fn logout_confirmation_quits() {
    let mut a = app(Mode::UltraStable, "logout");
    assert_eq!(press(&mut a, KeyCode::Char('q')), Effect::None);
    assert!(a.modal.is_some());
    assert_eq!(press(&mut a, KeyCode::Char('n')), Effect::None);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Char('q'));
    assert_eq!(press(&mut a, KeyCode::Char('y')), Effect::Quit);
}

#[test]
fn ctrl_c_and_esc_never_leave_home() {
    let mut a = app(Mode::UltraStable, "ctrlc");
    assert_eq!(
        a.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Effect::None
    );
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.stack.len(), 1);
    assert!(matches!(a.view(), View::Home { .. }));
}

#[test]
fn handbook_opens_and_reads() {
    let mut a = app(Mode::UltraStable, "handbook");
    press(&mut a, KeyCode::Char('1')); // Knowledge
    press(&mut a, KeyCode::Enter); // Handbook
    assert!(matches!(a.view(), View::Handbook { .. }));
    press(&mut a, KeyCode::Enter); // First page
    match a.view() {
        View::Pager { title, text, .. } => {
            assert_eq!(title, "FIRST STEPS");
            assert!(text.contains("Welcome"));
        }
        _ => panic!("expected pager"),
    }
    assert!(render(&mut a, 80, 24).contains("Welcome to Resilience"));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.stack.len(), 1);
}

#[test]
fn search_finds_apps_tools_and_pages() {
    let mut a = app(Mode::UltraStable, "search");
    press(&mut a, KeyCode::Char('/'));
    for c in "first".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    let View::Search { hits, .. } = a.view() else {
        panic!("search view")
    };
    assert!(
        hits.iter()
            .any(|h| matches!(h, crate::app::SearchHit::Page(_)))
    );
    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    for c in "editor".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    // '?' and 'q' are typed into the query, not interpreted as commands.
    press(&mut a, KeyCode::Char('?'));
    let View::Search { query, .. } = a.view() else {
        panic!("search view")
    };
    assert_eq!(query, "editor?");
}

#[test]
fn new_note_validates_name_and_opens_editor() {
    let mut a = app(Mode::UltraStable, "notes");
    let (v, _) = a.activate(Entry::Builtin(Builtin::Notes));
    a.stack.push(v.expect("notes view"));
    press(&mut a, KeyCode::Char('n'));
    for c in "../bad".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(
        a.modal,
        Some(Modal::NewNote { error: Some(_), .. })
    ));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('n'));
    for c in "groceries".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => {
            assert_eq!(r.argv, a.sys.restricted_editor);
            assert!(r.extra_args[0].ends_with("Notes/groceries.txt"));
        }
        other => panic!("expected editor, got {other:?}"),
    }
    let View::Notes { notes, .. } = a.view() else {
        panic!("notes view")
    };
    assert_eq!(notes.len(), 1);
}

#[test]
fn theme_change_is_saved() {
    let mut a = app(Mode::UltraStable, "theme");
    let (v, _) = a.activate(Entry::Builtin(Builtin::Settings));
    a.stack.push(v.expect("settings"));
    press(&mut a, KeyCode::Home);
    for _ in 0..3 {
        press(&mut a, KeyCode::Down); // Mono, the last theme
    }
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.settings.theme, ThemeName::Mono);
    let (loaded, _) = UserSettings::load(&a.settings_path);
    assert_eq!(loaded.theme, ThemeName::Mono);
    assert!(render(&mut a, 80, 24).contains("(current)"));
}

#[test]
fn shell_request_comes_from_system_config() {
    let mut a = app(Mode::Stable, "shell");
    let (_, eff) = a.activate(Entry::Builtin(Builtin::Shell));
    match eff {
        Effect::Run(r) => assert_eq!(r.argv, a.sys.shell),
        other => panic!("expected shell, got {other:?}"),
    }
    // Defence in depth: even if requested directly, ultra-stable gets nothing.
    let mut u = app(Mode::UltraStable, "shell-ultra");
    let (_, eff) = u.activate(Entry::Builtin(Builtin::Shell));
    assert_eq!(eff, Effect::None);
}

#[test]
fn wrap_preserves_content() {
    use ratatui::text::Line;
    let long = Line::from("alpha beta gamma delta epsilon zeta eta theta iota kappa");
    let out = ui::wrap(vec![long], 12);
    assert!(out.iter().all(|l| l.width() <= 12));
    let joined: Vec<String> = out
        .iter()
        .map(|l| l.to_string().trim().to_string())
        .collect();
    assert_eq!(
        joined.join(" "),
        "alpha beta gamma delta epsilon zeta eta theta iota kappa"
    );
    let huge = ui::wrap(vec![Line::from("x".repeat(50))], 10);
    assert_eq!(huge.len(), 5);
}

/// `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` prints screens
/// for visual review.
#[test]
fn dump_screens() {
    if std::env::var_os("RESILIENCE_DUMP").is_none() {
        return;
    }
    let mut a = app(Mode::Stable, "dump");
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Char('4'));
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    open(&mut a, Builtin::Users);
    press(&mut a, KeyCode::Down);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::End);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    open(&mut a, Builtin::Recovery);
    println!("{}\n", render(&mut a, 80, 24));
    make_generations(&a);
    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Down);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    open(&mut a, Builtin::Backup);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    a.modal = None;
    open(&mut a, Builtin::Packs);
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' '));
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    a.net.level = resilience_core::network::LinkLevel::Routed;
    press(&mut a, KeyCode::Char('d'));
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    open(&mut a, Builtin::Library);
    println!("{}\n", render(&mut a, 80, 24));
    // Cryptography.
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    press(&mut a, KeyCode::Char('3'));
    println!("{}\n", render(&mut a, 80, 24));
    make_keys(&a);
    open(&mut a, Builtin::Keys);
    println!("{}\n", render(&mut a, 80, 24));
    std::fs::write(a.home.join("plan.txt"), "x").expect("w");
    open(&mut a, Builtin::Encrypt);
    type_text(&mut a, "plan");
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    open(&mut a, Builtin::Random);
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
}

/// Every non-ASCII glyph the interface draws must exist in the console font
/// (ter-i16n, CP437 repertoire). ter-v16n once rendered the logo as garbage
/// on real hardware; the unit tests could not see it. Keep this list in sync
/// with nix/modules/interface.nix.
const CONSOLE_GLYPHS: &str = "▀▄█═║╔╗╚╝─│┌┐└┘░▒▓";

fn assert_console_glyphs(screen: &str) {
    for ch in screen.chars() {
        assert!(
            ch.is_ascii() || CONSOLE_GLYPHS.contains(ch),
            "glyph {ch:?} (U+{:04X}) is not in the console font",
            ch as u32
        );
    }
}

#[test]
fn only_console_font_glyphs_are_drawn() {
    let mut a = app(Mode::Advanced, "glyphs");
    let sequences: [&[KeyCode]; 7] = [
        &[],
        &[KeyCode::F(1)],
        &[KeyCode::F(5)],
        &[KeyCode::Char('/'), KeyCode::Char('e')],
        &[KeyCode::Char('2')],
        &[KeyCode::Char('q')],
        &[KeyCode::Char('1'), KeyCode::Enter],
    ];
    for seq in sequences {
        for c in seq {
            press(&mut a, *c);
        }
        for (w, h) in [(80, 24), (60, 20), (160, 50)] {
            for ch in render(&mut a, w, h).chars() {
                assert!(
                    ch.is_ascii() || CONSOLE_GLYPHS.contains(ch),
                    "glyph {ch:?} (U+{:04X}) is not in the console font",
                    ch as u32
                );
            }
        }
        a.modal = None;
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

fn open(a: &mut App, b: Builtin) {
    let (v, e) = a.activate(Entry::Builtin(b));
    assert_eq!(e, Effect::None);
    a.stack.push(v.expect("builtin opens a view"));
}

/// Move the selection of the top list view to the row whose text matches.
fn select_user(a: &mut App, name: &str) {
    let View::Users { rows, .. } = a.view() else {
        panic!("not on the users view")
    };
    let i = rows
        .iter()
        .position(|r| r.name == name)
        .expect("user listed");
    press(a, KeyCode::Home);
    for _ in 0..i {
        press(a, KeyCode::Down);
    }
}

#[test]
fn users_and_recovery_are_offered_in_every_mode() {
    for m in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let a = app(m, &format!("offer-{m:?}"));
        let sys = a.entries(Category::System);
        assert!(sys.contains(&Entry::Builtin(Builtin::Users)), "{m:?}");
        assert!(sys.contains(&Entry::Builtin(Builtin::Recovery)), "{m:?}");
        assert!(sys.contains(&Entry::Builtin(Builtin::Backup)), "{m:?}");
    }
}

#[test]
fn mode_change_explains_consequences_and_defaults_to_cancel() {
    let mut a = app(Mode::UltraStable, "setmode");
    open(&mut a, Builtin::Users);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("USERS & SECURITY MODES"), "{screen}");
    assert!(
        screen.contains("carol") && screen.contains("ADVANCED"),
        "{screen}"
    );
    select_user(&mut a, "bob");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::ChooseMode { .. }));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("NEW MODE FOR 'bob'"), "{screen}");
    press(&mut a, KeyCode::End); // Advanced
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Confirm {
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("sudo"), "{body}");
            assert!(body.contains("administrator must type"), "{body}");
            assert!(body.contains("next login"), "{body}");
            assert_eq!(
                *action,
                Pending::SetMode {
                    user: "bob".into(),
                    to: Mode::Advanced
                }
            );
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    let shown = render(&mut a, 80, 24);
    assert!(shown.contains("CHANGE SECURITY MODE"), "{shown}");
    // Enter on the default button cancels; nothing runs.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
    // Confirm explicitly: the helper runs through pkexec with a fixed argv.
    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec",
                    "/store/bin/resilience-admin",
                    "set-mode",
                    "bob",
                    "advanced"
                ]
            );
            assert!(r.pause_after, "the helper's answer stays on screen");
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
            assert!(matches!(a.view(), View::Users { .. }), "{:?}", a.view());
            assert!(r.extra_args.is_empty());
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            assert_eq!(
                r.env,
                vec![(
                    "SHELL".to_string(),
                    "/run/current-system/sw/bin/resilience-session".to_string()
                )]
            );
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn demotion_warns_that_sessions_end() {
    let mut a = app(Mode::Stable, "demote");
    open(&mut a, Builtin::Users);
    select_user(&mut a, "bob");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Home); // Ultra-Stable
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { body, .. }) = &a.modal else {
        panic!("expected confirmation")
    };
    assert!(body.contains("sessions of 'bob' will be closed"), "{body}");
}

#[test]
fn unsafe_mode_changes_are_refused_before_asking_for_a_password() {
    let mut a = app(Mode::Advanced, "refuse");
    open(&mut a, Builtin::Users);
    // carol is the only administrator.
    select_user(&mut a, "carol");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Home);
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Message { title, body, error }) => {
            assert_eq!(title, "NOT POSSIBLE");
            assert!(body.contains("last Advanced account"), "{body}");
            assert!(error);
        }
        other => panic!("expected refusal, got {other:?}"),
    }
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Esc);
    // demo logs in automatically and must stay Ultra-Stable.
    select_user(&mut a, "demo");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("automatically")),
        "{:?}",
        a.modal
    );
}

#[test]
fn no_administrator_means_no_mode_change() {
    let mut a = app(Mode::UltraStable, "noadmin");
    std::fs::write(&a.paths.group_file, "").expect("w");
    let cfg = std::fs::read_to_string(&a.paths.admin_config).expect("r");
    std::fs::write(
        &a.paths.admin_config,
        cfg.replace("declared_mode = \"advanced\"", "declared_mode = \"stable\""),
    )
    .expect("w");
    open(&mut a, Builtin::Users);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO ADMINISTRATOR"),
        "{:?}",
        a.modal
    );
}

#[test]
fn users_view_survives_missing_or_corrupt_configuration() {
    let mut a = app(Mode::UltraStable, "badcfg");
    std::fs::write(&a.paths.admin_config, "this is = not [valid").expect("w");
    open(&mut a, Builtin::Users);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("could not be read"), "{screen}");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    let mut b = app(Mode::UltraStable, "nocfg");
    std::fs::remove_file(&b.paths.admin_config).expect("rm");
    open(&mut b, Builtin::Users);
    assert!(render(&mut b, 80, 24).contains("could not be read"));
}

#[test]
fn recovery_section_shows_health_generations_and_guide() {
    let mut a = app(Mode::UltraStable, "recovery");
    std::fs::write(
        a.sys.handbook_dir.join("40-recovery.md"),
        "# Recovery\nWhen something goes wrong.\n",
    )
    .expect("w");
    open(&mut a, Builtin::Recovery);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("RECOVERY & DIAGNOSTICS"), "{screen}");
    assert!(screen.contains("[2] System generations"), "{screen}");
    press(&mut a, KeyCode::Char('1'));
    assert!(matches!(a.view(), View::Pager { title, .. } if title.contains("HEALTH")));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('2'));
    assert!(matches!(a.view(), View::Generations { gens, .. } if gens.is_empty()));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("No generations are recorded"), "{screen}");
    // Nothing to select, so Enter does nothing.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('4'));
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains("When something goes wrong")),
        "{:?}",
        a.view()
    );
}

/// Three generations in the test store: 3 is booted and the default, 2 is
/// an older complete system, 1 has no switch-to-configuration.
fn make_generations(a: &App) {
    use std::os::unix::fs::symlink;
    for n in 1..=3u64 {
        let sys = a.paths.store.join(format!("{n}abc-nixos-system-26.05.{n}"));
        std::fs::create_dir_all(sys.join("bin")).expect("mkdir");
        std::fs::write(sys.join("nixos-version"), format!("26.05.{n}\n")).expect("w");
        if n > 1 {
            std::fs::write(sys.join("bin/switch-to-configuration"), "").expect("w");
        }
        symlink(&sys, a.paths.profiles.join(format!("system-{n}-link"))).expect("ln");
        if n == 3 {
            symlink(&sys, &a.paths.booted_system).expect("ln");
            symlink(&sys, &a.paths.running_system).expect("ln");
        }
    }
    symlink("system-3-link", a.paths.profiles.join("system")).expect("ln");
}

fn open_generations(a: &mut App) {
    open(a, Builtin::Recovery);
    press(a, KeyCode::Char('2'));
    assert!(
        matches!(a.view(), View::Generations { .. }),
        "{:?}",
        a.view()
    );
}

#[test]
fn generations_are_listed_with_their_state() {
    let mut a = app(Mode::UltraStable, "gens-list");
    make_generations(&a);
    open_generations(&mut a);
    for (w, h) in [(80, 24), (60, 20), (40, 12), (120, 40)] {
        assert_console_glyphs(&render(&mut a, w, h));
    }
    let s = render(&mut a, 100, 30);
    assert!(s.contains("SYSTEM GENERATIONS"), "{s}");
    assert!(s.contains("started, default"), "{s}");
    assert!(s.contains("26.05.3"), "{s}");
    assert!(s.contains("never your personal files"), "{s}");
    // Enter on the current default only explains.
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, error: false, .. })
            if body.contains("already the default")),
        "{:?}",
        a.modal
    );
    press(&mut a, KeyCode::Esc);
    // Generation 1 is not a complete system: refused before any password.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, error: true, .. })
            if body.contains("not a complete system")),
        "{:?}",
        a.modal
    );
}

#[test]
fn making_an_older_generation_the_default_confirms_then_runs_the_helper() {
    let mut a = app(Mode::UltraStable, "gens-set");
    make_generations(&a);
    open_generations(&mut a);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Confirm {
            title,
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert_eq!(title, "DEFAULT GENERATION");
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("generation 2"), "{body}");
            assert!(body.contains("older system software"), "{body}");
            assert!(
                body.contains("Nothing changes until the next restart"),
                "{body}"
            );
            assert_eq!(*action, Pending::SetDefaultGeneration { number: 2 });
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    assert_console_glyphs(&render(&mut a, 80, 24));
    // Enter on the default button cancels.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());

    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec".to_string(),
                    "/store/bin/resilience-admin".into(),
                    "set-default-generation".into(),
                    "2".into(),
                ]
            );
            assert!(r.pause_after);
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            assert_eq!(r.env[0].0, "SHELL");
            // The helper switched the profile; the list shows it on return.
            let p = a.paths.profiles.join("system");
            std::fs::remove_file(&p).expect("rm");
            std::os::unix::fs::symlink("system-2-link", &p).expect("ln");
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
            let s = render(&mut a, 100, 30);
            assert!(s.contains("   2  ") && s.contains("default"), "{s}");
            let View::Generations { gens, .. } = a.view() else {
                panic!("still on the list")
            };
            assert!(
                gens.iter()
                    .find(|g| g.number == 2)
                    .is_some_and(|g| g.default)
            );
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn no_administrator_means_no_default_generation_change() {
    let mut a = app(Mode::UltraStable, "gens-noadmin");
    make_generations(&a);
    let cfg = std::fs::read_to_string(&a.paths.admin_config).expect("r");
    std::fs::write(
        &a.paths.admin_config,
        cfg.replace("declared_mode = \"advanced\"", "declared_mode = \"stable\""),
    )
    .expect("w");
    std::fs::write(&a.paths.group_file, "").expect("w");
    open_generations(&mut a);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO ADMINISTRATOR"),
        "{:?}",
        a.modal
    );
}

#[test]
fn diagnostic_report_is_private_and_never_overwritten() {
    use std::os::unix::fs::PermissionsExt;
    let mut a = app(Mode::UltraStable, "report");
    open(&mut a, Builtin::Recovery);
    press(&mut a, KeyCode::Char('3'));
    let Some(Modal::Message { title, body, error }) = &a.modal else {
        panic!("expected message, got {:?}", a.modal)
    };
    assert_eq!(title, "REPORT SAVED", "{body}");
    assert!(!error);
    let dir = a.home.join("Diagnostics");
    let files: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].path()).expect("read");
    assert!(text.contains("RESILIENCE OS DIAGNOSTIC REPORT"));
    assert!(text.contains("ULTRA-STABLE mode"), "{text}");
    let mode = files[0].metadata().expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn new_views_render_at_several_sizes() {
    let mut a = app(Mode::Advanced, "newsizes");
    for b in [Builtin::Users, Builtin::Recovery, Builtin::Backup] {
        open(&mut a, b);
        for (w, h) in [(80, 24), (40, 12), (120, 40), (200, 60)] {
            assert_console_glyphs(&render(&mut a, w, h));
        }
        if b == Builtin::Users {
            press(&mut a, KeyCode::Enter); // choose mode for the first account
            press(&mut a, KeyCode::Down);
            for (w, h) in [(80, 24), (40, 12), (120, 40)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            press(&mut a, KeyCode::Enter); // confirmation or refusal modal
            assert!(a.modal.is_some());
            for (w, h) in [(80, 24), (60, 20)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            a.modal = None;
        }
        if b == Builtin::Backup {
            press(&mut a, KeyCode::Enter); // open the stick
            assert!(matches!(a.view(), View::Backups { .. }));
            for (w, h) in [(80, 24), (40, 12), (120, 40)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            press(&mut a, KeyCode::Enter); // new backup confirmation
            assert!(a.modal.is_some());
            for (w, h) in [(80, 24), (60, 20)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            a.modal = None;
        }
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

// ----- backups & USB sticks ---------------------------------------------------

fn media_names(a: &App) -> Vec<String> {
    let View::Media { volumes, .. } = a.view() else {
        panic!("not on the media view")
    };
    volumes.iter().map(|v| v.display_name()).collect()
}

#[test]
fn only_removable_media_are_listed_and_opening_mounts_without_a_password() {
    let mut a = app(Mode::UltraStable, "media");
    open(&mut a, Builtin::Backup);
    // The internal disk is never offered; the encrypted card is shown as unusable.
    assert_eq!(media_names(&a), vec!["STICK", "SECRET"]);
    let screen = render(&mut a, 80, 24);
    assert!(
        !screen.contains("INTERNAL") && !screen.contains("vdb"),
        "{screen}"
    );
    // The encrypted card cannot be opened.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Message { title, body, .. }) => {
            assert_eq!(title, "CANNOT USE THIS DEVICE");
            assert!(body.contains("encrypted"), "{body}");
        }
        other => panic!("expected a message, got {other:?}"),
    }
    a.modal = None;
    assert_eq!(udisks_log(&a), "", "nothing was mounted");
    // The stick: mounted through udisks, never asking for a password.
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        udisks_log(&a),
        "mount --no-user-interaction --block-device /dev/sda1\n"
    );
    let View::Backups { mount, files, .. } = a.view() else {
        panic!("not on the backups view: {:?}", a.view())
    };
    assert_eq!(mount, Path::new("/run/media/tester/STICK"));
    assert!(files.is_empty());
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("BACKUPS ON STICK"), "{screen}");
    assert!(screen.contains("Make a new backup"), "{screen}");
    // Back on the list the stick shows as open; opening it again does not remount.
    press(&mut a, KeyCode::Esc);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains(" open"), "{screen}");
    press(&mut a, KeyCode::Enter);
    assert_eq!(udisks_log(&a).lines().count(), 1);
}

#[test]
fn new_backup_asks_first_then_runs_the_unprivileged_tool() {
    let mut a = app(Mode::UltraStable, "newbackup");
    std::fs::write(a.home.join("file"), vec![0u8; 4096]).expect("write");
    open(&mut a, Builtin::Backup);
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Enter); // "+ Make a new backup"
    let Some(Modal::Confirm {
        body,
        action,
        yes_focused,
        ..
    }) = &a.modal
    else {
        panic!("expected a confirmation, got {:?}", a.modal)
    };
    assert!(body.contains("passphrase"), "{body}");
    assert!(body.contains("Your files are not changed"), "{body}");
    assert!(!yes_focused, "Cancel is the default");
    assert_eq!(
        *action,
        Pending::Backup {
            mount: "/run/media/tester/STICK".into()
        }
    );
    // Enter on the default choice cancels.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Enter);
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, BACKUP_LABEL);
    assert_eq!(req.argv, vec!["/store/bin/resilience-backup", "create"]);
    assert_eq!(
        req.extra_args,
        vec![PathBuf::from("/run/media/tester/STICK")]
    );
    assert!(req.pause_after && req.polkit_agent.is_none() && req.env.is_empty());
    a.after_run(&req.label, &Ok(std::process::ExitStatus::default()));
    assert!(matches!(a.view(), View::Backups { .. }));
}

#[test]
fn restore_and_check_use_the_selected_backup() {
    let mut a = app(Mode::Stable, "restore");
    let stick = a.home.join("stick");
    let dir = stick.join("Resilience-backups");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("home-20261114-103000.tar.zst.age"), "x").expect("w");
    std::fs::write(dir.join("home-20261101-080000.tar.zst.age"), "x").expect("w");
    std::fs::write(dir.join("home-20261101-080000.tar.zst.age.sha256"), "x").expect("w");
    open(&mut a, Builtin::Backup);
    let View::Media { volumes, .. } = a.view() else {
        panic!("media")
    };
    let volume = volumes[0].clone();
    a.stack.push(View::Backups {
        volume,
        mount: stick.clone(),
        files: resilience_core::backup::list_backups(&stick),
        list: resilience_shell::list::ListState::default().with_selected(Some(0)),
    });
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("2026-11-14 10:30:00 UTC"), "{screen}");
    press(&mut a, KeyCode::Down); // newest backup
    // c checks without asking anything.
    let Effect::Run(req) = press(&mut a, KeyCode::Char('c')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, CHECK_LABEL);
    assert_eq!(req.argv[1], "check");
    assert_eq!(
        req.extra_args,
        vec![dir.join("home-20261114-103000.tar.zst.age")]
    );
    // Enter restores, after a confirmation that says where the files go.
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm {
        body, yes_focused, ..
    }) = &a.modal
    else {
        panic!("expected a confirmation")
    };
    assert!(!yes_focused);
    assert!(body.contains("Restored/backup-20261114-103000"), "{body}");
    assert!(body.contains("Nothing that exists is"), "{body}");
    assert!(body.contains("no checksum file"), "{body}");
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, RESTORE_LABEL);
    assert_eq!(req.argv, vec!["/store/bin/resilience-backup", "restore"]);
    assert_eq!(
        req.extra_args,
        vec![dir.join("home-20261114-103000.tar.zst.age")]
    );
    // The older backup has a checksum: no warning.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { body, .. }) = &a.modal else {
        panic!("expected a confirmation")
    };
    assert!(!body.contains("no checksum file"), "{body}");
    a.modal = None;
    // The first row has nothing to check.
    press(&mut a, KeyCode::Home);
    assert_eq!(press(&mut a, KeyCode::Char('c')), Effect::None);
}

#[test]
fn eject_says_when_it_is_safe_to_unplug() {
    let mut a = app(Mode::UltraStable, "eject");
    open(&mut a, Builtin::Backup);
    press(&mut a, KeyCode::Enter); // mount and open
    std::fs::write(a.home.join("sys/busy"), "").expect("w");
    press(&mut a, KeyCode::Char('e'));
    match &a.modal {
        Some(Modal::Message { title, body, error }) => {
            assert_eq!(title, "CANNOT EJECT");
            assert!(*error && body.contains("Do not unplug it yet"), "{body}");
            assert!(body.contains("still in use"), "{body}");
        }
        other => panic!("expected a message, got {other:?}"),
    }
    a.modal = None;
    // Back on the list (the backups view closed); try again once not busy.
    assert!(matches!(a.view(), View::Media { .. }));
    std::fs::remove_file(a.home.join("sys/busy")).expect("rm");
    press(&mut a, KeyCode::Char('e'));
    match &a.modal {
        Some(Modal::Message { title, error, .. }) => {
            assert_eq!(title, "SAFE TO UNPLUG");
            assert!(!error);
        }
        other => panic!("expected a message, got {other:?}"),
    }
    let log = udisks_log(&a);
    assert!(
        log.ends_with(
            "unmount --no-user-interaction --block-device /dev/sda1\n\
             power-off --no-user-interaction --block-device /dev/sda\n"
        ),
        "{log}"
    );
    let View::Media { volumes, .. } = a.view() else {
        panic!("media")
    };
    assert!(volumes[0].mountpoint.is_none());
}

#[test]
fn media_view_survives_a_failing_lsblk() {
    let mut a = app(Mode::UltraStable, "nolsblk");
    a.sys.tools.lsblk = "/nonexistent/lsblk".into();
    open(&mut a, Builtin::Backup);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("could not be read"), "{screen}");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Char('e'));
    assert!(a.modal.is_none());
    assert!(matches!(a.view(), View::Media { .. }));
}

// ----- date and time ------------------------------------------------------------

fn type_time(a: &mut App, text: &str) {
    for _ in 0..25 {
        press(a, KeyCode::Backspace);
    }
    for c in text.chars() {
        press(a, KeyCode::Char(c));
    }
}

fn open_clock(a: &mut App) {
    let (v, e) = a.activate(Entry::Builtin(Builtin::Clock));
    assert!(v.is_none());
    assert_eq!(e, Effect::None);
}

#[test]
fn clock_is_offered_in_every_mode_and_shows_the_current_time() {
    for m in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let a = app(m, &format!("clock-{m:?}"));
        assert!(
            a.entries(Category::System)
                .contains(&Entry::Builtin(Builtin::Clock)),
            "{m:?}"
        );
    }
    let mut a = app(Mode::UltraStable, "clock-open");
    open_clock(&mut a);
    match &a.modal {
        Some(Modal::SetTime { value, error, .. }) => {
            // Prefilled with the current local time, ready to correct.
            assert!(clock::parse_local(value).is_ok(), "{value}");
            assert!(error.is_none());
        }
        other => panic!("expected the clock dialog, got {other:?}"),
    }
    for (w, h) in [(80, 24), (60, 20), (40, 12), (120, 40)] {
        let s = render(&mut a, w, h);
        assert_console_glyphs(&s);
        if w >= 80 {
            assert!(s.contains("DATE & TIME") && s.contains("time zone"), "{s}");
        }
    }
}

#[test]
fn setting_the_clock_confirms_then_runs_the_helper_with_utc() {
    let mut a = app(Mode::UltraStable, "clock-set");
    open_clock(&mut a);
    type_time(&mut a, "2031-03-01 10:00");
    press(&mut a, KeyCode::Char('x')); // only digits and separators are typed
    press(&mut a, KeyCode::Enter);
    let expected = clock::local_to_unix(clock::parse_local("2031-03-01 10:00").expect("valid"))
        .expect("representable");
    match &a.modal {
        Some(Modal::Confirm {
            title,
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert_eq!(title, "SET THE CLOCK");
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("2031-03-01 10:00"), "{body}");
            assert!(body.contains("administrator must type"), "{body}");
            assert!(body.contains(&admin::rfc3339_utc(expected)), "{body}");
            assert_eq!(*action, Pending::SetTime { utc: expected });
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    assert_console_glyphs(&render(&mut a, 80, 24));
    // Enter on the default button cancels; nothing runs.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());

    open_clock(&mut a);
    type_time(&mut a, "2031-03-01 10:00");
    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec".to_string(),
                    "/store/bin/resilience-admin".into(),
                    "set-time".into(),
                    admin::rfc3339_utc(expected),
                ]
            );
            assert!(r.extra_args.is_empty());
            assert!(r.pause_after);
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            assert_eq!(r.env[0].0, "SHELL");
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn impossible_or_out_of_range_times_are_explained_in_the_dialog() {
    let mut a = app(Mode::Stable, "clock-bad");
    open_clock(&mut a);
    for (input, why) in [
        ("2031-02-30 10:00", "does not exist"),
        ("2031-03-01 25:00", "does not exist"),
        ("2020-06-01 10:00", "2025 or later"),
        ("2100-06-01 10:00", "before the year 2100"),
        ("2031-3-1 10:00", "YYYY-MM-DD HH:MM"),
        ("", "YYYY-MM-DD HH:MM"),
    ] {
        type_time(&mut a, input);
        assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
        match &a.modal {
            Some(Modal::SetTime {
                error: Some(e),
                value,
                ..
            }) => {
                assert!(e.contains(why), "{input:?}: {e}");
                assert_eq!(value, input, "the typed text is kept for correction");
            }
            other => panic!("{input:?}: expected an error in the dialog, got {other:?}"),
        }
        assert_console_glyphs(&render(&mut a, 80, 24));
    }
    // Esc closes the dialog without doing anything.
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::None);
    assert!(a.modal.is_none());
}

#[test]
fn no_administrator_means_no_clock_change() {
    let mut a = app(Mode::UltraStable, "clock-noadmin");
    std::fs::write(&a.paths.group_file, "").expect("w");
    let cfg = std::fs::read_to_string(&a.paths.admin_config).expect("r");
    std::fs::write(
        &a.paths.admin_config,
        cfg.replace("declared_mode = \"advanced\"", "declared_mode = \"stable\""),
    )
    .expect("w");
    open_clock(&mut a);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, body, .. })
            if title == "NO ADMINISTRATOR" && body.contains("Setting the clock")),
        "{:?}",
        a.modal
    );
}

// ----- Wi-Fi ----------------------------------------------------------------------

fn nmcli_log(a: &App) -> String {
    std::fs::read_to_string(a.home.join("sys/nmcli.log")).unwrap_or_default()
}

fn profiles(a: &App) -> String {
    std::fs::read_to_string(a.home.join("sys/nm-profiles")).unwrap_or_default()
}

fn select_network(a: &mut App, ssid: &str) {
    let View::Wifi {
        overview: Some(o), ..
    } = a.view()
    else {
        panic!("not on the Wi-Fi view: {:?}", a.view())
    };
    let i = o
        .networks
        .iter()
        .position(|n| n.ssid == ssid)
        .expect("network listed");
    press(a, KeyCode::Home);
    for _ in 0..i {
        press(a, KeyCode::Down);
    }
}

#[test]
fn wifi_is_offered_in_every_mode_and_lists_networks() {
    for m in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let a = app(m, &format!("wifi-{m:?}"));
        assert!(
            a.entries(Category::Networking)
                .contains(&Entry::Builtin(Builtin::Wifi)),
            "{m:?}"
        );
    }
    let mut a = app(Mode::UltraStable, "wifi-list");
    open(&mut a, Builtin::Wifi);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("WI-FI NETWORKS"), "{screen}");
    for n in [
        "Home", "Cafe", "Library", "Corp", "saved", "802.1X", "55%", "WEP",
    ] {
        assert!(screen.contains(n), "{n}: {screen}");
    }
    // One row per network name: Cafe appears once in the list.
    let View::Wifi {
        overview: Some(o), ..
    } = a.view()
    else {
        panic!()
    };
    assert_eq!(o.networks.iter().filter(|n| n.ssid == "Cafe").count(), 1);
    for (w, h) in [(80, 24), (40, 12), (120, 40), (200, 60)] {
        assert_console_glyphs(&render(&mut a, w, h));
    }
}

/// Wait for the background connection attempt (fake nmcli) to finish.
fn finish_wifi_job(a: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while a.wifi_job.is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "connection attempt hangs"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        a.on_tick();
    }
}

fn type_password(a: &mut App, p: &str) {
    for c in p.chars() {
        press(a, KeyCode::Char(c));
    }
}

#[test]
fn joining_a_new_network_asks_for_the_password_and_passes_it_on_stdin_only() {
    let mut a = app(Mode::UltraStable, "wifi-join");
    open(&mut a, Builtin::Wifi);
    select_network(&mut a, "Cafe");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::WifiPassword { .. })),
        "{:?}",
        a.modal
    );
    // Nothing is created before the password is confirmed.
    assert!(!nmcli_log(&a).contains("connection add"));
    type_password(&mut a, "right-password");
    let s = render(&mut a, 80, 24);
    assert!(
        s.contains("WI-FI PASSWORD") && s.contains("**************"),
        "{s}"
    );
    assert!(
        !s.contains("right-password"),
        "the password is never shown: {s}"
    );
    assert_console_glyphs(&s);
    // Too short: explained in the dialog, nothing runs.
    a.modal = Some(Modal::WifiPassword {
        net: match &a.modal {
            Some(Modal::WifiPassword { net, .. }) => net.clone(),
            _ => unreachable!(),
        },
        uuid: None,
        value: "short".into(),
        error: None,
    });
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::WifiPassword { error: Some(e), .. }) if e.contains("at least 8")),
        "{:?}",
        a.modal
    );
    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    type_password(&mut a, "right-password");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    let log = nmcli_log(&a);
    let add = log
        .lines()
        .find(|l| l.starts_with("connection add"))
        .expect("profile created");
    assert!(add.contains("connection.permissions user:tester"), "{add}");
    assert!(add.contains("wifi-sec.psk-flags 0"), "{add}");
    assert!(a.wifi_job.as_ref().is_some_and(|j| j.new_profile));
    assert!(render(&mut a, 80, 24).contains("connecting to 'Cafe'"));
    finish_wifi_job(&mut a);
    assert!(a.modal.is_none(), "{:?}", a.modal);
    assert!(!nmcli_log(&a).contains("right-password"), "never in argv");
    assert_eq!(
        std::fs::read_to_string(a.home.join("sys/nm-stdin")).expect("stdin"),
        "802-11-wireless-security.psk:right-password\n"
    );
    assert!(profiles(&a).contains(":802-11-wireless:yes") && profiles(&a).contains("Cafe:"));
    let s = render(&mut a, 80, 24);
    assert!(s.contains("*Cafe"), "{s}");
}

#[test]
fn a_wrong_password_saves_nothing_and_says_so() {
    let mut a = app(Mode::UltraStable, "wifi-wrong");
    open(&mut a, Builtin::Wifi);
    select_network(&mut a, "Cafe");
    press(&mut a, KeyCode::Enter);
    type_password(&mut a, "wrong-password");
    press(&mut a, KeyCode::Enter);
    finish_wifi_job(&mut a);
    match &a.modal {
        Some(Modal::Message { title, body, error }) => {
            assert_eq!(title, "NOT CONNECTED");
            assert!(*error);
            assert!(
                body.contains("probably wrong") && body.contains("Nothing was saved"),
                "{body}"
            );
        }
        other => panic!("expected an explanation, got {other:?}"),
    }
    assert!(!profiles(&a).contains("Cafe:"), "{}", profiles(&a));
    // Esc in the password dialog creates nothing at all.
    a.modal = None;
    press(&mut a, KeyCode::Enter);
    type_password(&mut a, "whatever-1");
    press(&mut a, KeyCode::Esc);
    assert!(a.modal.is_none() && a.wifi_job.is_none());
    assert_eq!(nmcli_log(&a).matches("connection add").count(), 1);
}

#[test]
fn saved_networks_connect_without_a_password_and_can_be_forgotten() {
    let mut a = app(Mode::UltraStable, "wifi-saved");
    open(&mut a, Builtin::Wifi);
    select_network(&mut a, "Home");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none(), "no password asked: {:?}", a.modal);
    assert!(
        a.wifi_job
            .as_ref()
            .is_some_and(|j| !j.new_profile && !j.with_password)
    );
    finish_wifi_job(&mut a);
    assert!(profiles(&a).contains("Home:0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10:802-11-wireless:yes"));
    assert!(!nmcli_log(&a).contains("connection add"));
    // Disconnect.
    select_network(&mut a, "Home");
    press(&mut a, KeyCode::Char('d'));
    assert!(profiles(&a).contains("Home:0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10:802-11-wireless:no"));
    // The saved password is refused: the password dialog opens for the
    // same profile; a failed retry keeps the profile.
    std::fs::write(a.home.join("sys/home-refused"), "").expect("w");
    select_network(&mut a, "Home");
    press(&mut a, KeyCode::Enter);
    finish_wifi_job(&mut a);
    match &a.modal {
        Some(Modal::WifiPassword { uuid, error, .. }) => {
            assert_eq!(
                uuid.as_deref(),
                Some("0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10")
            );
            assert!(error.as_deref().is_some_and(|e| e.contains("not accepted")));
        }
        other => panic!("expected the password dialog, got {other:?}"),
    }
    type_password(&mut a, "wrong-again");
    press(&mut a, KeyCode::Enter);
    finish_wifi_job(&mut a);
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NOT CONNECTED"));
    assert!(profiles(&a).contains("Home:"), "a saved network is kept");
    a.modal = None;
    // Forget: confirmation with Cancel as default.
    select_network(&mut a, "Home");
    press(&mut a, KeyCode::Char('f'));
    assert!(matches!(
        &a.modal,
        Some(Modal::Confirm {
            yes_focused: false,
            ..
        })
    ));
    press(&mut a, KeyCode::Enter);
    assert!(profiles(&a).contains("Home:"));
    press(&mut a, KeyCode::Char('f'));
    press(&mut a, KeyCode::Char('y'));
    assert!(!profiles(&a).contains("Home:"), "{}", profiles(&a));
    // A profile set up by an administrator cannot be removed: clear message.
    select_network(&mut a, "Shared");
    press(&mut a, KeyCode::Char('f'));
    press(&mut a, KeyCode::Char('y'));
    match &a.modal {
        Some(Modal::Message { body, error, .. }) => {
            assert!(*error);
            assert!(body.contains("not allowed"), "{body}");
        }
        other => panic!("expected an explanation, got {other:?}"),
    }
}

#[test]
fn open_unsupported_and_refused_networks_are_explained() {
    let mut a = app(Mode::UltraStable, "wifi-special");
    open(&mut a, Builtin::Wifi);
    // Open network: a warning first, Cancel by default, nothing created.
    select_network(&mut a, "Library");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::Confirm { title, yes_focused: false, .. }) if title == "OPEN NETWORK")
    );
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(!nmcli_log(&a).contains("connection add"));
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Char('y'));
    assert!(a.wifi_job.as_ref().is_some_and(|j| !j.with_password));
    finish_wifi_job(&mut a);
    let log = nmcli_log(&a);
    assert!(
        log.contains("con-name Library") && !log.contains("wifi-sec"),
        "{log}"
    );
    // Enterprise and WEP: not offered.
    for ssid in ["Corp", "Old"] {
        a.modal = None;
        select_network(&mut a, ssid);
        assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
        assert!(
            matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NOT POSSIBLE"),
            "{ssid}: {:?}",
            a.modal
        );
    }
    // polkit refuses to create the profile: explained, nothing runs.
    a.modal = None;
    std::fs::write(a.home.join("sys/deny-add"), "").expect("w");
    select_network(&mut a, "Cafe");
    press(&mut a, KeyCode::Enter);
    type_password(&mut a, "right-password");
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("Insufficient privileges") && body.contains("not allowed")),
        "{:?}",
        a.modal
    );
    assert!(a.wifi_job.is_none());
}

#[test]
fn wifi_off_or_missing_is_explained() {
    let mut a = app(Mode::UltraStable, "wifi-off");
    std::fs::write(a.home.join("sys/radio-off"), "").expect("w");
    open(&mut a, Builtin::Wifi);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("Wi-Fi is switched off"), "{s}");
    press(&mut a, KeyCode::Char('w'));
    assert!(nmcli_log(&a).contains("radio wifi on"));
    let s = render(&mut a, 80, 24);
    assert!(s.contains("Cafe"), "{s}");

    let mut b = app(Mode::UltraStable, "wifi-none");
    std::fs::write(b.home.join("sys/no-wifi"), "").expect("w");
    open(&mut b, Builtin::Wifi);
    let s = render(&mut b, 80, 24);
    assert!(s.contains("No Wi-Fi adapter"), "{s}");

    let mut c = app(Mode::UltraStable, "wifi-broken");
    c.sys.tools.nmcli = c.home.join("sys/missing-nmcli").display().to_string();
    open(&mut c, Builtin::Wifi);
    let s = render(&mut c, 80, 24);
    assert!(s.contains("not installed"), "{s}");
}

// ----- offline library and resource packs (ADR 0013) -----------------------

fn pack_rows(a: &App) -> Vec<PackRow> {
    let View::Packs { s, .. } = a.view() else {
        panic!("not on the packs view")
    };
    s.rows()
}

fn selected_row(a: &App) -> PackRow {
    let View::Packs { s, list } = a.view() else {
        panic!("not on the packs view")
    };
    s.rows()[list.selected().expect("a row is selected")]
}

/// Move the packs selection to the row of catalog pack `i` (or other file).
fn select_row(a: &mut App, want: PackRow) {
    for _ in 0..20 {
        if selected_row(a) == want {
            return;
        }
        press(a, KeyCode::Down);
    }
    panic!("row {want:?} not reachable");
}

#[test]
fn packs_screen_groups_by_category_and_skips_headers() {
    let mut a = app(Mode::UltraStable, "packs-rows");
    open(&mut a, Builtin::Packs);
    let rows = pack_rows(&a);
    use resilience_core::packs::PackCategory::*;
    assert_eq!(
        rows,
        vec![
            PackRow::Header(Encyclopedia),
            PackRow::Pack(0),
            PackRow::Pack(2),
            PackRow::Header(Health),
            PackRow::Pack(1),
            PackRow::OthersHeader,
            PackRow::Other(0),
        ]
    );
    // The selection never rests on a header, and wraps around.
    assert_eq!(selected_row(&a), PackRow::Pack(0));
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Down);
    assert_eq!(selected_row(&a), PackRow::Pack(1));
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Down);
    assert_eq!(selected_row(&a), PackRow::Pack(0));
    press(&mut a, KeyCode::Up);
    assert_eq!(selected_row(&a), PackRow::Other(0));

    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("-- ENCYCLOPEDIA --"), "{screen}");
    assert!(screen.contains("[*] Wikipedia (English)"), "{screen}");
    assert!(screen.contains("installed"), "{screen}");
    assert!(screen.contains("[ ] Medicine (English)"), "{screen}");
    assert!(screen.contains("25% done"), "{screen}");
    assert!(screen.contains("old version"), "{screen}");
    assert_console_glyphs(&screen);

    // Language filter: en only, then fr only, then everything again.
    press(&mut a, KeyCode::Char('f'));
    assert!(!pack_rows(&a).contains(&PackRow::Pack(2)));
    assert!(render(&mut a, 80, 24).contains("language: en"));
    press(&mut a, KeyCode::Char('f'));
    let rows = pack_rows(&a);
    assert!(rows.contains(&PackRow::Pack(2)) && !rows.contains(&PackRow::Pack(0)));
    assert_eq!(selected_row(&a), PackRow::Pack(2));
    press(&mut a, KeyCode::Char('f'));
    assert_eq!(pack_rows(&a).len(), 7);
}

#[test]
fn ticking_packs_and_downloading_runs_the_library_with_a_fixed_argv() {
    let mut a = app(Mode::UltraStable, "packs-download");
    open(&mut a, Builtin::Packs);
    // Nothing ticked: an explanation.
    press(&mut a, KeyCode::Char('d'));
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NOTHING SELECTED"));
    a.modal = None;
    // Tick: wiki-en (installed -> remove), wiki-fr (add), medicine (resume).
    press(&mut a, KeyCode::Char(' '));
    select_row(&mut a, PackRow::Pack(2));
    press(&mut a, KeyCode::Char(' '));
    select_row(&mut a, PackRow::Pack(1));
    press(&mut a, KeyCode::Char(' '));
    select_row(&mut a, PackRow::Other(0));
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Char(' ')); // and untick again
    // A partial download cycles: continue, throw away, nothing, continue.
    select_row(&mut a, PackRow::Pack(1));
    press(&mut a, KeyCode::Char(' '));
    assert!(render(&mut a, 80, 24).contains("[-] Medicine (English)"));
    assert!(render(&mut a, 80, 24).contains("discard"));
    press(&mut a, KeyCode::Char(' '));
    assert!(render(&mut a, 80, 24).contains("[ ] Medicine (English)"));
    press(&mut a, KeyCode::Char(' '));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("[-] Wikipedia (English)"), "{screen}");
    assert!(screen.contains("[+] Wikipedia (French)"), "{screen}");
    // 3000 + (4000 - 1000 already received).
    assert!(screen.contains("Add 2 (5.9 KiB)  Remove 1"), "{screen}");

    // Offline: refused with a way out.
    a.net.level = resilience_core::network::LinkLevel::LocalLink;
    press(&mut a, KeyCode::Char('d'));
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, body, .. })
            if title == "NO INTERNET CONNECTION" && body.contains("USB stick")),
        "{:?}",
        a.modal
    );
    a.modal = None;

    a.net.level = resilience_core::network::LinkLevel::Routed;
    press(&mut a, KeyCode::Char('d'));
    let Some(Modal::Confirm { body, action, .. }) = &a.modal else {
        panic!("expected a confirmation, got {:?}", a.modal)
    };
    assert!(body.contains("+ Wikipedia (French)"), "{body}");
    assert!(body.contains("- Wikipedia (English)"), "{body}");
    assert!(body.contains("administrator"), "{body}");
    assert_eq!(
        action,
        &Pending::ChangePacks {
            get: vec!["medicine-en".into(), "wiki-fr".into()],
            remove: vec!["wiki_en_2026-09.zim".into()],
            from: None,
        }
    );
    // Cancel is the default.
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Char('d'));
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run")
    };
    assert_eq!(req.label, PACKS_LABEL);
    assert_eq!(
        req.argv,
        [
            "resilience-library",
            "change",
            "--remove",
            "wiki_en_2026-09.zim",
            "--get",
            "medicine-en",
            "--get",
            "wiki-fr"
        ]
    );
    assert_eq!(
        req.polkit_agent, None,
        "the library registers its own agent"
    );
    assert!(req.pause_after);

    // Afterwards the screen is re-read and the ticks are cleared.
    let dir = a.home.join("sys/packs");
    std::fs::remove_file(dir.join("wiki_en_2026-09.zim")).expect("rm");
    std::fs::write(dir.join("wiki_fr_2026-10.zim"), vec![1u8; 3000]).expect("w");
    a.after_run(PACKS_LABEL, &Ok(std::process::ExitStatus::default()));
    let View::Packs { s, .. } = a.view() else {
        panic!("still on the packs view")
    };
    assert!(s.marked.is_empty());
    use resilience_core::packs::PackState;
    assert_eq!(s.inv.states[0], PackState::Absent);
    assert_eq!(s.inv.states[2], PackState::Installed);
}

#[test]
fn packs_from_a_usb_stick() {
    let mut a = app(Mode::UltraStable, "packs-usb");
    open(&mut a, Builtin::Packs);
    // u: the stick list, then Enter mounts the stick through udisks.
    press(&mut a, KeyCode::Char('u'));
    assert!(matches!(
        a.view(),
        View::Media {
            purpose: MediaPurpose::InstallPacks,
            ..
        }
    ));
    assert!(render(&mut a, 80, 24).contains("COPY PACKS FROM A USB STICK"));
    press(&mut a, KeyCode::Enter); // the stick is the only usable volume, first
    assert!(udisks_log(&a).contains("mount --no-user-interaction --block-device /dev/sda1"));
    // The fake mount point is empty: nothing found, back on the packs view.
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO PACKS ON THIS STICK"),
        "{:?}",
        a.modal
    );
    assert!(matches!(a.view(), View::Packs { .. }));
    a.modal = None;

    // A stick with wiki-fr in Resilience-packs/ and medicine (wrong size).
    let stick = a.home.join("stick");
    std::fs::create_dir_all(stick.join("Resilience-packs")).expect("mkdir");
    std::fs::write(
        stick.join("Resilience-packs/wiki_fr_2026-10.zim"),
        vec![0u8; 3000],
    )
    .expect("w");
    std::fs::write(stick.join("med_en_2026-10.zim"), vec![0u8; 12]).expect("w");
    // Nothing ticked: every pack found that is not installed is offered.
    a.packs_from_mount(&stick);
    let Some(Modal::Confirm { body, action, .. }) = &a.modal else {
        panic!("expected a confirmation, got {:?}", a.modal)
    };
    assert!(body.contains("Copy from the USB stick"), "{body}");
    assert_eq!(
        action,
        &Pending::ChangePacks {
            get: vec!["wiki-fr".into()],
            remove: vec![],
            from: Some(stick.clone()),
        }
    );
    // Ticked packs that are not on the stick are named.
    a.modal = None;
    select_row(&mut a, PackRow::Pack(1));
    press(&mut a, KeyCode::Char(' '));
    select_row(&mut a, PackRow::Pack(2));
    press(&mut a, KeyCode::Char(' '));
    a.packs_from_mount(&stick);
    let Some(Modal::Confirm { body, .. }) = &a.modal else {
        panic!("expected a confirmation")
    };
    assert!(
        body.contains("Not on this stick: Medicine (English)"),
        "{body}"
    );
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run")
    };
    assert_eq!(
        req.argv,
        [
            "resilience-library".to_string(),
            "change".into(),
            "--from".into(),
            stick.display().to_string(),
            "--get".into(),
            "wiki-fr".into()
        ]
    );
}

#[test]
fn library_lists_installed_packs_and_opens_them_in_the_browser() {
    let mut a = app(Mode::UltraStable, "library");
    open(&mut a, Builtin::Library);
    let View::Library { items, .. } = a.view() else {
        panic!("not on the library view")
    };
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "System documentation (built in)",
            "Wikipedia (English)",
            "Wikipedia (English) (older version)"
        ]
    );
    // Built-in packs need no check.
    assert_eq!(press(&mut a, KeyCode::Char('c')), Effect::None);
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "ALREADY CHECKED"));
    a.modal = None;
    press(&mut a, KeyCode::Down);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("OFFLINE LIBRARY"), "{screen}");
    assert_console_glyphs(&screen);
    let dir = a.home.join("sys/packs");
    let Effect::Run(req) = press(&mut a, KeyCode::Enter) else {
        panic!("expected a run")
    };
    assert_eq!(
        req.argv,
        [
            "resilience-library".to_string(),
            "open".into(),
            dir.join("wiki_en_2026-09.zim").display().to_string()
        ]
    );
    // s: search the full text; the words are one argument, never a shell.
    press(&mut a, KeyCode::Char('s'));
    assert!(matches!(a.modal, Some(Modal::PackSearch { .. })));
    press(&mut a, KeyCode::Enter); // empty: nothing happens
    assert!(matches!(a.modal, Some(Modal::PackSearch { .. })));
    for c in "boil; water".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    let Effect::Run(req) = press(&mut a, KeyCode::Enter) else {
        panic!("expected a run")
    };
    assert_eq!(
        req.argv[3..],
        ["--search".to_string(), "boil; water".into()]
    );
    // p: straight to the packs screen.
    press(&mut a, KeyCode::Char('p'));
    assert!(matches!(a.view(), View::Packs { .. }));
}

#[test]
fn library_entries_are_hidden_without_a_catalog() {
    let mut a = app(Mode::UltraStable, "no-packs");
    assert!(
        a.entries(Category::Knowledge)
            .contains(&Entry::Builtin(Builtin::Packs))
    );
    std::fs::remove_file(&a.paths.packs_config).expect("rm");
    let e = a.entries(Category::Knowledge);
    assert!(!e.contains(&Entry::Builtin(Builtin::Packs)));
    assert!(!e.contains(&Entry::Builtin(Builtin::Library)));
    assert!(a.search("resource packs").is_empty());
    // No maps either: the section is empty, so it is not offered.
    assert!(a.entries(Category::Maps).is_empty());
    assert!(!a.home_categories().contains(&Category::Maps));
    assert!(a.search("atlas").is_empty());
    a.paths.packs_config = a.home.join("missing.toml");
    let (v, e) = a.activate(Entry::Builtin(Builtin::Library));
    assert!(v.is_none() && e == Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "RESOURCE PACKS UNAVAILABLE")
    );
}

#[test]
fn suggested_sets_tick_what_is_missing() {
    let mut a = app(Mode::UltraStable, "pack-sets");
    open(&mut a, Builtin::Packs);
    press(&mut a, KeyCode::Char('s'));
    assert!(matches!(a.view(), View::PackSets { .. }));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("SUGGESTED SETS"), "{screen}");
    // wiki-en is installed; medicine (3000 left) and wiki-fr (3000) are not.
    assert!(screen.contains("Starter set"), "{screen}");
    assert!(screen.contains("2 to get, 5.9 KiB"), "{screen}");
    assert_console_glyphs(&screen);
    press(&mut a, KeyCode::Enter);
    let View::Packs { s, .. } = a.view() else {
        panic!("back on the packs view")
    };
    assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    assert!(
        a.current_toast()
            .is_some_and(|n| n.text.contains("2 more packs ticked"))
    );
    // Esc leaves the sets without ticking anything.
    press(&mut a, KeyCode::Char('s'));
    press(&mut a, KeyCode::Esc);
    assert!(matches!(a.view(), View::Packs { .. }));
}

#[test]
fn packs_on_a_stick_are_read_without_installing_and_can_be_checked() {
    let mut a = app(Mode::UltraStable, "stick-read");
    let stick = a.home.join("stick");
    std::fs::create_dir_all(stick.join("Resilience-packs")).expect("mkdir");
    std::fs::write(stick.join("wiki_fr_2026-10.zim"), vec![0u8; 3000]).expect("w");
    std::fs::write(stick.join("Resilience-packs/own_docs.zim"), b"x").expect("w");
    std::fs::write(stick.join("Resilience-packs/notes.txt"), b"x").expect("w");
    let cfg = resilience_core::packs::PackConfig::load(&a.paths.packs_config).expect("cfg");
    let items = stick_items(&cfg, "STICK", &stick, PackKind::Zim);
    let got: Vec<(&str, &ItemPlace)> = items.iter().map(|i| (i.title.as_str(), &i.place)).collect();
    let on = |known| ItemPlace::Stick {
        volume: "STICK".into(),
        known,
    };
    assert_eq!(
        got,
        vec![
            ("Wikipedia (French)", &on(true)),
            ("own_docs.zim", &on(false))
        ]
    );
    // In the library: c checks a stick file with the library tool.
    open(&mut a, Builtin::Library);
    if let Some(View::Library { items: list, .. }) = a.stack.last_mut() {
        list.push(items[0].clone());
    }
    press(&mut a, KeyCode::End);
    let screen = render(&mut a, 80, 24);
    assert!(
        screen.contains("USB stick    Wikipedia (French)"),
        "{screen}"
    );
    assert!(screen.contains("not checked yet"), "{screen}");
    let Effect::Run(req) = press(&mut a, KeyCode::Char('c')) else {
        panic!("expected a run")
    };
    assert_eq!(
        req.argv,
        [
            "resilience-library".to_string(),
            "check".into(),
            stick.join("wiki_fr_2026-10.zim").display().to_string()
        ]
    );
    // u: choose a stick to open for reading; Enter mounts it, back in the library.
    press(&mut a, KeyCode::Char('u'));
    assert!(matches!(
        a.view(),
        View::Media {
            purpose: MediaPurpose::ReadPacks,
            ..
        }
    ));
    assert!(render(&mut a, 80, 24).contains("READ PACKS ON A USB STICK"));
    press(&mut a, KeyCode::Enter);
    assert!(udisks_log(&a).contains("mount --no-user-interaction --block-device /dev/sda1"));
    assert!(
        matches!(a.view(), View::Library { .. }),
        "back in the library"
    );
}

// ----- map packs in the resource-pack machinery (ADR 0016, slice 1) --------

/// The coverage text of the test map pack: long enough to wrap, and with a
/// last word that shows whether it was shown in full.
/// Add the built-in world overview to the test catalog of `write_packs`.
fn add_world_map(a: &App) -> PathBuf {
    let sys = a.home.join("sys");
    let world = sys.join("map-world-natural-earth_2022-05.mbtiles");
    std::fs::write(&world, b"SQLite format 3\0").expect("w");
    let mut text = std::fs::read_to_string(sys.join("packs.toml")).expect("read");
    text.push_str(&format!(
        "[[builtin]]\ntitle = \"World overview map (built in)\"\n\
         description = \"The whole world at overview detail. No streets.\"\n\
         category = \"maps\"\nlanguage = \"en\"\npath = \"{}\"\n\
         license = \"Public domain\"\nsource = \"Natural Earth\"\n",
        world.display()
    ));
    std::fs::write(sys.join("packs.toml"), text).expect("write");
    world
}

fn map_argv(files: &[&Path]) -> Vec<String> {
    let mut argv = vec!["resilience-maps".to_string(), "view".into()];
    for f in files {
        argv.extend(["--pack".to_string(), f.display().to_string()]);
    }
    argv
}

#[test]
fn map_opens_the_viewer_on_built_in_and_installed_maps() {
    let mut a = app(Mode::UltraStable, "map-open");
    assert_eq!(
        a.entries(Category::Maps),
        [
            Entry::Builtin(Builtin::Map),
            Entry::Builtin(Builtin::MapFiles)
        ]
    );
    assert!(
        a.search("atlas")
            .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Map)))
    );
    // No map at all: explained, nothing runs.
    let (v, e) = a.activate(Entry::Builtin(Builtin::Map));
    assert!(v.is_none() && e == Effect::None);
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO MAP"));
    a.modal = None;

    // Only the built-in overview.
    let world = add_world_map(&a);
    let (v, e) = a.activate(Entry::Builtin(Builtin::Map));
    assert!(v.is_none());
    let Effect::Run(req) = e else {
        panic!("expected a run")
    };
    assert_eq!(req.argv, map_argv(&[&world]));
    assert!(req.extra_args.is_empty() && req.polkit_agent.is_none() && !req.pause_after);

    // With an installed pack: the overview first, then the pack. The older
    // version left in the packs directory is not opened by default.
    add_map_pack(&a);
    let pack = a.home.join("sys/packs/map-osm-testland_2026-01.mbtiles");
    let (_, e) = a.activate(Entry::Builtin(Builtin::Map));
    let Effect::Run(req) = e else {
        panic!("expected a run")
    };
    assert_eq!(req.argv, map_argv(&[&world, &pack]));

    // Every mode may look at maps.
    for mode in [Mode::Stable, Mode::Advanced] {
        let b = app(mode, "map-open-modes");
        assert_eq!(b.entries(Category::Maps).len(), 2, "{mode:?}");
    }
}

#[test]
fn maps_list_shows_only_maps_and_opens_the_chosen_one() {
    let mut a = app(Mode::UltraStable, "map-list");
    open(&mut a, Builtin::MapFiles);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("This computer has no map yet"), "{screen}");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    a.stack.pop();

    let world = add_world_map(&a);
    add_map_pack(&a);
    let dir = a.home.join("sys/packs");
    let pack = dir.join("map-osm-testland_2026-01.mbtiles");
    let older = dir.join("map-osm-testland_2025-07.mbtiles");
    // A map on an open stick, as the list would find it.
    let stick = a.home.join("stick");
    std::fs::create_dir_all(stick.join("Resilience-packs")).expect("mkdir");
    let own = stick.join("Resilience-packs/own-town.mbtiles");
    std::fs::write(&own, b"x").expect("w");
    let cfg = resilience_core::packs::PackConfig::load(&a.paths.packs_config).expect("cfg");
    let on_stick = stick_items(&cfg, "STICK", &stick, PackKind::Map);
    assert_eq!(on_stick.len(), 1);

    open(&mut a, Builtin::MapFiles);
    if let Some(View::Library { items, .. }) = a.stack.last_mut() {
        items.extend(on_stick);
    }
    let View::Library { kind, items, .. } = a.view() else {
        panic!("not on the maps list")
    };
    assert_eq!(*kind, PackKind::Map);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "World overview map (built in)",
            "Testland (OpenStreetMap)",
            "Testland (OpenStreetMap) (older version)",
            "own-town.mbtiles"
        ]
    );
    for (w, h) in [(80, 24), (60, 20), (160, 50)] {
        let screen = render(&mut a, w, h);
        assert!(screen.contains("MAPS"), "{screen}");
        assert!(screen.contains("World overview map"), "{screen}");
        assert!(!screen.contains("OFFLINE LIBRARY"), "{screen}");
        assert!(!screen.contains("Wikipedia"), "{screen}");
        assert_console_glyphs(&screen);
    }
    assert!(render(&mut a, 80, 24).contains("Enter Open the map"));

    let run = |a: &mut App| match press(a, KeyCode::Enter) {
        Effect::Run(req) => req.argv,
        e => panic!("expected a run, got {e:?}"),
    };
    // The overview alone.
    assert_eq!(run(&mut a), map_argv(&[&world]));
    // An installed pack: over the overview.
    press(&mut a, KeyCode::Down);
    assert_eq!(run(&mut a), map_argv(&[&world, &pack]));
    // The older version: asked for, so opened, before the current one.
    press(&mut a, KeyCode::Down);
    assert_eq!(run(&mut a), map_argv(&[&world, &older, &pack]));
    // A file on a stick: the same, with the installed pack.
    press(&mut a, KeyCode::Down);
    assert_eq!(run(&mut a), map_argv(&[&world, &own, &pack]));
    // c checks it with the library tool; s (full-text search) is for ZIM files.
    let Effect::Run(req) = press(&mut a, KeyCode::Char('c')) else {
        panic!("expected a run")
    };
    assert_eq!(
        req.argv,
        [
            "resilience-library".to_string(),
            "check".into(),
            own.display().to_string()
        ]
    );
    assert_eq!(press(&mut a, KeyCode::Char('s')), Effect::None);
    assert!(a.modal.is_none());
    // Help is about maps.
    press(&mut a, KeyCode::F(1));
    assert!(render(&mut a, 80, 24).contains("== MAPS =="));
    press(&mut a, KeyCode::Esc);
    // p: the packs screen.
    press(&mut a, KeyCode::Char('p'));
    assert!(matches!(a.view(), View::Packs { .. }));
}

#[test]
fn at_most_eight_maps_are_given_to_the_viewer() {
    let mut a = app(Mode::UltraStable, "map-many");
    add_world_map(&a);
    let sys = a.home.join("sys");
    let mut text = std::fs::read_to_string(sys.join("packs.toml")).expect("read");
    for i in 0..10 {
        let file = format!("map-osm-region{i}_2026-01.mbtiles");
        text.push_str(&format!(
            "[[pack]]\nid = \"map-osm-region{i}\"\ntitle = \"Region {i}\"\n\
             description = \"D\"\ncategory = \"maps\"\nlanguage = \"en\"\n\
             file = \"{file}\"\nsize = 1\nsha256 = \"{}\"\nlicense = \"ODbL-1.0\"\n\
             source = \"S\"\ndata_date = \"2026-01-01\"\ncoverage = \"Region {i} only\"\n\
             attribution = \"(c) OpenStreetMap contributors\"\n",
            "c".repeat(64)
        ));
        std::fs::write(sys.join("packs").join(file), b"x").expect("w");
    }
    std::fs::write(sys.join("packs.toml"), text).expect("write");
    let (_, e) = a.activate(Entry::Builtin(Builtin::Map));
    let Effect::Run(req) = e else {
        panic!("expected a run")
    };
    assert_eq!(req.argv.iter().filter(|x| *x == "--pack").count(), 8);
    assert!(req.argv[3].ends_with("map-world-natural-earth_2022-05.mbtiles"));
}

const MAP_COVERAGE: &str = "Testland only: streets, places and points of interest from \
    OpenStreetMap. Buildings and house numbers are left out. Outside Testland only the world \
    overview is shown. OpenStreetMap is made by volunteers: something absent from the map may \
    still exist. END-OF-COVERAGE";

/// Add a map pack (installed, no download address) and an older version of
/// it to the test catalog of `write_packs`. It is catalog pack 3.
fn add_map_pack(a: &App) {
    let sys = a.home.join("sys");
    let mut text = std::fs::read_to_string(sys.join("packs.toml")).expect("read");
    text.push_str(&format!(
        "[[pack]]\nid = \"map-osm-testland\"\ntitle = \"Testland (OpenStreetMap)\"\n\
         description = \"A detailed map of Testland\"\ncategory = \"maps\"\nlanguage = \"en\"\n\
         file = \"map-osm-testland_2026-01.mbtiles\"\nsize = 16\nsha256 = \"{}\"\n\
         license = \"ODbL-1.0\"\nsource = \"Geofabrik extract, built by resilience-mapmaker\"\n\
         data_date = \"2026-01-01\"\ncoverage = \"{MAP_COVERAGE}\"\n\
         attribution = \"(c) OpenStreetMap contributors\"\n",
        "b".repeat(64)
    ));
    std::fs::write(sys.join("packs.toml"), text).expect("write");
    std::fs::write(
        sys.join("packs/map-osm-testland_2026-01.mbtiles"),
        b"SQLite format 3\0",
    )
    .expect("w");
    std::fs::write(sys.join("packs/map-osm-testland_2025-07.mbtiles"), b"old").expect("w");
}

#[test]
fn map_pack_details_show_date_coverage_and_attribution() {
    let mut a = app(Mode::UltraStable, "map-details");
    add_map_pack(&a);
    open(&mut a, Builtin::Packs);
    use resilience_core::packs::PackCategory::*;
    assert_eq!(
        pack_rows(&a),
        vec![
            PackRow::Header(Encyclopedia),
            PackRow::Pack(0),
            PackRow::Pack(2),
            PackRow::Header(Health),
            PackRow::Pack(1),
            PackRow::Header(Maps),
            PackRow::Pack(3),
            PackRow::OthersHeader,
            PackRow::Other(0),
            PackRow::Other(1),
        ]
    );
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("-- MAPS --"), "{screen}");
    assert!(screen.contains("[*] Testland (OpenStreetMap)"), "{screen}");

    select_row(&mut a, PackRow::Pack(3));
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    let View::Pager { title, text, .. } = a.view() else {
        panic!("details are a scrolling view")
    };
    assert_eq!(title, "PACK DETAILS");
    // The coverage text is kept whole, never shortened.
    assert!(
        text.contains(&format!("Coverage: {MAP_COVERAGE}\n")),
        "{text}"
    );
    assert!(text.contains("Data date: 2026-01-01"), "{text}");
    assert!(
        text.contains("Attribution: (c) OpenStreetMap contributors"),
        "{text}"
    );
    assert!(text.contains("License: ODbL-1.0"), "{text}");
    assert!(text.contains("opened from Maps & navigation"), "{text}");
    assert!(text.contains("copy it from a USB stick"), "{text}");
    // On the smallest screen, the coverage, date and attribution are all on
    // the first page, before the size and the fingerprint.
    let screen = render(&mut a, 80, 24);
    for want in [
        "PACK DETAILS",
        "Coverage: Testland only",
        "END-OF-COVERAGE",
        "Data date: 2026-01-01",
        "Attribution: (c) OpenStreetMap",
    ] {
        assert!(screen.contains(want), "{want}: {screen}");
    }
    assert_console_glyphs(&screen);
    press(&mut a, KeyCode::Esc);
    assert!(matches!(a.view(), View::Packs { .. }));

    // A ZIM entry without the new fields shows none of them.
    select_row(&mut a, PackRow::Pack(0));
    press(&mut a, KeyCode::Enter);
    let View::Pager { text, .. } = a.view() else {
        panic!("details")
    };
    assert!(
        text.starts_with("Wikipedia (English)\n\nAbout wiki-en\n"),
        "{text}"
    );
    for absent in ["Coverage", "Data date", "Attribution", "Maps & navigation"] {
        assert!(!text.contains(absent), "{absent}: {text}");
    }
    assert!(text.contains("State: installed"), "{text}");
    press(&mut a, KeyCode::Esc);

    // The older map file is recognised as an older version of the map pack.
    select_row(&mut a, PackRow::Other(0));
    let View::Packs { s, .. } = a.view() else {
        panic!("packs")
    };
    let older: Vec<_> = s
        .inv
        .others
        .iter()
        .map(|o| (o.file.as_str(), o.older_of))
        .collect();
    assert_eq!(
        older,
        [
            ("map-osm-testland_2025-07.mbtiles", Some(3)),
            ("wiki_en_2026-03.zim", Some(0))
        ]
    );
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains("older version of 'Testland")
            && text.contains("opened in Maps & navigation"))
    );
}

#[test]
fn the_library_does_not_offer_map_files_to_the_reader() {
    let mut a = app(Mode::UltraStable, "map-library");
    add_map_pack(&a);
    let stick = a.home.join("stick");
    std::fs::create_dir_all(stick.join("Resilience-packs")).expect("mkdir");
    std::fs::write(
        stick.join("map-osm-testland_2026-01.mbtiles"),
        b"SQLite format 3\0",
    )
    .expect("w");
    std::fs::write(stick.join("Resilience-packs/own-town.mbtiles"), b"x").expect("w");
    std::fs::write(stick.join("Resilience-packs/own_docs.zim"), b"x").expect("w");

    open(&mut a, Builtin::Library);
    let View::Library { items, .. } = a.view() else {
        panic!("not on the library view")
    };
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "System documentation (built in)",
            "Wikipedia (English)",
            "Wikipedia (English) (older version)"
        ]
    );
    assert!(
        items
            .iter()
            .all(|i| !i.path.to_string_lossy().ends_with(".mbtiles"))
    );
    // Going through the whole list never runs the reader on a map.
    for _ in 0..items.len() {
        if let Effect::Run(req) = press(&mut a, KeyCode::Enter) {
            assert!(!req.argv.iter().any(|x| x.ends_with(".mbtiles")), "{req:?}");
        }
        press(&mut a, KeyCode::Down);
    }

    // On a stick: the library sees only ZIM files; maps are kept apart for
    // Maps & navigation.
    let cfg = resilience_core::packs::PackConfig::load(&a.paths.packs_config).expect("cfg");
    let zim: Vec<String> = stick_items(&cfg, "STICK", &stick, PackKind::Zim)
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(zim, ["own_docs.zim"]);
    let maps: Vec<(String, ItemPlace)> = stick_items(&cfg, "STICK", &stick, PackKind::Map)
        .into_iter()
        .map(|i| (i.title, i.place))
        .collect();
    let on = |known| ItemPlace::Stick {
        volume: "STICK".into(),
        known,
    };
    assert_eq!(
        maps,
        [
            ("Testland (OpenStreetMap)".to_string(), on(true)),
            ("own-town.mbtiles".to_string(), on(false))
        ]
    );
}

fn type_text(a: &mut App, text: &str) {
    for c in text.chars() {
        press(a, KeyCode::Char(c));
    }
}

#[test]
fn manual_pages_open_in_a_secure_pager_or_are_searched() {
    let mut a = app(Mode::UltraStable, "man");
    assert!(
        a.entries(Category::Knowledge)
            .contains(&Entry::Builtin(Builtin::Manual))
    );
    let (v, e) = a.activate(Entry::Builtin(Builtin::Manual));
    assert!(v.is_none() && e == Effect::None);
    assert!(matches!(a.modal, Some(Modal::ManPage { .. })));
    assert_console_glyphs(&render(&mut a, 80, 24));
    // Options and shell syntax are refused in the dialog.
    type_text(&mut a, "-H ls");
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::ManPage { error: Some(e), .. }) if e.contains("no leading -"))
    );
    a.modal = Some(Modal::ManPage {
        value: String::new(),
        error: None,
    });
    type_text(&mut a, "nope");
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::ManPage { error: Some(e), .. }) if e.contains("No manual page"))
    );
    a.modal = Some(Modal::ManPage {
        value: String::new(),
        error: None,
    });
    type_text(&mut a, "ls");
    let Effect::Run(req) = press(&mut a, KeyCode::Enter) else {
        panic!("expected a run")
    };
    assert_eq!(req.argv[1..], ["ls".to_string()]);
    assert!(req.argv[0].ends_with("/sys/man"));
    assert!(req.env.contains(&("LESSSECURE".into(), "1".into())));
    assert!(req.clean_env, "man starts from an empty environment");
    assert!(
        req.env
            .contains(&("MANPAGER".into(), "/store/bin/less".into()))
    );
    // Several words: the matching pages, inside the interface.
    a.modal = Some(Modal::ManPage {
        value: String::new(),
        error: None,
    });
    type_text(&mut a, "list files");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    let View::Pager { text, .. } = a.view() else {
        panic!("expected the search results")
    };
    assert!(
        text.contains("ls (1)") && text.contains("list files"),
        "{text}"
    );
}

#[test]
fn files_are_found_by_name_and_shown_read_only() {
    let mut a = app(Mode::UltraStable, "files");
    let h = a.home.clone();
    std::fs::create_dir_all(h.join("Notes")).expect("mkdir");
    std::fs::create_dir_all(h.join("Data")).expect("mkdir");
    std::fs::create_dir_all(h.join(".secret")).expect("mkdir");
    std::fs::write(h.join("Notes/Water plan.md"), "Boil for one minute.\n").expect("w");
    std::fs::write(
        h.join("Data/budget.csv"),
        "item,cost,notes\nrice,3,\"dry, keep\"\nbeans,2,\n",
    )
    .expect("w");
    std::fs::write(h.join("Data/manual.pdf"), b"%PDF-1.4 fake").expect("w");
    std::fs::write(h.join("Data/photos.zip"), b"PK\x03\x04").expect("w");
    std::fs::write(h.join("Data/radio.bin"), b"\x00\x01\x02").expect("w");
    std::fs::write(h.join(".secret/water-key.txt"), "x").expect("w");
    open(&mut a, Builtin::Files);
    // Typing goes to the query, even '/' and '?'.
    type_text(&mut a, "water");
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("~/Notes/Water plan.md"), "{screen}");
    assert!(
        !screen.contains("water-key"),
        "hidden files are not searched: {screen}"
    );
    assert_console_glyphs(&screen);
    press(&mut a, KeyCode::Enter);
    let View::Pager { text, .. } = a.view() else {
        panic!("text shown in the pager")
    };
    assert!(text.contains("Boil for one minute"));
    press(&mut a, KeyCode::Esc);

    // A CSV file as a table; the columns scroll sideways.
    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "budget");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::Table(_)));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("item  | cost | notes"), "{screen}");
    assert!(screen.contains("rice  | 3    | dry, keep"), "{screen}");
    press(&mut a, KeyCode::Right);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("cost | notes"), "{screen}");
    assert!(!screen.contains("item |"), "{screen}");
    press(&mut a, KeyCode::Esc);
    assert!(matches!(a.view(), View::Files { .. }));

    let show = |a: &mut App, name: &str| {
        for _ in 0..10 {
            press(a, KeyCode::Backspace);
        }
        type_text(a, name);
        press(a, KeyCode::Enter)
    };
    // A PDF through pdftotext, an archive listed, never extracted.
    show(&mut a, "manual");
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains("PDF TEXT of") && text.contains("manual.pdf"))
    );
    press(&mut a, KeyCode::Esc);
    show(&mut a, "photos");
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains("inside.txt") && text.contains("nothing is extracted"))
    );
    press(&mut a, KeyCode::Esc);
    // Anything else: details; F3 adds the SHA-256.
    show(&mut a, "radio");
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("press F3")),
        "{:?}",
        a.modal
    );
    a.modal = None;
    assert_eq!(press(&mut a, KeyCode::F(3)), Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, body, .. })
        if title == "FILE DETAILS" && body.contains("SHA-256: ae4b3280e56e2faf83f414a6e3dabe9d5fbe18976544c05fed121accb85b53fc")),
        "{:?}",
        a.modal
    );
}

// ----- cryptography (ADR 0015) ------------------------------------------------

const AGE_PUB: &str = "age1fnc4atdyhfzcxgdtr7zmcgx6mgwzd4x4gehhge7myxhn4kjdy9ps7jmt65";
const MS_PUB: &str = "RWQAfmjhd7nnGUBTzemb8G8wF9ElertEKU4C1kTZoS2w7SMHLeK0wIfG";

fn crypto_argv(e: &Effect) -> (Vec<String>, Vec<PathBuf>) {
    match e {
        Effect::Run(r) => {
            assert_eq!(r.argv[0], "/store/bin/resilience-crypto");
            assert!(r.pause_after && r.polkit_agent.is_none());
            (r.argv[1..].to_vec(), r.extra_args.clone())
        }
        other => panic!("expected a run, got {other:?}"),
    }
}

/// Own keys and one contact (Alice) in the key store of `a`'s home.
fn make_keys(a: &App) {
    use resilience_core::crypto::{Contact, KeyStore};
    let ks = KeyStore::for_home(&a.home);
    ks.ensure_dirs().expect("dirs");
    std::fs::write(ks.encryption_key(), "-----BEGIN AGE ENCRYPTED FILE-----\n").expect("w");
    std::fs::write(ks.encryption_pub(), format!("{AGE_PUB}\n")).expect("w");
    std::fs::write(
        ks.signing_key(),
        "untrusted comment: minisign encrypted secret key\n",
    )
    .expect("w");
    std::fs::write(
        ks.signing_pub(),
        format!("untrusted comment: minisign public key\n{MS_PUB}\n"),
    )
    .expect("w");
    ks.save_contact(&Contact {
        name: "Alice".into(),
        age: Some(AGE_PUB.into()),
        minisign: None,
        added: None,
    })
    .expect("contact");
}

#[test]
fn security_tools_are_offered_in_every_mode() {
    for mode in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let mut a = app(mode, "crypto-modes");
        let names: Vec<&str> = a
            .search("encrypt")
            .iter()
            .filter_map(|h| match h {
                SearchHit::Entry(Entry::Builtin(b)) => Some(b.name()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"Encrypt a file"), "{mode:?}: {names:?}");
        assert!(names.contains(&"Encrypted messages"), "{mode:?}: {names:?}");
        for b in [
            Builtin::Verify,
            Builtin::Checksums,
            Builtin::Keys,
            Builtin::Random,
        ] {
            assert_eq!(b.category(), Category::Security);
            assert!(mode.allows(b.min_mode()));
        }
        let screen = render(&mut a, 80, 24);
        assert!(screen.contains("SECURITY & CRYPTOGRAPHY"), "{screen}");
    }
}

#[test]
fn encrypting_a_file_asks_for_whom_then_runs_the_tool() {
    let mut a = app(Mode::UltraStable, "crypto-encrypt");
    std::fs::write(a.home.join("plan.txt"), "meet at the bridge").expect("w");
    open(&mut a, Builtin::Encrypt);
    assert!(matches!(
        a.view(),
        View::Files {
            pick: Some(Pick::Encrypt),
            ..
        }
    ));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("CHOOSE THE FILE TO ENCRYPT"), "{screen}");
    assert!(screen.contains("Enter Choose this file"), "{screen}");
    type_text(&mut a, "plan");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    // No keys yet: only a passphrase, ticked.
    let Some(Modal::Recipients { rows, ticked, .. }) = &a.modal else {
        panic!("recipients: {:?}", a.modal)
    };
    assert_eq!(rows, &vec![Recipient::Passphrase]);
    assert_eq!(ticked, &vec![true]);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("[x] A passphrase"), "{screen}");
    assert_console_glyphs(&screen);
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["encrypt", "--passphrase"]);
    assert_eq!(extra, vec![a.home.join("plan.txt")]);
    // The picker closed.
    assert!(!matches!(a.view(), View::Files { .. }));

    // With keys: me and Alice, never together with a passphrase.
    make_keys(&a);
    open(&mut a, Builtin::Encrypt);
    type_text(&mut a, "plan");
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Recipients { rows, .. }) = &a.modal else {
        panic!("recipients")
    };
    assert_eq!(
        rows,
        &vec![
            Recipient::Passphrase,
            Recipient::Me,
            Recipient::Contact("Alice".into())
        ]
    );
    press(&mut a, KeyCode::Char(' ')); // untick the passphrase
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Recipients { error: Some(e), .. }) if e.contains("Tick")),
        "nothing ticked: {:?}",
        a.modal
    );
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' '));
    let Some(Modal::Recipients { ticked, .. }) = &a.modal else {
        panic!("recipients")
    };
    assert_eq!(ticked, &vec![false, true, true]);
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Char(' ')); // the passphrase unticks the keys
    let Some(Modal::Recipients { ticked, .. }) = &a.modal else {
        panic!("recipients")
    };
    assert_eq!(ticked, &vec![true, false, false]);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' '));
    let (args, _) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["encrypt", "--me", "--to", "Alice"]);
    // Esc cancels without running anything.
    open(&mut a, Builtin::Encrypt);
    type_text(&mut a, "plan");
    press(&mut a, KeyCode::Enter);
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::None);
    assert!(a.modal.is_none());
}

#[test]
fn decrypt_verify_and_inspect_pick_a_file_and_run_the_tool() {
    let mut a = app(Mode::UltraStable, "crypto-pick");
    std::fs::write(a.home.join("plan.txt.age"), "age-encryption.org/v1\n").expect("w");
    std::fs::write(a.home.join("release.txt"), "1.0").expect("w");
    std::fs::write(a.home.join("release.txt.minisig"), "untrusted comment: x\n").expect("w");
    for (b, query, op, name) in [
        (Builtin::Decrypt, ".age", "decrypt", "plan.txt.age"),
        (Builtin::Verify, ".minisig", "verify", "release.txt.minisig"),
        (Builtin::Inspect, "plan", "inspect", "plan.txt.age"),
    ] {
        open(&mut a, b);
        assert!(matches!(a.view(), View::Files { query, .. } if query.is_empty()));
        type_text(&mut a, query);
        let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
        assert_eq!(args, vec![op]);
        assert_eq!(extra, vec![a.home.join(name)]);
    }
    // Signing needs a key: explained, nothing opens.
    let (v, e) = a.activate(Entry::Builtin(Builtin::Sign));
    assert!(v.is_none() && e == Effect::None);
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO KEY YET"));
    a.modal = None;
    make_keys(&a);
    open(&mut a, Builtin::Sign);
    type_text(&mut a, "release.txt");
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["sign"]);
    assert_eq!(extra, vec![a.home.join("release.txt")]);
}

#[test]
fn messages_and_checksums_menus() {
    let mut a = app(Mode::UltraStable, "crypto-menus");
    std::fs::write(a.home.join("debian.iso"), "x").expect("w");
    open(&mut a, Builtin::Messages);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Write an encrypted message"), "{screen}");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        &a.modal,
        Some(Modal::Recipients {
            target: EncryptTarget::Message,
            ..
        })
    ));
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["write-message", "--passphrase"]);
    assert!(extra.is_empty());
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        a.view(),
        View::Files {
            pick: Some(Pick::ReadMessage),
            ..
        }
    ));
    press(&mut a, KeyCode::Esc);

    // Compare with a published checksum: the input is checked first.
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    open(&mut a, Builtin::Checksums);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    type_text(&mut a, "debian");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(&a.modal, Some(Modal::ExpectedSum { .. })));
    type_text(&mut a, "not a sum");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::ExpectedSum { error: Some(e), .. }) if e.contains("hexadecimal"))
    );
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("COMPARE WITH A CHECKSUM"), "{screen}");
    for _ in 0..9 {
        press(&mut a, KeyCode::Backspace);
    }
    let sum = format!("SHA256 (debian.iso) = {}", "ab".repeat(32));
    type_text(&mut a, &sum);
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["compare".to_string(), sum]);
    assert_eq!(extra, vec![a.home.join("debian.iso")]);
}

#[test]
fn random_secrets_are_shown_once_and_never_logged() {
    let mut a = app(Mode::UltraStable, "crypto-random");
    open(&mut a, Builtin::Random);
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Message { body, error, .. }) = &a.modal else {
        panic!("{:?}", a.modal)
    };
    assert!(!error);
    assert!(
        body.contains("graceful encore snooze reoccur factor 6"),
        "{body}"
    );
    assert!(body.contains("about 76 bits"), "{body}");
    let screen = render(&mut a, 80, 24);
    assert_console_glyphs(&screen);
    a.modal = None;
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("q2Fz+R0b/x9kLm3nT7uVwYzA") && body.contains("144 bits"))
    );
    a.modal = None;
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("00112233 44556677"))
    );
    for n in &a.notices {
        assert!(
            !n.text.contains("graceful") && !n.text.contains("q2Fz"),
            "{}",
            n.text
        );
    }
}

#[test]
fn keys_screen_creates_adds_exports_and_removes() {
    let mut a = app(Mode::UltraStable, "crypto-keys");
    open(&mut a, Builtin::Keys);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("not created yet"), "{screen}");
    assert!(screen.contains("No contacts yet"), "{screen}");
    assert_console_glyphs(&screen);
    let (args, _) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["new-keys"]);
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Char('t')));
    assert_eq!(args, vec!["add-contact"]);
    assert!(extra.is_empty());
    press(&mut a, KeyCode::Char('a'));
    assert!(matches!(
        a.view(),
        View::Files {
            pick: Some(Pick::AddContact),
            ..
        }
    ));
    press(&mut a, KeyCode::Esc);
    // No keys: nothing to export.
    press(&mut a, KeyCode::Char('e'));
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO KEY YET"));
    a.modal = None;

    // After a run, the list shows the keys and the contact.
    make_keys(&a);
    let ok: std::io::Result<std::process::ExitStatus> = Ok(std::process::ExitStatus::default());
    a.after_run(CRYPTO_LABEL, &ok);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("ends with ...ps7j mt65"), "{screen}");
    assert!(screen.contains("number 19E7B977E1687E00"), "{screen}");
    assert!(screen.contains("Alice"), "{screen}");
    assert!(screen.contains("1 contact)"), "{screen}");
    // Enter now shows details (public keys only).
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains(AGE_PUB) && text.contains(MS_PUB))
    );
    press(&mut a, KeyCode::Esc);

    // Export to a stick: the list of sticks, then the tool on the mount.
    press(&mut a, KeyCode::Char('e'));
    assert!(matches!(
        a.view(),
        View::Media {
            purpose: MediaPurpose::ExportKeys,
            ..
        }
    ));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("SAVE YOUR PUBLIC KEYS"), "{screen}");
    let (args, extra) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(args, vec!["export-keys"]);
    assert_eq!(extra, vec![PathBuf::from("/run/media/tester/STICK")]);
    a.after_run(CRYPTO_LABEL, &ok);
    assert!(matches!(a.view(), View::Keys { .. }));

    // Remove Alice: confirmation, Cancel by default.
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Delete);
    assert!(matches!(
        &a.modal,
        Some(Modal::Confirm {
            yes_focused: false,
            ..
        })
    ));
    press(&mut a, KeyCode::Enter);
    assert!(
        resilience_core::crypto::KeyStore::for_home(&a.home)
            .contacts()
            .len()
            == 1,
        "cancel keeps the contact"
    );
    press(&mut a, KeyCode::Delete);
    press(&mut a, KeyCode::Char('y'));
    assert!(
        resilience_core::crypto::KeyStore::for_home(&a.home)
            .contacts()
            .is_empty()
    );
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("0 contacts"), "{screen}");
}

#[test]
fn openpgp_keys_are_listed_offered_and_removable() {
    let mut a = app(Mode::UltraStable, "crypto-pgp");
    std::fs::write(a.home.join("plan.txt"), "x").expect("w");
    // gpg is only asked once the user has a GnuPG folder.
    open(&mut a, Builtin::Keys);
    assert!(!render(&mut a, 80, 24).contains("Bob Builder"));
    press(&mut a, KeyCode::Esc);
    std::fs::create_dir_all(a.home.join(".gnupg")).expect("mkdir");
    open(&mut a, Builtin::Keys);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Bob Builder"), "{screen}");
    assert!(
        screen.contains("OpenPGP key        ...B0B0 B0B0"),
        "{screen}"
    );
    assert!(screen.contains("g creates one"), "{screen}");
    assert!(screen.contains("(1 contact)"), "{screen}");
    assert_console_glyphs(&screen);
    let (args, _) = crypto_argv(&press(&mut a, KeyCode::Char('g')));
    assert_eq!(args, vec!["new-openpgp-key"]);
    // Details, then removal (confirmed) through gpg --delete-keys.
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::Pager { text, .. } if text.contains("0123 4567 89AB CDEF")));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Delete);
    press(&mut a, KeyCode::Char('y'));
    let log = std::fs::read_to_string(a.home.join("sys/gpg.log")).expect("log");
    assert!(
        log.contains("--yes --homedir")
            && log.contains("--delete-keys 0123456789ABCDEF0123456789ABCDEFB0B0B0B0"),
        "{log}"
    );

    // Encrypt for: OpenPGP keys are offered, never mixed with age.
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    make_keys(&a);
    open(&mut a, Builtin::Encrypt);
    type_text(&mut a, "plan.txt");
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Recipients { rows, .. }) = &a.modal else {
        panic!("recipients")
    };
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(
        matches!(&rows[3], Recipient::OpenPgp { name, .. } if name == "Bob Builder <bob@example.org>")
    );
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("OpenPGP: Bob Builder"), "{screen}");
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' ')); // me (age)
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Char(' ')); // Bob (OpenPGP): unticks me
    let Some(Modal::Recipients { ticked, .. }) = &a.modal else {
        panic!("recipients")
    };
    assert_eq!(ticked, &vec![false, false, false, true]);
    let (args, _) = crypto_argv(&press(&mut a, KeyCode::Enter));
    assert_eq!(
        args,
        vec![
            "encrypt",
            "--to-pgp",
            "0123456789ABCDEF0123456789ABCDEFB0B0B0B0"
        ]
    );
}

/// A fake `pkexec` (in the test admin.toml) that logs its arguments and
/// fails for the layout "de", as the helper would for a layout it refuses.
fn fake_keymap_helper(a: &mut App) -> std::path::PathBuf {
    use resilience_core::keyboard::{KeyboardConfig, Layout};
    use std::os::unix::fs::PermissionsExt;
    let dir = a.home.join("sys");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let log = dir.join("pkexec.log");
    let script = dir.join("pkexec");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\necho \"SHELL=$SHELL\" >> {}\nif [ \"$3\" = de ]; then echo 'this keyboard layout is not offered' >&2; exit 1; fi\n",
            log.display(),
            log.display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let admin = std::fs::read_to_string(&a.paths.admin_config).expect("admin.toml");
    std::fs::write(
        &a.paths.admin_config,
        admin.replace("/run/wrappers/bin/pkexec", &script.display().to_string()),
    )
    .expect("admin.toml");
    let l = |id: &str, label: &str| Layout {
        id: id.into(),
        label: label.into(),
    };
    a.sys.keyboard = KeyboardConfig {
        default: "us".into(),
        layouts: vec![
            l("us", "English (US, QWERTY)"),
            l("fr", "French (AZERTY)"),
            l("de", "German (QWERTZ)"),
        ],
        state_file: None,
    };
    log
}

#[test]
fn keyboard_layout_is_loaded_saved_and_restored_at_login() {
    let mut a = app(Mode::UltraStable, "keyboard");
    let log = fake_keymap_helper(&mut a);
    let (v, _) = a.activate(Entry::Builtin(Builtin::Settings));
    a.stack.push(v.expect("settings"));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Keyboard  French (AZERTY)"), "{screen}");
    assert!(
        screen.contains("Keyboard  English (US, QWERTY)  (current)"),
        "{screen}"
    );
    // Themes first, then layouts: French is the 6th row.
    press(&mut a, KeyCode::Home);
    for _ in 0..5 {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.settings.keyboard.as_deref(), Some("fr"));
    assert_eq!(
        UserSettings::load(&a.settings_path).0.keyboard.as_deref(),
        Some("fr")
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("log"),
        "/store/bin/resilience-admin set-keymap fr\nSHELL=/run/current-system/sw/bin/resilience-session\n"
    );
    assert!(render(&mut a, 80, 24).contains("French (AZERTY)  (current)"));
    // A layout loadkeys refuses is reported and not saved.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.settings.keyboard.as_deref(), Some("fr"));
    assert!(a.notices.iter().any(|n| n.text.contains("not offered")));

    // Next login: the saved choice is loaded again; the default is not
    // (the login prompt already loaded it); a choice no longer offered
    // falls back to the default with a warning.
    let mut b = app(Mode::UltraStable, "keyboard-login");
    let log = fake_keymap_helper(&mut b);
    b.settings.keyboard = Some("fr".into());
    b.apply_saved_layout();
    b.settings.keyboard = Some("us".into());
    b.apply_saved_layout();
    b.settings.keyboard = Some("xx; rm".into());
    b.apply_saved_layout();
    let calls = std::fs::read_to_string(&log).expect("log");
    assert_eq!(calls.matches("set-keymap").count(), 1, "{calls}");
    assert!(calls.contains("set-keymap fr"), "{calls}");
    assert!(
        b.notices
            .iter()
            .any(|n| n.text.contains("no longer offered"))
    );
}

#[test]
fn the_mesh_is_offered_explained_and_turned_on_and_off() {
    let mut a = app(Mode::UltraStable, "mesh");
    // Without a mesh on this system: no entry, not even in search.
    assert!(!a.builtin_present(Builtin::Mesh));
    let log = fake_keymap_helper(&mut a);
    let admin = std::fs::read_to_string(&a.paths.admin_config).expect("admin.toml");
    std::fs::write(
        &a.paths.admin_config,
        format!("mesh_unit = \"resilience-mesh.service\"\n{admin}"),
    )
    .expect("admin.toml");
    assert!(a.builtin_present(Builtin::Mesh));
    assert!(
        a.search("mesh")
            .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Mesh)))
    );
    assert!(
        a.search("router")
            .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Mesh)))
    );

    // Off: what turning it on means, Cancel first.
    a.activate(Entry::Builtin(Builtin::Mesh));
    let screen = render(&mut a, 100, 40);
    assert!(screen.contains("MESH NETWORK"), "{screen}");
    assert!(screen.contains("The mesh network is off."), "{screen}");
    assert!(screen.contains("cannot join an"), "{screen}");
    assert!(screen.contains("Turn on"), "{screen}");
    assert_console_glyphs(&screen);
    press(&mut a, KeyCode::Enter);
    assert!(!log.exists(), "turned on without confirming");
    a.activate(Entry::Builtin(Builtin::Mesh));
    press(&mut a, KeyCode::Left);
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        std::fs::read_to_string(&log).expect("log"),
        "/store/bin/resilience-admin set-mesh on\nSHELL=/run/current-system/sw/bin/resilience-session\n"
    );
    assert!(
        a.notices
            .last()
            .expect("notice")
            .text
            .contains("The mesh network is on")
    );

    // On (the service wrote its cards): turning it off.
    std::fs::write(&a.paths.mesh_devices, "wlan0\n").expect("w");
    a.activate(Entry::Builtin(Builtin::Mesh));
    let screen = render(&mut a, 100, 40);
    assert!(
        screen.contains("The mesh network is ON (wlan0)"),
        "{screen}"
    );
    assert!(screen.contains("Turn off"), "{screen}");
    press(&mut a, KeyCode::Left);
    press(&mut a, KeyCode::Enter);
    assert!(
        std::fs::read_to_string(&log)
            .expect("log")
            .contains("set-mesh off")
    );
    assert!(a.notices.last().expect("n").text.contains("is off"));
}

// ----- local messages (ADR 0018) ---------------------------------------------

const ME: &str = "0123456789abcdef0123456789abcdef";
const MARIE: &str = "aaaabbbbccccddddeeeeffff00001111";

/// Local messaging enabled on the machine, the account's program "running".
fn chat_app(tag: &str) -> App {
    let a = app(Mode::UltraStable, tag);
    std::fs::write(&a.paths.comms_client, "[reticulum]\n").expect("w");
    let box_ = resilience_core::messages::store_dir(&a.home);
    std::fs::create_dir_all(box_.join("messages")).expect("mkdir");
    std::fs::write(box_.join("address"), format!("{ME}\n")).expect("w");
    std::fs::write(
        box_.join("status.json"),
        format!(
            r#"{{"address":"{ME}","name":"demo","pid":1,"time":{}}}"#,
            crate::app::chat_now()
        ),
    )
    .expect("w");
    a
}

/// What the messaging program would write for a message.
fn chat_record(a: &App, id: &str, direction: &str, state: &str, verified: bool, text: &str) {
    let body = serde_json_string(text);
    let detail = if state == "queued" {
        "waiting for the recipient"
    } else {
        ""
    };
    std::fs::write(
        resilience_core::messages::store_dir(&a.home)
            .join("messages")
            .join(format!("{id}.json")),
        format!(
            r#"{{"id":"{id}","direction":"{direction}","peer":"{MARIE}","text":{body},"created":{},"state":"{state}","verified":{verified},"detail":"{detail}"}}"#,
            crate::app::chat_now()
        ),
    )
    .expect("w");
}

/// A JSON string literal (the tests' texts need only these escapes).
fn serde_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Let the periodic reload of the messages screens happen.
fn chat_refresh(a: &mut App) {
    for _ in 0..8 {
        a.on_tick();
    }
}

#[test]
fn local_messages_exist_only_where_messaging_is_enabled() {
    let a = app(Mode::UltraStable, "chat-off");
    assert!(a.entries(Category::Communications).is_empty());
    assert!(!a.home_categories().contains(&Category::Communications));
    assert!(
        !a.search("nearby")
            .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Chat)))
    );
    for mode in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let b = app(mode, "chat-on");
        std::fs::write(&b.paths.comms_client, "x").expect("w");
        assert_eq!(
            b.entries(Category::Communications),
            [Entry::Builtin(Builtin::Chat)],
            "{mode:?}"
        );
        assert!(
            b.search("nearby")
                .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Chat)))
        );
    }
}

#[test]
fn my_address_and_a_stopped_program_are_said_plainly() {
    let mut a = app(Mode::UltraStable, "chat-stopped");
    std::fs::write(&a.paths.comms_client, "x").expect("w");
    open(&mut a, Builtin::Chat);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("LOCAL MESSAGES"), "{screen}");
    assert!(screen.contains("No messages yet"), "{screen}");
    assert!(screen.contains("Messaging is not running"), "{screen}");
    assert_console_glyphs(&screen);
    press(&mut a, KeyCode::Char('m'));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Messaging has not started"), "{screen}");
    press(&mut a, KeyCode::Enter);

    let mut a = chat_app("chat-address");
    open(&mut a, Builtin::Chat);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Messaging is running"), "{screen}");
    assert!(screen.contains("01234567.."), "{screen}");
    press(&mut a, KeyCode::Char('m'));
    let screen = render(&mut a, 80, 24);
    assert!(
        screen.contains("0123 4567 89ab cdef 0123 4567 89ab cdef"),
        "{screen}"
    );
    assert!(!screen.contains("not running"), "{screen}");
}

#[test]
fn a_message_store_and_a_copy_left_in_it_are_shown() {
    // No store heard: said plainly on the conversations screen.
    let mut a = chat_app("chat-nostore");
    open(&mut a, Builtin::Chat);
    let screen = render(&mut a, 120, 30);
    assert!(
        screen.contains("No message store on this network"),
        "{screen}"
    );

    // A store in use (ADR 0019) and a message whose copy waits in it.
    let mut a = chat_app("chat-store");
    let box_ = resilience_core::messages::store_dir(&a.home);
    std::fs::write(
        box_.join("status.json"),
        format!(
            r#"{{"address":"{ME}","name":"demo","pid":1,"time":{},"store":"{MARIE}"}}"#,
            crate::app::chat_now()
        ),
    )
    .expect("w");
    std::fs::write(
        box_.join("messages").join("m1.json"),
        format!(
            r#"{{"id":"m1","direction":"out","peer":"{MARIE}","text":"Left for later","created":{},"state":"sent","stored":true,"detail":"a copy waits"}}"#,
            crate::app::chat_now()
        ),
    )
    .expect("w");
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Up);
    let screen = render(&mut a, 120, 30);
    assert!(screen.contains("wait in a message store"), "{screen}");
    press(&mut a, KeyCode::Enter);
    let screen = render(&mut a, 120, 30);
    assert!(screen.contains("[sent, kept on the network]"), "{screen}");
    assert!(screen.contains("Left for later"), "{screen}");
    assert!(!screen.contains("[delivered]"), "{screen}");
    assert_console_glyphs(&screen);
}

#[test]
fn writing_to_an_address_saves_a_request_and_never_claims_more() {
    let mut a = chat_app("chat-write");
    let box_ = resilience_core::messages::store_dir(&a.home);
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Char('n'));
    assert!(render(&mut a, 80, 24).contains("WRITE TO AN ADDRESS"));
    // Not an address; then my own address: explained, nothing written.
    type_text(&mut a, "marie");
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 80, 24).contains("An address has 32 characters"));
    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, ME);
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 80, 24).contains("This is your own address"));
    assert!(!box_.join("outbox").exists());
    press(&mut a, KeyCode::Esc);
    assert!(a.modal.is_none());

    // As read aloud: groups, upper case.
    press(&mut a, KeyCode::Char('n'));
    type_text(&mut a, "AAAA bbbb cccc dddd eeee ffff 0000 1111");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::Chat { peer, .. } if peer == MARIE));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("WRITE A MESSAGE"), "{screen}");
    // Empty: refused in the dialog.
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 80, 24).contains("the message is empty"));
    type_text(&mut a, "Water at the \"school\", 9h; $(reboot)");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());

    let files: Vec<PathBuf> = std::fs::read_dir(box_.join("outbox"))
        .expect("outbox")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].extension().is_some_and(|e| e == "json"));
    let text = std::fs::read_to_string(&files[0]).expect("read");
    assert!(text.contains(&format!("\"to\":\"{MARIE}\"")), "{text}");
    assert!(
        text.contains(r#""text":"Water at the \"school\", 9h; $(reboot)""#),
        "{text}"
    );
    // Saved, not "sent" and certainly not "delivered".
    let said = &a.notices.last().expect("notice").text;
    assert!(said.contains("Message saved"), "{said}");
    let lower = said.to_lowercase();
    assert!(
        !lower.contains("delivered") && !lower.contains("sent"),
        "{said}"
    );
    let screen = render(&mut a, 80, 24);
    assert!(
        screen.contains("No messages with this person yet"),
        "{screen}"
    );
    assert!(!screen.contains("[delivered]"), "{screen}");
}

#[test]
fn a_message_can_have_several_lines() {
    let mut a = chat_app("chat-lines");
    let box_ = resilience_core::messages::store_dir(&a.home);
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Char('n'));
    type_text(&mut a, MARIE);
    press(&mut a, KeyCode::Enter);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    type_text(&mut a, "Water:");
    a.handle_key(ctrl('n'));
    type_text(&mut a, "school, 9h");
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    a.handle_key(ctrl('j'));
    type_text(&mut a, "Bring cans");
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("> Water:"), "{screen}");
    assert!(screen.contains("  school, 9h"), "{screen}");
    assert!(screen.contains("  Bring cans█"), "{screen}");
    assert!(screen.contains("Ctrl-N new line"), "{screen}");
    // Backspace takes a line break back like any character.
    for _ in 0.."Bring cans".len() + 1 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "Bring cans.");
    assert!(
        matches!(a.modal, Some(Modal::ChatInput { ref value, .. }) if value == "Water:\nschool, 9h\nBring cans.")
    );
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    let files: Vec<PathBuf> = std::fs::read_dir(box_.join("outbox"))
        .expect("outbox")
        .map(|e| e.expect("entry").path())
        .collect();
    let text = std::fs::read_to_string(&files[0]).expect("read");
    assert!(
        text.contains(r#""text":"Water:\nschool, 9h\nBring cans.""#),
        "{text}"
    );
    // Only line breaks: still empty.
    press(&mut a, KeyCode::Char('w'));
    for _ in 0..3 {
        a.handle_key(ctrl('n'));
    }
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 80, 24).contains("the message is empty"));
    press(&mut a, KeyCode::Esc);
    // A name has one line.
    press(&mut a, KeyCode::Char('a'));
    a.handle_key(ctrl('n'));
    assert!(matches!(a.modal, Some(Modal::ChatInput { ref value, .. }) if value.is_empty()));
    press(&mut a, KeyCode::Esc);
    // At most 40 lines (MAX_TYPED_LINES).
    press(&mut a, KeyCode::Char('w'));
    for _ in 0..100 {
        type_text(&mut a, "x");
        a.handle_key(ctrl('n'));
    }
    let Some(Modal::ChatInput { value, .. }) = &a.modal else {
        panic!("dialog closed");
    };
    assert_eq!(value.split('\n').count(), 40);
    // The end being typed stays on the screen.
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("WRITE A MESSAGE"), "{screen}");
    assert!(screen.contains("xxxxxxxxxxxxxxxxxxxxx"), "{screen}");
    assert!(screen.contains("█"), "{screen}");
}

#[test]
fn a_file_is_chosen_copied_and_sent_with_a_text() {
    let mut a = chat_app("chat-send-file");
    let box_ = resilience_core::messages::store_dir(&a.home);
    std::fs::write(a.home.join("water-points.txt"), "School\nChurch\n").expect("w");
    std::fs::write(a.home.join("too-big.iso"), vec![0u8; 900_001]).expect("w");
    chat_record(&a, "m1", "out", "delivered", false, "hello");
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 100, 30).contains("f File"));

    // Too large: said, nothing written.
    press(&mut a, KeyCode::Char('f'));
    assert!(render(&mut a, 100, 30).contains("CHOOSE THE FILE TO SEND"));
    type_text(&mut a, "too-big");
    press(&mut a, KeyCode::Enter);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("CANNOT SEND THIS FILE"), "{screen}");
    assert!(screen.contains("900 KB"), "{screen}");
    assert!(!box_.join("outbox").exists());
    press(&mut a, KeyCode::Esc);
    assert!(matches!(a.view(), View::Chat { .. }));

    press(&mut a, KeyCode::Char('f'));
    type_text(&mut a, "water-po");
    press(&mut a, KeyCode::Enter);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("SEND A FILE"), "{screen}");
    assert!(screen.contains("water-points.txt' (14 bytes)"), "{screen}");
    type_text(&mut a, "Where to find water");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none(), "{:?}", a.modal);
    assert!(matches!(a.view(), View::Chat { .. }));
    let said = &a.notices.last().expect("notice").text;
    assert!(said.contains("copy of the file is saved"), "{said}");
    let mut names: Vec<String> = std::fs::read_dir(box_.join("outbox"))
        .expect("outbox")
        .map(|e| e.expect("e").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names[0].ends_with(".file") && names[1].ends_with(".json"));
    assert_eq!(
        std::fs::read_to_string(box_.join("outbox").join(&names[0])).expect("r"),
        "School\nChurch\n"
    );
    let req = std::fs::read_to_string(box_.join("outbox").join(&names[1])).expect("r");
    assert!(req.contains(r#""file":"water-points.txt""#), "{req}");
    assert!(req.contains(r#""text":"Where to find water""#), "{req}");
}

#[test]
fn a_received_file_is_viewed_and_saved_without_replacing_anything() {
    let mut a = chat_app("chat-recv-file");
    let box_ = resilience_core::messages::store_dir(&a.home);
    std::fs::create_dir_all(box_.join("files")).expect("mkdir");
    std::fs::write(box_.join("files/in-1.txt"), "Bring cans\n").expect("w");
    let now = crate::app::chat_now();
    let rec = |id: &str, file: &str| {
        std::fs::write(
            box_.join("messages").join(format!("{id}.json")),
            format!(
                r#"{{"id":"{id}","direction":"in","peer":"{MARIE}","text":"","created":{now},"state":"received","verified":true,"file":{file}}}"#
            ),
        )
        .expect("w");
    };
    rec("in-0", r#"{"name":"huge.bin","size":5000000}"#);
    rec(
        "in-1",
        r#"{"name":"../list\u001b[2J.txt","size":11,"data":"in-1.txt"}"#,
    );
    std::fs::create_dir_all(a.home.join("Received")).expect("mkdir");
    std::fs::write(a.home.join("Received/_.._list [2J.txt"), "mine").expect("w");
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Enter);
    let screen = render(&mut a, 100, 30);
    assert!(
        screen.contains("[file] huge.bin (5.0 MB)  not kept"),
        "{screen}"
    );
    assert!(
        screen.contains("[file] ../list [2J.txt (11 bytes)  o open, s save"),
        "{screen}"
    );
    assert_console_glyphs(&screen);

    // The newest (in-1) is selected: view it, read-only, inside the TUI.
    assert_eq!(press(&mut a, KeyCode::Char('o')), Effect::None);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("Bring cans"), "{screen}");
    press(&mut a, KeyCode::Esc);
    assert!(matches!(a.view(), View::Chat { .. }));

    // Saved under a safe name beside the user's own file, which stays.
    press(&mut a, KeyCode::Char('s'));
    let said = a.notices.last().expect("notice").text.clone();
    assert!(
        said.contains("Saved as ~/Received/_.._list [2J (2).txt"),
        "{said}"
    );
    assert_eq!(
        std::fs::read_to_string(a.home.join("Received/_.._list [2J (2).txt")).expect("r"),
        "Bring cans\n"
    );
    assert_eq!(
        std::fs::read_to_string(a.home.join("Received/_.._list [2J.txt")).expect("r"),
        "mine"
    );
    assert!(!box_.join("files/in-1.txt").exists());
    // Moved, so neither shown nor saved again.
    press(&mut a, KeyCode::Char('o'));
    assert!(a.notices.last().expect("n").text.contains("no longer here"));
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Char('s'));
    assert!(a.notices.last().expect("n").text.contains("was not kept"));
}

#[test]
fn a_conversation_shows_what_is_known_about_each_message() {
    let mut a = chat_app("chat-states");
    let box_ = resilience_core::messages::store_dir(&a.home);
    chat_record(&a, "m1", "out", "queued", false, "Water at the school, 9h");
    open(&mut a, Builtin::Chat);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("[..] aaaabbbb.."), "{screen}");
    assert!(screen.contains("You: Water at the school, 9h"), "{screen}");
    assert!(
        screen.contains("1 waiting or sent without proof"),
        "{screen}"
    );
    press(&mut a, KeyCode::Enter);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("MESSAGES WITH aaaabbbb.."), "{screen}");
    assert!(screen.contains("You [waiting]"), "{screen}");
    assert!(screen.contains("waiting for the recipient"), "{screen}");

    // The program reports progress; the screen follows by itself.
    chat_record(&a, "m1", "out", "sent", false, "Water at the school, 9h");
    chat_refresh(&mut a);
    assert!(render(&mut a, 100, 30).contains("You [sent]"));
    chat_record(
        &a,
        "m1",
        "out",
        "delivered",
        false,
        "Water at the school, 9h",
    );
    chat_record(&a, "in-1", "in", "received", true, "OK, see you there");
    chat_record(
        &a,
        "in-2",
        "in",
        "received",
        false,
        "trust me\u{1b}[2J\u{202e}\nsecond line",
    );
    chat_record(&a, "m2", "out", "failed", false, "too late");
    chat_record(&a, "m3", "out", "queued", false, "still trying");
    chat_refresh(&mut a);
    let screen = render(&mut a, 100, 40);
    assert!(screen.contains("You [delivered]"), "{screen}");
    assert!(screen.contains("Them [from them]"), "{screen}");
    assert!(
        screen.contains("Them [from them, NOT VERIFIED]"),
        "{screen}"
    );
    assert!(screen.contains("You [FAILED]"), "{screen}");
    // The escape sequence and the direction override are not drawn.
    assert!(screen.contains("trust me[2J"), "{screen}");
    assert!(screen.contains("second line"), "{screen}");
    assert!(!screen.contains('\u{1b}') && !screen.contains('\u{202e}'));

    // Small and large screens: the newest message stays in view.
    for (w, h) in [(160, 50), (80, 24), (60, 20)] {
        let screen = render(&mut a, w, h);
        assert!(screen.contains("still trying"), "{screen}");
        assert_console_glyphs(&screen);
    }

    // c gives up on a waiting message only.
    let View::Chat { records, list, .. } = a.view() else {
        panic!("not in a conversation")
    };
    let order: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(list.selected(), Some(records.len() - 1));
    let waiting = order.iter().position(|i| *i == "m3").expect("m3");
    let done = order.iter().position(|i| *i == "m1").expect("m1");
    let n = order.len();
    press(&mut a, KeyCode::Home);
    for _ in 0..done {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Char('c'));
    assert!(!box_.join("cancel").exists());
    press(&mut a, KeyCode::Home);
    for _ in 0..waiting {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Char('c'));
    assert!(box_.join("cancel/m3").is_file());
    assert_eq!(
        std::fs::read_dir(box_.join("cancel")).expect("ls").count(),
        1
    );
    assert!(n >= 5);

    // a: a name of my own for this address; i: the whole address.
    press(&mut a, KeyCode::Char('a'));
    assert!(render(&mut a, 100, 40).contains("NAME THIS PERSON"));
    type_text(&mut a, "Marie");
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 100, 40).contains("MESSAGES WITH Marie (aaaabbbb..)"));
    press(&mut a, KeyCode::Char('i'));
    let screen = render(&mut a, 100, 40);
    assert!(
        screen.contains("aaaa bbbb cccc dddd eeee ffff 0000 1111"),
        "{screen}"
    );
    assert!(screen.contains("You named this address: Marie"), "{screen}");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::F(1));
    assert!(render(&mut a, 100, 40).contains("== LOCAL MESSAGES =="));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("[!!] Marie"), "{screen}");
}

#[test]
fn people_nearby_are_listed_with_claimed_names() {
    let mut a = chat_app("chat-nearby");
    let box_ = resilience_core::messages::store_dir(&a.home);
    open(&mut a, Builtin::Chat);
    press(&mut a, KeyCode::Char('p'));
    assert!(render(&mut a, 80, 24).contains("Nobody has been heard"));
    std::fs::write(
        box_.join("peers.json"),
        format!(
            r#"{{"{MARIE}":{{"name":"marie\u001b[31m","heard":{}}}}}"#,
            crate::app::chat_now()
        ),
    )
    .expect("w");
    press(&mut a, KeyCode::Char('r'));
    for (w, h) in [(80, 24), (60, 20), (160, 50)] {
        let screen = render(&mut a, w, h);
        assert!(screen.contains("PEOPLE NEARBY"), "{screen}");
        assert_console_glyphs(&screen);
    }
    let screen = render(&mut a, 100, 30);
    // A claimed name is quoted and questioned, and cleaned.
    assert!(screen.contains("\"marie [31m\"?"), "{screen}");
    assert!(
        screen.contains("aaaa bbbb cccc dddd eeee ffff 0000 1111"),
        "{screen}"
    );
    press(&mut a, KeyCode::Char('i'));
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("says its name is"), "{screen}");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Char('a'));
    type_text(&mut a, "Marie C.");
    press(&mut a, KeyCode::Enter);
    assert!(render(&mut a, 100, 30).contains("Marie C. "));
    // Enter opens the conversation, ready to write.
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::Chat { peer, .. } if peer == MARIE));
    press(&mut a, KeyCode::Char('w'));
    type_text(&mut a, "hello");
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        std::fs::read_dir(box_.join("outbox")).expect("ls").count(),
        1
    );
}

/// A live system with the installer (ADR 0022): a fake lsblk with one
/// usable disk and the live stick (mounted, so never offered).
fn enable_installer(a: &App) {
    use std::os::unix::fs::PermissionsExt;
    let sys = a.home.join("sys");
    let lsblk = sys.join("install-lsblk");
    std::fs::write(
        &lsblk,
        r#"#!/bin/sh
cat <<LSBLK
NAME="sda" PATH="/dev/sda" PKNAME="" TYPE="disk" RM="0" RO="0" SIZE="500107862016" MODEL="Samsung SSD 860" SERIAL="S3Z" TRAN="sata" FSTYPE="" LABEL="" PARTLABEL="" MOUNTPOINT=""
NAME="sda1" PATH="/dev/sda1" PKNAME="sda" TYPE="part" RM="0" RO="0" SIZE="499570991104" MODEL="" SERIAL="" TRAN="" FSTYPE="ntfs" LABEL="Windows" PARTLABEL="" MOUNTPOINT=""
NAME="sdb" PATH="/dev/sdb" PKNAME="" TYPE="disk" RM="1" RO="0" SIZE="32010928128" MODEL="Flash Disk" SERIAL="" TRAN="usb" FSTYPE="" LABEL="" PARTLABEL="" MOUNTPOINT=""
NAME="sdb1" PATH="/dev/sdb1" PKNAME="sdb" TYPE="part" RM="1" RO="0" SIZE="1600000000" MODEL="" SERIAL="" TRAN="" FSTYPE="iso9660" LABEL="RESILIENCE" PARTLABEL="" MOUNTPOINT="/iso"
LSBLK
"#,
    )
    .expect("write lsblk");
    std::fs::set_permissions(&lsblk, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut text = format!(
        "edition = \"standard\"\npath = \"/run/current-system/sw/bin\"\n\
         keymaps = [\"us\", \"fr\"]\nlsblk = \"{}\"\n",
        lsblk.display()
    );
    for k in [
        "installer",
        "wipefs",
        "sfdisk",
        "udevadm",
        "mkfs_vfat",
        "mkfs_ext4",
        "cryptsetup",
        "mount",
        "umount",
        "nixos_install",
        "nixos_enter",
        "loadkeys",
        "grub_install",
        "sync",
    ] {
        text.push_str(&format!("{k} = \"/store/bin/{k}\"\n"));
    }
    text.push_str("[systems]\n");
    for k in ["uefi_plain", "uefi_luks", "bios_plain", "bios_luks"] {
        text.push_str(&format!("{k} = \"/store/{k}\"\n"));
    }
    std::fs::write(sys.join("install.toml"), text).expect("write install.toml");
}

fn install_rows(a: &App) -> Vec<InstallRow> {
    match a.view() {
        View::Install { form, .. } => form.rows(),
        v => panic!("not the Install screen: {v:?}"),
    }
}

fn install_form(a: &App) -> InstallForm {
    match a.view() {
        View::Install { form, .. } => (**form).clone(),
        v => panic!("not the Install screen: {v:?}"),
    }
}

fn select_install_row(a: &mut App, row: InstallRow) {
    let i = install_rows(a).iter().position(|r| *r == row).expect("row");
    press(a, KeyCode::Home);
    for _ in 0..i {
        press(a, KeyCode::Down);
    }
}

#[test]
fn installer_is_offered_only_on_live_systems() {
    let a = app(Mode::UltraStable, "install-absent");
    assert!(
        !a.entries(Category::System)
            .contains(&Entry::Builtin(Builtin::Install))
    );
    enable_installer(&a);
    assert!(
        a.entries(Category::System)
            .contains(&Entry::Builtin(Builtin::Install))
    );
}

#[test]
fn installer_gathers_answers_and_runs_only_after_the_disk_name_is_typed() {
    let mut a = app(Mode::UltraStable, "install");
    enable_installer(&a);
    let (v, e) = a.activate(Entry::Builtin(Builtin::Install));
    assert_eq!(e, Effect::None);
    a.stack.push(v.expect("install view"));
    let f = install_form(&a);
    assert!(f.encrypt, "encryption is on by default");
    assert_eq!(f.firmware, resilience_core::install::Firmware::Bios);
    assert_eq!(f.keymap, "us");
    assert_eq!(
        f.accounts
            .iter()
            .map(|x| (x.name.as_str(), x.mode))
            .collect::<Vec<_>>(),
        [("user", Mode::Stable), ("administrator", Mode::Advanced)]
    );

    // Install without a disk: refused with a message.
    select_install_row(&mut a, InstallRow::Install);
    press(&mut a, KeyCode::Enter);
    assert!(matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "CHOOSE A DISK"));
    press(&mut a, KeyCode::Enter);

    // The live stick is in use and cannot be chosen; the SSD can.
    select_install_row(&mut a, InstallRow::Disk);
    press(&mut a, KeyCode::Enter);
    let View::InstallDisks { disks, .. } = a.view() else {
        panic!("disk list");
    };
    assert_eq!(disks.len(), 2);
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("Windows"), "{screen}");
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(matches!(&a.modal, Some(Modal::Message { error: true, .. })));
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        install_form(&a).disk.map(|d| d.path),
        Some("/dev/sda".to_string())
    );

    // Encryption off and on again; keyboard French.
    select_install_row(&mut a, InstallRow::Encrypt);
    press(&mut a, KeyCode::Enter);
    assert!(!install_form(&a).encrypt);
    press(&mut a, KeyCode::Enter);
    assert!(install_form(&a).encrypt);
    select_install_row(&mut a, InstallRow::Keyboard);
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert_eq!(install_form(&a).keymap, "fr");

    // Rename the first account (a bad name is refused), change its mode.
    select_install_row(&mut a, InstallRow::Account(0));
    press(&mut a, KeyCode::Enter);
    for _ in 0..4 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "root");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        &a.modal,
        Some(Modal::InstallText { error: Some(_), .. })
    ));
    for _ in 0..4 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "jeanne");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Char('m'));
    assert_eq!(install_form(&a).accounts[0].name, "jeanne");
    assert_eq!(install_form(&a).accounts[0].mode, Mode::Advanced);
    press(&mut a, KeyCode::Char('m'));
    assert_eq!(install_form(&a).accounts[0].mode, Mode::UltraStable);
    press(&mut a, KeyCode::Char('m'));

    // Add an account with a full name; remove the administrator.
    press(&mut a, KeyCode::Char('a'));
    type_text(&mut a, "kid");
    press(&mut a, KeyCode::Enter);
    type_text(&mut a, "The kid");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    select_install_row(&mut a, InstallRow::Account(1));
    press(&mut a, KeyCode::Char('d'));
    let names: Vec<String> = install_form(&a)
        .accounts
        .into_iter()
        .map(|x| x.name)
        .collect();
    assert_eq!(names, ["jeanne", "kid"]);

    // The summary names the disk, what is on it, and warns: no administrator.
    select_install_row(&mut a, InstallRow::Install);
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        &a.modal,
        Some(Modal::InstallText {
            field: InstallInput::Confirm,
            ..
        })
    ));
    let screen = render(&mut a, 100, 50);
    for want in [
        "ERASED",
        "/dev/sda",
        "Windows",
        "no Advanced",
        "type the disk",
    ] {
        assert!(screen.contains(want), "{want}: {screen}");
    }
    // Anything but the disk's name does nothing.
    type_text(&mut a, "sdb");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(
        &a.modal,
        Some(Modal::InstallText { error: Some(_), .. })
    ));
    for _ in 0..3 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "sda");
    let Effect::Run(req) = press(&mut a, KeyCode::Enter) else {
        panic!("the installer should run");
    };
    assert_eq!(req.label, INSTALL_LABEL);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        req.argv,
        s(&[
            "/run/wrappers/bin/pkexec",
            "/store/bin/installer",
            "run",
            "--disk",
            "/dev/sda",
            "--size",
            "500107862016",
            "--model",
            "Samsung SSD 860",
            "--firmware",
            "bios",
            "--encrypt",
            "yes",
            "--keymap",
            "fr",
            "--account",
            "jeanne:stable:Everyday account",
            "--account",
            "kid:stable:The kid",
        ])
    );
    assert_eq!(
        req.env,
        vec![(
            "SHELL".to_string(),
            "/run/current-system/sw/bin/resilience-session".to_string()
        )]
    );
}

// ----- offline updates (ADR 0023) ----------------------------------------

const RELEASE_KEY: &str = "resilience-1:q7BpySi/HVuhHVoI3ItwWPLezKuu9eI6JTw1s46B6wW=";
const MANIFEST: &str = "/nix/store/1rvx69m7j5wd3g2cwpdhcxnz7cpjlzk7-resilience-update-manifest";

fn updatable(a: &mut App) {
    a.sys.updates = Some(resilience_core::update::UpdateSettings {
        nix: "/store/bin/nix".into(),
        trusted_keys: vec![RELEASE_KEY.into()],
        variant: "installed-standard-uefi".into(),
        edition: "standard".into(),
        version: "0.3.0".into(),
    });
}

fn stick_volume(mount: Option<PathBuf>) -> resilience_core::media::Volume {
    resilience_core::media::Volume {
        device: "/dev/sda1".into(),
        drive: "/dev/sda".into(),
        label: "STICK".into(),
        fstype: "vfat".into(),
        size: 134_217_728,
        model: String::new(),
        mountpoint: mount,
        available: None,
        read_only: false,
        problem: None,
    }
}

/// A stick folder holding a bundle whose pointer says `version`.
fn update_stick(a: &App, version: &str) -> PathBuf {
    let stick = a.home.join("update-stick");
    let b = stick.join("resilience-update");
    std::fs::create_dir_all(&b).expect("mkdir");
    std::fs::write(
        b.join("resilience-update.toml"),
        format!("format = 1\nedition = \"standard\"\nversion = \"{version}\"\nmanifest = \"{MANIFEST}\"\n"),
    )
    .expect("w");
    stick
}

fn push_updates(a: &mut App, stick: &Path) {
    let rows = crate::app::update_ui_rows(vec![stick_volume(Some(stick.to_path_buf()))]);
    a.stack.push(View::Updates {
        rows,
        error: None,
        list: resilience_shell::list::ListState::default().with_selected(Some(0)),
    });
}

#[test]
fn update_entry_only_when_the_system_takes_updates() {
    let mut a = app(Mode::UltraStable, "upd-hidden");
    open(&mut a, Builtin::Recovery);
    let s = render(&mut a, 80, 24);
    assert!(!s.contains("Update from a USB stick"), "{s}");
    // [3] is the report, as before.
    assert!(s.contains("[3] Export a diagnostic report"), "{s}");

    let mut a = app(Mode::UltraStable, "upd-shown");
    updatable(&mut a);
    open(&mut a, Builtin::Recovery);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("[3] Update from a USB stick"), "{s}");
    press(&mut a, KeyCode::Char('3'));
    assert!(matches!(a.view(), View::Updates { .. }), "{:?}", a.view());
    let s = render(&mut a, 100, 30);
    assert!(s.contains("UPDATE FROM A USB STICK"), "{s}");
    // The fake stick is not open: Enter mounts it through udisks.
    assert!(s.contains("not open"), "{s}");
    press(&mut a, KeyCode::Enter);
    assert!(udisks_log(&a).contains("mount --no-user-interaction --block-device /dev/sda1"));
    assert!(matches!(a.view(), View::Updates { .. }));

    // A key that is not a valid Nix key: no updates offered.
    let mut a = app(Mode::UltraStable, "upd-badkey");
    updatable(&mut a);
    if let Some(u) = a.sys.updates.as_mut() {
        u.trusted_keys = vec!["nonsense".into()];
    }
    assert!(
        !a.recovery_items()
            .contains(&crate::app::RecoveryItem::Update)
    );
}

#[test]
fn update_rows_read_the_claim_on_opened_sticks_only() {
    let a = app(Mode::UltraStable, "upd-rows");
    let stick = update_stick(&a, "0.4.0");
    let empty = a.home.join("empty-stick");
    std::fs::create_dir_all(&empty).expect("mkdir");
    let bad = a.home.join("bad-stick");
    std::fs::create_dir_all(bad.join("resilience-update")).expect("mkdir");
    std::fs::write(
        bad.join("resilience-update/resilience-update.toml"),
        "format = 9\n",
    )
    .expect("w");
    let rows = crate::app::update_ui_rows(vec![
        stick_volume(Some(stick.clone())),
        stick_volume(Some(empty)),
        stick_volume(Some(bad)),
        stick_volume(None),
    ]);
    let (dir, p) = rows[0].ready().expect("an update");
    assert_eq!(dir, stick.join("resilience-update"));
    assert_eq!(p.version.to_string(), "0.4.0");
    assert!(rows[1].bundle.is_none() && rows[1].claim.is_none());
    assert!(matches!(&rows[2].claim, Some(Err(_))));
    assert!(rows[2].ready().is_none());
    assert!(rows[3].bundle.is_none());
}

#[test]
fn checking_an_update_runs_the_helper_as_the_user() {
    let mut a = app(Mode::UltraStable, "upd-check");
    updatable(&mut a);
    let stick = update_stick(&a, "0.4.0");
    push_updates(&mut a, &stick);
    let s = render(&mut a, 100, 30);
    assert!(s.contains("update 0.4.0"), "{s}");
    assert!(s.contains("it is not checked"), "{s}");
    for (w, h) in [(80, 24), (60, 20), (40, 12)] {
        assert_console_glyphs(&render(&mut a, w, h));
    }
    match press(&mut a, KeyCode::Char('c')) {
        Effect::Run(r) => {
            assert_eq!(r.label, UPDATE_CHECK_LABEL);
            assert_eq!(
                r.argv,
                vec![
                    "/store/bin/resilience-admin".to_string(),
                    "check-update".into()
                ]
            );
            assert_eq!(r.extra_args, vec![stick.join("resilience-update")]);
            assert!(r.polkit_agent.is_none(), "checking needs no password");
            assert!(r.pause_after);
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
            assert!(matches!(a.view(), View::Updates { .. }));
        }
        other => panic!("expected a run, got {other:?}"),
    }
}

#[test]
fn installing_an_update_confirms_then_runs_the_helper_through_pkexec() {
    let mut a = app(Mode::UltraStable, "upd-apply");
    updatable(&mut a);
    let stick = update_stick(&a, "0.4.0");
    push_updates(&mut a, &stick);
    press(&mut a, KeyCode::Enter);
    let dir = stick.join("resilience-update");
    match &a.modal {
        Some(Modal::Confirm {
            title,
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert_eq!(title, "INSTALL AN UPDATE");
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("standard 0.4.0"), "{body}");
            assert!(body.contains("This computer runs standard 0.3.0"), "{body}");
            assert!(body.contains("next restart"), "{body}");
            assert!(body.contains("checked again"), "{body}");
            assert_eq!(*action, Pending::ApplyUpdate { dir: dir.clone() });
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    assert_console_glyphs(&render(&mut a, 80, 24));
    // Enter on the default button cancels.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());

    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(r.label, UPDATE_APPLY_LABEL);
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec".to_string(),
                    "/store/bin/resilience-admin".into(),
                    "apply-update".into(),
                    dir.to_string_lossy().into_owned(),
                ]
            );
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
            assert!(matches!(a.view(), View::Updates { .. }));
        }
        other => panic!("expected a run, got {other:?}"),
    }
}

#[test]
fn a_stick_without_an_update_only_explains() {
    let mut a = app(Mode::UltraStable, "upd-none");
    updatable(&mut a);
    let empty = a.home.join("empty-stick");
    std::fs::create_dir_all(&empty).expect("mkdir");
    push_updates(&mut a, &empty);
    assert!(render(&mut a, 100, 30).contains("no update"));
    assert_eq!(press(&mut a, KeyCode::Char('c')), Effect::None);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO UPDATE HERE"),
        "{:?}",
        a.modal
    );
    a.modal = None;
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(&a.modal, Some(Modal::Message { .. })));
}

// ----- the licence notice (ADR 0024) -------------------------------------

/// The notice on, with a fake pkexec that appends a record line (or, when
/// `refuse` exists, fails like the helper refusing).
fn licence_app(tag: &str) -> App {
    use std::os::unix::fs::PermissionsExt;
    let mut a = app(Mode::UltraStable, tag);
    let dir = a.home.join("sys");
    let text = dir.join("LICENSE");
    std::fs::write(
        &text,
        "Hippocratic License Version Number: 2.1.\nPurpose. ...\nEND-OF-LICENCE-TEXT\n",
    )
    .expect("w");
    let record = dir.join("licence-acceptances.log");
    let script = dir.join("pkexec");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> {d}/pkexec.log\n\
             if [ -e {d}/refuse ]; then echo 'Refused: this is not the licence text of this system.' >&2; exit 2; fi\n\
             echo \"time=2026-11-18T20:00:00Z user=tester uid=1000 sha256=$3 licence=\\\"Hippocratic License\\\" version=2.1\" >> {r}\n",
            d = dir.display(),
            r = record.display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let admin = std::fs::read_to_string(&a.paths.admin_config).expect("admin.toml");
    std::fs::write(
        &a.paths.admin_config,
        admin.replace("/run/wrappers/bin/pkexec", &script.display().to_string()),
    )
    .expect("admin.toml");
    a.sys.licence = Some(resilience_core::licence::LicenceSettings {
        name: "Hippocratic License".into(),
        version: "2.1".into(),
        file: text,
        sha256: "ab".repeat(32),
        record,
        prompt: true,
    });
    let v = a.licence_view().expect("the notice is needed");
    a.stack.push(v);
    a
}

#[test]
fn the_licence_notice_blocks_everything_until_read_ticked_and_recorded() {
    let mut a = licence_app("licence");
    for (w, h) in [(80, 24), (60, 20), (40, 12), (120, 40)] {
        assert_console_glyphs(&render(&mut a, w, h));
    }
    let s = render(&mut a, 100, 30);
    assert!(s.contains("HIPPOCRATIC LICENSE 2.1: SUMMARY"), "{s}");
    assert!(
        s.contains("[ ] I certify that I have read the Hippocratic License 2.1"),
        "{s}"
    );
    // Nothing else is reachable: search, help, status, Esc to the menu.
    for k in [
        KeyCode::Char('/'),
        KeyCode::F(1),
        KeyCode::F(5),
        KeyCode::Char('?'),
    ] {
        press(&mut a, k);
        assert!(
            matches!(a.view(), View::Licence { .. }),
            "{k:?}: {:?}",
            a.view()
        );
        assert!(a.modal.is_none());
    }
    // Ticking or confirming before opening the text only explains.
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::Licence { ticked: false, .. }));
    assert!(
        a.notices
            .iter()
            .any(|n| n.text.contains("Read the original text first"))
    );
    assert!(!a.home.join("sys/pkexec.log").exists());
    // The original text, then the box.
    press(&mut a, KeyCode::Tab);
    let s = render(&mut a, 100, 30);
    assert!(
        s.contains("ORIGINAL TEXT (2/2)") && s.contains("END-OF-LICENCE-TEXT"),
        "{s}"
    );
    press(&mut a, KeyCode::Enter);
    assert!(
        a.notices
            .iter()
            .any(|n| n.text.contains("Tick the box first"))
    );
    press(&mut a, KeyCode::Char(' '));
    assert!(render(&mut a, 100, 30).contains("[x] I certify"));
    // Esc asks before logging out; the safe choice is the default.
    press(&mut a, KeyCode::Esc);
    assert!(matches!(
        &a.modal,
        Some(Modal::Confirm {
            action: Pending::DeclineLicence,
            yes_focused: false,
            ..
        })
    ));
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(a.view(), View::Licence { ticked: true, .. }));
    // Enter records it through the helper: the notice is gone.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    let calls = std::fs::read_to_string(a.home.join("sys/pkexec.log")).expect("log");
    assert_eq!(
        calls,
        format!(
            "/store/bin/resilience-admin record-licence {}\n",
            "ab".repeat(32)
        )
    );
    assert!(matches!(a.view(), View::Home { .. }), "{:?}", a.view());
    assert!(!a.licence_needed());
    assert!(a.licence_view().is_none());
}

#[test]
fn a_refused_record_keeps_the_notice_and_declining_logs_out() {
    let mut a = licence_app("licence-refused");
    std::fs::write(a.home.join("sys/refuse"), "").expect("w");
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Char('x'));
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, body, error: true }) if title == "NOT RECORDED" && body.contains("not the licence text")),
        "{:?}",
        a.modal
    );
    a.modal = None;
    assert!(matches!(a.view(), View::Licence { .. }));
    assert!(a.licence_needed());
    press(&mut a, KeyCode::Char('q'));
    press(&mut a, KeyCode::Char('y'));
    // 'y' on the confirmation logs out.
    let mut b = licence_app("licence-decline");
    press(&mut b, KeyCode::Char('q'));
    assert_eq!(press(&mut b, KeyCode::Char('y')), Effect::Quit);
}

#[test]
fn the_notice_is_not_shown_once_recorded_or_when_off() {
    let mut a = licence_app("licence-other");
    // Another account's record, or another text's, does not count.
    let l = a.sys.licence.clone().expect("licence");
    std::fs::write(
        &l.record,
        format!(
            "time=x user=bob uid=1001 sha256={}\ntime=x user=tester uid=1000 sha256={}\n",
            l.sha256,
            "cd".repeat(32)
        ),
    )
    .expect("w");
    assert!(a.licence_needed());
    std::fs::write(
        &l.record,
        format!("time=x user=tester uid=1000 sha256={}\n", l.sha256),
    )
    .expect("w");
    assert!(!a.licence_needed());
    std::fs::remove_file(&l.record).expect("rm");
    if let Some(l) = a.sys.licence.as_mut() {
        l.prompt = false;
    }
    assert!(!a.licence_needed());
}

/// Community network (ADR 0025): the screen exists only where sharing is
/// offered; computers heard sharing are listed with how to reach them;
/// their pages open through `resilience-library open-remote`; sharing asks
/// for confirmation (Cancel first) and an administrator's password, and
/// while it is on every screen says so.
#[test]
fn community_network_finds_reads_and_shares() {
    let mut a = app(Mode::UltraStable, "community");
    assert!(!a.builtin_present(Builtin::Community));
    let admin = std::fs::read_to_string(&a.paths.admin_config).expect("admin.toml");
    std::fs::write(
        &a.paths.admin_config,
        format!(
            "share = {{ unit = \"resilience-share\", services = [\"packs\", \"library\", \"files\", \"pages\"], state_dir = \"/run/resilience-share\" }}\n{admin}"
        ),
    )
    .expect("admin.toml");
    assert!(a.builtin_present(Builtin::Community));
    assert!(
        a.search("shelter")
            .contains(&SearchHit::Entry(Entry::Builtin(Builtin::Community)))
    );
    // This computer is on 192.168.1.0/24 (eth1).
    std::fs::write(
        &a.paths.route4,
        "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
         eth1\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n",
    )
    .expect("route");
    std::fs::write(&a.paths.route6, "").expect("route6");
    let now = crate::app::chat_now();
    let store = resilience_core::messages::store_dir(&a.home);
    std::fs::create_dir_all(&store).expect("mkdir");
    std::fs::write(
        store.join("shares.json"),
        format!(
            r#"{{"0123456789abcdef0123456789abcdef": {{"name": "School library", "services": ["packs", "library"], "port": 80, "addresses": ["192.168.1.9", "fe80::9"], "heard": {now}}},
                "fedcba9876543210fedcba9876543210": {{"name": "Far away", "services": ["pages"], "port": 80, "addresses": ["10.9.9.9"], "heard": {now}}},
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": {{"name": "Free WiFi", "services": ["packs"], "port": 80, "addresses": ["8.8.8.8"], "heard": {now}}}}}"#
        ),
    )
    .expect("shares");

    let (v, _) = a.activate(Entry::Builtin(Builtin::Community));
    a.stack.push(v.expect("community"));
    let screen = render(&mut a, 100, 30);
    assert!(screen.contains("COMMUNITY NETWORK"), "{screen}");
    assert!(
        screen.contains("Share this computer's resources"),
        "{screen}"
    );
    assert!(screen.contains("\"School library\"?"), "{screen}");
    assert!(screen.contains("192.168.1.9:80"), "{screen}");
    assert!(screen.contains("not reachable from here"), "{screen}");
    assert!(!screen.contains("Free WiFi"), "{screen}");
    assert_console_glyphs(&screen);

    // The reachable computer: its front page, library and packs.
    let rows = a.community_rows();
    let school = rows
        .iter()
        .position(|r| matches!(r, crate::app::CommunityRow::Machine { heard, .. } if heard.name == "School library"))
        .expect("school");
    let far = rows
        .iter()
        .position(|r| matches!(r, crate::app::CommunityRow::Machine { heard, .. } if heard.name == "Far away"))
        .expect("far");
    press(&mut a, KeyCode::Home);
    for _ in 0..far {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.modal, Some(Modal::Message { ref title, .. }) if title == "NOT REACHABLE"));
    a.modal = None;
    press(&mut a, KeyCode::Home);
    for _ in 0..school {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Enter);
    let View::PeerMenu { actions, .. } = a.view() else {
        panic!("a computer's menu");
    };
    assert_eq!(
        actions,
        &[
            crate::app::PeerAction::Index,
            crate::app::PeerAction::Library,
            crate::app::PeerAction::Packs
        ]
    );
    press(&mut a, KeyCode::Down);
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => assert_eq!(
            r.argv,
            vec![
                a.sys.tools.library.clone(),
                "open-remote".into(),
                "192.168.1.9:80".into(),
                "library".into()
            ]
        ),
        e => panic!("expected the library to open, got {e:?}"),
    }
    press(&mut a, KeyCode::Esc);

    // Typing an address: only local ones on this computer's networks.
    press(&mut a, KeyCode::Char('a'));
    type_text(&mut a, "8.8.8.8");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        a.modal,
        Some(Modal::ShareInput { error: Some(_), .. })
    ));
    for _ in 0..7 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "192.168.1.20");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    let View::PeerMenu { actions, .. } = a.view() else {
        panic!("a computer's menu");
    };
    assert_eq!(actions.len(), 5);
    press(&mut a, KeyCode::Esc);

    // Sharing: the form, the warning (Cancel first), then the helper.
    let net = &a.paths.sys_net;
    for (name, flags, kind) in [
        ("eth1", "0x1003", "1"),
        ("lo", "0x9", "772"),
        ("eth2", "0x1002", "1"),
    ] {
        std::fs::create_dir_all(net.join(name)).expect("mkdir");
        std::fs::write(net.join(name).join("flags"), flags).expect("w");
        std::fs::write(net.join(name).join("type"), kind).expect("w");
    }
    press(&mut a, KeyCode::Home);
    press(&mut a, KeyCode::Enter);
    let View::ShareSetup { form, .. } = a.view() else {
        panic!("the share form");
    };
    assert_eq!(form.interfaces, vec!["eth1".to_string()]);
    // The name: a bad one is refused, a good one kept.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    for _ in 0..12 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "<b>");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(
        a.modal,
        Some(Modal::ShareInput { error: Some(_), .. })
    ));
    for _ in 0..3 {
        press(&mut a, KeyCode::Backspace);
    }
    type_text(&mut a, "School library");
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    let shared = a.paths.shared_dir.join("files");
    std::fs::create_dir_all(&shared).expect("mkdir");
    {
        use std::os::unix::fs::PermissionsExt;
        for (n, mode) in [
            ("notice.txt", 0o644),
            ("private.txt", 0o600),
            (".hidden", 0o644),
        ] {
            std::fs::write(shared.join(n), "x").expect("w");
            std::fs::set_permissions(shared.join(n), std::fs::Permissions::from_mode(mode))
                .expect("chmod");
        }
    }
    let screen = render(&mut a, 100, 30);
    assert!(
        screen.contains("SHARE THIS COMPUTER'S RESOURCES"),
        "{screen}"
    );
    assert!(
        screen.contains("Shared folder: 1 file(s) served, 1 private"),
        "{screen}"
    );
    assert!(screen.contains("ANYONE"), "{screen}");
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { ref body, .. }) = a.modal else {
        panic!("a confirmation");
    };
    assert!(body.contains("ANYONE"), "{body}");
    // Cancel is the default.
    assert!(matches!(press(&mut a, KeyCode::Enter), Effect::None));
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Left);
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec",
                    "/store/bin/resilience-admin",
                    "set-share",
                    "on",
                    "eth1",
                    "--name",
                    "School library",
                    "packs",
                    "library"
                ]
            );
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
        }
        e => panic!("expected the helper, got {e:?}"),
    }

    // While sharing: a line on every screen, and Enter stops it.
    std::fs::write(
        &a.paths.share_state,
        "interface = \"eth1\"\nname = \"School library\"\nservices = [\"packs\", \"library\"]\nconnection = \"\"\n",
    )
    .expect("state");
    a.after_run(
        crate::app::SET_SHARE_LABEL_FOR_TESTS,
        &Ok(std::process::ExitStatus::default()),
    );
    assert!(matches!(a.view(), View::Community { .. }));
    let screen = render(&mut a, 100, 30);
    assert!(
        screen.contains("SHARING packs, library on eth1"),
        "{screen}"
    );
    assert!(screen.contains("[ON] Sharing"), "{screen}");
    press(&mut a, KeyCode::Home);
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Left);
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => assert_eq!(&r.argv[2..], ["set-share", "off"]),
        e => panic!("expected the helper, got {e:?}"),
    }
    a.stack.truncate(1);
    assert!(render(&mut a, 100, 30).contains("SHARING packs, library"));

    // The hotspot (ADR 0025 §6): offered where the helper has it, Cancel
    // first, then the helper in the foreground (it shows the password).
    let admin = std::fs::read_to_string(&a.paths.admin_config).expect("admin.toml");
    std::fs::write(
        &a.paths.admin_config,
        format!(
            "hotspot = {{ unit = \"resilience-hotspot.service\", state_dir = \"/run/resilience-hotspot\", ssid = \"Resilience-hotspot\", mesh_devices = \"/run/resilience-mesh/devices\" }}\n{admin}"
        ),
    )
    .expect("admin.toml");
    let rows = a.community_rows();
    let hot = rows
        .iter()
        .position(|r| matches!(r, crate::app::CommunityRow::Hotspot { on: None }))
        .expect("hotspot row");
    let (v, _) = a.activate(Entry::Builtin(Builtin::Community));
    a.stack.push(v.expect("community"));
    assert!(render(&mut a, 100, 30).contains("Create a network: hotspot"));
    press(&mut a, KeyCode::Home);
    for _ in 0..hot {
        press(&mut a, KeyCode::Down);
    }
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { ref body, .. }) = a.modal else {
        panic!("a confirmation");
    };
    assert!(body.contains("no access to the"), "{body}");
    assert!(matches!(press(&mut a, KeyCode::Enter), Effect::None));
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Left);
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => {
            assert_eq!(&r.argv[2..], ["set-hotspot", "on"]);
            assert!(r.pause_after, "the password must stay on screen");
            assert!(r.polkit_agent.is_some());
        }
        e => panic!("expected the helper, got {e:?}"),
    }
    std::fs::write(
        &a.paths.hotspot_state,
        "device = \"wlan0\"\nssid = \"Resilience-hotspot\"\n",
    )
    .expect("state");
    assert!(
        a.community_rows()
            .contains(&crate::app::CommunityRow::Hotspot {
                on: Some("\"Resilience-hotspot\" on wlan0".into())
            })
    );
}

// Pointer entry points used by the GUI (ADR 0026).

#[test]
fn choose_row_opens_like_enter() {
    let mut a = app(Mode::UltraStable, "choose");
    let cats = a.home_categories();
    assert!(cats.len() >= 2);
    assert_eq!(a.choose_row(1), Effect::None);
    match a.view() {
        View::Category { cat, .. } => assert_eq!(*cat, cats[1]),
        _ => panic!("category view"),
    }
    assert_eq!(a.selected_row(), Some(0));
    // A modal blocks pointer choices underneath it.
    a.go_home();
    press(&mut a, KeyCode::Char('q'));
    assert!(a.modal.is_some());
    assert_eq!(a.choose_row(0), Effect::None);
    assert!(matches!(a.view(), View::Home { .. }));
}

#[test]
fn go_home_and_set_search() {
    let mut a = app(Mode::UltraStable, "gohome");
    a.choose_row(0);
    a.set_search("edit");
    let View::Search { query, hits, .. } = a.view() else {
        panic!("search view")
    };
    assert_eq!(query, "edit");
    assert!(!hits.is_empty());
    let depth = a.stack.len();
    // Typing more replaces the search, it does not stack another one.
    a.set_search("editor");
    assert_eq!(a.stack.len(), depth);
    a.set_search(&"x".repeat(100));
    let View::Search { query, .. } = a.view() else {
        panic!("search view")
    };
    assert_eq!(query.chars().count(), 64);
    a.go_home();
    assert_eq!(a.stack.len(), 1);
    assert!(matches!(a.view(), View::Home { .. }));
}

#[test]
fn go_home_never_skips_the_licence_notice() {
    let mut a = licence_app("gohome-licence");
    assert!(matches!(a.view(), View::Licence { .. }));
    a.go_home();
    a.set_search("x");
    assert!(matches!(a.view(), View::Licence { .. }));
    assert_eq!(a.choose_row(0), Effect::None);
    assert!(matches!(a.view(), View::Licence { .. }));
}

#[test]
fn search_ranks_names_before_descriptions() {
    let a = app(Mode::UltraStable, "search-rank");
    let hits = a.search("recovery");
    assert!(
        matches!(
            hits.first(),
            Some(SearchHit::Entry(Entry::Builtin(Builtin::Recovery)))
        ),
        "{hits:?}"
    );
}

fn add_mapmaker(a: &App) {
    let regions = a.home.join("sys/regions.json");
    std::fs::write(
        &regions,
        r#"[
  {"id":"europe","name":"Europe","pbf":"https://download.geofabrik.de/europe-latest.osm.pbf","poly":"https://download.geofabrik.de/europe.poly"},
  {"id":"bretagne","name":"Bretagne","parent":"europe","pbf":"https://download.geofabrik.de/europe/france/bretagne-latest.osm.pbf","poly":"https://download.geofabrik.de/europe/france/bretagne.poly"}
]"#,
    )
    .expect("regions");
    std::fs::write(
        &a.paths.mapmaker_config,
        format!(
            "program = \"/bin/mapmaker\"\nregions = \"{}\"\nexport_config = \"/e\"\n\
             osmium = \"/o\"\ntippecanoe = \"/t\"\ncurl = \"/c\"\nmd5sum = \"/m\"\ntools = \"x\"\n",
            regions.display()
        ),
    )
    .expect("config");
}

#[test]
fn make_a_map_lists_regions_runs_the_builder_and_lists_the_result() {
    let mut a = app(Mode::UltraStable, "make-map");
    assert!(!a.builtin_present(Builtin::MakeMap));
    add_mapmaker(&a);
    assert!(a.builtin_present(Builtin::MakeMap));
    assert!(
        a.entries(Category::Maps)
            .contains(&Entry::Builtin(Builtin::MakeMap))
    );
    open(&mut a, Builtin::MakeMap);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("Europe > Bretagne"), "{screen}");
    // Typing filters; j and k are letters here, not moves.
    for c in "bjk".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    assert!(render(&mut a, 80, 24).contains("No region matches"));
    press(&mut a, KeyCode::Backspace);
    press(&mut a, KeyCode::Backspace);
    for c in "ret".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("REGIONS (1)"), "{screen}");
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => {
            assert_eq!(r.argv, ["/bin/mapmaker", "make", "--region", "bretagne"]);
            assert!(r.pause_after);
            assert!(r.polkit_agent.is_none());
        }
        e => panic!("expected the builder, got {e:?}"),
    }
    assert!(matches!(a.view(), View::MakeMap { .. }));

    // The builder made the map: it is marked, listed and opened.
    let maps = a.home.join("Maps");
    std::fs::create_dir_all(&maps).expect("mkdir");
    std::fs::write(maps.join("map-osm-bretagne_2026-11.mbtiles"), b"x").expect("w");
    std::fs::write(maps.join("notes.txt"), b"x").expect("w");
    a.after_run(MAKE_MAP_LABEL, &Ok(std::process::ExitStatus::default()));
    assert!(render(&mut a, 100, 24).contains("made here: data of 2026-11"));
    press(&mut a, KeyCode::Esc);
    add_world_map(&a);
    open(&mut a, Builtin::MapFiles);
    let View::Library { items, .. } = a.view() else {
        panic!("not on the maps list")
    };
    let made: Vec<_> = items
        .iter()
        .filter(|i| i.place == ItemPlace::Made)
        .collect();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].title, "Bretagne");
    assert_eq!(made[0].tag(), "Made here");
    a.stack.pop();
    match a.activate(Entry::Builtin(Builtin::Map)) {
        (_, Effect::Run(r)) => assert!(
            r.argv
                .iter()
                .any(|x| x.ends_with("Maps/map-osm-bretagne_2026-11.mbtiles")),
            "{:?}",
            r.argv
        ),
        other => panic!("expected the map viewer, got {other:?}"),
    }
}
