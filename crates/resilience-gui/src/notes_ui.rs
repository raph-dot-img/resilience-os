//! The built-in note editor of the graphical edition (ADR 0026 amendment
//! 1): plain text, iced's multi-line editor (cursor, selection, clipboard,
//! undo-free on purpose: simple). Ctrl+S saves; Esc or Close asks before
//! throwing changes away (Cancel is the default). Notes are read and saved
//! only through `resilience_core::notes` (size bound, UTF-8, private file
//! + rename).

use std::path::{Path, PathBuf};

use iced::keyboard::{self, key::Named};
use iced::widget::text_editor::{self, Binding, Content, KeyPress};
use iced::widget::{Space, button, column, container, row, text, text_editor as editor};
use iced::{Element, Fill, Theme};
use resilience_core::notes;

use crate::style::{self, Palette};

/// The editor widget's id (focused when a note opens).
pub const EDITOR: &str = "note-editor";

#[derive(Debug, Clone)]
pub enum NoteMsg {
    Edit(text_editor::Action),
    Save,
    /// Close: asks first when there are unsaved changes.
    Close,
    /// In the "unsaved changes" question.
    SaveAndClose,
    Discard,
    KeepEditing,
}

pub struct Editor {
    pub dir: PathBuf,
    pub path: PathBuf,
    pub name: String,
    pub content: Content,
    /// The text as last read or saved.
    saved: String,
    pub error: Option<String>,
    /// The "unsaved changes" question is open.
    pub asking: bool,
}

/// What the GUI does after a message.
#[derive(Debug, PartialEq, Eq)]
pub enum After {
    Stay,
    /// Close the editor; `saved` when the note was written.
    Close {
        saved: bool,
    },
}

impl Editor {
    pub fn open(dir: &Path, path: &Path) -> Result<Editor, String> {
        let text = notes::read_for_edit(dir, path).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Editor {
            dir: dir.to_path_buf(),
            path: path.to_path_buf(),
            name,
            content: Content::with_text(&text),
            saved: text,
            error: None,
            asking: false,
        })
    }

    /// The editor's text as it will be saved.
    fn text(&self) -> String {
        self.content.text()
    }

    pub fn dirty(&self) -> bool {
        self.text() != self.saved
    }

    fn save(&mut self) -> bool {
        let t = self.text();
        match notes::save(&self.dir, &self.path, &t) {
            Ok(()) => {
                self.saved = t;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("Not saved: {e}"));
                false
            }
        }
    }

    pub fn update(&mut self, m: NoteMsg) -> After {
        match m {
            NoteMsg::Edit(a) => {
                if !self.asking {
                    self.content.perform(a);
                }
                After::Stay
            }
            NoteMsg::Save => {
                if !self.asking {
                    self.save();
                }
                After::Stay
            }
            NoteMsg::Close => {
                if self.dirty() {
                    self.asking = true;
                    After::Stay
                } else {
                    After::Close { saved: false }
                }
            }
            NoteMsg::SaveAndClose => {
                if self.save() {
                    After::Close { saved: true }
                } else {
                    self.asking = false;
                    After::Stay
                }
            }
            NoteMsg::Discard => After::Close { saved: false },
            NoteMsg::KeepEditing => {
                self.asking = false;
                After::Stay
            }
        }
    }

    pub fn view<'a, M: Clone + 'a>(
        &'a self,
        p: &Palette,
        wrap: impl Fn(NoteMsg) -> M + Copy + 'a,
    ) -> Element<'a, M> {
        let p = *p;
        let title = row![
            text(format!("Note: {}", self.name))
                .size(20)
                .font(style::UI_FONT_BOLD),
            text(if self.dirty() { "  (not saved)" } else { "" })
                .size(16)
                .style(style::dim_text),
            Space::new().width(Fill),
            button(text("Save  Ctrl+S").size(15))
                .on_press(wrap(NoteMsg::Save))
                .style(move |t: &Theme, s| style::dialog_button(t, s, true, false, false)),
            button(text("Close  Esc").size(15))
                .on_press(wrap(NoteMsg::Close))
                .style(move |t: &Theme, s| style::dialog_button(t, s, false, false, false)),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let body = editor(&self.content)
            .id(iced::widget::Id::new(EDITOR))
            .on_action(move |a| wrap(NoteMsg::Edit(a)))
            .key_binding(move |k: KeyPress| bindings(k, wrap))
            .font(style::MONO_FONT)
            .size(16)
            .height(Fill)
            .padding(14);
        let mut col = column![title].spacing(14).padding([20, 28]);
        if let Some(e) = &self.error {
            col = col.push(text(e.clone()).size(15).color(p.error));
        }
        if self.asking {
            col = col.push(
                container(
                    row![
                        text("This note has changes that are not saved.").size(16),
                        Space::new().width(Fill),
                        button(text("Keep editing  Enter").size(15))
                            .on_press(wrap(NoteMsg::KeepEditing))
                            .style(move |t: &Theme, s| style::dialog_button(
                                t, s, true, false, true
                            )),
                        button(text("Save and close  S").size(15))
                            .on_press(wrap(NoteMsg::SaveAndClose))
                            .style(move |t: &Theme, s| {
                                style::dialog_button(t, s, false, false, false)
                            }),
                        button(text("Close without saving  D").size(15))
                            .on_press(wrap(NoteMsg::Discard))
                            .style(move |t: &Theme, s| style::dialog_button(
                                t, s, false, true, false
                            )),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                )
                .padding(14)
                .style(move |_| style::card(&p)),
            );
        }
        col = col.push(body);
        col.into()
    }
}

/// Keys of the "unsaved changes" question: Enter and Esc keep editing
/// (the safe default), S saves and closes, D closes without saving.
pub fn answer_key(k: resilience_shell::input::KeyCode) -> Option<NoteMsg> {
    use resilience_shell::input::KeyCode;
    match k {
        KeyCode::Enter | KeyCode::Esc => Some(NoteMsg::KeepEditing),
        KeyCode::Char('s' | 'S') => Some(NoteMsg::SaveAndClose),
        KeyCode::Char('d' | 'D') => Some(NoteMsg::Discard),
        _ => None,
    }
}

/// Ctrl+S saves and Esc closes; every other key edits as usual. While the
/// question is open, Esc means "keep editing" (the safe choice).
fn bindings<M>(k: KeyPress, wrap: impl Fn(NoteMsg) -> M) -> Option<Binding<M>> {
    match k.key.as_ref() {
        keyboard::Key::Character("s") if k.modifiers.command() => {
            Some(Binding::Custom(wrap(NoteMsg::Save)))
        }
        keyboard::Key::Named(Named::Escape) => Some(Binding::Custom(wrap(NoteMsg::Close))),
        _ => Binding::from_key_press(k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "resilience-gui-notes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn edit_save_and_close_flow() {
        let dir = temp();
        let p = notes::create(&dir, "plan").expect("create");
        std::fs::write(&p, "one\n").expect("w");
        let mut e = Editor::open(&dir, &p).expect("open");
        assert!(!e.dirty());
        // Closing an unchanged note closes at once.
        assert_eq!(e.update(NoteMsg::Close), After::Close { saved: false });
        // A change: Close asks; Keep editing goes back; Save and close writes.
        e.content
            .perform(text_editor::Action::Edit(text_editor::Edit::Insert('x')));
        assert!(e.dirty());
        assert_eq!(e.update(NoteMsg::Close), After::Stay);
        assert!(e.asking);
        // Typing is ignored while the question is open.
        e.update(NoteMsg::Edit(text_editor::Action::Edit(
            text_editor::Edit::Insert('y'),
        )));
        assert_eq!(e.update(NoteMsg::KeepEditing), After::Stay);
        assert!(!e.asking);
        assert_eq!(e.update(NoteMsg::Close), After::Stay);
        assert_eq!(
            e.update(NoteMsg::SaveAndClose),
            After::Close { saved: true }
        );
        assert_eq!(std::fs::read_to_string(&p).expect("r"), "xone\n");
        std::fs::remove_dir_all(&dir).expect("rm");
    }

    #[test]
    fn a_file_without_final_newline_keeps_none() {
        let dir = temp();
        let p = notes::create(&dir, "short").expect("create");
        std::fs::write(&p, "no newline").expect("w");
        let mut e = Editor::open(&dir, &p).expect("open");
        assert!(!e.dirty(), "{:?}", e.content.text());
        e.update(NoteMsg::Save);
        assert_eq!(std::fs::read_to_string(&p).expect("r"), "no newline");
        std::fs::remove_dir_all(&dir).expect("rm");
    }

    #[test]
    fn refuses_what_is_not_a_note() {
        let dir = temp();
        let outside = std::env::temp_dir().join("resilience-gui-not-a-note.txt");
        assert!(Editor::open(&dir, &outside).is_err());
        std::fs::remove_dir_all(&dir).expect("rm");
    }
}
