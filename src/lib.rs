//! # arclet-sdk
//!
//! Declarative Rust SDK for authoring [Arclet](https://github.com/HarshalKudale/Arclet)
//! plugins — the Rust counterpart of `@arclet/sdk`. A plugin is a native
//! executable: declare actions with [`Plugin`] / [`Action`], call `.run()`, and
//! let the `arclet-plugin` CLI generate `manifest.json` / `actions.json`.
//!
//! The host spawns the plugin and talks to it over stdio (protocol v2). Handlers
//! are plain synchronous closures: invokes run in order per instance, queries
//! and events in parallel, so a slow handler never blocks the connection; the
//! host ack is sent before the handler starts.

mod codegen;
mod ctx;
mod error;
mod host;
pub mod input;
mod log;
mod plugin;
mod runtime;
mod setup;

pub use codegen::{make_full_id, Stage};
pub use ctx::{Ctx, Event, InitialCtx, OpenCtx, QueryCtx};
pub use error::{Error, Result};
pub use host::{focus_app, installed_apps, send_keys};
pub use log::log;
pub use plugin::{Action, Kind, Plugin};
pub use runtime::{Runtime, Visual, PROTOCOL};
pub use serde_json::{json, Value};
pub use setup::{run_setup, SetupCtx};

use std::path::{Path, PathBuf};

/// Tell the host this plugin needs setup again (expired token, revoked auth…).
/// Inside a handler prefer `ctx.mark_setup_required(reason)`.
pub fn mark_setup_required(reason: impl ToString) {
    match runtime::global() {
        Some(rt) => rt.mark_setup_required(reason),
        None => log("mark_setup_required: not connected to the host"),
    }
}

/// This plugin's persistent data folder (`%APPDATA%\Arclet\plugin-data\<id>`):
/// survives updates and reinstalls. Works in the plugin (from the host's
/// `hello`) and in setup programs (`ARCLET_PLUGIN_DATA`); `.` otherwise.
pub fn data_dir() -> PathBuf {
    if let Some(rt) = runtime::global() {
        return rt.data_dir().to_path_buf();
    }
    std::env::var_os("ARCLET_PLUGIN_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `data_dir()/name`. If the file still sits in the plugin folder (where
/// pre-0.3 plugins kept it) and not in the data dir yet, it is moved over once.
pub fn data_file(name: &str) -> PathBuf {
    let target = data_dir().join(name);
    let legacy = Path::new(name);
    if !target.exists() && legacy.is_file() && legacy.canonicalize().ok() != target.canonicalize().ok() {
        if std::fs::rename(legacy, &target).is_err() {
            // Different volume: copy, then drop the old copy.
            if std::fs::copy(legacy, &target).is_ok() {
                let _ = std::fs::remove_file(legacy);
            }
        }
        log(format!("moved {name} into the plugin data folder"));
    }
    target
}

/// Load `KEY=VALUE` lines from `.env` in the plugin's data folder (moved there
/// from the plugin folder on first run) into the process environment. Existing
/// variables win. Missing file is fine.
pub fn load_env() {
    let Ok(text) = std::fs::read_to_string(data_file(".env")) else { return };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let v = v
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        if std::env::var_os(k.trim()).is_none() {
            std::env::set_var(k.trim(), v);
        }
    }
}

/// `env_or("VLC_PORT", "8080")` — env var with a default (empty counts as unset).
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into())
}
