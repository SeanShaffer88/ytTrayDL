# ytTrayDL 0.13.3

## Trimming and playback

- Compatible single-file and batch trims can use stream-copy after ffprobe
  checks the codecs and selected options. Cuts can start at an earlier keyframe
  and may not be frame-accurate. Transforms, quality and bitrate selections,
  explicit hardware encoding, unknown codecs or unavailable ffprobe require
  re-encoding. The converter reports the method and any fallback.
- Copy attempts use an isolated temporary directory and publish without
  overwriting through a hard link. A nonzero copy failure can fall back to
  re-encoding after cleanup; cancellation, timeout, I/O and publication errors
  stop the batch. Hard-link support is required for copy publication, and an
  interruption can leave a `.yttraydl-trim-*` directory. Failed re-encodes may
  leave a partial output. Hardware encoder failures never silently use software.
- The local player now explains ffplay discovery and links to the official
  FFmpeg download page. ffplay is not included in the managed ffmpeg download.
  Network URL playback remains deferred; the player is local-only.

## Downloads

- `ytTrayDL-Setup-v0.13.3-windows-x64.exe`: per-user Windows installer.
- `ytTrayDL-v0.13.3-windows-x64.zip`: portable Windows build.
- `ytTrayDL-v0.13.3-real-media-evidence.json`: sanitized hosted test summary.
- `SHA256SUMS.txt`: checksums for the installer, ZIP and evidence summary.

To skip tool downloads during Setup, use `/TOOLSDOWNLOAD=skip` and preinstall
or configure trusted tools; see the [installer guidance](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.3/docs/INSTALLER-TOOLS.md).
Existing user-supplied tools are not authenticated by Setup.

## Testing and limitations

Hosted Windows tests exercise real synthetic media, installer lifecycle,
previous-stable upgrade, uninstall/reinstall, offline skip and a fresh tool
provisioning report. The release workflow verifies package inventories, hashes,
and a sanitized real-media report; its repair verifier is also tested against
simulated draft metadata. These tests do not establish interactive player
controls, shortcuts, hotkey recovery, UI Automation or physical GPU encoding.
Those checks remain unverified and are listed in the [roadmap](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.3/ROADMAP.md)
and [hardware matrix](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.3/docs/HARDWARE-ENCODING-MATRIX.md).
Interactive mixed-DPI validation is deferred to a dedicated display milestone.
