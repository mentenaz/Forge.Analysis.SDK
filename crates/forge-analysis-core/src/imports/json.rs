//! A strict JSON reader that keeps the byte span of every value and every key.
//!
//! Why not `serde_json`: facts need exact spans, and a `package.json` with a repeated key is
//! a known trick (`JSON.parse` keeps the last one), so repeats must stay visible. The grammar
//! is RFC 8259: no comments, no trailing commas, no single quotes, no `NaN`.
//!
//! Limits (a file over them is refused, never partly read): 4 MiB, 64 levels of nesting.
//!
//! `parse_lenient` is for the files whose own format allows more: editor settings and tasks, dev
//! container files and `global.json` are JSON with comments (`//`, `/* */`) and trailing commas.
//! Everything else stays strict, and repeated keys stay visible in both.

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jv {
    pub start: u32,
    pub end: u32,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// In source order, repeats included.
    Object(Vec<Member>),
    Array(Vec<Jv>),
    Str(String),
    Number,
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub key: String,
    pub key_start: u32,
    pub key_end: u32,
    pub value: Jv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub at: u32,
    pub what: &'static str,
}

pub fn parse(src: &str) -> Result<Jv, JsonError> {
    parse_with(src, false)
}

/// JSON with comments and trailing commas, as editors read their own files.
pub fn parse_lenient(src: &str) -> Result<Jv, JsonError> {
    parse_with(src, true)
}

fn parse_with(src: &str, lenient: bool) -> Result<Jv, JsonError> {
    if src.len() > MAX_BYTES {
        return Err(JsonError { at: 0, what: "file too large" });
    }
    let b = src.as_bytes();
    let mut p = P { b, i: if b.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 }, depth: 0, lenient };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != b.len() {
        return Err(p.err("content after the value"));
    }
    Ok(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
    lenient: bool,
}

impl P<'_> {
    fn err(&self, what: &'static str) -> JsonError {
        JsonError { at: self.i as u32, what }
    }

    fn ws(&mut self) {
        loop {
            while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
            if !self.lenient || self.b.get(self.i) != Some(&b'/') {
                return;
            }
            match self.b.get(self.i + 1) {
                Some(b'/') => {
                    while !matches!(self.b.get(self.i), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                Some(b'*') => {
                    self.i += 2;
                    // An unterminated comment swallows the rest, so the value after it fails to parse.
                    while self.i < self.b.len() && !self.b[self.i..].starts_with(b"*/") {
                        self.i += 1;
                    }
                    self.i = (self.i + 2).min(self.b.len());
                }
                _ => return,
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Jv, JsonError> {
        let start = self.i as u32;
        let kind = match self.b.get(self.i) {
            None => return Err(self.err("unexpected end")),
            Some(b'{') => self.object()?,
            Some(b'[') => self.array()?,
            Some(b'"') => Kind::Str(self.string()?),
            Some(b't') => self.literal(b"true", Kind::Bool(true))?,
            Some(b'f') => self.literal(b"false", Kind::Bool(false))?,
            Some(b'n') => self.literal(b"null", Kind::Null)?,
            Some(b'-' | b'0'..=b'9') => self.number()?,
            Some(_) => return Err(self.err("unexpected character")),
        };
        Ok(Jv { start, end: self.i as u32, kind })
    }

    fn literal(&mut self, word: &[u8], kind: Kind) -> Result<Kind, JsonError> {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(kind)
        } else {
            Err(self.err("bad literal"))
        }
    }

    fn number(&mut self) -> Result<Kind, JsonError> {
        self.eat(b'-');
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("bad number")),
        }
        if self.eat(b'.') {
            let s = self.i;
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            if self.i == s {
                return Err(self.err("bad number"));
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let s = self.i;
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            if self.i == s {
                return Err(self.err("bad number"));
            }
        }
        Ok(Kind::Number)
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = match self.b.get(self.i) {
                Some(c @ b'0'..=b'9') => c - b'0',
                Some(c @ b'a'..=b'f') => c - b'a' + 10,
                Some(c @ b'A'..=b'F') => c - b'A' + 10,
                _ => return Err(self.err("bad \\u escape")),
            };
            v = v * 16 + u32::from(d);
            self.i += 1;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let run = self.i;
            while let Some(&c) = self.b.get(self.i) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // The input is a &str, and a run stops only at ASCII bytes, so this is valid UTF-8.
            out.push_str(std::str::from_utf8(&self.b[run..self.i]).map_err(|_| self.err("bad utf-8"))?);
            match self.b.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let e = *self.b.get(self.i).ok_or_else(|| self.err("unterminated string"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                } else {
                                    self.i = save;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            // A lone surrogate is legal JSON but not a Rust char.
                            out.push(ch.unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
            }
        }
    }

    fn enter(&mut self) -> Result<(), JsonError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Kind, JsonError> {
        self.enter()?;
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            self.depth -= 1;
            return Ok(Kind::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            if self.eat(b',') {
                self.ws();
                if self.lenient && self.eat(b']') {
                    self.depth -= 1;
                    return Ok(Kind::Array(items));
                }
                continue;
            }
            if self.eat(b']') {
                self.depth -= 1;
                return Ok(Kind::Array(items));
            }
            return Err(self.err("expected , or ]"));
        }
    }

    fn object(&mut self) -> Result<Kind, JsonError> {
        self.enter()?;
        self.i += 1;
        let mut members = Vec::new();
        self.ws();
        if self.eat(b'}') {
            self.depth -= 1;
            return Ok(Kind::Object(members));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return Err(self.err("expected a key"));
            }
            let key_start = self.i as u32;
            let key = self.string()?;
            let key_end = self.i as u32;
            self.ws();
            if !self.eat(b':') {
                return Err(self.err("expected :"));
            }
            self.ws();
            let value = self.value()?;
            members.push(Member { key, key_start, key_end, value });
            self.ws();
            if self.eat(b',') {
                self.ws();
                if self.lenient && self.eat(b'}') {
                    self.depth -= 1;
                    return Ok(Kind::Object(members));
                }
                continue;
            }
            if self.eat(b'}') {
                self.depth -= 1;
                return Ok(Kind::Object(members));
            }
            return Err(self.err("expected , or }"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Jv {
        parse(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
    }

    fn bad(s: &str) {
        assert!(parse(s).is_err(), "should be rejected: {s}");
    }

    #[test]
    fn lenient_mode_allows_comments_and_trailing_commas_and_strict_mode_does_not() {
        let src = "// head\n{ /* a */ \"a\": [1, 2,], \"b\": {\"c\": 1,}, }";
        assert!(parse(src).is_err());
        let v = parse_lenient(src).unwrap();
        let Kind::Object(ms) = v.kind else { panic!() };
        assert_eq!(ms.len(), 2);
        // Repeats stay visible, and the limits still apply.
        let rep = parse_lenient("{\"a\":1,\"a\":2}").unwrap();
        assert!(matches!(rep.kind, Kind::Object(ref m) if m.len() == 2));
        assert!(parse_lenient(&format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1))).is_err());
    }

    #[test]
    fn lenient_mode_still_rejects_broken_json() {
        for s in ["{\"a\": /* never closed", "{\"a\" 1}", "[1 2]", "{,}", "[,]", "{\"a\":1,,}", "/ {}", "{\"a\": // x"] {
            assert!(parse_lenient(s).is_err(), "{s}");
        }
        // A `/` inside a string is not a comment.
        assert!(parse_lenient("{\"url\": \"http://x/*y*/\"}").is_ok());
    }

    #[test]
    fn scalars_and_containers_parse() {
        assert_eq!(ok("null").kind, Kind::Null);
        assert_eq!(ok(" true ").kind, Kind::Bool(true));
        assert_eq!(ok("-0.5e+10").kind, Kind::Number);
        assert_eq!(ok("\"a\"").kind, Kind::Str("a".into()));
        assert!(matches!(ok("[1, [2], {}]").kind, Kind::Array(v) if v.len() == 3));
    }

    #[test]
    fn spans_point_at_the_text() {
        let src = r#"{"a": "xy", "b": [1]}"#;
        let v = ok(src);
        let Kind::Object(m) = &v.kind else { panic!() };
        assert_eq!(&src[m[0].key_start as usize..m[0].key_end as usize], "\"a\"");
        assert_eq!(&src[m[0].value.start as usize..m[0].value.end as usize], "\"xy\"");
        assert_eq!(&src[m[1].value.start as usize..m[1].value.end as usize], "[1]");
        assert_eq!((v.start, v.end), (0, src.len() as u32));
    }

    #[test]
    fn repeated_keys_are_all_kept_in_order() {
        let Kind::Object(m) = ok(r#"{"a":1,"a":2}"#).kind else { panic!() };
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].key, "a");
    }

    #[test]
    fn escapes_decode() {
        assert_eq!(ok(r#""a\n\t\"\\\/\u0041""#).kind, Kind::Str("a\n\t\"\\/A".into()));
        assert_eq!(ok(r#""\ud83d\ude00""#).kind, Kind::Str("😀".into()));
        assert_eq!(ok(r#""\ud83dx""#).kind, Kind::Str("\u{FFFD}x".into()));
        assert_eq!(ok("\"é\"").kind, Kind::Str("é".into()));
    }

    #[test]
    fn a_byte_order_mark_is_skipped_and_spans_stay_in_the_original() {
        let src = "\u{FEFF}{\"a\":1}";
        let v = ok(src);
        assert_eq!(v.start, 3);
        assert_eq!(&src[v.start as usize..v.end as usize], "{\"a\":1}");
    }

    #[test]
    fn anything_outside_the_grammar_is_rejected() {
        for s in [
            "", "{", "[1,]", "{\"a\":1,}", "{'a':1}", "{a:1}", "[01]", "[1.]", "[.5]", "[+1]", "NaN",
            "[Infinity]", "// c\n{}", "{} x", "\"a\nb\"", "\"\\x41\"", "\"\\u12\"", "tru", "[1 2]",
            "{\"a\" 1}", "\"abc", "\u{a0}{}",
        ] {
            bad(s);
        }
    }

    #[test]
    fn nesting_and_size_are_capped() {
        bad(&format!("{}{}", "[".repeat(65), "]".repeat(65)));
        ok(&format!("{}{}", "[".repeat(64), "]".repeat(64)));
        bad(&format!("\"{}\"", "a".repeat(MAX_BYTES)));
    }

    #[test]
    fn deep_input_does_not_overflow_the_stack() {
        bad(&"[".repeat(200_000));
    }
}
