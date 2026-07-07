use anyhow::Result;
use fastpad_file::{ByteOffset, FileHandle};
use fastpad_tasks::CancellationToken;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineStage {
    Contains {
        needle: String,
        case_sensitive: bool,
        invert: bool,
    },
    Regex {
        pattern: String,
        case_sensitive: bool,
        invert: bool,
    },
    ExtractField {
        delimiter: u8,
        zero_based_field: usize,
    },
    Head {
        lines: usize,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pipeline {
    pub stages: Vec<PipelineStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineOptions {
    pub preview_limit: usize,
    pub chunk_size: usize,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            preview_limit: 10_000,
            chunk_size: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub preview: Vec<String>,
    pub processed_lines: u64,
    pub emitted_lines: u64,
    pub hidden_lines: u64,
    pub bytes_scanned: u64,
    pub cancelled: bool,
}

pub struct PipelineEngine;

impl PipelineEngine {
    pub fn run(
        file: &FileHandle,
        pipeline: &Pipeline,
        options: &PipelineOptions,
        cancel: &CancellationToken,
    ) -> Result<PipelineResult> {
        let mut compiled = CompiledPipeline::compile(pipeline)?;
        let mut preview = Vec::new();
        let mut processed_lines = 0u64;
        let mut emitted_lines = 0u64;
        let mut hidden_lines = 0u64;
        let mut bytes_scanned = 0u64;
        let mut carry = Vec::<u8>::new();
        let mut offset = 0u64;
        let file_len = file.current_len()?;

        while offset < file_len {
            if cancel.is_cancelled() {
                break;
            }
            let chunk = file.read_at_most(ByteOffset(offset), options.chunk_size.max(4096))?;
            if chunk.is_empty() {
                break;
            }
            bytes_scanned = offset + chunk.len() as u64;
            carry.extend_from_slice(&chunk);

            let mut cursor = 0usize;
            while let Some((line_end, terminator_len)) = next_line_break(&carry, cursor) {
                let raw = &carry[cursor..line_end];
                processed_lines += 1;
                match compiled.apply(raw) {
                    Some(line) => {
                        emitted_lines += 1;
                        if preview.len() < options.preview_limit {
                            preview.push(line);
                        }
                    }
                    None => hidden_lines += 1,
                }
                cursor = line_end + terminator_len;
                if compiled.should_stop(emitted_lines as usize) || cancel.is_cancelled() {
                    break;
                }
            }
            carry.drain(0..cursor);
            offset += chunk.len() as u64;

            if compiled.should_stop(emitted_lines as usize) {
                break;
            }
        }

        if !carry.is_empty()
            && !cancel.is_cancelled()
            && !compiled.should_stop(emitted_lines as usize)
        {
            processed_lines += 1;
            match compiled.apply(&carry) {
                Some(line) => {
                    emitted_lines += 1;
                    if preview.len() < options.preview_limit {
                        preview.push(line);
                    }
                }
                None => hidden_lines += 1,
            }
        }

        Ok(PipelineResult {
            preview,
            processed_lines,
            emitted_lines,
            hidden_lines,
            bytes_scanned: bytes_scanned.min(file_len),
            cancelled: cancel.is_cancelled(),
        })
    }
}

struct CompiledPipeline {
    stages: Vec<CompiledStage>,
    head_limit: Option<usize>,
}

impl CompiledPipeline {
    fn compile(pipeline: &Pipeline) -> Result<Self> {
        let mut stages = Vec::new();
        let mut head_limit: Option<usize> = None;
        for stage in &pipeline.stages {
            match stage {
                PipelineStage::Contains {
                    needle,
                    case_sensitive,
                    invert,
                } => stages.push(CompiledStage::Contains {
                    needle: if *case_sensitive {
                        needle.as_bytes().to_vec()
                    } else {
                        ascii_lower(needle.as_bytes())
                    },
                    case_sensitive: *case_sensitive,
                    invert: *invert,
                }),
                PipelineStage::Regex {
                    pattern,
                    case_sensitive,
                    invert,
                } => stages.push(CompiledStage::Regex {
                    regex: RegexBuilder::new(pattern)
                        .case_insensitive(!case_sensitive)
                        .multi_line(false)
                        .build()?,
                    invert: *invert,
                }),
                PipelineStage::ExtractField {
                    delimiter,
                    zero_based_field,
                } => stages.push(CompiledStage::ExtractField {
                    delimiter: *delimiter,
                    zero_based_field: *zero_based_field,
                }),
                PipelineStage::Head { lines } => {
                    head_limit = Some(head_limit.map_or(*lines, |existing| existing.min(*lines)));
                }
            }
        }
        Ok(Self { stages, head_limit })
    }

    fn apply(&mut self, raw: &[u8]) -> Option<String> {
        let mut current = raw.to_vec();
        for stage in &self.stages {
            match stage {
                CompiledStage::Contains {
                    needle,
                    case_sensitive,
                    invert,
                } => {
                    let haystack = if *case_sensitive {
                        current.clone()
                    } else {
                        ascii_lower(&current)
                    };
                    let found = contains_bytes(&haystack, needle);
                    if found == *invert {
                        return None;
                    }
                }
                CompiledStage::Regex { regex, invert } => {
                    let found = regex.is_match(&current);
                    if found == *invert {
                        return None;
                    }
                }
                CompiledStage::ExtractField {
                    delimiter,
                    zero_based_field,
                } => {
                    current = current
                        .split(|byte| byte == delimiter)
                        .nth(*zero_based_field)
                        .unwrap_or_default()
                        .to_vec();
                }
            }
        }
        Some(String::from_utf8_lossy(strip_cr(&current)).into_owned())
    }

    fn should_stop(&self, emitted_lines: usize) -> bool {
        self.head_limit.is_some_and(|limit| emitted_lines >= limit)
    }
}

enum CompiledStage {
    Contains {
        needle: Vec<u8>,
        case_sensitive: bool,
        invert: bool,
    },
    Regex {
        regex: regex::bytes::Regex,
        invert: bool,
    },
    ExtractField {
        delimiter: u8,
        zero_based_field: usize,
    },
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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

fn strip_cr(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|byte| byte.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::{FileHandle, FileOpenOptions};
    use std::io::Write;

    #[test]
    fn filters_and_extracts_fields_streaming() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "INFO,ok\nERROR,bad\nWARN,slow\nERROR,worse\n").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let pipeline = Pipeline {
            stages: vec![
                PipelineStage::Contains {
                    needle: "ERROR".into(),
                    case_sensitive: true,
                    invert: false,
                },
                PipelineStage::ExtractField {
                    delimiter: b',',
                    zero_based_field: 1,
                },
            ],
        };

        let result = PipelineEngine::run(
            &file,
            &pipeline,
            &PipelineOptions::default(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert_eq!(result.preview, vec!["bad", "worse"]);
        assert_eq!(result.emitted_lines, 2);
    }
}
