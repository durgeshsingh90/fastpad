# FastPad Big Text Analysis Requirements

Codex-ready add-on specification for read-only analysis mode and large text file features.

## Summary

- **module_count**: 14
- **requirement_count**: 138
- **must_count**: 56
- **should_count**: 78
- **could_count**: 4

## MVP Priority Order

1. Read-Only Analysis Mode
2. Streaming Search with Live Results
3. Live Filter Mode
4. File Intelligence Panel
5. Log File Mode
6. Line Inspector
7. Bookmarks and Notes
8. Statistics and Frequency Analysis
9. Pattern Detection and Data Extraction
10. Analysis Pipelines
11. Performance and Activity Diagnostics

## Modules

### BTA-001 — Read-Only Analysis Mode

A maximum-performance mode for inspecting very large files without enabling expensive editing features.

Key requirements:
- `BTA-001-001` Automatically enable Read-Only Analysis Mode for huge files (MUST)
- `BTA-001-002` Manual toggle for Read-Only Analysis Mode (SHOULD)
- `BTA-001-003` Disable destructive editing commands in analysis mode (MUST)
- `BTA-001-004` Allow copy, search, filter, bookmark, and export in analysis mode (MUST)
- `BTA-001-005` Show active optimizations in mode banner (SHOULD)
- `BTA-001-006` Allow safe conversion from analysis mode to editable mode (SHOULD)
- `BTA-001-007` Warn before converting huge file to editable mode (MUST)

### BTA-002 — File Intelligence Panel

Immediately summarizes file metadata, structure, statistics, encoding, and risk indicators.

Key requirements:
- `BTA-002-001` Show file name, path, size, modified time, owner, and permissions (MUST)
- `BTA-002-002` Show detected encoding and BOM status (MUST)
- `BTA-002-003` Show detected line ending type (MUST)
- `BTA-002-004` Estimate total line count lazily for huge files (MUST)
- `BTA-002-005` Show longest line and average line length (SHOULD)
- `BTA-002-006` Detect binary/text confidence score (MUST)
- `BTA-002-007` Detect likely file type: log, CSV, JSON, XML, SQL, source, markdown, binary (SHOULD)
- `BTA-002-008` Show warning for very long lines (MUST)
- ...and 1 more

### BTA-003 — Streaming Search with Live Results

Search starts immediately and streams results while scanning continues.

Key requirements:
- `BTA-003-001` Start search before full file is indexed (MUST)
- `BTA-003-002` Show live result count while searching (MUST)
- `BTA-003-003` Allow navigation to results before search completes (MUST)
- `BTA-003-004` Support literal search (MUST)
- `BTA-003-005` Support regex search (MUST)
- `BTA-003-006` Support case-sensitive and case-insensitive search (MUST)
- `BTA-003-007` Support whole-word search (MUST)
- `BTA-003-008` Support search cancellation (MUST)
- ...and 4 more

### BTA-004 — Live Filter Mode

A non-destructive filtered view that shows matching lines without modifying the file.

Key requirements:
- `BTA-004-001` Show only lines containing text (MUST)
- `BTA-004-002` Show only lines matching regex (MUST)
- `BTA-004-003` Hide matching lines (SHOULD)
- `BTA-004-004` Combine multiple filters with AND/OR/NOT (SHOULD)
- `BTA-004-005` Keep original file untouched (MUST)
- `BTA-004-006` Show hidden line count (MUST)
- `BTA-004-007` Allow export of filtered view (SHOULD)
- `BTA-004-008` Allow filters to be saved and reused (SHOULD)
- ...and 1 more

### BTA-005 — Text Query Language

A simple SQL-like/Unix-like query syntax for non-programmers to inspect text files.

Key requirements:
- `BTA-005-001` Support contains('text') query (MUST)
- `BTA-005-002` Support regex('pattern') query (MUST)
- `BTA-005-003` Support AND, OR, NOT (MUST)
- `BTA-005-004` Support field comparisons for structured logs (SHOULD)
- `BTA-005-005` Support timestamp comparisons when timestamps detected (SHOULD)
- `BTA-005-006` Support saved queries (SHOULD)
- `BTA-005-007` Show query parse errors clearly (MUST)
- `BTA-005-008` Compile query into streaming execution plan (MUST)

### BTA-006 — Log File Mode

Specialized mode for server/application/payment logs.

Key requirements:
- `BTA-006-001` Auto-detect log files (SHOULD)
- `BTA-006-002` Highlight ERROR, WARN, INFO, DEBUG, TRACE levels (MUST)
- `BTA-006-003` Filter by log level (MUST)
- `BTA-006-004` Jump to next/previous error (MUST)
- `BTA-006-005` Bookmark all errors (SHOULD)
- `BTA-006-006` Parse timestamps from common formats (SHOULD)
- `BTA-006-007` Timeline view by timestamp (SHOULD)
- `BTA-006-008` Collapse repeated messages (SHOULD)
- ...and 2 more

### BTA-007 — Structured Data Modes

Special modes for CSV, JSON, XML, SQL, and line-delimited JSON.

Key requirements:
- `BTA-007-001` CSV table view for large CSV files (SHOULD)
- `BTA-007-002` CSV column resizing (SHOULD)
- `BTA-007-003` CSV column filter (SHOULD)
- `BTA-007-004` CSV column sort using streaming/indexed mode (COULD)
- `BTA-007-005` JSON tree viewer (SHOULD)
- `BTA-007-006` JSON pretty print (SHOULD)
- `BTA-007-007` JSON minify (SHOULD)
- `BTA-007-008` JSON path search (SHOULD)
- ...and 4 more

### BTA-008 — Inspectors

Panels that inspect a line, token, byte offset, pattern, or selected region.

Key requirements:
- `BTA-008-001` Line inspector (MUST)
- `BTA-008-002` Token inspector (SHOULD)
- `BTA-008-003` Byte offset inspector (MUST)
- `BTA-008-004` Hex view for selected bytes (SHOULD)
- `BTA-008-005` Show line length, byte offset, hash, whitespace, and Unicode details (MUST)
- `BTA-008-006` Show token occurrence count (SHOULD)
- `BTA-008-007` Show first/last occurrence for selected token (SHOULD)
- `BTA-008-008` Show surrounding context (SHOULD)

### BTA-009 — Pattern Detection and Data Extraction

Detect and extract useful entities from text files.

Key requirements:
- `BTA-009-001` Detect emails (SHOULD)
- `BTA-009-002` Detect URLs (SHOULD)
- `BTA-009-003` Detect IPv4 and IPv6 addresses (SHOULD)
- `BTA-009-004` Detect UUIDs (SHOULD)
- `BTA-009-005` Detect JWT-like tokens (SHOULD)
- `BTA-009-006` Detect Base64-like strings (SHOULD)
- `BTA-009-007` Detect hex strings (SHOULD)
- `BTA-009-008` Detect ISO dates and timestamps (SHOULD)
- ...and 3 more

### BTA-010 — Statistics and Frequency Analysis

Analyze large text files to find counts, distributions, repeated lines, top terms, and anomalies.

Key requirements:
- `BTA-010-001` Count lines, words, bytes, characters (MUST)
- `BTA-010-002` Count empty and whitespace-only lines (SHOULD)
- `BTA-010-003` Find longest and shortest lines (SHOULD)
- `BTA-010-004` Find most common lines (SHOULD)
- `BTA-010-005` Find duplicate lines (SHOULD)
- `BTA-010-006` Find unique lines (SHOULD)
- `BTA-010-007` Find most common words/tokens (SHOULD)
- `BTA-010-008` Find most common IPs, URLs, errors, IDs using pattern categories (SHOULD)
- ...and 2 more

### BTA-011 — Bookmarks, Notes, and Timeline

Advanced investigation workflow for marking lines, categories, comments, and navigation points.

Key requirements:
- `BTA-011-001` Create bookmark on line (MUST)
- `BTA-011-002` Create colored bookmark categories (MUST)
- `BTA-011-003` Create bookmark note/comment (SHOULD)
- `BTA-011-004` Bookmark all search results (SHOULD)
- `BTA-011-005` Bookmark all log errors (SHOULD)
- `BTA-011-006` Bookmark timeline panel (SHOULD)
- `BTA-011-007` Export bookmarks and notes (SHOULD)
- `BTA-011-008` Persist bookmarks in session without modifying source file (MUST)

### BTA-012 — Analysis Pipelines

Reusable visual or declarative pipelines for grep/awk/sort/uniq-like workflows.

Key requirements:
- `BTA-012-001` Create pipeline with stages (MUST)
- `BTA-012-002` Filter stage (MUST)
- `BTA-012-003` Extract stage (MUST)
- `BTA-012-004` Transform stage (SHOULD)
- `BTA-012-005` Group by stage (SHOULD)
- `BTA-012-006` Count stage (SHOULD)
- `BTA-012-007` Sort stage (SHOULD)
- `BTA-012-008` Dedupe stage (SHOULD)
- ...and 6 more

### BTA-013 — Performance and Activity Diagnostics

Expose memory, CPU, chunks, threads, background tasks, and rendering health.

Key requirements:
- `BTA-013-001` Show file size and mapped bytes (MUST)
- `BTA-013-002` Show RAM used by document (SHOULD)
- `BTA-013-003` Show chunk cache size (SHOULD)
- `BTA-013-004` Show visible line count (SHOULD)
- `BTA-013-005` Show active background tasks (MUST)
- `BTA-013-006` Cancel background tasks (MUST)
- `BTA-013-007` Pause/resume long-running analysis tasks (SHOULD)
- `BTA-013-008` Show search speed (SHOULD)
- ...and 2 more

### BTA-014 — Smart Copy and Export

Copy or export selected/filter/search/pipeline results into useful formats.

Key requirements:
- `BTA-014-001` Copy as plain text (MUST)
- `BTA-014-002` Copy as Markdown (SHOULD)
- `BTA-014-003` Copy as CSV (SHOULD)
- `BTA-014-004` Copy as JSON (SHOULD)
- `BTA-014-005` Copy as HTML with highlighting (SHOULD)
- `BTA-014-006` Export filtered view (SHOULD)
- `BTA-014-007` Export search results (SHOULD)
- `BTA-014-008` Export extracted entities (SHOULD)
- ...and 2 more

