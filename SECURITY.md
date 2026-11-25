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
that infrastructure can defeat this check. This is not independent signature
verification or an immutable version pin across installations; the provider
resolves a concrete release for each operation. Existing user-supplied tools are
only checked for presence and are not authenticated by Setup.

Upstream yt-dlp publishes signed checksum files and a public OpenPGP key. The
current Windows installer does **not** verify those signatures: it does not ship
an OpenPGP verifier or a separately authenticated, lifecycle-managed key. Adding
an optional system `gpg` or downloading a key beside the artifact would not be a
reliable fail-closed trust anchor. Independently establish and pin the signing key,
ship a maintained verifier, test rotation and make verification mandatory before
claiming signature protection. The selected ffmpeg provider is checked against
GitHub's release-asset digest; no independent signature verification is performed.

Tools run with the user's privileges. For least-privilege/offline setup use
`/TOOLSDOWNLOAD=skip`; see [installer details](docs/INSTALLER-TOOLS.md).
Never paste credentials into logs or public reports.
