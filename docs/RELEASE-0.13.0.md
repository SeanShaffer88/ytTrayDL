# ytTrayDL 0.13.0

## What's new

- The main panel has a narrower initial layout, a separate status band, grouped
  link and engine cards, stacked quick controls and full-width navigation. Its
  body scrolls at compact sizes while footer actions remain accessible.
- Settings uses radio buttons for value choices instead of cyclic arrows. Larger
  groups open in a radio-button menu. Sections can be selected directly from a
  menu; page and list navigation remain paginated for compact windows.
- Section dividers and grouped surfaces draw on the supplied layout references
  while retaining ytTrayDL's FluentGruv-inspired colors and existing modules.

## Downloads

- `ytTrayDL-Setup-v0.13.0-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.13.0-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

Player playback requires separately installed **ffplay**, which is not included
in the installer or managed ffmpeg download.

## Verification and limitations

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and checksums. Consult its run for
results. Headless layout tests and hosted Windows checks do not replace an
interactive visual, keyboard, shortcut, playback or physical DPI and monitor
review on Windows 10/11. The refreshed UI is an adaptation of the references,
not a pixel-perfect reproduction; full FluentGruv compliance remains unverified.
