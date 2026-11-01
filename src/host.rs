//! Host-owned capabilities. The desktop app performs these on the plugin's
//! behalf over the live WebSocket (`host_call` / `host_result`).

use crate::error::{Error, Result};
use crate::log::log;
use crate::runtime::global;
use serde_json::{json, Value};

fn call(method: &str, params: Value) -> Result<Value> {
    global()
        .ok_or_else(|| Error::msg("KnobDeck host bridge is not connected"))?
        .host_call(method, params)
}

/// Ask the host to press a key combination (e.g. `"ctrl+shift+m"`).
pub fn send_keys(keys: &str) -> Result {
    let keys = keys.trim();
    if keys.is_empty() {
        return Err(Error::msg("send_keys requires a non-empty key string"));
    }
    log(format!("host.send_keys({keys})"));
    call("send_keys", json!({"keys": keys})).map(|_| ())
}

/// Ask the host to focus an app by executable path or exe name. Returns whether it was focused.
pub fn focus_app(target: &str) -> Result<bool> {
    let target = target.trim();
    if target.is_empty() {
        return Err(Error::msg("focus_app requires a non-empty executable path or exe name"));
    }
    log(format!("host.focus_app({target})"));
    Ok(call("focus_app", json!({"path": target}))?["focused"].as_bool().unwrap_or(false))
}

/// List the apps the host knows are installed.
pub fn installed_apps() -> Result<Vec<Value>> {
    log("host.installed_apps()");
    Ok(call("list_installed_apps", json!({}))?.as_array().cloned().unwrap_or_default())
}
