# ytTrayDL 0.11.0

## What's new

- The converter offers opt-in NVIDIA NVENC, Intel Quick Sync and AMD AMF for
  H.264/HEVC MP4 and MKV output. A suitable GPU, driver and ffmpeg build are
  required. Hardware encoding uses encoder-default quality and does not silently
  fall back to software if initialization fails.
- When ffprobe supplies a usable duration, conversion shows approximate
  per-file percentage and ETA, including for trimmed files. Estimates can be
  inaccurate; conversion falls back to encoded time if duration is unknown.
- The player can search its queue, add media through bounded cancellable folder
  scans and import UTF-8 M3U/M3U8 playlists. Import skips URL, network-share
  and nested playlist entries and does not start playback.
- Explicitly save, append or delete one player queue, and save or restore the
  ffplay path, starting volume and repeat preference. These files store paths and
  options in normal or portable app data, not media; playback never resumes
  automatically. App data is retained on uninstall.
- Shared dark-theme surfaces, text, accent and custom focus outlines are closer
  to FluentGruv. This is partial alignment, not a full compliance claim.

## Downloads

- `ytTrayDL-Setup-v0.11.0-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.11.0-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

Player playback requires separately installed **ffplay**; it is not included in
the installer or managed ffmpeg download. Install it next to the selected
ffmpeg, put it on PATH, or choose its executable in the player options. Failed
or cancelled conversions may leave partial output files.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and checksums. Consult its CI run
for results. Automated checks do not replace interactive Windows 10/11 testing:
real GPU hardware encoding, actual ffplay playback, playlist import, folder
scanning, option/queue storage, window and shortcut behavior, keyboard access
and physical DPI/monitor behavior remain unverified on a real Windows machine.
Light-theme switching and full FluentGruv compliance are not implemented or
verified. See the README and design verification matrix for other limitations.
