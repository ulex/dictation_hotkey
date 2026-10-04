//! Native dictation implementation; Windows modules are gated so the core is testable anywhere.
#[cfg(windows)]
pub mod audio;
pub mod bounded;
#[cfg(windows)]
pub mod clipboard;
pub mod config;
pub mod diagnostics;
pub mod hotkey;
#[cfg(windows)]
pub mod logs_ui;
#[cfg(windows)]
pub mod network_handle;
#[cfg(windows)]
pub mod output;
#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod paths;
pub mod protocol;
#[cfg(windows)]
pub mod runtime;
#[cfg(windows)]
pub mod service;
#[cfg(windows)]
pub mod service_ws;
pub mod session;
#[cfg(windows)]
pub mod settings_ui;
pub mod spool;
#[cfg(windows)]
pub mod startup;
pub mod wire;
