#requires -Version 7.0
param([Parameter(Mandatory)][string]$AppPath,[Parameter(Mandatory)][string]$Destination,[Parameter(Mandatory)][string]$FixtureDirectory,[Parameter(Mandatory)][string]$YtDlpPath,[Parameter(Mandatory)][string]$FfmpegPath,[Parameter(Mandatory)][string]$PythonPath,[switch]$AllowActions,[switch]$AllowTrustedExecution,[switch]$InspectPopup,[switch]$UseCliUrl,[switch]$ExternalDownloadClick,[Parameter(Mandatory)][string]$ProcessRunner,[ValidateSet('None','CleanupFailure','UiAStall')][string]$HarnessControl='None',[switch]$AllowFocusHandoff,[switch]$VerifyDownloadsSizeDuringQueue)
. (Join-Path $PSScriptRoot 'Cleanup.ps1')
. (Join-Path $PSScriptRoot 'Focus.ps1')
$resources=@();$UiWorkerPath=Join-Path $PSScriptRoot 'OwnedUi.ps1';$UiDeadlineSeconds=20
$ErrorActionPreference='Stop';$code=125;$app=$null;$server=$null;$r=[ordered]@{status='Unknown';cases=@();focusAssistance=@();uiaExecution=@();cleanup=@();error=$null;scope='Actual GUI manual URL field and popup Invoke; real ffmpeg extraction shutdown on original loopback media'}
function Wait-For([scriptblock]$P,[int]$Seconds=12){$deadline=[DateTime]::UtcNow.AddSeconds($Seconds);do{if(&$P){return $true};Start-Sleep -Milliseconds 80}while([DateTime]::UtcNow-lt$deadline);return $false}
function Ui($Window,$Action,$Name,$Value,$File){$a=@('-NoProfile','-ExecutionPolicy','Bypass','-File',$UiWorkerPath,'-ProcessId',$app.Id,'-WindowHandle',$Window,'-ExpectedPath',$app.Path,'-ExpectedStartUtc',$app.StartTime.ToUniversalTime().ToString('o'),'-Action',$Action,'-AllowActions');if($Name){$a+=@('-Name',$Name)};if($Value){$a+=@('-Value',$Value)};$run=& (Join-Path (Split-Path $PSScriptRoot -Parent) 'Invoke-OwnedFieldProcess.ps1') -ProcessRunner $ProcessRunner -Executable "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -Arguments $a -TimeoutSeconds $UiDeadlineSeconds -EvidencePath (Join-Path $dest ($File+'.process.json'));$run.stdout|Set-Content (Join-Path $dest $File);$r.uiaExecution+=@{path=$run.evidencePath;rootPid=$run.rootPid;passed=$run.passed;exitCode=$run.exitCode;timedOut=$run.timedOut;cleanupVerified=$run.cleanupVerified};if(-not$run.passed){$error=[InvalidOperationException]::new("Owned UI action failed: $File");$error.Data['HardExecutionFailure']=($run.timedOut-or-not$run.cleanupVerified-or$run.exitCode-notin@(0,3));throw $error}}
try{
 if(-not$AllowActions-or-not$AllowTrustedExecution){throw 'Both execution acknowledgements required'}
 if($HarnessControl-ne'CleanupFailure'){
  if(Get-Process yttraydl -ErrorAction SilentlyContinue){throw 'Existing yttraydl process; no signaling performed'}
  [Threading.Mutex]$existingMutex=$null
  try{if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$existingMutex)){throw 'Existing session-wide single-instance identity (possibly renamed executable); no signaling performed.'}}finally{if($null-ne$existingMutex){$existingMutex.Dispose()}}
 }
 $dest=[IO.Path]::GetFullPath($Destination);if(Test-Path $dest){throw 'Destination must be new'};$null=New-Item -ItemType Directory "$dest/ytTrayDL-data"
 $source=(Resolve-Path $AppPath).Path;$exe=Join-Path $dest 'yttraydl.exe';Copy-Item $source $exe;Set-Content "$dest/portable.txt" ''
 $r.app=@{path=$source;snapshot=$exe;sha256=(Get-FileHash $exe).Hash;version=(Get-Item $exe).VersionInfo.FileVersion}
 $listen=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$listen.Start();$port=$listen.LocalEndpoint.Port;$listen.Stop()
 $psi=[Diagnostics.ProcessStartInfo]::new((Resolve-Path $PythonPath).Path);foreach($arg in @('-I','-m','http.server',"$port",'--bind','127.0.0.1','--directory',(Resolve-Path $FixtureDirectory).Path)){$psi.ArgumentList.Add($arg)};$psi.UseShellExecute=$false;$psi.CreateNoWindow=$true;$psi.RedirectStandardError=$true;$psi.RedirectStandardOutput=$true;$server=[Diagnostics.Process]::Start($psi);$resources+=@{role='Server';process=$server;path=$psi.FileName;startUtc=$server.StartTime.ToUniversalTime()};$url="http://127.0.0.1:$port/combined.mp4";$r.server=@{pid=$server.Id;url=$url;fixtureHash=(Get-FileHash (Join-Path $FixtureDirectory 'combined.mp4')).Hash}
 $ready=Wait-For {try{$response=Invoke-WebRequest $url -Method Head -TimeoutSec 1;return $response.StatusCode-eq200}catch{return $false}} 5;if(-not$ready){throw 'Owned fixture server unavailable'}
 if($HarnessControl-eq'CleanupFailure'){$r.scope='Negative control only: owned server cleanup failure; no GUI/media coverage';$r.cases+=@{name='Synthetic candidate-pass fixture for cleanup-status guard';status='Pass';synthetic=$true;coverage='Harness-only: owned loopback server ready; no functional GUI/media coverage'};$r.status='Pass';$code=0;$skip=[InvalidOperationException]::new('Cleanup negative-control dispatch');$skip.Data['CleanupControl']=$true;throw $skip}
 @{monitoring_enabled=$false;watch_interval_mins=0;subscriptions=@();close_action='exit';ytdlp_path=(Resolve-Path $YtDlpPath).Path;ffmpeg_path=(Resolve-Path $FfmpegPath).Path;download_folder="$dest/downloads";default_preset='audio_mp3';popup_timeout_secs=600;embed_thumbnail=$false;embed_metadata=$false;notify_on_complete=$false;notify_on_error=$false;sound_on_finish=$false;open_folder_on_finish=$false;skip_already_downloaded=$false;extra_args='--ignore-config --no-cache-dir --postprocessor-args "ExtractAudio+ffmpeg_i1:-re -stream_loop 5"';cookies_from_browser=''}|ConvertTo-Json|Set-Content "$dest/ytTrayDL-data/settings.json"
 Add-Type -Path (Join-Path $PSScriptRoot 'Native.cs');$launch=[Diagnostics.ProcessStartInfo]::new($exe);$launch.UseShellExecute=$false;$launch.CreateNoWindow=$true;$launch.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden;$launch.WorkingDirectory=$dest
 foreach($key in @('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','XDG_CONFIG_HOME','PROGRAMDATA','ALLUSERSPROFILE','TEMP','TMP')){$privateEnv=Join-Path $dest ('environment/'+$key);$null=New-Item -ItemType Directory $privateEnv;$launch.Environment[$key]=$privateEnv}
 if($InspectPopup -or $UseCliUrl -or $ExternalDownloadClick){$launch.ArgumentList.Add($url)};$r.environmentIsolation=@{workingDirectory=$dest;redirectedVariables=@('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','XDG_CONFIG_HOME','PROGRAMDATA','ALLUSERSPROFILE','TEMP','TMP')};$app=[Diagnostics.Process]::Start($launch);$r.pid=$app.Id;$resources+=@{role='App';process=$app;path=$exe;startUtc=$app.StartTime.ToUniversalTime()};if($HarnessControl-eq'UiAStall'){$r.scope='Negative control only: stalled UIA surrogate with owned application cleanup';$UiWorkerPath=Join-Path $dest 'acknowledged-stall.ps1';Set-Content $UiWorkerPath 'Start-Sleep -Seconds 300';$UiDeadlineSeconds=2;Ui 0 'Snapshot' '' '' 'stall-uia.json';throw 'Controlled UIA stall unexpectedly completed'}
 if(-not(Wait-For {@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw 'Owned main window unavailable'}
 $main=@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL'})[0]
 if(-not ($InspectPopup -or $UseCliUrl -or $ExternalDownloadClick)){Ui $main 'TypeValue' '' $url 'set-url.json';Start-Sleep -Milliseconds 200;Ui $main 'InvokeManualDownload' '' '' 'open-popup.json'}else{$r.scope='Documented CLI URL opens actual download popup; GUI confirms Download and exits during real ffmpeg'}
 if(-not(Wait-For {@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL — download?'}).Count-eq1})){throw 'Actual download popup missing'}
 if($VerifyDownloadsSizeDuringQueue){
  $r.focusAssistance+=@(Confirm-FieldGuiFocus -Process $app -Window $main -Stage 'empty-downloads-open' -RequestPath (Join-Path $dest 'focus-needed-downloads.json') -AllowAssistance:$AllowFocusHandoff)
  [FieldGuiNative]::TrayMenu($app.Id)
  if(-not(Wait-For {@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Class($_)-eq'#32768'}).Count-eq1})){throw 'Owned Downloads native menu not observed'}
  $menu=@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Class($_)-eq'#32768'})[0]
  $command=[FieldGuiNative]::DispatchObservedMenu($menu,$app.Id,'Downloads…')
  if(-not(Wait-For {@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL — Downloads'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)}).Count-eq1})){throw 'Actual empty Downloads window unavailable'}
  $downloadsWindow=@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL — Downloads'})[0]
  [FieldGuiNative]::MoveResizeOwned($downloadsWindow,$app.Id,480,200,680,620);Start-Sleep -Milliseconds 1000
  $r.downloadsQueueGeometry=@{method='Observed native HMENU dynamic enabled item ID and standard owned WM_COMMAND; native normal-window resize';commandId=$command;hwnd=$downloadsWindow;before=[FieldGuiNative]::WindowBounds($downloadsWindow,$app.Id);beforeMetrics=[FieldGuiNative]::Metrics($downloadsWindow,$app.Id)}
 }
 $popup=@([FieldGuiNative]::Windows($app.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL — download?'})[0];Start-Sleep -Seconds 3;$r.popupGeometry=[FieldGuiNative]::Metrics($popup,$app.Id);$r.popupGeometry|ConvertTo-Json|Set-Content (Join-Path $dest 'popup-metrics.json');Start-Sleep -Seconds 2;if($InspectPopup -or $ExternalDownloadClick){@{pid=$app.Id;startUtc=$app.StartTime.ToUniversalTime().ToString('o');path=$app.Path;hwnd=$popup;title=[FieldGuiNative]::Title($popup);bounds=[FieldGuiNative]::WindowBounds($popup,$app.Id);url=$url}|ConvertTo-Json -Depth 5|Set-Content (Join-Path $dest 'inspection-ready.json');if(-not(Wait-For {Test-Path (Join-Path $dest 'resume.txt')} 180)){throw 'Inspection handoff deadline exceeded'}};if(-not $ExternalDownloadClick){try{Ui $popup 'Invoke' '⬇ Download' '' 'confirm-download.json'}catch{if($_.Exception.Data['HardExecutionFailure']){throw};$r.popupGeometry=[FieldGuiNative]::Metrics($popup,$app.Id);@{method='CUA observed Download rectangle via DWM visible-frame translation; x85 and29px from bottom; exact432width/546or584height/96DPI/title, workarea/foreground/PID guards';metrics=$r.popupGeometry}|ConvertTo-Json -Depth 5|Set-Content (Join-Path $dest 'popup-pixel-contract.json');$r.focusAssistance+=@(Confirm-FieldGuiFocus -Process $app -Window $popup -Stage 'download-confirm' -RequestPath (Join-Path $dest 'focus-needed-download.json') -AllowAssistance:$AllowFocusHandoff -RequireOperatorAcknowledgement);[FieldGuiNative]::ClickObservedDownload($popup,$app.Id)}}else{$r.downloadConfirmationMethod='Parent CUA actual owned popup Download click; private resume marker after handoff'}
 $script:observed=@();$active=Wait-For {$all=@(Get-CimInstance Win32_Process);$ids=@($app.Id);for($i=0;$i-lt6;$i++){$next=@($all|Where-Object{$_.ParentProcessId-in$ids}|ForEach-Object ProcessId);$ids+=@($next|Where-Object{$_-notin$ids})};$script:treeIds=$ids;$script:observed=@($all|Where-Object{$_.ProcessId-in$ids-and$_.Name-eq'ffmpeg.exe'-and$_.ExecutablePath-eq(Resolve-Path $FfmpegPath).Path});$script:observed.Count-gt0} 25
 if(-not$active){throw 'Real owned ffmpeg descendant was not observed before intervention'}
 if($VerifyDownloadsSizeDuringQueue){
  $r.downloadsQueueGeometry.after=[FieldGuiNative]::WindowBounds($downloadsWindow,$app.Id);$r.downloadsQueueGeometry.afterMetrics=[FieldGuiNative]::Metrics($downloadsWindow,$app.Id)
  $same=($r.downloadsQueueGeometry.before|ConvertTo-Json -Compress)-eq($r.downloadsQueueGeometry.after|ConvertTo-Json -Compress)
  $r.cases+=@{name='Custom Downloads bounds survive empty-to-active real ffmpeg queue transition';status=$(if($same){'Pass'}else{'Fail'});geometry=$r.downloadsQueueGeometry;scope='Actual same open window before confirmation and during observed owned ffmpeg; no seeded job status'}
 }
 $r.ffmpeg=@($script:observed|Select-Object ProcessId,ParentProcessId,ExecutablePath,CommandLine);$r.observedUtc=[DateTime]::UtcNow.ToString('o');$descendants=@($script:treeIds|Where-Object{$_-ne$app.Id})
 try{Ui $main 'Invoke' 'Exit ytTrayDL' '' 'exit-control.json';$r.exitMethod='Actual named main UIA Exit control'}catch{if($_.Exception.Data['HardExecutionFailure']){throw};$r.exitMethod='Owned WM_CLOSE OS close-button delivery with private close_action exit; named UIA control unavailable';[FieldGuiNative]::Close($main,$app.Id)};$exited=$app.WaitForExit(8000);$gone=Wait-For {@(Get-Process -Id $descendants -ErrorAction SilentlyContinue).Count-eq0} 8
 $before=@(Get-ChildItem "$dest/downloads" -File -Recurse -ErrorAction SilentlyContinue|Select-Object FullName,Length,LastWriteTimeUtc);Start-Sleep -Milliseconds 1500;$after=@(Get-ChildItem "$dest/downloads" -File -Recurse -ErrorAction SilentlyContinue|Select-Object FullName,Length,LastWriteTimeUtc);$stable=($before|ConvertTo-Json -Compress)-eq($after|ConvertTo-Json -Compress)
 $history=Join-Path $dest 'ytTrayDL-data/history.json';$completed=$false;if(Test-Path $history){$completed=@((Get-Content $history -Raw|ConvertFrom-Json).entries|Where-Object status -eq completed).Count -gt 0}
 $pass=$exited-and$app.ExitCode-eq0-and$gone-and$stable-and-not$completed;$r.cases+=@{name='GUI Exit during observed real ffmpeg';status=$(if($pass){'Pass'}else{'Fail'});exited=$exited;exitCode=$(if($exited){$app.ExitCode});ownedDescendantsGone=$gone;noWritesAfterExit=$stable;completedHistory=$completed;descendantPids=$descendants};$r.status=if($pass){'Pass'}else{'Fail'};$code=if($pass){0}else{2}
}catch{if(-not$_.Exception.Data['CleanupControl']){$r.error=$_.Exception.Message;$r.status='Blocked'}}finally{
 foreach($resource in @($resources|Sort-Object @{Expression={if($_.role-eq'App'){0}else{1}}})){
  $cleanup=Stop-FieldGuiResource -Process $resource.process -Role $resource.role -ExpectedPath $resource.path -ExpectedStartUtc $resource.startUtc -TimeoutMilliseconds 5000 -InjectPrimaryFailure:($HarnessControl-eq'CleanupFailure'-and$resource.role-eq'Server');$r.cleanup+=@($cleanup)
  if($cleanup.status-ne'Pass'){$r.status='Fail';$code=2;$r.cleanupFailed=$true}
 }
 if($code-eq0-and($r.cases.Count-eq0-or@($r.cases|Where-Object status -ne Pass).Count-gt0)){$r.status='Fail';$code=2;$r.error='No complete passing scenario assertions recorded'} 
 $r.exitCode=$code;if($dest-and(Test-Path $dest)){$r|ConvertTo-Json -Depth 12|Set-Content "$dest/result.json"};$r|ConvertTo-Json -Depth 12
}
exit $code



















