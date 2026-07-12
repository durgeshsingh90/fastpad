# Architecture

FastPad follows the supplied Unix-style blueprint: small modules, bounded work, byte-offset addressing, progressive output, and cancellation for long-running tasks.

## Modes

`fastpad_core::ModeManager` chooses between:

- `ViewAnalysis`: read-only, mmap/chunk access, viewport rendering only.
- `Edit`: rope-backed editable buffer with undo/redo and atomic save.

Users only see these two modes. Internally, `ModeManager` now selects an `InternalEngineProfile` so the same public document interface can route work through specialized strategies without exposing engine names in the UI:

- `NormalEdit`
- `LargeOptimizedEdit`
- `StreamingEdit`
- `HugeFileAnalysis`
- `StructuredDataAnalysis`
- `BinaryInspection`

Selection is adaptive. It uses file size, sampled line count, longest sampled line, average sampled line length, file kind, binary/encoding detection, user intent, and macOS memory pressure. Large, binary-looking, structured-data, memory-constrained, or risky files enter `ViewAnalysis` by default. Files that remain in `Edit` can surface the user-safe status message `Large File Optimizations Enabled` without exposing implementation details.

Edit commands are gated through `CommandRegistry` so UI/menu/plugin layers cannot bypass mode capabilities.

## File Opening

`fastpad_file::FileHandle::open` reads metadata and a small sample. The sample records encoding, binary confidence, line endings, sampled line count, longest sampled line, average sampled line length, and coarse file kind from extension. It may create a read-only mmap, but all public reads are byte-range bounded. `read_entire_if_under` is only used by Edit Mode after adaptive threshold checks.

## Multi-Document Model

FastPad treats tabs as lightweight views over shared documents:

`Application -> Window -> Tab -> View -> Document -> Text Buffer`

`fastpad_core::DocumentManager` owns documents once, indexes open file paths, and creates tabs with independent `DocumentViewState` values. Opening an already-open path creates another tab referencing the existing `DocumentId`; it does not reload the file or duplicate the text buffer. This supports future split views, compare views, and multi-window document sharing without changing the storage model.

## Viewports and Line Indexing

`fastpad_line_index::LazyLineIndex` keeps a contiguous line-start index only for regions that need line-number mapping. Byte-offset and percentage navigation do not force a full scan; they discover a nearby line start with bounded backtracking.

`fastpad_viewport::ViewportEngine` returns visible `LineSlice` values plus byte anchors. The native shell currently displays one viewport at a time and exposes `Page Down` for read-only paging.

For View/Analysis documents, the AppKit shell starts one bounded background line-index warmup task at a time. The worker builds from a `LineIndexSnapshot`, emits progress through `TaskProgress`, and applies the finished snapshot back to the document manager; search/filter/tail tasks preempt that warmup so interactive analysis stays responsive.

## Search and Pipelines

`fastpad_search` scans chunks with overlap so literal matches crossing chunk boundaries are preserved. Results are bounded by `max_results`, while `matches_seen` continues counting all matches discovered before cancellation.

`fastpad_pipeline` streams lines through composable non-destructive stages. Preview output is bounded, and the source file remains untouched.

The native Search menu exposes current-document search, contains-filter preview, tail follow, and cancellation through a bottom analysis results pane. File-backed search and filter jobs run on background `TaskHandle`s and push progressive preview text through progress updates; tail follow polls appended byte ranges into the same pane.

## Benchmarks

`fastpad_benchmarks` provides a repeatable JSON benchmark harness for core startup, document open, first viewport/render, full literal search, peak RSS deltas, and edit typing latency. The default fixture is intentionally small for local iteration, while `scripts/run_benchmarks.sh` accepts larger fixture sizes such as `BYTES=1G` and `BYTES=10G` for large-file runs on suitable machines.

## Editing

`fastpad_edit::EditBuffer` uses `ropey::Rope` and records edits as transactions. `fastpad_replace` builds descending edit transactions so replace-all applies safely as one undo step.

## Native Shell

`fastpad_app_macos` uses AppKit directly from Rust through `cocoa`/`objc`. The app shell owns native windows, menus, open/save actions, and translates UI commands into core document operations.

The current shell presents a single primary window with a lightweight tab strip. Finder/start-script file opens route into the existing app instance by default, and tab switching/duplication/pinning are wired through native menu commands.

## Current MVP Limits

- Edit Mode still uses `NSTextView`; View/Analysis Mode uses a custom AppKit virtual view backed by bounded render plans.
- `LargeOptimizedEdit` and `StreamingEdit` are selected by the core decision model, but the first AppKit edit surface still requires a bounded full-text load. Files beyond the current bounded edit load are routed to `ViewAnalysis` until a true virtual edit engine is implemented.
- Current-document search/filter/tail panels exist, but workspace search, saved filters, result navigation, export flows, and richer task diagnostics are future work.
- Drag-and-drop tab reordering, split views, multiple windows, recently closed tabs, session restore, syntax highlighting, block selection, bookmarks, and performance dashboards are future work.
- Benchmark dashboards and CI regression gates are future work.
