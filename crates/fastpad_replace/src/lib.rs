use anyhow::Result;
use fastpad_edit::{Edit, EditBuffer};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceQuery {
    pub pattern: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub max_preview: usize,
}

impl ReplaceQuery {
    pub fn literal(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            replacement: replacement.into(),
            regex: false,
            case_sensitive: true,
            max_preview: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplacePreview {
    pub char_range: std::ops::Range<usize>,
    pub original: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceSummary {
    pub replacements: usize,
    pub preview: Vec<ReplacePreview>,
    pub truncated: bool,
}

pub struct ReplaceEngine;

impl ReplaceEngine {
    pub fn preview(buffer: &EditBuffer, query: &ReplaceQuery) -> Result<ReplaceSummary> {
        let text = buffer.text();
        let matches = collect_replacements(&text, query)?;
        let truncated = matches.len() > query.max_preview;
        Ok(ReplaceSummary {
            replacements: matches.len(),
            preview: matches.into_iter().take(query.max_preview).collect(),
            truncated,
        })
    }

    pub fn replace_all(buffer: &mut EditBuffer, query: &ReplaceQuery) -> Result<ReplaceSummary> {
        let text = buffer.text();
        let matches = collect_replacements(&text, query)?;
        let mut edits = Vec::with_capacity(matches.len());
        for found in matches.iter().rev() {
            edits.push(Edit {
                range: found.char_range.clone(),
                inserted: found.replacement.clone(),
                deleted: found.original.clone(),
            });
        }
        buffer.apply_transaction("replace all", edits)?;
        Ok(ReplaceSummary {
            replacements: matches.len(),
            preview: matches.into_iter().take(query.max_preview).collect(),
            truncated: false,
        })
    }
}

fn collect_replacements(text: &str, query: &ReplaceQuery) -> Result<Vec<ReplacePreview>> {
    if query.pattern.is_empty() {
        return Ok(Vec::new());
    }

    if query.regex {
        let regex = RegexBuilder::new(&query.pattern)
            .case_insensitive(!query.case_sensitive)
            .multi_line(true)
            .build()?;
        let mut out = Vec::new();
        for captures in regex.captures_iter(text) {
            let Some(found) = captures.get(0) else {
                continue;
            };
            let mut replacement = String::new();
            captures.expand(&query.replacement, &mut replacement);
            out.push(ReplacePreview {
                char_range: byte_range_to_char_range(text, found.start()..found.end()),
                original: found.as_str().to_string(),
                replacement,
            });
        }
        Ok(out)
    } else {
        let regex = RegexBuilder::new(&regex::escape(&query.pattern))
            .case_insensitive(!query.case_sensitive)
            .multi_line(true)
            .build()?;
        let mut out = Vec::new();
        for found in regex.find_iter(text) {
            out.push(ReplacePreview {
                char_range: byte_range_to_char_range(text, found.start()..found.end()),
                original: found.as_str().to_string(),
                replacement: query.replacement.clone(),
            });
        }
        Ok(out)
    }
}

fn byte_range_to_char_range(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = text[..range.start].chars().count();
    let end = start + text[range.start..range.end].chars().count();
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_replace_uses_capture_groups() {
        let mut buffer = EditBuffer::from_text("id=12 id=42");
        let query = ReplaceQuery {
            pattern: r"id=(\d+)".into(),
            replacement: "value=$1".into(),
            regex: true,
            case_sensitive: true,
            max_preview: 10,
        };

        let summary = ReplaceEngine::replace_all(&mut buffer, &query).unwrap();

        assert_eq!(summary.replacements, 2);
        assert_eq!(buffer.text(), "value=12 value=42");
    }

    #[test]
    fn literal_replace_is_unicode_safe() {
        let mut buffer = EditBuffer::from_text("Cafe CAFE café");
        let query = ReplaceQuery {
            pattern: "café".into(),
            replacement: "coffee".into(),
            regex: false,
            case_sensitive: false,
            max_preview: 10,
        };

        let summary = ReplaceEngine::replace_all(&mut buffer, &query).unwrap();

        assert_eq!(summary.replacements, 1);
        assert_eq!(buffer.text(), "Cafe CAFE coffee");
    }
}
