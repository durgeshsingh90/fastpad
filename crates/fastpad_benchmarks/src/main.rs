use anyhow::{bail, Context, Result};
use fastpad_core::{AppSettings, CommandRegistry, DocumentManager, OpenIntent, OpenTabRequest};
use fastpad_diagnostics::RuntimeBudget;
use fastpad_edit::EditBuffer;
use fastpad_file::{FileHandle, FileOpenOptions};
use fastpad_render::{RenderOptions, RenderPlan};
use fastpad_search::{SearchEngine, SearchQuery};
use fastpad_tasks::CancellationToken;
use fastpad_viewport::ViewportRequest;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_FIXTURE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_DENSE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_ITERATIONS: usize = 3;
const DEFAULT_TYPING_OPS: usize = 1_000;
const FIXTURE_LINE: &[u8] = b"INFO target alpha beta gamma 0123456789\n";

#[derive(Debug, Clone)]
struct Config {
    fixture: PathBuf,
    output: Option<PathBuf>,
    bytes: u64,
    dense_limit_bytes: u64,
    iterations: usize,
    typing_ops: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fixture: PathBuf::from("target/fastpad-benchmark-fixture.log"),
            output: None,
            bytes: DEFAULT_FIXTURE_BYTES,
            dense_limit_bytes: DEFAULT_DENSE_LIMIT_BYTES,
            iterations: DEFAULT_ITERATIONS,
            typing_ops: DEFAULT_TYPING_OPS,
        }
    }
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    generated_unix_ms: u128,
    fixture_path: String,
    fixture_bytes: u64,
    iterations: usize,
    typing_ops: usize,
    runtime_budget: RuntimeBudget,
    cases: Vec<BenchmarkCase>,
}

#[derive(Debug, Serialize)]
struct BenchmarkCase {
    name: String,
    iterations: usize,
    avg_ms: f64,
    min_ms: f64,
    max_ms: f64,
    total_ms: f64,
    bytes: Option<u64>,
    throughput_mib_s: Option<f64>,
    peak_rss_before_bytes: Option<u64>,
    peak_rss_after_bytes: Option<u64>,
    peak_rss_delta_bytes: Option<i128>,
    details: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct Timing {
    iterations: usize,
    avg_ms: f64,
    min_ms: f64,
    max_ms: f64,
    total_ms: f64,
    peak_rss_before_bytes: Option<u64>,
    peak_rss_after_bytes: Option<u64>,
}

fn main() -> Result<()> {
    let config = Config::parse(env::args().skip(1))?;
    ensure_fixture(&config.fixture, config.bytes, config.dense_limit_bytes)
        .with_context(|| format!("prepare {}", config.fixture.display()))?;

    let cases = vec![
        bench_startup(config.iterations)?,
        bench_open_document(&config)?,
        bench_first_viewport_and_render(&config)?,
        bench_search(&config)?,
        bench_typing_latency(&config)?,
    ];

    let report = BenchmarkReport {
        generated_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        fixture_path: config.fixture.display().to_string(),
        fixture_bytes: config.bytes,
        iterations: config.iterations,
        typing_ops: config.typing_ops,
        runtime_budget: RuntimeBudget::default(),
        cases,
    };

    let json = serde_json::to_string_pretty(&report)?;
    if let Some(output) = &config.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, &json).with_context(|| format!("write {}", output.display()))?;
    }
    println!("{json}");

    Ok(())
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--fixture" => {
                    config.fixture = PathBuf::from(value_for(&mut args, "--fixture")?);
                }
                "--output" => {
                    config.output = Some(PathBuf::from(value_for(&mut args, "--output")?));
                }
                "--bytes" => {
                    config.bytes = parse_size(&value_for(&mut args, "--bytes")?)?;
                }
                "--dense-limit" => {
                    config.dense_limit_bytes = parse_size(&value_for(&mut args, "--dense-limit")?)?;
                }
                "--iterations" => {
                    config.iterations = value_for(&mut args, "--iterations")?
                        .parse::<usize>()
                        .context("parse --iterations")?
                        .max(1);
                }
                "--typing-ops" => {
                    config.typing_ops = value_for(&mut args, "--typing-ops")?
                        .parse::<usize>()
                        .context("parse --typing-ops")?
                        .max(1);
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument {unknown}"),
            }
        }

        Ok(config)
    }
}

fn value_for(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("missing value for {flag}"))
}

fn print_help() {
    println!(
        "Usage: fastpad-benchmarks [--bytes 8M] [--iterations 3] [--fixture PATH] [--output PATH]\n\
         Size suffixes: K, M, G. Defaults are quick and safe; use --bytes 1G or --bytes 10G for large-file runs."
    );
}

fn parse_size(value: &str) -> Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("empty size");
    }

    let (number, multiplier) = match trimmed.as_bytes().last().copied() {
        Some(b'k' | b'K') => (&trimmed[..trimmed.len() - 1], 1024u64),
        Some(b'm' | b'M') => (&trimmed[..trimmed.len() - 1], 1024u64 * 1024),
        Some(b'g' | b'G') => (&trimmed[..trimmed.len() - 1], 1024u64 * 1024 * 1024),
        _ => (trimmed, 1u64),
    };

    Ok(number
        .parse::<u64>()
        .with_context(|| format!("parse size {value}"))?
        .saturating_mul(multiplier))
}

fn ensure_fixture(path: &Path, bytes: u64, dense_limit_bytes: u64) -> Result<()> {
    if path.metadata().map(|metadata| metadata.len()).unwrap_or(0) == bytes {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if bytes <= dense_limit_bytes {
        write_dense_fixture(path, bytes)
    } else {
        write_sparse_fixture(path, bytes)
    }
}

fn write_dense_fixture(path: &Path, bytes: u64) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    let mut written = 0u64;
    while written < bytes {
        let remaining = (bytes - written) as usize;
        let chunk = if remaining < FIXTURE_LINE.len() {
            &FIXTURE_LINE[..remaining]
        } else {
            FIXTURE_LINE
        };
        writer.write_all(chunk)?;
        written += chunk.len() as u64;
    }
    writer.flush()?;
    Ok(())
}

fn write_sparse_fixture(path: &Path, bytes: u64) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(path)?;
    file.set_len(bytes)?;

    for offset in sparse_offsets(bytes) {
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(FIXTURE_LINE)?;
    }
    file.flush()?;
    Ok(())
}

fn sparse_offsets(bytes: u64) -> Vec<u64> {
    let last_offset = bytes.saturating_sub(FIXTURE_LINE.len() as u64);
    let mut offsets = vec![0, bytes / 2, last_offset];
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

fn bench_startup(iterations: usize) -> Result<BenchmarkCase> {
    let mut last_tabs = 0usize;
    let timing = measure(iterations, || {
        let settings = AppSettings::default();
        let manager = DocumentManager::new(settings);
        let _commands = CommandRegistry::default();
        last_tabs = manager.tab_count();
        Ok(())
    })?;

    let mut details = BTreeMap::new();
    details.insert("tabs".into(), json!(last_tabs));
    Ok(case("startup_core", timing, None, details))
}

fn bench_open_document(config: &Config) -> Result<BenchmarkCase> {
    let mut last_mode = String::new();
    let mut last_tabs = 0usize;
    let timing = measure(config.iterations, || {
        let mut manager = DocumentManager::new(AppSettings::default());
        let request = manager.begin_open_tab(&config.fixture, OpenIntent::default())?;
        match request {
            OpenTabRequest::Existing(_) => {}
            OpenTabRequest::Pending(pending) => {
                let document = pending.open()?;
                last_mode = document.mode().label().to_string();
                manager.finish_open_tab(document);
            }
        }
        last_tabs = manager.tab_count();
        Ok(())
    })?;

    let mut details = BTreeMap::new();
    details.insert("mode".into(), json!(last_mode));
    details.insert("tabs".into(), json!(last_tabs));
    Ok(case("open_document", timing, Some(config.bytes), details))
}

fn bench_first_viewport_and_render(config: &Config) -> Result<BenchmarkCase> {
    let mut manager = DocumentManager::new(AppSettings {
        analysis_threshold_bytes: 1,
        ..AppSettings::default()
    });
    let request = manager.begin_open_tab(&config.fixture, OpenIntent::default())?;
    let OpenTabRequest::Pending(pending) = request else {
        bail!("benchmark fixture unexpectedly reused before open");
    };
    manager.finish_open_tab(pending.open()?);

    let mut last_lines = 0usize;
    let mut last_bytes = 0u64;
    let timing = measure(config.iterations, || {
        let active = manager.active().context("active document")?;
        let mut document = active.write();
        let viewport = document.viewport(ViewportRequest {
            anchor: fastpad_viewport::ViewAnchor::Start,
            max_lines: 120,
            max_bytes: 512 * 1024,
        })?;
        let plan = RenderPlan::from_viewport_with_options(
            &viewport,
            RenderOptions {
                visible_line_count: 120,
                overscan_lines: 12,
                ..RenderOptions::default()
            },
        );
        last_lines = plan.lines.len();
        last_bytes = viewport.end.0.saturating_sub(viewport.start.0);
        Ok(())
    })?;

    let mut details = BTreeMap::new();
    details.insert("rendered_lines".into(), json!(last_lines));
    details.insert("viewport_bytes".into(), json!(last_bytes));
    Ok(case(
        "first_viewport_render",
        timing,
        Some(last_bytes),
        details,
    ))
}

fn bench_search(config: &Config) -> Result<BenchmarkCase> {
    let file = FileHandle::open(&config.fixture, FileOpenOptions::default())?;
    let mut query = SearchQuery::literal("target");
    query.max_results = 64;
    query.chunk_size = 1024 * 1024;

    let mut matches_seen = 0u64;
    let mut bytes_scanned = 0u64;
    let timing = measure(config.iterations, || {
        let summary = SearchEngine::search(&file, &query, &CancellationToken::new())?;
        matches_seen = summary.matches_seen;
        bytes_scanned = summary.bytes_scanned;
        Ok(())
    })?;

    let mut details = BTreeMap::new();
    details.insert("pattern".into(), json!(query.pattern));
    details.insert("matches_seen".into(), json!(matches_seen));
    details.insert("bytes_scanned".into(), json!(bytes_scanned));
    Ok(case(
        "search_literal_full_scan",
        timing,
        Some(bytes_scanned),
        details,
    ))
}

fn bench_typing_latency(config: &Config) -> Result<BenchmarkCase> {
    let seed = "line 0000000000\n".repeat(4096);
    let mut last_chars = 0usize;
    let timing = measure(config.iterations, || {
        let mut buffer = EditBuffer::from_text(&seed);
        for _ in 0..config.typing_ops {
            buffer.insert(buffer.len_chars(), "x")?;
        }
        last_chars = buffer.len_chars();
        Ok(())
    })?;

    let mut details = BTreeMap::new();
    details.insert("typing_ops".into(), json!(config.typing_ops));
    details.insert("final_chars".into(), json!(last_chars));
    details.insert(
        "avg_ms_per_insert".into(),
        json!(timing.avg_ms / config.typing_ops as f64),
    );
    Ok(case("typing_latency", timing, None, details))
}

fn measure(iterations: usize, mut op: impl FnMut() -> Result<()>) -> Result<Timing> {
    let iterations = iterations.max(1);
    let rss_before = peak_rss_bytes();
    let mut samples = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        op()?;
        samples.push(start.elapsed());
    }

    let rss_after = peak_rss_bytes();
    Ok(timing(samples, rss_before, rss_after))
}

fn timing(
    samples: Vec<Duration>,
    peak_rss_before_bytes: Option<u64>,
    peak_rss_after_bytes: Option<u64>,
) -> Timing {
    let iterations = samples.len().max(1);
    let mut min_ms = f64::MAX;
    let mut max_ms = 0.0f64;
    let mut total_ms = 0.0f64;

    for sample in samples {
        let ms = duration_ms(sample);
        min_ms = min_ms.min(ms);
        max_ms = max_ms.max(ms);
        total_ms += ms;
    }

    Timing {
        iterations,
        avg_ms: total_ms / iterations as f64,
        min_ms,
        max_ms,
        total_ms,
        peak_rss_before_bytes,
        peak_rss_after_bytes,
    }
}

fn case(
    name: impl Into<String>,
    timing: Timing,
    bytes: Option<u64>,
    details: BTreeMap<String, Value>,
) -> BenchmarkCase {
    let throughput_mib_s = bytes.and_then(|bytes| {
        if timing.avg_ms > 0.0 {
            Some((bytes as f64 / 1024.0 / 1024.0) / (timing.avg_ms / 1000.0))
        } else {
            None
        }
    });
    let peak_rss_delta_bytes = match (timing.peak_rss_before_bytes, timing.peak_rss_after_bytes) {
        (Some(before), Some(after)) => Some(after as i128 - before as i128),
        _ => None,
    };

    BenchmarkCase {
        name: name.into(),
        iterations: timing.iterations,
        avg_ms: timing.avg_ms,
        min_ms: timing.min_ms,
        max_ms: timing.max_ms,
        total_ms: timing.total_ms,
        bytes,
        throughput_mib_s,
        peak_rss_before_bytes: timing.peak_rss_before_bytes,
        peak_rss_after_bytes: timing.peak_rss_after_bytes,
        peak_rss_delta_bytes,
        details,
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn peak_rss_bytes() -> Option<u64> {
    unsafe {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed().assume_init();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return None;
        }

        #[cfg(target_os = "macos")]
        {
            Some(usage.ru_maxrss as u64)
        }
        #[cfg(target_os = "linux")]
        {
            Some((usage.ru_maxrss as u64).saturating_mul(1024))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size_suffixes() {
        assert_eq!(parse_size("42").unwrap(), 42);
        assert_eq!(parse_size("2K").unwrap(), 2 * 1024);
        assert_eq!(parse_size("3M").unwrap(), 3 * 1024 * 1024);
        assert_eq!(parse_size("4G").unwrap(), 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn sparse_offsets_stay_inside_file() {
        let offsets = sparse_offsets(1024);
        assert_eq!(offsets.first().copied(), Some(0));
        assert!(offsets.iter().all(|offset| *offset < 1024));
    }
}
