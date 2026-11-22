//! KVP (key=value&key=value) query-string parsing with URL decoding.

/// Decode a percent-encoded byte (plus signs become spaces only inside query
/// values, which is standard `application/x-www-form-urlencoded` behaviour).
fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => {
                let hex = &s[i + 1..i + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a query string (without the leading `?`) into ordered key/value pairs.
/// The LAST occurrence of a repeated key wins (WMS convention).
pub fn parse_kvp(query: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = pct_decode(k).trim().to_ascii_uppercase();
        let val = pct_decode(v).trim().to_string();
        if key.is_empty() {
            continue;
        }
        // Drop earlier occurrences of the same key (keep the last).
        if let Some(i) = out.iter().rposition(|(k2, _)| *k2 == key) {
            out.remove(i);
        }
        out.push((key, val));
    }
    out
}

/// Convenience: turn the parsed pairs into a case-insensitive lookup.
pub fn lookup<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter()
        .rev()
        .find(|(k, _)| k == &key.to_ascii_uppercase())
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_uppercases_keys() {
        let kv = parse_kvp("service=WMS&request=GetCapabilities&version=1.3.0");
        assert_eq!(lookup(&kv, "service"), Some("WMS"));
        assert_eq!(lookup(&kv, "request"), Some("GetCapabilities"));
    }

    #[test]
    fn last_wins() {
        let kv = parse_kvp("bbox=1,2,3,4&BBOX=5,6,7,8");
        assert_eq!(lookup(&kv, "bbox"), Some("5,6,7,8"));
    }

    #[test]
    fn url_decode() {
        let kv = parse_kvp("layers=a%20b&format=image%2Fpng&x=1+2");
        assert_eq!(lookup(&kv, "layers"), Some("a b"));
        assert_eq!(lookup(&kv, "format"), Some("image/png"));
        assert_eq!(lookup(&kv, "x"), Some("1 2"));
    }
}
