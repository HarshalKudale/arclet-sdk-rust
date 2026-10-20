//! `arclet-plugin` — CLI for Rust Arclet plugins (run from the plugin crate folder).
//!
//!   dev [--skip-setup] [--watch]   cargo build (debug), assemble dist/<id>/ for
//!                                  "Load Unpacked"; --watch rebuilds on change and
//!                                  Arclet restarts the plugin when its exe changes
//!   build                          cargo build --release, assemble dist/<id>/
//!   package                        zip dist/<id>/ into dist/<id>.arclet
//!
//! Codegen runs every built binary with ARCLET_CODEGEN=1; the plugin binary
//! prints { manifest, actions } and the setup binary exits silently.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const USAGE: &str = "arclet-plugin <dev|build|package>\n\n  dev [--skip-setup] [--watch]  build (debug) + assemble dist/<id>/ for \"Load Unpacked\"\n  build                         build (release) + assemble dist/<id>/\n  package                       zip dist/<id>/ -> dist/<id>.arclet\n";

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("\n  ✗ {msg}");
    std::process::exit(1)
}

fn copy_dir(src: &Path, dest: &Path) {
    if !src.is_dir() {
        return;
    }
    fs::create_dir_all(dest).unwrap_or_else(|e| die(e));
    for entry in fs::read_dir(src).unwrap_or_else(|e| die(e)).flatten() {
        let (s, d) = (entry.path(), dest.join(entry.file_name()));
        if s.is_dir() {
            copy_dir(&s, &d)
        } else {
            fs::copy(&s, &d).unwrap_or_else(|e| die(format!("copy {}: {e}", s.display())));
        }
    }
}

/// `cargo build`, returning every produced executable.
fn cargo_build(release: bool) -> Vec<PathBuf> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--message-format=json-render-diagnostics"]);
    if release {
        cmd.arg("--release");
    }
    let out = cmd.stderr(Stdio::inherit()).output().unwrap_or_else(|e| die(format!("cargo: {e}")));
    if !out.status.success() {
        die("cargo build failed");
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter_map(|m| m["executable"].as_str().map(PathBuf::from))
        .collect()
}

/// Run each binary in codegen mode; the one that prints a manifest is the plugin.
fn codegen(exes: &[PathBuf], stage: &str) -> (PathBuf, Value) {
    for exe in exes {
        let out = Command::new(exe)
            .env("ARCLET_CODEGEN", "1")
            .env("ARCLET_STAGE", stage)
            .stderr(Stdio::inherit())
            .output()
            .unwrap_or_else(|e| die(format!("{}: {e}", exe.display())));
        if !out.status.success() {
            die(format!("codegen failed for {}", exe.display()));
        }
        if let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) {
            if v["manifest"].is_object() {
                return (exe.clone(), v);
            }
        }
    }
    die("no binary produced a manifest — does main() call Plugin::run()?")
}

fn write_json(path: PathBuf, v: &Value) {
    fs::write(&path, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
}

/// Copy the setup page (+ assets dir) into dist/<id>/setup-ui and point the manifest at it.
fn stage_setup_ui(manifest: &mut Value, meta: &Value, dist: &Path) {
    let Some(page) = meta["setup_page"].as_str() else { return };
    let page = Path::new(page);
    if !page.is_file() {
        die(format!("Setup page \"{}\" not found.", page.display()));
    }
    let ui = dist.join("setup-ui");
    fs::create_dir_all(&ui).unwrap_or_else(|e| die(e));
    let name = page.file_name().unwrap();
    fs::copy(page, ui.join(name)).unwrap_or_else(|e| die(e));
    if let Some(assets) = meta["setup_assets_dir"].as_str() {
        copy_dir(Path::new(assets), &ui);
    }
    manifest["setup"]["page"] = format!("setup-ui/{}", name.to_string_lossy()).into();
}

/// Copy a built binary into `dist`. Arclet may be running the previous copy,
/// which Windows won't let us overwrite but will let us rename, so move it aside.
fn stage_exe(exe: &Path, dist: &Path) {
    let dest = dist.join(exe.file_name().unwrap());
    if dest.exists() {
        let old = dest.with_extension("old");
        let _ = fs::remove_file(&old);
        let _ = fs::rename(&dest, &old);
    }
    fs::copy(exe, &dest).unwrap_or_else(|e| die(format!("copy {}: {e}", exe.display())));
}

fn assemble(release: bool, skip_setup: bool) {
    // Both stages ship the binaries inside dist/<id>/: running them out of
    // target/ would lock them against the next cargo build.
    let stage = "build";
    println!("\n  Arclet Plugin — {}", if release { "build" } else { "dev" });
    let exes = cargo_build(release);
    let (plugin_exe, mut out) = codegen(&exes, stage);
    let mut manifest = out["manifest"].take();
    let actions = out["actions"].take();
    let id = manifest["id"].as_str().unwrap_or_else(|| die("manifest has no id")).to_string();

    let dist = Path::new("dist").join(&id);
    fs::create_dir_all(&dist).unwrap_or_else(|e| die(e));
    if skip_setup {
        manifest["setup_required"] = false.into();
    }
    stage_setup_ui(&mut manifest, &out["meta"], &dist);
    copy_dir(Path::new("icons"), &dist.join("icons"));

    // Self-contained: ship the plugin binary and (if declared) the setup binary.
    let mut ship = vec![plugin_exe.clone()];
    if let Some(setup) = manifest["setup"]["executable"].as_str() {
        match exes.iter().find(|e| e.file_name().map_or(false, |n| n == Path::new(setup).file_name().unwrap())) {
            Some(e) => ship.push(e.clone()),
            None => die(format!("setup binary \"{setup}\" was not built — add a [[bin]] for it")),
        }
    }
    for exe in ship {
        stage_exe(&exe, &dist);
    }
    if release {
        for readme in ["readme.md", "README.md"] {
            if Path::new(readme).is_file() {
                fs::copy(readme, dist.join("readme.md")).ok();
                break;
            }
        }
    }

    write_json(dist.join("manifest.json"), &manifest);
    write_json(dist.join("actions.json"), &actions);
    println!("  ✓ dist/{id}/ written{}", if release { "\n  Run \"arclet-plugin package\" to produce a .arclet file." } else { "" });
}

fn package() {
    let dirs: Vec<PathBuf> = fs::read_dir("dist")
        .unwrap_or_else(|_| die("no dist/ — run `arclet-plugin build` first"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    let dir = match dirs.as_slice() {
        [d] => d.clone(),
        [] => die("no dist/<id>/manifest.json — run `arclet-plugin build` first"),
        _ => die("multiple plugins under dist/ — remove the stale ones"),
    };
    let out = Path::new("dist").join(format!("{}.arclet", dir.file_name().unwrap().to_string_lossy()));
    // bsdtar (ships with Windows 10+, macOS, most Linux) writes zip with --format zip.
    let status = Command::new("tar")
        .args(["--format", "zip", "-cf"])
        .arg(&out)
        .arg("-C")
        .arg(&dir)
        // Top-level names (no "./" prefix); skip binaries `dev` moved aside.
        .args(
            fs::read_dir(&dir)
                .unwrap_or_else(|e| die(e))
                .flatten()
                .map(|e| e.file_name())
                .filter(|n| Path::new(n).extension().map_or(true, |x| x != "old")),
        )
        .status()
        .unwrap_or_else(|e| die(format!("tar: {e}")));
    if !status.success() {
        die("tar failed to create the archive");
    }
    println!("  ✓ {}", out.display());
}

/// Newest modification time under `path` (recursive).
fn newest_mtime(path: &Path) -> Option<std::time::SystemTime> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_dir() {
        return meta.modified().ok();
    }
    fs::read_dir(path).ok()?.flatten().filter_map(|e| newest_mtime(&e.path())).max()
}

/// Rebuild (`dev`) whenever src/, icons/, setup-ui/ or Cargo.toml change. Each
/// rebuild runs as a child so a compile error doesn't end the watch.
fn watch(skip_setup: bool) -> ! {
    let exe = std::env::current_exe().unwrap_or_else(|e| die(e));
    let mut last = None;
    loop {
        let stamp = ["src", "icons", "setup-ui", "Cargo.toml"].iter().filter_map(|p| newest_mtime(Path::new(p))).max();
        if stamp != last {
            last = stamp;
            let mut cmd = Command::new(&exe);
            cmd.arg("dev");
            if skip_setup {
                cmd.arg("--skip-setup");
            }
            let _ = cmd.status();
            println!("  … watching for changes (Ctrl+C to stop)");
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("dev") if flag("--watch") => watch(flag("--skip-setup")),
        Some("dev") => assemble(false, flag("--skip-setup")),
        Some("build") => assemble(true, false),
        Some("package") => package(),
        _ => print!("{USAGE}"),
    }
}
