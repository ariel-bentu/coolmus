//! Tiny, forgiving helpers for reading OneNote's output HTML and writing input HTML.
//! OneNote's HTML is simple and machine-generated, so a full parser is unnecessary.

use std::collections::HashMap;

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// An opening tag found in the document, with its attributes and the
/// byte offset just after the tag.
#[derive(Debug, Clone)]
pub struct Tag {
    pub attrs: HashMap<String, String>,
    pub end: usize,
}

impl Tag {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
}

/// Finds every `<name ...>` opening tag (case-insensitive).
pub fn find_tags(html: &str, name: &str) -> Vec<Tag> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{}", name.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&needle) {
        let start = from + rel + needle.len();
        from = start;
        // Must be followed by whitespace, '>' or '/', so "<p" doesn't match "<pre".
        match html[start..].chars().next() {
            Some(c) if c.is_whitespace() || c == '>' || c == '/' => {}
            _ => continue,
        }
        let Some((attrs, end)) = parse_attrs(html, start) else { break };
        out.push(Tag { attrs, end });
        from = end;
    }
    out
}

fn parse_attrs(html: &str, mut i: usize) -> Option<(HashMap<String, String>, usize)> {
    let b = html.as_bytes();
    let mut attrs = HashMap::new();
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        if b[i] == b'>' {
            return Some((attrs, i + 1));
        }
        let ks = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'>' && b[i] != b'/' {
            i += 1;
        }
        let key = html[ks..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                i = vs;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = unescape(&html[vs..i.min(b.len())]);
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = unescape(&html[vs..i]);
            }
        }
        if !key.is_empty() {
            attrs.insert(key, val);
        }
    }
}

/// Text content between the end of `tag` and the next `</name>`, with inner tags stripped.
pub fn inner_text(html: &str, tag: &Tag, name: &str) -> String {
    let rest = &html[tag.end..];
    let close = format!("</{name}");
    let end = rest.to_ascii_lowercase().find(&close).unwrap_or(rest.len());
    let mut text = String::new();
    let mut in_tag = false;
    for c in rest[..end].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    unescape(text.trim())
}

pub fn title(html: &str) -> Option<String> {
    let tag = find_tags(html, "title").into_iter().next()?;
    Some(inner_text(html, &tag, "title"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tags_and_text() {
        let html = r#"<html><head><title>A &amp; B</title></head><body>
            <pre>x</pre><p data-id="sketcher-tags" id="p:{1}{2}">Tags: <span>#a</span> #b</p>
            <object data-attachment="x.sketcher.json" data='https://g/res/1/$value' type="application/json" />
            </body></html>"#;
        assert_eq!(title(html).as_deref(), Some("A & B"));
        let ps = find_tags(html, "p");
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].get("id"), Some("p:{1}{2}"));
        assert_eq!(inner_text(html, &ps[0], "p"), "Tags: #a #b");
        let objs = find_tags(html, "object");
        assert_eq!(objs[0].get("data"), Some("https://g/res/1/$value"));
    }
}
