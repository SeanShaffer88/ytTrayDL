# 0.8.1 GUI Responsiveness And Tool Progress

## Measured Cause

An owned private portable profile with the real managed yt-dlp/ffmpeg/ffprobe
tools reproduced the slow UI at 600×450 Main and 600×670 Settings, 96 DPI.
Field-only instrumentation records bounded in-memory segment timings; its
worker writes the report on normal exit, without per-frame disk output.

| Initial comparison | Baseline 0.8.0 | First 0.8.1 candidate |
|---|---:|---:|
| App update wall time, p95 | 768.261 ms | 1.116 ms |
| App update wall time, maximum | 1,658.300 ms | 22.786 ms |
| Geometry at frame start, p95 | 438.572 ms | 0.015 ms |
| Geometry at frame end, p95 | 327.389 ms | Removed redundant pass |
| Rendering including immediate child presentation, p95 | 2.014 ms | 1.077 ms |
| Settings persistence, p95 | 0.417 ms | No asynchronous persistence introduced |

These are observed `App::update` wall times, including synchronous immediate
child rendering/presentation but excluding root presentation. They are not
physical keyboard-to-photon, idle CPU, accessibility, or display-latency results.
The baseline recorded 42 updates; the first candidate retained 1,365 updates.
The field recorder has an explicit 8,192-sample bound. Final delivery requires
three matched frame-only runs, plus separate native editing/progress checks;
their exact-binary evidence belongs to the delivery verification report.

The main bottleneck was geometry discovery, not ordinary status lookup, settings
file writes, or the 200 ms fallback repaint. PID-only global window enumeration
queried titles on other threads in the same process. Windows can send synchronous
`WM_GETTEXT` for those windows, blocking the GUI. Discovery now enumerates only
the calling GUI thread and checks PID/TID before querying titles. Cached HWNDs
are validated with owner identity and a window-role property; native destruction
removes that identity, so recycled handles cannot silently target another window.

Normal bounds are captured once per frame for open windows before close/hide
intents. A newly created window gets one targeted restore. Hidden or minimized
geometry never overwrites normal bounds, and minimization does not re-arm
restoration. Recovery threads use the GUI-published Main HWND, including tray-only
startup; they never fall back to an unbounded cross-thread title search.

## Progress And UI State

Main and Settings show the same canonical operation state: stage, byte count and
total when known, measured speed/ETA, and Cancel. Unknown totals remain
indeterminate. Byte updates are bounded to five per second; stage and terminal
events request a root repaint immediately. Cancel affects only its operation,
inherits whole-app shutdown cancellation, and remains busy until cleanup is
acknowledged. A successful commit that wins a late cancel is reconciled as Ready.

Tool/autostart status is resolved off-thread into a cached snapshot on startup,
Settings opening, path changes, external settings reload, and operation completion.
Generation IDs reject stale snapshots and progress. A disconnected/panicked
worker clears busy state and offers retry rather than spinning forever. Failed
version probes display an unavailable result rather than permanent ellipsis.

The manual URL editor and progress/Cancel are placed before potentially long
engine warnings; starting a tool operation resets that panel scroll position.
Settings tool paths are elided with full hover text and explicit Copy, and Close
uses supported text. Existing colors and wizard flow remain unchanged.

The periodic repaint remains a fallback for native/background event paths whose
wake coverage is not complete. Existing settings-file checks and small measured
synchronous saves remain; no zero-idle-CPU or completely I/O-free UI claim is made.

## Verification Contract

- Full Rust/native/MSRV gates, including owned other-thread 150 ms `WM_GETTEXT`
  negative control, exclusion, cached handle destruction/recreation, and worker
  disconnect/retry regression tests.
- Actual official typed tool provisioning through the canonical pipeline;
  nonzero byte, speed/ETA and post-commit terminal events retained.
- Actual private production GUI editing, visible Main/Settings progress, Cancel,
  Settings destruction/reopening, retry, and normal Exit observed independently.
  The exact filename-template marker was read back from settings. Both cancelled
  downloads preserved the prior manifest and all three tool hashes, left only the
  original generation and no owned download children. Original GUI launch did not
  retain an exit-code handle; normal Exit was observed via UI, log and process
  disappearance without inventing a captured exit code.
- `Test-GuiResponsiveness.ps1 -FramesOnly` explicitly measures frame/native-message
  workloads without claiming typing qualification. Its default mode still fails
  unless the exact editable field receives the expected text. Initial unsuccessful
  synthetic typing attempts are retained and do not count as editing passes.
- Three matched frame-only baseline/candidate runs, recovery/geometry checks and
  exact-head CI/package checks remain required delivery gates. No host installer
  execution is authorized or claimed; installer lifecycle checks use hosted VMs.

The CUA progress/cancel fixture used a temporary loopback CONNECT relay limited
to official GitHub hosts, four MiB/s, eight connections and five minutes. TLS
remained end-to-end; only the private child received HTTPS_PROXY. The relay was
identity-checked and stopped after the UI test. This fixture is not shipped and
is not used during matched frame measurements.
