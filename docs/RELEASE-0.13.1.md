# ytTrayDL 0.13.1

## What's new

- Setup reuses available yt-dlp and ffmpeg, downloading only missing tool
  families. Existing files are detected using the app's lookup rules without
  being executed or overwritten, and are not automatically updated. If a
  configured ffmpeg path is missing, correct it or choose offline setup.
- The main window is non-resizable at the requested reference size. Its saved
  position is retained, while legacy saved sizes are ignored; it can shrink to
  fit a smaller monitor work area.
- Rules categories use visible radio buttons where space permits, with a compact
  radio menu in smaller Settings windows.
- Repeated module requests from the panel and tray ask the existing window to
  focus, matching the direct-launch intent behavior.

## Downloads

- `ytTrayDL-Setup-v0.13.1-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.13.1-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

Player playback requires separately installed **ffplay**, which is not included
in the installer or managed ffmpeg download. Existing ffmpeg installations are
not checked for ffprobe by Setup.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and checksums. Consult its run for
results. Hosted Windows installer and native intent tests and headless egui tests
are not interactive Windows validation. Foreground focus, fixed-window sizing on
physical mixed-DPI displays, actual playback, accessibility and keyboard behavior
still need hands-on Windows 10/11 review. Full FluentGruv compliance and online
player integration remain incomplete.
