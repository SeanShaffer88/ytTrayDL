# 0.7.1 validation patch — release candidate

This candidate adds real-tool field validation and disposable Windows installer
lifecycle jobs. Release publication is pending the complete gate, actual field
evidence, independent review, and a passing merged commit. The layout is unchanged.

## Verification matrix

| Gate | Candidate status | Evidence required |
|---|---|---|
| Real video, merge, audio, playlist, subtitles | Not run in CI | `real-media-field-evidence`, all assertions pass |
| Application retry, final path, completion hook | Not run in CI | Real-tool harness evidence; transport retry alone is insufficient |
| Pause/resume, cancel and shutdown during ffmpeg | Not run in CI | Observed owned ffmpeg child, bounded intervention and cleanup |
| Actual GUI hide/relaunch and startup behavior | Not run by installer jobs | Owned interactive fixture application evidence |
| Clean install / uninstall | Not run | `installer-clean-evidence` from a fresh hosted Windows VM |
| Actual 0.6.1 upgrade / uninstall | Not run | `installer-upgrade-evidence`, baseline checksum verified |
| Linux/Windows stable and MSRV 1.89 | Pending | All CI jobs green for candidate commit |
| Final release payload | Pending | Merged commit, version 0.7.1, checksums, no test executables |

Update statuses only from observed execution. Generating scripts, compiling a
binary, or verifying a startup registry string does not establish runtime coverage.
Loopback fixtures cover downloads and postprocessing, not external extractor discovery.

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

`cargo audit --no-fetch --json` reported zero known vulnerabilities against local
database commit `965bf9ad083b58e6340d65829d85217b18be9290` (2026-11-03).
This is a cached database scan, not a newly fetched advisory scan. Six notices
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
