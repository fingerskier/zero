//! Deterministic CBOR subset per doc/KERNEL.md §3.
//!
//! RFC 8949 Core Deterministic Encoding, further restricted: definite
//! lengths only; text-string map keys, unique, sorted bytewise by encoded
//! form; shortest-form integers; no floats; no tags; simple values limited
//! to false/true/null. The decoder REJECTS non-canonical input — decode →
//! re-encode is byte-identical by construction (INVARIANTS I-6).

use thiserror::Error;

/// Maximum nesting depth (registry `limits.max_cbor_depth`).
pub const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cbor {
    Uint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    /// Entries in encoded-key order; `encode` sorts, `decode` verifies.
    Map(Vec<(String, Cbor)>),
    Bool(bool),
    Null,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CborError {
    #[error("duplicate map key")]
    DuplicateKey,
    #[error("map keys not in canonical order")]
    KeyOrder,
    #[error("nesting depth exceeds {MAX_DEPTH}")]
    Depth,
    #[error("non-canonical encoding (non-shortest form or indefinite length)")]
    NonCanonical,
    #[error("unsupported CBOR item (floats, tags, and other simples are excluded)")]
    Unsupported,
    #[error("truncated input")]
    Truncated,
    #[error("trailing bytes after top-level item")]
    Trailing,
    #[error("invalid UTF-8 in text string")]
    Utf8,
}

// ---------------------------------------------------------------- encode

fn write_header(out: &mut Vec<u8>, major: u8, arg: u64) {
    let mt = major << 5;
    match arg {
        0..=23 => out.push(mt | arg as u8),
        24..=0xff => {
            out.push(mt | 24);
            out.push(arg as u8);
        }
        0x100..=0xffff => {
            out.push(mt | 25);
            out.extend_from_slice(&(arg as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(mt | 26);
            out.extend_from_slice(&(arg as u32).to_be_bytes());
        }
        _ => {
            out.push(mt | 27);
            out.extend_from_slice(&arg.to_be_bytes());
        }
    }
}

fn encode_text_key(key: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() + 2);
    write_header(&mut out, 3, key.len() as u64);
    out.extend_from_slice(key.as_bytes());
    out
}

fn encode_into(value: &Cbor, out: &mut Vec<u8>, depth: usize) -> Result<(), CborError> {
    if depth > MAX_DEPTH {
        return Err(CborError::Depth);
    }
    match value {
        Cbor::Uint(n) => write_header(out, 0, *n),
        Cbor::Bytes(b) => {
            write_header(out, 2, b.len() as u64);
            out.extend_from_slice(b);
        }
        Cbor::Text(s) => {
            write_header(out, 3, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        Cbor::Array(items) => {
            write_header(out, 4, items.len() as u64);
            for item in items {
                encode_into(item, out, depth + 1)?;
            }
        }
        Cbor::Map(entries) => {
            let mut encoded: Vec<(Vec<u8>, &Cbor)> = entries
                .iter()
                .map(|(k, v)| (encode_text_key(k), v))
                .collect();
            encoded.sort_by(|a, b| a.0.cmp(&b.0));
            for pair in encoded.windows(2) {
                if pair[0].0 == pair[1].0 {
                    return Err(CborError::DuplicateKey);
                }
            }
            write_header(out, 5, encoded.len() as u64);
            for (key_bytes, v) in encoded {
                out.extend_from_slice(&key_bytes);
                encode_into(v, out, depth + 1)?;
            }
        }
        Cbor::Bool(false) => out.push(0xf4),
        Cbor::Bool(true) => out.push(0xf5),
        Cbor::Null => out.push(0xf6),
    }
    Ok(())
}

/// Canonical encoding of `value`.
pub fn encode(value: &Cbor) -> Result<Vec<u8>, CborError> {
    let mut out = Vec::new();
    encode_into(value, &mut out, 0)?;
    Ok(out)
}

// ---------------------------------------------------------------- decode

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Result<u8, CborError> {
        let b = *self.input.get(self.pos).ok_or(CborError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CborError> {
        let end = self.pos.checked_add(n).ok_or(CborError::Truncated)?;
        if end > self.input.len() {
            return Err(CborError::Truncated);
        }
        let slice = &self.input[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Read a length/value argument, enforcing shortest form.
    fn arg(&mut self, additional: u8) -> Result<u64, CborError> {
        match additional {
            0..=23 => Ok(additional as u64),
            24 => {
                let v = self.byte()? as u64;
                if v < 24 {
                    return Err(CborError::NonCanonical);
                }
                Ok(v)
            }
            25 => {
                let v = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64;
                if v < 0x100 {
                    return Err(CborError::NonCanonical);
                }
                Ok(v)
            }
            26 => {
                let v = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64;
                if v < 0x1_0000 {
                    return Err(CborError::NonCanonical);
                }
                Ok(v)
            }
            27 => {
                let v = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
                if v < 0x1_0000_0000 {
                    return Err(CborError::NonCanonical);
                }
                Ok(v)
            }
            // 28-30 reserved, 31 = indefinite length: both non-canonical here.
            _ => Err(CborError::NonCanonical),
        }
    }

    fn item(&mut self, depth: usize) -> Result<Cbor, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError::Depth);
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        match major {
            0 => Ok(Cbor::Uint(self.arg(additional)?)),
            2 => {
                let len = self.arg(additional)? as usize;
                Ok(Cbor::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.arg(additional)? as usize;
                let s = std::str::from_utf8(self.take(len)?).map_err(|_| CborError::Utf8)?;
                Ok(Cbor::Text(s.to_owned()))
            }
            4 => {
                let len = self.arg(additional)? as usize;
                let mut items = Vec::with_capacity(len.min(1024));
                for _ in 0..len {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Cbor::Array(items))
            }
            5 => {
                let len = self.arg(additional)? as usize;
                let mut entries: Vec<(String, Cbor)> = Vec::with_capacity(len.min(1024));
                let mut prev_key: Option<Vec<u8>> = None;
                for _ in 0..len {
                    let key_start = self.pos;
                    let key = match self.item(depth + 1)? {
                        Cbor::Text(k) => k,
                        _ => return Err(CborError::Unsupported),
                    };
                    let key_bytes = self.input[key_start..self.pos].to_vec();
                    if let Some(prev) = &prev_key {
                        if key_bytes == *prev {
                            return Err(CborError::DuplicateKey);
                        }
                        if key_bytes < *prev {
                            return Err(CborError::KeyOrder);
                        }
                    }
                    prev_key = Some(key_bytes);
                    entries.push((key, self.item(depth + 1)?));
                }
                Ok(Cbor::Map(entries))
            }
            7 => match initial {
                0xf4 => Ok(Cbor::Bool(false)),
                0xf5 => Ok(Cbor::Bool(true)),
                0xf6 => Ok(Cbor::Null),
                _ => Err(CborError::Unsupported),
            },
            // major 1 (negative ints) unused by the kernel; 6 = tags.
            _ => Err(CborError::Unsupported),
        }
    }
}

/// Decode one canonical top-level item; rejects trailing bytes.
pub fn decode(input: &[u8]) -> Result<Cbor, CborError> {
    let mut cursor = Cursor { input, pos: 0 };
    let value = cursor.item(0)?;
    if cursor.pos != input.len() {
        return Err(CborError::Trailing);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_byte_identical() {
        let value = Cbor::Map(vec![
            ("zz".into(), Cbor::Uint(300)),
            (
                "a".into(),
                Cbor::Array(vec![Cbor::Bytes(vec![1, 2]), Cbor::Null]),
            ),
        ]);
        let bytes = encode(&value).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(encode(&decoded).unwrap(), bytes);
    }

    #[test]
    fn rejects_duplicate_keys() {
        // {"a": 1, "a": 2}
        assert_eq!(
            decode(&[0xa2, 0x61, 0x61, 0x01, 0x61, 0x61, 0x02]),
            Err(CborError::DuplicateKey)
        );
    }

    #[test]
    fn rejects_non_shortest_uint() {
        // 23 encoded with a one-byte argument
        assert_eq!(decode(&[0x18, 0x17]), Err(CborError::NonCanonical));
    }

    #[test]
    fn rejects_out_of_order_keys() {
        // {"b": 1, "a": 2}
        assert_eq!(
            decode(&[0xa2, 0x61, 0x62, 0x01, 0x61, 0x61, 0x02]),
            Err(CborError::KeyOrder)
        );
    }

    #[test]
    fn rejects_floats_and_tags() {
        assert_eq!(decode(&[0xf9, 0x00, 0x00]), Err(CborError::Unsupported)); // f16
        assert_eq!(decode(&[0xc0, 0x00]), Err(CborError::Unsupported)); // tag 0
    }
}
