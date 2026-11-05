#requires -Version 7.0
<#
.SYNOPSIS
Runs bounded real-window lifecycle checks against a private portable copy.
.NOTES
Exit 0: all selected scenarios pass; 2: failure; 3: required interactive cases remain Unknown; 125: blocked.
No registry, clipboard, installed profile or global keyboard access. WM_CLOSE is delivered only to an owned PID/window.
#>
[CmdletBinding()]param(
 [Parameter(Mandatory)][string]$AppPath,
 [Parameter(Mandatory)][string]$Destination,
 [Parameter(Mandatory)][string]$ProcessRunner,
 [switch]$AllowActions,[switch]$AllowTrustedExecution,[switch]$InspectMainControl,[switch]$EdgeConfirmations,[switch]$AllowFocusHandoff,[switch]$ExternalAskExit,
 [ValidateNotNullOrEmpty()][ValidateCount(1,9)][ValidateSet('exit','tray','taskbar','hide','ask','background','controls','tray-controls','persistence')][string[]]$Scenario=@('exit','tray','taskbar','hide','ask','background','controls','tray-controls','persistence'),
 [string]$YtDlpPath, [string]$FfmpegPath,
 [ValidateSet('Physical','ObservedCommand')][string]$TrayTransport='ObservedCommand',
 [string]$UiHelper=(Join-Path $env:USERPROFILE 'Documents/DevTools/Test-WindowsUi.ps1')
)
. (Join-Path $PSScriptRoot 'field-gui/Focus.ps1')
. (Join-Path $PSScriptRoot 'field-gui/Cleanup.ps1')
$ErrorActionPreference='Stop'
$result=[ordered]@{schemaVersion=1;status='Unknown';observedUtc=[DateTime]::UtcNow.ToString('o');focusAssistance=@();selectedScenarios=@($Scenario);cases=@();processes=@();uiaExecution=@();resourceCleanup=@();limitations=@('No accessibility certification.','WM_CLOSE validates native close delivery; it does not validate clicking a close button.');error=$null}
$owned=$null;$code=125
function Record($Name,$Status,$Evidence){$result.cases+=@{name=$Name;status=$Status;evidence=$Evidence}}
function Test-ExternalAskExitDeadline([DateTime]$ExitUtc,[DateTime]$RequestedUtc,[DateTime]$DeadlineUtc){
 $actual=$ExitUtc.ToUniversalTime();return $actual-ge$RequestedUtc.ToUniversalTime()-and$actual-le$DeadlineUtc.ToUniversalTime()
}
function VisibleWindows($Process){
 $Process.Refresh();if($Process.HasExited){return @()}
 @([FieldGuiNative]::Windows($Process.Id)|Where-Object{[FieldGuiNative]::IsWindowVisible([IntPtr]$_) -and [FieldGuiNative]::Title($_) -match 'ytTrayDL'})
}
function Stop-TrackedOwned($Process){
 $info=@($result.processes|Where-Object id -eq $Process.Id)[-1]
 if($null-eq$info){throw 'Cleanup has no owned launch identity record'}
 $cleanup=Stop-FieldGuiResource -Process $Process -Role 'LifecycleFixture' -ExpectedPath $info.path -ExpectedStartUtc ([DateTime]$info.startUtc) -TimeoutMilliseconds 5000
 $result.resourceCleanup+=@($cleanup)
 if($cleanup.status-ne'Pass'){throw ('Owned fixture cleanup failed: '+$cleanup.error)}
}
function Wait-State([scriptblock]$Predicate){$limit=[DateTime]::UtcNow.AddSeconds(8);do{if(&$Predicate){return $true};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $limit);return $false}
function Invoke-OwnedUi($Process,$Window,$Action,$Name,$Output){
 $args=@('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'field-gui/OwnedUi.ps1'),'-ProcessId',$Process.Id,'-WindowHandle',$Window,'-ExpectedPath',$Process.Path,'-ExpectedStartUtc',$Process.StartTime.ToUniversalTime().ToString('o'),'-Action',$Action,'-AllowActions')
 if($Name){$args+=@('-Name',$Name)}
 $run=& (Join-Path $PSScriptRoot 'Invoke-OwnedFieldProcess.ps1') -ProcessRunner $ProcessRunner -Executable "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -Arguments $args -TimeoutSeconds 20 -EvidencePath ($Output+'.process.json')
 $run.stdout|Set-Content $Output;$result.uiaExecution+=@{evidencePath=$run.evidencePath;rootPid=$run.rootPid;exitCode=$run.exitCode;passed=$run.passed;timedOut=$run.timedOut;cleanupVerified=$run.cleanupVerified}
 if($run.timedOut-or-not$run.cleanupVerified-or$run.exitCode-notin@(0,3)){throw ('Owned UIA execution hard failure: '+$run.error)}
 return $run.exitCode
}
function Invoke-TrayItem($Process,$Label,$Root){
 if(-not[FieldGuiNative]::IsForegroundOwned($Process.Id)){
  $main=@([FieldGuiNative]::Windows($Process.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)})
  if($main.Count-ne1){throw 'Exact owned visible main required for native-menu focus precondition'}
  $result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $Process -Window $main[0] -Stage 'native-menu-focus' -RequestPath (Join-Path $Root ('focus-needed-menu-'+[Guid]::NewGuid().ToString('N')+'.json')) -AllowAssistance:$AllowFocusHandoff)
 }
 [FieldGuiNative]::TrayMenu($Process.Id)
 $found=Wait-State { @([FieldGuiNative]::Windows($Process.Id)|Where-Object{[FieldGuiNative]::Class($_) -eq '#32768'}).Count -eq 1 }
 if(-not $found){throw 'Owned native tray menu not observed'}
 $menu=@([FieldGuiNative]::Windows($Process.Id)|Where-Object{[FieldGuiNative]::Class($_) -eq '#32768'})[0]
 @{method='OS tray callback with actual owned HWND/HMENU observation';transport=$TrayTransport;hwnd=$menu;labels=[FieldGuiNative]::MenuLabels($menu,$Process.Id);selected=$Label}|ConvertTo-Json -Depth 5|Set-Content (Join-Path $Root ('menu-'+[Guid]::NewGuid().ToString('N')+'.json'))
 if($TrayTransport -eq 'Physical'){[FieldGuiNative]::SelectMenu($menu,$Process.Id,$Label)}else{$id=[FieldGuiNative]::DispatchObservedMenu($menu,$Process.Id,$Label);@{method='Observed native HMENU unique enabled item GetMenuItemID then owned WM_CANCELMODE and standard WM_COMMAND; OS delivery equivalence, no physical menu click claimed';label=$Label;commandId=$id;ownerPid=$Process.Id;menuHwnd=$menu}|ConvertTo-Json|Set-Content (Join-Path $Root ('menu-command-'+[Guid]::NewGuid().ToString('N')+'.json'))}
}
try {
 if($ExternalAskExit-and'ask'-notin$Scenario){throw 'ExternalAskExit requires the ask scenario; no application launched.'}
 if(-not $AllowActions -or -not $AllowTrustedExecution){throw 'Both -AllowActions and -AllowTrustedExecution are required.'}
 if(-not $IsWindows){throw 'Windows required.'}
 $app=(Resolve-Path -LiteralPath $AppPath).Path
 $dest=[IO.Path]::GetFullPath($Destination)
 if(Test-Path -LiteralPath $dest){throw 'Destination must be new.'}
 $existing=@(Get-Process -Name yttraydl -ErrorAction SilentlyContinue)
 if($existing.Count){throw ('Existing unowned yttraydl process(es): '+(($existing|ForEach-Object{$_.Id}) -join ',')+'. No signaling performed.')}
 [Threading.Mutex]$existingMutex=$null
 try{if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$existingMutex)){throw 'Existing session-wide single-instance identity (possibly renamed executable); no signaling performed.'}}finally{if($null-ne$existingMutex){$existingMutex.Dispose()}}
 Add-Type -Path (Join-Path $PSScriptRoot 'field-gui/Native.cs')
 $null=New-Item -ItemType Directory -Path $dest
 $result.environment=@{os=[Environment]::OSVersion.VersionString;sessionId=(Get-Process -Id $PID).SessionId;userInteractive=[Environment]::UserInteractive;powerShell=$PSVersionTable.PSVersion.ToString();dpi='Unknown: not measured';theme='Unknown: not measured'}
 $inputApp=$app;$snapshot=Join-Path $dest 'input.exe';Copy-Item -LiteralPath $app -Destination $snapshot;$app=$snapshot
 $result.app=@{source=$inputApp;snapshot=$snapshot;sha256=(Get-FileHash -LiteralPath $snapshot -Algorithm SHA256).Hash;fileVersion=(Get-Item -LiteralPath $snapshot).VersionInfo.FileVersion}
 foreach($mode in $Scenario){
  $root=Join-Path $dest $mode;$data=Join-Path $root 'ytTrayDL-data';$null=New-Item -ItemType Directory -Path $data
  $exe=Join-Path $root 'yttraydl.exe';Copy-Item -LiteralPath $app -Destination $exe;Set-Content -LiteralPath (Join-Path $root 'portable.txt') -Value ''
  if($YtDlpPath){Copy-Item -LiteralPath (Resolve-Path -LiteralPath $YtDlpPath).Path -Destination (Join-Path $root 'yt-dlp.exe')}
  if($FfmpegPath){Copy-Item -LiteralPath (Resolve-Path -LiteralPath $FfmpegPath).Path -Destination (Join-Path $root 'ffmpeg.exe')}
  $settings=@{monitoring_enabled=$false;tray_icon_visible=$true;close_action=$(if($mode -in @('background','controls','tray-controls','persistence')){'exit'}else{$mode});ytdlp_path='';ffmpeg_path='';watch_interval_mins=0;subscriptions=@();start_with_windows=$false;download_folder=(Join-Path $root 'downloads');notify_on_complete=$false;notify_on_error=$false;notify_on_auto_start=$false;extra_args='--ignore-config --no-cache-dir';cookies_from_browser=''}
  $settings|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $data 'settings.json')
  $launch=[Diagnostics.ProcessStartInfo]::new($exe);$launch.UseShellExecute=$false;$launch.CreateNoWindow=$true;$launch.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden;$launch.WorkingDirectory=$root
  foreach($key in @('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','XDG_CONFIG_HOME','PROGRAMDATA','ALLUSERSPROFILE','TEMP','TMP')){$privateEnv=Join-Path $root ('environment/'+$key);$null=New-Item -ItemType Directory $privateEnv;$launch.Environment[$key]=$privateEnv}
  if($mode -eq 'background'){$launch.ArgumentList.Add('--background')}
  $owned=[Diagnostics.Process]::Start($launch);$result.processes+=@{id=$owned.Id;startUtc=$owned.StartTime.ToUniversalTime().ToString('o');mode=$mode;path=$exe}
  Start-Sleep -Milliseconds 1000
  if($mode -eq 'background'){
   $hidden=@(VisibleWindows $owned).Count -eq 0 -and -not $owned.HasExited
   Record 'background-start-hidden' $(if($hidden){'Pass'}else{'Fail'}) @{pid=$owned.Id;visible=@(VisibleWindows $owned)}
  }else{
   $shown=Wait-State { @(VisibleWindows $owned).Count -gt 0 }
   Record "$mode-initial-window" $(if($shown){'Pass'}else{'Fail'}) @{pid=$owned.Id;visible=@(VisibleWindows $owned)}
   if(-not $shown){throw "No visible owned window for $mode"}
   $result.environment.dpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]@(VisibleWindows $owned)[0])
   if($EdgeConfirmations -and $mode -in @('ask','controls')){
    $main=@(VisibleWindows $owned)[0];$m=[FieldGuiNative]::Metrics($main,$owned.Id);[FieldGuiNative]::MoveResizeOwned($main,$owned.Id,$m[14]-$m[2]-12,$m[15]-$m[3]-12,$m[2],$m[3]);Start-Sleep -Milliseconds 1000
    $result['edgeParent-'+$mode]=[FieldGuiNative]::Metrics($main,$owned.Id)
    if($mode-eq'controls'){
     $result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $owned -Window $main -Stage 'edge-hide' -RequestPath (Join-Path $root 'focus-needed-edge-hide.json') -AllowAssistance:$AllowFocusHandoff -RequireOperatorAcknowledgement);[FieldGuiNative]::ClickObserved($main,$owned.Id,'ytTrayDL',602,452,96,80,429)
     if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'Hide the tray icon?'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw 'Actual edge Hide confirmation not observed'}
     $child=@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'Hide the tray icon?'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)})[0]
     $fits=Wait-State {$script:edgeMetrics=[FieldGuiNative]::Metrics($child,$owned.Id);$a=$script:edgeMetrics;$a[8]-ge$a[12]-and$a[9]-ge$a[13]-and($a[8]+$a[10])-le$a[14]-and($a[9]+$a[11])-le$a[15]}
     Record 'actual-edge-Hide-confirmation-within-workarea' $(if($fits){'Pass'}else{'Fail'}) @{method='Actual physical main Hide button; CUA-observed DWM center80,429 in602x452/96DPI with strict input guards';metrics=$script:edgeMetrics}
     if(-not$fits){throw 'Hide confirmation outside actual monitor workarea'}
     [FieldGuiNative]::Close($child,$owned.Id);if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'Hide the tray icon?'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq0})){throw 'Hide confirmation normal cancel did not settle'}
     Record 'edge-Hide-normal-cancel-recovery' $(if(@(VisibleWindows $owned).Count-eq1-and-not$owned.HasExited){'Pass'}else{'Fail'}) @{method='Owned confirmation WM_CLOSE cancels, actual main remains visible';pid=$owned.Id}
    }
   }
   if($mode -eq 'persistence'){
    $window=@(VisibleWindows $owned)[0];$before=[FieldGuiNative]::WindowBounds($window,$owned.Id);$target=@(250,150,560,650)
    [FieldGuiNative]::MoveResizeOwned($window,$owned.Id,$target[0],$target[1],$target[2],$target[3]);Start-Sleep -Milliseconds 1000
    $moved=[FieldGuiNative]::WindowBounds($window,$owned.Id);$mainDpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]$window)
    $secondary=@(@{label='Settings…';title='ytTrayDL — Settings';target=@(350,160,640,700)},@{label='Downloads…';title='ytTrayDL — Downloads';target=@(480,200,680,620)})
    foreach($tool in $secondary){
     Invoke-TrayItem $owned $tool.label $root
     $title=$tool.title;if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw "Persistence tool window missing: $title"}
     $h=@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)})[0]
     $tool.before=[FieldGuiNative]::WindowBounds($h,$owned.Id);$tool.initialMetrics=[FieldGuiNative]::Metrics($h,$owned.Id);if($tool.label-eq'Downloads…'){$initial=$tool.initialMetrics;$scale=$initial[16]/96.0;$freshOkay=$initial[6]-eq[int][Math]::Round(800*$scale)-and$initial[7]-eq[int][Math]::Round(360*$scale);Record 'fresh-empty-Downloads-default-client-size' $(if($freshOkay){'Pass'}else{'Fail'}) @{expectedLogicalClient=@(800,360);actualMetrics=$initial;scope='Fresh private profile, empty history and no queued jobs; subsequent custom-size assertions remain required'}};$t=$tool.target;[FieldGuiNative]::MoveResizeOwned($h,$owned.Id,$t[0],$t[1],$t[2],$t[3]);Start-Sleep -Milliseconds 1000;$tool.moved=[FieldGuiNative]::WindowBounds($h,$owned.Id);$tool.movedDpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]$h);[FieldGuiNative]::Close($h,$owned.Id);if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq0})){throw "Persistence tool close did not settle: $title"}
     Invoke-TrayItem $owned $tool.label $root
     if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw "Persistence tool reopen missing: $title"}
     $h=@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)})[0];Start-Sleep -Milliseconds 500;$tool.sameProcessReopen=[FieldGuiNative]::WindowBounds($h,$owned.Id);[FieldGuiNative]::Close($h,$owned.Id);if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq0})){throw "Persistence tool close did not settle: $title"}
    }
    [FieldGuiNative]::Close($window,$owned.Id);$firstPid=$owned.Id;$exited=$owned.WaitForExit(8000)
    if(-not$exited -or $owned.ExitCode-ne0){throw 'Persistence setup exit failed'}
    $settingsPath=Join-Path $data 'settings.json';$null=Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json
    $result.persistenceReadback=@{path=$settingsPath;sha256AfterFirstExit=(Get-FileHash -LiteralPath $settingsPath).Hash}
    $statePath=Join-Path $data 'window-state.json';if(Test-Path -LiteralPath $statePath){$result.persistenceReadback.windowStateAfterFirstExit=@{path=$statePath;sha256=(Get-FileHash -LiteralPath $statePath).Hash;json=(Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json)}}else{$result.persistenceReadback.windowStateAfterFirstExit=@{exists=$false}}
    $stateMatches=$false
    if(Test-Path -LiteralPath $statePath){
     $state=Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json;$stateMatches=$state.schema_version-eq1
     $expectedGeometry=@{main=$moved;settings=$secondary[0].moved;downloads=$secondary[1].moved};$expectedDpi=@{main=$mainDpi;settings=$secondary[0].movedDpi;downloads=$secondary[1].movedDpi};$keys=@('x','y','width','height')
     foreach($name in $expectedGeometry.Keys){$g=$state.$name;if($null-eq$g){$stateMatches=$false;continue};for($i=0;$i-lt4;$i++){if([Math]::Abs([double]$g.($keys[$i])-$expectedGeometry[$name][$i])-gt2){$stateMatches=$false}};if($g.dpi-ne$expectedDpi[$name]){$stateMatches=$false}}
    }
    Record 'private-native-window-state-readback' $(if($stateMatches){'Pass'}else{'Fail'}) @{path=$statePath;expectedSchema=1;expectedDpiByWindow=$expectedDpi;expectedNativeRectangles=@{main=$moved;settings=$secondary[0].moved;downloads=$secondary[1].moved};readOnly=$true}
    $owned=[Diagnostics.Process]::Start($launch);$result.processes+=@{id=$owned.Id;startUtc=$owned.StartTime.ToUniversalTime().ToString('o');mode='persistence-relaunch';path=$exe}
    if(-not(Wait-State {@(VisibleWindows $owned).Count-gt0})){throw 'Persistence relaunch window missing'};Start-Sleep -Milliseconds 1000
    $restored=[FieldGuiNative]::WindowBounds(@(VisibleWindows $owned)[0],$owned.Id);$matches=$true
    for($i=0;$i-lt4;$i++){if([Math]::Abs($moved[$i]-$target[$i])-gt2 -or [Math]::Abs($restored[$i]-$moved[$i])-gt2){$matches=$false}}
    Record 'actual-main-move-resize-exit-relaunch-restoration' $(if($matches){'Pass'}else{'Fail'}) @{method='Owned HWND SetWindowPos normal rectangle, WM_CLOSE exit0, same private-profile relaunch';firstPid=$firstPid;relaunchPid=$owned.Id;before=$before;target=$target;moved=$moved;restored=$restored;tolerancePhysicalPixels=2;dpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]@(VisibleWindows $owned)[0]);limitations='No actual second-monitor or changed-DPI environment available; pure tests are separate evidence.'}
    foreach($tool in $secondary){
     Invoke-TrayItem $owned $tool.label $root;$title=$tool.title
     if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw "Persistence relaunched tool missing: $title"}
     $h=@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)})[0];Start-Sleep -Milliseconds 500;$tool.restored=[FieldGuiNative]::WindowBounds($h,$owned.Id);$matches=$true
     for($i=0;$i-lt4;$i++){if([Math]::Abs($tool.moved[$i]-$tool.target[$i])-gt2-or[Math]::Abs($tool.sameProcessReopen[$i]-$tool.moved[$i])-gt2-or[Math]::Abs($tool.restored[$i]-$tool.moved[$i])-gt2){$matches=$false}}
     Record ('actual-'+$tool.title+'-move-resize-close-relaunch-restoration') $(if($matches){'Pass'}else{'Fail'}) @{method='Actual native tray menu opens tool; owned SetWindowPos and normal WM_CLOSE; same-process reopen then app exit/relaunch and actual tray reopen';before=$tool.before;target=$tool.target;moved=$tool.moved;sameProcessReopen=$tool.sameProcessReopen;restored=$tool.restored;tolerancePhysicalPixels=2;dpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]$h)}
     [FieldGuiNative]::Close($h,$owned.Id);if(-not(Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$title-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq0})){throw "Persistence tool final close did not settle: $title"}
    }
    [FieldGuiNative]::Close(@(VisibleWindows $owned)[0],$owned.Id);$done=$owned.WaitForExit(8000);if(-not$done-or$owned.ExitCode-ne0){throw 'Persistence final application exit failed'};$result.persistenceReadback.sha256AfterFinalExit=(Get-FileHash -LiteralPath $settingsPath).Hash;if(Test-Path -LiteralPath $statePath){$result.persistenceReadback.windowStateAfterFinalExit=@{path=$statePath;sha256=(Get-FileHash -LiteralPath $statePath).Hash;json=(Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json)}};$owned=$null;continue
   }
   if($mode -eq 'tray-controls'){
    foreach($item in @(@{label='Settings…';title='Settings'},@{label='Downloads…';title='Downloads'})){
     Invoke-TrayItem $owned $item.label $root
     $observed=Wait-State {@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::IsWindowVisible([IntPtr]$_) -and [FieldGuiNative]::Title($_) -like ('*'+$item.title+'*')}).Count -gt 0}
     Record ('native-tray-'+$item.title) $(if($observed){'Pass'}else{'Fail'}) @{pid=$owned.Id;observed=$observed;windows=@([FieldGuiNative]::Windows($owned.Id)|ForEach-Object{[FieldGuiNative]::Title($_)})}
    }
    Invoke-TrayItem $owned 'Exit ytTrayDL' $root
    $exited=$owned.WaitForExit(8000)
    Record 'native-tray-menu-Exit' $(if($exited -and $owned.ExitCode -eq 0){'Pass'}else{'Fail'}) @{pid=$owned.Id;exited=$exited;exitCode=$(if($exited){$owned.ExitCode}else{$null})}
    if(-not $exited){Stop-TrackedOwned $owned};$owned=$null;continue
   }
   if($mode -eq 'exit' -and (Test-Path -LiteralPath $UiHelper)){
    $assert=Join-Path $root 'assertions.json';@{schemaVersion=1;checks=@(@{id='owned-window';selector=@{controlType='Window'}})}|ConvertTo-Json -Depth 8|Set-Content $assert
    & $UiHelper -ProcessId $owned.Id -AssertionsPath $assert -Destination (Join-Path $root 'uia') -Json | Set-Content (Join-Path $root 'uia-output.json')
    $result.uiHelperExitCode=$LASTEXITCODE
   }
   if($mode -eq 'controls' -and $InspectMainControl){
    $h=@(VisibleWindows $owned)[0];@{pid=$owned.Id;path=$owned.Path;startUtc=$owned.StartTime.ToUniversalTime().ToString('o');hwnd=$h;title=[FieldGuiNative]::Title($h);metrics=[FieldGuiNative]::Metrics($h,$owned.Id)}|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'control-ready.json')
    $deadline=[DateTime]::UtcNow.AddSeconds(180);do{$owned.Refresh();if($owned.HasExited){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow-lt$deadline)
    $exited=$owned.HasExited;Record 'actual-main-Exit-control' $(if($exited-and$owned.ExitCode-eq0){'Pass'}else{'Unknown'}) @{method='Parent CUA observed and clicked actual owned main Exit; retained process handle records exit';pid=$owned.Id;exited=$exited;exitCode=$(if($exited){$owned.ExitCode})}
    if(-not$exited){Stop-TrackedOwned $owned};$owned=$null;continue
   }
   if($mode -eq 'controls'){
    $assert=Join-Path $root 'assertions.json';@{schemaVersion=1;checks=@(@{id='exit-button';selector=@{name='Exit ytTrayDL';controlType='Button'};patterns=@('Invoke')})}|ConvertTo-Json -Depth 8|Set-Content $assert
    & $UiHelper -ProcessId $owned.Id -AssertionsPath $assert -Destination (Join-Path $root 'uia') -Json | Set-Content (Join-Path $root 'uia-output.json');$helperExit=$LASTEXITCODE
    $invokeExit=$null;$exitDelivery=$false
    if($helperExit -eq 0){
     $action=Join-Path $root 'action.json';@{schemaVersion=1;checks=@(@{id='root';selector=@{controlType='Window'}});workflow=@(@{action='Invoke';selector=@{name='Exit ytTrayDL';controlType='Button'}})}|ConvertTo-Json -Depth 8|Set-Content $action
     & $UiHelper -ProcessId $owned.Id -AssertionsPath $action -AllowActions -Json|Set-Content (Join-Path $root 'invoke.json');$invokeExit=$LASTEXITCODE;$delivery=Get-Content (Join-Path $root 'invoke.json') -Raw|ConvertFrom-Json;$exitDelivery=($invokeExit-eq0)-or($invokeExit-eq3-and$delivery.error-like'*Selected process has exited*');$method='Actual named main UIA Exit Invoke'
    }else{
     $h=@(VisibleWindows $owned)[0];@{method='Parent CUA observed actual Exit button DWM center518,429 in602x452 at96DPI; strict title/frame/foreground/PID/actualcursor/SendInput acceptedcount;1000ms pointer hold';metrics=[FieldGuiNative]::Metrics($h,$owned.Id)}|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'control-pixel-contract.json')
     $result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $owned -Window $h -Stage 'main-exit' -RequestPath (Join-Path $root 'focus-needed-main-exit.json') -AllowAssistance:$AllowFocusHandoff -RequireOperatorAcknowledgement);$activatedInvoke=Invoke-OwnedUi $owned $h 'Invoke' 'Exit ytTrayDL' (Join-Path $root 'activated-exit-action.json');if($activatedInvoke-eq0){$exitDelivery=$true;$invokeExit=0;$method='Actual named main UIA Exit Invoke after assisted provider activation'}else{$result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $owned -Window $h -Stage 'main-exit-input' -RequestPath (Join-Path $root 'focus-needed-main-exit-input.json') -AllowAssistance:$AllowFocusHandoff);[FieldGuiNative]::ClickObserved($h,$owned.Id,'ytTrayDL',602,452,96,518,429);$exitDelivery=$true;$method='Actual physical main Exit click from CUA-observed guarded DWM geometry; UIA prerequisite remained unavailable'}
    }
    $exited=$owned.WaitForExit(8000);Record 'actual-main-Exit-control' $(if($exitDelivery-and$exited-and$owned.ExitCode-eq0){'Pass'}else{'Fail'}) @{pid=$owned.Id;method=$method;helperExitCode=$helperExit;invokeExitCode=$invokeExit;exited=$exited;exitCode=$(if($exited){$owned.ExitCode}else{$null})}
    if(-not$exited){Stop-TrackedOwned $owned};$owned=$null;continue
   }
   $window=@(VisibleWindows $owned)[0];[FieldGuiNative]::Close($window,$owned.Id)
   if($mode -eq 'exit'){
    $exited=$owned.WaitForExit(8000);Record 'close-exit' $(if($exited -and $owned.ExitCode -eq 0){'Pass'}else{'Fail'}) @{pid=$owned.Id;exited=$exited;exitCode=$(if($exited){$owned.ExitCode}else{$null})}
   }elseif($mode -eq 'ask'){
    $dialog=0;$found=Wait-State {$script:dialog=@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_) -eq 'Close ytTrayDL?'});$script:dialog.Count -eq 1}
    if($found -and $EdgeConfirmations){$fits=Wait-State {$script:edgeMetrics=[FieldGuiNative]::Metrics($script:dialog[0],$owned.Id);$a=$script:edgeMetrics;$a[8]-ge$a[12]-and$a[9]-ge$a[13]-and($a[8]+$a[10])-le$a[14]-and($a[9]+$a[11])-le$a[15]};Record 'actual-edge-Close-confirmation-within-workarea' $(if($fits){'Pass'}else{'Fail'}) @{method='Owned edge main WM_CLOSE opens real Ask dialog';metrics=$script:edgeMetrics};if(-not$fits){throw 'Close confirmation outside actual monitor workarea'}}
    if($found-and$ExternalAskExit){
     $requested=[DateTime]::UtcNow;$deadline=$requested.AddSeconds(180);$readyPath=Join-Path $root 'ask-exit-ready.json';$evidencePath=Join-Path $root 'ask-exit-action-evidence.json';$start=$owned.StartTime.ToUniversalTime()
     $identity=@{pid=$owned.Id;path=$owned.Path;startUtc=$start.ToString('o');hwnd=$script:dialog[0];title='Close ytTrayDL?'}
     @{schemaVersion=1;scenario='ask';requestedUtc=$requested.ToString('o');deadlineUtc=$deadline.ToString('o');identity=$identity;metrics=[FieldGuiNative]::Metrics($script:dialog[0],$owned.Id);actionEvidencePath=$evidencePath;requiredEvidence=@{status='Performed';method='CUA';action='Exit ytTrayDL';freshScreenshot=$true;identity=$identity;completedUtc='Actual UTC action completion within request/deadline'}}|ConvertTo-Json -Depth 7|Set-Content $readyPath
     do{$owned.Refresh();if($owned.HasExited-and(Test-Path -LiteralPath $evidencePath)){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow-lt$deadline)
     $exited=$owned.HasExited;$evidenceValid=$false;$evidenceError=$null
     if(Test-Path -LiteralPath $evidencePath){try{$actual=Get-Content -LiteralPath $evidencePath -Raw|ConvertFrom-Json;$when=([DateTime]$actual.completedUtc).ToUniversalTime();$actualIdentity=$actual.identity;$evidenceValid=$actual.status-eq'Performed'-and$actual.method-eq'CUA'-and$actual.action-eq'Exit ytTrayDL'-and$actual.freshScreenshot-eq$true-and$actualIdentity.pid-eq$identity.pid-and$actualIdentity.path-eq$identity.path-and([DateTime]$actualIdentity.startUtc).ToUniversalTime().Ticks-eq$start.Ticks-and$actualIdentity.hwnd-eq$identity.hwnd-and$actualIdentity.title-eq$identity.title-and$when-ge$requested-and$when-le$deadline}catch{$evidenceError=$_.Exception.Message}}
     $exitUtc=$null;$exitWithinDeadline=$false;if($exited){$exitUtc=$owned.ExitTime.ToUniversalTime();$exitWithinDeadline=Test-ExternalAskExitDeadline $exitUtc $requested $deadline}
     $pass=$exited-and$owned.ExitCode-eq0-and$evidenceValid-and$exitWithinDeadline
     Record 'close-ask-dialog-Exit' $(if($pass){'Pass'}else{'Fail'}) @{method='External actual CUA Ask Exit; assisted, not unattended automation';pid=$owned.Id;exited=$exited;exitCode=$(if($exited){$owned.ExitCode});exitUtc=$(if($exitUtc){$exitUtc.ToString('o')});exitWithinDeadline=$exitWithinDeadline;actualActionEvidenceValid=$evidenceValid;evidenceError=$evidenceError;readyPath=$readyPath;actionEvidencePath=$evidencePath;deadlineUtc=$deadline.ToString('o')}
     $result.limitations+=@('Ask Exit used explicit actual CUA business action; no unattended Ask input qualification.')
    }elseif($found){$actionExit=Invoke-OwnedUi $owned $script:dialog[0] 'Invoke' 'Exit ytTrayDL' (Join-Path $root 'dialog-action.json');if($actionExit-ne0){@{method='CUA observed Ask Exit DWM-visible-frame center159,86; exact472x362 frame96DPI/title and foreground/PID guards';metrics=[FieldGuiNative]::Metrics($script:dialog[0],$owned.Id)}|ConvertTo-Json|Set-Content (Join-Path $root 'dialog-pixel-contract.json');$result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $owned -Window $script:dialog[0] -Stage 'ask-exit' -RequestPath (Join-Path $root 'focus-needed-ask-exit.json') -AllowAssistance:$AllowFocusHandoff -RequireOperatorAcknowledgement);$actionExit=Invoke-OwnedUi $owned $script:dialog[0] 'Invoke' 'Exit ytTrayDL' (Join-Path $root 'activated-dialog-action.json');if($actionExit-ne0){$result.focusAssistance+=@(Confirm-FieldGuiFocus -Process $owned -Window $script:dialog[0] -Stage 'ask-exit-input' -RequestPath (Join-Path $root 'focus-needed-ask-exit-input.json') -AllowAssistance:$AllowFocusHandoff);[FieldGuiNative]::ClickObserved($script:dialog[0],$owned.Id,'Close ytTrayDL?',472,362,96,159,86);$actionExit=0}};$exited=$owned.WaitForExit(8000);Record 'close-ask-dialog-Exit' $(if($actionExit-eq0 -and $exited -and $owned.ExitCode-eq0){'Pass'}else{'Fail'}) @{pid=$owned.Id;actionExit=$actionExit;exited=$exited}}
    else{Record 'close-ask-dialog' 'Fail' @{pid=$owned.Id;reason='Expected native dialog not observed'}}
   }elseif($mode -eq 'taskbar'){
    $minimized=Wait-State { @([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::IsIconic([IntPtr]$_)}).Count -gt 0 }
    Record 'close-taskbar' $(if($minimized){'Pass'}else{'Fail'}) @{pid=$owned.Id;minimized=$minimized}
   }else{
    $hidden=Wait-State { @(VisibleWindows $owned).Count -eq 0 }
    Record "close-$mode-hidden" $(if($hidden -and -not $owned.HasExited){'Pass'}else{'Fail'}) @{pid=$owned.Id;visible=@(VisibleWindows $owned)}
   }
  }
  if($mode -in @('tray','hide','background')){
   $secondary=Start-Process -FilePath $exe -ArgumentList '--show' -WindowStyle Hidden -PassThru
   $secondaryExited=$secondary.WaitForExit(8000);$shown=Wait-State {@(VisibleWindows $owned).Count -gt 0}
   Record "$mode-relaunch-same-pid" $(if($secondaryExited -and $secondary.ExitCode -eq 0 -and $shown -and -not $owned.HasExited){'Pass'}else{'Fail'}) @{originalPid=$owned.Id;secondaryPid=$secondary.Id;secondaryExitCode=$(if($secondaryExited){$secondary.ExitCode}else{$null});visible=@(VisibleWindows $owned)}
   if(-not $secondaryExited){if(-not$secondary.HasExited){$secondary.Kill($true);if(-not$secondary.WaitForExit(5000)){throw 'Owned secondary cleanup deadline exceeded'}}}
  }
  $owned.Refresh();if(-not $owned.HasExited){Stop-TrackedOwned $owned;$result.processes[-1].cleanup='Owned process tree terminated after observations; not an application exit assertion'}
  $owned=$null
 }

 $failed=@($result.cases|Where-Object status -eq Fail).Count -gt 0
 $unknown=($result.cases.Count-eq0 -or @($result.cases|Where-Object{$_.status-notin@('Pass','Fail')}).Count-gt0);$result.status=if($failed){'Fail'}elseif($unknown){'Unknown'}else{'Pass'};$code=if($failed){2}elseif($unknown){3}else{0}
}catch{$result.status='Blocked';$result.error=$_.Exception.Message;$result.failurePosition=$_.InvocationInfo.PositionMessage;$result.failureStack=$_.ScriptStackTrace;$code=125}
finally{
 if($null -ne $owned){try{$owned.Refresh();if(-not $owned.HasExited){Stop-TrackedOwned $owned}}catch{$result.cleanupError=$_.Exception.Message;$result.status='Fail';$code=2}}
 $result.exitCode=$code
 if($dest -and (Test-Path -LiteralPath $dest)){$result|ConvertTo-Json -Depth 20|Set-Content -LiteralPath (Join-Path $dest 'result.json')}
 $result|ConvertTo-Json -Depth 20
}
exit $code
