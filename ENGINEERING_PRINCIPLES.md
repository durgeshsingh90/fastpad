# FastPad Engineering Principles

This is the governing engineering document for FastPad. Every Codex or agent task
that changes this repository should read this file before editing and keep the
implementation aligned with these principles.

## Objective

FastPad is a native macOS text editor written in Rust.

Its purpose is not to become another IDE.

Its purpose is to become the fastest graphical text editor for macOS while
preserving the simplicity of Notepad++ and adopting the architectural principles
of Unix tools such as:

- `less`
- `more`
- `grep`
- `tail`
- `awk`
- `sed`
- `sort`
- `uniq`
- `wc`

Every implementation decision must optimize for speed, responsiveness,
simplicity, and predictable behavior.

## Core Principles

### Principle 1: The UI Must Never Freeze

No feature is allowed to block the UI thread.

Examples:

- opening files
- searching
- indexing
- parsing
- statistics
- syntax highlighting
- exporting
- pipelines

Everything expensive runs asynchronously.

### Principle 2: Nothing Should Be Loaded Before It Is Needed

Never:

- load an entire file unless the active mode and file characteristics require it
- parse an entire file
- syntax-highlight an entire file
- build every line index
- measure every line

Always load lazily.

### Principle 3: Render Only What The User Can See

The renderer only renders:

- visible lines
- configurable overscan

Nothing else.

### Principle 4: Memory Usage Must Scale With The Viewport

Memory usage must scale with the viewport, active caches, and active indexes.
It must not scale directly with file size.

Opening a 10 GB file must not require 10 GB of RAM.

### Principle 5: Search Behaves Like `grep`

Search must be:

- streaming
- progressive
- cancellable
- background

Results should appear immediately.

### Principle 6: View/Analysis Mode Behaves Like `less`

View/Analysis Mode must be:

- memory mapped or otherwise bounded
- read only
- streaming
- fast to start
- compatible with follow mode
- compatible with pipelines
- compatible with filters

### Principle 7: Edit Mode Behaves Like A Lightweight Editor

Edit Mode should support:

- rope or piece table storage
- undo
- redo
- replace
- block selection
- multi cursor
- atomic save

### Principle 8: Every Long-Running Task Must Support Cancellation

Users should never wait for work they no longer need.

Opening, searching, indexing, parsing, exporting, filtering, and pipeline work
must be cancellable when it can run longer than a frame.

### Principle 9: Expensive Subsystems Must Expose Performance Metrics

Examples:

- open time
- frame time
- search throughput
- memory usage
- cache hit ratio

Performance regressions are bugs.

### Principle 10: Background Work Must Never Reduce Editing Responsiveness

The active document always has higher priority than every background task.

Inactive work must yield, pause, cancel, or hibernate when it competes with the
active editor.

## User Experience Rules

FastPad should always open with a new untitled document when launched normally.

Opening files opens them in tabs.

Opening a file that is already open activates its existing tab.

Documents are never duplicated unnecessarily.

Tabs are lightweight.

Documents are shared.

Users should be able to keep hundreds of files open.

Inactive tabs should release caches automatically.

## Adaptive Engine

FastPad exposes only two user-facing modes:

- View / Analysis Mode
- Edit Mode

Internally the implementation automatically adapts:

- memory mapping
- streaming
- lazy indexing
- cache policy
- syntax strategy
- rendering strategy

The user should not need to understand these implementation details.

## Unix Philosophy

Every analysis feature should be composable.

Example pipeline:

```text
Search -> Filter -> Extract -> Transform -> Sort -> Group -> Count -> Export
```

These stages must execute incrementally.

They must never require the full file in memory.

## Coding Standards

Every subsystem must:

- have a clearly defined responsibility
- expose a small interface
- avoid global mutable state
- be independently testable
- support benchmarks

When a feature conflicts with these principles, prefer the simpler, faster, and
more predictable implementation.
