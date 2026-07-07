# FastPad AI-Native Software Requirements Specification

A Codex-friendly engineering specification for a native macOS Notepad++-style editor.

## Summary

- **global_requirement_count**: 56
- **module_count**: 24
- **module_requirement_count**: 354
- **total_requirement_count**: 410
- **supported_language_count**: 85
- **notepad_catalog_sections**: 8

## Modules

### MOD-001 — Application Shell

Owns macOS lifecycle, windows, menus, app activation, file-open events, and native integrations.

Responsibilities:
- Create main application window
- Create document windows
- Build native menu bar
- Map menu commands to command registry
- Receive Finder Open With events
- Handle app activation/deactivation
- Handle safe mode startup
- Coordinate shutdown

### MOD-002 — Document Manager

Owns document lifecycle, tabs, document identity, dirty state, read-only state, and mapping between UI tabs and editor buffers.

Responsibilities:
- Create untitled documents
- Open existing documents
- Close documents
- Track dirty documents
- Track document file path
- Track read-only mode
- Coordinate save/reload
- Coordinate external modification prompts
- Manage document metadata

### MOD-003 — File Engine

Provides safe and fast file I/O, encoding detection, line ending detection, atomic saves, memory mapping, and streaming reads.

Responsibilities:
- Open file handles
- Preflight file metadata
- Detect binary files
- Detect encoding
- Detect line endings
- Read small/normal files
- Stream large files
- Memory-map files when appropriate
- Atomic write
- Backup write optional
- File watch integration

### MOD-010 — Block / Column Selection Engine

Provides Notepad++-style rectangular selection and multi-caret editing across columns, including virtual space, tabs, Unicode, paste distribution, and undo grouping.

Responsibilities:
- Activate rectangular selection by keyboard and mouse
- Represent rectangular selection independent of line length
- Render rectangular highlight
- Create one caret per affected line
- Support typing across all selected lines
- Support paste distribution
- Support delete/backspace across rectangular area
- Support virtual space beyond line end
- Handle tabs and variable-width characters
- Integrate with undo as one transaction

### MOD-004 — Text Buffer

Owns editable text representation using rope/piece table.

Responsibilities:
- Text Buffer owns its internal state
- Text Buffer exposes a narrow API
- Text Buffer must be testable without the full UI
- Text Buffer must not block editor interactivity

### MOD-005 — Undo Redo Engine

Groups edits into reversible transactions.

Responsibilities:
- Undo Redo Engine owns its internal state
- Undo Redo Engine exposes a narrow API
- Undo Redo Engine must be testable without the full UI
- Undo Redo Engine must not block editor interactivity

### MOD-006 — Cursor Engine

Maintains caret position, desired visual column, movement semantics.

Responsibilities:
- Cursor Engine owns its internal state
- Cursor Engine exposes a narrow API
- Cursor Engine must be testable without the full UI
- Cursor Engine must not block editor interactivity

### MOD-007 — Selection Engine

Normal, word, line, range, and multi-selection behavior.

Responsibilities:
- Selection Engine owns its internal state
- Selection Engine exposes a narrow API
- Selection Engine must be testable without the full UI
- Selection Engine must not block editor interactivity

### MOD-008 — Rendering Pipeline

Converts visible document regions to painted glyphs and highlights.

Responsibilities:
- Rendering Pipeline owns its internal state
- Rendering Pipeline exposes a narrow API
- Rendering Pipeline must be testable without the full UI
- Rendering Pipeline must not block editor interactivity

### MOD-009 — Viewport and Scrolling

Maps document lines to screen coordinates and scroll state.

Responsibilities:
- Viewport and Scrolling owns its internal state
- Viewport and Scrolling exposes a narrow API
- Viewport and Scrolling must be testable without the full UI
- Viewport and Scrolling must not block editor interactivity

### MOD-011 — Search Engine

Literal and regex search across documents and files.

Responsibilities:
- Search Engine owns its internal state
- Search Engine exposes a narrow API
- Search Engine must be testable without the full UI
- Search Engine must not block editor interactivity

### MOD-012 — Replace Engine

Safe replacement operations and previews.

Responsibilities:
- Replace Engine owns its internal state
- Replace Engine exposes a narrow API
- Replace Engine must be testable without the full UI
- Replace Engine must not block editor interactivity

### MOD-013 — Syntax Highlighting Engine

Lexing, themes, token spans, incremental invalidation.

Responsibilities:
- Syntax Highlighting Engine owns its internal state
- Syntax Highlighting Engine exposes a narrow API
- Syntax Highlighting Engine must be testable without the full UI
- Syntax Highlighting Engine must not block editor interactivity

### MOD-014 — Code Folding Engine

Fold ranges, fold gutter, persistence, language-aware folding.

Responsibilities:
- Code Folding Engine owns its internal state
- Code Folding Engine exposes a narrow API
- Code Folding Engine must be testable without the full UI
- Code Folding Engine must not block editor interactivity

### MOD-015 — Large File Engine

Multi-GB viewing, virtual line index, chunk cache, streaming operations.

Responsibilities:
- Large File Engine owns its internal state
- Large File Engine exposes a narrow API
- Large File Engine must be testable without the full UI
- Large File Engine must not block editor interactivity

### MOD-016 — Workspace Engine

Folder tree, indexing, ignore rules, project search.

Responsibilities:
- Workspace Engine owns its internal state
- Workspace Engine exposes a narrow API
- Workspace Engine must be testable without the full UI
- Workspace Engine must not block editor interactivity

### MOD-017 — Command System

Command registry, palette, menu routing, shortcut routing.

Responsibilities:
- Command System owns its internal state
- Command System exposes a narrow API
- Command System must be testable without the full UI
- Command System must not block editor interactivity

### MOD-018 — Settings Engine

Global, user, project, language, and feature settings.

Responsibilities:
- Settings Engine owns its internal state
- Settings Engine exposes a narrow API
- Settings Engine must be testable without the full UI
- Settings Engine must not block editor interactivity

### MOD-019 — Theme Engine

Editor colors, token colors, UI appearance, font settings.

Responsibilities:
- Theme Engine owns its internal state
- Theme Engine exposes a narrow API
- Theme Engine must be testable without the full UI
- Theme Engine must not block editor interactivity

### MOD-020 — Plugin Host

Plugin lifecycle, APIs, permissions, isolation.

Responsibilities:
- Plugin Host owns its internal state
- Plugin Host exposes a narrow API
- Plugin Host must be testable without the full UI
- Plugin Host must not block editor interactivity

### MOD-021 — Macro Engine

Record, store, replay editor commands.

Responsibilities:
- Macro Engine owns its internal state
- Macro Engine exposes a narrow API
- Macro Engine must be testable without the full UI
- Macro Engine must not block editor interactivity

### MOD-022 — LSP Integration

Optional language server features.

Responsibilities:
- LSP Integration owns its internal state
- LSP Integration exposes a narrow API
- LSP Integration must be testable without the full UI
- LSP Integration must not block editor interactivity

### MOD-023 — Git Integration

Optional git status, diff, blame, changed-line markers.

Responsibilities:
- Git Integration owns its internal state
- Git Integration exposes a narrow API
- Git Integration must be testable without the full UI
- Git Integration must not block editor interactivity

### MOD-024 — Testing and Diagnostics

Automated tests, performance diagnostics, logging.

Responsibilities:
- Testing and Diagnostics owns its internal state
- Testing and Diagnostics exposes a narrow API
- Testing and Diagnostics must be testable without the full UI
- Testing and Diagnostics must not block editor interactivity


## MVP Plan

### phase_1_core
- Application shell
- Document manager
- File engine
- Text buffer
- Tabs
- Basic editing
- Undo/redo
- Search/replace
- Regex search
- Line numbers
- Syntax highlighting for 20 core languages
- Large-file read-only mode
- Virtual rendering
- Dark/light theme
- Session restore

### phase_2_power_user
- Block selection
- Multi-cursor
- Find in files
- Folder workspace
- Bookmarks
- Split view
- Macros
- Command palette
- Encoding conversion
- User-defined languages

### phase_3_modern
- Plugin API
- Tree-sitter
- LSP
- Git
- Markdown preview
- JSON/XML tools
- CSV viewer
- Compare files
- Performance diagnostics

