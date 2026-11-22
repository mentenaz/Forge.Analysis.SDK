//! A strict XML reader for build files (MSBuild projects, `nuget.config`) that keeps the byte span
//! of every element, attribute value and run of text.
//!
//! Written here, like `json.rs`, so that hostile input is bounded and nothing is fetched or
//! expanded: no external crate, no network, no files.
//!
//! What it accepts: elements, attributes, text, CDATA, comments, processing instructions and the
//! five named entities plus numeric references. What it refuses, with an error and not a guess:
//! a DOCTYPE (so no entity can be defined, and nothing can expand), any other entity, a repeated
//! attribute, a closing tag that does not match, content after the root, and a file over the
//! limits below. Names are kept as written; callers compare them without regard to case, because
//! MSBuild does.
//!
//! Limits (a file over them is refused, never partly read): 4 MiB, 64 levels of nesting.

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Child>,
    /// From the `<` of the start tag to the `>` of the end tag.
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub value: String,
    /// The span of the value, without its quotes.
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Child {
    Element(Element),
    Text { text: String, start: u32, end: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    pub at: u32,
    pub what: &'static str,
}

impl Element {
    /// The value of an attribute, whatever its case.
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Child::Element(e) => Some(e),
            Child::Text { .. } => None,
        })
    }

    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// All the text directly inside this element, joined, and the span it covers.
    pub fn text(&self) -> Option<(String, u32, u32)> {
        let mut out = String::new();
        let (mut start, mut end) = (u32::MAX, 0);
        for c in &self.children {
            if let Child::Text { text, start: s, end: e } = c {
                out.push_str(text);
                start = start.min(*s);
                end = end.max(*e);
            }
        }
        (start != u32::MAX && !out.trim().is_empty()).then_some((out, start, end))
    }
}

pub fn parse(src: &str) -> Result<Element, XmlError> {
    if src.len() > MAX_BYTES {
        return Err(XmlError { at: 0, what: "file too large" });
    }
    let b = src.as_bytes();
    let mut p = P { src, b, i: if b.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 }, depth: 0 };
    p.misc()?;
    if p.b.get(p.i) != Some(&b'<') {
        return Err(p.err("no root element"));
    }
    let root = p.element()?;
    p.misc()?;
    if p.i != b.len() {
        return Err(p.err("content after the root element"));
    }
    Ok(root)
}

struct P<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
    depth: usize,
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn is_name_char(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

impl P<'_> {
    fn err(&self, what: &'static str) -> XmlError {
        XmlError { at: self.i as u32, what }
    }

    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn starts(&self, s: &str) -> bool {
        self.b[self.i..].starts_with(s.as_bytes())
    }

    /// Skips up to the end of `end`, which must be present.
    fn skip_to(&mut self, end: &str, what: &'static str) -> Result<(), XmlError> {
        match self.src[self.i..].find(end) {
            Some(n) => {
                self.i += n + end.len();
                Ok(())
            }
            None => Err(self.err(what)),
        }
    }

    /// Whitespace, comments and processing instructions (including the XML declaration).
    fn misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.ws();
            if self.starts("<!--") {
                self.i += 4;
                self.skip_to("-->", "unterminated comment")?;
            } else if self.starts("<?") {
                self.i += 2;
                self.skip_to("?>", "unterminated processing instruction")?;
            } else if self.starts("<!") {
                // A DOCTYPE is where entities are defined. None is accepted, so none can expand.
                return Err(self.err("a DOCTYPE is not accepted"));
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let s = self.i;
        if !self.b.get(self.i).copied().is_some_and(is_name_start) {
            return Err(self.err("expected a name"));
        }
        while self.b.get(self.i).copied().is_some_and(is_name_char) {
            self.i += 1;
        }
        Ok(self.src[s..self.i].to_string())
    }

    /// Decodes the entities in `raw`. Only the five named ones and numeric references.
    fn decode(&self, raw: &str, at: usize) -> Result<String, XmlError> {
        if !raw.contains('&') {
            return Ok(raw.to_string());
        }
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(i) = rest.find('&') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            let Some(semi) = after.find(';') else {
                return Err(XmlError { at: at as u32, what: "unterminated entity" });
            };
            let ent = &after[..semi];
            match ent {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ => {
                    let code = ent
                        .strip_prefix("#x")
                        .or_else(|| ent.strip_prefix("#X"))
                        .map(|h| u32::from_str_radix(h, 16))
                        .or_else(|| ent.strip_prefix('#').map(|d| d.parse::<u32>()));
                    match code.and_then(Result::ok).and_then(char::from_u32) {
                        Some(c) => out.push(c),
                        None => return Err(XmlError { at: at as u32, what: "unknown entity" }),
                    }
                }
            }
            rest = &after[semi + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn element(&mut self) -> Result<Element, XmlError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        let start = self.i as u32;
        self.i += 1; // <
        let name = self.name()?;
        let mut attrs: Vec<Attr> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        loop {
            let had_ws = matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r'));
            self.ws();
            match self.b.get(self.i) {
                Some(b'/') => {
                    self.i += 1;
                    if self.b.get(self.i) != Some(&b'>') {
                        return Err(self.err("expected >"));
                    }
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(Element { name, attrs, children: Vec::new(), start, end: self.i as u32 });
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) if had_ws => {
                    let an = self.name()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b'=') {
                        return Err(self.err("expected = after an attribute name"));
                    }
                    self.i += 1;
                    self.ws();
                    let q = match self.b.get(self.i) {
                        Some(&q @ (b'"' | b'\'')) => q,
                        _ => return Err(self.err("an attribute value must be quoted")),
                    };
                    self.i += 1;
                    let vs = self.i;
                    while let Some(&c) = self.b.get(self.i) {
                        if c == q {
                            break;
                        }
                        if c == b'<' {
                            return Err(self.err("< inside an attribute value"));
                        }
                        self.i += 1;
                    }
                    if self.b.get(self.i).is_none() {
                        return Err(self.err("unterminated attribute value"));
                    }
                    let value = self.decode(&self.src[vs..self.i], vs)?;
                    if !seen.insert(an.clone()) {
                        return Err(self.err("a repeated attribute"));
                    }
                    attrs.push(Attr { name: an, value, start: vs as u32, end: self.i as u32 });
                    self.i += 1; // closing quote
                }
                _ => return Err(self.err("unexpected character in a tag")),
            }
        }
        let mut children: Vec<Child> = Vec::new();
        loop {
            let ts = self.i;
            while self.b.get(self.i).is_some_and(|&c| c != b'<') {
                self.i += 1;
            }
            if self.i > ts {
                let text = self.decode(&self.src[ts..self.i], ts)?;
                children.push(Child::Text { text, start: ts as u32, end: self.i as u32 });
            }
            if self.b.get(self.i).is_none() {
                return Err(self.err("unexpected end of the file"));
            }
            if self.starts("<!--") {
                self.i += 4;
                self.skip_to("-->", "unterminated comment")?;
            } else if self.starts("<![CDATA[") {
                self.i += 9;
                let cs = self.i;
                self.skip_to("]]>", "unterminated CDATA")?;
                let text = self.src[cs..self.i - 3].to_string();
                children.push(Child::Text { text, start: cs as u32, end: (self.i - 3) as u32 });
            } else if self.starts("<?") {
                self.i += 2;
                self.skip_to("?>", "unterminated processing instruction")?;
            } else if self.starts("<!") {
                return Err(self.err("a DOCTYPE is not accepted"));
            } else if self.starts("</") {
                self.i += 2;
                let close = self.name()?;
                if close != name {
                    return Err(self.err("a closing tag that does not match"));
                }
                self.ws();
                if self.b.get(self.i) != Some(&b'>') {
                    return Err(self.err("expected >"));
                }
                self.i += 1;
                self.depth -= 1;
                return Ok(Element { name, attrs, children, start, end: self.i as u32 });
            } else {
                children.push(Child::Element(self.element()?));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Element {
        parse(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
    }

    fn bad(s: &str) {
        assert!(parse(s).is_err(), "should be refused: {s}");
    }

    #[test]
    fn elements_attributes_text_and_spans_are_kept() {
        let src = "<?xml version=\"1.0\"?>\n<!-- c -->\n<P a=\"1\" B='x &amp; y'><T>hi &lt;there&gt;</T><E/><![CDATA[<raw>]]></P>";
        let p = ok(src);
        assert_eq!(p.name, "P");
        assert_eq!(p.attr("b").unwrap().value, "x & y");
        assert_eq!(&src[p.attr("a").unwrap().start as usize..p.attr("a").unwrap().end as usize], "1");
        let t = p.elements().next().unwrap();
        assert_eq!(t.text().unwrap().0, "hi <there>");
        assert_eq!(p.elements().count(), 2);
        assert_eq!(p.text().unwrap().0, "<raw>");
        assert_eq!(&src[p.start as usize..p.end as usize], &src[src.find("<P").unwrap()..]);
    }

    #[test]
    fn numeric_references_decode() {
        assert_eq!(ok("<a v=\"&#65;&#x42;\"/>").attr("v").unwrap().value, "AB");
    }

    #[test]
    fn a_doctype_and_other_entities_are_refused() {
        bad("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>");
        bad("<!DOCTYPE a><a/>");
        bad("<a>&nbsp;</a>");
        bad("<a v=\"&nope;\"/>");
        bad("<a>&amp</a>");
        bad("<a>&#xZZ;</a>");
    }

    #[test]
    fn malformed_files_are_refused() {
        for s in [
            "",
            "text",
            "<a>",
            "<a></b>",
            "<a b=1/>",
            "<a b=\"1\" b=\"2\"/>",
            "<a b=\"<\"/>",
            "<a/><b/>",
            "<a/>junk",
            "<a b=\"x/>",
            "<a b/>",
            "<a<b/>",
            "<a><!-- never closed </a>",
            "<a><![CDATA[never closed</a>",
            "<a><?pi never closed</a>",
            "<1a/>",
        ] {
            bad(s);
        }
    }

    #[test]
    fn nesting_and_size_are_capped() {
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 1), "</a>".repeat(MAX_DEPTH + 1));
        bad(&deep);
        let fine = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        ok(&fine);
        assert!(parse(&" ".repeat(MAX_BYTES + 1)).is_err());
    }
}
