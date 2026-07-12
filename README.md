# FastPad

FastPad is a native macOS text editor written in Rust. The project is optimized around two explicit modes:

- **View/Analysis Mode** for huge read-only files, log inspection, search, filtering, and byte-offset navigation.
- **Edit Mode** for normal editing with a rope buffer, undo/redo, replace, and atomic save.

The current repository is an MVP implementation scaffolded from the supplied SRS and Unix-style engineering blueprint. It intentionally prioritizes the invariants that matter for large files: bounded reads, lazy line indexing, cancellable long-running work, and native macOS shell integration.

## What Works Now

- Native AppKit macOS window, menu bar, `Open...`, `Save`, and `Page Down`.
- Notepad++-style menu categories are visible, including Macro, Run, and Plugins placeholders; unfinished commands are disabled until their engines/UI are implemented.
- Single-window multi-tab shell with shared documents, lightweight per-tab view state, tab switching, duplicate tab, and pin indicators.
- `Save As...`, `Save a Copy As...`, multi-file open, Finder Open With, existing-window file routing, and quit prompts for unsaved documents.
- Automatic mode selection based on file size/risk.
- Read-only mmap/chunk file engine with bounded byte-range reads.
- Lazy line index, visible-only viewport extraction, and bounded background line-index warmup.
- Grep-style literal/regex search with cancellation and bounded results.
- Native Search menu panel for current-document search, line filtering, tail follow, progressive previews, and cancellation.
- Rope-based Edit Mode buffer with undo/redo transactions.
- Regex/literal replace-all as one undoable edit transaction.
- Non-destructive streaming pipeline preview stages: contains, regex, field extraction, and head.
- Tail-follow state for growing files.
- Diagnostics structs for open/search/render budgets.
- JSON benchmark harness for startup, open, viewport/render, search, memory, and typing latency.

## Build

Install Rust if needed:

```sh
brew install rust
```

Run tests:

```sh
cargo test
```

Run smoke tests against the project specification files used during development:

```sh
scripts/smoke_attached_files.sh
```

Run the default quick benchmark suite:

```sh
scripts/run_benchmarks.sh
```

Run a large-file benchmark fixture without changing the harness:

```sh
BYTES=1G ITERATIONS=1 scripts/run_benchmarks.sh
```

Run the macOS app directly:

```sh
cargo run -p fastpad_app_macos --bin FastPad
```

Open a file at launch:

```sh
cargo run -p fastpad_app_macos --bin FastPad -- /path/to/file.log
```

Create and run a minimal `.app` bundle:

```sh
scripts/run_macos_app.sh
```

## Project Layout

- `crates/fastpad_app_macos`: AppKit application shell.
- `crates/fastpad_core`: document/tab/view manager, mode manager, command capabilities.
- `crates/fastpad_file`: mmap/chunked file access, file intelligence, atomic write.
- `crates/fastpad_line_index`: lazy line boundary discovery.
- `crates/fastpad_viewport`: less/more-style visible region model.
- `crates/fastpad_search`: grep-style streaming search.
- `crates/fastpad_pipeline`: non-destructive filter/extract/count previews.
- `crates/fastpad_edit`: rope edit buffer and undo/redo.
- `crates/fastpad_replace`: sed-like replace operations for Edit Mode.
- `crates/fastpad_tail`: tail-follow state.
- `crates/fastpad_render`: render plan model for visible lines and overlays.
- `crates/fastpad_tasks`: cancellation and background task handles.
- `crates/fastpad_diagnostics`: performance budget structs and timers.
- `crates/fastpad_benchmarks`: benchmark harness and JSON report generator.

## Non-Negotiable Invariants

- Do not load huge files into a `String`.
- Do not build a full line index before first render.
- Do not block the UI thread for scanning, searching, indexing, or parsing.
- Do not expose destructive editing commands in View/Analysis Mode.
- Use byte offsets for large-file references.
- Emit progressive results for long operations.
- Every long-running operation must have a cancellation token.
