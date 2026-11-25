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
