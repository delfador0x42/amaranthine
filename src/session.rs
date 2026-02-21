//! Session accumulator: lightweight state tracking what's happening RIGHT NOW.
//!
//! Updated by every hook invocation and MCP call. Used to:
//! - Dedup injected context (injected FxHashSet)
//! - Weight search results (focus_topics)
//! - Suppress noise (phase-aware output)
//! - Track build state (last_build)
//!
//! Session identity: TTY name + 4h idle timeout.
//! Concurrency: flock for single-writer, multi-reader.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

extern "C" {
    fn ttyname(fd: i32) -> *const i8;
    fn flock(fd: i32, operation: i32) -> i32;
}

const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;
const IDLE_TIMEOUT_SECS: u64 = 4 * 3600; // 4 hours

/// Session state — lives in ~/.amaranthine/session.json.
pub struct Session {
    pub id: String,
    pub started: u64,
    pub last_active: u64,
    pub focus_topics: Vec<String>,
    pub phase: Phase,
    pub files: Vec<FileEntry>,
    pub injected: crate::fxhash::FxHashSet<u32>,  // O(1) dedup
    pub last_build: Option<BuildState>,
    pub tool_seq: Vec<String>,    // recent tool names (sliding window)
    pub pending_notes: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    Research,
    Build,
    Verify,
    Debug,
    Unknown,
}

pub struct FileEntry {
    pub path: String,
    pub op: FileOp,
    pub t: u64,
}

#[derive(Clone, Copy)]
pub enum FileOp { Read, Created, Edited }

pub struct BuildState {
    pub ok: bool,
    pub t: u64,
    pub errors: Vec<String>,
}

/// Get TTY name for current process (e.g. "/dev/ttys003").
/// Returns None if not attached to a terminal.
fn tty_name() -> Option<String> {
    let ptr = unsafe { ttyname(0) }; // STDIN_FILENO = 0
    if ptr.is_null() { return None; }
    let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(|s| s.to_string())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn session_path(dir: &Path) -> PathBuf {
    dir.join("session.json")
}

impl Session {
    /// Create a fresh session with TTY-based identity.
    pub fn new() -> Self {
        let now = now_secs();
        let tty = tty_name().unwrap_or_default();
        // Sanitize TTY: "/dev/ttys003" → "ttys003"
        let tty_short = tty.rsplit('/').next().unwrap_or("unknown");
        let id = format!("{tty_short}-{now}");
        Session {
            id, started: now, last_active: now,
            focus_topics: Vec::new(), phase: Phase::Unknown,
            files: Vec::new(),
            injected: crate::fxhash::FxHashSet::default(),
            last_build: None, tool_seq: Vec::new(),
            pending_notes: Vec::new(),
        }
    }

    /// Load session from disk. Returns None if expired, missing, or corrupt.
    pub fn load(dir: &Path) -> Option<Self> {
        let path = session_path(dir);
        let mut file = File::open(&path).ok()?;
        let mut buf = String::new();
        file.read_to_string(&mut buf).ok()?;
        let val = crate::json::parse(&buf).ok()?;
        let s = Self::from_json(&val)?;
        // Check idle timeout
        let now = now_secs();
        if now.saturating_sub(s.last_active) > IDLE_TIMEOUT_SECS {
            return None; // expired
        }
        // Check TTY match (if we have a TTY)
        if let Some(tty) = tty_name() {
            let tty_short = tty.rsplit('/').next().unwrap_or("");
            if !s.id.starts_with(tty_short) {
                return None; // different terminal
            }
        }
        Some(s)
    }

    /// Load or create a new session.
    pub fn load_or_new(dir: &Path) -> Self {
        Self::load(dir).unwrap_or_else(Self::new)
    }

    /// Save session to disk with flock for atomicity.
    pub fn save(&mut self, dir: &Path) -> Result<(), String> {
        self.last_active = now_secs();
        let path = session_path(dir);
        let tmp = dir.join(".session.tmp");

        let file = OpenOptions::new().create(true).write(true).open(&tmp)
            .map_err(|e| format!("session write: {e}"))?;
        let fd = file.as_raw_fd();
        let ret = unsafe { flock(fd, LOCK_EX) };
        if ret != 0 { return Err("session flock failed".into()); }

        let json = self.to_json();
        std::fs::write(&tmp, &json).map_err(|e| format!("session write: {e}"))?;
        unsafe { flock(fd, LOCK_UN) };
        drop(file);
        std::fs::rename(&tmp, &path).map_err(|e| format!("session rename: {e}"))?;
        Ok(())
    }

    /// Record that an entry index was injected (for dedup). O(1).
    pub fn mark_injected(&mut self, idx: u32) {
        self.injected.insert(idx);
    }

    /// Check if an entry index was already injected this session. O(1).
    pub fn was_injected(&self, idx: u32) -> bool {
        self.injected.contains(&idx)
    }

    /// Record a file operation.
    pub fn track_file(&mut self, path: &str, op: FileOp) {
        let now = now_secs();
        // Update existing or append
        if let Some(f) = self.files.iter_mut().find(|f| f.path == path) {
            f.op = op; f.t = now;
        } else {
            self.files.push(FileEntry { path: path.to_string(), op, t: now });
        }
    }

    /// Record a tool invocation. Phase is derived from build state, not tools.
    pub fn record_tool(&mut self, tool: &str) {
        self.tool_seq.push(tool.to_string());
        // Keep sliding window of last 10
        if self.tool_seq.len() > 10 {
            self.tool_seq.remove(0);
        }
        self.phase = self.detect_phase();
    }

    /// Record build result. This is the strongest phase signal.
    pub fn record_build(&mut self, ok: bool, errors: Vec<String>) {
        self.last_build = Some(BuildState { ok, t: now_secs(), errors });
        // Build result immediately updates phase
        self.phase = self.detect_phase();
    }

    /// Add a topic to focus set (deduped).
    pub fn add_focus_topic(&mut self, topic: &str) {
        if !self.focus_topics.iter().any(|t| t == topic) {
            self.focus_topics.push(topic.to_string());
        }
    }

    /// Queue a note for batch storage on session end.
    pub fn queue_note(&mut self, note: String) {
        self.pending_notes.push(note);
    }

    /// Phase detection: build-state-driven with tool-sequence fallback.
    /// Priority: build state (strongest) > tool sequence (heuristic).
    fn detect_phase(&self) -> Phase {
        // If we have a recent build result (within last 5 minutes), it dominates
        let now = now_secs();
        if let Some(bs) = &self.last_build {
            let age = now.saturating_sub(bs.t);
            if age < 300 {
                return if bs.ok { Phase::Verify } else { Phase::Debug };
            }
        }

        // Fallback: tool sequence heuristic
        if self.tool_seq.len() < 3 { return Phase::Unknown; }
        let window = &self.tool_seq[self.tool_seq.len().saturating_sub(6)..];

        let reads = window.iter().filter(|t| *t == "Read" || *t == "Grep" || *t == "Glob").count();
        let edits = window.iter().filter(|t| *t == "Edit" || *t == "Write").count();
        let bashes = window.iter().filter(|t| *t == "Bash").count();

        if bashes >= 2 && edits >= 1 { return Phase::Debug; }
        if bashes >= 1 && edits == 0 { return Phase::Verify; }
        if edits >= 2 { return Phase::Build; }
        if reads >= 3 { return Phase::Research; }
        Phase::Unknown
    }

    // --- Serialization (direct string building, no Value tree) ---

    fn to_json(&self) -> String {
        let mut b = String::with_capacity(1024);
        b.push_str("{\n  \"id\": \"");
        crate::json::escape_into(&self.id, &mut b);
        b.push_str("\",\n  \"started\": ");
        push_u64(&mut b, self.started);
        b.push_str(",\n  \"last_active\": ");
        push_u64(&mut b, self.last_active);
        b.push_str(",\n  \"focus_topics\": [");
        for (i, t) in self.focus_topics.iter().enumerate() {
            if i > 0 { b.push(','); }
            b.push('"');
            crate::json::escape_into(t, &mut b);
            b.push('"');
        }
        b.push_str("],\n  \"phase\": \"");
        b.push_str(self.phase.as_str());
        b.push_str("\",\n  \"files\": [");
        for (i, f) in self.files.iter().enumerate() {
            if i > 0 { b.push(','); }
            b.push_str("{\"path\":\"");
            crate::json::escape_into(&f.path, &mut b);
            b.push_str("\",\"op\":\"");
            b.push_str(f.op.as_str());
            b.push_str("\",\"t\":");
            push_u64(&mut b, f.t);
            b.push('}');
        }
        // Serialize FxHashSet as sorted JSON array for deterministic output
        b.push_str("],\n  \"injected\": [");
        let mut sorted: Vec<u32> = self.injected.iter().copied().collect();
        sorted.sort_unstable();
        for (i, idx) in sorted.iter().enumerate() {
            if i > 0 { b.push(','); }
            crate::text::itoa_push(&mut b, *idx);
        }
        b.push_str("],\n  \"last_build\": ");
        match &self.last_build {
            None => b.push_str("null"),
            Some(bs) => {
                b.push_str("{\"ok\":");
                b.push_str(if bs.ok { "true" } else { "false" });
                b.push_str(",\"t\":");
                push_u64(&mut b, bs.t);
                b.push_str(",\"errors\":[");
                for (i, e) in bs.errors.iter().enumerate() {
                    if i > 0 { b.push(','); }
                    b.push('"');
                    crate::json::escape_into(e, &mut b);
                    b.push('"');
                }
                b.push_str("]}");
            }
        }
        b.push_str(",\n  \"tool_seq\": [");
        for (i, t) in self.tool_seq.iter().enumerate() {
            if i > 0 { b.push(','); }
            b.push('"');
            crate::json::escape_into(t, &mut b);
            b.push('"');
        }
        b.push_str("],\n  \"pending_notes\": [");
        for (i, n) in self.pending_notes.iter().enumerate() {
            if i > 0 { b.push(','); }
            b.push('"');
            crate::json::escape_into(n, &mut b);
            b.push('"');
        }
        b.push_str("]\n}\n");
        b
    }

    fn from_json(val: &crate::json::Value) -> Option<Self> {
        let id = val.get("id")?.as_str()?.to_string();
        let started = val.get("started")?.as_i64()? as u64;
        let last_active = val.get("last_active")?.as_i64()? as u64;

        let focus_topics = match val.get("focus_topics") {
            Some(crate::json::Value::Arr(arr)) => {
                arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
            }
            _ => Vec::new(),
        };

        let phase = val.get("phase").and_then(|v| v.as_str())
            .map(Phase::from_str).unwrap_or(Phase::Unknown);

        let files = match val.get("files") {
            Some(crate::json::Value::Arr(arr)) => {
                arr.iter().filter_map(|v| {
                    let path = v.get("path")?.as_str()?.to_string();
                    let op = FileOp::from_str(v.get("op")?.as_str()?);
                    let t = v.get("t")?.as_i64()? as u64;
                    Some(FileEntry { path, op, t })
                }).collect()
            }
            _ => Vec::new(),
        };

        // Deserialize JSON array → FxHashSet
        let injected = match val.get("injected") {
            Some(crate::json::Value::Arr(arr)) => {
                arr.iter().filter_map(|v| v.as_i64().map(|n| n as u32)).collect()
            }
            _ => crate::fxhash::FxHashSet::default(),
        };

        let last_build = val.get("last_build").and_then(|v| {
            if matches!(v, crate::json::Value::Null) { return None; }
            let ok = match v.get("ok") {
                Some(crate::json::Value::Bool(b)) => *b,
                _ => return None,
            };
            let t = v.get("t")?.as_i64()? as u64;
            let errors = match v.get("errors") {
                Some(crate::json::Value::Arr(arr)) => {
                    arr.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect()
                }
                _ => Vec::new(),
            };
            Some(BuildState { ok, t, errors })
        });

        let tool_seq = match val.get("tool_seq") {
            Some(crate::json::Value::Arr(arr)) => {
                arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
            }
            _ => Vec::new(),
        };

        let pending_notes = match val.get("pending_notes") {
            Some(crate::json::Value::Arr(arr)) => {
                arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
            }
            _ => Vec::new(),
        };

        Some(Session {
            id, started, last_active, focus_topics, phase,
            files, injected, last_build, tool_seq, pending_notes,
        })
    }
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Research => "research",
            Phase::Build => "build",
            Phase::Verify => "verify",
            Phase::Debug => "debug",
            Phase::Unknown => "unknown",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "research" => Phase::Research,
            "build" => Phase::Build,
            "verify" => Phase::Verify,
            "debug" => Phase::Debug,
            _ => Phase::Unknown,
        }
    }

    /// Public phase parser for MCP dispatch.
    pub fn from_str_pub(s: &str) -> Self { Self::from_str(s) }
}

impl FileOp {
    fn as_str(&self) -> &'static str {
        match self {
            FileOp::Read => "read",
            FileOp::Created => "created",
            FileOp::Edited => "edited",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "created" => FileOp::Created,
            "edited" => FileOp::Edited,
            _ => FileOp::Read,
        }
    }
}

fn push_u64(buf: &mut String, n: u64) {
    use std::fmt::Write;
    write!(buf, "{n}").unwrap();
}
