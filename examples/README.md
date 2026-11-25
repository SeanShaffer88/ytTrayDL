# Examples

These examples use synthetic URLs and do not download media.

## Per-domain rules and prompt/automatic behavior

Run `cargo run --locked --example rules_configuration`. It constructs in-memory
settings, demonstrates a YouTube-domain quality pin and auto-download rule,
then compares prompt and global auto-download decisions. Nothing is persisted.
Rules still apply scope and ignored-domain checks before automatic decisions.

## Silent versus prompt setup

In Settings → General choose **Ask me (popup)** for confirmation or
**Auto-download** to use the selected default quality. Monitoring and rules
still apply. For unattended installer use `ytTrayDL-Setup-vX.Y.Z-windows-x64.exe
/VERYSILENT /TOOLSDOWNLOAD=skip` to install without tool downloads or automatic
app launch. This is an installer command, not a download command.

## Direct launch and tool provisioning

`yttraydl.exe --downloads`, `--converter`, `--tags`, or `--player` opens the
matching window, forwarding to the existing instance on Windows. For explicit
managed-tool provisioning, `yttraydl.exe --provision-tools yt-dlp --report
C:\existing-dir\new-report.json` may access the network and requires a new
absolute report path. See [security policy](../SECURITY.md) for the trust model.
The `native_intent` and `perf_harness` examples are developer fixtures, not
ordinary user commands; follow their source documentation before running them.
