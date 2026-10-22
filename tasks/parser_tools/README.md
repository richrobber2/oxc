# Parser tools and DHAT allocation checks

Local, unpublished Oxc consumer for bounded JS/TS/JSX/TSX syntax validation and
formatting. This adds a tool around existing Oxc APIs; it changes no grammar,
formatter rules, linter policy or other tools' allocators.

## Source checking and formatting

```sh
cargo build --locked -p oxc_parser_tools --profile release-with-debug
target/release-with-debug/oxc-parser-tools check source.ts
target/release-with-debug/oxc-parser-tools format source.ts
```

The tool accepts a regular UTF-8 file of at most 1 MiB. It reads at most the
limit plus one byte rather than trusting file size. Unsupported extensions,
parse diagnostics (including recovered parses), fatal parses, semantic early
errors and printing failures return failure. The process opens source read-only;
it writes formatted stdout only after
validation and successful printing. To save it, write a separate temporary file
and rename it only on success. Shell redirection opens/truncates its destination
before the tool runs, so never redirect directly onto the input path. Diagnostic
output goes to stderr, with at most ten diagnostics, using borrowed source
instead of cloning the whole file per diagnostic.

Validation uses the normal parser with regular-expression parsing enabled,
then semantic syntax checking. Those structures are dropped before the text-in
formatter builds its own AST/IR. This deliberately pays for a second parse to
keep the formatter's parser configuration separate from semantic analysis.
Check output reports statement/comment counts and validation-arena used bytes
and capacity; capacity includes spare space and is not AST object size.

These are **syntax and input-handling checks**, not vulnerability detection,
taint analysis or a sandbox. Code and regular-expression literals are parsed,
never executed. Dynamic
`RegExp(string)` patterns are not validated. Unix opens are nonblocking before
the opened handle is checked, so FIFOs cannot bypass the regular-file rejection.
Use oxlint, Opengrep, OSV-Scanner and Gitleaks for their respective
rule, dependency and secret checks. Byte and displayed-diagnostic limits do not
bound parser recursion, internal diagnostic allocation, CPU time or printer
output size. Isolate hostile inputs with OS process/resource limits. This tool
is intended for explicitly selected local source files.

## Separate DHAT executable

```sh
cargo build --locked -p oxc_parser_tools --features heap-profile --profile release-with-debug
target/release-with-debug/oxc-parser-tools-heap diagnostics-owned 3
target/release-with-debug/oxc-parser-tools-heap diagnostics-borrowed 3
target/release-with-debug/oxc-parser-tools-heap check-js 3
target/release-with-debug/oxc-parser-tools-heap check-ts 3
target/release-with-debug/oxc-parser-tools-heap format-js 3
target/release-with-debug/oxc-parser-tools-heap format-ts 3
target/release-with-debug/oxc-parser-tools-heap reject-malformed 3
```

On constrained machines, pass the same overrides to every build/check:
`--config profile.release-with-debug.lto=false
--config profile.release-with-debug.codegen-units=8
--config profile.release-with-debug.debug=1`. These retain optimization and line
information while reducing link time. Run Cargo operations serially in the
same target directory. Use a package-scoped build, not a whole-workspace build,
to avoid unifying the existing track-allocations feature into this harness.

DHAT 0.3.3 is optional, and its global allocator is declared only in the dedicated
executable. Normal tools retain their normal allocator. Its reports include
arena **chunk** allocations, unlike the existing allocation tracker's deliberate
arena-chunk exclusion. DHAT cannot attribute individual objects within a chunk.
Validation arena usage/capacity is captured separately, outside profiling, and
included as `[usedBytes, capacityBytes]` in `validationArena` for valid cases.

Each invocation is a fresh process with 1–100 iterations of one fixed synthetic
case. Fixture construction, preflight validation, formatter idempotence checks,
a result-checksum baseline and lazy-state warmup are outside the window.
Measured iterations include complete processing, output hashing and destruction
of per-operation values. Reports and summary serialization happen afterward.
Checksums use a non-cryptographic FNV-1a accumulator for observability, not for
security. Formatted-code and diagnostic checksums cover their full strings;
check-only cases hash statement/comment counts, not complete AST equivalence.
A mismatch fails the invocation. Formatter cases check 256 statements
and comments; curated JS/TS formatting must be idempotent, and the malformed
case must be rejected. The diagnostics pair renders identical messages/labels
against the same 840,016-byte source, with 64 diagnostics: one variant
models full-file cloning per diagnostic from the old example pattern; the other
calls the actual borrowed-source helper. This isolates source-copy cost, not
whole-parser or installed oxlint memory. No actual vulnerability is executed.

Reports use create-new files under `target/parser-tool-profiles`, with private
Unix creation permissions. DHAT is explicitly finished before success; failure
paths return normally so the profiler destructor can flush. A signal or abort
can prevent flushing. DHAT slows execution, so these are allocation measurements,
not latency benchmarks, total process RSS or a proof of freedom from leaks.

## Verification

Run package-scoped tests and lint with the same profile overrides if used above:

```sh
cargo test --locked -p oxc_parser_tools --all-features
cargo clippy --locked -p oxc_parser_tools --all-targets --all-features -- -D warnings
cargo fmt -p oxc_parser_tools -- --check
```

The tool's tests cover fail-closed malformed/semantic inputs, Unicode comments,
TypeScript formatting idempotence, byte/UTF-8 limits and exact owned/borrowed
diagnostic equivalence. CLI tests also cover nonblocking FIFO rejection, input
file preservation and the DHAT relative-allocation guard. Oxc's existing grammar
and formatting conformance suites remain the authority for language coverage; this consumer does not replace them.

## Allocation regression test

With `--all-features`, the CLI integration suite runs the two diagnostic DHAT
variants in separate processes against the same synthetic input. It requires
matching output checksums, zero tracked end retention, and borrowed-source total
and peak allocations below one tenth of the source-copy variant. This relative
guard avoids imposing machine-specific absolute byte counts. It measures the
rendering helper rather than whole-parser memory or total process RAM. The
source and prepared diagnostics are intentionally outside the profiling window.

Reports are relative to the current working directory, independent of
`CARGO_TARGET_DIR`; the command examples assume the workspace root and default
Cargo target directory. The test uses and cleans its own temporary directory.
