# Security policy

## Supported versions

Security fixes are intended for the latest published release. Older releases,
including portable copies, should be upgraded; they are not separately maintained.

## Private disclosure

Please do not publish an exploit, sensitive logs, or credentials in a public
issue. Use GitHub's [private vulnerability reporting](https://github.com/SeanShaffer88/ytTrayDL/security/advisories/new)
to report a suspected vulnerability. If that form is unavailable, contact the
maintainer privately through their GitHub profile to arrange a secure channel;
do not place the details in a public issue. Include the affected version, impact,
reproduction steps and a minimal proof of concept. Remove personal data first.

Reports concerning ytTrayDL's installer, update/provisioning code, parsing,
process execution, clipboard, local data and Windows integration are in scope.
Third-party service vulnerabilities should also be reported to their maintainers.
We aim to acknowledge reports within seven days and provide an initial assessment
within 14 days; these are goals, not a guaranteed service-level agreement.
Coordinate disclosure with the maintainer so a fix can be prepared first.

## Download trust model

The managed tool provider uses HTTPS and official GitHub release metadata,
requires its advertised SHA-256 digest and expected size, checks the downloaded
bytes, validates Windows x64 executables, checks versions in bounded owned
processes, and atomically activates a generation only after verification. The
checksum and artifact come from the same release infrastructure: compromise of
that infrastructure alone can defeat the ffmpeg check. The provider resolves a
concrete release for each operation; it does not pin one version across
installations or provide anti-rollback protection. Existing user-supplied tools
are only checked for presence and are not authenticated by Setup.

For yt-dlp the provider additionally requires the release's detached OpenPGP
signature over `SHA2-256SUMS`, verifies it with the pinned primary RSA public
key, and requires its unique `yt-dlp.exe` digest to match the downloaded bytes
and GitHub's asset digest. Missing, malformed, unsupported or invalid signatures
fail closed before any executable is run or generation activated. The pin is
the RSA key (exponent 65537) from upstream `public.key` at Git commit
`12647e03d417feaa9ea6a458bea5ebd747494a53`, OpenPGP fingerprint
`AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581`; its extracted modulus has
SHA-256 `8b310e4e89140be86dc76b3ed24cb9bffe8868b005660a34e9b6b7b19e05ac99`.
The key was obtained from upstream's GitHub repository, not independently
confirmed out of band; a prior compromise of that repository could substitute
the pin. Once shipped, compromising release metadata alone cannot replace the
pinned key. Maintainers must independently authenticate and review replacement
keys, update the pin and fixtures, and release an app update before upstream
rotates it. Offline pinning cannot automatically discover revocation or expiry;
users should update ytTrayDL promptly after any reported compromise. Verification
accepts only binary-document OpenPGP v4 RSA/SHA-512 detached signatures, not a
full OpenPGP policy engine. An upstream format or algorithm change fails closed
until reviewed and supported. The selected GyanD/codexffmpeg release did not
publish detached signatures for its ZIP assets when reviewed (2026-11-24); the
provider has no independent signature verification and remains checked against
GitHub's release-asset SHA-256 digest and size. Reassess this on provider changes.

Tools run with the user's privileges. For least-privilege/offline setup use
`/TOOLSDOWNLOAD=skip`; see [installer details](docs/INSTALLER-TOOLS.md).
Never paste credentials into logs or public reports.
