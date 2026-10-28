# ytTrayDL — design notes

## Purpose

Make yt-dlp feel like a built-in Windows feature: copy a link, get the
video. Two equally first-class flows:

- **Ask** — a small popup with the essentials (title, quality, download /
  not now / ignore). For people who pick quality case by case.
- **Silent** — zero interruption. Defaults + per-domain rules decide;
  toasts report outcomes (failures always, starts only if asked).

## Stack decision

- **Rust** (the project-family default), single static `.exe`, no runtime.
- **egui/eframe 0.33** for the popup/panel/settings — same UI stack as
  InstallScope, immediate-mode viewports map naturally to "small popup
  window + a few utility windows".
- **`tray-icon` 0.21** for the notification-area icon and its native
  context menu (the menu is the primary configuration surface, per the
  product brief).
- **`windows-sys` 0.59** for the Win32 pieces egui doesn't cover:
  clipboard listener window, single-instance mutex/event, hotkey,
  registry autostart, ShellExecute.
- Product-family look: the black/red Command Deck identity
  (near-black panels `#101010`, hairlines `#242424`, red accent
  `#E5484D` / deep red `#6B0700`, text `#F5F5F5`/`#A8A8A8`/`#6E6E6E`).

Alternatives considered: C#/WinForms (`NotifyIcon`) is the classic tray
stack and would be equally valid, but splits the toolchain from the rest
of the family for no user-visible gain; a WebView shell was rejected as
heavyweight for a tray utility.

## Clipboard monitoring

Per Microsoft Learn ("Using the Clipboard"), a **clipboard format
listener** is the recommended mechanism: `AddClipboardFormatListener`
on a window, `WM_CLIPBOARDUPDATE` when content changes. ytTrayDL runs a
message-only window (`HWND_MESSAGE`) on a dedicated thread, reads
`CF_UNICODETEXT` with a short retry (the clipboard is often momentarily
locked by the app that set it), suppresses repeats via
`GetClipboardSequenceNumber`, and forwards text to the UI over an mpsc
channel. No polling anywhere.

## Decision pipeline

clipboard text → `extract_urls` (hand parser; strips prose punctuation,
keeps balanced parentheses, dedupes within the text) → `SeenCache`
(configurable window, default 5 min, keyed on the normalized URL) →
`rules::decide`:

1. exact URL on the ignore list → Ignore
2. host matches an ignored domain (host or any subdomain) → Ignore
3. already-downloaded (only when known from an info probe) → Ignore
4. out of detection scope → Ignore
5. host matches an auto-download domain → AutoDownload
6. global default action → Prompt / AutoDownload

Ignored detections are listed in the panel ("Ignored recently") so the
behaviour is transparent and debuggable, not magic.

### Already-downloaded, honestly

An archive hit can only be *known* after an info probe (the archive
stores `extractor id` pairs). Two layers handle it:

- **Download time (authoritative):** every download passes
  `--download-archive` (when enabled), so yt-dlp itself skips repeats
  and reports "has already been downloaded" — surfaced in the UI, no
  bytes re-fetched.
- **Popup time (advisory):** the popup's background info probe checks
  the archive; on a hit the popup labels the item and its button
  becomes "Download anyway", which runs that one job without the
  archive so the user can force a fresh copy.

## The hidden tray icon problem

Hiding the icon is a requested feature and a classic footgun. ytTrayDL's
rules:

- Hiding always goes through a confirmation that lists the recovery
  paths *before* the icon disappears (tray menu, panel button, and the
  Settings checkbox all route there).
- Recovery is triple-redundant:
  1. **Relaunch** — a named mutex (`Local\ytTrayDL_SingleInstance_v1`)
     detects the second launch; a named event
     (`Local\ytTrayDL_ShowYourself_v1`) tells the running instance to
     restore its icon and open Settings; the second process exits.
  2. **Ctrl+Alt+G** hotkey registered on the listener window.
  3. **`yttraydl.exe --show`** — same named-event path.
- `tray_icon_visible` is persisted, so a restart respects the choice —
  and the setting is a plain JSON field the user can flip by hand in
  the worst case.

## yt-dlp integration

- **Locating:** user path → beside the exe → managed copy
  (`%APPDATA%\ytTrayDL\bin\yt-dlp.exe`) → PATH. The found source and
  version are always visible in the panel/Settings.
- **Installing/updating the managed copy:** downloaded from the official
  GitHub release URL using Windows' bundled `curl.exe` (Windows 10
  1803+), written to a temp file and renamed into place. No downloader
  code of our own to get wrong.
- **Downloads:** a job thread per download running yt-dlp with
  `--newline --no-colors --continue --no-mtime`, the preset's `-f`
  selector, `--merge-output-format` for video, `-o <folder>\<template>`,
  `--download-archive`, playlist/embed/cookies options per settings, and
  the user's extra args last (so they override). stdout+stderr lines are
  merged and parsed for `Destination:`, `% of … at … ETA …`, merger /
  extractor lines and `ERROR` lines. Cancel kills the child; the `.part`
  file remains and `--continue` resumes it later. Progress % and ETA —
  a standing family requirement — come from yt-dlp's own reporting.
- **Info probe:** `--dump-single-json` on a worker thread; the popup
  opens instantly and fills in. Failure degrades to "download anyway?"
  with the bare URL — never a blocked popup.
- **ffmpeg** is detected (user path / beside exe / managed bin / PATH)
  and its absence is surfaced in the panel, because merging and MP3
  conversion silently degrade without it.

## Data & safety

- `settings.json` — schema-versioned, every field serde-defaulted so
  older files keep loading; saves are write-temp-then-rename; a corrupt
  file is renamed to `settings.json.bad` and defaults are used (the app
  always starts).
- `history.json` — newest 200 results; same atomic save.
- `download-archive.txt` — yt-dlp's own format, owned by yt-dlp.
- No elevation anywhere; autostart is the per-user `Run` key.
- URLs are passed to yt-dlp as a single argv element after `--`, never
  through a shell, so clipboard content can't become shell syntax.

## Known limitations (current)

- Popup position on multi-monitor setups uses the primary monitor's
  size; per-monitor placement is a follow-up.
- History timestamps render as UTC time-of-day in the Downloads window
  (no datetime dependency); full unix timestamps are in `history.json`.
- The tray icon is created inside egui's event loop (the documented
  pattern for `tray-icon` + winit); if a future eframe version changes
  its threading, tray creation may need to move to a `resumed` hook.
- Playlist info probing is flat; per-item selection UI is future work.
- yt-dlp/ffmpeg *runtime* behaviour depends on the user's copies; only
  ytTrayDL's own argument construction is verified here.

## Queue management (0.2.9)

The Downloads window is the management surface for the
manager's three lists — queue, paused, finished:

- **Reorder**: queued jobs swap neighbours (the pump takes
  from the front, so order is download order).
- **Pause/resume**: pausing a running job kills its yt-dlp
  child and files the job as Paused — not finished, so it
  never lands in history as a failure. The `.part` file is
  kept, and because every download runs with `--continue`,
  a resume picks up where it stopped. Paused jobs hold no
  process and survive until resumed or cancelled.
- **Retry**: a finished Failed/Cancelled job requeues with
  its original preset and archive choice. **Auto-retry**
  (a setting, default off) does the same automatically for
  failures, carrying an attempt count on the job; a manual
  cancel is never auto-retried, and an intermediate failure
  that will be retried is not written to history.
- **Housekeeping**: finished entries can be removed one by
  one or cleared wholesale (files always stay), and Cancel
  all moves queued/paused jobs to finished-as-Cancelled
  while flagging running jobs — the same path app exit uses.

Popup placement gained Top-left, Bottom-left, and Center
(primary monitor, like the existing corners) alongside
Bottom-right, Top-right, and Near-cursor.

## 0.3.0 additions

- **Quality by domain**: `Settings::domain_presets` (domain +
  preset pairs, normalized/sorted/deduped like the other rule
  lists). `rules::preset_for_url` resolves the pin for a URL;
  it seeds the popup's quality combo and replaces the global
  default for auto-downloads. The popup choice stays per-
  download — the pin is a starting point, not a lock.
- **Per-download playlist choice**: `Job::playlist_all`
  (`Option<bool>`) travels from the popup through
  `enqueue_full` into `build_download_args_with_playlist`;
  `None` follows the global `allow_playlists`. Retries
  (manual and automatic) carry the choice forward. The popup
  grows an extra row only when the probe identifies a
  playlist.
- **SponsorBlock**: a settings flag adds
  `--sponsorblock-remove all` to the argument vector.
- **Taskbar progress**: `platform::set_taskbar_progress`
  drives `ITaskbarList3`. windows-sys 0.59 ships the CLSID
  and TBPFLAG constants but not the interface, so the three
  vtbl slots used (HrInit, SetProgressValue,
  SetProgressState) are called by index from
  `windows_impl.rs`, with the object cached in a
  thread-local (COM apartment rules). The app sends the mean
  completion across active jobs, and only on change.
- **Finish sound**: `platform::play_notification_sound`
  (`MessageBeep(MB_ICONASTERISK)`), fired from the same
  completion branch as the toast.
- **Drag-and-drop**: files dropped on the panel are read as
  text (≤ 1 MiB), URLs extracted and routed through the
  standard dedupe + rules pipeline — a drop behaves like a
  series of copies.
- **Settings export/import**: export writes the live
  settings as pretty JSON; import parses via
  `Settings::from_json` (same normalization as load),
  replaces the live settings, re-applies autostart, and
  re-localizes the tool binaries.
- **Update check**: `fetch_latest_ytdlp_tag` reads the
  GitHub releases API with the bundled curl (the same
  transport as the managed downloads) and the yt-dlp tab
  compares it with the running copy's version. It runs
  only from the button — no background phoning home.

## 0.4.0 additions

- **Channel watcher**: `Settings::subscriptions` (URL +
  preset). A worker thread per poll runs
  `fetch_playlist_entries` (`--flat-playlist --dump-json
  --playlist-end 10`) per subscription; `saved::WatchData`
  (`watch.json`) records the entry ids each URL has shown.
  First poll of a URL baselines only. New ids enqueue via
  the normal download path (archive dedupe still applies).
- **Subtitles-only preset**: no `-f` at all; the arg vector
  carries `--skip-download --write-subs --write-auto-subs
  --sub-langs <setting> --convert-subs srt`. Merge-format,
  embed-subs and split-chapters are suppressed for it.
- **Audio convert presets**: MP3/M4A/Opus/FLAC/WAV share one
  arg shape (`-x --audio-format <fmt> --audio-quality 0`)
  via `Preset::audio_format`.
- **Popup upgrades**: `VideoInfo` gained
  height/format_note/filesize(_approx); the popup meta line
  shows resolution + size. The thumbnail URL is fetched
  with the bundled curl (`fetch_url_bytes`, 3 MB cap) and
  decoded with the `image` crate (JPEG/PNG only) on a
  worker thread, then uploaded as an egui texture.
  `PopupRequest::folder` carries a per-download output
  folder into `Job::output_dir` / `build_download_args_full`.
  "Later" parks the link via `saved.rs` (`saved.json`).
- **Failure banner**: final failures (after auto-retry)
  set `App::failure_notice`; Retry requeues the finished
  job (original preset + folder preserved).
- **Queue-empty action**: fires once when a queue that had
  active work drains completely (nothing queued/running/
  paused): sound, `SetSuspendState`, or graceful exit.
- **Low-disk guard**: `platform::free_disk_bytes`
  (`GetDiskFreeSpaceExW`) gates `pump_downloads` — queued
  jobs wait under the threshold; running jobs finish.
- **Post-download command**: on success, the user's command
  line runs through the shell with `{file}`/`{dir}`/
  `{url}`/`{title}` substituted.
- **Portable mode**: `settings::config_dir` checks once
  for `portable.txt` next to the exe; when present, all
  data resolves to `<exe dir>/ytTrayDL-data`. Every stored
  file already funnels through `config_dir()`.
- **Settings animation toggle**: `animate_settings_drawer`
  false snaps the drawer animation value to its target;
  the window resize runs once at the final width.
- **Batch paste** needed no change: the clipboard handler
  already routes every extracted URL (verified by reading
  `process_platform_events`); popups queue in order.

## 0.4.1 fix

- **Settings closes before the window hides.** Field
  report on 0.4.0: closing the window to the tray with the
  Settings drawer open brought back a blank window on
  restore — the close paths reset the drawer state and hid
  the window in the same frame, so the OS window kept its
  drawer-open width while hidden and the shrink-back resize
  never ran. All panel-hiding paths (X with CloseAction
  Tray, the close-choice dialog's Tray choice, the panel's
  ✕ Close button, and `hide_entirely`) now route through
  `App::begin_panel_hide`: the drawer closes like a normal
  Settings close (slide or snap per the animation setting,
  window shrinks to panel width) and `pending_panel_hide`
  hides the panel only once `settings_anim` reaches 0.
  Reopening Settings mid-close cancels the hide.

## 0.5.0 polish pass

Visual only — no behaviour, settings, or argument changes.
`apply_theme` (src/gui.rs) now sets the full egui style
instead of only swapping colours onto egui's defaults:

- **Tokens:** corner radius 8 (windows 10), card fill
  `#1A1A1A` on the `#101010` panel, control fills
  `#1C1C1C` / hover `#282828`, hairlines `#242424`, hover
  outline `#3A3A3A`, selection deep red `#6B0700`, accent
  `#E5484D`. Shared helpers: `card_frame`, `section_card`,
  `section_heading`, `primary_button` (bright accent fill),
  `status_pill`.
- **Type/spacing:** Heading 20 / Body 14 / Button 14 /
  Small 12 / Monospace 13; spacing on a 4/8 grid
  (item spacing 8×6, button padding 10×5, window margin 12,
  interact height 24). Sliders, combo boxes, checkboxes,
  radio buttons, scrollbars and progress bars all read
  their look from these visuals/spacing values; sliders
  show a filled (selection-colour) trail.
- **Panel:** status pill, yt-dlp status in a card, the
  manual-link field stretches to the row width, Recent
  titles truncate with an ellipsis (full title on hover),
  and the footer is pinned to the window bottom by a
  flexible spacer above a hairline.
- **Settings:** sections are cards with headings, tabs are
  selectable pill buttons, slider rows use a fixed 190px
  label column so sliders and value boxes align; slider
  suffixes read "30 s" style. The drawer's old 6×3 row
  spacing override now matches the central grid.
- **Popup / Downloads / toasts / dialogs:** popup title at
  16px, thumbnail with rounded corners (`Image`
  corner-radius), aligned Quality / Save-to label columns,
  bright primary Download; Downloads sections are cards;
  toasts are per-toast cards with a coloured accent bar on
  a transparent viewport panel; About and both dialogs use
  the same cards and primary/secondary buttons.
- **Known egui limits:** one corner radius per widget state
  (no separate small-control radius, so small buttons share
  the 8px radius), and no per-widget shadow/blur styling —
  polish is limited to what egui 0.33's style system can
  express.

## 0.5.1 fixes

Field-driven (eleventh field report, on v0.5.0):

- **Settings is its own window again.** The slide-out drawer
  (0.2.1–0.5.0) is removed: `settings_ui` renders a separate
  viewport (600×640 default, 480×420 minimum), built like
  Downloads/About and reusing the same tab content functions
  and 0.5.0 styling. The drawer animation, the window
  grow/shrink resize, and the 0.4.1 deferred panel hide are
  gone with it; `begin_panel_hide` is direct again and also
  closes the Settings window so nothing floats alone while
  the app sits in the tray. The `animate_settings_drawer`
  settings field remains in the file format, unused, and
  its Settings row is removed.
- **Panel width budgets.** The status pill truncates its
  suffix inside the pill (full line on hover); the
  manual-link row reserves its Download button first and
  subtracts the text field's frame margin (egui's TextEdit
  outer width is desired_width + margin — the 0.5.0 budget
  missed it, and treated the button's min_size as its
  width); the Settings/Downloads/Download-folder buttons
  and the footer buttons get explicit sizes with truncating
  labels; Recent rows budget the title before the folder
  button instead of placing the button with a
  right-to-left sub-layout first (which claimed the whole
  row, starved the title, and landed the button on the
  panel's extreme edge). The panel also clips at its own
  content rect, so any residual overflow is cut at the
  panel edge. The budgets hold at the 400px default and
  the 340px minimum width.
- **Painted status dots.** The ● character (U+25CF) is
  not in egui's default font and rendered as a hollow
  square; status dots (panel pill, Recent rows, Downloads
  history) are now painted filled circles (`status_dot`).
- **Tray toggles no longer mutate muda in-dispatch.** The
  tray handlers keep their settings-file/registry work but
  no longer call `set_checked` inside muda's menu dispatch
  (the RefCell-borrowed panic class proven in the eighth
  report; prime suspect — not yet proven — for the tray
  "Start with Windows" untick exiting the program).
  `sync_tray`, in app context, owns all three checkmarks:
  monitoring and auto-download follow the live settings,
  autostart is read back from the registry and pushed only
  on change. The autostart handler branch logs each step
  so a recurrence is pinpointed by the log. The hide-icon
  branch keeps its in-handler `set_visible` — it mutates
  the tray icon itself, not a menu item, and has worked in
  the field since 0.2.7; a judgment call, recorded here.

## Verification matrix

| Area | State | Evidence |
| --- | --- | --- |
| URL extraction / normalization | Verified | Unit tests in `url_detect.rs` |
| Domain rules + decision engine | Verified | Unit tests in `rules.rs` / `settings.rs` |
| Dedupe window | Verified | Unit tests in `dedupe.rs` |
| yt-dlp argument building | Verified | Unit tests in `ytdlp.rs` (arg vectors per preset) |
| Progress line parsing | Verified | Unit tests against real yt-dlp line shapes |
| Info JSON parsing | Verified | Unit tests in `ytdlp.rs` |
| Download manager queueing/cancel logic | Verified (logic) | Unit tests; process plumbing partially verified |
| Queue management + popup positions (0.2.9) | Logic verified; UI **Unverified** on-machine | Manager unit tests cover pause/resume/reorder/retry/remove/clear/cancel-all and the paused-event routing; `--limit-rate` covered by arg-building tests. The Downloads-window controls, tray Pause-all item, and the three new popup positions need a real-machine pass |
| 0.3.0 feature set | Logic verified; UI **Unverified** on-machine | Arg tests cover SponsorBlock + playlist override; rules tests cover domain-preset resolution. The popup playlist row, taskbar progress bar, finish sound, drag-and-drop, settings import/export, and the update check need a real-machine pass |
| 0.4.0 feature set | Logic verified; UI **Partially verified** on-machine (rendering) | Arg tests cover subtitles-only, audio-convert formats, split-chapters and the output-dir override; unit tests cover playlist-entry parsing and the watcher's baseline/new-entry diffing. SEAN's 2026-10-23 captures of a real 0.4.0 run show the panel and all four Settings tabs rendering with the new controls present (animation toggle + queue-empty action; split chapters + disk guard + post-download command; channel watcher editor; quality-by-domain) and the hide-icon dialog. The behaviours themselves — watcher end-to-end, popup thumbnail, per-download folder, saved list, failure banner, queue-empty actions, disk guard, post-download command, portable mode — still need a real-machine pass |
| Ninth field report (2026-10-23) | Blank window on restore from tray (0.4.0) | SEAN ran 0.4.0 (captures above; a completed download sits in Recent, ffmpeg found, managed yt-dlp 2026.08.19). Closing the window to the tray with the Settings drawer open, then restoring, showed a blank window. Cause: the close paths reset the drawer state and hid the window in the same frame — the OS window kept its drawer-open width while hidden and the shrink-back resize never ran. 0.4.1 routes every panel hide through `begin_panel_hide` (drawer closes fully first, then the hide lands) |
| Tenth field report (2026-10-23) | v0.4.1 fix **Verified** | SEAN ran 0.4.1 and confirmed "all good" — restoring from the tray after closing with Settings open now returns a working window; the settings-close-first fix is field-verified |
| Eleventh field report (2026-10-27) | Panel right-edge clipping + tray autostart exit on v0.5.0 | SEAN's captures of the real v0.5.0 install: at launch the status pill and the manual-link Download button run off the panel's right edge and the footer Exit button truncates; after a resize Exit still clips and a Recent row's folder button sits half off the edge; with the Settings drawer open, panel content paints under the drawer. The status dots also render as hollow squares (the ● character, U+25CF, is missing from egui's default font). Separately: unticking "Start with Windows" in the tray menu exits the program — prime suspect is the handler calling set_checked on the muda item inside muda's own dispatch (the RefCell-borrowed panic class proven in the eighth report); mechanism not yet proven, so the 0.5.1 autostart branch logs every step (current → target, registry done, settings saved) to pinpoint it if it recurs. 0.5.1: Settings is its own window (drawer machinery removed), panel rows are width-budgeted with a hard clip at the panel rect, dots are painted circles, and tray checkmarks are owned by sync_tray in app context. Fix **Unverified** until SEAN's next run |
| 0.5.0 polish pass (theme, panel, settings, popup, downloads, toasts, dialogs) | **Partially verified** on-machine (rendering) | Theme/layout code compiles and passes the full gate on the build VM; egui style values were checked against the vendored egui 0.33.3 source. SEAN's 2026-10-27 captures of a real v0.5.0 install show the new look rendering (panel, cards, pill tabs, primary buttons) — and found the right-edge clipping defect fixed in 0.5.1 (eleventh field report) |
| Whole-crate compile, Windows target | Partially verified | `cargo check/clippy --target x86_64-pc-windows-msvc` + Windows CI |
| Eighth field report (2026-10-23) | "Everything works fine" on 0.2.7, with full log | FIELD-CONFIRMED: sends=drained (29/29, 38/38), heartbeats continue across tray interactions, left-click OS restore fires (`show_main_window -> true`), handler-direct Exit executes. Two findings in the same log: (1) the exit path PANICKED — `muda windows mod.rs:122: RefCell already borrowed`, then "panic in a function that cannot unwind" — because the handler dropped the tray handles (muda objects) inside muda's own menu dispatch, which holds that RefCell; the process aborted mid-drop, which still read as "exited" to the user. (2) Menu-ID correction: the handler log proves MenuId 1011 = **Exit** (the 0.2.7 handler names it from the stored ids) — the sixth/seventh reports' arithmetic inference (1011 = Hide icon) was wrong; in 0.2.5 SEAN was clicking Exit ×3 + one other item. Code never relied on the arithmetic (ids are captured from the item objects), only the report narrative did. 0.2.8: Exit routes through the app's graceful shutdown (event forwarded) + a 750 ms forced-exit fallback thread; nothing is dropped inside the dispatch. SEAN-requested in the same report: single left-click toggles the window — `platform::toggle_main_window` (reads real OS visibility; SW_HIDE/SW_SHOW+foreground), handler sends `TrayUiEvent::WindowToggled(visible)` which the app adopts as `show_panel`; 350 ms debounce because Windows delivers a second Up for double-clicks (visible in this very log), which would toggle twice. |
| In-app ffmpeg install (0.2.0) | **Unverified** on-machine | Download/unpack logic mirrors the yt-dlp installer (curl.exe + tar.exe, both in-box); the archive-layout finder `find_ffmpeg_exes` and the managed `--ffmpeg-location` wiring are unit-tested. Needs a real click-through. |
| Close-window choice dialog (0.2.0) | **Unverified** on-machine | CloseAction setting: default/serde/legacy-load unit-tested; the four outcomes reuse proven paths (exit flow, tray hide, hide_entirely, ViewportCommand::Minimized). Needs a real click-through of each choice. |
| Seventh field report (2026-10-23) | Tray menu still not working on 0.2.6, with full log | The 0.2.6 instrumentation pays off: `app created` ×1 (no recreation), `channel replaced: false` (fresh channel), hb at +5 s shows frames=713 (update running at ~140 fps, panel visible) — then handler lines for two menu picks (later handler-proven to be Exit picks — see the eighth report's MenuId correction) and NOTHING further: no consumption lines, no later heartbeats, while handlers keep firing. Reading eframe 0.33.3's vendored runner explains the wedge: repaint scheduling (`request_repaint_after` delay dedup, `UserEvent::RequestRepaint` cumulative-pass staleness check, `RepaintAt` bookkeeping) is left permanently disarmed by the tray menu's nested TrackPopupMenu message loop on the winit thread — winit keeps dispatching window messages (tray callbacks fire, handlers log) but no egui pass is ever scheduled again, so events queued for `update()` wait forever. 0.2.7 removes egui from the critical path: tray handles live in a thread-local cell (tray-icon/muda handles are `Rc`-based and thread-affine — a shared `Mutex` cell does not compile); the menu handler EXECUTES exit, the three toggles and hide-icon (settings-file writes + checkmark/visibility under the same access), and open-folder directly; only downloads/settings/about/download-from-clipboard are forwarded (and now also raise the window at OS level). App reloads settings when the file's mtime changes so handler writes are adopted, not overwritten. (Resolved by the eighth report.) |
| Installer (Inno Setup, 0.2.1) | Build verified in CI; install behavior **unverified** | `installer/yttraydl.iss` — per-user, no admin, Start-menu entry, optional desktop/startup tasks, uninstaller keeps `%APPDATA%\ytTrayDL`. |
| Sixth field report (2026-10-23) | Tray menu still doesn't work; tray click doesn't restore window (0.2.5 + screenshots + full log) | The 0.2.5 log splits the bug precisely: handlers FIRE for every icon/menu click (production leg fixed by 0.2.4), but ZERO consumption lines — events die between handler and `process_tray_events`. Two live suspects: (a) the `OnceLock` sender strands sends on a dead channel if the app object is ever recreated (send errors were swallowed by `let _ =`); (b) `update()` stops running while the root viewport is hidden, so nobody drains. 0.2.6 attacks both: sender slot is replaceable (`Mutex`, overwritten at install) + send results/count failures logged; update heartbeat (frames/sends/errs/drained/panel every 5 s) + "app created" log to prove (b) or a recreation; and the left-click handler restores the window at OS level (`platform::show_main_window`, EnumWindows by PID+title) independent of egui scheduling. SEAN's screenshots also confirm: panel, Settings drawer (width-only expansion working), and the hide-icon dialog all render correctly on 0.2.5. (Answered by the seventh report.) |
| Fifth field report (2026-10-23) | Window changes height when Settings opens | By design of the 0.2.1 drawer animation (height grew toward 580 px while widening) — SEAN's call: expansion should be width-only. 0.2.5 pins the height to the window's live height during the animation; the drawer content already scrolls, so no minimum-height need remains. Same release train: SEAN also asked for a 32-bit version — i686 check/clippy joined the gate, CI builds both architectures (x86 exe verified PE32), and v0.2.4 gained x86 installer + portable assets. SEAN also confirmed detection scope stays Known-sites-only by default (69 built-in domains + user allow list). Drawer behaviour confirmed via SEAN's 0.2.5 screenshots (sixth report); dual-arch assets ship since 0.2.4. |
| Fourth field report (2026-10-20) | "Exit doesn't work"; tray click behavior wrong | Tray clicks were genuinely wrong and are fixed in 0.2.3: menu popped on left-click (tray-icon default) while the same click toggled the window; now left-click opens+focuses, menu is right-click only (`with_menu_on_left_click(false)`). The Exit half was then NOT the stale-instance suspect: **closed by SEAN's 0.2.3 log (2026-10-23)** — a clean `launch: v0.2.3` with `already_running=false`, tray created, and then *zero* tray-menu click lines despite a right-click → Exit. Delivery itself was dead: polling `TrayIconEvent::receiver()` / `MenuEvent::receiver()` yielded nothing under eframe, in every version — the icon rendered and the menu displayed (both native-side), so the app looked alive while deaf. (The 0.2.1 "tray Exit" report was this same bug, misattributed twice.) **0.2.4 fix:** the integration tray-icon documents for winit apps — `TrayIconEvent::set_event_handler` + `MenuEvent::set_event_handler` forwarding into an app-owned channel, installed before the icon is built, waking the UI via `Context::request_repaint`, and logging at production (handler) and consumption (named click) so any residual failure localizes from the log alone. Global receivers remain drained as a documented no-op fallback. Delivery confirmed by the 0.2.5 log (handlers fire for every click — sixth report); the consumption half of the fix landed in 0.2.7 and was field-confirmed in the eighth report. |
| Third field report (2026-10-20) | X button exited instead of prompting | Root cause found in egui's close protocol: an unanswered `close_requested` closes the viewport — the app must answer with `ViewportCommand::CancelClose` to hold the window open. The 0.2.0 close-choice code reacted but never vetoed, so the dialog design never actually worked on a real machine (SEAN's click-through is what caught it). 0.2.2 vetoes for Ask/Tray/Taskbar/Hide; Exit needs no veto. Also in 0.2.2: the panel Settings button toggles the drawer. Pending SEAN's confirmation. |
| Second field report (2026-10-20) | Tray Exit appeared to do nothing | The 0.2.0 quit path saved state, sent `ViewportCommand::Close` to the root, and relied on the window stack unwinding to end the process — on SEAN's machine it did not end. 0.2.1 makes exit deterministic (cancel downloads → save → diag-log → `process::exit(0)`), logs tray Exit requests, and the close-choice logic never intercepts a close while exit is in progress. (The Exit clicks themselves were victims of the fourth report's dead delivery; final confirmation in the eighth report.) |
| First field report (2026-10-20) | Root cause CONFIRMED via the 0.1.1 diagnostic log | v0.1.0/0.1.1 exe "did nothing" on a real PC. The log captured a panic in wgpu 27.0.1 instance creation: "No wgpu backend feature that is implemented for the target platform was enabled" — the build enabled eframe's wgpu feature without any backend (DX12/Vulkan). The hidden-by-design first launch was a real design flaw but NOT the cause of this crash. Fixed in 0.1.2: glow (OpenGL) renderer via Cargo feature + pinned `Renderer::Glow`, plus catch_unwind around startup so any startup panic shows the error dialog. Awaiting re-test of 0.1.2 on the same machine. |
| Settings slide-out drawer + condensed layout (0.2.1) | Code-reviewed; visual check pending | Settings moved from a separate window into a right-side drawer in the main window: opening animates the window wider over ~220 ms (ease-out), content is fixed-width and clipped mid-slide, closing reverses it; window size while closed is tracked so the animation base follows user resizes. Drawer content: tighter row spacing, Notifications + Advanced collapsible (collapsed by default). All open paths (panel button, tray menu, relaunch/--show) show the panel first; tray/hide close actions snap the drawer shut with the window. |
| Clipboard listener on a real PC | Partially verified | SEAN's captures (2026-10-23) show the detection popup on his machine mid-countdown with probe-filled title/uploader/duration for a copied YouTube link; no log examined |
| Tray menu actions | Verified | 0.2.7 field log (eighth report): every action executes; sends=drained |
| Hide icon + relaunch recovery | Partially verified | Hide confirmation dialog captured from a real run (0.2.5); since 0.2.7 the handler hides the icon directly. The full hide → relaunch/Ctrl+Alt+G/`--show` recovery cycle has not been reported end-to-end |
| Popup placement / DPI behaviour | **Unverified** | Needs an on-machine run |
| End-to-end download of a real video | Partially verified | SEAN's panel capture (2026-10-23) shows the same video from the popup capture listed under Recent, with "ffmpeg found" and the managed yt-dlp copy reported in the panel; no log examined |

## Roadmap ideas (post-v0.1)

- Per-monitor popup placement + remember last position.
- Playlist item picker in the popup.
- ~~Thumbnail image in the popup~~ — shipped in 0.4.0 (bundled curl
  for bytes + the `image` crate for decode).
- Drag-and-drop a URL/text file onto the panel to queue downloads.
- Optional global hotkey "download clipboard now" (bypasses the popup).
- ~~Signed release + GitHub Releases publishing with checksums~~ —
  publishing + SHA256SUMS done (since v0.2.1); signing intentionally
  not pursued (owner's rule: no signing unless a platform requires
  it).
