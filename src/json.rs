use std::fmt;

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(i64),
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

fn escape_into(s: &str, buf: &mut String) {
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if c < '\x20' => {
                use fmt::Write;
                write!(buf, "\\u{:04x}", c as u32).unwrap();
            }
            c => buf.push(c),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => {
                write!(f, "\"")?;
                for c in s.chars() {
                    match c {
                        '"' => write!(f, "\\\"")?,
                        '\\' => write!(f, "\\\\")?,
                        '\n' => write!(f, "\\n")?,
                        '\r' => write!(f, "\\r")?,
                        '\t' => write!(f, "\\t")?,
                        c if c < '\x20' => write!(f, "\\u{:04x}", c as u32)?,
                        c => write!(f, "{c}")?,
                    }
                }
                write!(f, "\"")
            }
            Value::Arr(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 { write!(f, ",")?; }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Obj(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 { write!(f, ",")?; }
                    write!(f, "\"{k}\":{v}")?;
                }
                write!(f, "}}")
            }
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
        self.pos += 1;
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
        s.parse::<i64>()
            .or_else(|_| s.parse::<f64>().map(|f| f as i64))
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
