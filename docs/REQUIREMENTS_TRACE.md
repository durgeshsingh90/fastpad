# Requirements Trace

This MVP maps the supplied specs to implemented repository components.

## Implemented Core

- `MODE-001` to `MODE-007`: represented by `EditorMode`, `ModeManager`, and `CommandRegistry` in `fastpad_core`.
- `APP-001`, `APP-002`: application launches with no file or one/multiple file arguments.
- `APP-003`: Finder Open With events are routed through the app delegate.
- `APP-006`: quit prompts before terminating when the active document has unsaved changes.
- `APP-008`: native macOS menu bar exists with Notepad++-style File/Edit/Search/View/Encoding/Language/Settings/Tools sections plus visible Macro/Run/Plugins placeholders.
- `MDI-001`: single-window tabs are the default document surface; tabs reference shared documents through independent view state.
- `MDI-002`: opening an already-open path creates a new lightweight tab without reloading the underlying document.
- `FILE-001` to `FILE-005`, `FILE-009`, `FILE-010`: represented by `fastpad_file`.
- `LINE-001` to `LINE-006`: represented by `fastpad_line_index`.
- `VIEW-001` to `VIEW-006`: represented by `fastpad_viewport` and the AppKit `Page Down` command.
- `GREP-001`, `GREP-002`, `GREP-004`, `GREP-005`, `GREP-006`, `GREP-011`, `GREP-012`: represented by `fastpad_search`.
- `TAIL-001` to `TAIL-005`: represented by `fastpad_tail`.
- `PIPE-001` to `PIPE-003`, `PIPE-009`, `PIPE-012`: represented by `fastpad_pipeline`.
- `EDIT-001` to `EDIT-004`, `EDIT-009`, `EDIT-010`: represented by `fastpad_edit`, `fastpad_replace`, and `fastpad_core`.
- `SED-001` to `SED-004`: represented by `fastpad_replace`.
- `TASK-001`: represented by `fastpad_tasks::CancellationToken`.
- `DIAG-001` to `DIAG-004`: represented by `fastpad_diagnostics` metric structs.

## Not Yet Exposed In UI

- Full search panel and live result list.
- Filter/pipeline builder.
- File intelligence side panel.
- Full implementation behind disabled Notepad++-style menu items.
- Multiple independent windows, drag/drop tab reordering, split views, and session restore.
- Bookmarks, notes, and inspectors.
- Tail-follow controls.
- Performance dashboard.
- Syntax highlighting and custom renderer.
- Block selection and multi-cursor editing.

## Deferred By Design

The project objective explicitly excludes IDE features, debugger, terminal, cloud sync, accounts, analytics, and AI assistant behavior for v1.
