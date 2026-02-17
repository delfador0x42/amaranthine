//! Binary inverted index: build from corpus, write to file.
//! Format designed for mmap'd zero-copy query at ~200ns.
//!
//! Layout: [Header][TermTable][PostingLists][EntryMeta][SnippetPool]

use std::collections::HashMap;
use std::path::Path;

// Magic bytes: "AMRN" + version
pub const MAGIC: [u8; 4] = [b'A', b'M', b'R', b'N'];
pub const VERSION: u32 = 1;

/// Fixed-size header at offset 0.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct Header {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_entries: u32,
    pub num_terms: u32,
    pub table_cap: u32,    // hash table capacity (power of 2)
    pub avgdl_x100: u32,   // avgdl * 100 (fixed-point, avoids f32 in header)
    pub postings_off: u32,  // byte offset to posting lists
    pub meta_off: u32,      // byte offset to entry metadata
    pub snippet_off: u32,   // byte offset to snippet pool
    pub total_len: u32,     // total file size
}

/// One slot in the term hash table (open addressing).
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct TermSlot {
    pub hash: u64,          // full 64-bit hash (0 = empty slot)
    pub postings_off: u32,  // offset into posting list region
    pub postings_len: u16,  // number of postings
    pub _pad: u16,
}

/// One posting: which entry matched, how many times.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct Posting {
    pub entry_id: u16,
    pub tf: u16,            // term frequency in this entry
    pub idf_x1000: u32,    // pre-computed idf * 1000 (fixed-point)
}

/// Entry metadata for result display.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct EntryMeta {
    pub topic_id: u8,
    pub _pad: u8,
    pub word_count: u16,
    pub snippet_off: u32,   // offset into snippet pool
    pub snippet_len: u16,
    pub _pad2: u16,
}

/// Build context: accumulates terms and postings during indexing.
pub struct IndexBuilder {
    terms: HashMap<String, Vec<(u16, u16)>>,  // term → [(entry_id, tf)]
    entries: Vec<EntryInfo>,
    topics: Vec<String>,
    total_words: usize,
}

struct EntryInfo {
    topic_idx: u8,
    word_count: u16,
    snippet: String, // topic + header + first 2 content lines
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self { terms: HashMap::new(), entries: Vec::new(), topics: Vec::new(), total_words: 0 }
    }

    /// Register a topic name, return its index.
    pub fn add_topic(&mut self, name: &str) -> u8 {
        if let Some(i) = self.topics.iter().position(|t| t == name) {
            return i as u8;
        }
        let i = self.topics.len() as u8;
        self.topics.push(name.to_string());
        i
    }

    /// Index one entry's text. Returns the entry_id.
    pub fn add_entry(&mut self, topic_idx: u8, text_lower: &str, snippet: String) -> u16 {
        let entry_id = self.entries.len() as u16;
        let words: Vec<&str> = text_lower.split_whitespace().collect();
        let wc = words.len();
        self.total_words += wc;

        // Count term frequencies
        let mut tf_map: HashMap<&str, u16> = HashMap::new();
        for w in &words {
            *tf_map.entry(w).or_insert(0) += 1;
        }

        // Also index CamelCase/snake_case splits
        for w in &words {
            for part in split_compound(w) {
                if part.len() >= 3 && !tf_map.contains_key(part.as_str()) {
                    *tf_map.entry("").or_insert(0) += 0; // skip empty
                }
            }
        }

        for (term, tf) in tf_map {
            if term.is_empty() || term.len() < 2 { continue; }
            self.terms.entry(term.to_string())
                .or_default()
                .push((entry_id, tf));
        }

        self.entries.push(EntryInfo {
            topic_idx,
            word_count: wc.min(u16::MAX as usize) as u16,
            snippet,
        });
        entry_id
    }

    /// Serialize the index to a byte vector.
    pub fn build(&self) -> Vec<u8> {
        let n = self.entries.len() as f64;
        let avgdl = if self.entries.is_empty() { 100.0 }
            else { self.total_words as f64 / n };

        // Hash table: power-of-2 capacity, ~75% load factor
        let num_terms = self.terms.len();
        let table_cap = (num_terms * 4 / 3 + 1).next_power_of_two().max(16);
        let mask = table_cap - 1;

        // Lay out posting lists sequentially
        let mut posting_buf: Vec<Posting> = Vec::new();
        let mut term_entries: Vec<(u64, u32, u16)> = Vec::new(); // (hash, off, len)

        for (term, postings) in &self.terms {
            let h = hash_term(term);
            let off = posting_buf.len() as u32;
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            let idf_x1000 = (idf * 1000.0) as u32;

            for &(eid, tf) in postings {
                posting_buf.push(Posting { entry_id: eid, tf, idf_x1000 });
            }
            term_entries.push((h, off, postings.len() as u16));
        }

        // Build hash table (open addressing, linear probing)
        let mut table: Vec<TermSlot> = (0..table_cap).map(|_| TermSlot {
            hash: 0, postings_off: 0, postings_len: 0, _pad: 0
        }).collect();

        for (h, off, len) in &term_entries {
            let mut idx = (*h as usize) & mask;
            loop {
                if table[idx].hash == 0 {
                    table[idx] = TermSlot { hash: *h, postings_off: *off, postings_len: *len, _pad: 0 };
                    break;
                }
                idx = (idx + 1) & mask;
            }
        }

        // Build snippet pool + entry metadata
        let mut snippet_pool = Vec::<u8>::new();
        let mut meta_list = Vec::<EntryMeta>::new();

        for info in &self.entries {
            let soff = snippet_pool.len() as u32;
            let sbytes = info.snippet.as_bytes();
            let slen = sbytes.len().min(u16::MAX as usize) as u16;
            snippet_pool.extend_from_slice(&sbytes[..slen as usize]);
            meta_list.push(EntryMeta {
                topic_id: info.topic_idx,
                _pad: 0,
                word_count: info.word_count,
                snippet_off: soff,
                snippet_len: slen,
                _pad2: 0,
            });
        }

        // Compute offsets
        let hdr_size = std::mem::size_of::<Header>();
        let table_size = table_cap * std::mem::size_of::<TermSlot>();
        let postings_off = hdr_size + table_size;
        let postings_size = posting_buf.len() * std::mem::size_of::<Posting>();
        let meta_off = postings_off + postings_size;
        let meta_size = meta_list.len() * std::mem::size_of::<EntryMeta>();
        let snippet_off = meta_off + meta_size;
        let total_len = snippet_off + snippet_pool.len();

        let header = Header {
            magic: MAGIC,
            version: VERSION,
            num_entries: self.entries.len() as u32,
            num_terms: num_terms as u32,
            table_cap: table_cap as u32,
            avgdl_x100: (avgdl * 100.0) as u32,
            postings_off: postings_off as u32,
            meta_off: meta_off as u32,
            snippet_off: snippet_off as u32,
            total_len: total_len as u32,
        };

        // Serialize everything
        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(as_bytes(&header));
        for slot in &table { buf.extend_from_slice(as_bytes(slot)); }
        for p in &posting_buf { buf.extend_from_slice(as_bytes(p)); }
        for m in &meta_list { buf.extend_from_slice(as_bytes(m)); }
        buf.extend_from_slice(&snippet_pool);
        buf
    }
}

/// FNV-1a 64-bit hash — fast, good distribution, no deps.
pub fn hash_term(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0 { h = 1; } // 0 means empty slot
    h
}

/// Build index from the corpus directory and write to index.bin.
pub fn rebuild(dir: &Path) -> Result<String, String> {
    let files = crate::config::list_search_files(dir)?;
    let mut builder = IndexBuilder::new();

    for path in &files {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let topic_idx = builder.add_topic(&name);

        for section in crate::search::parse_sections(&content) {
            if section.is_empty() { continue; }
            let text_lower: String = section.iter()
                .map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
            let snippet = build_snippet(&name, &section);
            builder.add_entry(topic_idx, &text_lower, snippet);
        }
    }

    let bytes = builder.build();
    let index_path = dir.join("index.bin");
    std::fs::write(&index_path, &bytes).map_err(|e| e.to_string())?;
    Ok(format!("index: {} entries, {} terms, {} bytes → {}",
        builder.entries.len(), builder.terms.len(),
        bytes.len(), index_path.display()))
}

fn build_snippet(topic: &str, section: &[&str]) -> String {
    let header = section.first().map(|h| h.trim_start_matches("## ")).unwrap_or("");
    let content_lines: Vec<&str> = section.iter().skip(1)
        .filter(|l| !l.starts_with("[tags:") && !l.trim().is_empty())
        .take(2).copied().collect();
    format!("[{}] {} {}", topic, header,
        content_lines.join(" ").chars().take(120).collect::<String>())
}

fn split_compound(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in s.split(|c: char| c == '_' || c == '-') {
        if segment.is_empty() { continue; }
        let mut cur = String::new();
        for (i, ch) in segment.chars().enumerate() {
            if i > 0 && ch.is_uppercase() && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.push(ch);
        }
        if !cur.is_empty() { parts.push(cur); }
    }
    parts
}

/// Safe cast of a #[repr(C, packed)] struct to bytes.
fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(val as *const T as *const u8, std::mem::size_of::<T>()) }
}
