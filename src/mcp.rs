mod dispatch;
mod tools;

pub use dispatch::dispatch;

use crate::json::Value;
use std::io::{self, BufRead, Write as _};
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

static SESSION_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct ServerIndex { data: Vec<u8> }

static INDEX: RwLock<Option<ServerIndex>> = RwLock::new(None);
static INDEX_DIRTY: AtomicBool = AtomicBool::new(false);

pub(crate) fn log_session(msg: String) {
    if let Ok(mut log) = SESSION_LOG.lock() { log.push(msg); }
}

pub fn run(dir: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    ensure_datalog(dir);

    if std::env::var("AMARANTHINE_REEXEC").is_ok() {
        std::env::remove_var("AMARANTHINE_REEXEC");
        let notif = Value::Obj(vec![
            ("jsonrpc".into(), Value::Str("2.0".into())),
            ("method".into(), Value::Str("notifications/tools/list_changed".into())),
        ]);
        let mut out = stdout.lock();
        let _ = writeln!(out, "{notif}");
        let _ = out.flush();
    }

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.is_empty() { continue; }
        let msg = match crate::json::parse(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id");

        if method == "tools/call" {
            let p = msg.get("params");
            let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
            if name == "_reload" {
                let resp = rpc_ok(id, content_result("reloading amaranthine..."));
                let mut out = stdout.lock();
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
                drop(out);
                do_reload();
                continue;
            }
        }

        let resp = match method {
            "initialize" => Some(rpc_ok(id, init_result())),
            "notifications/initialized" | "initialized" => None,
            "tools/list" => Some(rpc_ok(id, Value::Obj(vec![
                ("tools".into(), tools::tool_list()),
            ]))),
            "tools/call" => {
                let p = msg.get("params");
                let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let args = p.and_then(|p| p.get("arguments"));
                Some(match dispatch::dispatch(name, args, dir) {
                    Ok(text) => rpc_ok(id, content_result(&text)),
                    Err(e) => rpc_err(id, -32603, &e),
                })
            }
            "ping" => Some(rpc_ok(id, Value::Obj(Vec::new()))),
            _ => id.map(|_| rpc_err(id, -32601, "method not found")),
        };

        if let Some(r) = resp {
            let mut out = stdout.lock();
            if let Err(e) = writeln!(out, "{r}") {
                eprintln!("amaranthine: stdout write error: {e}");
                break;
            }
            let _ = out.flush();
        }
    }
    Ok(())
}

fn do_reload() {
    use std::os::unix::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let src = exe.parent()
        .and_then(|p| p.parent())
        .and_then(|_| {
            let manifest = std::env::var("AMARANTHINE_SRC").ok()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    std::path::PathBuf::from(home).join("wudan/dojo/crash3/amaranthine")
                });
            let release = manifest.join("target/release/amaranthine");
            if release.exists() { Some(release) } else { None }
        });
    if let Some(src_bin) = src {
        if let Err(e) = std::fs::copy(&src_bin, &exe) {
            eprintln!("reload: copy failed: {e}");
        } else {
            let _ = std::process::Command::new("codesign")
                .args(["-s", "-", "-f"]).arg(&exe).output();
        }
    }
    std::env::set_var("AMARANTHINE_REEXEC", "1");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _err = std::process::Command::new(&exe).args(&args).exec();
    std::env::remove_var("AMARANTHINE_REEXEC");
    eprintln!("reload failed: {_err}");
}

fn ensure_datalog(dir: &Path) {
    if !crate::config::data_log_exists(dir) {
        if let Ok(files) = crate::config::list_topic_files(dir) {
            if !files.is_empty() {
                match crate::datalog::migrate_from_md(dir) {
                    Ok(n) => {
                        eprintln!("amaranthine: migrated {n} entries from .md to data.log");
                        for path in &files { let _ = std::fs::remove_file(path); }
                    }
                    Err(e) => eprintln!("amaranthine: migration failed: {e}"),
                }
            } else { let _ = crate::datalog::ensure_log(dir); }
        } else { let _ = crate::datalog::ensure_log(dir); }
    }
    match crate::inverted::rebuild(dir) {
        Ok((_, bytes)) => store_index(bytes),
        Err(_) => {} // no index yet, load_index in run() will try disk
    }
}

pub(crate) fn load_index(dir: &Path) {
    let index_path = dir.join("index.bin");
    if let Ok(data) = std::fs::read(&index_path) {
        store_index(data);
    }
}

pub(crate) fn store_index(data: Vec<u8>) {
    if let Ok(mut guard) = INDEX.write() {
        *guard = Some(ServerIndex { data });
    }
}

/// Borrow cached index data via closure. Returns None if no index loaded.
/// Uses RwLock read guard — concurrent reads don't block each other.
pub(crate) fn with_index<F, R>(f: F) -> Option<R>
where F: FnOnce(&[u8]) -> R {
    INDEX.read().ok().and_then(|guard| guard.as_ref().map(|idx| f(&idx.data)))
}

pub(crate) fn after_write(_dir: &Path, _topic: &str) {
    INDEX_DIRTY.store(true, Ordering::Release);
    // Don't invalidate corpus cache here — let with_corpus() mtime check handle
    // freshness. This allows rebuild() to reuse cached tokens on the next dispatch.
}

/// Rebuild index if dirty. Call before read operations.
/// F1: Uses returned bytes directly instead of re-reading from disk.
pub(crate) fn ensure_index_fresh(dir: &Path) {
    if INDEX_DIRTY.compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        match crate::inverted::rebuild(dir) {
            Ok((_, bytes)) => store_index(bytes),
            Err(_) => load_index(dir), // fallback: existing index.bin
        }
    }
}

fn init_result() -> Value {
    Value::Obj(vec![
        ("protocolVersion".into(), Value::Str("2024-11-05".into())),
        ("capabilities".into(), Value::Obj(vec![
            ("tools".into(), Value::Obj(Vec::new())),
        ])),
        ("serverInfo".into(), Value::Obj(vec![
            ("name".into(), Value::Str("amaranthine".into())),
            ("version".into(), Value::Str("5.2.0".into())),
        ])),
    ])
}

fn rpc_ok(id: Option<&Value>, result: Value) -> Value {
    Value::Obj(vec![
        ("jsonrpc".into(), Value::Str("2.0".into())),
        ("id".into(), id.cloned().unwrap_or(Value::Null)),
        ("result".into(), result),
    ])
}

fn rpc_err(id: Option<&Value>, code: i64, msg: &str) -> Value {
    Value::Obj(vec![
        ("jsonrpc".into(), Value::Str("2.0".into())),
        ("id".into(), id.cloned().unwrap_or(Value::Null)),
        ("error".into(), Value::Obj(vec![
            ("code".into(), Value::Num(code)),
            ("message".into(), Value::Str(msg.into())),
        ])),
    ])
}

fn content_result(text: &str) -> Value {
    Value::Obj(vec![("content".into(), Value::Arr(vec![
        Value::Obj(vec![
            ("type".into(), Value::Str("text".into())),
            ("text".into(), Value::Str(text.into())),
        ]),
    ]))])
}
