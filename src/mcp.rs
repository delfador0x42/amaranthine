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
static DIRTY_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

pub(crate) fn log_session(msg: String) {
    if let Ok(mut log) = SESSION_LOG.lock() { log.push(msg); }
}

pub fn run(dir: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    ensure_datalog(dir);

    // Start Unix socket listener for hook queries against in-memory index
    let _sock_guard = crate::sock::start_listener(dir);

    if std::env::var("AMARANTHINE_REEXEC").is_ok() {
        std::env::remove_var("AMARANTHINE_REEXEC");
        let mut out = stdout.lock();
        let _ = writeln!(out, r#"{{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}}"#);
        let _ = out.flush();
    }

    // Reusable line buffer — avoids allocation per message
    let mut line_buf = String::with_capacity(4096);
    let stdin_lock = stdin.lock();
    let mut reader = io::BufReader::new(stdin_lock);

    loop {
        line_buf.clear();
        match reader.read_line(&mut line_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => { return Err(e.to_string()); }
        }
        let line = line_buf.trim();
        if line.is_empty() || line.len() > 10_000_000 { continue; }
        let msg = match crate::json::parse(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id");

        let resp: Option<String> = match method {
            "initialize" => Some(rpc_ok_value(id, init_result())),
            "notifications/initialized" | "initialized" => None,
            "tools/list" => {
                // Fast path: pre-serialized tool list (cached, Arc avoids clone)
                let id_json = id_to_json(id);
                let tools_json = tools::tool_list_json();
                let mut out = stdout.lock();
                let _ = write!(out, r#"{{"jsonrpc":"2.0","id":{id_json},"result":{tools_json}}}"#);
                let _ = writeln!(out);
                let _ = out.flush();
                continue;
            }
            "tools/call" => {
                // Hot path: streaming write — no intermediate String allocation.
                // Extract name/args once (was duplicated for _reload check).
                let p = msg.get("params");
                let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let id_json = id_to_json(id);
                if name == "_reload" {
                    let mut out = stdout.lock();
                    let _ = writeln!(out,
                        r#"{{"jsonrpc":"2.0","id":{id_json},"result":{{"content":[{{"type":"text","text":"reloading amaranthine..."}}]}}}}"#);
                    let _ = out.flush();
                    drop(out);
                    do_reload();
                    continue;
                }
                let args = p.and_then(|p| p.get("arguments"));
                let mut out = stdout.lock();
                let ok = match dispatch::dispatch(name, args, dir) {
                    Ok(ref text) => write_rpc_ok(&mut out, &id_json, text),
                    Err(ref e) => write_rpc_err(&mut out, &id_json, e),
                };
                if let Err(e) = ok {
                    eprintln!("amaranthine: stdout write error: {e}");
                    break;
                }
                let _ = out.flush();
                continue;
            }
            "ping" => {
                let id_json = id_to_json(id);
                Some(format!(r#"{{"jsonrpc":"2.0","id":{id_json},"result":{{}}}}"#))
            }
            _ => id.map(|_| {
                let id_json = id_to_json(id);
                rpc_err_str(&id_json, -32601, "method not found")
            }),
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

/// Convert id Value to JSON string representation. Avoids Value::to_string() allocation
/// for common cases (integer and null).
fn id_to_json(id: Option<&Value>) -> String {
    match id {
        Some(Value::Num(n)) if n.fract() == 0.0 => format!("{}", *n as i64),
        Some(v) => v.to_string(),
        None => "null".into(),
    }
}

/// Streaming JSON-RPC success response — writes directly to stdout, zero intermediate String.
/// This is the hot path for every tools/call response.
fn write_rpc_ok(w: &mut impl io::Write, id_json: &str, text: &str) -> io::Result<()> {
    w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")?;
    w.write_all(id_json.as_bytes())?;
    w.write_all(b",\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"")?;
    write_json_escaped(w, text)?;
    w.write_all(b"\"}]}}\n")
}

/// Streaming JSON-RPC error response — writes directly to stdout, zero intermediate String.
fn write_rpc_err(w: &mut impl io::Write, id_json: &str, msg: &str) -> io::Result<()> {
    w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")?;
    w.write_all(id_json.as_bytes())?;
    w.write_all(b",\"error\":{\"code\":-32603,\"message\":\"")?;
    write_json_escaped(w, msg)?;
    w.write_all(b"\"}}\n")
}

/// Write JSON-escaped string directly to a writer (no intermediate String allocation).
/// Byte-level chunk-copy: scans for escape-needing bytes, writes clean chunks via write_all.
fn write_json_escaped(w: &mut impl io::Write, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    let mut last_copy = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            c if c < 0x20 => {
                if last_copy < i { w.write_all(&bytes[last_copy..i])?; }
                write!(w, "\\u{:04x}", c)?;
                last_copy = i + 1;
                continue;
            }
            _ => continue,
        };
        if last_copy < i { w.write_all(&bytes[last_copy..i])?; }
        w.write_all(esc)?;
        last_copy = i + 1;
    }
    if last_copy < bytes.len() { w.write_all(&bytes[last_copy..])?; }
    Ok(())
}

/// Build a JSON-RPC error response as String — used for non-tools/call errors.
fn rpc_err_str(id_json: &str, code: i64, msg: &str) -> String {
    let mut out = String::with_capacity(128 + msg.len());
    out.push_str(r#"{"jsonrpc":"2.0","id":"#);
    out.push_str(id_json);
    out.push_str(r#","error":{"code":"#);
    use std::fmt::Write as _;
    let _ = write!(out, "{code}");
    out.push_str(r#","message":""#);
    crate::json::escape_into(msg, &mut out);
    out.push_str(r#""}}"#);
    out
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
    match crate::inverted::rebuild_and_persist(dir) {
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
    // Record when dirty flag was set for debounce
    if let Ok(mut guard) = DIRTY_AT.lock() {
        if guard.is_none() { *guard = Some(std::time::Instant::now()); }
    }
}

/// Rebuild index if dirty and debounce window (100ms) has elapsed.
/// Burst writes within the window are coalesced into a single rebuild.
/// F1: Uses returned bytes directly instead of re-reading from disk.
pub(crate) fn ensure_index_fresh(dir: &Path) {
    if !INDEX_DIRTY.load(Ordering::Acquire) { return; }
    // Debounce: skip rebuild if dirty flag was set less than 100ms ago
    let elapsed = DIRTY_AT.lock().ok()
        .and_then(|g| g.map(|t| t.elapsed()));
    if let Some(dt) = elapsed {
        if dt < std::time::Duration::from_millis(50) {
            return; // too soon — let writes accumulate
        }
    }
    if INDEX_DIRTY.compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        if let Ok(mut guard) = DIRTY_AT.lock() { *guard = None; }
        match crate::inverted::rebuild(dir) {
            Ok((_, bytes)) => store_index(bytes),
            Err(_) => load_index(dir),
        }
    }
}

/// Initialize result — only called once per session, Value tree is fine here.
fn init_result() -> Value {
    Value::Obj(vec![
        ("protocolVersion".into(), Value::Str("2024-11-05".into())),
        ("capabilities".into(), Value::Obj(vec![
            ("tools".into(), Value::Obj(Vec::new())),
        ])),
        ("serverInfo".into(), Value::Obj(vec![
            ("name".into(), Value::Str("amaranthine".into())),
            ("version".into(), Value::Str("6.4.0".into())),
        ])),
    ])
}

/// Build JSON-RPC OK response wrapping a Value (only for initialize).
fn rpc_ok_value(id: Option<&Value>, result: Value) -> String {
    let id_json = id_to_json(id);
    let result_json = result.to_string();
    format!(r#"{{"jsonrpc":"2.0","id":{id_json},"result":{result_json}}}"#)
}
