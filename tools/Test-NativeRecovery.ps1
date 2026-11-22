#requires -Version 7.0
<#
.SYNOPSIS
Tests owned hidden-window recovery and second-instance URL delivery without downloads.
.NOTES
Creates a new private portable profile. No clipboard writes, global key input,
registry changes, installers, or downloader discovery. WM_HOTKEY dispatch is
tested; physical hotkey registration and accessibility are not certified.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$AppPath,
      [Parameter(Mandatory)][string]$Destination,
      [Parameter(Mandatory)][string]$IntentFixturePath,
      [switch]$AllowTrustedExecution)
$ErrorActionPreference = 'Stop'
$result = [ordered]@{status='Failed';cases=@();limitations=@('WM_HOTKEY message dispatch, not physical keyboard input.','Owned UIA snapshot, not accessibility certification.');error=$null}
$primary=$null;$secondary=$null;$probe=$null;$createdRoot=$false
try {
 if(-not $IsWindows -or -not $AllowTrustedExecution){throw 'Windows and AllowTrustedExecution are required.'}
 if(Test-Path -LiteralPath $Destination){throw 'Destination must be new.'}
 if(@(Get-Process -Name yttraydl -ErrorAction SilentlyContinue).Count){throw 'Existing yttraydl process; no signaling performed.'}
 [Threading.Mutex]$existing=$null
 try {if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$existing)){throw 'Existing single-instance mutex; no signaling performed.'}} finally {if($existing){$existing.Dispose()}}
 $source=(Resolve-Path -LiteralPath $AppPath).Path
 $null=New-Item -ItemType Directory -Path $Destination
 $createdRoot=$true
 $root=(Resolve-Path -LiteralPath $Destination).Path
 $exe=Join-Path $root 'yttraydl.exe';Copy-Item -LiteralPath $source -Destination $exe
 $null=New-Item -ItemType Directory -Path (Join-Path $root 'ytTrayDL-data')
 Set-Content -LiteralPath (Join-Path $root 'portable.txt') -Value ''
 $privateSettings=@{monitoring_enabled=$false;start_with_windows=$false;subscriptions=@();watch_interval_mins=0;close_action='tray';tray_icon_visible=$true;popup_timeout_secs=0;ytdlp_path=(Join-Path $root 'absent-downloader.exe');ffmpeg_path=(Join-Path $root 'absent-ffmpeg.exe');download_folder=(Join-Path $root 'downloads');notify_on_complete=$false;notify_on_error=$false}
 $privateSettings|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $root 'ytTrayDL-data/settings.json')
 Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class RecoveryNative {
 public delegate bool EnumProc(IntPtr h,IntPtr p);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent,IntPtr after,string cls,string title);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback,IntPtr p);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 public static IntPtr Window(uint wanted,string title){IntPtr found=IntPtr.Zero;EnumWindows((h,p)=>{uint pid;GetWindowThreadProcessId(h,out pid);if(pid==wanted){var s=new StringBuilder(256);GetWindowText(h,s,s.Capacity);if(s.ToString()==title){found=h;return false;}}return true;},IntPtr.Zero);return found;}
}
'@
 function Launch([string[]]$Arguments){
  $info=[Diagnostics.ProcessStartInfo]::new($exe);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WorkingDirectory=$root
  foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}
  foreach($key in @('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','XDG_CONFIG_HOME','TEMP','TMP')){$private=Join-Path $root ('environment/'+$key);$null=New-Item -ItemType Directory -Path $private -Force;$info.Environment[$key]=$private}
  $info.Environment['PATH']=''
  [Diagnostics.Process]::Start($info)
 }
 function Wait-Condition([scriptblock]$Condition){$deadline=[DateTime]::UtcNow.AddSeconds(12);do{if(&$Condition){return};Start-Sleep -Milliseconds 80}while([DateTime]::UtcNow-lt$deadline);throw 'Owned window condition timed out.'}
 $primary=Launch @('--background')
 $result.phase='listener'
 $listener=[IntPtr]::Zero
 Wait-Condition { $script:listener=[RecoveryNative]::FindWindowEx([IntPtr](-3),[IntPtr]::Zero,'ytTrayDLMessageWindow','ytTrayDL');$result.listener=$script:listener.ToInt64();$result.primaryId=$primary.Id;if($script:listener-ne[IntPtr]::Zero){[uint32]$owner=0;$null=[RecoveryNative]::GetWindowThreadProcessId($script:listener,[ref]$owner);$result.listenerOwner=$owner;$owner-eq$primary.Id}else{$false} }
 $main=[IntPtr]::Zero
 $result.phase='root'
 Wait-Condition { $script:main=[RecoveryNative]::Window($primary.Id,'ytTrayDL');$script:main-ne[IntPtr]::Zero }
 # Missing-tool recovery intentionally opens the panel. Close through the app's
 # tray policy so BOTH app state and native visibility are hidden.
 Wait-Condition {[RecoveryNative]::IsWindowVisible($main)}
 $null=[RecoveryNative]::PostMessage($main,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
 Wait-Condition {-not [RecoveryNative]::IsWindowVisible($main)}
 $hiddenUntil=[DateTime]::UtcNow.AddSeconds(1.2)
 do {if([RecoveryNative]::IsWindowVisible($main)){throw 'Hidden application state was not stable.'};Start-Sleep -Milliseconds 80}while([DateTime]::UtcNow-lt$hiddenUntil)
 if(-not [RecoveryNative]::PostMessage($listener,0x0312,[IntPtr]0xC11B,[IntPtr]::Zero)){throw 'Owned hotkey message failed.'}
 $result.phase='hotkey-visible'
 Wait-Condition {[RecoveryNative]::IsWindowVisible($main)}
 $result.cases+=@{name='hidden-root-hotkey-dispatch';status='Pass';pid=$primary.Id;hwnd=$main.ToInt64()}
 $url='https://example.invalid/audit-cli'
 $secondary=Launch @($url)
 if(-not $secondary.WaitForExit(8000)){throw 'Secondary URL sender did not exit.'}
 if($secondary.ExitCode-ne0){throw ('Secondary exit '+$secondary.ExitCode)}
 $popup=[IntPtr]::Zero
 $result.phase='forwarded-popup'
 Wait-Condition {$script:popup=[RecoveryNative]::Window($primary.Id,'ytTrayDL — download?');$script:popup-ne[IntPtr]::Zero-and[RecoveryNative]::IsWindowVisible($script:popup)}
 # UIA runs in a bounded child because a provider can stall.
 $snapshot=Join-Path $root 'popup-snapshot.json'
 $info=[Diagnostics.ProcessStartInfo]::new("$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe");$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
 foreach($argument in @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'field-gui/OwnedUi.ps1'),'-ProcessId',[string]$primary.Id,'-WindowHandle',[string]$popup.ToInt64(),'-ExpectedPath',$exe,'-ExpectedStartUtc',$primary.StartTime.ToUniversalTime().ToString('o'),'-Action','Snapshot')){$info.ArgumentList.Add($argument)}
 $probe=[Diagnostics.Process]::Start($info);$output=$probe.StandardOutput.ReadToEndAsync();$errorOutput=$probe.StandardError.ReadToEndAsync()
 if(-not $probe.WaitForExit(15000)){$probe.Kill($true);$null=$probe.WaitForExit(3000);throw 'Owned UIA snapshot timed out.'}
 $text=$output.GetAwaiter().GetResult();$text|Set-Content -LiteralPath $snapshot
 if($probe.ExitCode-ne0){throw ('UIA failed: '+$errorOutput.GetAwaiter().GetResult())}
 $nodes=($text|ConvertFrom-Json).nodes
 $result.cases+=@{name='second-instance-popup';status='Pass';secondaryExitCode=$secondary.ExitCode;popup=$popup.ToInt64()}
 $result.uiaExactText=if($url-cin@($nodes|ForEach-Object name)){'Verified'}else{'Unverified: provider returned no exact URL; actual receiver-state fixture below is authoritative for payload delivery.'}
 $privateSettings.close_action='exit'
 $privateSettings|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $root 'ytTrayDL-data/settings.json')
 Wait-Condition { (Get-Content -LiteralPath (Join-Path $root 'ytTrayDL-data/yttraydl.log') -Raw).Contains('settings reloaded (changed outside the UI)') }
 $null=[RecoveryNative]::PostMessage($main,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
 if(-not $primary.WaitForExit(8000)){throw 'Primary failed normal WM_CLOSE exit.'}
 if($primary.ExitCode-ne0){throw ('Primary exit '+$primary.ExitCode)}
 $result.cases+=@{name='owned-normal-exit';status='Pass';exitCode=$primary.ExitCode}
 # The GUI provider can expose no descendants until operator activation. Verify
 # exact payload delivery independently through the same actual platform API.
 [Threading.Mutex]$remainingMutex=$null
 try {if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$remainingMutex)){throw 'Primary session identity still exists; fixture refused.'}}finally{if($remainingMutex){$remainingMutex.Dispose()}}
 $fixture=(Resolve-Path -LiteralPath $IntentFixturePath).Path
 $intentInfo=[Diagnostics.ProcessStartInfo]::new($fixture);$intentInfo.UseShellExecute=$false;$intentInfo.CreateNoWindow=$true;$intentInfo.WorkingDirectory=$root;$intentInfo.RedirectStandardOutput=$true;$intentInfo.RedirectStandardError=$true
 $probe.Dispose();$probe=[Diagnostics.Process]::Start($intentInfo);$output=$probe.StandardOutput.ReadToEndAsync();$errorOutput=$probe.StandardError.ReadToEndAsync()
 if(-not $probe.WaitForExit(10000)){$probe.Kill($true);$null=$probe.WaitForExit(3000);throw 'Exact intent fixture timed out.'}
 $intent=$output.GetAwaiter().GetResult();$intent|Set-Content -LiteralPath (Join-Path $root 'intent-state.json')
 if($probe.ExitCode-ne0){throw ('Exact intent fixture failed: '+$errorOutput.GetAwaiter().GetResult())}
 $intentState=$intent|ConvertFrom-Json
 if($intentState.status-ne'Pass'-or$intentState.exactUrl-cne$url-or$intentState.senderExitCode-ne0){throw 'Invalid exact intent fixture result.'}
 $result.cases+=@{name='exact-native-url-receiver-state';status='Pass';url=$intentState.exactUrl;exitCode=$probe.ExitCode}
 $result.status='Pass'
} catch {$result.error=$_.Exception.Message} finally {
 foreach($owned in @($probe,$secondary,$primary)){if($null-ne$owned){if(-not$owned.HasExited){$owned.Kill($true);if(-not$owned.WaitForExit(3000)){$result.error+=' Owned cleanup did not finish within deadline.'};$result.status='Failed';$result.error+=' Owned emergency cleanup was required.'};$owned.Dispose()}}
 if($createdRoot){$result|ConvertTo-Json -Depth 7|Set-Content -LiteralPath (Join-Path $Destination 'result.json')}
}
$result|ConvertTo-Json -Depth 7
if($result.status-eq'Pass'){exit 0}else{exit 2}
