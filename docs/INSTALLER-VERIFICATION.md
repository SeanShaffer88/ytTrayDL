# Installer verification checklist

Run on a disposable Windows 10 and Windows 11 machine or VM using the exact
candidate installer. Capture the installer version and SHA-256, OS build, logs,
exit codes, test steps and results. These interactive checks are **unverified**;
existing hosted CI scenarios do not establish physical desktop behaviour.
Use only disposable paths and synthetic files. See [installer tools](INSTALLER-TOOLS.md)
for the provisioning and preservation contract.

- [ ] Clean per-user install: verify launch, shortcuts and tool discovery.
- [ ] Upgrade from the previous stable release: verify launch, settings and
  user data survive; check the candidate and baseline versions.
- [ ] Uninstall and reinstall: verify application-owned files and registration
  are removed as intended while settings, tools and user files are preserved.
- [ ] Select **Skip Downloads — Install Offline**; verify no tool provisioning
  occurs and post-install launch is disabled.
- [ ] Run `yttraydl.exe --provision-tools all --report C:\ExistingFolder\new-report.json`
  with a new absolute report path; verify exit status, report and managed-tool
  resolution. Do not reuse a stale report.
- [ ] Exercise interactive failure, retry, skip and cancel; document outcomes.

Retain sanitized evidence with the exact release candidate; do not publish user
paths, tokens, private media or credentials. Hosted installer CI and its artifacts
provide separate automated evidence. The candidate release workflow attaches a sanitized real-media evidence summary;
the new release flow still requires hosted validation.

## Hosted automation

The CI candidate adds disposable-hosted scenarios for the newest published
stable version strictly older than the candidate (selected from the first 100
releases and checked against its published checksum), uninstall followed by
reinstall, and installed-app `--provision-tools all` with a fresh report. The
report scenario also checks that an existing report is refused without changing
its bytes or the managed-tool manifest. The candidate runs remain required;
these silent tests cannot establish interactive wizard or physical desktop
behaviour. Historical baseline jobs remain separate regression tests.

The GitHub release API and published checksum are from the same repository and
do not provide independent authentication of the old installer.
