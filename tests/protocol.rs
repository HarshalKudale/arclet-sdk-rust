//! End-to-end: run the demo example against a mock host and drive the protocol.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn demo_exe() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop(); // deps/
    p.pop();
    p.join("examples").join(format!("demo{}", std::env::consts::EXE_SUFFIX))
}

fn send(s: &mut TcpStream, v: Value) {
    // Server → client frames are unmasked.
    let b = v.to_string().into_bytes();
    let mut f = vec![0x81];
    if b.len() < 126 {
        f.push(b.len() as u8);
    } else {
        f.push(126);
        f.extend((b.len() as u16).to_be_bytes());
    }
    f.extend(b);
    s.write_all(&f).unwrap();
}

fn read_frame(s: &mut TcpStream) -> Value {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).unwrap();
    let mut len = (h[1] & 0x7f) as usize;
    if len == 126 {
        let mut l = [0u8; 2];
        s.read_exact(&mut l).unwrap();
        len = u16::from_be_bytes(l) as usize;
    }
    let mut mask = [0u8; 4];
    assert!(h[1] & 0x80 != 0, "client frames must be masked");
    s.read_exact(&mut mask).unwrap();
    let mut p = vec![0u8; len];
    s.read_exact(&mut p).unwrap();
    p.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
    serde_json::from_slice(&p).unwrap()
}

/// Read frames until one has the wanted `type` (ignoring unrelated pushes like the ticker).
fn expect(s: &mut TcpStream, ty: &str) -> Value {
    for _ in 0..20 {
        let f = read_frame(s);
        if f["type"] == ty {
            return f;
        }
    }
    panic!("never saw a {ty} frame");
}

#[test]
fn register_sync_invoke_query() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!("arclet-sdk-test-{port}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(demo_exe())
        .args(["--plugin-id", "example-rust", "--port", &port.to_string(), "--token", "tok"])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn demo (run via `cargo test`, which builds examples)");

    let (mut s, _) = listener.accept().unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    // Consume the HTTP upgrade request and answer 101.
    let mut req = Vec::new();
    let mut b = [0u8; 1];
    while !req.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        req.push(b[0]);
    }
    s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").unwrap();

    let reg = read_frame(&mut s);
    assert_eq!(reg, json!({"type": "register", "plugin": "example-rust", "token": "tok"}));

    let mute = arclet_sdk::make_full_id("example-rust", "mute");
    send(&mut s, json!({"type": "sync", "instances": [{"instance_id": "i1", "action_id": mute, "inputs": {}}]}));
    let st = expect(&mut s, "state");
    assert_eq!((st["instance_id"].as_str(), st["state"].as_str()), (Some("i1"), Some("unmuted")));

    send(&mut s, json!({"type": "invoke", "request_id": "r1", "instance_id": "i1", "action_id": mute, "trigger": "press", "inputs": {}}));
    assert_eq!(expect(&mut s, "ack")["request_id"], "r1");
    let fan = expect(&mut s, "state");
    assert_eq!((fan["action_id"].as_str(), fan["state"].as_str()), (Some(mute.as_str()), Some("muted")));

    send(&mut s, json!({"type": "query", "request_id": "q1", "query_id": "items", "inputs": {"group": "fruit"}}));
    assert_eq!(expect(&mut s, "query_result")["result"], json!(["apple", "pear"]));

    // An action that fails reports an error frame and no panic.
    let pick = arclet_sdk::make_full_id("example-rust", "pick");
    send(&mut s, json!({"type": "invoke", "request_id": "r2", "instance_id": "i2", "action_id": pick, "trigger": "press", "inputs": {}}));
    assert_eq!(expect(&mut s, "error")["message"], "demo error");

    child.kill().ok();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn codegen_emits_manifest_and_actions() {
    let out = Command::new(demo_exe()).env("ARCLET_CODEGEN", "1").env("ARCLET_STAGE", "build").output().unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["manifest"]["id"], "example-rust");
    assert!(v["manifest"]["runtime"].get("language").is_none());
    let actions = v["actions"]["actions"].as_array().unwrap();
    assert_eq!(actions[0]["id"], arclet_sdk::make_full_id("example-rust", "ping"));
    assert_eq!(actions.iter().find(|a| a["type"] == "slider").unwrap()["max"], 100.0);
}
