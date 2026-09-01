use std::io::{self, BufRead};

use crate::app::MAX_INPUT_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    bytes: Vec<u8>,
    oversized: bool,
}

impl RawLine {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn was_oversized(&self) -> bool {
        self.oversized
    }
}

pub struct BoundedLineReader<R> {
    reader: R,
}

impl<R: BufRead> BoundedLineReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn next_line(&mut self) -> io::Result<Option<RawLine>> {
        let mut bytes = Vec::with_capacity(MAX_INPUT_BYTES + 1);
        let mut saw_input = false;
        let mut terminated = false;
        let mut oversized = false;

        loop {
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                break;
            }
            saw_input = true;

            let newline = available.iter().position(|byte| *byte == b'\n');
            let content_length = newline.unwrap_or(available.len());
            let remaining = (MAX_INPUT_BYTES + 1).saturating_sub(bytes.len());
            let copied = content_length.min(remaining);
            bytes.extend_from_slice(&available[..copied]);
            if copied < content_length {
                oversized = true;
            }

            let consumed = newline.map_or(available.len(), |position| position + 1);
            self.reader.consume(consumed);
            if newline.is_some() {
                terminated = true;
                break;
            }
        }

        if !saw_input {
            return Ok(None);
        }
        if terminated && bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        oversized |= bytes.len() > MAX_INPUT_BYTES;

        Ok(Some(RawLine { bytes, oversized }))
    }
}
