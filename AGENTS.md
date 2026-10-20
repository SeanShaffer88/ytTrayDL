# ytTrayDL — agent notes

- Architecture rule: all decision logic lives in the library
  (`settings`, `url_detect`, `rules`, `dedupe`, `ytdlp`, `download`,
  `history`) and must stay testable off-Windows. The GUI
  (`src/gui.rs`, `src/gui_windows.rs`) renders state and collects
  intent; it does not invent rules.
- Win32 surface lives only in `src/platform/windows_impl.rs`. Keep it
  narrow: clipboard listener, single-instance/show event, hotkey,
  autostart, ShellExecute, cursor position.
- Gate (all of it, every change):
  `cargo fmt --all`,
  `cargo clippy --all-targets -- -D warnings`,
  `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`
  (host clippy never sees `cfg(windows)` code),
  `cargo test --locked` (full suite — never a filtered subset),
  `cargo check --target x86_64-pc-windows-msvc --all-targets`.
- Never claim Windows runtime behaviour is verified until it has run
  on a real Windows machine; keep the verification matrix in
  `docs/DESIGN.md` current with Verified / Partially verified /
  Unverified labels.
- URLs from the clipboard are data, never shell text: yt-dlp is always
  spawned with an argv vector and the URL after `--`.
- Documentation names no third-party products beyond components
  actually used (yt-dlp, ffmpeg, Windows APIs).
- License: MIT. Look: black/red Command Deck family identity.
