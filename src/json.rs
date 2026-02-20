use std::fmt;

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Obj(pairs) => pairs.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, val: Value) {
        if let Value::Obj(pairs) = self {
            if let Some(existing) = pairs.iter_mut().find(|(k, _)| k == key) {
                existing.1 = val;
            } else {
                pairs.push((key.into(), val));
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn pretty(&self) -> String {
        let mut buf = String::new();
        self.write_pretty(&mut buf, 0);
        buf.push('\n');
        buf
    }

    fn write_pretty(&self, buf: &mut String, depth: usize) {
        match self {
            Value::Arr(items) if !items.is_empty() => {
                buf.push_str("[\n");
                for (i, v) in items.iter().enumerate() {
                    Self::indent(buf, depth + 1);
                    v.write_pretty(buf, depth + 1);
                    if i + 1 < items.len() { buf.push(','); }
                    buf.push('\n');
                }
                Self::indent(buf, depth);
                buf.push(']');
            }
            Value::Obj(pairs) if !pairs.is_empty() => {
                buf.push_str("{\n");
                for (i, (k, v)) in pairs.iter().enumerate() {
                    Self::indent(buf, depth + 1);
                    buf.push('"');
                    escape_into(k, buf);
                    buf.push_str("\": ");
                    v.write_pretty(buf, depth + 1);
                    if i + 1 < pairs.len() { buf.push(','); }
                    buf.push('\n');
                }
                Self::indent(buf, depth);
                buf.push('}');
            }
            other => {
                use fmt::Write;
                write!(buf, "{other}").unwrap();
            }
        }
    }

    fn indent(buf: &mut String, depth: usize) {
        for _ in 0..depth * 2 { buf.push(' '); }
    }
}

/// Escape a string for JSON embedding (no surrounding quotes).
/// Byte-level chunk-copy: scans for escape-needing bytes, memcpys clean chunks.
/// Public for use by mcp.rs and hook.rs response formatting.
pub fn escape_into(s: &str, buf: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut last_copy = 0;
    while i < bytes.len() {
        let esc = match bytes[i] {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            c if c < 0x20 => {
                // Safety: s is &str, slicing at ASCII positions = valid UTF-8
                if last_copy < i { buf.push_str(&s[last_copy..i]); }
                use fmt::Write;
                let _ = write!(buf, "\\u{:04x}", c);
                i += 1;
                last_copy = i;
                continue;
            }
            _ => { i += 1; continue; }
        };
        if last_copy < i { buf.push_str(&s[last_copy..i]); }
        buf.push_str(esc);
        i += 1;
        last_copy = i;
    }
    if last_copy < bytes.len() { buf.push_str(&s[last_copy..]); }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut buf = String::new();
        write_compact(self, &mut buf);
        f.write_str(&buf)
    }
}

fn write_compact(v: &Value, buf: &mut String) {
    match v {
        Value::Null => buf.push_str("null"),
        Value::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            use fmt::Write;
            if n.fract() == 0.0 && n.is_finite() { write!(buf, "{}", *n as i64).unwrap(); }
            else { write!(buf, "{n}").unwrap(); }
        }
        Value::Str(s) => {
            buf.push('"');
            escape_into(s, buf);
            buf.push('"');
        }
        Value::Arr(items) => {
            buf.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 { buf.push(','); }
                write_compact(v, buf);
            }
            buf.push(']');
        }
        Value::Obj(pairs) => {
            buf.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 { buf.push(','); }
                buf.push('"');
                escape_into(k, buf);
                buf.push_str("\":");
                write_compact(v, buf);
            }
            buf.push('}');
        }
    }
}

// --- Parser ---

pub fn parse(input: &str) -> Result<Value, String> {
    let mut p = Parser { b: input.as_bytes(), pos: 0 };
    p.value()
}

struct Parser<'a> { b: &'a [u8], pos: usize }

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.b.len()
            && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        { self.pos += 1; }
    }

    fn peek(&self) -> Option<u8> { self.b.get(self.pos).copied() }

    fn next(&mut self) -> Result<u8, String> {
        self.b.get(self.pos).copied()
            .map(|b| { self.pos += 1; b })
            .ok_or_else(|| "unexpected end".into())
    }

    fn expect(&mut self, s: &[u8]) -> Result<(), String> {
        for &c in s {
            if self.next()? != c { return Err(format!("expected '{}'", c as char)); }
        }
        Ok(())
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.peek() {
            Some(b'"') => self.string().map(Value::Str),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b't') => { self.expect(b"true")?; Ok(Value::Bool(true)) }
            Some(b'f') => { self.expect(b"false")?; Ok(Value::Bool(false)) }
            Some(b'n') => { self.expect(b"null")?; Ok(Value::Null) }
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected '{}'", c as char)),
            None => Err("unexpected end".into()),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // skip opening "
        // Fast path: if no escape sequences, substring copy (one alloc, exact size).
        // ~95% of MCP protocol strings have no escapes.
        let start = self.pos;
        let mut p = start;
        while p < self.b.len() {
            match self.b[p] {
                b'"' => {
                    // Safety: input is &str.as_bytes() (valid UTF-8), slicing
                    // between ASCII positions (start after '"', end at '"') is safe.
                    let s = unsafe { std::str::from_utf8_unchecked(&self.b[start..p]) }
                        .to_string();
                    self.pos = p + 1;
                    return Ok(s);
                }
                b'\\' => break, // has escapes, fall through to slow path
                _ => p += 1,
            }
        }
        // Slow path: string has escape sequences (or is unterminated → will error)
        let mut s = String::new();
        loop {
            let b = self.next()?;
            match b {
                b'"' => return Ok(s),
                b'\\' => match self.next()? {
                    b'"' => s.push('"'),  b'\\' => s.push('\\'),
                    b'/' => s.push('/'),  b'n' => s.push('\n'),
                    b'r' => s.push('\r'), b't' => s.push('\t'),
                    b'b' => s.push('\x08'), b'f' => s.push('\x0C'),
                    b'u' => {
                        let mut cp = 0u32;
                        for _ in 0..4 {
                            let h = self.next()?;
                            cp = cp * 16 + match h {
                                b'0'..=b'9' => (h - b'0') as u32,
                                b'a'..=b'f' => (h - b'a' + 10) as u32,
                                b'A'..=b'F' => (h - b'A' + 10) as u32,
                                _ => return Err("bad \\u hex".into()),
                            };
                        }
                        s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                    }
                    c => s.push(c as char),
                },
                _ if b < 0x80 => s.push(b as char),
                _ => {
                    let start = self.pos - 1;
                    let w = if b >= 0xF0 { 4 } else if b >= 0xE0 { 3 } else { 2 };
                    self.pos = (start + w).min(self.b.len());
                    if let Ok(u) = std::str::from_utf8(&self.b[start..self.pos]) {
                        s.push_str(u);
                    }
                }
            }
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') { self.pos += 1; }
        while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() { self.pos += 1; }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() { self.pos += 1; }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) { self.pos += 1; }
            while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() { self.pos += 1; }
        }
        let s = std::str::from_utf8(&self.b[start..self.pos]).unwrap_or("0");
        s.parse::<f64>()
            .map(Value::Num)
            .map_err(|e| e.to_string())
    }

    fn object(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut pairs = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') { self.pos += 1; return Ok(Value::Obj(pairs)); }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            if self.next()? != b':' { return Err("expected ':'".into()); }
            pairs.push((key, self.value()?));
            self.ws();
            match self.next()? {
                b',' => continue,
                b'}' => return Ok(Value::Obj(pairs)),
                _ => return Err("expected ',' or '}'".into()),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') { self.pos += 1; return Ok(Value::Arr(items)); }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.next()? {
                b',' => continue,
                b']' => return Ok(Value::Arr(items)),
                _ => return Err("expected ',' or ']'".into()),
            }
        }
    }
}
