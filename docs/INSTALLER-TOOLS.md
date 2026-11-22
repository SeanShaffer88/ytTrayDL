# Installer Tool Provisioning

The installer remains per-user and does not change PATH or require elevation.
Latest stable official yt-dlp and Gyan ffmpeg essentials downloads are resolved
by the application's canonical provider. Downloading instead of bundling keeps
the installer small and makes the versions current at install time; bundling
is technically possible but deliberately not selected.

## Installer Contract

The Download Tools page defaults to download. Explicit offline selection or
`/TOOLSDOWNLOAD=skip` skips provider execution and disables post-install launch.
Required silent downloads fail setup before file/registration changes when the
provider cannot finish. An invalid mode is refused. Interactive failures offer
Retry, Skip Tools, or Cancel Setup; cancel is queued until the pre-install event
returns. In-flight downloads are bounded by the canonical helper; the wizard
waits for it and shows indeterminate overall progress, without fabricated stages.

`PrepareToInstall` extracts the exact candidate under Setup's private temporary
directory, before normal application files are copied. It executes
`--provision-tools all --report <new-private-report>` and checks both process
creation and exit zero, plus fresh report existence. The same candidate owns
the report contract; the installer does not duplicate its JSON or download logic.
Failure details are bounded and retained in Setup's log. A successful managed
tool commit remains valid if a later application installation step fails.

Managed generations and `bin/current-tools.json` live in the application data
boundary. Settings, history, managed tools, external tools, user media and
unrelated files are not installer-owned uninstall targets. Existing generation
files are retained so a running download can finish with its original tool.

The application installer and portable payload include `THIRD-PARTY-NOTICES.txt`
with verbatim license/notice files and exact package/source provenance for the
new verification/archive dependency closure. This bounded record does not claim
a complete legal qualification of every historical dependency. Downloaded media
tools retain their own release license/provenance in their managed generations.

## Verification Scope

Local Inno Setup 7.1.1 syntax compilation passed. No installer was executed on
the host. CI explicitly selects Inno Setup 6.5.4 and executes installers only on
disposable GitHub-hosted Windows VMs. Hosted results for the exact commit remain
required before delivery.

Local verification on the frozen candidate:

| Check | Observed result |
|---|---|
| Release app | Windows x64 / PE32+, product and file version 0.8.0; SHA256 `83C6ADD5FA5879A0CAC602E78F97D98FC7B29D27F5737F53BDDB54C68A012395` |
| Release footprint | 7,104,512 bytes, +353,792 bytes (+5.24%) from the 6,750,720-byte audit baseline; statically linked verification/archive handling adds no external runtime dependency |
| Local installer compile | Inno 7.1.1, exit 0; 4,418,218-byte prototype including notices, SHA256 `FCB74ECC3CD193F822FB1B89DC50B2BF11A9BD769F05BD6B681BBD6F6CA45799`; this is not the authoritative CI Inno 6 artifact |
| Static package inventory | Strict portable app/README/license/notices/marker, setup, and portable ZIP roundtrip inventories passed, exit 0; app explicitly checked x64. Static inspection does not execute an installer |
| Dependency notices | 162,954 bytes; SHA256 `E0CFAE1BF0C00B8F27279B43B1BFBAFC7F70EAD202047D8564FDF41F6D2B7CDD`; included in installer and portable CI payload |
| Actual official tools | Private portable all-tools provisioning succeeded with yt-dlp 2026.10.26 and ffmpeg/ffprobe 9.1; the freshly linked actual library consumer passed, exit 0 |
| Offline rollback | Exact release app, private child proxy pointed to closed loopback: expected tool-operation exit 2, verified owned cleanup, and unchanged prior managed pointer, settings, and all three real tool hashes; verification wrapper exit 0 |
| Tooling bounds | State probe compiled and its explicit 1–1800-second deadline boundary test passed, exit 0; local installer host-execution guard refused execution before writes |

These direct-CLI checks validate the shared provider and actual resolver, not
installer lifecycle behavior. Hosted install/uninstall jobs remain distinct gates.

The existing four historical installer lifecycle scenarios explicitly skip tool
downloads. Dedicated scenarios exercise required live official downloads on
clean install and published-baseline upgrade, closed-loopback-proxy required
failure, and explicit offline skip. Tests inspect the installer-produced managed
pointer and all three file hashes immediately, without a second provisioning
call that could mask an integration failure. Uninstall must preserve those files
and synthetic user state. The independent state probe also calls the application's
actual managed-tool resolvers immediately after installation and after uninstall.
Owned installer process trees have an explicit 30-minute maximum, while ordinary
historical lifecycle phases retain their shorter five-minute bound. Offline tests preserve an inert executable fixture and
manifest bytes; that proves data retention, not media execution. Real media
execution is covered separately by the existing media field harness.

Interactive Retry/Skip/Cancel clicks and physical setup accessibility are not
claimed by headless silent installer jobs.

## Implementation Sources

- [CreateInputOptionPage](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_createinputoptionpage.htm)
- [PrepareToInstall event](https://jrsoftware.org/inno-setup/documentation/topic_scriptevents.htm)
- [Temporary extraction](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_extracttemporaryfile.htm)
- [Exec creation result and child exit code](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_exec.htm)
- [Task dialog special Retry/Ignore/Abort label order](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_taskdialogmsgbox.htm)
- [Indeterminate progress](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_createoutputmarqueeprogresspage.htm)
- [Queued close message](https://jrsoftware.org/inno-setup/documentation/topic_isxfunc_postmessage.htm)
- [Inno 6.5.4 cancellation implementation](https://github.com/jrsoftware/issrc/blob/is-6_5_4/Projects/Src/Setup.MainForm.pas)
