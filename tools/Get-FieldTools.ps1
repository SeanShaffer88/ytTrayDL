[CmdletBinding()]
param([Parameter(Mandatory)][string]$Destination, [Parameter(Mandatory)][string]$ProcessRunner)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows) { throw 'The pinned field tool bundle targets Windows x64.' }
$ProcessRunner = (Resolve-Path -LiteralPath $ProcessRunner).Path
if (Test-Path -LiteralPath $Destination) { throw 'Destination must be new.' }
$null = New-Item -ItemType Directory -Path $Destination
$Destination = (Resolve-Path -LiteralPath $Destination).Path
# yt-dlp pin obtained from the upstream release API asset digest on 2026-11-04.
# ffmpeg pin refreshed from the version-addressed release API on 2026-11-21;
# the previous daily build expired under its publisher's retention policy.
# GitHub release metadata is mutable; downloaded bytes must match these hashes.
# These tools are test-only; no binaries are redistributed with the application.
$pins = @(
    @{name='yt-dlp.exe'; version='2026.10.26'; url='https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe'; sha256='f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'; license='https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/LICENSE'; provenance='https://api.github.com/repos/yt-dlp/yt-dlp/releases/tags/2026.10.26'},
    @{name='ffmpeg.zip'; version='9.1'; url='https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip'; sha256='b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd'; size=119467219; license='Retain archive LICENSE/readme and component notices; https://www.gyan.dev/ffmpeg/builds/'; provenance='https://api.github.com/repos/GyanD/codexffmpeg/releases/tags/9.1'}
)
foreach ($pin in $pins) {
    $file = Join-Path $Destination $pin.name
    Invoke-WebRequest -Uri $pin.url -OutFile $file -TimeoutSec 120
    if ($pin.ContainsKey('size') -and (Get-Item -LiteralPath $file).Length -ne $pin.size) { throw "Size mismatch: $($pin.name)" }
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $pin.sha256) { throw "SHA256 mismatch: $($pin.name)" }
}
Expand-Archive -LiteralPath (Join-Path $Destination 'ffmpeg.zip') -DestinationPath (Join-Path $Destination 'ffmpeg')
$tools = @{ ytdlp=(Join-Path $Destination 'yt-dlp.exe') }
foreach ($name in @('ffmpeg','ffprobe')) {
    $matches = @(Get-ChildItem -LiteralPath (Join-Path $Destination 'ffmpeg') -Filter "$name.exe" -Recurse -File)
    if ($matches.Count -ne 1) { throw "Expected one $name executable" }
    $tools[$name] = $matches[0].FullName
}
$versions = @{}
foreach ($name in $tools.Keys) {
    [string[]]$arguments = if ($name -eq 'ytdlp') { @('--ignore-config','--no-cache-dir','--version') } else { @('-version') }
    $output = & $ProcessRunner --run-owned 30 $tools[$name] @arguments 2> (Join-Path $Destination "$name-probe.stderr.log")
    if ($LASTEXITCODE -ne 0) { throw "$name version probe failed: $LASTEXITCODE" }
    $versions[$name] = @{path=$tools[$name];sha256=(Get-FileHash $tools[$name]).Hash;version=($output -join "`n")}
}
if ($versions.ytdlp.version -ne '2026.10.26' -or $versions.ffmpeg.version -notmatch '^ffmpeg version 9\.1(?:-|\s)' -or $versions.ffprobe.version -notmatch '^ffprobe version 9\.1(?:-|\s)') { throw 'Pinned tool version mismatch' }
@{pins=$pins;tools=$versions} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $Destination 'provenance.json')
if ($env:GITHUB_ENV) {
    @("FIELD_YTDLP=$($tools.ytdlp)","FIELD_FFMPEG=$($tools.ffmpeg)","FIELD_FFPROBE=$($tools.ffprobe)") | Add-Content $env:GITHUB_ENV
}
$tools
