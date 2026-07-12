use anyhow::Result;
use fastpad_file::{ByteOffset, FileHandle};
use fastpad_line_index::{LazyLineIndex, LineIndexSnapshot, LineIndexStats, LineSlice};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ViewAnchor {
    Start,
    End,
    Byte(ByteOffset),
    Line(u64),
    Percentage(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportRequest {
    pub anchor: ViewAnchor,
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for ViewportRequest {
    fn default() -> Self {
        Self {
            anchor: ViewAnchor::Start,
            max_lines: 80,
            max_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    pub anchor: ViewAnchor,
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub file_len: u64,
    pub lines: Vec<LineSlice>,
}

impl Viewport {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn next_anchor(&self) -> ViewAnchor {
        self.lines
            .last()
            .map(|line| ViewAnchor::Byte(line.end))
            .unwrap_or(self.anchor)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ViewportEngine {
    index: LazyLineIndex,
}

impl ViewportEngine {
    pub fn new(index: LazyLineIndex) -> Self {
        Self { index }
    }

    pub fn index(&self) -> &LazyLineIndex {
        &self.index
    }

    pub fn index_mut(&mut self) -> &mut LazyLineIndex {
        &mut self.index
    }

    pub fn line_index_stats(&self) -> LineIndexStats {
        self.index.stats()
    }

    pub fn line_index_snapshot(&self) -> LineIndexSnapshot {
        self.index.snapshot()
    }

    pub fn replace_line_index_snapshot(&mut self, snapshot: LineIndexSnapshot) -> LineIndexStats {
        self.index.replace_with_snapshot(snapshot);
        self.index.stats()
    }

    pub fn render(&mut self, file: &FileHandle, request: ViewportRequest) -> Result<Viewport> {
        let file_len = file.current_len()?;
        let start = match request.anchor {
            ViewAnchor::Start => ByteOffset::ZERO,
            ViewAnchor::End => ByteOffset(file_len.saturating_sub(request.max_bytes as u64)),
            ViewAnchor::Byte(offset) => ByteOffset(offset.0.min(file_len)),
            ViewAnchor::Line(line) => self
                .index
                .offset_for_line(file, line)?
                .unwrap_or(ByteOffset(file_len)),
            ViewAnchor::Percentage(percent) => {
                let percent = percent.clamp(0.0, 1.0);
                ByteOffset((file_len as f64 * percent) as u64)
            }
        };
        let lines = self.index.visible_lines_from_offset(
            file,
            start,
            request.max_lines.max(1),
            request.max_bytes.max(4096),
        )?;
        let start = lines.first().map(|line| line.start).unwrap_or(start);
        let end = lines.last().map(|line| line.end).unwrap_or(start);

        Ok(Viewport {
            anchor: request.anchor,
            start,
            end,
            file_len,
            lines,
        })
    }
}
