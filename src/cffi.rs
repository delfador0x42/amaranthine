//! C FFI zero-alloc query path: ~100-200ns per query.
//! Used by lib.rs C extern functions. MCP path uses binquery.rs instead.

use crate::format::*;
use crate::binquery::{read_header, read_slot, read_at};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct RawResult {
    pub entry_id: u32,
    pub score_x1000: u32,
}

pub struct QueryState {
    pub generation: u32,
    pub entry_gen: Vec<u32>,
    pub scores: Vec<f64>,
}

impl QueryState {
    pub fn new(num_entries: usize) -> Self {
        Self { generation: 0, entry_gen: vec![0; num_entries], scores: vec![0.0; num_entries] }
    }
    fn ensure(&mut self, n: usize) {
        if self.entry_gen.len() < n { self.entry_gen.resize(n, 0); self.scores.resize(n, 0.0); }
    }
}

pub fn search_raw(
    data: &[u8], hashes: &[u64], state: &mut QueryState, out: &mut [RawResult],
) -> Result<usize, String> {
    let hdr = read_header(data)?;
    let num_entries = { hdr.num_entries } as usize;
    let table_cap = { hdr.table_cap } as usize;
    let avgdl = { hdr.avgdl_x100 } as f64 / 100.0;
    let post_off = { hdr.postings_off } as usize;
    let meta_off = { hdr.meta_off } as usize;
    let mask = table_cap - 1;

    state.ensure(num_entries);
    state.generation = state.generation.wrapping_add(1);
    if state.generation == 0 { state.generation = 1; }
    let gen = state.generation;

    let mut any_hit = false;
    for &h in hashes {
        let mut idx = (h as usize) & mask;
        for _ in 0..table_cap {
            let slot = read_slot(data, idx)?;
            let sh = { slot.hash };
            if sh == 0 { break; }
            if sh == h {
                any_hit = true;
                let p_off = { slot.postings_off } as usize;
                let p_len = { slot.postings_len } as usize;
                let base = post_off + p_off * std::mem::size_of::<Posting>();
                for i in 0..p_len {
                    let p = read_at::<Posting>(data, base + i * std::mem::size_of::<Posting>())?;
                    let eid = { p.entry_id } as usize;
                    if eid >= num_entries { continue; }
                    if state.entry_gen[eid] != gen {
                        state.scores[eid] = 0.0;
                        state.entry_gen[eid] = gen;
                    }
                    let m = read_at::<EntryMeta>(data, meta_off + eid * std::mem::size_of::<EntryMeta>())?;
                    let doc_len = { m.word_count } as f64;
                    let idf = { p.idf_x1000 } as f64 / 1000.0;
                    let tf = { p.tf } as f64;
                    let len_norm = 1.0 - 0.75 + 0.75 * doc_len / avgdl.max(1.0);
                    let tf_sat = (tf * 2.2) / (tf + 1.2 * len_norm);
                    state.scores[eid] += idf * tf_sat;
                }
                break;
            }
            idx = (idx + 1) & mask;
        }
    }
    if !any_hit { return Ok(0); }

    let limit = out.len();
    let mut n = 0usize;
    for eid in 0..num_entries {
        if state.entry_gen[eid] != gen { continue; }
        let s = (state.scores[eid] * 1000.0) as u32;
        if n < limit {
            let mut pos = n;
            while pos > 0 && out[pos - 1].score_x1000 < s { out[pos] = out[pos - 1]; pos -= 1; }
            out[pos] = RawResult { entry_id: eid as u32, score_x1000: s };
            n += 1;
        } else if s > out[n - 1].score_x1000 {
            let mut pos = n - 1;
            while pos > 0 && out[pos - 1].score_x1000 < s { out[pos] = out[pos - 1]; pos -= 1; }
            out[pos] = RawResult { entry_id: eid as u32, score_x1000: s };
        }
    }
    Ok(n)
}

pub fn snippet(data: &[u8], entry_id: u16) -> Option<&str> {
    snippet_u32(data, entry_id as u32)
}

pub fn snippet_u32(data: &[u8], entry_id: u32) -> Option<&str> {
    let hdr = read_header(data).ok()?;
    let n = { hdr.num_entries } as usize;
    let meta_off = { hdr.meta_off } as usize;
    let snip_off = { hdr.snippet_off } as usize;
    if entry_id as usize >= n { return None; }
    let m = read_at::<EntryMeta>(data, meta_off + entry_id as usize * std::mem::size_of::<EntryMeta>()).ok()?;
    let s_off = snip_off + { m.snippet_off } as usize;
    let s_len = { m.snippet_len } as usize;
    if s_off + s_len > data.len() { return None; }
    std::str::from_utf8(&data[s_off..s_off + s_len]).ok()
}
