# Rust macOS receipt

The `rust-macos` check runs on `macos-latest` for pull requests and pushes to `main`. It checks out the explicit pull request head, including a fork head, without persistent credentials. The event SHA and workflow SHA remain separate from that tested checkout.

`tools/run-rust-ci.py` owns these commands, in this order.

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --release --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

An ordinary failure does not suppress later commands. A timeout, output limit, launch failure, or exhausted resource budget stays red. An interruption leaves later phases explicitly unexecuted. The job has a 120 minute timeout. The producer has a 100 minute budget and individual limits of 5, 35, 35, and 45 minutes. It uses three Cargo build jobs, disables incremental compilation, and stops a child process group when its output exceeds 16 MiB or free disk drops below 1 GiB. A phase needs 2 GiB free to start.

The CLI refuses Cargo discovery and execution unless it observes GitHub Actions, a `github-hosted` runner, macOS, the `rust-macos` job, and the expected workspace. It also checks the event, tracked tree, lock, actual architecture, Rust release, Cargo version, and installed rustfmt and clippy components. These checks reduce accidental local execution. They are not authentication against someone deliberately fabricating a runner environment.

## JSON schema version 1

The artifact is named `rust-macos-R-A`, where `R` is the run ID and `A` is the attempt. It contains only `result.json`, at most 1 MiB. The receipt starts red before preflight. These are its top level fields.

| Field | Value |
| --- | --- |
| `schema_version`, `context` | `1`, `"rust-macos"` |
| `status` | `"rouge"` or `"vert"` |
| `started_at`, `finished_at` | UTC ISO timestamps. `finished_at` is null until finalization. |
| `provenance` | Null until all preflight checks pass. Otherwise the object below. |
| `phases` | Four objects in command order, with IDs `fmt`, `clippy_debug`, `clippy_release`, `test_workspace`. |
| `reasons` | A list of closed reason identifiers. |
| `inventory_complete` | Boolean. A limit or failed source inventory makes this false and the result red. |
| `cache` | The cache object below. |

`provenance` contains these fields.

```text
source_repository, workflow_repository   owner/name
source_sha, checkout_sha, tree_sha       lowercase SHA40
workflow_sha, event_sha                 lowercase SHA40
workflow_ref                           repository/.github/workflows/rust-macos.yml@ref
event                                  pull_request or push
run_id, run_attempt                     positive integers
job_name                               rust-macos
lock_sha256                            lowercase SHA256
toolchain                              channel, rustc_release, rustc_commit_hash,
                                       rustc_host, cargo_version, cargo_commit,
                                       components = [rustfmt, clippy]
os                                     name = macOS, version, runner_label = macos-latest,
                                       optional ImageOS and ImageVersion
architecture                           machine, runner_arch, rust_host
```

On a pull request, `source_repository` is the head repository. `workflow_repository` is the repository receiving the event. `source_sha` and `checkout_sha` must equal the expected head. `event_sha` can be the synthetic merge SHA. `workflow_ref` and `workflow_sha` identify the workflow GitHub selected. The producer checks `HEAD`, the tracked tree, and `Cargo.lock` before and after execution. An invalid preflight leaves `provenance=null`.

Each phase has the following fields.

```text
id, command, profile           fixed command registry, profile check/debug/release
state                          not_run or executed
exit_code, duration_ms         observed integers, or both null when not_run
counts                         null, or {passed, failed, ignored, filtered}
summary_complete               null outside an executed test phase, otherwise boolean
summaries                      list of {kind, target, counts, expected, summary_complete}
failures                       sorted tracked source identifiers
unresolved_failures            integer
reasons                        list of closed reason identifiers
```

A summary `kind` is `binary` or `doctest`. Its `target` comes from tracked Cargo manifest names or Rust file stems. Its `expected` is the announced test count, or null if absent. Its `counts` is null until a terminal summary appears. Multiple test groups for a doctest have separate entries. Aggregate counts contain only observed terminal summaries. Missing summaries, inconsistent counts, and a workspace total of zero remain red. Nonzero ignored or filtered counts remain red. No exemption or test filter is added.

`exit_code` preserves the process return code, including negative signal codes. A launched process that times out is `executed` with the code observed after termination and a `timeout` reason. A launch failure stays `not_run` with null code and duration. Formatting and clippy never acquire test counts. No npm, web, or UI phase is implied.

Failure identifiers are `tracked/path.rs::function` or a tracked `path.rs:line` from a doctest or panic. A location includes only verified coordinates, never the thread name or diagnostic. Locations and function identifiers can describe the same failed test. Counts come only from terminal summaries. A failure identifier cannot be overruled by a summary reporting zero failures. The producer resolves function leaves against tracked source. Duplicate names stay unresolved rather than choosing a file. It does not import the Linux inventory, whose current platform module parsing fails on `crates/wrapper/tests/keychain.rs`. This limitation does not suppress the four Rust commands. Diagnostics, assertions, arbitrary target names, environment dumps, and exception text never enter the receipt or console. Console output contains only fixed phase names, states, and return codes. Child output stays in bounded memory and is never uploaded.

The closed reasons are `preflight_failed`, `runner_error`, `checkout_changed`, `required_phase_failed`, `inventory_incomplete`, `report_limit`, `cache_restore_failed`, `cache_key_mismatch`, `launch_error`, `timeout`, `output_limit`, `interrupted`, `budget_exhausted`, `disk_budget`, `command_failed`, `unrecognized_target`, `inventory_limit`, `unbound_summary`, `missing_summary`, `measured_tests`, `empty_tests`, `failed_tests`, `ignored_tests`, `filtered_tests`, and `unresolved_failure_identifier`. A report size limit removes detailed lists, forces `summary_complete=false` for test results, and remains red.

## Cache and external proof

Each attempt prepares a new `CARGO_HOME` under `RUNNER_TEMP`. Only `registry/cache` archives are restored or saved. The cache excludes credentials, registry indexes, extracted sources, Git checkouts, and built targets. The build directory is separate under `RUNNER_TEMP`.

The exact key is `rust-macos-v1-macOS-M-T-L-H-R-exact`. `M` is the observed machine architecture. `T` is the SHA256 of the sorted JSON toolchain object. `L` is `lock_sha256`, `H` is the source SHA40, and `R` is the run ID. There is no prefix fallback. The producer validates the action's `cache-matched-key`. It discards archives from a nonexact match before running Cargo and stays red. Attempts of one run share a key but use separate empty Cargo homes.

`cache` has `key`, `restore_outcome`, `hit`, `saved`, `bytes_before`, and `bytes_after`. Unobserved values are null. The exact action `cache-hit` output determines the boolean `hit` after a successful restore. A cache restore failure remains red while the commands still run. `saved` is always null because saving happens after the producer exits. Cache action durations and successful save must be observed externally. A green receipt alone does not prove that the cache was saved.

Firstmate must recoup the expected head with the exact run, attempt, `rust-macos` job, check suite, check run, and artifact APIs. The run, job, and check head SHAs must agree. The artifact must be unique for `R` and `A`, unexpired, and associated with that run. Verify the GitHub ZIP digest before extracting the single bounded JSON member. Record its artifact ID, archive digest, and JSON hash separately. Verify the executed workflow and producer against the reviewed blobs. The artifact upload uses `always()` without masking a required failure. Upload failure also fails the job.

The first observation must be a real miss. Read its complete result first. Only after a green first observation and an externally confirmed save may one second attempt of the same run, head, and lock measure an exact hit. Compare phase durations, archive bytes, and cache action durations. This measures archive reuse. It makes no compilation speed claim. A red first observation leaves the warm observation missing. Missing artifacts, cancellation, checkout failure, and runner disappearance never count as green evidence.

## Local verification and remaining scope

The Python tests mock all Rust subprocesses. A small explicitly named `fake_rust_backend.py`, launched with Python, tests process limits. It is never named Cargo. Set an existing evidence directory to keep temporary fixtures there.

```sh
PYTHONDONTWRITEBYTECODE=1 RUST_CI_TEST_ROOT=/absolute/evidence/directory python3 -m unittest discover -s tools -p test_rust_ci.py
```

No local Cargo suite runs for this delivery. Known local aborts in `state_scale`, `delegation`, and `routine_quiet_scale` remain unresolved local requirements. Runner failures must be inventoried from the actual receipt without new ignores, skips, quarantine, or weakened assertions. Speech retains only its previously written exemption. The workflow adds no exemption for macOS permissions.

The Rust receipt does not prove JS, UI, the complete suite, or the first green night of #550. The existing compile and nightly workflows are unchanged. Other #570 work remains undelivered. Automatic closure of all #570 would conflict with this partial scope.

Firstmate tool proposals are **NON APPLIQUÉES**. `pret` could require `rust-macos` plus its JSON on exact head H, a green status, four zero return codes, and no failed, ignored, or filtered tests. `preuve-main-vert` could replace only its Rust evidence with that receipt and preserve npm, web, and UI as distinct unexecuted requirements. A proposed `cargo-gate` change would reduce local concurrency to two slots. A proposed launcher change would run only targeted local checks and reread the GitHub proof. UI work would still require a local build. This delivery changes none of those tools. No nightly check is activated before the first green night.
