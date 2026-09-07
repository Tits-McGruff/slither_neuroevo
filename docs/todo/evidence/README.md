# Evidence directory policy

This directory contains retained artifacts and historical reports from the
Rust-authoritative migration. Many early reports are intentionally detailed
because they reconstructed disputed history, characterized the TypeScript
reference, or captured benchmark/compatibility data that was not otherwise
encoded in tests.

For current and future work, do not create or expand a long Markdown evidence
report for every implementation slice. Prefer, in order:

1. focused automated tests/fixtures for correctness and regressions;
2. CI artifacts for broad cross-platform validation;
3. retained machine-readable benchmark/compatibility artifacts when numbers or
   raw data matter;
4. a short entry in `../rust-authoritative-runtime-implementation-log.md` for
   the meaningful checkpoint, important invariant, commit and remaining gate.

Create a standalone evidence report only when it preserves material information
that those mechanisms cannot express well, such as a benchmark methodology,
owner-data compatibility inventory, unusual cross-platform investigation, or a
high-impact failure analysis. Resolved setup/sandbox/path failures, repeated
full-suite counts, temporary-directory names, source-byte bookkeeping and
routine reviewer-status narration do not belong in durable evidence.

The existing verbose reports remain historical records. Their format is not a
template and future agents should not imitate their length merely for
consistency.
