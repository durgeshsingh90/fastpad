use anyhow::Result;
use fastpad_file::{ByteOffset, FileHandle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FollowStart {
    Beginning,
    End,
    Offset(ByteOffset),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailEvent {
    pub start: ByteOffset,
    pub bytes: Vec<u8>,
    pub truncated_or_rotated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailFollower {
    offset: ByteOffset,
    paused: bool,
}

impl TailFollower {
    pub fn new(file: &FileHandle, start: FollowStart) -> Result<Self> {
        let offset = match start {
            FollowStart::Beginning => ByteOffset::ZERO,
            FollowStart::End => ByteOffset(file.current_len()?),
            FollowStart::Offset(offset) => ByteOffset(offset.0.min(file.current_len()?)),
        };
        Ok(Self {
            offset,
            paused: false,
        })
    }

    pub fn offset(&self) -> ByteOffset {
        self.offset
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn poll(&mut self, file: &FileHandle, max_bytes: usize) -> Result<Option<TailEvent>> {
        if self.paused {
            return Ok(None);
        }
        let current_len = file.current_len()?;
        if current_len < self.offset.0 {
            self.offset = ByteOffset::ZERO;
            return Ok(Some(TailEvent {
                start: ByteOffset::ZERO,
                bytes: Vec::new(),
                truncated_or_rotated: true,
            }));
        }
        if current_len == self.offset.0 {
            return Ok(None);
        }
        let available = current_len - self.offset.0;
        let read_len = available.min(max_bytes as u64) as usize;
        let start = self.offset;
        let bytes = file.read_at_most(start, read_len)?;
        self.offset = ByteOffset(self.offset.0 + bytes.len() as u64);
        Ok(Some(TailEvent {
            start,
            bytes,
            truncated_or_rotated: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::FileOpenOptions;
    use std::io::Write;

    #[test]
    fn polls_appended_bytes_from_end() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "first").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let mut follower = TailFollower::new(&file, FollowStart::End).unwrap();

        writeln!(tmp, "second").unwrap();
        tmp.flush().unwrap();

        let event = follower.poll(&file, 1024).unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&event.bytes), "second\n");
    }
}
