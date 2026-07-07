use fastpad_viewport::Viewport;
use serde::{Deserialize, Serialize};

pub const DEFAULT_OVERSCAN_LINES: usize = 12;
pub const DEFAULT_MAX_COLUMNS: usize = 4096;
pub const DEFAULT_TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OverlayKind {
    SearchMatch,
    Selection,
    Bookmark,
    Diagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Overlay {
    pub line_index: usize,
    pub byte_start_in_line: usize,
    pub byte_len: usize,
    pub kind: OverlayKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderOptions {
    pub visible_line_count: usize,
    pub overscan_lines: usize,
    pub first_column: usize,
    pub max_columns: usize,
    pub tab_width: usize,
    pub show_line_numbers: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            visible_line_count: 80,
            overscan_lines: DEFAULT_OVERSCAN_LINES,
            first_column: 0,
            max_columns: DEFAULT_MAX_COLUMNS,
            tab_width: DEFAULT_TAB_WIDTH,
            show_line_numbers: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayoutCacheKey {
    pub source_start_byte: u64,
    pub source_end_byte: u64,
    pub text_hash: u64,
    pub first_column: usize,
    pub max_columns: usize,
    pub tab_width: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderLine {
    pub display_line_number: Option<u64>,
    pub text: String,
    pub visible_text: String,
    pub source_start_byte: u64,
    pub source_end_byte: u64,
    pub visible_byte_start_in_line: usize,
    pub visible_byte_end_in_line: usize,
    pub continued_left: bool,
    pub continued_right: bool,
    pub truncated: bool,
    pub in_visible_region: bool,
    pub layout_cache_key: LayoutCacheKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderPlan {
    pub lines: Vec<RenderLine>,
    pub overlays: Vec<Overlay>,
    pub gutter_width_columns: usize,
    pub first_column: usize,
    pub max_columns: usize,
    pub visible_line_count: usize,
    pub overscan_lines: usize,
    pub estimated_content_width_columns: usize,
    pub next_anchor_byte: Option<u64>,
}

impl Default for RenderPlan {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            overlays: Vec::new(),
            gutter_width_columns: 0,
            first_column: 0,
            max_columns: DEFAULT_MAX_COLUMNS,
            visible_line_count: 0,
            overscan_lines: 0,
            estimated_content_width_columns: 0,
            next_anchor_byte: None,
        }
    }
}

impl RenderPlan {
    pub fn from_viewport(viewport: &Viewport) -> Self {
        Self::from_viewport_with_options(
            viewport,
            RenderOptions {
                visible_line_count: viewport.lines.len().max(1),
                overscan_lines: 0,
                ..RenderOptions::default()
            },
        )
    }

    pub fn from_viewport_with_options(viewport: &Viewport, options: RenderOptions) -> Self {
        let visible_line_count = options.visible_line_count.max(1);
        let render_limit = visible_line_count.saturating_add(options.overscan_lines);
        let lines_to_render = viewport.lines.len().min(render_limit);
        let gutter_width_columns = if options.show_line_numbers {
            gutter_width(viewport)
        } else {
            0
        };

        let mut estimated_content_width_columns = gutter_width_columns;
        let mut next_anchor_byte = None;
        Self {
            lines: viewport
                .lines
                .iter()
                .take(lines_to_render)
                .enumerate()
                .map(|(index, line)| {
                    if index + 1 == visible_line_count.min(lines_to_render) {
                        next_anchor_byte = Some(line.end.0);
                    }
                    let clipped = clip_line(
                        &line.text,
                        options.first_column,
                        options.max_columns.max(1),
                        options.tab_width.max(1),
                    );
                    let expanded_columns = expanded_column_width(&line.text, options.tab_width);
                    estimated_content_width_columns = estimated_content_width_columns.max(
                        gutter_width_columns
                            .saturating_add(expanded_columns.min(options.max_columns.max(1))),
                    );
                    let text_hash = stable_text_hash(&line.text);
                    RenderLine {
                        display_line_number: line.line_number.map(|number| number + 1),
                        text: line.text.clone(),
                        visible_text: clipped.text,
                        source_start_byte: line.start.0,
                        source_end_byte: line.end.0,
                        visible_byte_start_in_line: clipped.byte_start,
                        visible_byte_end_in_line: clipped.byte_end,
                        continued_left: clipped.continued_left,
                        continued_right: clipped.continued_right || line.truncated,
                        truncated: line.truncated,
                        in_visible_region: index < visible_line_count,
                        layout_cache_key: LayoutCacheKey {
                            source_start_byte: line.start.0,
                            source_end_byte: line.end.0,
                            text_hash,
                            first_column: options.first_column,
                            max_columns: options.max_columns.max(1),
                            tab_width: options.tab_width.max(1),
                        },
                    }
                })
                .collect(),
            overlays: Vec::new(),
            gutter_width_columns,
            first_column: options.first_column,
            max_columns: options.max_columns.max(1),
            visible_line_count,
            overscan_lines: options.overscan_lines,
            estimated_content_width_columns,
            next_anchor_byte,
        }
    }

    pub fn to_plain_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                let mut text = String::new();
                if self.gutter_width_columns > 0 {
                    let number = line
                        .display_line_number
                        .map(|number| number.to_string())
                        .unwrap_or_default();
                    text.push_str(&format!(
                        "{number:>width$} | ",
                        width = self.gutter_width_columns
                    ));
                }
                if line.continued_left {
                    text.push_str("...");
                }
                text.push_str(&line.visible_text);
                if line.continued_right {
                    text.push_str("...");
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClippedLine {
    text: String,
    byte_start: usize,
    byte_end: usize,
    continued_left: bool,
    continued_right: bool,
}

fn clip_line(text: &str, first_column: usize, max_columns: usize, tab_width: usize) -> ClippedLine {
    let max_columns = max_columns.max(1);
    let tab_width = tab_width.max(1);
    let target_end = first_column.saturating_add(max_columns);
    let mut column = 0usize;
    let mut byte_start = text.len();
    let mut byte_end = text.len();
    let mut visible = String::new();

    for (byte_idx, ch) in text.char_indices() {
        let next_column = advance_column(column, ch, tab_width);
        if next_column <= first_column {
            column = next_column;
            continue;
        }
        if column >= target_end {
            byte_end = byte_idx;
            break;
        }

        if byte_start == text.len() {
            byte_start = byte_idx;
        }
        visible.push(ch);
        column = next_column;
        byte_end = byte_idx + ch.len_utf8();
    }

    if byte_start == text.len() {
        byte_start = text.len();
        byte_end = text.len();
    }

    ClippedLine {
        text: visible,
        byte_start,
        byte_end,
        continued_left: first_column > 0 && byte_start > 0,
        continued_right: byte_end < text.len(),
    }
}

fn advance_column(column: usize, ch: char, tab_width: usize) -> usize {
    if ch == '\t' {
        column + (tab_width - (column % tab_width))
    } else {
        column + 1
    }
}

fn expanded_column_width(text: &str, tab_width: usize) -> usize {
    text.chars().fold(0usize, |column, ch| {
        advance_column(column, ch, tab_width.max(1))
    })
}

fn gutter_width(viewport: &Viewport) -> usize {
    viewport
        .lines
        .iter()
        .filter_map(|line| line.line_number)
        .max()
        .map(|line| (line + 1).to_string().len().max(2))
        .unwrap_or(2)
}

fn stable_text_hash(text: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    text.as_bytes().iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::ByteOffset;
    use fastpad_line_index::LineSlice;
    use fastpad_viewport::{ViewAnchor, Viewport};

    fn viewport(lines: Vec<&str>) -> Viewport {
        let mut offset = 0u64;
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(idx, text)| {
                let start = offset;
                offset += text.len() as u64 + 1;
                LineSlice {
                    line_number: Some(idx as u64),
                    start: ByteOffset(start),
                    end: ByteOffset(offset),
                    text: text.to_string(),
                    truncated: false,
                }
            })
            .collect::<Vec<_>>();

        Viewport {
            anchor: ViewAnchor::Start,
            start: ByteOffset::ZERO,
            end: ByteOffset(offset),
            file_len: offset,
            lines,
        }
    }

    #[test]
    fn renders_only_visible_lines_plus_overscan() {
        let viewport = viewport(vec!["a", "b", "c", "d", "e"]);
        let plan = RenderPlan::from_viewport_with_options(
            &viewport,
            RenderOptions {
                visible_line_count: 2,
                overscan_lines: 1,
                ..RenderOptions::default()
            },
        );

        assert_eq!(plan.lines.len(), 3);
        assert!(plan.lines[0].in_visible_region);
        assert!(plan.lines[1].in_visible_region);
        assert!(!plan.lines[2].in_visible_region);
        assert_eq!(plan.next_anchor_byte, Some(4));
    }

    #[test]
    fn clips_extremely_long_lines_horizontally() {
        let viewport = viewport(vec!["0123456789"]);
        let plan = RenderPlan::from_viewport_with_options(
            &viewport,
            RenderOptions {
                first_column: 3,
                max_columns: 4,
                ..RenderOptions::default()
            },
        );

        assert_eq!(plan.lines[0].visible_text, "3456");
        assert_eq!(plan.lines[0].visible_byte_start_in_line, 3);
        assert_eq!(plan.lines[0].visible_byte_end_in_line, 7);
        assert!(plan.lines[0].continued_left);
        assert!(plan.lines[0].continued_right);
    }

    #[test]
    fn emits_stable_layout_cache_keys() {
        let viewport = viewport(vec!["same"]);
        let first = RenderPlan::from_viewport(&viewport);
        let second = RenderPlan::from_viewport(&viewport);

        assert_eq!(
            first.lines[0].layout_cache_key,
            second.lines[0].layout_cache_key
        );
    }
}
