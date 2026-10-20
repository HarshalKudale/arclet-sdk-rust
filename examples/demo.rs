//! Port of the JS persistent example: button, toggle, hold, slider, query-backed
//! inputs and a background ticker. Run `arclet-plugin dev` from a crate using it.

use arclet_sdk::input::{self, InputExt};
use arclet_sdk::{json, Action, Plugin, Visual};
use std::time::Duration;

fn main() {
    Plugin::new("example-rust", "Example Rust")
        .version("1.0.0")
        .author("Arclet")
        .description("Reference plugin in Rust.")
        .action(
            Action::button("ping", "Ping (log)")
                .inputs(vec![input::text("message", "Message").placeholder("hello")])
                .on_press(|ctx| {
                    ctx.log(format!("ping instance={} inputs={}", ctx.instance_id, ctx.inputs));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("mute", "Demo Mute (fan-out)")
                .state("unmuted", "Unmuted", "🔊")
                .state("muted", "Muted", "🔇")
                .on_press(|ctx| {
                    ctx.fan_out(if ctx.state.as_deref() == Some("muted") { "unmuted" } else { "muted" });
                    Ok(())
                }),
        )
        .action(
            Action::hold("ptt", "Push-to-talk")
                .released("Idle", "○")
                .held("Holding", "●")
                .on_hold_start(|ctx| Ok(ctx.set_state("held")))
                .on_hold_end(|ctx| Ok(ctx.set_state("released"))),
        )
        .action(
            Action::slider("volume", "Volume", 0.0, 100.0).unit("%").initial("50").on_adjust(|ctx| {
                let v = ctx.value.unwrap_or(50.0 + ctx.delta.unwrap_or(0.0)).clamp(0.0, 100.0);
                ctx.set_state(v);
                Ok(())
            }),
        )
        .action(
            Action::button("pick", "Query demo").inputs(vec![
                input::query_select("group", "Group", "groups", &[]),
                input::query_select("item", "Item", "items", &["group"]),
            ]).on_press(|ctx| ctx.fail("demo error")),
        )
        .query("groups", |_| Ok(json!([{"value": "fruit", "label": "Fruit"}, {"value": "veg", "label": "Veg"}])))
        .query("items", |ctx| {
            Ok(match ctx.input_str("group").as_str() {
                "fruit" => json!(["apple", "pear"]),
                _ => json!(["carrot", "pea"]),
            })
        })
        .on_ready(|rt| {
            let rt = rt.clone();
            std::thread::spawn(move || {
                let mut n = 0;
                loop {
                    n += 1;
                    rt.notify_visual("ping", Visual::new().label(format!("tick {n}")));
                    std::thread::sleep(Duration::from_secs(3));
                }
            });
            Ok(())
        })
        .run()
}
