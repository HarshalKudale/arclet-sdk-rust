# knobdeck-sdk (Rust)

Declarative Rust SDK for authoring [KnobDeck](https://github.com/HarshalKudale/knobdeck) plugins — the
Rust counterpart of [`@knobdeck/sdk`](../knobdeck-sdk). Same wire protocol, same action model, same
generated `manifest.json` / `actions.json`; a plugin is a native executable.

## Quick start

```toml
# Cargo.toml of your plugin crate
[dependencies]
knobdeck-sdk = { path = "../../knobdeck-sdk-rust" }
```

```rust
use knobdeck_sdk::{Action, Plugin};

fn main() {
    Plugin::new("demo", "Demo")
        .version("1.0.0")
        .action(Action::button("ping", "Ping").on_press(|ctx| {
            ctx.log("pong");
            Ok(())
        }))
        .run()
}
```

Install the CLI once, then run it from the plugin crate folder:

```bash
cargo install --path ../knobdeck-sdk-rust --bin knobdeck-plugin
knobdeck-plugin dev       # debug build -> dist/<id>/ (use "Load Unpacked" in KnobDeck)
knobdeck-plugin dev --watch  # rebuild on change; KnobDeck restarts the plugin
knobdeck-plugin build     # release build -> self-contained dist/<id>/
knobdeck-plugin package   # dist/<id>.knobdeck
```

## Action types

| Builder | Handlers | Notes |
|---|---|---|
| `Action::button(id, label)` | `on_press` | |
| `Action::toggle(id, label)` | `on_press` | `.state(value, label, icon)` ×N; first is the default; `.initial(..)` / `.initial_with(..)` override |
| `Action::hold(id, label)` | `on_hold_start`, `on_hold_end` | `.released(label, icon)`, `.held(label, icon)` |
| `Action::slider(id, label, min, max)` | `on_adjust` | `ctx.value` (absolute) or `ctx.delta`; `.step()`, `.unit()` |
| `Action::wheel(id, label)` | `on_open` → `Vec<slice>`, `on_child_press` | max 16 buttons (extra are truncated) |

`.volatile()` re-computes the initial state on every sync / focus change. `.icon()` and `.inputs()` apply to all.

`.live(every, |ic| Ok(Visual::new().label(..).icon(..)))` gives any action Stream Deck-style live content: it runs every `every` for each placed instance (`ic.input_str(..)` reads its inputs) and pushes the label/icon when they change. The action and its press handler stay as they are, and while the plugin is not running the tile shows its configured icon and label.

An icon is a [Lucide](https://lucide.dev/icons) name (`"play"`), an image file in the plugin's `icons/` folder (`"mute.svg"`), or a `data:` URI. Emoji and `icon:` pack refs are not rendered by the host.

## Handlers and context

Handlers are `Fn(&Ctx) -> knobdeck_sdk::Result`. `?` works on any `std::error::Error`; an error that
escapes a handler is logged and sent to the host as an error frame. Invokes for the same instance
run in order on that instance's worker thread (after the host ack is sent); different instances,
queries and events run in parallel, so blocking I/O in a handler is fine.

```rust
ctx.set_state("on");            // this instance        ctx.fan_out("on");  // every instance
ctx.set_label("CPU 20%");       // live visuals          ctx.set_visual(Visual::new().label("x").icon("y"));
ctx.set_error("msg");           // report, keep going    return ctx.fail("msg");  // report + stop
ctx.input_str("url"); ctx.input_bool("on"); ctx.input_f64("n");
ctx.state; ctx.fg_process; ctx.fg_title; ctx.trigger; ctx.child_id; ctx.value; ctx.delta;
ctx.send_keys("ctrl+m")?; ctx.focus_app("code.exe")?; ctx.installed_apps()?;
ctx.mark_setup_required("Discord token expired");   // host stops the plugin, asks for setup
ctx.data_dir();                 // plugin-data/<id> — survives updates (see Data and logs)
```

## Windows and macOS

The host runs on Windows and macOS and spawns the binary built for its OS.

```rust
Plugin::new("mac-tools", "Mac Tools").platforms(&["macos"])        // whole plugin: one OS
Action::button("lock", "Lock").platforms(&["windows"])              // one action: one OS
knobdeck_sdk::platform();  knobdeck_sdk::is_mac();                      // "windows" | "macos"
ctx.send_keys("mod t")?;   // `mod` is ⌘ on macOS and Ctrl on Windows; `cmd`/`win` is ⌘ / Win
```

Without `platforms` a plugin or action is offered on both OSes. Elsewhere the host hides it from
pickers, and won't start a plugin whose `platforms` exclude its OS. `fg_process` is the exe name on
Windows (`code.exe`) and the bundle id on macOS (`com.microsoft.VSCode`). `focus_app` takes an exe
name/path on Windows and a bundle id, `.app` path or app name on macOS.

`build` writes `runtime.executables` (and `setup.executables`) keyed by OS. Building on Windows
and then on macOS into the same `dist/<id>/` keeps both entries and both binaries, so
`package` produces one `.knobdeck` that runs on both.

**Keystrokes are a last resort.** `send_keys` is for actions that *are* typing, or when no OS API,
app interface (local API, scripting, Accessibility menu press) or system app exists. Shortcuts can
be captured by the app in focus, remapped or turned off by the user. If you must use one, prefer
the user's configured system binding and comment why no API fits.

## Inputs, queries

```rust
use knobdeck_sdk::input::{self, InputExt};
Action::button("open", "Open").inputs(vec![
    input::text("url", "URL").placeholder("https://…"),
    input::checkbox("new_tab", "New tab").default_value(true),
    input::select("mode", "Mode", &[("a", "A"), ("b", "B")]),
    input::query_select("scene", "Scene", "scenes", &[]),   // live options from a query
])
// Any raw `json!({...})` descriptor works too (showIf, etc.).

Plugin::new(..).query("scenes", |ctx| Ok(json!([{"value": "1", "label": "Main"}])))
```

## Background work

`on_ready` fires once after the first sync with a cloneable, `Send` [`Runtime`]; move it into your
own threads to push `set_state` / `notify` / `notify_visual` / `set_error`. `subscribe(&["focus_changed"])`
+ `on_event` receive host events. `on_shutdown` runs when the host stops the plugin (stdin closes
or `shutdown` arrives); the host kills it 2 s later.

## Data and logs

Keep tokens and settings in `knobdeck_sdk::data_dir()` (`%APPDATA%\KnobDeck\plugin-data\<id>` on
Windows, `~/Library/Application Support/KnobDeck/plugin-data/<id>` on macOS) (or `knobdeck_sdk::data_file("token.json")`,
which also moves a file left in the plugin folder by an older version). The folder survives
updates and reinstalls. `log()` and anything printed (even `println!`) go to the plugin's
`log.txt` in that folder; stdout is reserved for the protocol.

## Setup program

A second Cargo `[[bin]]` that calls `run_setup`; declare it on the plugin with `.setup(bin, page)`:

```rust
fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let host = ctx.input_str("host");
        if host.is_empty() { return ctx.fail("Host cannot be empty"); }
        std::fs::write(ctx.data_dir().join(".env"), format!("HOST={host}\n"))?;
        Ok(())
    })
}
```

At runtime the plugin calls `knobdeck_sdk::load_env()` to read that `.env`. Declare collected fields with
`.setup_inputs(vec![..])` on the plugin and pass `None` as the page: the host draws the form.
`knobdeck_sdk::open_url(url)` opens a link in the default browser.

## Signing in to a web service (feature `oauth`)

`knobdeck-sdk = { ..., features = ["oauth"] }` adds `knobdeck_sdk::oauth::OAuth`:

- `authorize_pkce(auth_url, scopes, port, extra)`: browser sign-in with PKCE, answered on
  `http://127.0.0.1:{port}/callback` (register that redirect URI). No client secret is shipped.
- `device_flow(device_url, form)`: for services without PKCE for desktop apps (Twitch); opens
  the activation page and waits for approval.
- `access_token()`: the saved token, refreshed when it is about to expire. A refused refresh calls
  `mark_setup_required`, so the host asks the user to sign in again.

Run the sign-in in the setup program; the token is kept as `oauth-token.json` in the data folder.

## Differences from the JS SDK

- No bundling step: `build` ships the release binaries in `dist/<id>/`; the manifest has no `runtime.language`, so the host runs it as a native executable.
- `.env` is read at runtime (`load_env()`), not inlined at build time.
- `category` is not part of the generated `actions.json` in the current JS SDK, so it is omitted here too.

## Layout

`src/runtime.rs` (stdio protocol v2), `src/plugin.rs` (builders), `src/codegen.rs` (manifest/actions),
`src/bin/knobdeck-plugin.rs` (CLI), `tests/protocol.rs` (drives the demo over pipes).

## Protocol v2

One JSON object per line. Host → plugin on stdin: `hello` (first), `sync`, `invoke`, `request`
(`query` / `wheel`), `event`, `result` (answer to `host_call`), `shutdown`. Plugin → host on stdout:
`ready` (first), `ack`, `state`, `error`, `result` (answer to `request`), `host_call`, `subscribe`,
`setup_required`. Unknown types are ignored. The manifest declares `"protocol": 2`.
