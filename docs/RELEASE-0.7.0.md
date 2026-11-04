# ytTrayDL 0.7.0 migration and validation contract

Use this guide to migrate completion hooks and assess the scope of validation.
The `v0.7.0` tag and corresponding GitHub release are the publication record;
this document defines the checks and records the evidence available before
publication. An unchecked item below has no completed result recorded here.

## Completion-hook migration

In Settings → Downloads, your post-download command now identifies a native
executable followed by arguments. ytTrayDL parses the template first, expands
placeholders in each argument once, and launches the executable directly.
It no longer sends this setting to a shell. Your saved setting is retained.

For example, the setting `explorer.exe "{dir}"` continues to open the destination
folder. Quote executable paths or arguments containing spaces with double
quotes. Whitespace separates arguments outside double quotes; `""` preserves an
empty argument. Single quotes do not group text. An unmatched double quote fails.

Backslashes remain literal except immediately before a double quote: each pair
becomes one backslash, and an odd remaining backslash escapes the quote. A quoted
path ending in a backslash therefore needs a doubled trailing backslash.

Supported placeholders are `{file}`, `{dir}`, `{url}` and `{title}`. Use them only
in arguments, never in the executable. Inserted text stays within its argument;
it cannot introduce another argument or trigger another placeholder expansion.

Replace legacy shell pipelines, redirection, command chaining, shell scripts and
interpreter commands with a native executable that accepts ordinary arguments.
Unsupported templates report an error. Environment-variable references such as
`%USERPROFILE%` are passed literally; use a literal executable path. Review your
saved hook before relying on it after upgrading.

## Build requirements

You need Rust 1.89 or later. Windows builds target
`x86_64-pc-windows-msvc` and require the MSVC C++ tools and Windows SDK.
CI installs Rust 1.89.0 with the minimal profile and checks the full locked graph
with all targets on Linux and native Windows. Stable CI continues to run format,
host and Windows clippy, the full locked test suite and Windows all-target check.
The native Windows job also runs the full suite before building release assets.

The local native Windows `cargo +1.89.0 check --locked --all-targets` exited 0.
Linux and Windows minimum-toolchain checks also run in CI; their results are
recorded by the commit's GitHub Actions checks.
If the locked supported graph fails on 1.89.0, inspect the exact package
and target before changing the minimum or dependencies; do not bypass the check.

The release build selects `cargo build --release --locked --bin yttraydl` so the
native regression fixture is not built as a release application. Artifact
inventory must still confirm that the fixture is absent from shipped assets.

## Dependency security update

The lockfile updates Tokio from 1.54.0 to 1.54.1 to address
RUSTSEC-2026-0384. The coordinator reports `cargo audit` exited 0 with zero known
vulnerabilities after this scoped update. Informational warnings remain: five
unmaintained dependencies, one unsound dependency (`glib`, absent from the Windows
dependency graph), and two yanked dependencies. This is not a warning-free audit
or evidence that runtime and release gates passed.

## Library module documentation

See [Process and completion-hook modules](PROCESS.md) for the public APIs,
consumer example, provenance and containment limits. The final disk-correction
run passed all 98 native tests (83 library, 1 hook integration, 4 manager/background
and 10 process lifecycle), plus format, both clippy gates and the Windows-target
all-target check; all five commands exited 0. Native controlled-memory clipboard
tests passed. See [the verification matrix](DESIGN.md#verification-matrix) for
individual test evidence and the remaining manual runtime limits.

A preliminary unpacked-crate consumer passed, and a hook-guard mutation produced
the expected failing test before source restoration. Refresh the final archive
and repeat independent consumption after documentation and source are final.

## Release checklist

Recorded native evidence:

- [x] Pass the full 98-test locked suite after the disk-admission correction.
- [x] Pass format, host clippy, Windows-target clippy and Windows all-target check.
- [x] Pass the local Rust 1.89.0 locked all-target check on native Windows.
- [x] Record native process-tree fixtures and controlled-memory clipboard results in [DESIGN.md](DESIGN.md#verification-matrix).

Publication contract (final outcomes are recorded by GitHub Actions and Releases):

- Independent final diff review approved the last disk correction.
- Require passing Linux and Windows Rust 1.89.0 checks and stable CI, including Unix runtime tests.
- Refresh the final packed crate and verify its consumption in an independent project.
- Build the final source in release mode and verify executable version 0.7.0.
- Merge passing work, then obtain canonical assets from that merged commit's Windows CI.
- Verify executable/installer version, inventory and SHA-256 hashes; exclude the native process fixture.
- Publish notes with verified assets and explicit runtime limits.

Manual runtime follow-up (unverified; record these limits with publication):

- [ ] Exercise malformed data through the real Windows clipboard listener.
- [ ] Verify a configured hook and final output path after a real media download.
- [ ] Exercise GUI lifecycle controls and real network probe/install behavior.
- [ ] Exercise installation/upgrade and portable startup on Windows.

These manual follow-ups are distinct from the native fixture gates. Static
package checks do not establish installer lifecycle behavior. The installer
compiler is unavailable locally; Windows CI installs it and builds the installer.
No local installer execution is claimed. The historical 69-test baseline and the
earlier 97-test run do not replace the final 98-test evidence.

## Rollback

Keep the 0.6.1 release assets available. Before upgrading, exit ytTrayDL and back
up your data folder (`%APPDATA%\ytTrayDL`, or `ytTrayDL-data` beside a portable
executable). If you need to roll back, exit 0.7.0 and restore the previous
application and backed-up data. Review completion hooks before enabling them:
0.6.1 uses the previous shell execution semantics. Downgrade/installer lifecycle
behavior remains unverified until exercised on Windows.

## Publication notes contract

0.7.0 tightens clipboard and process boundaries, tracks the final output file,
and makes disk-space admission specific to each destination with automatic
recovery. Background work gains deadlines and bounded output; pause, cancellation
and shutdown clean up owned process trees. Completion hooks now accept a native
executable and quoted arguments rather than shell commands. Source builds require
Rust 1.89 or later. See the migration instructions above before upgrading a
configured hook. Attach the final verification results and artifact hashes to
the GitHub release,
and retain the manual runtime limits listed above.
