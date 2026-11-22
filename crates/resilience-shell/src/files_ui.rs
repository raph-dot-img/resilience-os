//! "Find & view files": search file names in the home folder and on open
//! USB sticks, and show a file inside the interface. Read-only: nothing
//! from a file is ever run (see `resilience_core::files`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::list::ListState;
use resilience_core::files::{self, Kind, MAX_TEXT};
use resilience_core::human_bytes;
use resilience_core::proc::run_full;

use super::{App, Effect, Level, Modal, View};

/// A CSV or TSV file shown as a table. Column widths are computed once.
#[derive(Debug)]
pub struct Table {
    pub title: String,
    pub rows: Vec<Vec<String>>,
    /// Width of each column: its widest cell in the first 500 rows, 3 to 30.
    pub widths: Vec<usize>,
    /// First row shown (after the header) and first column shown.
    pub row: usize,
    pub col: usize,
}

impl Table {
    pub fn new(title: &str, rows: Vec<Vec<String>>) -> Table {
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![3usize; cols];
        for r in rows.iter().take(500) {
            for (c, cell) in r.iter().enumerate() {
                widths[c] = widths[c].max(cell.chars().count().min(30));
            }
        }
        Table {
            title: title.to_string(),
            rows,
            widths,
            row: 0,
            col: 0,
        }
    }
}

pub const MAX_TABLE_ROWS: usize = 20_000;

impl App {
    /// Places to search: the home folder and every open USB stick.
    pub(super) fn file_roots(&self) -> Vec<(PathBuf, String)> {
        let mut roots = vec![(self.home.clone(), "~".to_string())];
        let (volumes, _) = self.load_volumes();
        for v in volumes.iter().filter(|v| v.usable()) {
            if let Some(m) = &v.mountpoint {
                roots.push((m.clone(), v.display_name()));
            }
        }
        roots
    }

    pub(super) fn open_files(&mut self) -> View {
        let idx = files::index(&self.file_roots());
        if idx.truncated {
            self.notify(
                Level::Warn,
                format!("Only the first {} files are searched.", files::MAX_ENTRIES),
            );
        }
        View::Files {
            idx: Box::new(idx),
            query: String::new(),
            list: ListState::default(),
            pick: None,
        }
    }

    /// Show a file: a view to push, or an effect (packs open in the library).
    pub(super) fn view_file(&mut self, path: &Path, shown: &str) -> (Option<View>, Effect) {
        let head = files::head(path);
        match files::kind(path, &head) {
            Kind::Text => match files::read_text(path) {
                Ok(text) => (Some(pager(shown, text)), Effect::None),
                Err(e) => {
                    self.file_error(shown, &e.to_string());
                    (None, Effect::None)
                }
            },
            Kind::Table { separator } => match files::read_text(path) {
                Ok(text) => {
                    let rows = files::parse_table(&text, separator, MAX_TABLE_ROWS);
                    (
                        Some(View::Table(Box::new(Table::new(shown, rows)))),
                        Effect::None,
                    )
                }
                Err(e) => {
                    self.file_error(shown, &e.to_string());
                    (None, Effect::None)
                }
            },
            Kind::Pdf => {
                let p = path.to_string_lossy();
                // At most 500 pages; the text is cut like any text file.
                let out = run_full(
                    &[
                        &self.sys.tools.pdftotext,
                        "-layout",
                        "-enc",
                        "UTF-8",
                        "-l",
                        "500",
                        &p,
                        "-",
                    ],
                    Duration::from_secs(60),
                );
                match out.filter(|o| o.success()) {
                    Some(o) if !o.stdout.trim().is_empty() => {
                        (Some(pager(shown, clean(&o.stdout))), Effect::None)
                    }
                    Some(_) => {
                        self.file_error(
                            shown,
                            "this PDF contains no text (it may be scanned pictures)",
                        );
                        (None, Effect::None)
                    }
                    None => {
                        self.file_error(shown, "the PDF could not be read (damaged or protected?)");
                        (None, Effect::None)
                    }
                }
            }
            Kind::Archive => {
                let p = path.to_string_lossy();
                match run_full(
                    &[&self.sys.tools.bsdtar, "-tvf", &p],
                    Duration::from_secs(30),
                )
                .filter(|o| o.success())
                {
                    Some(o) => {
                        let text = format!(
                            "Contents of the archive (nothing is extracted):\n\n{}",
                            clean(&o.stdout)
                        );
                        (Some(pager(shown, text)), Effect::None)
                    }
                    None => {
                        self.file_error(shown, "the archive could not be listed");
                        (None, Effect::None)
                    }
                }
            }
            Kind::Zim => {
                let item = super::LibItem {
                    title: shown.to_string(),
                    description: String::new(),
                    path: path.to_path_buf(),
                    size: 0,
                    category: None,
                    language: None,
                    place: super::library::ItemPlace::Other,
                };
                (None, self.open_pack_effect(&item, None))
            }
            Kind::Other => {
                self.file_info(path, shown, false);
                (None, Effect::None)
            }
        }
    }

    /// F3: details, with the SHA-256 computed on the spot for files up to
    /// 256 MiB; bigger files are hashed in the foreground with progress.
    pub(super) fn file_details(&mut self, path: &Path, shown: &str) -> Effect {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        if size <= 256 << 20 {
            self.file_info(path, shown, true);
            return Effect::None;
        }
        Effect::Run(super::RunRequest {
            label: "Checksum".into(),
            argv: vec![
                self.sys.tools.sha256sum.clone(),
                "--".into(),
                path.to_string_lossy().into_owned(),
            ],
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "SHA-256 of {} ({}). This reads the whole file; Ctrl-C stops.",
                resilience_core::printable(shown),
                human_bytes(size)
            )),
        })
    }

    /// Size, date and (if `with_sha`) SHA-256 of a file.
    fn file_info(&mut self, path: &Path, shown: &str, with_sha: bool) {
        let meta = std::fs::metadata(path);
        let size = meta.as_ref().map_or(0, |m| m.len());
        let modified = meta
            .ok()
            .and_then(|m| m.modified().ok())
            .map(crate::fmt::date_time)
            .unwrap_or_else(|| "?".into());
        let p = path.to_string_lossy();
        let sha = if with_sha {
            run_full(
                &[&self.sys.tools.sha256sum, "--", &p],
                Duration::from_secs(60),
            )
            .filter(|o| o.success())
            .and_then(|o| o.stdout.split_whitespace().next().map(String::from))
            .unwrap_or_else(|| "could not be computed".into())
        } else {
            "press F3 in the list to compute it".into()
        };
        self.modal = Some(Modal::Message {
            title: "FILE DETAILS".into(),
            body: format!(
                "{shown}\n\nSize: {}\nModified: {modified}\nSHA-256: {sha}\n\nCompare the \
                 SHA-256 with the one published by the file's source to check that it is \
                 intact.",
                human_bytes(size)
            ),
            error: false,
        });
    }

    fn file_error(&mut self, shown: &str, why: &str) {
        self.modal = Some(Modal::Message {
            title: "CANNOT SHOW THIS FILE".into(),
            body: format!("{shown}: {why}."),
            error: true,
        });
    }
}

fn pager(title: &str, text: String) -> View {
    View::Pager {
        title: title.to_string(),
        text,
        scroll: 0,
    }
}

/// Text from another program: capped, without control characters.
fn clean(s: &str) -> String {
    let mut t: String = s
        .chars()
        .take(MAX_TEXT as usize)
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    if s.chars().count() > MAX_TEXT as usize {
        t.push_str("\n\n[The rest is not shown.]");
    }
    t
}
