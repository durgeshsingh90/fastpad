use anyhow::{Context, Result};
use memchr::memchr2_iter;
use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;
pub const DEFAULT_SAMPLE_SIZE: usize = 64 * 1024;
pub const DEFAULT_LONG_LINE_WARNING_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ByteOffset(pub u64);

impl ByteOffset {
    pub const ZERO: Self = Self(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: ByteOffset,
    pub len: usize,
}

impl ByteRange {
    pub fn new(start: u64, len: usize) -> Self {
        Self {
            start: ByteOffset(start),
            len,
        }
    }

    pub fn end(self) -> u64 {
        self.start.0.saturating_add(self.len as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncodingHint {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Unknown8Bit,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Binary,
    Csv,
    Json,
    Log,
    SourceCode,
    SqlDump,
    Tsv,
    Xml,
    Text,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIntelligence {
    pub encoding: EncodingHint,
    pub line_ending: LineEnding,
    pub binary_confidence: f32,
    pub has_bom: bool,
    pub likely_text: bool,
    pub very_long_line_warning: bool,
    pub sampled_line_count: usize,
    pub longest_line_bytes: usize,
    pub average_line_bytes: usize,
    pub sample_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub len: u64,
    pub readonly: bool,
    pub modified: Option<SystemTime>,
    pub kind: FileKind,
    pub intelligence: FileIntelligence,
}

#[derive(Debug, Clone)]
pub struct FileOpenOptions {
    pub prefer_mmap: bool,
    pub writable: bool,
    pub chunk_size: usize,
    pub sample_size: usize,
}

impl Default for FileOpenOptions {
    fn default() -> Self {
        Self {
            prefer_mmap: true,
            writable: false,
            chunk_size: DEFAULT_CHUNK_SIZE,
            sample_size: DEFAULT_SAMPLE_SIZE,
        }
    }
}

pub struct FileHandle {
    path: PathBuf,
    file: File,
    mmap: Option<Mmap>,
    metadata: FileMetadata,
    chunk_size: usize,
}

impl FileHandle {
    pub fn open(path: impl AsRef<Path>, options: FileOpenOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut open = OpenOptions::new();
        open.read(true).write(options.writable);
        let file = open
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        let raw_metadata = file
            .metadata()
            .with_context(|| format!("metadata {}", path.display()))?;
        let len = raw_metadata.len();
        let sample = read_sample(&file, min(options.sample_size as u64, len) as usize)?;
        let intelligence = inspect_sample(&sample);
        let kind = detect_file_kind(&path, &intelligence);
        let mmap = if options.prefer_mmap && len > 0 && raw_metadata.is_file() {
            // SAFETY: The map is read-only and tied to a file handle stored on FileHandle.
            unsafe { MmapOptions::new().map(&file).ok() }
        } else {
            None
        };
        let metadata = FileMetadata {
            path: path.clone(),
            len,
            readonly: raw_metadata.permissions().readonly(),
            modified: raw_metadata.modified().ok(),
            kind,
            intelligence,
        };

        Ok(Self {
            path,
            file,
            mmap,
            metadata,
            chunk_size: options.chunk_size.max(4096),
        })
    }

    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> u64 {
        self.metadata.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn current_len(&self) -> Result<u64> {
        Ok(fs::metadata(&self.path)
            .with_context(|| format!("metadata {}", self.path.display()))?
            .len())
    }

    pub fn read_at_most(&self, start: ByteOffset, max_len: usize) -> Result<Vec<u8>> {
        let start = min(start.0, self.current_len()?);
        let available = self.current_len()?.saturating_sub(start);
        let len = min(max_len as u64, available) as usize;
        self.read_range(ByteRange {
            start: ByteOffset(start),
            len,
        })
    }

    pub fn read_range(&self, range: ByteRange) -> Result<Vec<u8>> {
        let current_len = self.current_len()?;
        let start = min(range.start.0, current_len);
        let end = min(range.end(), current_len);
        if end <= start {
            return Ok(Vec::new());
        }

        if let Some(mmap) = &self.mmap {
            if end <= mmap.len() as u64 {
                return Ok(mmap[start as usize..end as usize].to_vec());
            }
        }

        let mut file = self
            .file
            .try_clone()
            .with_context(|| format!("clone file handle {}", self.path.display()))?;
        file.seek(SeekFrom::Start(start))?;
        let mut out = vec![0; (end - start) as usize];
        file.read_exact(&mut out)?;
        Ok(out)
    }

    pub fn read_lossy(&self, range: ByteRange) -> Result<String> {
        let bytes = self.read_range(range)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn initial_window(&self, max_bytes: usize) -> Result<Vec<u8>> {
        self.read_at_most(ByteOffset::ZERO, max_bytes)
    }

    pub fn tail_window(&self, max_bytes: usize) -> Result<(ByteOffset, Vec<u8>)> {
        let len = self.current_len()?;
        let start = len.saturating_sub(max_bytes as u64);
        Ok((
            ByteOffset(start),
            self.read_at_most(ByteOffset(start), max_bytes)?,
        ))
    }

    pub fn read_entire_if_under(&self, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let len = self.current_len()?;
        if len > max_bytes {
            return Ok(None);
        }
        self.read_range(ByteRange::new(0, len as usize)).map(Some)
    }
}

pub fn atomic_write(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fastpad-file");
    let tmp_path = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));

    {
        let mut tmp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        tmp.write_all(bytes)?;
        tmp.sync_all()?;
    }

    fs::rename(&tmp_path, path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))?;
    Ok(())
}

fn read_sample(file: &File, len: usize) -> Result<Vec<u8>> {
    let mut sample_file = file.try_clone()?;
    sample_file.seek(SeekFrom::Start(0))?;
    let mut sample = vec![0; len];
    let read = sample_file.read(&mut sample)?;
    sample.truncate(read);
    Ok(sample)
}

pub fn inspect_sample(sample: &[u8]) -> FileIntelligence {
    let (encoding, has_bom) = detect_encoding(sample);
    let binary_confidence = detect_binary_confidence(sample);
    let line_ending = detect_line_ending(sample);
    let likely_text = !matches!(encoding, EncodingHint::Binary) && binary_confidence < 0.25;
    let line_stats = sample_line_stats(sample);
    let very_long_line_warning = line_stats.longest_line_bytes > DEFAULT_LONG_LINE_WARNING_BYTES;

    FileIntelligence {
        encoding: if binary_confidence > 0.65 {
            EncodingHint::Binary
        } else {
            encoding
        },
        line_ending,
        binary_confidence,
        has_bom,
        likely_text,
        very_long_line_warning,
        sampled_line_count: line_stats.line_count,
        longest_line_bytes: line_stats.longest_line_bytes,
        average_line_bytes: line_stats.average_line_bytes,
        sample_bytes: sample.len(),
    }
}

pub fn detect_file_kind(path: &Path, intelligence: &FileIntelligence) -> FileKind {
    if !intelligence.likely_text || matches!(intelligence.encoding, EncodingHint::Binary) {
        return FileKind::Binary;
    }

    let Some(extension) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
    else {
        return FileKind::Text;
    };

    match extension.as_str() {
        "csv" => FileKind::Csv,
        "json" | "jsonl" => FileKind::Json,
        "log" | "out" | "trace" => FileKind::Log,
        "sql" | "dump" => FileKind::SqlDump,
        "tsv" => FileKind::Tsv,
        "xml" | "html" | "htm" => FileKind::Xml,
        "c" | "cc" | "cpp" | "cs" | "css" | "go" | "h" | "hpp" | "java" | "js" | "jsx" | "kt"
        | "lua" | "m" | "mm" | "php" | "py" | "rb" | "rs" | "scala" | "sh" | "swift" | "toml"
        | "ts" | "tsx" | "yaml" | "yml" => FileKind::SourceCode,
        "md" | "markdown" | "txt" | "text" => FileKind::Text,
        _ => FileKind::Unknown,
    }
}

fn detect_encoding(sample: &[u8]) -> (EncodingHint, bool) {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (EncodingHint::Utf8Bom, true);
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return (EncodingHint::Utf16Le, true);
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return (EncodingHint::Utf16Be, true);
    }
    if std::str::from_utf8(sample).is_ok() {
        (EncodingHint::Utf8, false)
    } else {
        (EncodingHint::Unknown8Bit, false)
    }
}

fn detect_binary_confidence(sample: &[u8]) -> f32 {
    if sample.is_empty() {
        return 0.0;
    }
    let nul_count = sample.iter().filter(|byte| **byte == 0).count();
    let control_count = sample
        .iter()
        .filter(|byte| {
            let byte = **byte;
            byte < 0x09 || (byte > 0x0D && byte < 0x20)
        })
        .count();
    ((nul_count * 4 + control_count) as f32 / sample.len() as f32).min(1.0)
}

fn detect_line_ending(sample: &[u8]) -> LineEnding {
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut cr = 0usize;
    let mut i = 0usize;
    while i < sample.len() {
        match sample[i] {
            b'\r' if sample.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 2;
            }
            b'\r' => {
                cr += 1;
                i += 1;
            }
            b'\n' => {
                lf += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let kinds = [lf > 0, crlf > 0, cr > 0]
        .into_iter()
        .filter(|present| *present)
        .count();
    match (kinds, lf, crlf, cr) {
        (0, _, _, _) => LineEnding::Unknown,
        (1, _, 0, 0) => LineEnding::Lf,
        (1, 0, _, 0) => LineEnding::Crlf,
        (1, 0, 0, _) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SampleLineStats {
    line_count: usize,
    longest_line_bytes: usize,
    average_line_bytes: usize,
}

fn sample_line_stats(sample: &[u8]) -> SampleLineStats {
    if sample.is_empty() {
        return SampleLineStats {
            line_count: 0,
            longest_line_bytes: 0,
            average_line_bytes: 0,
        };
    }

    let mut line_count = 0usize;
    let mut longest = 0usize;
    let mut start = 0usize;
    let mut line_bytes = 0usize;
    let mut idx_iter = memchr2_iter(b'\n', b'\r', sample).peekable();
    while let Some(idx) = idx_iter.next() {
        longest = longest.max(idx.saturating_sub(start));
        line_bytes += idx.saturating_sub(start);
        line_count += 1;
        if sample[idx] == b'\r' && sample.get(idx + 1) == Some(&b'\n') {
            let _ = idx_iter.next_if_eq(&(idx + 1));
            start = idx + 2;
        } else {
            start = idx + 1;
        }
    }

    if start < sample.len() {
        let last_len = sample.len().saturating_sub(start);
        longest = longest.max(last_len);
        line_bytes += last_len;
        line_count += 1;
    }

    SampleLineStats {
        line_count,
        longest_line_bytes: longest,
        average_line_bytes: line_bytes / line_count.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_only_requested_range() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "alpha\nbeta\ngamma\n").unwrap();
        let handle = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();

        assert_eq!(handle.read_lossy(ByteRange::new(6, 4)).unwrap(), "beta");
    }

    #[test]
    fn detects_mixed_line_endings() {
        let info = inspect_sample(b"a\nb\r\nc\rd");
        assert_eq!(info.line_ending, LineEnding::Mixed);
        assert!(info.likely_text);
        assert_eq!(info.sampled_line_count, 4);
        assert_eq!(info.longest_line_bytes, 1);
        assert_eq!(info.average_line_bytes, 1);
    }

    #[test]
    fn refuses_full_read_above_limit() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "123456789").unwrap();
        let handle = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();

        assert!(handle.read_entire_if_under(4).unwrap().is_none());
        assert_eq!(
            handle.read_entire_if_under(32).unwrap().unwrap(),
            b"123456789"
        );
    }

    #[test]
    fn detects_long_lines_from_sample_without_full_scan() {
        let sample = vec![b'x'; DEFAULT_LONG_LINE_WARNING_BYTES + 1];
        let info = inspect_sample(&sample);

        assert!(info.very_long_line_warning);
        assert_eq!(info.longest_line_bytes, DEFAULT_LONG_LINE_WARNING_BYTES + 1);
        assert_eq!(info.sampled_line_count, 1);
    }

    #[test]
    fn classifies_file_kind_from_extension_after_binary_check() {
        let text_info = inspect_sample(b"a,b\n1,2\n");
        assert_eq!(
            detect_file_kind(Path::new("events.csv"), &text_info),
            FileKind::Csv
        );
        assert_eq!(
            detect_file_kind(Path::new("main.rs"), &text_info),
            FileKind::SourceCode
        );

        let binary_info = inspect_sample(&[0, 1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(
            detect_file_kind(Path::new("events.csv"), &binary_info),
            FileKind::Binary
        );
    }
}
