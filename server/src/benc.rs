//! A small bencode reader (DHT messages are tiny dictionaries).

#[derive(Debug, PartialEq)]
pub enum Benc<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Benc<'a>>),
    Dict(Vec<(&'a [u8], Benc<'a>)>),
}

impl<'a> Benc<'a> {
    pub fn get(&self, key: &str) -> Option<&Benc<'a>> {
        match self {
            Self::Dict(pairs) => pairs.iter().find(|(k, _)| *k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Benc<'a>] {
        match self {
            Self::List(l) => l,
            _ => &[],
        }
    }
}

/// Decodes one value; trailing bytes are ignored.
pub fn decode(data: &[u8]) -> Option<Benc<'_>> {
    value(data, &mut 0, 0)
}

fn value<'a>(d: &'a [u8], at: &mut usize, depth: u8) -> Option<Benc<'a>> {
    if depth > 8 {
        return None;
    }
    match *d.get(*at)? {
        b'i' => {
            let end = *at + d[*at..].iter().position(|b| *b == b'e')?;
            let n = std::str::from_utf8(&d[*at + 1..end]).ok()?.parse().ok()?;
            *at = end + 1;
            Some(Benc::Int(n))
        }
        b'l' => {
            *at += 1;
            let mut list = Vec::new();
            while *d.get(*at)? != b'e' {
                list.push(value(d, at, depth + 1)?);
            }
            *at += 1;
            Some(Benc::List(list))
        }
        b'd' => {
            *at += 1;
            let mut pairs = Vec::new();
            while *d.get(*at)? != b'e' {
                let key = string(d, at)?;
                pairs.push((key, value(d, at, depth + 1)?));
            }
            *at += 1;
            Some(Benc::Dict(pairs))
        }
        b'0'..=b'9' => string(d, at).map(Benc::Bytes),
        _ => None,
    }
}

fn string<'a>(d: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let colon = *at + d.get(*at..)?.iter().position(|b| *b == b':')?;
    let len: usize = std::str::from_utf8(&d[*at..colon]).ok()?.parse().ok()?;
    let out = d.get(colon + 1..(colon + 1).checked_add(len)?)?;
    *at = colon + 1 + len;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_nested_values_and_rejects_garbage() {
        let v = decode(b"d1:rd2:id2:ab5:token3:xyz6:valuesl6:\x01\x02\x03\x04\x1a\xe1ee1:t2:\x00\x071:y1:r1:ni-5ee").unwrap();
        assert_eq!(v.get("y").and_then(Benc::bytes), Some(&b"r"[..]));
        assert_eq!(v.get("n"), Some(&Benc::Int(-5)));
        let r = v.get("r").unwrap();
        assert_eq!(r.get("token").and_then(Benc::bytes), Some(&b"xyz"[..]));
        assert_eq!(r.get("values").unwrap().list().len(), 1);
        assert!(v.get("missing").is_none() && v.get("y").unwrap().list().is_empty());

        assert!(decode(b"d1:a").is_none());
        assert!(decode(b"5:abc").is_none());
        assert!(decode(b"99999999999999999999:a").is_none());
        assert!(decode(&[b'l'; 64]).is_none());
        assert!(decode(b"").is_none());
    }
}
