//! # arclet-sdk
//!
//! Declarative Rust SDK for authoring [Arclet](https://github.com/HarshalKudale/Arclet)
//! plugins — the Rust counterpart of `@arclet/sdk`. A plugin is a native
//! executable: declare actions with [`Plugin`] / [`Action`], call `.run()`, and
//! let the `arclet-plugin` CLI generate `manifest.json` / `actions.json`.
//!
//! Handlers are plain synchronous closures. Each invoke / query / event runs on
//! its own thread, so a slow handler never blocks the connection; the host ack
//! is sent before the handler starts.

mod codegen;
mod ctx;
mod error;
mod host;
pub mod input;
mod log;
mod plugin;
mod runtime;
mod setup;
mod ws;

pub use codegen::{make_full_id, Stage};
pub use ctx::{Ctx, Event, InitialCtx, OpenCtx, QueryCtx};
pub use error::{Error, Result};
pub use host::{focus_app, installed_apps, send_keys};
pub use log::log;
pub use plugin::{Action, Kind, Plugin};
pub use runtime::{Runtime, Visual};
pub use serde_json::{json, Value};
pub use setup::{run_setup, SetupCtx};

use std::path::PathBuf;

fn manifest_path() -> PathBuf {
    PathBuf::from("manifest.json")
}

pub(crate) fn set_setup_required(required: bool) -> std::io::Result<()> {
    let path = manifest_path();
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    if manifest["setup_required"].as_bool() == Some(required) {
        return Ok(());
    }
    manifest["setup_required"] = required.into();
    std::fs::write(&path, serde_json::to_string_pretty(&manifest)? + "\n")
}

/// Mark the plugin as requiring setup again (expired token, changed password…)
/// by writing `setup_required: true` to `manifest.json` in the plugin folder.
/// Inside a handler prefer `ctx.mark_setup_required()`, which also tells the
/// host over the live connection.
pub fn mark_setup_required() {
    if let Err(e) = set_setup_required(true) {
        log(format!("mark_setup_required: could not update manifest.json: {e}"));
    }
}

/// Load `KEY=VALUE` lines from `.env` in the plugin folder (cwd) into the
/// process environment. Existing variables win. Missing file is fine.
pub fn load_env() {
    let Ok(text) = std::fs::read_to_string(".env") else { return };
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
