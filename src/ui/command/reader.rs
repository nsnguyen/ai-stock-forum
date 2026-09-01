use std::io::{self, BufRead};

use sha2::{Digest, Sha256};

use crate::{app::MAX_INPUT_BYTES, domain::Sha256Digest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    bytes: Vec<u8>,
    full_byte_length: usize,
    input_digest: Sha256Digest,
}

impl RawLine {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn was_oversized(&self) -> bool {
        self.full_byte_length > MAX_INPUT_BYTES
    }

    pub const fn full_byte_length(&self) -> usize {
        self.full_byte_length
    }

    pub const fn input_digest(&self) -> &Sha256Digest {
        &self.input_digest
    }
}

pub(super) struct LineAccumulator {
    bytes: Vec<u8>,
    full_byte_length: usize,
    hasher: Sha256,
    pending_cr: bool,
    saw_physical_byte: bool,
}

impl LineAccumulator {
    pub(super) fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_INPUT_BYTES + 1),
            full_byte_length: 0,
            hasher: Sha256::new(),
            pending_cr: false,
            saw_physical_byte: false,
        }
    }

    pub(super) fn push_chunk(&mut self, chunk: &[u8]) -> io::Result<Vec<RawLine>> {
        let mut lines = Vec::new();
        for byte in chunk {
            if let Some(line) = self.push_byte(*byte)? {
                lines.push(line);
            }
        }
        Ok(lines)
    }

    pub(super) fn finish_eof(&mut self) -> io::Result<Option<RawLine>> {
        if self.pending_cr {
            self.pending_cr = false;
            self.commit_byte(b'\r')?;
        }
        if self.saw_physical_byte {
            Ok(Some(self.finish_line()))
        } else {
            Ok(None)
        }
    }

    fn push_byte(&mut self, byte: u8) -> io::Result<Option<RawLine>> {
        self.saw_physical_byte = true;
        if self.pending_cr {
            self.pending_cr = false;
            if byte == b'\n' {
                return Ok(Some(self.finish_line()));
            }
            self.commit_byte(b'\r')?;
        }

        match byte {
            b'\r' => {
                self.pending_cr = true;
                Ok(None)
            }
            b'\n' => Ok(Some(self.finish_line())),
            byte => {
                self.commit_byte(byte)?;
                Ok(None)
            }
        }
    }

    fn commit_byte(&mut self, byte: u8) -> io::Result<()> {
        self.full_byte_length = self
            .full_byte_length
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "input length overflow"))?;
        self.hasher.update([byte]);
        if self.bytes.len() < MAX_INPUT_BYTES + 1 {
            self.bytes.push(byte);
        }
        Ok(())
    }

    fn finish_line(&mut self) -> RawLine {
        let digest = std::mem::take(&mut self.hasher).finalize();
        let input_digest = Sha256Digest::parse(&hex::encode(digest))
            .expect("sha256 output is canonical lowercase hexadecimal");
        let line = RawLine {
            bytes: std::mem::replace(
                &mut self.bytes,
                Vec::with_capacity(MAX_INPUT_BYTES + 1),
            ),
            full_byte_length: std::mem::take(&mut self.full_byte_length),
            input_digest,
        };
        self.pending_cr = false;
        self.saw_physical_byte = false;
        line
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
        let mut accumulator = LineAccumulator::new();
        loop {
            let (consumed, line, eof) = {
                let available = self.reader.fill_buf()?;
                if available.is_empty() {
                    (0, None, true)
                } else {
                    let mut consumed = 0;
                    let mut line = None;
                    for byte in available {
                        consumed += 1;
                        if let Some(complete) = accumulator.push_byte(*byte)? {
                            line = Some(complete);
                            break;
                        }
                    }
                    (consumed, line, false)
                }
            };
            self.reader.consume(consumed);
            if let Some(line) = line {
                return Ok(Some(line));
            }
            if eof {
                return accumulator.finish_eof();
            }
        }
    }
}
