# Changelog

## 0.2.8 — 2026-10-23

- **Single left-click on the tray icon now toggles the
  window**: click once to show it, click again to hide it.
  The toggle reads the window's real visibility each time,
  and quick double-clicks are debounced so they can't
  toggle twice.
- **Cleaner tray Exit.** The 0.2.7 exit path could panic
  inside the menu library while tearing down (the app still
  exited, but abruptly). Exit now goes through the app's
  normal graceful shutdown — cancelling downloads and
  saving state — with a forced fallback only if the UI
  loop never responds.

## 0.2.7 — 2026-10-23

- **Tray menu actions now execute where the click lands.**
  Field logs from 0.2.5/0.2.6 showed tray clicks being
  delivered reliably, but the app's UI loop could stop
  scheduling frames after the tray's context menu had been
  shown (an eframe repaint-scheduling quirk), so anything
  waiting for the UI loop waited forever. Exit, the
  Monitoring / Auto-download / Start-with-Windows toggles,
  Open download folder, and Hide tray icon now run directly
  in the tray event handler — no UI frame involved — and
  tray-made setting changes are written to the settings file,
  which the app now watches and reloads when it changes.
  The remaining menu items (Downloads, Settings, About,
  Download from clipboard) still go through the app, and
  choosing one now also raises the main window immediately.
  Note: hiding the icon from the tray menu now hides it
  right away; the explanatory confirmation dialog still
  guards the same action inside the main window.

## 0.2.6 — 2026-10-23

- **Tray restore hardened.** Clicking the tray icon now also
  restores the window at the OS level, immediately — it no
  longer waits for the app's next UI frame to process the
  request. The tray-event plumbing was also made self-healing
  (the event channel is replaced rather than set-once, and
  delivery failures are logged instead of swallowed), with
  counters in the diagnostic log so any remaining delivery
  problem can be pinpointed from the log alone.

## 0.2.5 — 2026-10-23

- **The window no longer changes height when Settings
  opens** (SEAN). The drawer still slides out and the window
  grows wider by the drawer's width — and shrinks back on
  close — but the height stays exactly where you left it.
  Drawer content scrolls, so any height works.

## 0.2.4 — 2026-10-23

- **32-bit (x86) builds added**: every release now ships a
  portable zip and an installer for 32-bit Windows alongside
  the 64-bit ones, and CI compiles both architectures.
- **Fixed: tray icon and tray menu clicks never reached the
  app.** Root cause found from a 0.2.3 diagnostic log: the app
  polled tray-icon's global event receivers, which delivered
  nothing in this eframe setup — the icon rendered and the
  menu opened, but no click (icon or menu, including Exit)
  ever arrived. The app now uses the integration the tray
  library documents for winit-based apps: registered event
  handlers that forward clicks into the app's own queue and
  wake the UI immediately. Side benefit: tray clicks now
  respond instantly instead of within the ~200 ms poll.

## 0.2.3 — 2026-10-20

- **Tray icon clicks behave the way Windows users expect**
  (SEAN): a single left-click opens the window (and focuses
  it); the menu appears on right-click only. Before, left-click
  both toggled the window *and* popped the menu.
- **Better field diagnostics for the exit reports.** The
  diagnostic log now records the app version at launch and every
  tray-menu click by name, so a click that "does nothing" can be
  traced: either it reached the app (and the log shows it) or it
  never arrived (stale instance / event problem). Note: if an
  older ytTrayDL is still running, launching a newer copy just
  wakes the old one — exit the old copy first (Task Manager if
  its Exit won't respond) before judging the new build.

## 0.2.2 — 2026-10-20

- **X button fixed (field report).** Clicking the main window's
  X exited the app instead of showing the close-choice dialog:
  the app reacted to the close request but never sent egui's
  `CancelClose` veto, and an unanswered close request closes the
  window anyway. (This bug was present since the dialog's
  introduction in 0.2.0.) Every non-exit close action now vetoes
  the close first, then acts — Ask shows the dialog, Tray /
  Taskbar / Hide do what they say, Exit still quits.
- **Settings button toggles.** The panel's ⚙ Settings button now
  slides the drawer out *and* back in, instead of only opening.

## 0.2.1 — 2026-10-20

- **Exit fixed (field report).** Tray-menu Exit appeared to do
  nothing — the quit relied on the window stack unwinding after a
  graceful close, and that handshake could stall invisibly. Exit
  is now deterministic: running downloads are cancelled (their
  yt-dlp children are killed), settings/history are saved, the
  exit is written to the diagnostic log, and the process ends.
  The close-choice logic also never intercepts a close while an
  exit is already in progress.
- **Installer build.** Releases now also ship
  `ytTrayDL-Setup-v0.2.1-windows-x64.exe` (Inno Setup): per-user
  install, Start-menu entry, optional desktop shortcut and
  start-with-Windows, standard uninstaller that keeps your
  `%APPDATA%\ytTrayDL` data. The portable zip remains available.
- **Settings slide out.** Settings are no longer a separate
  window: the main window grows and the settings drawer slides
  out from its right side (✕ or the Settings button slides it
  back). Layout inside is condensed too — tighter row spacing,
  and the Notifications and Advanced sections are collapsible
  (collapsed by default).

## 0.2.0 — 2026-10-20

### Added

- **Install ffmpeg from inside the app** (SEAN's request). When
  ffmpeg is missing, the control panel and Settings → yt-dlp offer
  "⬇ Install ffmpeg": it downloads the GyanD essentials build
  (the Windows build ffmpeg.org links to, ~110 MB) from gyan.dev
  using Windows' bundled curl.exe, unpacks it with the bundled
  tar.exe, and puts ffmpeg.exe + ffprobe.exe in
  `%APPDATA%\ytTrayDL\bin` next to the managed yt-dlp. No new
  dependencies were added. The managed ffmpeg is now also passed
  to yt-dlp explicitly (`--ffmpeg-location`) when no custom path
  is set, so an in-app install takes effect even when yt-dlp
  itself lives elsewhere.
- **Closing the main window asks what to do** (SEAN's request):
  Exit / Minimize to tray / Minimize to taskbar / Hide entirely
  (keeps running in the background; Ctrl+Alt+G, relaunching, or
  `--show` brings it back). A "Remember my choice" checkbox in
  the dialog — and the "When I close the main window" setting in
  Settings → General — controls it; the default is to ask.

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
