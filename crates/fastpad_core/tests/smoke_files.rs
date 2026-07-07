use fastpad_core::{AppSettings, Document, DocumentId, EditorMode, OpenIntent};
use fastpad_file::{ByteOffset, FileHandle, FileOpenOptions};
use fastpad_search::{SearchEngine, SearchQuery};
use fastpad_tasks::CancellationToken;
use fastpad_viewport::{ViewAnchor, ViewportRequest};
use std::path::PathBuf;

#[test]
fn smoke_open_configured_files() {
    let Some(paths) = std::env::var_os("FASTPAD_SMOKE_FILES") else {
        eprintln!("FASTPAD_SMOKE_FILES not set; skipping attached-file smoke test");
        return;
    };

    let settings = AppSettings::default();
    let mut checked = 0usize;

    for path in std::env::split_paths(&paths) {
        smoke_one_file(path, &settings);
        checked += 1;
    }

    assert!(checked > 0, "FASTPAD_SMOKE_FILES did not contain any paths");
}

fn smoke_one_file(path: PathBuf, settings: &AppSettings) {
    assert!(path.exists(), "missing smoke file: {}", path.display());

    let file = FileHandle::open(&path, FileOpenOptions::default())
        .unwrap_or_else(|error| panic!("failed to open {}: {error:#}", path.display()));
    assert!(!file.is_empty(), "empty smoke file: {}", path.display());
    assert!(
        file.metadata().intelligence.likely_text,
        "file should be detected as text: {}",
        path.display()
    );

    let first_window = file
        .read_at_most(ByteOffset::ZERO, 4096)
        .unwrap_or_else(|error| panic!("failed first read {}: {error:#}", path.display()));
    assert!(
        !first_window.is_empty(),
        "first window empty for {}",
        path.display()
    );

    let mut doc = Document::open(
        DocumentId(1),
        &path,
        settings,
        OpenIntent {
            force_analysis: true,
            force_edit: false,
        },
    )
    .unwrap_or_else(|error| panic!("failed forced analysis open {}: {error:#}", path.display()));
    assert_eq!(doc.mode(), EditorMode::ViewAnalysis);
    let viewport = doc
        .viewport(ViewportRequest {
            anchor: ViewAnchor::Start,
            max_lines: 40,
            max_bytes: 64 * 1024,
        })
        .unwrap_or_else(|error| panic!("failed viewport {}: {error:#}", path.display()));
    assert!(
        !viewport.lines.is_empty(),
        "no viewport lines for {}",
        path.display()
    );

    let default_doc = Document::open(DocumentId(2), &path, settings, OpenIntent::default())
        .unwrap_or_else(|error| panic!("failed default open {}: {error:#}", path.display()));
    if default_doc.mode() == EditorMode::Edit {
        assert!(
            !default_doc.full_text_for_editing().unwrap().is_empty(),
            "edit text empty for {}",
            path.display()
        );
    }

    let sample = String::from_utf8_lossy(&first_window);
    let token = sample
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .find(|token| token.len() >= 4)
        .expect("smoke files should contain searchable text");
    let summary = SearchEngine::search(
        &file,
        &SearchQuery {
            pattern: token.to_string(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            max_results: 16,
            chunk_size: 4096,
        },
        &CancellationToken::new(),
    )
    .unwrap_or_else(|error| panic!("failed search {}: {error:#}", path.display()));
    assert!(
        summary.matches_seen > 0,
        "search token `{token}` not found in {}",
        path.display()
    );
}
