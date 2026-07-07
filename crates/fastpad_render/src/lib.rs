use fastpad_viewport::Viewport;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderLine {
    pub display_line_number: Option<u64>,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderPlan {
    pub lines: Vec<RenderLine>,
    pub overlays: Vec<Overlay>,
}

impl RenderPlan {
    pub fn from_viewport(viewport: &Viewport) -> Self {
        Self {
            lines: viewport
                .lines
                .iter()
                .map(|line| RenderLine {
                    display_line_number: line.line_number.map(|number| number + 1),
                    text: line.text.clone(),
                    truncated: line.truncated,
                })
                .collect(),
            overlays: Vec::new(),
        }
    }
}
