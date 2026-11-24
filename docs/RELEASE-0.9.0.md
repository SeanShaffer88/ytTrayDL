# ytTrayDL 0.9.0

## What's new

- Choose lossy audio download quality and optionally install a Downloads shortcut.
- Open the separate Video converter from the panel, tray or optional desktop
  shortcut. It supports sequential batch conversion, trim, crop, resize,
  rotation, audio removal, video and audio profiles, quality choices and
  encoded-time progress. HEVC and AV1 require suitable ffmpeg encoders.
- Open the separate Lyrics & tags window to inspect and edit audio metadata,
  manually add lyrics, export embedded artwork or replace it in supported
  audio formats. Exports create new files and do not overwrite the original or
  an existing destination. Detected lyrics are not deliberately replaced;
  verify the copy because ffmpeg may omit or alter metadata or artwork.
- Optional online lyrics search sends the entered title and artist to LRCLIB
  only when requested. Review and choose a result before export.

## Downloads

- `ytTrayDL-Setup-v0.9.0-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.9.0-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

The player, hardware acceleration and percentage conversion progress are not
included. ffmpeg/ffprobe are required for conversion and tag operations;
encoders and supported media vary by build. Failed or cancelled operations can
leave partial outputs. Existing user data is retained on uninstall.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable version, architecture and checksums. Consult the linked
CI run for its exact results. Automated hosted checks are not a substitute for
interactive Windows 10/11 testing: the new module windows, shortcuts, artwork
and live lyrics lookup have not been verified on a real Windows machine, and
live LRCLIB lookup was not available from the development sandbox. No claim of
full FluentGruv compliance or physical DPI/monitor validation is made.
