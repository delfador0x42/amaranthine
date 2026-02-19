//! Query engine for the binary inverted index v2.
//! All reads are pointer arithmetic on a &[u8] slice.

use crate::format::*;

// --- Formatted search (MCP path) ---

pub fn search(data: &[u8], query: &str, limit: usize) -> Result<String, String> {
    let hits = search_v2(data, query, limit)?;
    if hits.is_empty() { return Ok(format!("0 matches for '{query}'")); }
    let mut out = String::new();
    for h in &hits {
        out.push_str("  ");
        out.push_str(&h.snippet);
        out.push('\n');
    }
    out.push_str(&format!("{} match(es) [index]\n", hits.len()));
    Ok(out)
}

// --- Structured search ---

pub struct SearchHit {
    pub entry_id: u32,
    pub topic_id: u16,
    pub score: f64,
    pub snippet: String,
    pub date_minutes: i32,
    pub log_offset: u32,
}

pub fn search_v2(data: &[u8], query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let hdr = read_header(data)?;
    let terms = crate::text::query_terms(query);
    if terms.is_empty() { return Err("empty query".into()); }

    let num_entries = { hdr.num_entries } as usize;
    let table_cap = { hdr.table_cap } as usize;
    let avgdl = { hdr.avgdl_x100 } as f64 / 100.0;
    let post_off = { hdr.postings_off } as usize;
    let meta_off = { hdr.meta_off } as usize;
    let snip_off = { hdr.snippet_off } as usize;
    let mask = table_cap - 1;

    let mut scores = vec![0.0f64; num_entries];
    let mut matched = vec![false; num_entries];

    for term in &terms {
        let h = hash_term(term);
        let mut idx = (h as usize) & mask;
        for _ in 0..table_cap {
            let slot = read_slot(data, idx)?;
            let sh = { slot.hash };
            if sh == 0 { break; }
            if sh == h {
                let p_off = { slot.postings_off } as usize;
                let p_len = { slot.postings_len } as usize;
                let base = post_off + p_off * std::mem::size_of::<Posting>();
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

    let mut results: Vec<(usize, f64)> = scores.iter().enumerate()
        .filter(|(i, _)| matched[*i]).map(|(i, s)| (i, *s)).collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    let mut hits = Vec::new();
    for (eid, score) in results {
        let m = read_at::<EntryMeta>(data, meta_off + eid * std::mem::size_of::<EntryMeta>())?;
        let s_off = snip_off + { m.snippet_off } as usize;
        let s_len = { m.snippet_len } as usize;
        let snippet = if s_off + s_len <= data.len() {
            std::str::from_utf8(&data[s_off..s_off + s_len]).unwrap_or("").to_string()
        } else { String::new() };
        hits.push(SearchHit {
            entry_id: eid as u32, topic_id: { m.topic_id }, score,
            snippet, date_minutes: { m.date_minutes },
            log_offset: { m.log_offset },
        });
    }
    Ok(hits)
}

// --- V2 section readers ---

pub fn topic_table(data: &[u8]) -> Result<Vec<(u16, String, u16)>, String> {
    let hdr = read_header(data)?;
    let top_off = { hdr.topics_off } as usize;
    let tname_off = { hdr.topic_names_off } as usize;
    let n = { hdr.num_topics } as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let te = read_at::<TopicEntry>(data, top_off + i * std::mem::size_of::<TopicEntry>())?;
        let no = tname_off + { te.name_off } as usize;
        let nl = { te.name_len } as usize;
        let name = if no + nl <= data.len() {
            std::str::from_utf8(&data[no..no + nl]).unwrap_or("?").to_string()
        } else { "?".into() };
        out.push((i as u16, name, { te.entry_count }));
    }
    Ok(out)
}

pub fn topic_name(data: &[u8], topic_id: u16) -> Result<String, String> {
    let hdr = read_header(data)?;
    let top_off = { hdr.topics_off } as usize;
    let tname_off = { hdr.topic_names_off } as usize;
    let n = { hdr.num_topics } as usize;
    if topic_id as usize >= n { return Err("topic_id out of range".into()); }
    let te = read_at::<TopicEntry>(data, top_off + topic_id as usize * std::mem::size_of::<TopicEntry>())?;
    let no = tname_off + { te.name_off } as usize;
    let nl = { te.name_len } as usize;
    if no + nl > data.len() { return Err("name out of bounds".into()); }
    Ok(std::str::from_utf8(&data[no..no + nl]).unwrap_or("?").to_string())
}

pub fn xref_edges(data: &[u8]) -> Result<Vec<(u16, u16, u16)>, String> {
    let hdr = read_header(data)?;
    let off = { hdr.xref_off } as usize;
    let n = { hdr.num_xrefs } as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let x = read_at::<XrefEdge>(data, off + i * std::mem::size_of::<XrefEdge>())?;
        out.push(({ x.src_topic }, { x.dst_topic }, { x.mention_count }));
    }
    Ok(out)
}

pub struct SourcedHit {
    pub entry_id: u32,
    pub topic_id: u16,
    pub source_path: String,
    pub date_minutes: i32,
}

pub fn sourced_entries(data: &[u8]) -> Result<Vec<SourcedHit>, String> {
    let hdr = read_header(data)?;
    let meta_off = { hdr.meta_off } as usize;
    let src_off = { hdr.source_off } as usize;
    let n = { hdr.num_entries } as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let m = read_at::<EntryMeta>(data, meta_off + i * std::mem::size_of::<EntryMeta>())?;
        let sl = { m.source_len } as usize;
        if sl == 0 { continue; }
        let so = src_off + { m.source_off } as usize;
        if so + sl > data.len() { continue; }
        let path = std::str::from_utf8(&data[so..so + sl]).unwrap_or("").to_string();
        out.push(SourcedHit {
            entry_id: i as u32, topic_id: { m.topic_id },
            source_path: path, date_minutes: { m.date_minutes },
        });
    }
    Ok(out)
}

pub fn entry_log_offset(data: &[u8], entry_id: u32) -> Result<u32, String> {
    let hdr = read_header(data)?;
    let meta_off = { hdr.meta_off } as usize;
    let n = { hdr.num_entries } as usize;
    if entry_id as usize >= n { return Err("entry_id out of range".into()); }
    let m = read_at::<EntryMeta>(data, meta_off + entry_id as usize * std::mem::size_of::<EntryMeta>())?;
    Ok(m.log_offset)
}

pub fn entries_for_topic(data: &[u8], topic_id: u16) -> Result<Vec<u32>, String> {
    let hdr = read_header(data)?;
    let meta_off = { hdr.meta_off } as usize;
    let n = { hdr.num_entries } as usize;
    let mut entries: Vec<(u32, i32)> = Vec::new();
    for i in 0..n {
        let m = read_at::<EntryMeta>(data, meta_off + i * std::mem::size_of::<EntryMeta>())?;
        if { m.topic_id } == topic_id {
            entries.push((i as u32, { m.date_minutes }));
        }
    }
    entries.sort_by_key(|&(_, d)| d);
    Ok(entries.into_iter().map(|(id, _)| id).collect())
}

pub fn find_topic_id(data: &[u8], name: &str) -> Result<u16, String> {
    let topics = topic_table(data)?;
    topics.iter().find(|(_, n, _)| n == name)
        .map(|(id, _, _)| *id)
        .ok_or_else(|| format!("topic '{}' not found in index", name))
}

pub fn index_version(data: &[u8]) -> Result<u32, String> {
    if data.len() < 8 { return Err("too small".into()); }
    let v = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    Ok(v)
}

pub fn entry_count(data: &[u8]) -> Result<usize, String> {
    let hdr = read_header(data)?;
    Ok({ hdr.num_entries } as usize)
}

pub fn index_info(data: &[u8]) -> Result<String, String> {
    let hdr = read_header(data)?;
    let ne = { hdr.num_entries };
    let nt = { hdr.num_terms };
    let tc = { hdr.table_cap };
    let ad = { hdr.avgdl_x100 } as f64 / 100.0;
    let tl = { hdr.total_len };
    let ntop = { hdr.num_topics };
    let nxr = { hdr.num_xrefs };
    Ok(format!("index v2: {ne} entries, {nt} terms, {ntop} topics, {nxr} xrefs, table_cap={tc}, avgdl={ad:.1}, {tl} bytes"))
}

// --- Low-level readers (pub for cffi.rs) ---

pub fn read_header(data: &[u8]) -> Result<Header, String> {
    if data.len() < std::mem::size_of::<Header>() { return Err("index too small".into()); }
    let hdr: Header = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const Header) };
    if hdr.magic != MAGIC { return Err("bad index magic".into()); }
    let v = { hdr.version };
    if v != VERSION { return Err(format!("index version {v} != {VERSION} — run rebuild_index")); }
    Ok(hdr)
}

pub fn read_slot(data: &[u8], idx: usize) -> Result<TermSlot, String> {
    let off = std::mem::size_of::<Header>() + idx * std::mem::size_of::<TermSlot>();
    read_at::<TermSlot>(data, off)
}

pub fn read_at<T: Copy>(data: &[u8], off: usize) -> Result<T, String> {
    if off + std::mem::size_of::<T>() > data.len() { return Err("read out of bounds".into()); }
    Ok(unsafe { std::ptr::read_unaligned(data.as_ptr().add(off) as *const T) })
}
