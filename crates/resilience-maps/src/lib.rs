//! Offline map viewer of Resilience OS (ADR 0016): the hardened reader for
//! map packs (MBTiles files with vector tiles and our own `rs_` tables).
//!
//! Map files are untrusted input, parsed as the logged-in user in this
//! separate foreground process (never in the TUI):
//!
//! - [`pack`]: opening one file with every SQLite hardening of ADR §7,
//!   tiles, coverage and the capabilities of its `rs_` tables;
//! - [`schema`]: what the reader accepts in a file's schema;
//! - [`metadata`]: validation of the `metadata` table;
//! - [`profile`]: the exact schema of profile `resilience-1` and its blob
//!   layouts, shared with the pack builder;
//! - [`inflate`]: tile decompression with hard limits;
//! - [`rlimit`]: the process's address-space limit;
//! - [`info`]: the `info` command's output;
//! - [`text`]: cleaning untrusted strings before they reach a terminal;
//! - [`mapmaker`]: the pack builder `resilience-mapmaker` (build machines
//!   only).
//!
//! The viewer (`view`, ADR §9 item 6):
//!
//! - [`args`]: its command line;
//! - [`app`]: the state machine `MapApp` (keys, reader answers, status);
//! - [`ui`]: drawing its screens with ratatui;
//! - [`render`]: packs and a view to a canvas (pure);
//! - [`area`]: coverage areas and point-in-polygon;
//! - [`loader`]: the reader thread (opening, checks, tiles) and which
//!   files count as checked;
//! - [`cache`]: the bounded cache of decoded tiles;
//! - [`position`]: the last position file.

pub mod app;
pub mod area;
pub mod args;
pub mod cache;
pub mod inflate;
pub mod info;
pub mod loader;
pub mod mapmaker;
pub mod metadata;
pub mod pack;
pub mod position;
pub mod profile;
pub mod render;
pub mod rlimit;
pub mod scene;
pub mod schema;
pub mod text;
pub mod ui;

// The test fixtures name this crate `resilience_maps`, so that
// `tests/cli.rs` can include the same file.
#[cfg(test)]
extern crate self as resilience_maps;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod view_tests;
