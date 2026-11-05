#requires -Version 7.0
<#
.SYNOPSIS
Runs one private original-content compact GUI visual fixture.
.NOTES
Exit0 requires native lifecycle/geometry and an explicit supplied manual Pass.
No download/install/control actions are sent. Operator inspects/scrolls, then Exits.
#>
[CmdletBinding()]param(
 [Parameter(Mandatory)][string]$AppPath,[Parameter(Mandatory)][string]$Destination,
 [ValidateSet('LongContent','MissingTools')][string]$Mode='LongContent',
 [string]$YtDlpPath,[string]$FfmpegPath,[switch]$AllowActions,[switch]$AllowTrustedExecution,[switch]$UseDefaultSize,
 [ValidateRange(30,600)][int]$TimeoutSeconds=300
)
$ErrorActionPreference='Stop';. (Join-Path $PSScriptRoot 'Cleanup.ps1')
$p=$null;$code=125;$r=[ordered]@{status='Unknown';scope='Original private fixture lifecycle/native geometry plus separately supplied manual review; no automatic visual inference';visualReview='Unknown';mode=$Mode;cases=@();cleanup=@();error=$null}
try{
 if(-not$AllowActions-or-not$AllowTrustedExecution){throw 'Explicit action and trusted-execution acknowledgements required'}
 if(Get-Process yttraydl -ErrorAction SilentlyContinue){throw 'Existing app; no signaling performed'}
 [Threading.Mutex]$m=$null;try{if([Threading.Mutex]::TryOpenExisting('Local\ytTrayDL_SingleInstance_v1',[ref]$m)){throw 'Existing single-instance identity; no signaling performed'}}finally{if($m){$m.Dispose()}}
 $dest=[IO.Path]::GetFullPath($Destination);if(Test-Path $dest){throw 'New destination required'};$data=Join-Path $dest 'ytTrayDL-data';$null=New-Item -ItemType Directory $data
 $source=(Resolve-Path -LiteralPath $AppPath).Path;$exe=Join-Path $dest 'yttraydl.exe';Copy-Item -LiteralPath $source -Destination $exe;Set-Content (Join-Path $dest 'portable.txt') ''
 $r.app=@{source=$source;snapshot=$exe;sha256=(Get-FileHash $exe).Hash;version=(Get-Item $exe).VersionInfo.FileVersion}
 if($Mode-eq'LongContent'){
  if(-not$YtDlpPath-or-not$FfmpegPath){throw 'LongContent requires explicit private trusted tool paths'}
  Copy-Item -LiteralPath (Resolve-Path $YtDlpPath).Path -Destination (Join-Path $dest 'yt-dlp.exe');Copy-Item -LiteralPath (Resolve-Path $FfmpegPath).Path -Destination (Join-Path $dest 'ffmpeg.exe')
 }
 $titles=@(for($i=1;$i-le12;$i++){'Original field notes on quiet gardens by the river - Part {0:00}' -f $i})
 $titles[8]='Original field notes - café, forêt, résumé and riverside walks - Part 09'
 $titles[9]='http://127.0.0.1:9/OriginalUnbrokenVideoName/'+('OriginalTitleSegment'*8)+'/Part10'
 $titles[10]=('Original Unicode café résumé - '*8)+'Part 11'
 $saved=@(for($i=1;$i-le12;$i++){@{url=('http://127.0.0.1:9/original-fixture/'+$i+'/'+('long-owned-segment-'*8));title=$titles[$i-1];saved_at_unix=1700000000+$i}})
 $files=Join-Path $data 'original-inert-files';$null=New-Item -ItemType Directory $files
 $history=@(for($i=1;$i-le8;$i++){$file=$null;if($i%2-eq0){$file=Join-Path $files ('original-fixture-{0:00}.txt'-f$i);Set-Content $file 'Original inert UI fixture only; not an actual downloaded media artifact.'};@{finished_at_unix=1700001000+$i;url=('http://127.0.0.1:9/original-history/'+$i+'/'+('original-segment-'*6));title=$titles[$i-1];status=$(if($i%3-eq0){'cancelled'}elseif($i%2){'failed'}else{'completed'});file=$file;error=$(if($i%2){'Original synthetic failure text; no network operation occurred. '+('detail-'*16)}else{$null});preset_label='Original fixture quality'}})
 $saved|ConvertTo-Json -Depth 6|Set-Content (Join-Path $data 'saved.json');@{entries=$history}|ConvertTo-Json -Depth 6|Set-Content (Join-Path $data 'history.json')
 $settings=@{monitoring_enabled=$false;tray_icon_visible=$true;close_action='exit';watch_interval_mins=0;subscriptions=@();start_with_windows=$false;download_folder=(Join-Path $dest 'downloads');notify_on_complete=$false;notify_on_error=$false;sound_on_finish=$false;open_folder_on_finish=$false;extra_args='--ignore-config --no-cache-dir';cookies_from_browser='';ytdlp_path='';ffmpeg_path=''}
 if($Mode-eq'MissingTools'){$settings.ytdlp_path=Join-Path $dest 'absent-yt-dlp.exe';$settings.ffmpeg_path=Join-Path $dest 'absent-ffmpeg.exe'}
 $settings|ConvertTo-Json -Depth 6|Set-Content (Join-Path $data 'settings.json')
 $r.originalFixtures=@{savedCount=$saved.Count;historyCount=$history.Count;realisticTitleLengths=@($titles[0..7]|ForEach-Object Length);distinguishingSuffixAfterCharacter=58;conditionalHistoryFiles='Private inert text fixtures only; no open-file actions';savedSha256=(Get-FileHash (Join-Path $data 'saved.json')).Hash;historySha256=(Get-FileHash (Join-Path $data 'history.json')).Hash;scope='Generated original text/loopback URLs; seeded statuses are not actual downloads; no real user data'}
 $launch=[Diagnostics.ProcessStartInfo]::new($exe);$launch.UseShellExecute=$false;$launch.CreateNoWindow=$true;$launch.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden;$launch.WorkingDirectory=$dest
 foreach($key in @('APPDATA','LOCALAPPDATA','USERPROFILE','HOME','XDG_CONFIG_HOME','PROGRAMDATA','ALLUSERSPROFILE','TEMP','TMP')){$private=Join-Path $dest ('environment/'+$key);$null=New-Item -ItemType Directory $private;$launch.Environment[$key]=$private}
 $emptyPath=Join-Path $dest 'environment/empty-path';$null=New-Item -ItemType Directory $emptyPath;$launch.Environment['PATH']=$emptyPath
 Add-Type -Path (Join-Path $PSScriptRoot 'Native.cs');$p=[Diagnostics.Process]::Start($launch);$start=$p.StartTime.ToUniversalTime();$r.pid=$p.Id
 $deadline=[DateTime]::UtcNow.AddSeconds(12);do{$main=@([FieldGuiNative]::Windows($p.Id)|Where-Object{[FieldGuiNative]::Title($_)-eq'ytTrayDL'-and[FieldGuiNative]::IsWindowVisible([IntPtr]$_)});if($main.Count-eq1){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow-lt$deadline)
 if($main.Count-ne1){throw 'Owned main window missing'};$h=$main[0];$before=[FieldGuiNative]::Metrics($h,$p.Id);$scale=$before[16]/96.0;$width=[int][Math]::Round(340*$scale)+$before[2]-$before[6];$height=[int][Math]::Round(420*$scale)+$before[3]-$before[7]
 if(-not$UseDefaultSize){[FieldGuiNative]::MoveResizeOwned($h,$p.Id,250,150,$width,$height)}
 Start-Sleep -Milliseconds 1000;$metrics=[FieldGuiNative]::Metrics($h,$p.Id);$requestedWidth=if($UseDefaultSize){600}else{340};$sizeOkay=$metrics[6]-eq[int][Math]::Round($requestedWidth*$scale)-and$metrics[7]-eq[int][Math]::Round(420*$scale)
 $r.cases+=@{name=$(if($UseDefaultSize){'actual-fresh-default-client-size'}else{'actual-minimum-client-size'});status=$(if($sizeOkay){'Pass'}else{'Fail'});requestedLogical=@($requestedWidth,420);actualMetrics=$metrics}
 $ready=@{schemaVersion=1;pid=$p.Id;startUtc=$start.ToString('o');path=$exe;hwnd=$h;title='ytTrayDL';metrics=$metrics;profile=$data;mode=$Mode;originalFixtures=$r.originalFixtures;review='Observe header/footer at top/middle/bottom; scroll to all original rows; do not download/install/open files or links; only explicitly targeted synthetic Saved Remove is permitted';exit='Actual footer Exit after review; or write review-done.json after CUA completion';donePath=(Join-Path $dest 'review-done.json')}
 $ready|ConvertTo-Json -Depth 8|Set-Content (Join-Path $dest 'visual-ready.json')
 $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds);do{$p.Refresh();if($p.HasExited){break};if(Test-Path (Join-Path $dest 'review-done.json')){[FieldGuiNative]::Close($h,$p.Id);break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow-lt$deadline)
 $exited=$p.WaitForExit(8000);$normal=$exited-and$p.ExitCode-eq0;$r.cases+=@{name='fixture-normal-exit';status=$(if($normal){'Pass'}else{'Fail'});exited=$exited;exitCode=$(if($exited){$p.ExitCode})}
 $manualPath=Join-Path $dest 'review-done.json'
 if(Test-Path -LiteralPath $manualPath){$r.manualReviewPath=$manualPath;$r.manualReview=Get-Content -LiteralPath $manualPath -Raw|ConvertFrom-Json;if($r.manualReview.status-in@('Pass','Fail')){$r.visualReview=$r.manualReview.status}}
 $r.status=if(-not$normal-or-not$sizeOkay){'Fail'}elseif($r.visualReview-eq'Pass'){'Pass'}elseif($r.visualReview-eq'Fail'){'Fail'}else{'Unknown'}
 $code=if($r.status-eq'Pass'){0}elseif($r.status-eq'Unknown'){3}else{2}
}catch{$r.error=$_.Exception.Message;$r.status='Blocked'}finally{
 if($p){$cleanup=Stop-FieldGuiResource -Process $p -Role 'CompactVisualFixture' -ExpectedPath $exe -ExpectedStartUtc $start -TimeoutMilliseconds 5000;$r.cleanup+=@($cleanup);if($cleanup.status-ne'Pass'){$r.status='Fail';$code=2}}
 $r.exitCode=$code;if($dest-and(Test-Path $dest)){$r|ConvertTo-Json -Depth 10|Set-Content (Join-Path $dest 'result.json')};$r|ConvertTo-Json -Depth 10
}
exit $code

