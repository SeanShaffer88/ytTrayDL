# 0.7.1 Windows field qualification

This patch adds saved window geometry, repairs real-media completion paths and
hidden-window recovery, and makes installer data preservation a required gate.
Release publication requires a passing PR, independent review, and verification
of artifacts from the passing merged commit. CI results remain authoritative for
the exact commit being shipped.

## Verification matrix

| Area | Observed qualification / required gate |
|---|---|
| Native full suite | 112 tests passed; format, both Clippy targets, Windows check and release build passed |
| Real media | All nine loopback scenarios passed locally against both tool sets and in hosted CI: video, merge, audio, subtitles, playlist, retry, pause, cancel, shutdown |
| Final output | Actual file existence/content, UTF-8 paths, subtitle final paths, completion-hook paths and persisted history asserted |
| Actual GUI ffmpeg exit | GUI-confirmed download, observed real ffmpeg, actual main Exit, normal exit0, five descendants gone, no completed history or later writes |
| Window geometry | Native work-area clamp observed; Main/Settings/Downloads normal rectangle and saved JSON assertions passed. Full interactive harness results are recorded separately |
| Installer | Both clean and actual 0.6.1-upgrade tests reproduced foreign-media deletion. Broad deletion removed; all four corrected hosted lifecycle jobs must pass before merge |
| Linux/Windows and MSRV | Required stable gates and Rust 1.89 locked checks, including the optional field harness |
| Release payload | Verify merged commit, x64/version 0.7.1, strict portable inventory, ZIP roundtrip, checksums and published-asset readback before completion |

Window geometry uses bounded, versioned `window-state.json` alongside settings.
Main, Settings and Downloads retain normal size and position; About retains
position. Hidden/minimized/maximized state is not restored. Transient prompts
remain anchored and are translated inside the nearest monitor work area. Pure
tests cover negative monitor origins, changed DPI, oversized/scaled bounds,
corrupt or unknown state, backward defaults and consecutive saves. Actual native
geometry runs used one monitor at 96 DPI; no physical monitor/DPI change is claimed.

GUI tests use private profiles and exact process/window ownership. Native tray
menu command delivery is distinguished from a physical notification-area click.
A CUA-confirmed actual Download click and actual UIA Exit were observed separately.
The scripts preserve blocked/failed attempts rather than treating unavailable
controls as passing. They do not certify accessibility.

Loopback fixtures qualify downloads and postprocessing, not external public
extractor discovery. Startup argv, registry and shortcut checks plus a private
`--background` launch do not claim an actual OS sign-in/reboot. Installer execution
runs only on disposable hosted Windows VMs, never the user's host.
## Running the field harness

Build with `cargo build --locked --features field-validation --bin field_validation`.
The binary accepts explicit `--yt-dlp`, `--ffmpeg`, `--ffprobe`, a new `--root`,
`--scenario all`, and `--timeout-seconds 120`. It uses original generated media
and a loopback server. Never substitute a user's profile or authentication data.
CI resolves exact release URLs and SHA256 pins in `tools/Get-FieldTools.ps1` and
retains version, hash and license/provenance metadata. Tools are not redistributed.

Installer tests require `-TrustIsolatedRunner`, Windows, `GITHUB_ACTIONS=true`,
and `RUNNER_ENVIRONMENT=github-hosted`; they refuse ordinary host execution.
Each scenario runs in its own job, with a 300-second bound per installer phase.
The actual application library loads synthetic settings/history before and after
upgrade/uninstall, and hashes verify data preservation. Registry and shortcut
checks do not claim actual OS sign-in or GUI behavior on the headless runner.
The user-media sentinel under the install directory deliberately tests whether
uninstall deletes files that the installer does not own; a failure blocks release.

## Dependency review

Updated one at a time: `clipboard-win` 5.4.1 → 5.4.3 and `async-executor` 1.14.0
→ 1.14.1. Full baseline test suites passed individually on Windows; Rust 1.89
remains the MSRV and Tokio remains 1.54.1. The [clipboard source comparison](https://github.com/DoumanAsh/clipboard-win/compare/3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342...ed27008ae5d1f10cfbb266de039f0fb35fd05a56)
includes bounded HTML fragment handling and a five-millisecond clipboard-open
retry sleep. The [executor changelog](https://github.com/smol-rs/async-executor/blob/master/CHANGELOG.md)
describes a use-after-free regression fix during executor destruction.

`cargo audit --json` refreshed advisory data and reported zero known vulnerabilities against
database commit `965bf9ad083b58e6340d65829d85217b18be9290` (2026-11-03).
The scan completed on 2026-11-05 UTC with 446 dependencies and 1,403 advisories. Six notices
remain visible and unsuppressed:

| Package / advisory | Actual target reachability and disposition |
|---|---|
| proc-macro-error 1.0.4 / RUSTSEC-2024-0370 | No active inverse tree on Windows or configured Linux target; retained optional platform graph, deferred upstream migration |
| ttf-parser 0.25.1 / RUSTSEC-2026-0192 | Windows and Linux through epaint → ab_glyph → owned_ttf_parser; font-stack migration requires separate compatibility review |
| zbus_names 4.3.5 / RUSTSEC-2026-0371 | Linux accessibility stack; upstream migration deferred |
| zvariant 5.15.1 / RUSTSEC-2026-0370 | Linux accessibility stack; upstream migration deferred |
| zvariant_derive 5.15.1 / RUSTSEC-2026-0372 | Linux accessibility build stack; upstream migration deferred |
| glib 0.18.5 / RUSTSEC-2024-0429 | No active inverse tree for Windows or configured Linux target; locked optional platform graph still carries the unsoundness notice |

No warning suppression or blind major upgrades are used. A fresh audit remains
required before publication; maintenance notices are not proof of active exploitation.

## Packaging and recovery

Build release artifacts explicitly with
`cargo build --release --locked --bin yttraydl`. Package only the application,
README, license, and intended portable marker; exclude field/probe/fixture binaries.
Verify installer and portable versions/hashes from the merged commit before publishing
`v0.7.1`. Keep the previous release assets available. If data loss, process cleanup,
or startup regressions are observed, hold publication or remove the affected asset
and restore the previous known release; preserve settings/history/archive and collect
bounded evidence before further migration. No automatic downgrade is claimed.

## Legacy uninstall-log migration

Fresh installs preserve foreign media after removal of recursive deletion.
Upgrades also need to replace the historical uninstall log: the old destructive
record otherwise survives Inno's default append behavior. This is a documented
exception to the usual recommendation against [log replacement](https://jrsoftware.org/ishelp/topic_setup_uninstalllogmode.htm).

The replacement covers the verified in-place 0.6.1 and 0.7.0 inventories: all
three application files, Start Menu shortcut, and exact legacy desktop shortcut
and startup value. Cleanup of optional resources is recorded even when candidate
tasks are deselected. Deselection does not immediately remove inherited resources;
uninstall does. No old uninstaller is executed and no private log format is patched.

Four disposable-VM cases are required: clean, 0.6.1 with tasks on, 0.6.1 on-to-off,
and 0.7.0 on-to-off. They preserve media seeded before upgrade, four data hashes,
and unrelated sibling shortcut/Run sentinels, while removing every known owned
file, shortcut, startup value and registration. Actual 0.6.1 legacy-log RED is
retained; new variants must pass against the corrected head. Relocated/mismatched
installation modes or unknown historical inventories are not qualified. Future
removed or renamed resources need exact legacy cleanup and migration regressions.
## Compact defaults

The fresh main window is 600×420 logical pixels, with unchanged controls and a
fixed footer. The body scrolls when notices or populated lists need more space.
Fresh empty Downloads uses 800×360; populated Downloads uses 800×560. The creation
hint is selected once per opening, so content changes never resize an open window.
Saved normal bounds always override these defaults. Settings retains its existing
content-sized default. Native layout qualification includes the narrow 340×420
main window, overflow reachability and persistence regression checks.
Video-list titles now use measured remaining width around their real controls,
instead of fixed 26/40/44-character cuts. Full title and URL remain available in
hover details. Metadata uses a bounded secondary line where needed. Existing
button order, disabled conditions, row identity and action targeting are retained;
headless egui regressions check growth, containment, traversal and Retry targeting.
At minimum widths, missing-engine actions wrap rather than clip outside the card.