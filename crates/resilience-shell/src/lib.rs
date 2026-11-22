//! Interface logic shared by the Resilience front ends (ADR 0026).
//!
//! The state machine (`app::App::handle_key` returning `Effect`s), its
//! screens' state and the help texts live here, with no terminal or
//! graphics code. `resilience-tui` and `resilience-gui` render it and
//! translate their input into [`input::KeyEvent`]s.

pub mod app;
pub mod fmt;
pub mod help;
pub mod input;
pub mod list;
