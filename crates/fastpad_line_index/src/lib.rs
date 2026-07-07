use anyhow::Result;
use fastpad_file::{ByteOffset, FileHandle};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const DEFAULT_BACKTRACK_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineSlice {
    pub line_number: Option<u64>,
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineIndexStats {
    pub contiguous_offsets: usize,
    pub scanned_until: ByteOffset,
    pub complete: bool,
    pub visited_line_starts: usize,
}

#[derive(Debug, Clone)]
pub struct LazyLineIndex {
    line_starts: Vec<u64>,
    scanned_until: u64,
    complete: bool,
    chunk_size: usize,
    visited_line_starts: BTreeSet<u64>,
}

impl Default for LazyLineIndex {
    fn default() -> Self {
        Self::new(1024 * 1024)
    }
}

impl LazyLineIndex {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            line_starts: vec![0],
            scanned_until: 0,
            complete: false,
            chunk_size: chunk_size.max(4096),
            visited_line_starts: BTreeSet::new(),
        }
    }

    pub fn stats(&self) -> LineIndexStats {
        LineIndexStats {
            contiguous_offsets: self.line_starts.len(),
            scanned_until: ByteOffset(self.scanned_until),
            complete: self.complete,
            visited_line_starts: self.visited_line_starts.len(),
        }
    }

    pub fn scan_to_offset(&mut self, file: &FileHandle, target: ByteOffset) -> Result<()> {
        if self.complete || target.0 <= self.scanned_until {
            return Ok(());
        }

        let file_len = file.current_len()?;
        while self.scanned_until < target.0.min(file_len) {
            let chunk = file.read_at_most(ByteOffset(self.scanned_until), self.chunk_size)?;
            if chunk.is_empty() {
                self.complete = true;
                break;
            }

            let mut cursor = 0usize;
            while let Some((break_at, terminator_len)) = next_line_break(&chunk, cursor) {
                self.line_starts
                    .push(self.scanned_until + break_at as u64 + terminator_len as u64);
                cursor = break_at + terminator_len;
            }

            self.scanned_until += chunk.len() as u64;
            if self.scanned_until >= file_len {
                self.complete = true;
            }
        }
        Ok(())
    }

    pub fn offset_for_line(
        &mut self,
        file: &FileHandle,
        zero_based_line: u64,
    ) -> Result<Option<ByteOffset>> {
        while !self.complete && self.line_starts.len() <= zero_based_line as usize {
            self.scan_to_offset(
                file,
                ByteOffset(self.scanned_until.saturating_add(self.chunk_size as u64)),
            )?;
        }
        Ok(self
            .line_starts
            .get(zero_based_line as usize)
            .copied()
            .map(ByteOffset))
    }

    pub fn line_number_for_offset(
        &mut self,
        file: &FileHandle,
        offset: ByteOffset,
    ) -> Result<Option<u64>> {
        if offset.0 > self.scanned_until && !self.complete {
            self.scan_to_offset(file, offset)?;
        }
        if offset.0 > self.scanned_until && !self.complete {
            return Ok(None);
        }
        let idx = match self.line_starts.binary_search(&offset.0) {
            Ok(idx) => idx,
            Err(0) => 0,
            Err(idx) => idx - 1,
        };
        Ok(Some(idx as u64))
    }

    pub fn visible_lines_from_offset(
        &mut self,
        file: &FileHandle,
        offset: ByteOffset,
        max_lines: usize,
        max_bytes: usize,
    ) -> Result<Vec<LineSlice>> {
        let line_start = self.discover_line_start(file, offset)?;
        let base_line_number = if line_start.0 <= self.scanned_until || self.complete {
            self.line_number_for_offset(file, line_start)?
        } else {
            None
        };
        let bytes = file.read_at_most(line_start, max_bytes)?;
        let mut lines = Vec::with_capacity(max_lines);
        let mut cursor = 0usize;
        let mut current_start = line_start.0;
        let mut current_line_number = base_line_number;

        while lines.len() < max_lines && cursor < bytes.len() {
            let (break_at, terminator_len, truncated) = match next_line_break(&bytes, cursor) {
                Some((break_at, terminator_len)) => (break_at, terminator_len, false),
                None => (bytes.len(), 0, true),
            };
            let raw = &bytes[cursor..break_at];
            let text = strip_cr(raw);
            let end = line_start.0 + break_at as u64 + terminator_len as u64;

            self.visited_line_starts.insert(current_start);
            lines.push(LineSlice {
                line_number: current_line_number,
                start: ByteOffset(current_start),
                end: ByteOffset(end),
                text: String::from_utf8_lossy(text).into_owned(),
                truncated,
            });

            if terminator_len == 0 {
                break;
            }
            cursor = break_at + terminator_len;
            current_start = line_start.0 + cursor as u64;
            current_line_number = current_line_number.map(|line| line + 1);
        }

        Ok(lines)
    }

    fn discover_line_start(&mut self, file: &FileHandle, offset: ByteOffset) -> Result<ByteOffset> {
        let file_len = file.current_len()?;
        let offset = ByteOffset(offset.0.min(file_len));
        if offset.0 == 0 || file_len == 0 {
            return Ok(ByteOffset::ZERO);
        }

        if offset.0 <= self.scanned_until || self.complete {
            self.scan_to_offset(file, offset)?;
            let idx = match self.line_starts.binary_search(&offset.0) {
                Ok(idx) => idx,
                Err(0) => 0,
                Err(idx) => idx - 1,
            };
            return Ok(ByteOffset(self.line_starts[idx]));
        }

        let backtrack = DEFAULT_BACKTRACK_BYTES.min(offset.0 as usize);
        let start = offset.0 - backtrack as u64;
        let bytes = file.read_at_most(ByteOffset(start), backtrack)?;
        for idx in (0..bytes.len()).rev() {
            match bytes[idx] {
                b'\n' => return Ok(ByteOffset(start + idx as u64 + 1)),
                b'\r' => {
                    let next = bytes.get(idx + 1).copied();
                    return Ok(ByteOffset(
                        start + idx as u64 + if next == Some(b'\n') { 2 } else { 1 },
                    ));
                }
                _ => {}
            }
        }
        Ok(ByteOffset(start))
    }
}

fn strip_cr(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

fn next_line_break(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut idx = start;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\n' => return Some((idx, 1)),
            b'\r' if bytes.get(idx + 1) == Some(&b'\n') => return Some((idx, 2)),
            b'\r' => return Some((idx, 1)),
            _ => idx += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::FileOpenOptions;
    use std::io::Write;

    #[test]
    fn discovers_visible_lines_without_full_scan_for_late_offset() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "one\ntwo\nthree\nfour\n").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let mut index = LazyLineIndex::new(4);

        let lines = index
            .visible_lines_from_offset(&file, ByteOffset(10), 2, 64)
            .unwrap();

        assert_eq!(lines[0].text, "three");
        assert_eq!(index.stats().scanned_until, ByteOffset(0));
    }

    #[test]
    fn resolves_line_to_offset_progressively() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "a\nb\nc\n").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let mut index = LazyLineIndex::new(2);

        assert_eq!(
            index.offset_for_line(&file, 2).unwrap(),
            Some(ByteOffset(4))
        );
        assert!(index.stats().scanned_until > ByteOffset(0));
    }
}
