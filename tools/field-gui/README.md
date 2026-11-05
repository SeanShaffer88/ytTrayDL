# GUI field test adapters

Test-only tooling; never include these scripts or test executables in release assets.

`../Invoke-GuiFieldValidation.ps1` validates actual owned native close delivery,
private-profile background/show recovery, named main UIA Exit, and native tray
menu Settings / Downloads / Exit. The main script requires an explicit executable,
a new destination and both execution/action acknowledgements. Select only the
requested scenarios. Its result is lifecycle coverage only. The persistence scenario moves and resizes normal main, Settings and Downloads HWNDs inside the selected workarea, closes/reopens the tool windows, exits/relaunches the same private profile, and compares actual native rectangles plus read-only schema-1 window-state.json geometry/hash evidence. Real second-monitor/DPI changes are outside this host run.

`Invoke-MediaExit.ps1` additionally requires explicit trusted yt-dlp, ffmpeg,
Python and an original synthetic fixture directory containing `combined.mp4`.
It creates a bounded loopback HTTP server. Default mode types its own URL into the actual GUI; explicit `-UseCliUrl` uses the documented URL launch contract instead. It
uses actual popup controls, observes a real descendant ffmpeg before exit, then
checks application/child exit, stable download files and no completed history.
A missing popup/control/ffmpeg observation is nonpassing. No external extractor
coverage or actual sign-in behavior is claimed.

`OwnedUi.ps1` runs in a Windows PowerShell child with process-scoped execution
policy only. It checks executable path, process creation time and exact HWND
ownership; never captures password values or the clipboard. Empty/ambiguous
provider trees fail. UIA geometry is used only for the unique manual Edit and its
adjacent download action. Native mouse input is guarded by the owned foreground
PID and `WindowFromPoint`, holds each click across render frames, then restores
the prior cursor position. Keyboard text is bounded WM_CHAR transport to the
owned focused window; no global Tab or clipboard transport is used.

`Native.cs` is an ephemeral native test adapter. The default `ObservedCommand`
tray transport opens the actual native popup through the tray-icon 0.21.3 callback,
resolves a unique enabled label and `GetMenuItemID` from its observed HMENU, checks
foreground and exact owned PID/HWND, dismisses only that owned menu and delivers
standard `WM_COMMAND` to its verified `tray_icon_app` owner. This is native command
delivery equivalence; it does not claim a physical menu or shell-icon click.
The optional `Physical` transport checks actual cursor readback and `SendInput`
accepted counts before observing outcomes. Host physical-input failures remain
nonpassing evidence; they are never recast as passing physical clicks.
All profiles disable monitoring and watching and use synthetic private output. The media app receives child-only private home/config/cache/temp environment roots because the metadata query does not inherit download extra arguments. The loopback server uses Python isolated mode (`-I`).
Preflight refuses existing yttraydl processes because the application's mutex
identity is session-wide. Failure cleanup targets only processes launched by
the current run. No host startup registration or installed profile is modified.
Automated checks do not certify accessibility. See each run's JSON for exact
binaries, PIDs, methods, observed outcomes and unperformed checks.





Both entry scripts require an explicit absolute `-ProcessRunner` pointing to the
trusted built InstallerStateProbe supporting `--run-owned`. UIA children run
through `Invoke-OwnedFieldProcess.ps1`, which reuses production OwnedProcess
containment and enforces a deadline. Timeout, abnormal execution or unverified
cleanup is a hard failure; a normal unavailable-provider result remains
nonpassing UIA evidence even when a separately guarded physical control succeeds.

`-AllowFocusHandoff` permits a bounded 180-second operator/CUA focus and UIA
provider activation handoff for an exact owned PID/path/HWND. The request JSON
specifies its `.activated` acknowledgement path. The operator activates and
observes only that window, then acknowledges; the script independently requires
the actual foreground PID before performing its own control action. These runs
are assisted, not unattended GUI qualification. Physical fallback geometry is
from actual CUA observations and refuses any title/frame-size/DPI mismatch.

Media cleanup records every launched application/server PID, identity, primary
outcome, emergency outcome and deadline. Any primary cleanup failure remains
nonpassing even after verified emergency termination. Explicit acknowledged
`-HarnessControl CleanupFailure` and `-HarnessControl UiAStall` negative controls
exercise that false-pass guard and a real two-second contained helper timeout.
They claim no functional GUI/media coverage. A stalled-helper control also
verifies bounded cleanup of the private owned app and loopback server.
