use anyhow::Result;
use fastpad_file::{ByteOffset, ByteRange, FileHandle};
use fastpad_tasks::CancellationToken;
use memchr::memmem;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const DEFAULT_SEARCH_CHUNK: usize = 1024 * 1024;
const DEFAULT_CONTEXT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub max_results: usize,
    pub chunk_size: usize,
}

impl SearchQuery {
    pub fn literal(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            max_results: 100_000,
            chunk_size: DEFAULT_SEARCH_CHUNK,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchMatch {
    pub range: ByteRange,
    pub line_start: ByteOffset,
    pub line_end: ByteOffset,
    pub line_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSummary {
    pub matches_seen: u64,
    pub results: Vec<SearchMatch>,
    pub truncated: bool,
    pub bytes_scanned: u64,
    pub elapsed: Duration,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct SearchProgress<'a> {
    pub matches_seen: u64,
    pub results: &'a [SearchMatch],
    pub bytes_scanned: u64,
    pub total_bytes: u64,
    pub elapsed: Duration,
    pub cancelled: bool,
}

pub struct SearchEngine;

impl SearchEngine {
    pub fn search(
        file: &FileHandle,
        query: &SearchQuery,
        cancel: &CancellationToken,
    ) -> Result<SearchSummary> {
        Self::search_with_progress(file, query, cancel, |_| {})
    }

    pub fn search_with_progress(
        file: &FileHandle,
        query: &SearchQuery,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(SearchProgress<'_>),
    ) -> Result<SearchSummary> {
        let start_time = Instant::now();
        if query.pattern.is_empty() {
            return Ok(SearchSummary {
                matches_seen: 0,
                results: Vec::new(),
                truncated: false,
                bytes_scanned: 0,
                elapsed: start_time.elapsed(),
                cancelled: false,
            });
        }

        let mut results = Vec::new();
        let mut matches_seen = 0u64;
        let mut bytes_scanned = 0u64;
        let file_len = file.current_len()?;
        let chunk_size = query.chunk_size.max(4096);
        let overlap = if query.regex {
            DEFAULT_CONTEXT_LIMIT.min(chunk_size / 2)
        } else {
            query
                .pattern
                .len()
                .saturating_sub(1)
                .min(DEFAULT_CONTEXT_LIMIT)
        };
        let mut offset = 0u64;
        let mut carry = Vec::<u8>::new();
        let regex = if query.regex {
            Some(
                RegexBuilder::new(&query.pattern)
                    .case_insensitive(!query.case_sensitive)
                    .multi_line(true)
                    .build()?,
            )
        } else {
            None
        };
        let literal = if !query.regex {
            let bytes = query.pattern.as_bytes().to_vec();
            Some(if query.case_sensitive {
                bytes
            } else {
                ascii_lower(&bytes)
            })
        } else {
            None
        };

        while offset < file_len {
            if cancel.is_cancelled() {
                break;
            }

            let chunk = file.read_at_most(ByteOffset(offset), chunk_size)?;
            if chunk.is_empty() {
                break;
            }
            let combined_start = offset.saturating_sub(carry.len() as u64);
            let mut haystack = Vec::with_capacity(carry.len() + chunk.len());
            haystack.extend_from_slice(&carry);
            haystack.extend_from_slice(&chunk);

            if let Some(regex) = &regex {
                for found in regex.find_iter(&haystack) {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let global_start = combined_start + found.start() as u64;
                    let global_end = combined_start + found.end() as u64;
                    if should_skip_overlap(global_start, global_end, offset, carry.len() as u64) {
                        continue;
                    }
                    if query.whole_word && !is_whole_word(&haystack, found.start(), found.end()) {
                        continue;
                    }
                    matches_seen += 1;
                    if results.len() < query.max_results {
                        results.push(match_context(
                            file,
                            ByteOffset(global_start),
                            (global_end - global_start) as usize,
                        )?);
                    }
                }
            } else if let Some(needle) = &literal {
                let searchable = if query.case_sensitive {
                    haystack.clone()
                } else {
                    ascii_lower(&haystack)
                };
                for found in memmem::find_iter(&searchable, needle) {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let global_start = combined_start + found as u64;
                    let global_end = global_start + needle.len() as u64;
                    if should_skip_overlap(global_start, global_end, offset, carry.len() as u64) {
                        continue;
                    }
                    if query.whole_word && !is_whole_word(&haystack, found, found + needle.len()) {
                        continue;
                    }
                    matches_seen += 1;
                    if results.len() < query.max_results {
                        results.push(match_context(file, ByteOffset(global_start), needle.len())?);
                    }
                }
            }

            bytes_scanned = offset + chunk.len() as u64;
            let keep = overlap.min(haystack.len());
            carry.clear();
            carry.extend_from_slice(&haystack[haystack.len() - keep..]);
            offset += chunk.len() as u64;
            on_progress(SearchProgress {
                matches_seen,
                results: &results,
                bytes_scanned: bytes_scanned.min(file_len),
                total_bytes: file_len,
                elapsed: start_time.elapsed(),
                cancelled: cancel.is_cancelled(),
            });
        }

        Ok(SearchSummary {
            matches_seen,
            truncated: matches_seen as usize > results.len(),
            results,
            bytes_scanned: bytes_scanned.min(file_len),
            elapsed: start_time.elapsed(),
            cancelled: cancel.is_cancelled(),
        })
    }

    pub fn search_bytes(
        bytes: &[u8],
        query: &SearchQuery,
        cancel: &CancellationToken,
    ) -> Result<SearchSummary> {
        let start_time = Instant::now();
        if query.pattern.is_empty() {
            return Ok(SearchSummary {
                matches_seen: 0,
                results: Vec::new(),
                truncated: false,
                bytes_scanned: 0,
                elapsed: start_time.elapsed(),
                cancelled: false,
            });
        }

        let regex = if query.regex {
            Some(
                RegexBuilder::new(&query.pattern)
                    .case_insensitive(!query.case_sensitive)
                    .multi_line(true)
                    .build()?,
            )
        } else {
            None
        };
        let literal = if !query.regex {
            let needle = query.pattern.as_bytes().to_vec();
            Some(if query.case_sensitive {
                needle
            } else {
                ascii_lower(&needle)
            })
        } else {
            None
        };
        let haystack = if query.case_sensitive || query.regex {
            bytes.to_vec()
        } else {
            ascii_lower(bytes)
        };
        let mut results = Vec::new();
        let mut matches_seen = 0u64;

        if let Some(regex) = regex {
            for found in regex.find_iter(bytes) {
                if cancel.is_cancelled() {
                    break;
                }
                if query.whole_word && !is_whole_word(bytes, found.start(), found.end()) {
                    continue;
                }
                matches_seen += 1;
                if results.len() < query.max_results {
                    results.push(match_context_bytes(
                        bytes,
                        ByteOffset(found.start() as u64),
                        found.end() - found.start(),
                    ));
                }
            }
        } else if let Some(needle) = literal {
            for found in memmem::find_iter(&haystack, &needle) {
                if cancel.is_cancelled() {
                    break;
                }
                if query.whole_word && !is_whole_word(bytes, found, found + needle.len()) {
                    continue;
                }
                matches_seen += 1;
                if results.len() < query.max_results {
                    results.push(match_context_bytes(
                        bytes,
                        ByteOffset(found as u64),
                        needle.len(),
                    ));
                }
            }
        }

        Ok(SearchSummary {
            matches_seen,
            truncated: matches_seen as usize > results.len(),
            results,
            bytes_scanned: bytes.len() as u64,
            elapsed: start_time.elapsed(),
            cancelled: cancel.is_cancelled(),
        })
    }
}

fn should_skip_overlap(
    global_start: u64,
    global_end: u64,
    chunk_offset: u64,
    carry_len: u64,
) -> bool {
    carry_len > 0 && global_start < chunk_offset && global_end <= chunk_offset
}

fn match_context(file: &FileHandle, start: ByteOffset, len: usize) -> Result<SearchMatch> {
    let file_len = file.current_len()?;
    let back = DEFAULT_CONTEXT_LIMIT.min(start.0 as usize);
    let context_start = start.0 - back as u64;
    let forward = DEFAULT_CONTEXT_LIMIT;
    let context = file.read_at_most(
        ByteOffset(context_start),
        back.saturating_add(len).saturating_add(forward),
    )?;
    let local_start = (start.0 - context_start) as usize;
    let line_start_local = context[..local_start]
        .iter()
        .rposition(|byte| *byte == b'\n' || *byte == b'\r')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let local_match_end = local_start.saturating_add(len).min(context.len());
    let line_end_local = context[local_match_end..]
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
        .map(|idx| local_match_end + idx)
        .unwrap_or(context.len());

    let line_start = ByteOffset(context_start + line_start_local as u64);
    let line_end = ByteOffset((context_start + line_end_local as u64).min(file_len));
    let line_text =
        String::from_utf8_lossy(&context[line_start_local..line_end_local]).into_owned();

    Ok(SearchMatch {
        range: ByteRange::new(start.0, len),
        line_start,
        line_end,
        line_text,
    })
}

fn match_context_bytes(bytes: &[u8], start: ByteOffset, len: usize) -> SearchMatch {
    let start_idx = start.0 as usize;
    let end_idx = start_idx.saturating_add(len).min(bytes.len());
    let line_start = bytes[..start_idx]
        .iter()
        .rposition(|byte| *byte == b'\n' || *byte == b'\r')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = bytes[end_idx..]
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
        .map(|idx| end_idx + idx)
        .unwrap_or(bytes.len());
    SearchMatch {
        range: ByteRange::new(start.0, len),
        line_start: ByteOffset(line_start as u64),
        line_end: ByteOffset(line_end as u64),
        line_text: String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned(),
    }
}

fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|byte| byte.to_ascii_lowercase()).collect()
}

fn is_whole_word(haystack: &[u8], start: usize, end: usize) -> bool {
    let before = start.checked_sub(1).and_then(|idx| haystack.get(idx));
    let after = haystack.get(end);
    !before.is_some_and(|byte| is_word(*byte)) && !after.is_some_and(|byte| is_word(*byte))
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::FileOpenOptions;
    use std::io::Write;

    #[test]
    fn finds_literal_across_chunk_boundary() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "abcdxxneedlezz").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let token = CancellationToken::new();
        let mut query = SearchQuery::literal("needle");
        query.chunk_size = 8;

        let summary = SearchEngine::search(&file, &query, &token).unwrap();

        assert_eq!(summary.matches_seen, 1);
        assert_eq!(summary.results[0].range.start, ByteOffset(6));
    }

    #[test]
    fn honors_whole_word() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "cat scatter cat").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let token = CancellationToken::new();
        let mut query = SearchQuery::literal("cat");
        query.whole_word = true;

        let summary = SearchEngine::search(&file, &query, &token).unwrap();

        assert_eq!(summary.matches_seen, 2);
    }

    #[test]
    fn search_with_progress_reports_partial_counts() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "needle one\nnope\nneedle two\n").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let token = CancellationToken::new();
        let mut query = SearchQuery::literal("needle");
        query.chunk_size = 8;
        let mut snapshots = Vec::new();

        let summary = SearchEngine::search_with_progress(&file, &query, &token, |progress| {
            snapshots.push((
                progress.bytes_scanned,
                progress.matches_seen,
                progress.results.len(),
            ));
        })
        .unwrap();

        assert_eq!(summary.matches_seen, 2);
        assert!(!snapshots.is_empty());
        assert_eq!(snapshots.last().unwrap().0, file.current_len().unwrap());
        assert_eq!(snapshots.last().unwrap().1, 2);
    }
}
