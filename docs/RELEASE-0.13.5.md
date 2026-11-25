# ytTrayDL 0.13.5

## Main window and tool windows

- The navigation buttons fill the space above the main window's bottom toolbar.
- Settings, Downloads, Player, Lyrics & Tags, Video Encoder and About open at
  the main window's position when possible. Windows near a monitor edge are
  adjusted to remain within the work area. You can move each window independently
  while it is open; Settings and Downloads retain their saved sizes.

## Downloads

- `ytTrayDL-Setup-v0.13.5-windows-x64.exe`: per-user Windows installer.
- `ytTrayDL-v0.13.5-windows-x64.zip`: portable Windows build.
- `ytTrayDL-v0.13.5-real-media-evidence.json`: sanitized hosted test summary.
- `SHA256SUMS.txt`: checksums for the installer, ZIP and evidence summary.

To skip tool downloads during Setup, use `/TOOLSDOWNLOAD=skip` and preinstall
or configure trusted tools; see the [installer guidance](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.5/docs/INSTALLER-TOOLS.md).
Existing user-supplied tools are not authenticated by Setup.

## Testing and limitations

Hosted checks exercise egui layout bounds, native tests, Windows builds, media,
and installer scenarios. Interactive Windows rendering, placement, focus,
accessibility, physical GPU behavior and display scaling remain unverified for
this revision. Mixed-DPI validation is deferred.
