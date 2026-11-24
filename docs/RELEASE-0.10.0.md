# ytTrayDL 0.10.0

## What's new

- Open a local player queue from the main panel, tray menu or optional desktop
  shortcut (`yttraydl.exe --player` for portable use).
- Add up to 500 local audio and video files, select, reorder or remove tracks,
  and use repeat, previous, next and stop. The queue lasts for the current session.
- The separate ffplay playback window provides pause, seek, volume and mute
  keyboard controls. Closing that window advances the queue; closing the queue
  manager leaves playback running.

## Downloads

- `ytTrayDL-Setup-v0.10.0-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.10.0-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

Player playback requires a separately installed **ffplay**, which is not included
in the installer or managed ffmpeg download. Install it next to the selected
ffmpeg, put it on PATH, or select its executable in the player options. Local
files only; network playback and playlist files are not supported.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and checksums. Consult the linked
CI run for its exact results. Automated checks are not a substitute for
interactive Windows 10/11 testing: actual ffplay playback, controls, module
window lifecycle, tray access and player shortcut behavior have not been
verified on a real Windows machine. No claim of full FluentGruv compliance or
physical DPI/monitor validation is made. See the README and design verification
matrix for other module limitations.
