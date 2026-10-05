//! Shared dictation core with native Windows and macOS platform adapters.
#[cfg(windows)]
pub mod audio;
pub mod bounded;
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
#[cfg(any(windows, target_os = "macos"))]
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

#[cfg(target_os = "macos")]
#[path = "platform/macos/audio.rs"]
pub mod audio;

#[cfg(target_os = "macos")]
#[path = "platform/macos/paths.rs"]
pub mod paths;

#[cfg(target_os = "macos")]
#[path = "platform/macos/service.rs"]
pub mod service;

#[cfg(target_os = "macos")]
#[path = "platform/macos/service_ws.rs"]
pub mod service_ws;

#[cfg(target_os = "macos")]
#[path = "platform/macos/app.rs"]
pub mod macos_app;
#[cfg(target_os = "macos")]
#[path = "platform/macos/bridge.rs"]
mod macos_bridge;

#[cfg(any(windows, target_os = "macos", test))]
mod realtime;
