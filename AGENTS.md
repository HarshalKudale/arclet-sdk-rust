# knobdeck-sdk: working notes for agents

- To **build a plugin**, follow `skills/knobdeck-plugin/SKILL.md`; start with `knobdeck-plugin new`.
- To **change the SDK**: the protocol is `src/runtime.rs` (v2, one JSON object per line), the
  builders `src/plugin.rs`, the generated manifest/actions `src/codegen.rs`, the CLI
  `src/bin/knobdeck-plugin.rs` (+ `knobdeck-plugin/scaffold.rs`, `knobdeck-plugin/drive.rs`).
- `knobdeck-plugin new` embeds `templates/plugin/*` and `skills/knobdeck-plugin/**` with
  `include_str!`: when you add a file to either, add it to `scaffold.rs` too.
- Keep the skill true: every API it names must exist, and `references/examples.md` must compile
  (scaffold a crate per example and `cargo build` it before changing the examples).
- First-party plugins (`../knobdeck-plugins`) depend on this repo's git master: push SDK changes
  before relying on them there.
- Before finishing: `cargo test` (includes `tests/protocol.rs`, which drives the demo over pipes)
  with no warnings.
