# Architecture

FastPad follows the supplied Unix-style blueprint: small modules, bounded work, byte-offset addressing, progressive output, and cancellation for long-running tasks.

## Modes

`fastpad_core::ModeManager` chooses between:

- `ViewAnalysis`: read-only, mmap/chunk access, viewport rendering only.
- `Edit`: rope-backed editable buffer with undo/redo and atomic save.

Large, binary-looking, or risky files enter `ViewAnalysis` by default. Edit commands are gated through `CommandRegistry` so UI/menu/plugin layers cannot bypass mode capabilities.

## File Opening

`fastpad_file::FileHandle::open` reads metadata and a small sample. It may create a read-only mmap, but all public reads are byte-range bounded. `read_entire_if_under` is only used by Edit Mode after threshold checks.

## Viewports and Line Indexing

`fastpad_line_index::LazyLineIndex` keeps a contiguous line-start index only for regions that need line-number mapping. Byte-offset and percentage navigation do not force a full scan; they discover a nearby line start with bounded backtracking.

`fastpad_viewport::ViewportEngine` returns visible `LineSlice` values plus byte anchors. The native shell currently displays one viewport at a time and exposes `Page Down` for read-only paging.

## Search and Pipelines

`fastpad_search` scans chunks with overlap so literal matches crossing chunk boundaries are preserved. Results are bounded by `max_results`, while `matches_seen` continues counting all matches discovered before cancellation.

`fastpad_pipeline` streams lines through composable non-destructive stages. Preview output is bounded, and the source file remains untouched.

## Editing

`fastpad_edit::EditBuffer` uses `ropey::Rope` and records edits as transactions. `fastpad_replace` builds descending edit transactions so replace-all applies safely as one undo step.

## Native Shell

`fastpad_app_macos` uses AppKit directly from Rust through `cocoa`/`objc`. The app shell owns native windows, menus, open/save actions, and translates UI commands into core document operations.

## Current MVP Limits

- The AppKit shell uses `NSTextView` as the initial UI surface. View/Analysis Mode pages through bounded viewports instead of implementing a custom virtual renderer.
- Search and pipeline engines are available in core crates, but the first AppKit shell does not yet expose full search/filter panels.
- Save-as, session restore, syntax highlighting, block selection, bookmarks, and performance dashboards are future work.

