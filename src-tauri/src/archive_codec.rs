//! Bounded, record-at-a-time JSON decoding for version-one and version-two archives.
//!
//! serde_json's slice decoder avoids the per-byte Read calls of its streaming
//! decoder. Only one node and the small envelope are buffered, never the tree.
use super::{err, Archive, NodeWire};
use disk_scan::{LegacyTreeBuilder, Snapshot};
use std::{
    io::{self, BufRead, Read},
    path::PathBuf,
};

// A flat eight-million-child array is under 64 MiB. Allow room for the other
// fields, while bounding allocations for malformed or hostile records.
pub(super) const MAX_RECORD_BYTES: usize = 128 * 1024 * 1024;
pub(super) const MAX_FIELD_BYTES: usize = 16 * 1024 * 1024;

// Serde's reader decoder grows its string scratch buffer before visiting a field.
pub(super) struct StringLimitedReader<R> {
    pub inner: R,
    limit: usize,
    quoted: bool,
    escaped: bool,
    length: usize,
}
impl<R> StringLimitedReader<R> {
    pub fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            quoted: false,
            escaped: false,
            length: 0,
        }
    }
}
impl<R: Read> Read for StringLimitedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(out)?;
        for &byte in &out[..count] {
            if self.quoted {
                if !self.escaped && byte == b'"' {
                    self.quoted = false;
                } else {
                    self.length += 1;
                    if self.length > self.limit {
                        return Err(io::Error::other("Saved scan string exceeds field limit"));
                    }
                    if self.escaped {
                        self.escaped = false;
                    } else if byte == b'\\' {
                        self.escaped = true;
                    }
                }
            } else if byte == b'"' {
                self.quoted = true;
                self.length = 0;
            }
        }
        Ok(count)
    }
}

#[derive(Debug)]
pub(super) enum DecodeError {
    LegacyRequired,
    Invalid(String),
}
impl From<String> for DecodeError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for DecodeError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}

#[cfg(test)]
pub(super) fn decode(reader: &mut impl BufRead, node_limit: usize) -> Result<Archive, DecodeError> {
    decode_with_limit(reader, node_limit, MAX_RECORD_BYTES)
}

pub(super) fn decode_with_limit(
    reader: &mut impl BufRead,
    node_limit: usize,
    record_limit: usize,
) -> Result<Archive, DecodeError> {
    let mut parser = Parser {
        reader,
        record: Vec::new(),
        nodes: None,
        node_limit,
        record_limit,
    };
    parser.whitespace()?;
    // The slow reader preserves support for serde's positional struct arrays.
    if parser.peek()? != Some(b'{') {
        return Err(DecodeError::LegacyRequired);
    }
    let mut envelope = Vec::new();
    parser.object(&mut envelope, false)?;
    parser.whitespace()?;
    if parser.peek()?.is_some() {
        return Err("Saved scan contains trailing decoded data".into());
    }
    let mut archive: Archive = serde_json::from_slice(&envelope).map_err(err)?;
    if let Some(nodes) = parser.nodes {
        archive.snapshot.nodes = nodes;
    }
    Ok(archive)
}

struct Parser<'a, R> {
    reader: &'a mut R,
    record: Vec<u8>,
    nodes: Option<Snapshot>,
    node_limit: usize,
    record_limit: usize,
}
impl<R: BufRead> Parser<'_, R> {
    fn peek(&mut self) -> Result<Option<u8>, DecodeError> {
        Ok(self.reader.fill_buf().map_err(err)?.first().copied())
    }
    fn whitespace(&mut self) -> Result<(), DecodeError> {
        loop {
            let bytes = self.reader.fill_buf().map_err(err)?;
            let n = bytes
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
                .count();
            let done = n < bytes.len() || bytes.is_empty();
            self.reader.consume(n);
            if done {
                return Ok(());
            }
        }
    }
    fn expect(&mut self, expected: u8) -> Result<(), DecodeError> {
        self.whitespace()?;
        if self.peek()? != Some(expected) {
            return Err("Invalid saved scan JSON structure".into());
        }
        self.reader.consume(1);
        Ok(())
    }
    fn append(out: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), DecodeError> {
        if bytes.len() > limit.saturating_sub(out.len()) {
            return Err(DecodeError::LegacyRequired);
        }
        out.extend_from_slice(bytes);
        Ok(())
    }
    // Scan JSON boundaries in whole decoded buffers; serde_json remains the
    // authority for escapes, numeric syntax, types and nesting correctness.
    fn value(&mut self) -> Result<(), DecodeError> {
        self.whitespace()?;
        self.record.clear();
        let first = self.peek()?.ok_or("Truncated saved scan JSON")?;
        let structured = matches!(first, b'{' | b'[');
        let string_value = first == b'"';
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        loop {
            let bytes = self.reader.fill_buf().map_err(err)?;
            if bytes.is_empty() {
                if structured || string_value || self.record.is_empty() {
                    return Err("Truncated saved scan JSON".into());
                }
                return Ok(());
            }
            let mut consumed = 0;
            let mut done = false;
            for &byte in bytes {
                if !structured && !string_value && matches!(byte, b',' | b'}' | b']') {
                    done = true;
                    break;
                }
                consumed += 1;
                if quoted {
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        quoted = false;
                        if string_value {
                            done = true;
                            break;
                        }
                    }
                } else {
                    match byte {
                        b'"' => quoted = true,
                        b'{' | b'[' => {
                            depth += 1;
                            if depth > 128 {
                                return Err("Saved scan JSON nesting exceeds limit".into());
                            }
                        }
                        b'}' | b']' => {
                            depth = depth
                                .checked_sub(1)
                                .ok_or("Invalid saved scan JSON nesting")?;
                            if structured && depth == 0 {
                                done = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Self::append(&mut self.record, &bytes[..consumed], self.record_limit)?;
            self.reader.consume(consumed);
            if done {
                return Ok(());
            }
        }
    }
    fn object(&mut self, out: &mut Vec<u8>, snapshot: bool) -> Result<(), DecodeError> {
        self.expect(b'{')?;
        Self::append(out, b"{", self.record_limit)?;
        self.whitespace()?;
        if self.peek()? == Some(b'}') {
            self.reader.consume(1);
            return Self::append(out, b"}", self.record_limit);
        }
        let mut first = true;
        loop {
            if !first {
                Self::append(out, b",", self.record_limit)?;
            }
            first = false;
            self.value()?;
            let key: String = serde_json::from_slice(&self.record).map_err(err)?;
            Self::append(out, &self.record, self.record_limit)?;
            self.expect(b':')?;
            Self::append(out, b":", self.record_limit)?;
            if !snapshot && key == "snapshot" {
                // Duplicate snapshot keys are subsequently rejected by serde.
                self.whitespace()?;
                if self.peek()? == Some(b'{') {
                    self.object(out, true)?;
                } else {
                    return Err(DecodeError::LegacyRequired);
                }
            } else if snapshot && key == "nodes" {
                if self.nodes.is_some() {
                    return Err("Duplicate saved scan nodes".into());
                }
                self.node_array()?;
                Self::append(out, b"[]", self.record_limit)?;
            } else {
                self.value()?;
                Self::append(out, &self.record, self.record_limit)?;
            }
            self.whitespace()?;
            match self.peek()? {
                Some(b',') => self.reader.consume(1),
                Some(b'}') => {
                    self.reader.consume(1);
                    return Self::append(out, b"}", self.record_limit);
                }
                _ => return Err("Invalid saved scan JSON object".into()),
            }
        }
    }
    fn node_array(&mut self) -> Result<(), DecodeError> {
        self.expect(b'[')?;
        let mut nodes = LegacyTreeBuilder::new(PathBuf::new());
        let mut remaining_children = self.node_limit.saturating_sub(1);
        self.whitespace()?;
        if self.peek()? == Some(b']') {
            self.reader.consume(1);
            self.nodes = Some(nodes.finish());
            return Ok(());
        }
        loop {
            if nodes.snapshot().node_count() == self.node_limit {
                return Err("Saved scan exceeds node limit".into());
            }
            self.value()?;
            let node: NodeWire = serde_json::from_slice(&self.record).map_err(err)?;
            remaining_children = remaining_children
                .checked_sub(node.children.len())
                .ok_or("Saved scan exceeds child reference limit")?;
            nodes.append(node.into_node())?;
            self.whitespace()?;
            match self.peek()? {
                Some(b',') => self.reader.consume(1),
                Some(b']') => {
                    self.reader.consume(1);
                    self.nodes = Some(nodes.finish());
                    return Ok(());
                }
                _ => return Err("Invalid saved scan node array".into()),
            }
        }
    }
}
