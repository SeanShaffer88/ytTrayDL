# ytTrayDL 0.13.4

## Main window

- The fixed main window no longer has a scrollbar. Its navigation buttons are
  Player, Lyrics & Tags, Video Encoder, Settings, Downloads and Output Folder,
  in that order. Short work areas use two columns in reading order.
- Downloads contains detailed activity, saved and ignored links, failure retry
  actions, and managed-tool progress and cancellation. Install or configure
  tools under Settings → Tools.

## Downloads

- `ytTrayDL-Setup-v0.13.4-windows-x64.exe`: per-user Windows installer.
- `ytTrayDL-v0.13.4-windows-x64.zip`: portable Windows build.
- `ytTrayDL-v0.13.4-real-media-evidence.json`: sanitized hosted test summary.
- `SHA256SUMS.txt`: checksums for the installer, ZIP and evidence summary.

To skip tool downloads during Setup, use `/TOOLSDOWNLOAD=skip` and preinstall
or configure trusted tools; see the [installer guidance](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.4/docs/INSTALLER-TOOLS.md).
Existing user-supplied tools are not authenticated by Setup.

## Testing and limitations

Hosted checks exercise egui layout bounds, native tests, Windows builds, media,
and installer scenarios. They do not establish interactive Windows rendering,
focus, accessibility, physical GPU behavior or display scaling; those remain
unverified. Mixed-DPI validation is deferred.
