//! Tag normalization and search queries.

/// Canonical form of a user-typed tag: trimmed, leading `#` removed,
/// inner whitespace collapsed to `-`. Returns `None` for empty input.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let t = raw.trim().trim_start_matches('#').trim();
    if t.is_empty() {
        return None;
    }
    Some(t.split_whitespace().collect::<Vec<_>>().join("-"))
}

/// Adds a tag if not already present (case-insensitive). Returns true if added.
pub fn add_tag(tags: &mut Vec<String>, raw: &str) -> bool {
    let Some(tag) = normalize_tag(raw) else { return false };
    if tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
        return false;
    }
    tags.push(tag);
    true
}

/// A parsed search query. `#tag` terms must all match a tag exactly
/// (case-insensitive); other words must each appear in the title or a tag.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Query {
    pub tags: Vec<String>,
    pub words: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut q = Query::default();
        for term in input.split_whitespace() {
            if let Some(tag) = term.strip_prefix('#') {
                if !tag.is_empty() {
                    q.tags.push(tag.to_lowercase());
                }
            } else {
                q.words.push(term.to_lowercase());
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.words.is_empty()
    }

    pub fn matches(&self, title: &str, tags: &[String]) -> bool {
        let tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let title = title.to_lowercase();
        self.tags.iter().all(|q| tags.iter().any(|t| t == q))
            && self
                .words
                .iter()
                .all(|w| title.contains(w.as_str()) || tags.iter().any(|t| t.contains(w.as_str())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize() {
        assert_eq!(normalize_tag("  #My  Idea "), Some("My-Idea".into()));
        assert_eq!(normalize_tag("#"), None);
    }

    #[test]
    fn query_matching() {
        let tags = vec!["Home".to_string(), "design".to_string()];
        let m = |q: &str| Query::parse(q).matches("Kitchen layout", &tags);
        assert!(m("#home"));
        assert!(m("kitchen #design"));
        assert!(m("des"));
        assert!(!m("#des"));
        assert!(!m("garage"));
    }
}
