#requires -Version 7.0
<#
.SYNOPSIS
Measures native dispatch during bounded actions on an owned private GUI.
.NOTES
WM_NULL dispatch and message-driven input are measured, not physical input-to-photon.
Private profile, no clipboard/registry writes, no external downloads or installers.
#>
[CmdletBinding()]param(
 [Parameter(Mandatory)][string]$AppPath,
 [Parameter(Mandatory)][string]$ManagedBinSource,
 [Parameter(Mandatory)][string]$Destination,
 [switch]$AllowTrustedExecution,
 [switch]$FramesOnly,
 [ValidateRange(30,240)][int]$SampleCount=60)
$ErrorActionPreference='Stop'
$owned=$null;$created=$false
$result=[ordered]@{status='Failed';framesOnly=[bool]$FramesOnly;selectedAssertions=$(if($FramesOnly){@('Owned lifecycle','Native message dispatch','Frame segment timings','No typing assertion selected')}else{@('Owned lifecycle','Native message dispatch','Frame segment timings','Exact edited field')});cases=@();measurements=@();error=$null;limitations=@('WM_NULL measures native dispatch latency, not physical input-to-photon.','Typing/scroll/resize use owned HWND messages.','Frame metrics include immediate child rendering and presentation, but exclude root presentation.')}
try {
 if(-not$IsWindows-or-not$AllowTrustedExecution){throw 'Windows and explicit trusted execution required'}
 if(Get-Process yttraydl -ErrorAction SilentlyContinue){throw 'Existing unowned app; no signaling performed'}
 [Threading.Mutex]$mutex=$null;try{if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$mutex)){throw 'Existing single-instance mutex; test refused'}}finally{if($mutex){$mutex.Dispose()}}
 if(Test-Path -LiteralPath $Destination){throw 'Destination must be new'}
 $source=(Resolve-Path -LiteralPath $AppPath).Path;$binSource=(Resolve-Path -LiteralPath $ManagedBinSource).Path
 $null=New-Item -ItemType Directory -Path $Destination;$created=$true;$root=(Resolve-Path $Destination).Path
 $data=Join-Path $root 'ytTrayDL-data';$null=New-Item -ItemType Directory -Path $data
 Copy-Item -LiteralPath $binSource -Destination (Join-Path $data 'bin') -Recurse
 $exe=Join-Path $root 'yttraydl.exe';Copy-Item -LiteralPath $source -Destination $exe;Set-Content (Join-Path $root 'portable.txt') ''
 $settings=@{monitoring_enabled=$false;start_with_windows=$false;subscriptions=@();watch_interval_mins=0;close_action='exit';tray_icon_visible=$true;download_folder=(Join-Path $root ('downloads/'+('original-long-folder-name/'*4)));notify_on_complete=$false;notify_on_error=$false;notify_on_auto_start=$false;filename_template='original-benchmark-template';ytdlp_path='';ffmpeg_path=''}
 $settings|ConvertTo-Json|Set-Content (Join-Path $data 'settings.json') -Encoding utf8NoBOM
 $result.app=@{source=$source;sha256=(Get-FileHash $source).Hash;profile=$root}
 Add-Type -Path (Join-Path $PSScriptRoot 'field-gui/Native.cs')
 Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class ResponsivenessNative {
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint id);
 [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h,uint m,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr r);
 [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
 [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
 public static void Guard(long h,int expected){uint id;GetWindowThreadProcessId(new IntPtr(h),out id);if(id!=expected)throw new Exception("Owned HWND identity changed");}
 public static double Ping(long h,int pid){Guard(h,pid);IntPtr r;var clock=Stopwatch.StartNew();if(SendMessageTimeout(new IntPtr(h),0,IntPtr.Zero,IntPtr.Zero,2,1000,out r)==IntPtr.Zero)throw new Exception("WM_NULL timed out");return clock.Elapsed.TotalMilliseconds;}
 public static void Message(long h,int pid,uint message,long w,long l){Guard(h,pid);if(!PostMessage(new IntPtr(h),message,new IntPtr(w),new IntPtr(l)))throw new Exception("Owned message rejected");}
 public static void Size(long h,int pid,int x,int y,int width,int height){Guard(h,pid);if(!SetWindowPos(new IntPtr(h),IntPtr.Zero,x,y,width,height,0x14))throw new Exception("Owned resize failed");}
 public static void Focus(long h,int pid){Guard(h,pid);SetForegroundWindow(new IntPtr(h));}
}
'@
 function Window([string]$Title){@([FieldGuiNative]::Windows($owned.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq$Title})|Select-Object -First 1}
 function Wait-Window([string]$Title){$until=[DateTime]::UtcNow.AddSeconds(15);do{$h=Window $Title;if($h){return [long]$h};if($owned.HasExited){throw 'Owned app exited early'};Start-Sleep -Milliseconds 80}while([DateTime]::UtcNow-lt$until);throw ('Owned window absent: '+$Title)}
 function Sample([string]$Phase,[long]$Window,[scriptblock]$Action){
  $values=[Collections.Generic.List[double]]::new()
  for($i=0;$i-lt$SampleCount;$i++){if($Action){&$Action $i};$values.Add([ResponsivenessNative]::Ping($Window,$owned.Id));Start-Sleep -Milliseconds 12}
  $sorted=@($values|Sort-Object);$result.measurements+=@{phase=$Phase;count=$values.Count;p50Ms=$sorted[[int][Math]::Floor(($sorted.Count-1)*0.50)];p95Ms=$sorted[[int][Math]::Floor(($sorted.Count-1)*0.95)];maxMs=$sorted[-1];samplesMs=$values.ToArray()}
 }
 $info=[Diagnostics.ProcessStartInfo]::new($exe);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WorkingDirectory=$root
 foreach($key in @('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','TEMP','TMP')){$private=Join-Path $root ('environment/'+$key);$null=New-Item -ItemType Directory -Path $private;$info.Environment[$key]=$private}
 $info.Environment['PATH']='';$info.Environment['YTTRAYDL_FIELD_UI_REPORT']=(Join-Path $root 'frame-metrics.json')
 $owned=[Diagnostics.Process]::Start($info);$main=Wait-Window 'ytTrayDL'
 [ResponsivenessNative]::Size($main,$owned.Id,40,60,600,450)
 Start-Sleep -Milliseconds 400
 Sample 'main-only' $main $null
 $secondInfo=[Diagnostics.ProcessStartInfo]::new($exe);$secondInfo.UseShellExecute=$false;$secondInfo.CreateNoWindow=$true;$secondInfo.ArgumentList.Add('--show');$second=[Diagnostics.Process]::Start($secondInfo)
 if(-not$second.WaitForExit(5000)){$second.Kill($true);$null=$second.WaitForExit(3000);throw 'Second show request timed out'};$second.Dispose()
 $settingsWindow=Wait-Window 'ytTrayDL — Settings';[ResponsivenessNative]::Size($settingsWindow,$owned.Id,700,60,600,670)
 Sample 'settings-open' $settingsWindow $null
 [ResponsivenessNative]::Focus($settingsWindow,$owned.Id)
 # Message coordinates target the actual native client; independently recorded
 # typing-state assertion below prevents claiming editing if focus missed it.
 [ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x200,0,(55*65536+220))
 [ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x201,1,(55*65536+220));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x202,0,(55*65536+220))
 Start-Sleep -Milliseconds 250
 Sample 'settings-scroll' $settingsWindow {param($i);[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x200,0,(350*65536+300));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x20A,$(if($i%2){120L*65536}else{-120L*65536}),0)}
 Sample 'settings-resize' $settingsWindow {param($i);[ResponsivenessNative]::Size($settingsWindow,$owned.Id,700,60,(600+($i%2)*60),(670+($i%2)*20))}
 [ResponsivenessNative]::Size($settingsWindow,$owned.Id,700,60,600,670)
 if(-not$FramesOnly){
 # Return to Downloads tab/top and focus its first path field via owned messages.
 [ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x200,0,(55*65536+220));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x201,1,(55*65536+220));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x202,0,(55*65536+220))
 Start-Sleep -Milliseconds 200
 [ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x200,0,(130*65536+180));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x201,1,(130*65536+180));[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x202,0,(130*65536+180))
 $beforeSettings=Get-Content (Join-Path $data 'settings.json') -Raw|ConvertFrom-Json
 Sample 'settings-typing' $settingsWindow {param($i);if($i-lt40){[ResponsivenessNative]::Message($settingsWindow,$owned.Id,0x102,([int][char]'x'),0)}}
 $afterSettings=Get-Content (Join-Path $data 'settings.json') -Raw|ConvertFrom-Json
 $result.typingStateChanged=($afterSettings.download_folder-cne$beforeSettings.download_folder-and$afterSettings.download_folder.Contains(('x'*20)))-or($afterSettings.filename_template-cne$beforeSettings.filename_template-and$afterSettings.filename_template.Contains(('x'*20)))
 }else{$result.typingStateChanged=$null}
 $result.cases+=@{name='settings-open-scroll-resize-message-delivery';status='Pass';window=$settingsWindow;dpi=[FieldGuiNative]::GetDpiForWindow([IntPtr]$settingsWindow)}
 [FieldGuiNative]::Close($main,$owned.Id)
 if(-not$owned.WaitForExit(10000)){throw 'Owned normal exit timed out'}
 if($owned.ExitCode-ne0){throw ('Owned exit '+$owned.ExitCode)}
 $result.cases+=@{name='normal-exit';status='Pass';exitCode=$owned.ExitCode}
 if(-not(Test-Path (Join-Path $root 'frame-metrics.json'))){throw 'Field instrumented frame report missing'}
 if(-not$FramesOnly-and-not$result.typingStateChanged){throw 'Exact editable setting did not receive benchmark text; typing responsiveness remains unverified.'}
 $result.status='Pass'
}catch{$result.error=$_.Exception.Message}finally{
 if($owned){if(-not$owned.HasExited){$owned.Kill($true);$null=$owned.WaitForExit(5000);$result.status='Failed';$result.error+=' Emergency owned cleanup required.'};$owned.Dispose()}
 if($created){$result|ConvertTo-Json -Depth 8|Set-Content (Join-Path $root 'result.json')}
}
$result|Select-Object status,error,typingStateChanged,@{n='timings';e={$_.measurements|Select-Object phase,p50Ms,p95Ms,maxMs}}|ConvertTo-Json -Depth 5
if($result.status-eq'Pass'){exit 0}else{exit 2}
