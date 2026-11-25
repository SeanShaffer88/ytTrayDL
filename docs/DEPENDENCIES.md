# Dependency advisory tracking

The lockfile includes platform-specific and optional edges, not all of which
are compiled for the Windows application. Recheck actual reachability with
`cargo tree --target x86_64-pc-windows-msvc -i <crate>` and Linux equivalents
when updating dependencies. RustSec status changes over time; this is a review
plan, not a substitute for a fresh scan.

| Crate / advisory | Current path and impact | Remediation / tracking |
|---|---|---|
| `url` / RUSTSEC-2026-0592 | Stack overflow on nested blob origins in 2.5.8. | Lockfile updated to patched 2.5.9 with form_urlencoded 1.2.3; CI must confirm resolution and audit. |
| `proc-macro-error` 1.0.4 / RUSTSEC-2024-0370 | Unmaintained build-time macro through optional GTK/glib macro stack; not application runtime code. | Track upstream GTK/tray dependency migration; do not enable unused GTK features for Windows. |
| `ttf-parser` 0.25.1 / RUSTSEC-2026-0192 | Unmaintained parser through `owned_ttf_parser` / GUI font stack; font parsing remains security relevant. | Track egui/font-stack migration and evaluate a maintained compatible parser when upstream supports it. |
| `zbus_names` 4.3.5 / RUSTSEC-2026-0371 | Unmaintained through zbus / Linux accessibility stack; target-specific, not the Windows tray implementation. | Monitor zbus/accesskit upstream replacement and update the compatible dependency family together. |
| `zvariant` 5.15.1 / RUSTSEC-2026-0370 | Unmaintained D-Bus serialization path used by Linux accessibility dependencies. | Same upstream zbus/accesskit migration; retain bounds and review transitive changes. |
| `zvariant_derive` 5.15.1 / RUSTSEC-2026-0372 | Unmaintained derive macro paired with zvariant; build time. | Update together with zvariant through upstream, not a manual incompatible override. |
| `glib` 0.18.5 / RUSTSEC-2024-0429 | RustSec unsoundness notice on the inactive optional GTK path in the Windows-target product graph. | Keep the optional GTK path disabled on Windows; track compatible upstream GTK upgrade and reassess before enabling. |

No advisory is asserted to be harmless if its path becomes active. Full
transitive reachability varies with target and features; cargo-audit scans the
lockfile and cargo-deny checks its configured graph. Upgrading or replacing a
transitive crate without compatible upstream APIs risks breaking platform builds;
these are tracked rather than hidden behind broad exemptions. See the historical
[dependency review](RELEASE-0.8.0.md#dependency-review) and
[audit](AUDIT-2026-11-21.md#dependency-resolution).

The GUI font package declares the SIL Open Font License 1.1 and Ubuntu Font
Licence 1.0 in addition to MIT/Apache; the policy explicitly allows those font
licenses for this dependency graph. This is a compatibility review aid, not a
claim that the existing third-party notices cover every historical dependency.
