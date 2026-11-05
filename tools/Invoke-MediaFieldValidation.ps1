#requires -Version 7.0
<#
.SYNOPSIS
Run the explicitly built real-media field harness with supplied trusted tools.
.NOTES
Does not build, install tools, use user settings, or publish. Root must be new.
Exit 2 denotes unavailable prerequisites; nonzero harness exits remain failures.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Harness,
    [Parameter(Mandatory)][string]$YtDlp,
    [Parameter(Mandatory)][string]$FFmpeg,
    [Parameter(Mandatory)][string]$FFprobe,
    [Parameter(Mandatory)][string]$Root,
    [ValidateSet('all','video','merge','audio','subtitles','playlist','retry','pause','cancel','shutdown')][string]$Scenario='all',
    [ValidateRange(10,120)][int]$TimeoutSeconds=120
)
$ErrorActionPreference='Stop'
$process=$null
$launched=$false
try {
    $paths=@($Harness,$YtDlp,$FFmpeg,$FFprobe)
    foreach($path in $paths) {
        if(-not [IO.Path]::IsPathFullyQualified($path) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) {throw "Unavailable executable: $path"}
    }
    if(-not [IO.Path]::IsPathFullyQualified($Root) -or (Test-Path -LiteralPath $Root)) {throw 'Root must be a new absolute directory.'}
    $identities=@($paths|ForEach-Object {Get-FileHash -LiteralPath $_ -Algorithm SHA256|Select-Object Path,Hash,Algorithm})
    $arguments=@('--yt-dlp',$YtDlp,'--ffmpeg',$FFmpeg,'--ffprobe',$FFprobe,'--root',$Root,'--scenario',$Scenario,'--timeout-seconds',"$TimeoutSeconds")
    $process=[Diagnostics.Process]::new()
    $process.StartInfo=[Diagnostics.ProcessStartInfo]::new($Harness)
    $process.StartInfo.UseShellExecute=$false
    $process.StartInfo.CreateNoWindow=$true
    $process.StartInfo.RedirectStandardOutput=$true
    $process.StartInfo.RedirectStandardError=$true
    foreach($argument in $arguments) {$process.StartInfo.ArgumentList.Add($argument)}
    $started=[DateTime]::UtcNow
    if(-not $process.Start()) {throw 'Harness did not start.'}
    $launched=$true
    $stdout=$process.StandardOutput.ReadToEndAsync()
    $stderr=$process.StandardError.ReadToEndAsync()
    # Each retry subcase is independently bounded; add fixture/tool setup time.
    $budgetSeconds=if($Scenario -eq 'all'){12*$TimeoutSeconds+120}else{3*$TimeoutSeconds+120}
    $timedOut=-not $process.WaitForExit($budgetSeconds*1000)
    if($timedOut) {$process.Kill($true);$process.WaitForExit()}
    $exitCode=if($timedOut){124}else{$process.ExitCode}
    if(-not (Test-Path -LiteralPath $Root -PathType Container)) {New-Item -ItemType Directory -Path $Root|Out-Null}
    $stdout.GetAwaiter().GetResult()|Set-Content -LiteralPath (Join-Path $Root 'stdout.log') -Encoding utf8
    $stderr.GetAwaiter().GetResult()|Set-Content -LiteralPath (Join-Path $Root 'stderr.log') -Encoding utf8
    $artifacts=@(Get-ChildItem -LiteralPath $Root -File -Recurse|Where-Object Name -ne 'invocation.json'|ForEach-Object {Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256|Select-Object Path,Hash,Algorithm})
    $commit=& git -C (Split-Path $PSScriptRoot -Parent) rev-parse HEAD
    if($LASTEXITCODE -ne 0){$commit=$null}
    [ordered]@{schemaVersion=1;startedUtc=$started.ToString('o');finishedUtc=[DateTime]::UtcNow.ToString('o');os=[Environment]::OSVersion.VersionString;sessionId=(Get-Process -Id $PID).SessionId;commit=$commit;harnessPid=$process.Id;args=$arguments;exitCode=$exitCode;timedOut=$timedOut;tools=$identities;artifacts=$artifacts}|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $Root 'invocation.json') -Encoding utf8
    Write-Output "Media field evidence: $Root; exit code: $exitCode"
    exit $exitCode
} catch {
    Write-Error -ErrorAction Continue $_
    exit 2
} finally {
    if($null -ne $process){
        if($launched -and -not $process.HasExited){
            $process.Kill($true)
            $process.WaitForExit()
        }
        $process.Dispose()
    }
}
