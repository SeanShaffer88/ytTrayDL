# ytTrayDL 0.12.0

## What's new

- Export the local player queue as a UTF-8 M3U or M3U8 playlist to a new file.
  Existing files are never overwritten; network-share, URL, non-UTF-8 and
  multiline paths are rejected. An I/O failure can leave a partial file.
- Remove duplicate player queue paths on request while retaining their first
  occurrence and the selected track. Comparison is lexical, without filesystem
  access or case folding. Clearing the queue no longer risks stale row indices.
- Installer checks cover optional module shortcuts and preservation of player
  settings and queues. An owned two-process Windows fixture checks actual native
  delivery of URL and module launch intents, including repeated Player requests.

## Downloads

- `ytTrayDL-Setup-v0.12.0-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.12.0-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

Player playback requires separately installed **ffplay**; it is not included in
the installer or managed ffmpeg download. Install it next to the selected
ffmpeg, put it on PATH, or choose its executable in the player options.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and checksums. Consult its run for
results. Hosted Windows tests cover the native intent receiver and installer
shortcut metadata, but do not verify interactive shortcut activation, window
focus, actual ffplay playback, GPU encoding, keyboard access or physical DPI
and monitor behavior on a real Windows 10/11 machine. Theme alignment remains
partial. See the README and design verification matrix for further limitations.
