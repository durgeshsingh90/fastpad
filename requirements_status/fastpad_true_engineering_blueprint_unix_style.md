# FastPad True Engineering Blueprint — Unix-Style Fast Text Editor

## Objective

Build a native macOS Rust text editor with only two modes: **View/Analysis Mode** and **Edit Mode**. The application should adapt the principles of Unix tools like `less`, `more`, `tail -f`, `grep`, `awk`, `sed`, `sort`, `uniq`, and `wc`.

## Core Decision

- **View/Analysis Mode**: read-only, fastest path, optimized for 10GB+ files.
- **Edit Mode**: editable, safe, undoable, still optimized.

## Hard Performance Targets

- **perceived_open_10gb_file_target_ms**: 200
- **perceived_open_multiple_10gb_files_target**: Each file opens by metadata + mmap/stream handle + first viewport only; never full-read.
- **first_viewport_render_target_ms**: 16
- **scroll_frame_budget_ms**: 16
- **typing_latency_edit_mode_ms**: 8
- **first_search_result_target_ms**: 200
- **ui_freeze_max_ms**: 16
- **ram_for_single_10gb_view_analysis_file_mb**: Target < 50-100 MB excluding OS page cache.
- **ram_scaling_rule**: RAM scales with visible regions, caches and indexes, not file size.
- **background_task_rule**: All background tasks must be cancellable and lower priority than UI.

## Unix Principles

### Do not read the entire file unless needed
- Used by: less, more, tail
- Application rule: Opening a huge file should only read metadata and the bytes required for the first viewport.

### Stream sequentially for search and analysis
- Used by: grep, awk, sed, wc
- Application rule: Search, filters, extraction, stats and pipelines operate chunk-by-chunk and emit results progressively.

### Use byte offsets as stable positions
- Used by: grep, tail, dd
- Application rule: Large-file references should use byte offsets first, with line numbers built lazily.

### Small composable operations
- Used by: grep | awk | sort | uniq
- Application rule: Analysis pipelines should compose filter, extract, transform, group, count and export stages.

### Progressive output
- Used by: tail -f, grep
- Application rule: Results appear as soon as they are found; the user must not wait for complete scans.

### Avoid unnecessary parsing
- Used by: less, grep
- Application rule: Syntax highlighting, full statistics, indexing and minimap are optional background tasks, never prerequisites for viewing.

### Pipeline cancellation
- Used by: Ctrl+C in shell
- Application rule: Every long-running operation has a cancellation token and visible cancel button.

## Modules

### ENG-001 — Mode Manager

Owns the two user-facing modes and enforces capability differences between View/Analysis Mode and Edit Mode.

Responsibilities:
- Choose mode during open
- Expose active capabilities
- Block invalid commands in current mode
- Convert View/Analysis Mode to Edit Mode when explicitly requested
- Prevent accidental destructive operations on huge files

Key requirements:
- `MODE-001` Only two user-facing modes exist: View/Analysis Mode and Edit Mode (MUST)
- `MODE-002` Open huge files directly in View/Analysis Mode by default (MUST)
- `MODE-003` Disable editing commands in View/Analysis Mode (MUST)
- `MODE-004` Allow copy, search, filter, follow, inspect, bookmark, extract and export in View/Analysis Mode (MUST)
- `MODE-005` Require explicit confirmation before converting huge file to Edit Mode (MUST)
- `MODE-006` Show current mode in status bar and title metadata (MUST)
- `MODE-007` Expose mode capabilities to command registry (MUST)
- `MODE-008` Do not allow plugin commands to bypass mode restrictions (MUST)

### ENG-002 — Unix-Style File Engine

Provides mmap, streaming, chunked reads, byte-offset addressing and atomic writes.

Responsibilities:
- Metadata-only open path
- Memory-map read-only files
- Chunked streaming fallback
- Lazy reading for first viewport
- Atomic save for Edit Mode
- Stable byte-offset references
- File growth/truncation detection

Key requirements:
- `FILE-001` Opening a file must never require reading the whole file (MUST)
- `FILE-002` Use memory mapping for View/Analysis Mode when safe (MUST)
- `FILE-003` Use chunked streaming when mmap is unavailable or unsafe (MUST)
- `FILE-004` Read only enough bytes to render initial viewport (MUST)
- `FILE-005` Expose byte-offset based access API (MUST)
- `FILE-006` Support tail-like reads from end of file (MUST)
- `FILE-007` Detect file growth for follow mode (MUST)
- `FILE-008` Detect truncation and log rotation (MUST)
- `FILE-009` Perform atomic writes in Edit Mode (MUST)
- `FILE-010` Never allocate buffer equal to file size in View/Analysis Mode (MUST)

### ENG-003 — Lazy Line Index

Builds line-number to byte-offset mapping incrementally instead of scanning the full file upfront.

Responsibilities:
- Support visible-region line discovery
- Support approximate total lines
- Support jump to byte offset immediately
- Support jump to line via progressive index
- Persist optional index cache
- Handle CRLF, LF and CR

Key requirements:
- `LINE-001` Do not build full line index during open (MUST)
- `LINE-002` Build line index lazily for visible and searched regions (MUST)
- `LINE-003` Support byte offset navigation without full line index (MUST)
- `LINE-004` Support approximate percentage navigation (MUST)
- `LINE-005` Support progressive jump-to-line with progress feedback (MUST)
- `LINE-006` Cache line boundaries for visited chunks (MUST)
- `LINE-007` Invalidate line index safely when file changes (MUST)
- `LINE-008` Support mixed line endings with warnings (MUST)

### ENG-004 — Viewport and Pager Engine

Implements less/more-style paging in a GUI using visible-only rendering and byte/line anchors.

Responsibilities:
- Render only visible lines
- Page up/down like more/less
- Scroll using byte and line anchors
- Preserve viewport around file changes
- Support jump to top, bottom, percentage, byte offset and line

Key requirements:
- `VIEW-001` Initial viewport renders without full-file scan (MUST)
- `VIEW-002` Render only visible lines plus overscan (MUST)
- `VIEW-003` Support page down and page up (MUST)
- `VIEW-004` Support jump to start and end of file (MUST)
- `VIEW-005` Support jump to percentage of file (MUST)
- `VIEW-006` Support jump to byte offset (MUST)
- `VIEW-007` Support jump to line with progressive indexing (MUST)
- `VIEW-008` Scrolling must remain smooth while background tasks run (MUST)
- `VIEW-009` Extremely long lines must be horizontally virtualized (MUST)

### ENG-005 — Streaming Search and Grep Engine

Implements grep-like literal and regex search with progressive results and cancellation.

Responsibilities:
- Literal search
- Regex search
- Case options
- Whole word
- Context lines
- Live results
- Cancellation
- Search in current file and workspace

Key requirements:
- `GREP-001` Search starts immediately without full indexing (MUST)
- `GREP-002` Search emits results progressively (MUST)
- `GREP-003` First result target under 200ms when match is near current/early region (MUST)
- `GREP-004` Support literal search using fast algorithms (MUST)
- `GREP-005` Support regex search using streaming-safe engine (MUST)
- `GREP-006` Support chunk-boundary matches (MUST)
- `GREP-007` Support before/after context lines like grep -C (MUST)
- `GREP-008` Support invert match like grep -v (MUST)
- `GREP-009` Support count-only mode like grep -c (MUST)
- `GREP-010` Support match-only extraction like grep -o (MUST)
- `GREP-011` Support cancellation like Ctrl+C (MUST)
- `GREP-012` Support bounded result buffer and export all results (MUST)

### ENG-006 — Tail Follow Engine

Implements tail -f style live file following for logs and growing files.

Responsibilities:
- Follow appended data
- Pause and resume
- Detect rotation/truncation
- Follow from end or current position
- Combine with filters/search highlights

Key requirements:
- `TAIL-001` Open file at end like tail (MUST)
- `TAIL-002` Follow appended lines like tail -f (MUST)
- `TAIL-003` Pause follow without closing file (MUST)
- `TAIL-004` Resume follow (MUST)
- `TAIL-005` Detect truncation (MUST)
- `TAIL-006` Detect log rotation (MUST)
- `TAIL-007` Apply live filters to followed data (MUST)
- `TAIL-008` Preserve manual scroll position when user scrolls away from bottom (MUST)

### ENG-007 — Filter, Awk and Pipeline Engine

Provides non-destructive filters and composable Unix-like analysis pipelines.

Responsibilities:
- Live filter
- Invert filter
- Extract fields
- Transform lines
- Group and count
- Sort and unique
- Export output

Key requirements:
- `PIPE-001` Filter lines without modifying original file (MUST)
- `PIPE-002` Support contains, regex, AND, OR, NOT (MUST)
- `PIPE-003` Support grep -v style inverted filter (MUST)
- `PIPE-004` Support awk-like field extraction for delimited text (MUST)
- `PIPE-005` Support sort-like streamed or external sort strategy (MUST)
- `PIPE-006` Support uniq-like duplicate collapse (MUST)
- `PIPE-007` Support wc-like counts (MUST)
- `PIPE-008` Support reusable saved pipelines (MUST)
- `PIPE-009` Support progressive preview after each stage (MUST)
- `PIPE-010` Support export pipeline result as text, CSV or JSON (MUST)
- `PIPE-011` Pipeline execution must be cancellable (MUST)
- `PIPE-012` Pipeline must respect memory budget (MUST)

### ENG-008 — Edit Buffer Engine

Provides optimized editable text representation for Edit Mode.

Responsibilities:
- Rope or piece table
- Undo/redo
- Multi-cursor
- Block selection
- Safe replace
- Incremental syntax
- Atomic save

Key requirements:
- `EDIT-001` Use rope or piece-table buffer in Edit Mode (MUST)
- `EDIT-002` Support insert, delete, replace (MUST)
- `EDIT-003` Support undo and redo as transactions (MUST)
- `EDIT-004` Support search and replace (MUST)
- `EDIT-005` Support regex replace with captures (MUST)
- `EDIT-006` Support block/column selection (MUST)
- `EDIT-007` Support multi-cursor editing (MUST)
- `EDIT-008` Support trim, sort, unique and line operations (MUST)
- `EDIT-009` Perform atomic save (MUST)
- `EDIT-010` Warn when file is too large for safe Edit Mode (MUST)

### ENG-009 — Replace and Sed-Like Transform Engine

Implements safe find/replace and sed-like line transformations.

Responsibilities:
- Replace current/all
- Regex capture replace
- Preview replace
- Transform selected lines
- Non-destructive preview in View/Analysis Mode

Key requirements:
- `SED-001` Support replace in Edit Mode (MUST)
- `SED-002` Support regex capture replacement (MUST)
- `SED-003` Support replace preview before applying (MUST)
- `SED-004` Support replace all as one undo transaction (MUST)
- `SED-005` Support non-destructive transform preview in View/Analysis Mode (MUST)
- `SED-006` Support export transformed output without modifying original (MUST)
- `SED-007` Support line-based transformations: trim, case, prefix, suffix (MUST)
- `SED-008` Support safe cancellation for large transforms (MUST)

### ENG-010 — Rendering Engine

Renders visible text, selections, highlights, search matches, bookmarks and diagnostics without full-document layout.

Responsibilities:
- Visible-only rendering
- Syntax span overlay
- Search result overlay
- Selection overlay
- Gutter and line numbers
- Long-line virtualization

Key requirements:
- `RENDER-001` Render visible lines only (MUST)
- `RENDER-002` Use overscan buffer (MUST)
- `RENDER-003` Cache line layouts (MUST)
- `RENDER-004` Render search highlights from byte ranges (MUST)
- `RENDER-005` Render bookmarks and markers (MUST)
- `RENDER-006` Render selections and block selections (MUST)
- `RENDER-007` Virtualize extremely long lines horizontally (MUST)
- `RENDER-008` Avoid full-document text measurement (MUST)

### ENG-011 — Task Scheduler and Cancellation

Coordinates background scanning, searching, indexing, parsing, filtering and exports.

Responsibilities:
- Priority queues
- Cancellation tokens
- Task progress
- Background throttling
- UI-first scheduling

Key requirements:
- `TASK-001` Every long-running task must have cancellation token (MUST)
- `TASK-002` UI tasks always outrank background analysis tasks (MUST)
- `TASK-003` Search and follow tasks can run concurrently (MUST)
- `TASK-004` Indexing pauses under memory pressure (MUST)
- `TASK-005` Expose background task panel (MUST)
- `TASK-006` Allow cancel from UI (MUST)
- `TASK-007` Throttle progress updates (MUST)
- `TASK-008` Recover from worker failure (MUST)

### ENG-012 — Diagnostics and Benchmarking

Ensures the editor remains as fast as Unix tools by making performance measurable.

Responsibilities:
- Performance dashboard
- Benchmarks
- Memory diagnostics
- Chunk cache metrics
- Search throughput
- Frame time

Key requirements:
- `DIAG-001` Measure perceived open time (MUST)
- `DIAG-002` Measure first viewport render time (MUST)
- `DIAG-003` Measure scroll frame time (MUST)
- `DIAG-004` Measure search throughput MB/s (MUST)
- `DIAG-005` Measure memory per open file (MUST)
- `DIAG-006` Measure chunk cache hit rate (MUST)
- `DIAG-007` Provide benchmark suite for 1GB, 10GB and 20x10GB scenarios (MUST)
- `DIAG-008` Fail CI when performance budget regresses beyond threshold (MUST)

## Absolute Rules for Codex

- Do not load a 10GB file into a String.
- Do not build a full line index before first render.
- Do not syntax-highlight the full file before showing it.
- Do not block the UI thread for file scanning, search, indexing or parsing.
- Do not expose edit commands in View/Analysis Mode.
- Do not let plugins bypass mode capabilities.
- Use byte offsets for large-file references.
- Emit progressive results for search/filter/pipelines.
- Every long-running task must support cancellation.
- Benchmark large-file scenarios before declaring success.
