//! Query engine for the binary inverted index.
//! Designed for mmap'd access: ~200ns per query.
//! All reads are pointer arithmetic on a &[u8] slice.

use crate::inverted::*;

/// Query the binary index. Returns formatted results.
pub fn search(data: &[u8], query: &str, limit: usize) -> Result<String, String> {
    if data.len() < std::mem::size_of::<Header>() {
        return Err("index too small".into());
    }
    let hdr = read_header(data)?;
    let terms = query_terms(query);
    if terms.is_empty() { return Err("empty query".into()); }

    // Copy packed fields to locals (avoids unaligned access UB)
    let num_entries = { hdr.num_entries } as usize;
    let table_cap = { hdr.table_cap } as usize;
    let avgdl = { hdr.avgdl_x100 } as f64 / 100.0;
    let postings_off = { hdr.postings_off } as usize;
    let meta_off = { hdr.meta_off } as usize;
    let snippet_off = { hdr.snippet_off } as usize;
    let mask = table_cap - 1;

    let mut scores: Vec<f64> = vec![0.0; num_entries];
    let mut matched: Vec<bool> = vec![false; num_entries];
    let mut any_hit = false;

    for term in &terms {
        let h = hash_term(term);
        let mut idx = (h as usize) & mask;

        for _ in 0..table_cap {
            let slot = read_slot(data, idx)?;
            let sh = { slot.hash };
            if sh == 0 { break; }
            if sh == h {
                any_hit = true;
                let p_off = { slot.postings_off } as usize;
                let p_len = { slot.postings_len } as usize;
                let base = postings_off + p_off * std::mem::size_of::<Posting>();
                for i in 0..p_len {
                    let p = read_at::<Posting>(data, base + i * std::mem::size_of::<Posting>())?;
                    let eid = { p.entry_id } as usize;
                    if eid >= num_entries { continue; }
                    let m = read_at::<EntryMeta>(data, meta_off + eid * std::mem::size_of::<EntryMeta>())?;
                    let doc_len = { m.word_count } as f64;
                    let idf = { p.idf_x1000 } as f64 / 1000.0;
                    let tf = { p.tf } as f64;
                    let len_norm = 1.0 - 0.75 + 0.75 * doc_len / avgdl.max(1.0);
                    let tf_sat = (tf * 2.2) / (tf + 1.2 * len_norm);
                    scores[eid] += idf * tf_sat;
                    matched[eid] = true;
                }
                break;
            }
            idx = (idx + 1) & mask;
        }
    }

    if !any_hit {
        return Ok(format!("0 matches for '{query}'"));
    }

    // Top-K by score
    let mut results: Vec<(usize, f64)> = scores.iter().enumerate()
        .filter(|(i, _)| matched[*i])
        .map(|(i, s)| (i, *s))
        .collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    let mut out = String::new();
    for (eid, _) in &results {
        let m = read_at::<EntryMeta>(data, meta_off + eid * std::mem::size_of::<EntryMeta>())?;
        let s_off = snippet_off + { m.snippet_off } as usize;
        let s_len = { m.snippet_len } as usize;
        if s_off + s_len <= data.len() {
            if let Ok(s) = std::str::from_utf8(&data[s_off..s_off + s_len]) {
                out.push_str("  ");
                out.push_str(s);
                out.push('\n');
            }
        }
    }
    let total = results.len();
    out.push_str(&format!("{total} match(es) [index]\n"));
    Ok(out)
}

/// Number of entries in the index.
pub fn entry_count(data: &[u8]) -> Result<usize, String> {
    let hdr = read_header(data)?;
    Ok({ hdr.num_entries } as usize)
}

/// Stats about the loaded index.
pub fn index_info(data: &[u8]) -> Result<String, String> {
    let hdr = read_header(data)?;
    let ne = { hdr.num_entries };
    let nt = { hdr.num_terms };
    let tc = { hdr.table_cap };
    let ad = { hdr.avgdl_x100 } as f64 / 100.0;
    let tl = { hdr.total_len };
    Ok(format!("index: {ne} entries, {nt} terms, table_cap={tc}, avgdl={ad:.1}, {tl} bytes"))
}

// --- Low-level readers ---

fn read_header(data: &[u8]) -> Result<Header, String> {
    let hdr: Header = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const Header) };
    if hdr.magic != MAGIC { return Err("bad index magic".into()); }
    let v = { hdr.version };
    if v != VERSION { return Err(format!("index version {v} != {VERSION}")); }
    Ok(hdr)
}

fn read_slot(data: &[u8], idx: usize) -> Result<TermSlot, String> {
    let off = std::mem::size_of::<Header>() + idx * std::mem::size_of::<TermSlot>();
    read_at::<TermSlot>(data, off)
}

fn read_at<T: Copy>(data: &[u8], off: usize) -> Result<T, String> {
    if off + std::mem::size_of::<T>() > data.len() {
        return Err("read out of bounds".into());
    }
    Ok(unsafe { std::ptr::read_unaligned(data.as_ptr().add(off) as *const T) })
}

fn query_terms(query: &str) -> Vec<String> {
    query.split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

// --- Zero-alloc search path ---

/// Result struct for zero-alloc search. C-compatible.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct RawResult {
    pub entry_id: u16,
    pub score_x1000: u32,
}

/// Reusable query state. Pre-allocated, never freed between queries.
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
        if self.entry_gen.len() < n {
            self.entry_gen.resize(n, 0);
            self.scores.resize(n, 0.0);
        }
    }
}

/// Zero-alloc search with pre-hashed terms. Writes results to `out`.
/// Returns number of results written. No heap allocation on hot path.
pub fn search_raw(
    data: &[u8], hashes: &[u64], state: &mut QueryState, out: &mut [RawResult],
) -> Result<usize, String> {
    let hdr = read_header(data)?;
    let num_entries = { hdr.num_entries } as usize;
    let table_cap = { hdr.table_cap } as usize;
    let avgdl = { hdr.avgdl_x100 } as f64 / 100.0;
    let postings_off = { hdr.postings_off } as usize;
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
                let base = postings_off + p_off * std::mem::size_of::<Posting>();
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

    // Insertion-sort top-K into output buffer
    let limit = out.len();
    let mut n = 0usize;
    for eid in 0..num_entries {
        if state.entry_gen[eid] != gen { continue; }
        let s = (state.scores[eid] * 1000.0) as u32;
        if n < limit {
            let mut pos = n;
            while pos > 0 && out[pos - 1].score_x1000 < s {
                out[pos] = out[pos - 1];
                pos -= 1;
            }
            out[pos] = RawResult { entry_id: eid as u16, score_x1000: s };
            n += 1;
        } else if s > out[n - 1].score_x1000 {
            let mut pos = n - 1;
            while pos > 0 && out[pos - 1].score_x1000 < s {
                out[pos] = out[pos - 1];
                pos -= 1;
            }
            out[pos] = RawResult { entry_id: eid as u16, score_x1000: s };
        }
    }
    Ok(n)
}

/// Get snippet for an entry_id. Returns (ptr, len) into index data.
/// Valid until the data slice is freed/reloaded.
pub fn snippet<'a>(data: &'a [u8], entry_id: u16) -> Option<&'a str> {
    let hdr = read_header(data).ok()?;
    let num_entries = { hdr.num_entries } as usize;
    let meta_off = { hdr.meta_off } as usize;
    let snippet_off = { hdr.snippet_off } as usize;
    let eid = entry_id as usize;
    if eid >= num_entries { return None; }
    let m = read_at::<EntryMeta>(data, meta_off + eid * std::mem::size_of::<EntryMeta>()).ok()?;
    let s_off = snippet_off + { m.snippet_off } as usize;
    let s_len = { m.snippet_len } as usize;
    if s_off + s_len > data.len() { return None; }
    std::str::from_utf8(&data[s_off..s_off + s_len]).ok()
}
