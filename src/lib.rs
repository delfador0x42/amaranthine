//! Amaranthine library — shared modules + C FFI for direct in-process query.
//!
//! Binary: uses modules via `use amaranthine::*`
//! C/FFI: links libamaranthine.dylib, queries index at ~200ns

pub mod binquery;
pub mod briefing;
pub mod cache;
pub mod callgraph;
pub mod cffi;
pub mod codepath;
pub mod compact;
pub mod compress;
pub mod config;
pub mod context;
pub mod datalog;
pub mod depgraph;
pub mod delete;
pub mod digest;
pub mod edit;
pub mod export;
pub mod format;
pub mod fxhash;
pub mod hook;
pub mod install;
pub mod intern;
pub mod inverted;
pub mod json;
pub mod lock;
pub mod mcp;
pub mod migrate;
pub mod prune;
pub mod reconstruct;
pub mod score;
pub mod search;
pub mod stats;
pub mod store;
pub mod text;
pub mod time;
pub mod topics;
pub mod xref;

// --- C FFI: direct in-process query, no MCP overhead ---

use std::ffi::{c_char, CStr, CString};
use std::time::SystemTime;

/// Opaque handle holding loaded index data + reusable query state.
pub struct AmrIndex {
    data: Vec<u8>,
    path: String,
    mtime: SystemTime,
    state: cffi::QueryState,
}

/// C-compatible result from zero-alloc search.
pub use cffi::RawResult as AmrResult;

/// Open an index file, load into memory. Returns null on failure.
#[no_mangle]
pub extern "C" fn amr_open(path: *const c_char) -> *mut AmrIndex {
    if path.is_null() { return std::ptr::null_mut(); }
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let data = match std::fs::read(path_str) {
        Ok(d) => d,
        Err(_) => return std::ptr::null_mut(),
    };
    let mtime = std::fs::metadata(path_str)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let num_entries = binquery::entry_count(&data).unwrap_or(0);
    let state = cffi::QueryState::new(num_entries);
    Box::into_raw(Box::new(AmrIndex { data, path: path_str.into(), mtime, state }))
}

/// Search the index. Caller must free result with amr_free_str.
#[no_mangle]
pub extern "C" fn amr_search(idx: *const AmrIndex, query: *const c_char, limit: u32) -> *mut c_char {
    if idx.is_null() || query.is_null() { return std::ptr::null_mut(); }
    let h = unsafe { &*idx };
    let q = match unsafe { CStr::from_ptr(query) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let result = match binquery::search(&h.data, q, limit as usize) {
        Ok(r) => r,
        Err(e) => format!("error: {e}"),
    };
    CString::new(result).map(|c| c.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Get index info string. Caller must free with amr_free_str.
#[no_mangle]
pub extern "C" fn amr_info(idx: *const AmrIndex) -> *mut c_char {
    if idx.is_null() { return std::ptr::null_mut(); }
    let h = unsafe { &*idx };
    let result = match binquery::index_info(&h.data) {
        Ok(r) => r,
        Err(e) => format!("error: {e}"),
    };
    CString::new(result).map(|c| c.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Check if index file changed. Returns 1=stale, 0=fresh, -1=error.
#[no_mangle]
pub extern "C" fn amr_is_stale(idx: *const AmrIndex) -> i32 {
    if idx.is_null() { return -1; }
    let h = unsafe { &*idx };
    match std::fs::metadata(&h.path).and_then(|m| m.modified()) {
        Ok(m) => if m != h.mtime { 1 } else { 0 },
        Err(_) => -1,
    }
}

/// Reload index from disk. Returns 0=success, -1=failure.
#[no_mangle]
pub extern "C" fn amr_reload(idx: *mut AmrIndex) -> i32 {
    if idx.is_null() { return -1; }
    let h = unsafe { &mut *idx };
    match std::fs::read(&h.path) {
        Ok(data) => {
            h.mtime = std::fs::metadata(&h.path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let n = binquery::entry_count(&data).unwrap_or(0);
            h.state = cffi::QueryState::new(n);
            h.data = data;
            0
        }
        Err(_) => -1,
    }
}

/// Free a string returned by amr_search or amr_info.
#[no_mangle]
pub extern "C" fn amr_free_str(s: *mut c_char) {
    if !s.is_null() { unsafe { drop(CString::from_raw(s)); } }
}

/// Close and free the index handle.
#[no_mangle]
pub extern "C" fn amr_close(idx: *mut AmrIndex) {
    if !idx.is_null() { unsafe { drop(Box::from_raw(idx)); } }
}

// --- Zero-alloc path: ~100-200ns per query ---

/// Hash a term for use with amr_search_raw. Caller caches the hash.
#[no_mangle]
pub extern "C" fn amr_hash(term: *const c_char) -> u64 {
    if term.is_null() { return 0; }
    let s = match unsafe { CStr::from_ptr(term) }.to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    format::hash_term(&s.to_lowercase())
}

/// Zero-alloc search with pre-hashed terms. Writes into caller's buffer.
/// Returns number of results written. No heap allocation on hot path.
#[no_mangle]
pub extern "C" fn amr_search_raw(
    idx: *mut AmrIndex, hashes: *const u64, nhashes: u32,
    out: *mut AmrResult, limit: u32,
) -> u32 {
    if idx.is_null() || hashes.is_null() || out.is_null() || limit == 0 { return 0; }
    let h = unsafe { &mut *idx };
    let hash_slice = unsafe { std::slice::from_raw_parts(hashes, nhashes as usize) };
    let out_slice = unsafe { std::slice::from_raw_parts_mut(out, limit as usize) };
    cffi::search_raw(&h.data, hash_slice, &mut h.state, out_slice).unwrap_or(0) as u32
}

/// Get snippet for an entry_id. Returns pointer + length into index data.
/// Valid until amr_reload or amr_close. Do NOT free the pointer.
#[no_mangle]
pub extern "C" fn amr_snippet(
    idx: *const AmrIndex, entry_id: u32, out_len: *mut u32,
) -> *const u8 {
    if idx.is_null() { return std::ptr::null(); }
    let h = unsafe { &*idx };
    match cffi::snippet_u32(&h.data, entry_id) {
        Some(s) => {
            if !out_len.is_null() { unsafe { *out_len = s.len() as u32; } }
            s.as_ptr()
        }
        None => {
            if !out_len.is_null() { unsafe { *out_len = 0; } }
            std::ptr::null()
        }
    }
}
