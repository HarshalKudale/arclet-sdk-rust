//! Standalone setup program support. Arclet runs the setup binary once when the
//! user clicks Setup, with the collected fields in `ARCLET_SETUP_INPUTS` (JSON).
//! Returning `Ok` → exit 0 → the plugin is marked ready; `Err` → exit 1.

use crate::ctx::input_readers;
use crate::error::{Error, Result};
use crate::log::log;
use serde_json::{json, Value};

pub struct SetupCtx {
    pub inputs: Value,
}

input_readers!(SetupCtx);

impl SetupCtx {
    pub fn log(&self, msg: impl AsRef<str>) {
        log(msg)
    }
    /// Print `message` to stderr (shown by the host) and fail setup.
    pub fn fail<T>(&self, message: impl ToString) -> Result<T> {
        let m = message.to_string();
        eprintln!("{m}");
        Err(Error::Fail(m))
    }
}

/// Flip `setup_required` to false in `manifest.json` (cwd = plugin folder).
fn mark_setup_complete() {
    let _ = crate::set_setup_required(false);
}

/// Run a setup program: `fn main() { arclet_sdk::run_setup(|ctx| { …; Ok(()) }) }`.
/// A no-op under `ARCLET_CODEGEN=1` so the CLI can probe every binary safely.
pub fn run_setup(run: impl FnOnce(&SetupCtx) -> Result) -> ! {
    if std::env::var("ARCLET_CODEGEN").as_deref() == Ok("1") {
        std::process::exit(0);
    }
    let inputs = match std::env::var("ARCLET_SETUP_INPUTS") {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            eprintln!("setup: warning: could not parse ARCLET_SETUP_INPUTS: {e}");
            json!({})
        }),
        Err(_) => json!({}),
    };
    match run(&SetupCtx { inputs }) {
        Ok(()) => {
            mark_setup_complete();
            log("setup: complete");
            std::process::exit(0)
        }
        Err(Error::Fail(_)) => std::process::exit(1),
        Err(Error::Other(m)) => {
            eprintln!("{m}");
            log(format!("setup error: {m}"));
            std::process::exit(1)
        }
    }
}
