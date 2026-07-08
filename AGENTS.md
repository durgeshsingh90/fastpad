# Agent Instructions

Scope: the entire repository.

Before making code or documentation changes, read `ENGINEERING_PRINCIPLES.md`.
Treat it as the governing project document for FastPad implementation decisions.

When a requested change conflicts with those principles, call out the conflict
and prefer the implementation that preserves UI responsiveness, bounded memory,
lazy work, cancellation, and predictable behavior.

Do not introduce features that eagerly load, parse, index, syntax-highlight, or
render whole documents unless the active mode and file characteristics clearly
require it.
