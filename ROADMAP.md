# Roadmap

Status below is deliberately evidence-based. Hosted automated tests are not a
substitute for interactive testing on a physical Windows desktop.

## Milestone 1 — interactive and hardware verification

- [ ] Windows 10 and 11: exercise the player queue, repeat, previous/next,
  stop, and ffplay's own playback controls on interactive desktops.
- [ ] Verify the five module shortcuts and their optional desktop shortcuts
  launch the intended windows and forward intent to an existing instance.
- [ ] Verify physical Ctrl+Alt+G and `--show` recovery when the main window is
  hidden, including focus and tray behaviour.
- [ ] Inspect popup text and controls with Windows UI Automation and a screen
  reader; record the actual accessible names and results.
- [ ] Record real GPU and driver results in the
  [hardware matrix](docs/HARDWARE-ENCODING-MATRIX.md).

All items above are **unverified** on physical Windows hardware. Record OS build,
app version, environment, steps and outcomes when evidence becomes available.

## Milestone 2 — playback and trimming polish

- [x] Exercise stream-copy trimming with pinned real ffmpeg/ffprobe on hosted
  Windows, including batch copy, transform-driven re-encoding and a publication
  collision. Offline tests cover fallback and cancellation. Hosted CI
  [45542476148](https://github.com/SeanShaffer88/ytTrayDL/actions/runs/45542476148)
  passed for commit `4a5b9b8`.
- [ ] Verify keyframe limitations, method reporting and safe fallback in an
  interactive Windows session. Physical GPU encoding remains unverified.
- [x] Provide a direct ffplay download link and explain discovery and refusal.
- [x] Evaluate network URL playback: defer it. The current player is local-only;
  network playback would require an explicit opt-in, URL and redirect policy,
  protocol restrictions, privacy disclosure and cancellation/timeout design.
  No streaming implementation or background request is included in this decision.

## Milestone 3 — installer and release hardening

- [x] Exercise clean install, previous-stable upgrade, uninstall/reinstall,
  offline skip and fresh provisioning report in disposable hosted Windows CI
  [45547947249](https://github.com/SeanShaffer88/ytTrayDL/actions/runs/45547947249)
  for commit `e588cb5`. The interactive steps in the
  [installer checklist](docs/INSTALLER-VERIFICATION.md) remain open; repeat the
  automated scenarios for each release candidate.
- [x] Verify the release packaging preflight with sanitized real-media evidence
  and checksums: hosted [45542474505](https://github.com/SeanShaffer88/ytTrayDL/actions/runs/45542474505)
  passed for commit `4a5b9b8` (workflow dispatch, no publication).
- [x] Exercise the repair verifier against built packages with simulated remote
  metadata in hosted [45547968517](https://github.com/SeanShaffer88/ytTrayDL/actions/runs/45547968517)
  for commit `e588cb5`, plus offline positive and negative tests.
- [ ] Verify actual tagged publication and draft repair using a future release;
  the simulated verification did not publish or repair a real draft.

## Deferred / Out of Scope

- Mixed-DPI interactive validation belongs to a dedicated display-behaviour
  milestone; it is explicitly not a release blocker for this roadmap.
- Full network streaming, accounts, telemetry and new background network calls
  are out of scope. Preserve the single-binary Rust/egui architecture.
