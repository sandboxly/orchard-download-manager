//! Orchard Download Manager: segmented, resumable downloads in a themeable
//! character-grid window (Orchard Commander's `oc-grid`).

pub mod category;
pub mod checksum;
pub mod clipboard;
pub mod download;
pub mod engine;
pub mod format;
pub mod icons;
pub mod input;
pub mod ipc;
pub mod links;
pub mod native_host;
pub mod settings;
pub mod system;
pub mod tasks;
pub mod theme;
pub mod app;
pub mod ui;

pub use app::App;
