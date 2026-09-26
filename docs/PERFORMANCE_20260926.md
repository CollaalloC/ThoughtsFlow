# Performance record — 2026-09-26

## Agent snapshot: independent read concurrency

The controlled service benchmark decreased from **136.99 ms to 81.97 ms median** across nine measured snapshots: **40.2% less elapsed time**. Both versions made **five CLI calls per snapshot**, or 45 calls across the measured rounds. This measures an artificial CLI latency model, not live Orca/OMP execution, UI refresh latency, or performance on other operating systems.

The baseline was collected against the sequential `AgentService::snapshot` implementation from HEAD `919c875`, before applying this change. The same ignored test and fixture were then run against the candidate with concurrent reads.

### Measurement method

- Host reported by the harness: `macos-aarch64`; `uname -sm`: `Darwin arm64`.
- Rust: `rustc 1.97.1 (8bab26f4f 2026-07-14)`, Cargo test profile, unoptimized with debug information.
- Real SQLite via `SqliteRepository::connect_in_memory()`, using the production migrations and shared single-connection pool. This includes SQLite query cost, **not disk I/O**.
- One persisted Mission, one settled Task/worker, one unanswered question, one worker-list page, no operation history.
- Every CLI call adds the same artificial 25 ms asynchronous delay, including `status` and `run-current`. Mock responses perform no external I/O and launch no subprocesses or models.
- Two warmup snapshots, then nine measured snapshots. Each measurement times the entire `service.snapshot(...)`; database setup, migration, compilation and warmup are outside the timed interval.
- Only the benchmark test is selected. It checks the resulting task/message projection and records command counts. There is no CI elapsed-time threshold.

| Metric | Before: sequential reads | After: independent reads together |
| --- | ---: | ---: |
| Median | 136.989875 ms | 81.974208 ms |
| Minimum | 135.573458 ms | 81.036750 ms |
| Maximum | 138.613792 ms | 83.548375 ms |
| Measured rounds | 9 | 9 |
| Calls per snapshot | 5 | 5 |
| Total measured CLI calls | 45 | 45 |

Raw sample order, milliseconds:

```text
before = [135.573458, 137.601542, 137.010709, 136.770792, 136.674458,
          137.906042, 138.613792, 136.623542, 136.989875]
after  = [81.444375, 81.036750, 81.739667, 81.974208, 82.092291,
          81.344708, 82.290791, 82.693500, 83.548375]
```

Both runs recorded nine invocations each of `status`, `orchestration run-current`, `worker-list`, `task-list` and `check --peek`.

### Change and safety boundaries

The connection checks still execute in sequence before fetching data. Then `tokio::try_join!` overlaps the independent worker inventory, Task list and non-consuming mailbox reads. With equal per-call delay `d` and one worker page, the CLI delay component changes from `5d` to `3d`; the measured result is consistent with that model. It is not a threefold whole-snapshot speedup.

The Mission lock still spans the entire snapshot. Worker pagination remains sequential within its own branch, including cursor progression, maximum page count and Run identity checks. Task/Dispatch association, release eligibility, unknown-operation guards and all mutation checks are unchanged. No runtime, liveness, permissions or result data is cached.

If any read fails, the whole snapshot remains unavailable and returns no partial task/message data. Pending read futures are dropped. The production CLI runner terminates its dropped child process, but this does not retract queries already received by Orca. These branches contain only reads, and mailbox reads retain `--peek`. If multiple reads fail, the warning can now describe the first observed failure instead of a fixed workers-then-tasks-then-mailbox order.

These remain independent observations of a live runtime, not an atomic Orca transaction. Control actions continue to check the current binding and resource ownership before mutation.

### Reproduce and track

From the repository root, run the current implementation with the ignored benchmark:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  agents::tests::benchmark_snapshot_cli_delay_model --offline -- \
  --ignored --nocapture --test-threads=1
```

The test prints a JSON record with the nine raw samples, median, per-command counts and host platform. Set `THOUGHSFLOW_BENCH_LABEL` to label a recorded run; the default is `current`. **The label does not change the implementation.** The captured historical runs used `before` and `after` respectively.

For a new before/after comparison, run this unchanged harness on each implementation before and after editing `snapshot`; retain both JSON records. To reproduce the historical sequential side on a separate checkout, apply the same benchmark harness to the `919c875` snapshot implementation. Do not compare a different fixture, worker page count, delay, optimization profile or machine as if only scheduling changed.

Regular behavior verification:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib agents::tests --offline
```

Result after this change: **32 passed, 0 failed, 1 ignored benchmark**. A Barrier test first failed against the sequential code, then passed after the change: all three reads must start after the connection checks, and a same-Mission write must wait until the snapshot releases its lock. Additional tests cover stale connection checks starting no queries, each read failure dropping the other two pending futures, and a later worker page failing its Run check without publishing partial data. Existing idempotency, unknown-result, transaction and cross-Mission tests continue to pass.

The Rust mock repository paths now come from `std::env::temp_dir().join(...)` rather than fixed `/tmp` paths. This corrects host path semantics in the fixtures; these measurements and tests were run on macOS, and do not constitute Windows or Linux execution evidence.

### Remaining measurements

Actual performance depends on process startup, runtime scheduling, host load, worker pagination and IPC/UI work that this mock does not model. A follow-up live snapshot benchmark should record those components and the tested Orca version separately. Model-provider latency and task execution throughput require their own measurements.

## Model catalog rendering bound

Provider settings retain the full discovered catalog for exact ID selection and metadata lookup, but render at most 100 matching datalist options. Filtering is local and does not trigger another directory request. This is a structural DOM bound, not a measured browser latency claim.

The public component test loads 500 models, verifies that 100 options are rendered, then types `vendor/model-499` and verifies that the exact formerly hidden model becomes the sole option. It also verifies only one discovery request. Existing profile selection clears only the search filter, so opening a saved Profile with a model absent from the returned list does not hide all newly discovered choices.
