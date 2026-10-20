# Changelog

## 0.1.2 — 2026-10-20

### Fixed

- **The actual startup crash.** The 0.1.1 diagnostic log captured
  it on the reporter's machine: the app panicked inside wgpu —
  "No wgpu backend feature that is implemented for the target
  platform was enabled." The build had enabled eframe's `wgpu`
  renderer (with `default-features = false`) without any wgpu
  backend feature, so window creation panicked before anything
  could appear — in both 0.1.0 and 0.1.1. This, not a hidden
  tray icon, is why double-clicking "did nothing".
- Switched the renderer to **glow (OpenGL)**: eframe feature
  changed in Cargo.toml and `Renderer::Glow` pinned in
  NativeOptions. Glow runs on effectively every Windows 10/11
  machine and is ample for this UI.
- Startup is now wrapped in `catch_unwind`: any future startup
  panic shows the error dialog and lands in the log, instead of
  exiting silently.

## 0.1.1 — 2026-10-20

### Fixed

- **First launch looked like nothing happened.** A manual launch
  (double-click, Start menu) now opens the control panel so the
  app is visibly alive. Only Start-with-Windows launches go
  straight to the tray (the Run entry now passes `--background`).
  Root cause of the field report: v0.1.0 started fully hidden and
  only auto-opened the panel when yt-dlp was missing; with yt-dlp
  present the only trace was a tray icon that Windows 11 tucks
  into the hidden-icons overflow by default.
- If tray-icon creation fails, the control panel opens instead of
  leaving an invisible process.

### Added

- Local diagnostic log at `%APPDATA%\ytTrayDL\yttraydl.log`:
  launch milestones, yt-dlp/ffmpeg detection, tray creation, and
  panics are recorded (local only, never sent anywhere).
- A fatal startup failure now shows an error dialog naming the
  log file, instead of exiting silently.

## 0.1.0 — 2026-10-20

> Renamed before first publication: the working title
> "ClipGrab" already belongs to a long-running video downloader,
> and SEAN settled on the final name **ytTrayDL**. Nothing was ever
> published under either earlier name.



First version.

- System-tray app (Rust + egui) with a native right-click tray menu:
  monitoring pause/resume, auto-download toggle, Download from
  clipboard, Downloads, download folder, Settings, start-with-Windows,
  hide tray icon, About, Exit. Left-click opens the control panel.
- Clipboard monitoring via the Win32 clipboard format listener API
  (`AddClipboardFormatListener` / `WM_CLIPBOARDUPDATE`) on a
  message-only window — event-driven, no polling.
- URL extraction from arbitrary clipboard text, normalized-URL dedupe
  (configurable window), and a pure-logic decision engine:
  ignored URLs → ignored domains → already-downloaded → detection
  scope → auto-download domains → default action.
- Download popup (bottom-right / top-right / near cursor): title,
  uploader, duration and thumbnail-free metadata from a yt-dlp
  `--dump-single-json` probe, quality preset picker, per-popup
  "always auto-download from this domain", Ignore menu (this link /
  this domain), "already downloaded" state with Download-anyway,
  auto-close timer that never downloads by itself.
- Silent auto-download mode with toast notifications (starts optional,
  completions and failures per settings).
- yt-dlp engine: locating order (user path → beside exe → managed →
  PATH), one-click install/update of the official standalone build into
  `%APPDATA%\ytTrayDL\bin` using Windows' bundled curl, ffmpeg presence
  detection, per-preset argument building, progress parsing with %,
  speed and ETA, queue with configurable concurrency, cancel with
  resume-later (`.part` + `--continue`), download archive support.
- Settings window with four sections (General, Downloads, Domains &
  rules, yt-dlp) — every control tooltip-explained; settings are
  schema-versioned JSON with atomic saves and corrupt-file recovery.
- Hide-tray-icon with triple recovery: relaunch (single-instance mutex
  + named show event), Ctrl+Alt+G hotkey, `--show` flag; the hide
  confirmation explains all three before the icon disappears.
- Downloads window with active jobs (progress/ETA/cancel), session
  results (open/retry), and persisted history (200 entries).
- Start-with-Windows via the per-user Run key; no elevation anywhere.
- Core library unit-tested (URL/rules/dedupe/args/progress/info);
  Windows-target compile + clippy gates in CI.

Known limitations are listed in `docs/DESIGN.md`.
