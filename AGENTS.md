# LogStudio project principles

Guidance for anyone changing LogStudio, including coding agents. [DEVELOPMENT.md](docs/DEVELOPMENT.md) covers the code layout, builds and checks.

## Approach to all project work

- Before implementing or optimizing, identify the actual goal and question assumptions and necessity: must this work be done this way at all?
- Consider removing unnecessary work and changing responsibilities, data flow, or architecture. Propose justified design changes within the scope of the change; do not preserve a flawed architecture merely to minimize the diff.
- Compare alternatives against correctness, user experience, performance, maintainability, and measured evidence. Distinguish observations from hypotheses and account for migration and compatibility costs.

## Expensive log processing

- Apply these principles to file loading, decoding, parsing, indexing, searching, filtering, statistics, paging, and rendering whenever CPU, memory, I/O, or UI responsiveness is material.
- Before optimizing an expensive operation, ask whether it is needed, when its result is needed, and which component should own it. Consider eliminating duplicate work, deferring optional work, and changing data flow before making the existing implementation faster.
- Compare semantically equivalent on-demand processing, streaming or block processing, cache reuse, and native or indexed approaches as relevant to the bottleneck. Eager preprocessing remains an option when measured benefits justify its startup and memory costs.
- Evaluate the effect on first usable display, subsequent interactions, completion time, and peak memory together. Keep long-running work cancellable, prevent stale results from replacing current results, and distinguish partial results from final counts.

## Behavior to preserve

- Preserve Android/iOS parsing behavior, PID/TID as strings, tag-filter semantics, physical line boundaries, and raw-log fidelity across processing strategies.
- Large-log literal and regex search uses ripgrep's grep-regex/grep-searcher crates, pinned and linked directly into the shared Rust core. The desktop app has no Python runtime or helper sidecar. Case-insensitive matching uses standard Unicode simple case folding. Document grammar, case semantics and unsupported patterns. Do not build a bespoke SIMD matcher.
- Keep first available rows/search independent of full field metadata. Preserve raw bytes, CR/LF/CRLF rows, field filters, bounded later pages and highlights. Use measured bounded concurrency, generation guards and cancellation independent of match callbacks.
- End users must not silently acquire a compiler or toolchain at launch. Provide reproducible native builds and honest platform verification limits.
- Optimize for predominantly case-insensitive log searches, but do not assume whole-file eager lowercase normalization is necessary.

## Validation proportional to the change

- Select checks based on the affected behavior and risks. Documentation-only changes do not require performance benchmarks; localized implementation changes need focused correctness checks and relevant measurements.
- For substantial changes to expensive processing or its architecture, use representative synthetic Android, iOS, and Unicode workloads. Evaluate affected stages and downstream effects, including first usable display, loading/parsing/indexing completion, first and repeated searches or filter applications, exact counts, paging, cancellation, and memory.
- Separate isolated operation measurements from integrated backend and browser results. Record workload, environment, and measurement limitations; distinguish observations from hypotheses and identify unverified platforms or interactions.
- Tests, fixtures and benchmarks use synthetic logs. Never commit real device logs or values copied from them.
- For any UI change (layout, styling, interaction states such as hover/focus/disabled, copy), capture screenshots of the affected UI after the change and attach them to the pull request. Show the changed state itself (for example, the hovered button), and cover light and dark schemes when colors are affected. If a state cannot be captured, say so and describe what was verified instead.
