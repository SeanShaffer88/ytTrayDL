# ytTrayDL 0.13.2

## Security and maintenance

- Managed yt-dlp downloads now require an upstream detached OpenPGP signature
  on the SHA-256 list, verified with the pinned release key, in addition to the
  existing release metadata digest, size, executable and version checks. A
  missing, invalid or unsupported signature fails closed before activation.
- Updated the transitive `url` parser to address RUSTSEC-2026-0592. Added
  dependency auditing, policy checks and scheduled bounded fuzz smoke tests.
- Added a security disclosure policy, dependency review, contributor guide,
  Windows safety notes and least-privilege/offline installer instructions.
- Expanded offline downloader, signature and generated-input tests, and split
  shared GUI presentation helpers into modules without changing their behavior.

## Downloads

- `ytTrayDL-Setup-v0.13.2-windows-x64.exe`: per-user installer.
- `ytTrayDL-v0.13.2-windows-x64.zip`: extract to its own folder for portable use.
- `SHA256SUMS.txt`: package checksums.

To avoid tool downloads during Setup, use `/TOOLSDOWNLOAD=skip` and preinstall
or configure trusted tools; see [installer documentation](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.2/docs/INSTALLER-TOOLS.md).
Existing user-supplied tools are not authenticated by Setup. Player playback
requires separately installed **ffplay**, which is not bundled.

## Trust and verification limitations

The yt-dlp key was pinned from upstream's GitHub repository, not independently
verified out of band. Key rotation or revocation requires an app update, and a
new upstream signature format fails closed. The ffmpeg ZIP remains verified
against GitHub release metadata rather than an independent signature. See the
[security policy](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.2/SECURITY.md) for the complete trust model.

The release workflow builds from the tagged merge commit and checks package
inventory, executable versions, architecture and hashes. Hosted Windows tests
and headless checks do not establish interactive focus, DPI, GPU, hotkey,
accessibility or real playback behavior; those remain hands-on Windows checks.
The [audit addendum](https://github.com/SeanShaffer88/ytTrayDL/blob/v0.13.2/docs/AUDIT-2026-11-24.md) records test evidence and known
informational dependency advisories.
