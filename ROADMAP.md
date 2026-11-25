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

- [ ] Implement and verify safe stream-copy trimming where compatible, with
  explicit keyframe limitations, actual method reporting and safe fallback.
  Preserve profile, transforms and explicit encoder choices. Until implemented,
  trims continue to re-encode.
- [x] Provide a direct ffplay download link and explain discovery and refusal.
- [x] Evaluate network URL playback: defer it. The current player is local-only;
  network playback would require an explicit opt-in, URL and redirect policy,
  protocol restrictions, privacy disclosure and cancellation/timeout design.
  No streaming implementation or background request is included in this decision.

## Milestone 3 — installer and release hardening

- [ ] Complete the [installer verification checklist](docs/INSTALLER-VERIFICATION.md)
  on disposable Windows systems and record the report for the exact candidate.
- [ ] Validate the new release real-media evidence attachment and repair flow
  on hosted Windows; it packages a sanitized scenario summary with checksums.
  The workflow changes are not yet verified on a release run.

## Deferred / Out of Scope

- Mixed-DPI interactive validation belongs to a dedicated display-behaviour
  milestone; it is explicitly not a release blocker for this roadmap.
- Full network streaming, accounts, telemetry and new background network calls
  are out of scope. Preserve the single-binary Rust/egui architecture.
