//! Minimal multipart encoding (requests) and decoding (InkML responses).

pub struct Part<'a> {
    pub name: &'a str,
    pub content_type: &'a str,
    pub body: &'a [u8],
}

/// Encodes `multipart/form-data`. Returns (content-type header, body).
pub fn encode(parts: &[Part]) -> (String, Vec<u8>) {
    let boundary = format!("SketcherBoundary{}", sketch_core::new_id().simple());
    let mut body = Vec::new();
    for p in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                p.name, p.content_type
            )
            .as_bytes(),
        );
        body.extend_from_slice(p.body);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// A decoded part: lower-cased content type and body text.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPart {
    pub content_type: String,
    pub body: String,
}

/// Extracts `boundary=` from a Content-Type header.
pub fn boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        k.eq_ignore_ascii_case("boundary").then(|| v.trim().trim_matches('"').to_string())
    })
}

/// Splits a multipart body into parts.
pub fn decode(body: &str, boundary: &str) -> Vec<DecodedPart> {
    let delim = format!("--{boundary}");
    body.split(delim.as_str())
        .skip(1)
        .filter(|chunk| !chunk.starts_with("--"))
        .filter_map(|chunk| {
            let chunk = chunk.trim_start_matches(['\r', '\n']);
            let (head, content) = chunk.split_once("\r\n\r\n").or_else(|| chunk.split_once("\n\n"))?;
            let content_type = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim().eq_ignore_ascii_case("content-type").then(|| v.trim().to_lowercase())
                })
                .unwrap_or_default();
            let body = content.strip_suffix("\r\n").or_else(|| content.strip_suffix('\n')).unwrap_or(content);
            Some(DecodedPart { content_type, body: body.to_string() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (ct, body) = encode(&[
            Part { name: "presentation", content_type: "text/html", body: b"<html/>" },
            Part { name: "ink", content_type: "application/inkml+xml", body: b"<ink/>" },
        ]);
        let b = boundary(&ct).unwrap();
        let parts = decode(std::str::from_utf8(&body).unwrap(), &b);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], DecodedPart { content_type: "text/html".into(), body: "<html/>".into() });
        assert_eq!(parts[1].content_type, "application/inkml+xml");
    }
}
