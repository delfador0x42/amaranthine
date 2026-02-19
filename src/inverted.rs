//! Binary inverted index v2: build from data.log, write to index.bin.
//! Layout: [Header][TermTable][Postings][EntryMeta][Snippets]
//!         [TopicTable][TopicNames][SourcePool][XrefTable]

use std::collections::HashMap;
use std::path::Path;
use crate::format::*;

// --- Index Builder ---

struct EntryInfo {
    topic_id: u16,
    word_count: u16,
    snippet: String,
    date_minutes: i32,
    source: String,
    log_offset: u32,
    text_lower: String,
    tags: Vec<String>,
}

pub struct IndexBuilder {
    terms: HashMap<String, Vec<(u32, u16)>>,
    entries: Vec<EntryInfo>,
    topics: Vec<String>,
    total_words: usize,
    tag_freq: HashMap<String, usize>,
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self { terms: HashMap::new(), entries: Vec::new(), topics: Vec::new(), total_words: 0, tag_freq: HashMap::new() }
    }

    pub fn add_topic(&mut self, name: &str) -> u16 {
        if let Some(i) = self.topics.iter().position(|t| t == name) {
            return i as u16;
        }
        let i = self.topics.len() as u16;
        self.topics.push(name.to_string());
        i
    }

    pub fn add_entry(
        &mut self, topic_id: u16, text_lower: &str, snippet: String,
        date_minutes: i32, source: String, log_offset: u32, tags: Vec<String>,
    ) -> u32 {
        let entry_id = self.entries.len() as u32;
        let tokens = crate::text::tokenize(text_lower);
        let wc = tokens.len();
        self.total_words += wc;

        let mut tf_map: HashMap<&str, u16> = HashMap::new();
        for t in &tokens { *tf_map.entry(t.as_str()).or_insert(0) += 1; }

        for (term, tf) in tf_map {
            if term.is_empty() || term.len() < 2 { continue; }
            self.terms.entry(term.to_string()).or_default().push((entry_id, tf));
        }

        for tag in &tags { *self.tag_freq.entry(tag.clone()).or_insert(0) += 1; }

        self.entries.push(EntryInfo {
            topic_id, word_count: wc.min(u16::MAX as usize) as u16,
            snippet, date_minutes, source, log_offset,
            text_lower: text_lower.to_string(), tags,
        });
        entry_id
    }

    fn compute_xrefs(&self) -> Vec<XrefEdge> {
        let mut edges: HashMap<(u16, u16), u16> = HashMap::new();
        let names_lower: Vec<(u16, String)> = self.topics.iter().enumerate()
            .map(|(i, n)| (i as u16, n.to_lowercase())).collect();

        for entry in &self.entries {
            for &(dst, ref name) in &names_lower {
                if dst == entry.topic_id || name.len() < 3 { continue; }
                if entry.text_lower.contains(name.as_str()) {
                    *edges.entry((entry.topic_id, dst)).or_insert(0) += 1;
                }
            }
        }
        edges.into_iter().map(|((s, d), c)| XrefEdge {
            src_topic: s, dst_topic: d, mention_count: c, _pad: 0,
        }).collect()
    }

    pub fn build(&self) -> Vec<u8> {
        let n = self.entries.len() as f64;
        let avgdl = if n == 0.0 { 100.0 } else { self.total_words as f64 / n };
        let num_terms = self.terms.len();
        let table_cap = (num_terms * 4 / 3 + 1).next_power_of_two().max(16);
        let mask = table_cap - 1;

        // Tag bitmap: top 32 tags by frequency
        let tag_to_bit = self.build_tag_map();

        // Posting lists
        let mut post_buf: Vec<Posting> = Vec::new();
        let mut term_entries: Vec<(u64, u32, u32)> = Vec::new();
        for (term, postings) in &self.terms {
            let h = hash_term(term);
            let off = post_buf.len() as u32;
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            let idf_x1000 = (idf * 1000.0) as u32;
            for &(eid, tf) in postings {
                post_buf.push(Posting { entry_id: eid, tf, idf_x1000, _pad: 0 });
            }
            term_entries.push((h, off, postings.len() as u32));
        }

        // Hash table
        let mut table: Vec<TermSlot> = (0..table_cap)
            .map(|_| TermSlot { hash: 0, postings_off: 0, postings_len: 0 }).collect();
        for &(h, off, len) in &term_entries {
            let mut idx = (h as usize) & mask;
            loop {
                if table[idx].hash == 0 {
                    table[idx] = TermSlot { hash: h, postings_off: off, postings_len: len };
                    break;
                }
                idx = (idx + 1) & mask;
            }
        }

        // Snippet pool + source pool + entry metadata
        let mut snippets = Vec::<u8>::new();
        let mut sources = Vec::<u8>::new();
        let mut metas = Vec::<EntryMeta>::new();
        for info in &self.entries {
            let s_off = snippets.len() as u32;
            let sb = info.snippet.as_bytes();
            let s_len = sb.len().min(u16::MAX as usize) as u16;
            snippets.extend_from_slice(&sb[..s_len as usize]);

            let (src_off, src_len) = if info.source.is_empty() {
                (0u32, 0u16)
            } else {
                let o = sources.len() as u32;
                let b = info.source.as_bytes();
                let l = b.len().min(u16::MAX as usize) as u16;
                sources.extend_from_slice(&b[..l as usize]);
                (o, l)
            };

            let tag_bitmap = self.entry_tag_bitmap(&info.tags, &tag_to_bit);
            let confidence = compute_confidence(&info.source, info.date_minutes);
            let epoch_days = if info.date_minutes > 0 {
                (info.date_minutes as u32 / 1440) as u16
            } else { 0 };

            metas.push(EntryMeta {
                topic_id: info.topic_id, word_count: info.word_count,
                snippet_off: s_off, snippet_len: s_len,
                date_minutes: info.date_minutes,
                source_off: src_off, source_len: src_len,
                log_offset: info.log_offset,
                tag_bitmap, confidence, epoch_days, _pad: 0,
            });
        }

        // Topic table + name pool
        let mut tname_pool = Vec::<u8>::new();
        let mut ttable = Vec::<TopicEntry>::new();
        let mut tcounts = vec![0u16; self.topics.len()];
        for e in &self.entries { tcounts[e.topic_id as usize] += 1; }
        for (i, name) in self.topics.iter().enumerate() {
            let off = tname_pool.len() as u32;
            let nb = name.as_bytes();
            let len = nb.len().min(u16::MAX as usize) as u16;
            tname_pool.extend_from_slice(&nb[..len as usize]);
            ttable.push(TopicEntry { name_off: off, name_len: len, entry_count: tcounts[i] });
        }

        // Xrefs
        let xrefs = self.compute_xrefs();

        // Tag names section: [count: u8][len: u8][name]...
        let tag_names_buf = self.build_tag_names(&tag_to_bit);

        // Compute section offsets
        let hdr_sz = std::mem::size_of::<Header>();
        let tab_sz = table_cap * std::mem::size_of::<TermSlot>();
        let post_off = hdr_sz + tab_sz;
        let post_sz = post_buf.len() * std::mem::size_of::<Posting>();
        let meta_off = post_off + post_sz;
        let meta_sz = metas.len() * std::mem::size_of::<EntryMeta>();
        let snip_off = meta_off + meta_sz;
        let top_off = snip_off + snippets.len();
        let top_sz = ttable.len() * std::mem::size_of::<TopicEntry>();
        let tname_off = top_off + top_sz;
        let src_off = tname_off + tname_pool.len();
        let xref_off = src_off + sources.len();
        let xref_sz = xrefs.len() * std::mem::size_of::<XrefEdge>();
        let tagn_off = xref_off + xref_sz;
        let total = tagn_off + tag_names_buf.len();

        let header = Header {
            magic: MAGIC, version: VERSION,
            num_entries: self.entries.len() as u32, num_terms: num_terms as u32,
            num_topics: self.topics.len() as u16, num_xrefs: xrefs.len() as u16,
            table_cap: table_cap as u32, avgdl_x100: (avgdl * 100.0) as u32,
            postings_off: post_off as u32, meta_off: meta_off as u32,
            snippet_off: snip_off as u32, topics_off: top_off as u32,
            topic_names_off: tname_off as u32, source_off: src_off as u32,
            xref_off: xref_off as u32, total_len: total as u32,
            tag_names_off: tagn_off as u32, num_tags: tag_to_bit.len() as u32,
        };

        let mut buf = Vec::with_capacity(total);
        buf.extend_from_slice(as_bytes(&header));
        for s in &table { buf.extend_from_slice(as_bytes(s)); }
        for p in &post_buf { buf.extend_from_slice(as_bytes(p)); }
        for m in &metas { buf.extend_from_slice(as_bytes(m)); }
        buf.extend_from_slice(&snippets);
        for t in &ttable { buf.extend_from_slice(as_bytes(t)); }
        buf.extend_from_slice(&tname_pool);
        buf.extend_from_slice(&sources);
        for x in &xrefs { buf.extend_from_slice(as_bytes(x)); }
        buf.extend_from_slice(&tag_names_buf);
        buf
    }

    fn build_tag_map(&self) -> Vec<(String, u8)> {
        let mut sorted: Vec<_> = self.tag_freq.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        sorted.iter().take(32)
            .enumerate().map(|(i, (name, _))| ((*name).clone(), i as u8)).collect()
    }

    fn entry_tag_bitmap(&self, tags: &[String], tag_map: &[(String, u8)]) -> u32 {
        let mut bitmap = 0u32;
        for tag in tags {
            if let Some((_, bit)) = tag_map.iter().find(|(n, _)| n == tag) {
                bitmap |= 1u32 << *bit;
            }
        }
        bitmap
    }

    fn build_tag_names(&self, tag_map: &[(String, u8)]) -> Vec<u8> {
        let mut buf = vec![tag_map.len() as u8];
        let mut sorted: Vec<_> = tag_map.to_vec();
        sorted.sort_by_key(|(_, bit)| *bit);
        for (name, _) in &sorted {
            let b = name.as_bytes();
            buf.push(b.len().min(255) as u8);
            buf.extend_from_slice(&b[..b.len().min(255)]);
        }
        buf
    }
}

// --- Public functions ---

/// Build index from data.log. Migrates .md → data.log on first run.
pub fn rebuild(dir: &Path) -> Result<String, String> {
    let log_path = crate::datalog::ensure_log(dir)?;
    let log_size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

    // Auto-migrate if data.log is empty (just header)
    if log_size <= 8 {
        let md_files = crate::config::list_topic_files(dir).unwrap_or_default();
        if !md_files.is_empty() {
            let count = crate::datalog::migrate_from_md(dir)?;
            eprintln!("migrated {count} entries from .md → data.log");
        }
    }

    let entries = crate::datalog::iter_live(&log_path)?;
    let mut builder = IndexBuilder::new();

    for e in &entries {
        let tid = builder.add_topic(&e.topic);
        let text_lower = e.body.to_lowercase();
        let source = crate::text::extract_source(&e.body).unwrap_or_default();
        let tags = extract_entry_tags(&e.body);
        let snippet = build_snippet(&e.topic, e.timestamp_min, &e.body);
        builder.add_entry(tid, &text_lower, snippet, e.timestamp_min, source, e.offset, tags);
    }

    let bytes = builder.build();
    let index_path = dir.join("index.bin");
    std::fs::write(&index_path, &bytes).map_err(|e| e.to_string())?;

    let ne = builder.entries.len();
    let nt = builder.terms.len();
    let ntop = builder.topics.len();
    Ok(format!("index v2: {ne} entries, {nt} terms, {ntop} topics, {} bytes",
        bytes.len()))
}

fn extract_entry_tags(body: &str) -> Vec<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')))
        .map(|inner| inner.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default()
}

fn compute_confidence(source: &str, date_minutes: i32) -> u8 {
    if source.is_empty() { return 255; }
    let path = source.split(':').next().unwrap_or(source);
    let file_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return 255,
    };
    let entry_secs = (date_minutes as u64) * 60;
    let entry_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(entry_secs);
    if file_mtime > entry_time { 178 } else { 255 }
}

fn build_snippet(topic: &str, ts_min: i32, body: &str) -> String {
    let date = crate::time::minutes_to_date_str(ts_min);
    let lines: Vec<&str> = body.lines()
        .filter(|l| !l.starts_with("[tags:") && !l.starts_with("[source:")
            && !l.starts_with("[type:") && !l.starts_with("[modified:")
            && !l.trim().is_empty())
        .take(2).collect();
    format!("[{}] {} {}", topic, date, lines.join(" ").chars().take(120).collect::<String>())
}

