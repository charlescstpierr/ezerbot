# Fixture process ownership

`wrapper-chrome-fixture.cjs` admits two commands through one `createFixtureOwner` lifetime. The wrapper receives its launch identity once through private stdin. Chrome is the official headless shell selected by `pinnedFixtureShell()`, with crash reporting disabled. The current consumer is `generated-dialogs.test.js`.

The lifetime owns admission, all native supervisors, writer inspection, and one removal worker. Its root identity is the original device, inode, and canonical path. Closing admission is terminal. Repeated `close()` calls return the same promise. No caller supplies a PID as signal authority.

`await owner.spawn()` publishes its process handle before delivering `error`, `exit`, or `close`. IPC may batch all of those messages with the launch acknowledgment. The client records terminal state immediately and queues lifecycle notifications until the caller can attach listeners. A process that has already ended cannot regain signal authority during that publication interval.

## Signal authority

Each native supervisor creates its own session and forks an anchor. The anchor becomes the leader of a separate group and stays unreaped until the supervisor has sent its last group signal. The useful child joins that group before the supervisor releases its exec barrier. A dead but unreaped anchor still reserves its identifier. The supervisor never uses a group identifier after retiring that reservation.

Useful-child signals apply only while that direct child remains unreaped. Reaping changes its state before any further request can signal it. The useful child can be reaped before the group is retired, allowing the synthetic escaped writer to observe its parent's departure. The public process object's `pid` is diagnostic. Its `kill()` method addresses an opaque launch record.

The anchor holds a private lease pipe. Supervisor death closes the pipe and the anchor kills its own immutable group. Runner death closes the root owner's private IPC channel, which closes admission and performs the same cleanup. Control descriptors do not cross payload exec. Partial control, malformed requests, and failed exec never authorize an unregistered target.

## Writer termination

On Linux the native supervisor becomes a child subreaper before it forks. It first stops the reserved group, retires and reaps the anchor, reaps the useful child, and drains adopted descendants through `ECHILD`. A zombie leader alone is insufficient because Linux can retain other live threads. A failed or timed-out drain retains the root and reports failure.

On macOS the useful browser is reaped before the complete process census. For the pinned shell's admitted workload, the browser is the child-launch producer and ordinary renderer, GPU, and utility children inherit its group. These children do not become independent producers after browser termination. XNU publishes `SZOMB` after the task's threads are terminal. The census rejects stderr, malformed or truncated output, and output missing its own inspector or owner. Any remaining live group member delays removal. This argument is specific to the pinned workload, not arbitrary child programs.

The unchanged synthetic escaped writer retains the root in its arguments, creates no children, and ends after its parent disappears. Path observations wait for that writer or refuse cleanup for unknown holders. They never grant signal authority. A neighbouring path is not a holder. Linux zombie rows with multiple threads still count as live.

The wrapper's admitted operations create synthetic documents and previews in process. This fixture does not exercise its external application opener. The runner closes its CDP connection and fake HTTP server before asking the owner to remove the root.

## Removal and bounds

One separate worker checks the root identity again and calls `rmSync` once with `maxRetries: 0`. The owner requires a successful worker exit and receipt before reporting removal. The worker prevents a synchronous filesystem call from blocking the owner's deadline. A timeout signals only that still-owned worker and reports indeterminate deletion. It does not claim a stalled kernel operation was rolled back.

Each launch, command write, process census, descendant drain, and removal has a bound. The client also bounds its response wait. It can time out before receiving the owner's final diagnostic and then reports indeterminate deletion. Unknown state, failed launch, incomplete census, and changed root identity remain failures. There is no deletion retry or fallback authority based on process names, paths, timestamps, or historical IDs.

The path-based identity check is not an atomic inode-addressed deletion primitive. The private fixture root and its parent are trusted against hostile rename or replacement. The admitted workload excludes arbitrary replacement binaries, injected native code, crash-reporting flags, and unrelated hostile writers. Changing the shell pin, launch flags, or fixture consumer requires revisiting this model.

## Source basis and checks

The Chromium source basis is tag `155.0.8059.12`. The official distribution pin and installation receipt identify the executable; the fixture does not claim a reproducible source-to-binary build.

- [Headless crash initialization](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.12/headless/lib/headless_content_main_delegate.cc) requires explicit activation, absent here.
- [Launch options](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.12/base/process/launch.h) and [Mac child launcher](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.12/content/browser/child_process_launcher_helper_mac.cc) preserve ordinary group membership.
- [Linux zygote host](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.12/content/browser/zygote_host/zygote_host_impl_linux.cc) and [zygote fork path](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.12/content/zygote/zygote_linux.cc) retain the inherited group for this no-sandbox invocation.

`fixture-process-owner.test.cjs` checks the native admission and reservation protocol. `fixture-lifetime-publication.test.cjs` delivers batched lifecycle messages through an in-memory transport and verifies that the caller observes them after acquiring the handle. `wrapper-chrome-fixture.test.cjs` checks root lifetime, writer extinction, runner death, foreign witnesses, and bounds. Neither diagnostic PID mutation nor numeric-command injection is a demonstration of actual kernel PID reuse. `generated-dialogs.test.js` exercises the real wrapper and shell through authenticated HTTP and CDP.
