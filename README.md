# arclet-sdk (Rust)

Declarative Rust SDK for authoring [Arclet](https://github.com/HarshalKudale/Arclet) plugins — the
Rust counterpart of [`@arclet/sdk`](../arclet-sdk). Same wire protocol, same action model, same
generated `manifest.json` / `actions.json`; a plugin is a native executable.

## Quick start

```toml
# Cargo.toml of your plugin crate
[dependencies]
arclet-sdk = { path = "../../arclet-sdk-rust" }
```

```rust
use arclet_sdk::{Action, Plugin};

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
cargo install --path ../arclet-sdk-rust --bin arclet-plugin
arclet-plugin dev       # debug build -> dist/<id>/ (use "Load Unpacked" in Arclet)
arclet-plugin dev --watch  # rebuild on change; Arclet restarts the plugin
arclet-plugin build     # release build -> self-contained dist/<id>/
arclet-plugin package   # dist/<id>.arclet
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

Handlers are `Fn(&Ctx) -> arclet_sdk::Result`. `?` works on any `std::error::Error`; an error that
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
ctx.data_dir();                 // %APPDATA%\Arclet\plugin-data\<id> — survives updates
```

## Inputs, queries

```rust
use arclet_sdk::input::{self, InputExt};
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

Keep tokens and settings in `arclet_sdk::data_dir()` (or `arclet_sdk::data_file("token.json")`,
which also moves a file left in the plugin folder by an older version). The folder survives
updates and reinstalls. `log()` and anything printed (even `println!`) go to the plugin's
`log.txt` in that folder; stdout is reserved for the protocol.

## Setup program

A second Cargo `[[bin]]` that calls `run_setup`; declare it on the plugin with `.setup(bin, page)`:

```rust
fn main() {
    arclet_sdk::run_setup(|ctx| {
        let host = ctx.input_str("host");
        if host.is_empty() { return ctx.fail("Host cannot be empty"); }
        std::fs::write(ctx.data_dir().join(".env"), format!("HOST={host}\n"))?;
        Ok(())
    })
}
```

At runtime the plugin calls `arclet_sdk::load_env()` to read that `.env`. Declare collected fields with
`.setup_inputs(vec![..])` on the plugin.

## Differences from the JS SDK

- No bundling step: `build` ships the release exe(s) in `dist/<id>/`; the manifest has no `runtime.language`, so the host runs it as a native executable.
- `.env` is read at runtime (`load_env()`), not inlined at build time.
- `category` is not part of the generated `actions.json` in the current JS SDK, so it is omitted here too.

## Layout

`src/runtime.rs` (stdio protocol v2), `src/plugin.rs` (builders), `src/codegen.rs` (manifest/actions),
`src/bin/arclet-plugin.rs` (CLI), `tests/protocol.rs` (drives the demo over pipes).

## Protocol v2

One JSON object per line. Host → plugin on stdin: `hello` (first), `sync`, `invoke`, `request`
(`query` / `wheel`), `event`, `result` (answer to `host_call`), `shutdown`. Plugin → host on stdout:
`ready` (first), `ack`, `state`, `error`, `result` (answer to `request`), `host_call`, `subscribe`,
`setup_required`. Unknown types are ignored. The manifest declares `"protocol": 2`.
