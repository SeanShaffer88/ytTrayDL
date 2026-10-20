# ytTrayDL

**Copy a video link. It downloads.** ytTrayDL is a small Windows system-tray
companion for [yt-dlp](https://github.com/yt-dlp/yt-dlp). It watches your
clipboard, and when you copy a link from a site yt-dlp supports, it either
pops up a small window with download options — or, if you've told it to,
downloads quietly in the background with your defaults while you keep working.

![ytTrayDL icon](assets/icon.png)

## How it works

1. Run `yttraydl.exe`. The control panel opens; close it and the
   app keeps running in the system tray (notification area). Only
   Start-with-Windows launches skip the panel and go straight to
   the tray.
2. Copy a link — from a browser, a chat app, anywhere.
3a. **Ask mode (default):** a small popup appears near the corner of your
    screen with the video's title, a quality selector, and Download /
    Not now / Ignore options.
3b. **Auto-download mode:** no popup at all. The download starts immediately
    with your default quality and folder; a small toast confirms it finished
    (and failures are always reported, never silent).

Right-click the tray icon to pause monitoring, switch modes, open Settings,
open the download folder, and more. Left-click opens the control panel.

## Highlights

- **Real clipboard events, not polling** — ytTrayDL registers as a Windows
  clipboard format listener (`AddClipboardFormatListener` /
  `WM_CLIPBOARDUPDATE`), so it reacts instantly and idles at zero cost.
- **Per-domain rules** — ignore a domain forever, auto-download from the
  ones you trust, allow extra domains beyond the built-in list.
- **Sensible scope** — by default only known video/audio sites trigger it;
  ordinary web links don't. Switch to "any link" if you use yt-dlp's
  long-tail sites.
- **No duplicates** — re-copying the same link doesn't re-trigger, and a
  yt-dlp download archive means previously downloaded videos are skipped
  (the popup offers "Download anyway" when yt-dlp detects one).
- **Progress you can see** — live percent, speed and ETA in the panel and
  Downloads window, parsed from yt-dlp's own output.
- **Hide the tray icon if you want** — with three ways back: relaunch
  ytTrayDL, press **Ctrl+Alt+G**, or run `yttraydl.exe --show`. The
  confirmation dialog explains this before hiding, so you can't lock
  yourself out.
- **First-run friendly** — if yt-dlp isn't installed, one click downloads
  the official standalone `yt-dlp.exe` from GitHub into ytTrayDL's own
  folder. Same for ffmpeg: if it's missing, one click downloads the GyanD
  essentials build (the Windows build ffmpeg.org links to) and unpacks
  it next to yt-dlp. Best-quality merging and MP3 conversion need ffmpeg.
- **Closing the window is your choice** — the X button asks: Exit,
  minimize to tray, minimize to taskbar, or hide entirely (keeps
  running in the background; **Ctrl+Alt+G**, relaunching, or `--show`
  brings it back). Tick "Remember my choice" to stop being asked, and
  change it any time in Settings → General.
- **Private by design** — no accounts, no telemetry, nothing leaves your
  PC except the requests yt-dlp itself makes. Settings live in
  `%APPDATA%\ytTrayDL`.

## Quick start

Two ways to get it from the
[Releases page](https://github.com/SeanShaffer88/ytTrayDL/releases):

- **Installer** — `ytTrayDL-Setup-vX.Y.Z-windows-x64.exe`: per-user
  install (no admin) to `%LOCALAPPDATA%\Programs\ytTrayDL`, with a
  Start-menu entry, optional desktop shortcut and start-with-Windows,
  and a standard uninstaller. Your settings/history in
  `%APPDATA%\ytTrayDL` are kept on uninstall.
- **Portable zip** — `ytTrayDL-vX.Y.Z-windows-x64.zip`: unzip anywhere
  and run `yttraydl.exe`. Nothing is installed.

Then:

1. Run it. If yt-dlp is missing, the panel opens with an **Install yt-dlp**
   button.
2. Copy any supported link and try it.

Optional: enable **Start with Windows** in Settings (per-user, no admin
needed). If ffmpeg is missing, use the panel's **Install ffmpeg** button
(or drop `ffmpeg.exe` next to `yttraydl.exe` / on PATH yourself).

## Configuration tour

**Settings → General** — monitoring on/off, default action (Ask /
Auto-download), detection scope, popup position and auto-close timer,
dedupe window, which notifications you want, tray icon visibility,
start-with-Windows.

**Settings → Downloads** — download folder, file-name template, default
quality preset (Best, up to 2160p/1440p/1080p/720p/480p/360p, audio MP3,
audio original, or a custom yt-dlp format string), container (MP4/MKV/
original), embed thumbnail/metadata/subtitles, playlist behaviour,
simultaneous download count, extra yt-dlp arguments, browser cookies
(for signed-in content; yt-dlp reads them, ytTrayDL never sees them).

**Settings → Domains & rules** — the ignored / auto-download / allowed
domain lists and ignored specific links, with the rule order explained.

**Tray menu** — pause/resume, auto-download toggle, "Download from
clipboard" (acts on what's copied *now*), Downloads, download folder,
Settings, start-with-Windows, hide icon, About, Exit.

**Command line**

| Argument | Effect |
| --- | --- |
| `yttraydl.exe --show` (or `--settings`) | If already running: restore the tray icon and open Settings. If not running: start with the panel open. |
| `yttraydl.exe https://…` | Treat the URL like a copied link (opens the download popup). |

## Building

Rust 1.80+ (stable). The library (settings, URL detection, rules,
yt-dlp command building, download manager) is platform-independent and
fully unit-tested; the tray/Win32 parts compile for Windows.

```powershell
cargo build --release          # produces target\release\yttraydl.exe
cargo test                     # library tests
cargo clippy --all-targets     # lint gate (also run for the Windows target in CI)
```

Windows 10 and 11 are the target platforms. A release is a single
self-contained `.exe` — put `yt-dlp.exe` beside it, install it from the
app, or have it on PATH.

## Status & verification

v0.1.0. The core library is covered by unit tests and the whole crate
(including the Win32 clipboard listener and tray code) is compile-checked
for `x86_64-pc-windows-msvc` in CI. Runtime behaviour on a real Windows
machine (tray interaction, clipboard listener, popup placement) is the
release gate and is tracked in [docs/DESIGN.md](docs/DESIGN.md#verification-matrix)
with Verified / Partially verified / Unverified labels — nothing is
claimed as runtime-verified until it has run on a real PC.

## License

MIT — see [LICENSE](LICENSE). ytTrayDL drives yt-dlp as a separate
program; yt-dlp is published by its authors under the Unlicense.
