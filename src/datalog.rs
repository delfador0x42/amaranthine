//! Append-only data log: primary storage for all entries.
//! Format: LogHeader + sequential EntryRecord/DeleteRecord.
//! Never modified in place. Deletes append tombstones.

use std::io::{Read, Seek, SeekFrom, Write};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub const LOG_MAGIC: [u8; 4] = *b"AMRL";
pub const LOG_VERSION: u32 = 1;
const LOG_HEADER_SIZE: u64 = 8;
const ENTRY_HEADER_SIZE: usize = 12;
const DELETE_RECORD_SIZE: usize = 8;

/// One live entry from the log.
pub struct LogEntry {
    pub offset: u32,
    pub topic: String,
    pub body: String,
    pub timestamp_min: i32,
}

/// Create data.log with header if absent. Returns path.
pub fn ensure_log(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("data.log");
    if path.exists() { return Ok(path); }
    let mut f = File::create(&path).map_err(|e| format!("create data.log: {e}"))?;
    f.write_all(&LOG_MAGIC).map_err(|e| e.to_string())?;
    f.write_all(&LOG_VERSION.to_le_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(path)
}

/// Append one entry. Returns log offset of the written record.
pub fn append_entry(log_path: &Path, topic: &str, body: &str, ts_min: i32) -> Result<u32, String> {
    let mut f = OpenOptions::new().append(true).open(log_path)
        .map_err(|e| format!("open data.log: {e}"))?;
    let offset = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())? as u32;
    let tb = topic.as_bytes();
    let bb = body.as_bytes();
    let hdr: [u8; ENTRY_HEADER_SIZE] = entry_header(tb.len() as u8, bb.len() as u32, ts_min);
    f.write_all(&hdr).map_err(|e| e.to_string())?;
    f.write_all(tb).map_err(|e| e.to_string())?;
    f.write_all(bb).map_err(|e| e.to_string())?;
    f.sync_data().map_err(|e| e.to_string())?;
    Ok(offset)
}

/// Append a delete tombstone referencing target entry offset.
pub fn append_delete(log_path: &Path, target_offset: u32) -> Result<(), String> {
    let mut f = OpenOptions::new().append(true).open(log_path)
        .map_err(|e| format!("open data.log: {e}"))?;
    let mut rec = [0u8; DELETE_RECORD_SIZE];
    rec[0] = 0x02;
    rec[4..8].copy_from_slice(&target_offset.to_le_bytes());
    f.write_all(&rec).map_err(|e| e.to_string())?;
    f.sync_data().map_err(|e| e.to_string())?;
    Ok(())
}

/// Read a single entry from log at given offset.
pub fn read_entry(log_path: &Path, offset: u32) -> Result<LogEntry, String> {
    let mut f = File::open(log_path).map_err(|e| format!("open data.log: {e}"))?;
    read_entry_from(&mut f, offset)
}

/// Read a single entry from an already-open file handle (avoids re-open per call).
pub fn read_entry_from(f: &mut File, offset: u32) -> Result<LogEntry, String> {
    f.seek(SeekFrom::Start(offset as u64)).map_err(|e| e.to_string())?;
    let mut hdr = [0u8; ENTRY_HEADER_SIZE];
    f.read_exact(&mut hdr).map_err(|e| format!("read entry header: {e}"))?;
    if hdr[0] != 0x01 { return Err("not an entry record".into()); }
    let topic_len = hdr[1] as usize;
    let body_len = u32::from_le_bytes([hdr[2], hdr[3], hdr[4], hdr[5]]) as usize;
    let ts_min = i32::from_le_bytes([hdr[6], hdr[7], hdr[8], hdr[9]]);
    let mut topic_buf = vec![0u8; topic_len];
    f.read_exact(&mut topic_buf).map_err(|e| e.to_string())?;
    let mut body_buf = vec![0u8; body_len];
    f.read_exact(&mut body_buf).map_err(|e| e.to_string())?;
    Ok(LogEntry {
        offset,
        topic: String::from_utf8_lossy(&topic_buf).into(),
        body: String::from_utf8_lossy(&body_buf).into(),
        timestamp_min: ts_min,
    })
}

/// Iterate all live entries (skipping tombstoned ones).
/// Single-pass: collects entries and deleted offsets simultaneously, then filters.
pub fn iter_live(log_path: &Path) -> Result<Vec<LogEntry>, String> {
    let data = fs::read(log_path).map_err(|e| format!("read data.log: {e}"))?;
    if data.len() < LOG_HEADER_SIZE as usize { return Err("data.log too small".into()); }
    if data[..4] != LOG_MAGIC { return Err("bad data.log magic".into()); }

    let mut entries = Vec::new();
    let mut deleted = crate::fxhash::FxHashSet::default();
    let mut pos = LOG_HEADER_SIZE as usize;

    while pos < data.len() {
        match data[pos] {
            0x01 => {
                let offset = pos as u32;
                if pos + ENTRY_HEADER_SIZE > data.len() { break; }
                let tl = data[pos + 1] as usize;
                let bl = u32::from_le_bytes([
                    data[pos+2], data[pos+3], data[pos+4], data[pos+5]
                ]) as usize;
                let ts = i32::from_le_bytes([
                    data[pos+6], data[pos+7], data[pos+8], data[pos+9]
                ]);
                let rec_end = pos + ENTRY_HEADER_SIZE + tl + bl;
                if rec_end > data.len() { break; }
                let topic = String::from_utf8_lossy(
                    &data[pos+ENTRY_HEADER_SIZE..pos+ENTRY_HEADER_SIZE+tl]
                ).into();
                let body = String::from_utf8_lossy(
                    &data[pos+ENTRY_HEADER_SIZE+tl..rec_end]
                ).into();
                entries.push(LogEntry { offset, topic, body, timestamp_min: ts });
                pos = rec_end;
            }
            0x02 => {
                if pos + DELETE_RECORD_SIZE > data.len() { break; }
                let target = u32::from_le_bytes([
                    data[pos+4], data[pos+5], data[pos+6], data[pos+7]
                ]);
                deleted.insert(target);
                pos += DELETE_RECORD_SIZE;
            }
            _ => break,
        }
    }

    if !deleted.is_empty() {
        entries.retain(|e| !deleted.contains(&e.offset));
    }
    Ok(entries)
}

/// Migrate .md files into data.log. Returns entry count.
pub fn migrate_from_md(dir: &Path) -> Result<usize, String> {
    let log_path = ensure_log(dir)?;
    let files = crate::config::list_topic_files(dir)?;
    let mut count = 0;
    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sections = crate::delete::split_sections(&content);
        for (header, body) in &sections {
            let ts_str = header.strip_prefix("## ").unwrap_or("");
            let ts_min = crate::time::parse_date_minutes(ts_str).unwrap_or(0) as i32;
            let body_text = body.strip_prefix('\n').unwrap_or(body).trim_end();
            append_entry(&log_path, &name, body_text, ts_min)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Compact: rewrite data.log without deleted entries.
pub fn compact_log(dir: &Path) -> Result<String, String> {
    let log_path = dir.join("data.log");
    let entries = iter_live(&log_path)?;
    let before = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    // Write to tmp, rename over
    let tmp = dir.join("data.log.tmp");
    {
        let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&LOG_MAGIC).map_err(|e| e.to_string())?;
        f.write_all(&LOG_VERSION.to_le_bytes()).map_err(|e| e.to_string())?;
        for e in &entries {
            let tb = e.topic.as_bytes();
            let bb = e.body.as_bytes();
            let hdr = entry_header(tb.len() as u8, bb.len() as u32, e.timestamp_min);
            f.write_all(&hdr).map_err(|e| e.to_string())?;
            f.write_all(tb).map_err(|e| e.to_string())?;
            f.write_all(bb).map_err(|e| e.to_string())?;
        }
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, &log_path).map_err(|e| e.to_string())?;
    let after = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    Ok(format!("compacted: {} entries, {} → {} bytes", entries.len(), before, after))
}

/// Append one entry to an already-open file handle (no fsync). For batch writes.
pub fn append_entry_to(f: &mut File, topic: &str, body: &str, ts_min: i32) -> Result<u32, String> {
    let offset = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())? as u32;
    let tb = topic.as_bytes();
    let bb = body.as_bytes();
    let hdr: [u8; ENTRY_HEADER_SIZE] = entry_header(tb.len() as u8, bb.len() as u32, ts_min);
    f.write_all(&hdr).map_err(|e| e.to_string())?;
    f.write_all(tb).map_err(|e| e.to_string())?;
    f.write_all(bb).map_err(|e| e.to_string())?;
    Ok(offset)
}

fn entry_header(topic_len: u8, body_len: u32, ts_min: i32) -> [u8; ENTRY_HEADER_SIZE] {
    let mut h = [0u8; ENTRY_HEADER_SIZE];
    h[0] = 0x01;
    h[1] = topic_len;
    h[2..6].copy_from_slice(&body_len.to_le_bytes());
    h[6..10].copy_from_slice(&ts_min.to_le_bytes());
    // h[10..12] = pad (zeros)
    h
}
