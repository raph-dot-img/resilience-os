//! Personal notes stored as plain text files in `~/Notes`.
//!
//! Note names are validated strictly so that a name can never escape the
//! notes directory or start a hidden file.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NoteError {
    #[error("the name is empty")]
    Empty,
    #[error("the name is longer than 64 characters")]
    TooLong,
    #[error("only letters, digits, spaces, '-', '_' and '.' are allowed")]
    BadCharacter,
    #[error("the name may not start with '.' or a space")]
    BadStart,
    #[error("a note with this name already exists")]
    Exists,
    #[error("the note is larger than 1 MB: open it in a terminal editor")]
    TooBig,
    #[error("the note is not plain text (UTF-8): open it in a terminal editor")]
    NotText,
    #[error("this is not a note in your Notes folder")]
    NotANote,
    #[error("{0}")]
    Io(String),
}

/// Notes larger than this are not opened in the built-in editor (the
/// graphical edition's; the terminal editors have their own limits).
pub const MAX_EDIT: u64 = 1024 * 1024;

/// Whether `path` is a note the built-in editor may open: a direct child
/// of `dir` with a valid note name (no hidden file, no other folder).
pub fn is_note_path(dir: &Path, path: &Path) -> bool {
    path.parent() == Some(dir)
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| file_name_for(n).is_ok_and(|f| f == n))
}

/// Read a note for the built-in editor: a regular file (never through a
/// symbolic link), at most [`MAX_EDIT`] bytes, UTF-8.
pub fn read_for_edit(dir: &Path, path: &Path) -> Result<String, NoteError> {
    use std::io::Read;
    if !is_note_path(dir, path) {
        return Err(NoteError::NotANote);
    }
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| NoteError::Io(e.to_string()))?;
    let meta = f.metadata().map_err(|e| NoteError::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(NoteError::NotANote);
    }
    if meta.len() > MAX_EDIT {
        return Err(NoteError::TooBig);
    }
    let mut bytes = Vec::new();
    f.take(MAX_EDIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| NoteError::Io(e.to_string()))?;
    if bytes.len() as u64 > MAX_EDIT {
        return Err(NoteError::TooBig);
    }
    String::from_utf8(bytes).map_err(|_| NoteError::NotText)
}

/// Save a note from the built-in editor: written to a new private file in
/// the same folder, flushed to disk, then renamed over the note, so a
/// crash or a full disk never leaves half a note. At most [`MAX_EDIT`]
/// bytes.
pub fn save(dir: &Path, path: &Path, text: &str) -> Result<(), NoteError> {
    use std::io::Write;
    if !is_note_path(dir, path) {
        return Err(NoteError::NotANote);
    }
    if text.len() as u64 > MAX_EDIT {
        return Err(NoteError::TooBig);
    }
    let io = |e: std::io::Error| NoteError::Io(e.to_string());
    let mut tmp = None;
    for n in 0..100u32 {
        let t = dir.join(format!(".note-{}-{n}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&t)
        {
            Ok(f) => {
                tmp = Some((t, f));
                break;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        }
    }
    let Some((t, mut f)) = tmp else {
        return Err(NoteError::Io("no free temporary file name".into()));
    };
    let written = f
        .write_all(text.as_bytes())
        .and_then(|()| f.sync_all())
        .and_then(|()| fs::rename(&t, path));
    if let Err(e) = written {
        let _ = fs::remove_file(&t);
        return Err(io(e));
    }
    // The rename itself is durable once the folder is synced.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Note {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

pub fn notes_dir(home: &Path) -> PathBuf {
    home.join("Notes")
}

/// Validate a user-supplied note name and return the file name to use.
/// A `.txt` extension is added when the name has no `.txt`/`.md` extension.
pub fn file_name_for(name: &str) -> Result<String, NoteError> {
    let name = name.trim_end();
    if name.is_empty() {
        return Err(NoteError::Empty);
    }
    if name.chars().count() > 64 {
        return Err(NoteError::TooLong);
    }
    if name.starts_with('.') || name.starts_with(' ') {
        return Err(NoteError::BadStart);
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err(NoteError::BadCharacter);
    }
    if name.contains("..") {
        return Err(NoteError::BadCharacter);
    }
    Ok(if name.ends_with(".txt") || name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.txt")
    })
}

/// List notes, most recently modified first. A missing directory is empty.
pub fn list(dir: &Path) -> Vec<Note> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut notes: Vec<Note> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            (meta.is_file() && !name.starts_with('.')).then(|| Note {
                name,
                path: e.path(),
                modified: meta.modified().ok(),
                size: meta.len(),
            })
        })
        .collect();
    notes.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    notes
}

/// Create an empty private note (mode 0600) and return its path.
pub fn create(dir: &Path, name: &str) -> Result<PathBuf, NoteError> {
    let file = file_name_for(name)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| NoteError::Io(e.to_string()))?;
    let path = dir.join(file);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(_) => Ok(path),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(NoteError::Exists),
        Err(e) => Err(NoteError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert_eq!(
            file_name_for("shopping list").as_deref(),
            Ok("shopping list.txt")
        );
        assert_eq!(file_name_for("plan.md").as_deref(), Ok("plan.md"));
        assert_eq!(file_name_for("Médecine").as_deref(), Ok("Médecine.txt"));
        assert_eq!(file_name_for(""), Err(NoteError::Empty));
        assert_eq!(file_name_for("   "), Err(NoteError::Empty));
        assert_eq!(file_name_for(".hidden"), Err(NoteError::BadStart));
        assert_eq!(file_name_for("../etc/passwd"), Err(NoteError::BadStart));
        assert_eq!(file_name_for("a/b"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for("a..b"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for("tab\there"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for(&"x".repeat(65)), Err(NoteError::TooLong));
    }

    #[test]
    fn edits_round_trip_and_refuse_what_is_not_a_note() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("resilience-notes-ed-{}", std::process::id()));
        let p = create(&dir, "diary").expect("create");
        save(&dir, &p, "line 1\nligne 2 é\n").expect("save");
        assert_eq!(
            read_for_edit(&dir, &p).as_deref(),
            Ok("line 1\nligne 2 é\n")
        );
        assert_eq!(
            fs::metadata(&p).expect("m").permissions().mode() & 0o777,
            0o600
        );
        // No temporary file is left behind.
        assert_eq!(fs::read_dir(&dir).expect("ls").count(), 1);
        // Too big, not text, outside the folder, hidden, a symbolic link.
        assert_eq!(
            save(&dir, &p, &"x".repeat(MAX_EDIT as usize + 1)),
            Err(NoteError::TooBig)
        );
        fs::write(&p, [0xffu8, 0xfe, 0x00]).expect("w");
        assert_eq!(read_for_edit(&dir, &p), Err(NoteError::NotText));
        let outside = std::env::temp_dir().join("resilience-notes-outside.txt");
        assert_eq!(read_for_edit(&dir, &outside), Err(NoteError::NotANote));
        assert_eq!(save(&dir, &outside, "x"), Err(NoteError::NotANote));
        assert_eq!(
            read_for_edit(&dir, &dir.join(".hidden.txt")),
            Err(NoteError::NotANote)
        );
        let link = dir.join("link.txt");
        std::os::unix::fs::symlink("/etc/hostname", &link).expect("symlink");
        assert!(read_for_edit(&dir, &link).is_err());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn creates_private_notes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("resilience-notes-{}", std::process::id()));
        let p = create(&dir, "first").expect("create");
        assert_eq!(
            fs::metadata(&p).expect("m").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(create(&dir, "first"), Err(NoteError::Exists));
        let l = list(&dir);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, "first.txt");
        fs::remove_dir_all(&dir).expect("cleanup");
        assert!(list(&dir).is_empty());
    }
}
