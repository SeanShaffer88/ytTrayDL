# Changelog

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
