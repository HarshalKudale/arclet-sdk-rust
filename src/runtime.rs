//! Plugin runtime: connects to the host over WebSocket, registers, and routes
//! `sync` / `invoke` / `query` / `get_wheel` / `event` / `host_result` frames.
//! Every invoke / query / wheel / event runs on its own thread, so slow handlers
//! never block the socket reader (the Rust analogue of async handlers in JS).

use crate::codegen::make_full_id;
use crate::ctx::{Ctx, Event, InitialCtx, OpenCtx, QueryCtx};
use crate::error::{Error, Result};
use crate::log::log;
use crate::plugin::{Action, Kind, Plugin};
use crate::ws::{self, Frame};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const HOST: &str = "127.0.0.1";
const MAX_WHEEL_BUTTONS: usize = 16;

/// Partial visual update for one instance. Fields left `None` are not sent; a
/// patch without `state` leaves the instance's state/error untouched.
#[derive(Default, Clone, Debug)]
pub struct Visual {
    pub state: Option<String>,
    pub label: Option<String>,
    pub icon: Option<String>,
}

impl Visual {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn state(mut self, v: impl ToString) -> Self {
        self.state = Some(v.to_string());
        self
    }
    pub fn label(mut self, v: impl ToString) -> Self {
        self.label = Some(v.to_string());
        self
    }
    pub fn icon(mut self, v: impl ToString) -> Self {
        self.icon = Some(v.to_string());
        self
    }

    fn frame(&self, target: (&str, &str)) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), "state".into());
        m.insert(target.0.into(), target.1.into());
        if let Some(v) = &self.state {
            m.insert("state".into(), v.clone().into());
        }
        if let Some(v) = &self.label {
            m.insert("label".into(), v.clone().into());
        }
        if let Some(v) = &self.icon {
            m.insert("icon".into(), v.clone().into());
        }
        Value::Object(m)
    }
}

pub(crate) struct Inner {
    writer: Mutex<Option<TcpStream>>,
    state: Mutex<HashMap<String, String>>,
    action_of: Mutex<HashMap<String, String>>,
    inputs_of: Mutex<HashMap<String, Value>>,
    short_to_full: HashMap<String, String>,
    full_to_short: HashMap<String, String>,
    pending: Mutex<HashMap<String, Sender<std::result::Result<Value, String>>>>,
    next_id: AtomicU64,
}

/// Handle for pushing state/visuals/errors outside of a handler (timers, pollers,
/// external listeners). Cheap to clone and `Send`; hand it to your own threads.
#[derive(Clone)]
pub struct Runtime(pub(crate) Arc<Inner>);

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub(crate) fn global() -> Option<&'static Runtime> {
    RUNTIME.get()
}

impl Runtime {
    pub fn log(&self, msg: impl AsRef<str>) {
        log(msg)
    }

    pub(crate) fn send(&self, obj: Value) {
        let frame = ws::encode_text(&obj.to_string());
        if let Some(w) = self.0.writer.lock().unwrap().as_mut() {
            if let Err(e) = w.write_all(&frame) {
                log(format!("send error: {e}"));
            }
        }
        // Only frames that carry a `state` update the local mirror.
        if obj["type"] == "state" {
            if let Some(s) = obj["state"].as_str() {
                let mut st = self.0.state.lock().unwrap();
                if let Some(iid) = obj["instance_id"].as_str() {
                    st.insert(iid.into(), s.into());
                } else if let Some(aid) = obj["action_id"].as_str() {
                    let short = self.0.full_to_short.get(aid).map(String::as_str).unwrap_or(aid);
                    for (iid, a) in self.0.action_of.lock().unwrap().iter() {
                        if a == short {
                            st.insert(iid.clone(), s.into());
                        }
                    }
                }
            }
        }
    }

    fn full_id(&self, short: &str) -> String {
        self.0.short_to_full.get(short).cloned().unwrap_or_else(|| short.into())
    }

    /// Set the state of one instance.
    pub fn set_state(&self, instance_id: &str, value: impl ToString) {
        self.send(json!({"type": "state", "instance_id": instance_id, "state": value.to_string()}));
    }
    /// Fan-out: set the state of ALL instances of an action (short action id).
    pub fn notify(&self, action_id: &str, value: impl ToString) {
        self.send(json!({"type": "state", "action_id": self.full_id(action_id), "state": value.to_string()}));
    }
    pub fn set_visual(&self, instance_id: &str, patch: Visual) {
        self.send(patch.frame(("instance_id", instance_id)));
    }
    pub fn notify_visual(&self, action_id: &str, patch: Visual) {
        self.send(patch.frame(("action_id", &self.full_id(action_id))));
    }
    pub fn set_error(&self, instance_id: &str, message: impl ToString) {
        self.send(json!({"type": "error", "instance_id": instance_id, "message": message.to_string()}));
    }
    /// Subscribe to host events (e.g. `"focus_changed"`).
    pub fn subscribe<S: AsRef<str>>(&self, events: &[S]) {
        let names = normalize(events.iter().map(|s| s.as_ref().to_string()));
        if !names.is_empty() {
            self.send(json!({"type": "subscribe", "events": names}));
        }
    }
    /// Flag the plugin as needing setup again (writes `setup_required` to
    /// manifest.json and tells the host immediately).
    pub fn mark_setup_required(&self) {
        crate::mark_setup_required();
        self.send(json!({"type": "setup_required"}));
    }
    /// Instance ids of every placed instance of `action_id` (short id) learned from sync.
    pub fn instances_of(&self, action_id: &str) -> Vec<String> {
        self.0
            .action_of
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, a)| a.as_str() == action_id)
            .map(|(i, _)| i.clone())
            .collect()
    }
    /// Configured inputs of a placed instance.
    pub fn inputs_of(&self, instance_id: &str) -> Value {
        self.0.inputs_of.lock().unwrap().get(instance_id).cloned().unwrap_or(json!({}))
    }

    pub(crate) fn host_call(&self, method: &str, params: Value) -> Result<Value> {
        let id = format!("host-{}", self.0.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = channel();
        self.0.pending.lock().unwrap().insert(id.clone(), tx);
        self.send(json!({"type": "host_call", "request_id": id, "method": method, "params": params}));
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(Error::Other(e)),
            Err(_) => {
                self.0.pending.lock().unwrap().remove(&id);
                Err(Error::msg("Arclet host call timed out"))
            }
        }
    }
}

fn normalize(events: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in events {
        let t = e.trim().to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

fn compute_initial(def: Option<&Action>, ic: &InitialCtx) -> String {
    let Some(def) = def else { return "off".into() };
    if def.kind == Kind::Hold {
        return "released".into();
    }
    if let Some(init) = &def.initial {
        if let Some(v) = init.resolve(ic) {
            return v;
        }
    }
    if let Some(v) = def.states.first().and_then(|s| s.get("value")) {
        return value_str(v);
    }
    if def.kind == Kind::Slider {
        return value_str(&json!(def.min));
    }
    "off".into()
}

pub(crate) fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

struct Core {
    plugin: Plugin,
    rt: Runtime,
}

impl Core {
    fn action(&self, short: &str) -> Option<&Action> {
        self.plugin.actions.get(short)
    }
    fn short(&self, full: &str) -> String {
        self.rt.0.full_to_short.get(full).cloned().unwrap_or_else(|| full.into())
    }
}

pub(crate) fn run(plugin: Plugin) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port: u16 = match arg(&args, "--port").and_then(|p| p.parse().ok()) {
        Some(p) => p,
        None => {
            eprintln!("Arclet SDK: --port is required for plugins");
            std::process::exit(1)
        }
    };
    let plugin_id = arg(&args, "--plugin-id").filter(|s| !s.is_empty()).unwrap_or_else(|| plugin.meta.id.clone());
    let token = arg(&args, "--token").unwrap_or_default();

    let mut short_to_full = HashMap::new();
    let mut full_to_short = HashMap::new();
    for short in plugin.actions.keys() {
        let full = make_full_id(&plugin_id, short);
        short_to_full.insert(short.clone(), full.clone());
        full_to_short.insert(full, short.clone());
    }

    let addr = (HOST, port).to_socket_addrs().ok().and_then(|mut a| a.next()).expect("resolve host");
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
        Ok(s) => s,
        Err(e) => {
            log(format!("connect error: {e}"));
            std::process::exit(1)
        }
    };
    let _ = stream.set_nodelay(true);
    let mut reader = stream.try_clone().expect("clone socket");
    stream.write_all(ws::handshake_request(HOST, port).as_bytes()).expect("handshake");

    let rt = Runtime(Arc::new(Inner {
        writer: Mutex::new(Some(stream)),
        state: Default::default(),
        action_of: Default::default(),
        inputs_of: Default::default(),
        short_to_full,
        full_to_short,
        pending: Default::default(),
        next_id: AtomicU64::new(1),
    }));
    let _ = RUNTIME.set(rt.clone());

    // Panics inside handler threads are caught per-thread; keep the default
    // stderr noise out of the host and in log.txt instead.
    std::panic::set_hook(Box::new(|info| log(format!("panic: {info}"))));

    let core = Arc::new(Core { plugin, rt: rt.clone() });
    let mut buf: Vec<u8> = Vec::new();
    let mut upgraded = false;
    let mut chunk = [0u8; 16384];

    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);

        if !upgraded {
            let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            upgraded = true;
            buf.drain(..end + 4);
            // register MUST be the first WS frame
            rt.send(json!({"type": "register", "plugin": plugin_id, "token": token}));
            let mut wanted = core.plugin.subscriptions.clone();
            if core.plugin.actions.values().any(|a| a.volatile) {
                wanted.push("focus_changed".into());
            }
            let wanted = normalize(wanted.into_iter());
            if !wanted.is_empty() {
                rt.send(json!({"type": "subscribe", "events": wanted}));
            }
            log("registered");
        }

        for frame in ws::parse_frames(&mut buf) {
            match frame {
                Frame::Close => finish(&rt),
                Frame::Ping(p) => {
                    if let Some(w) = rt.0.writer.lock().unwrap().as_mut() {
                        let _ = w.write_all(&ws::encode_pong(&p));
                    }
                }
                Frame::Json(msg) => dispatch(&core, msg),
            }
        }
    }
    finish(&rt)
}

fn finish(rt: &Runtime) -> ! {
    for (_, tx) in rt.0.pending.lock().unwrap().drain() {
        let _ = tx.send(Err("Arclet host bridge disconnected".into()));
    }
    log("disconnected");
    std::process::exit(0)
}

/// Run `f` on its own thread, converting panics into a logged message.
fn spawn(what: &'static str, f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        if catch_unwind(AssertUnwindSafe(f)).is_err() {
            log(format!("{what}: handler panicked"));
        }
    });
}

fn dispatch(core: &Arc<Core>, msg: Value) {
    let c = core.clone();
    match msg["type"].as_str().unwrap_or("") {
        "sync" => handle_sync(core, &msg),
        "invoke" => {
            // ACK IMMEDIATELY on the reader thread, before any handler work.
            core.rt.send(json!({"type": "ack", "request_id": msg["request_id"]}));
            spawn("invoke", move || handle_invoke(&c, msg));
        }
        "query" => spawn("query", move || handle_query(&c, msg)),
        "get_wheel" => spawn("get_wheel", move || handle_get_wheel(&c, msg)),
        "event" => handle_event(core, msg),
        "host_result" => {
            let Some(id) = msg["request_id"].as_str() else { return };
            if let Some(tx) = core.rt.0.pending.lock().unwrap().remove(id) {
                let _ = tx.send(match msg["error"].as_str() {
                    Some(e) => Err(e.into()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                });
            }
        }
        _ => {}
    }
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(String::from)
}

fn obj_or_empty(v: &Value) -> Value {
    if v.is_object() {
        v.clone()
    } else {
        json!({})
    }
}

fn handle_sync(core: &Arc<Core>, msg: &Value) {
    let rt = &core.rt;
    let fg_process = opt_str(&msg["fg_process"]);
    let fg_title = opt_str(&msg["fg_title"]);
    let instances = msg["instances"].as_array().cloned().unwrap_or_default();
    log(format!("sync: {} instance(s)", instances.len()));

    for inst in &instances {
        let Some(iid) = inst["instance_id"].as_str() else { continue };
        let action_id = core.short(inst["action_id"].as_str().unwrap_or(""));
        let inputs = obj_or_empty(&inst["inputs"]);
        rt.0.action_of.lock().unwrap().insert(iid.into(), action_id.clone());
        rt.0.inputs_of.lock().unwrap().insert(iid.into(), inputs.clone());
        let def = core.action(&action_id);
        // Volatile actions re-query initial() on every sync (per-window state).
        let volatile = def.map_or(false, |d| d.volatile);
        let known = rt.0.state.lock().unwrap().contains_key(iid);
        if !known || volatile {
            let ic = InitialCtx {
                instance_id: iid.into(),
                action_id,
                inputs,
                fg_process: fg_process.clone(),
                fg_title: fg_title.clone(),
            };
            let s = compute_initial(def, &ic);
            rt.0.state.lock().unwrap().insert(iid.into(), s);
        }
        let s = rt.0.state.lock().unwrap().get(iid).cloned().unwrap_or_default();
        rt.send(json!({"type": "state", "instance_id": iid, "state": s}));
    }

    // on_ready fires once after the first sync: all instances are known.
    if let Some(f) = &core.plugin.on_ready {
        if !core.plugin.ready_fired.swap(true, Ordering::SeqCst) {
            let (f, rt) = (f.clone(), rt.clone());
            spawn("on_ready", move || {
                if let Err(e) = f(&rt) {
                    log(format!("on_ready error: {e}"));
                }
            });
        }
    }
}

fn handle_event(core: &Arc<Core>, msg: Value) {
    let name = msg["event"].as_str().unwrap_or("").to_string();
    let payload = obj_or_empty(&msg["payload"]);
    let fg_process = opt_str(&payload["fg_process"]);
    let fg_title = opt_str(&payload["fg_title"]);

    if name == "focus_changed" {
        let snapshot = core.rt.0.action_of.lock().unwrap().clone();
        for (iid, aid) in snapshot {
            let Some(def) = core.action(&aid).filter(|d| d.volatile) else { continue };
            let inputs = core.rt.0.inputs_of.lock().unwrap().get(&iid).cloned().unwrap_or(json!({}));
            let ic = InitialCtx {
                instance_id: iid.clone(),
                action_id: aid.clone(),
                inputs,
                fg_process: fg_process.clone(),
                fg_title: fg_title.clone(),
            };
            let next = compute_initial(Some(def), &ic);
            if core.rt.0.state.lock().unwrap().get(&iid) == Some(&next) {
                continue;
            }
            core.rt.send(json!({"type": "state", "instance_id": iid, "state": next}));
        }
    }

    if let Some(f) = core.plugin.on_event.clone() {
        let rt = core.rt.clone();
        spawn("on_event", move || {
            let evt = Event { name: name.clone(), payload, fg_process, fg_title };
            if let Err(e) = f(&evt, &rt) {
                log(format!("on_event error ({name}): {e}"));
            }
        });
    }
}

fn report(core: &Core, what: &str, instance_id: &str, r: Result) {
    if let Err(Error::Other(m)) = r {
        log(format!("{what}: {m}"));
        core.rt.set_error(instance_id, m);
    }
}

fn handle_invoke(core: &Arc<Core>, msg: Value) {
    let rt = &core.rt;
    let instance_id = msg["instance_id"].as_str().unwrap_or("").to_string();
    let full = msg["action_id"].as_str().unwrap_or("").to_string();
    let action_id = core.short(&full);
    let trigger = msg["trigger"].as_str().unwrap_or("press").to_string();
    let inputs = obj_or_empty(&msg["inputs"]);
    rt.0.action_of.lock().unwrap().insert(instance_id.clone(), action_id.clone());

    let Some(def) = core.action(&action_id) else {
        log(format!("unknown action: {action_id}"));
        return;
    };

    let child_id = match &msg["child_id"] {
        Value::Null => None,
        v => Some(value_str(v)),
    };
    let is_adjust = def.kind == Kind::Slider && trigger == "adjust";
    let ctx = Ctx {
        instance_id: instance_id.clone(),
        action_id: action_id.clone(),
        full_action_id: full,
        trigger: trigger.clone(),
        state: rt.0.state.lock().unwrap().get(&instance_id).cloned(),
        fg_process: opt_str(&msg["fg_process"]),
        fg_title: opt_str(&msg["fg_title"]),
        child_id: child_id.clone(),
        value: if is_adjust { inputs.get("value").and_then(num) } else { None },
        delta: if is_adjust { inputs.get("delta").and_then(num) } else { None },
        inputs,
        rt: rt.clone(),
    };

    if child_id.is_some() {
        match &def.on_child_press {
            Some(h) => report(core, &format!("wheel child ({action_id})"), &instance_id, h(&ctx)),
            None => log(format!("wheel action \"{action_id}\" has no on_child_press handler")),
        }
        return;
    }
    let handler = if is_adjust && def.on_adjust.is_some() {
        &def.on_adjust
    } else if trigger == "hold_start" && def.on_hold_start.is_some() {
        &def.on_hold_start
    } else if trigger == "hold_end" && def.on_hold_end.is_some() {
        &def.on_hold_end
    } else {
        &def.on_press
    };
    if let Some(h) = handler {
        report(core, &format!("handler ({action_id})"), &instance_id, h(&ctx));
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn handle_query(core: &Arc<Core>, msg: Value) {
    let rid = msg["request_id"].clone();
    let qid = msg["query_id"].as_str().unwrap_or("");
    let Some(provider) = core.plugin.queries.get(qid) else {
        log(format!("unknown query: {qid}"));
        core.rt.send(json!({"type": "query_result", "request_id": rid, "error": format!("unknown query \"{qid}\"")}));
        return;
    };
    let ctx = QueryCtx {
        inputs: obj_or_empty(&msg["inputs"]),
        fg_process: opt_str(&msg["fg_process"]),
        fg_title: opt_str(&msg["fg_title"]),
    };
    core.rt.send(match provider(&ctx) {
        Ok(v) => json!({"type": "query_result", "request_id": rid, "result": v}),
        Err(e) => {
            if let Error::Other(m) = &e {
                log(format!("query error ({qid}): {m}"));
            }
            json!({"type": "query_result", "request_id": rid, "error": e.message()})
        }
    });
}

fn handle_get_wheel(core: &Arc<Core>, msg: Value) {
    let rid = msg["request_id"].clone();
    let action_id = core.short(msg["action_id"].as_str().unwrap_or(""));
    let Some(handler) = core.action(&action_id).and_then(|d| d.on_open.as_ref()) else {
        log(format!("unknown wheel action: {action_id}"));
        core.rt.send(json!({"type": "wheel_result", "request_id": rid, "error": format!("unknown wheel action \"{action_id}\"")}));
        return;
    };
    if let Some(iid) = msg["instance_id"].as_str() {
        core.rt.0.action_of.lock().unwrap().insert(iid.into(), action_id.clone());
    }
    let ctx = OpenCtx {
        inputs: obj_or_empty(&msg["inputs"]),
        fg_process: opt_str(&msg["fg_process"]),
        fg_title: opt_str(&msg["fg_title"]),
    };
    core.rt.send(match handler(&ctx) {
        Ok(mut slices) => {
            if slices.len() > MAX_WHEEL_BUTTONS {
                log(format!(
                    "on_open() for wheel \"{action_id}\" returned {} slices — truncated to {MAX_WHEEL_BUTTONS}",
                    slices.len()
                ));
                slices.truncate(MAX_WHEEL_BUTTONS);
            }
            json!({"type": "wheel_result", "request_id": rid, "wheel": {"mode": "buttons", "slices": slices}})
        }
        Err(e) => {
            if let Error::Other(m) = &e {
                log(format!("on_open error ({action_id}): {m}"));
            }
            json!({"type": "wheel_result", "request_id": rid, "error": e.message()})
        }
    });
}
