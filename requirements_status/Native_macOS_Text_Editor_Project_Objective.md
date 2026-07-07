# Native macOS Text Editor - Project Vision & Objective

Version: 1.0

## Purpose

This project is an open-source, native macOS text editor written in
Rust.

The objective is **not** to compete commercially with VS Code,
Notepad++, Sublime Text, BBEdit, Zed, or other mature editors.

The project exists to:

-   Build a world-class systems programming project.
-   Learn operating systems, rendering, memory management and
    performance engineering.
-   Create a personal editor optimized for extremely large text files.
-   Serve as a reference implementation for building a high-performance
    native desktop application.

## Guiding Philosophy

The editor shall follow the Unix philosophy:

-   Do only the work required.
-   Never block the UI.
-   Stream whenever possible.
-   Prefer lazy evaluation.
-   Render only visible content.
-   Avoid unnecessary memory allocation.
-   Optimize based on measurements, not assumptions.

Every architectural decision should prioritize:

1.  Correctness
2.  Simplicity
3.  Performance
4.  Low memory usage
5.  Native macOS behavior
6.  Maintainability

## Platform

Initial platform: - macOS only

Implementation language: - Rust

The application should use native macOS APIs where appropriate and avoid
Electron, embedded browsers, HTML/CSS/JavaScript UI, telemetry, cloud
dependencies and unnecessary frameworks.

## Primary Goals

-   Native macOS application
-   Extremely fast startup
-   Extremely fast scrolling
-   Large file support
-   Low memory usage
-   Clean architecture
-   Open source
-   Educational codebase

## Explicit Non-Goals (v1)

-   IDE
-   Debugger
-   Terminal
-   Git integration
-   AI assistant
-   Extension marketplace
-   Remote collaboration
-   Cloud sync
-   User accounts
-   Analytics

## Large File Philosophy

Large files are first-class citizens.

The editor must support files ranging from kilobytes to hundreds of
gigabytes (subject to filesystem and OS limitations).

The application shall never assume the entire file can or should be
loaded into memory.

### Read Mode

Purpose: Optimized viewing and analysis of very large files.

Characteristics:

-   Memory mapped file access
-   Read-only
-   Minimal RAM usage
-   Viewport rendering only
-   Incremental search
-   Incremental syntax highlighting
-   Zero unnecessary copies
-   Background indexing

### Edit Mode

Purpose: Optimized editing while maintaining high performance.

Characteristics:

-   Piece Table editing model
-   Efficient undo/redo
-   Efficient insert/delete
-   Background save
-   Incremental indexing
-   Viewport rendering
-   Lazy computation

## Automatic Mode Selection

The editor should automatically select an operating mode based on:

-   File size
-   Available memory
-   User preference
-   Previous session preference

Users may manually switch modes whenever technically possible.

## User Experience Requirements

Opening a 100 KB file and a 100 GB file should feel fundamentally
similar.

The user should be able to interact with the file immediately while
background work continues.

Time-to-first-interaction is more important than total processing time.

Immediately after opening a large file, users should be able to:

-   Scroll
-   Select text
-   Copy text
-   Search
-   Jump to offsets
-   Navigate

Background tasks must never block interaction.

## Performance Objectives

Target characteristics:

-   Near-instant application startup
-   Immediate first paint
-   Immediate viewport rendering
-   Smooth scrolling
-   Responsive typing
-   Constant-feeling memory usage
-   Incremental processing

Performance should be benchmark-driven.

## Architecture Philosophy

Keep the architecture intentionally small.

Avoid unnecessary enterprise patterns.

Prefer focused modules with single responsibilities.

## Open Source Goals

This repository should become:

-   Learning resource
-   Portfolio project
-   Reference architecture
-   Example of efficient systems programming

Architecture documentation should be maintained alongside the
implementation.

## Success Criteria

The project is successful if:

-   It feels native.
-   It remains responsive on extremely large files.
-   The codebase is understandable.
-   Performance decisions are measurable.
-   The architecture remains simple.
-   It becomes the author's primary text editor.
