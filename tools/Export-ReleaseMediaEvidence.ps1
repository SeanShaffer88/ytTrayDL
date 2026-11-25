#requires -Version 7.0
# Export only reviewed, non-path fields from the synthetic hosted field run.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Provenance,
    [Parameter(Mandatory)][string]$Output,
    [Parameter(Mandatory)][string]$Commit
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Commit -notmatch '^[0-9a-f]{40}$') { throw 'Invalid commit identity' }
if (Test-Path -LiteralPath $Output) { throw 'Output must be new' }
$invocation = Get-Content -LiteralPath (Join-Path $Root 'invocation.json') -Raw | ConvertFrom-Json
$evidence = Get-Content -LiteralPath (Join-Path $Root 'evidence.json') -Raw | ConvertFrom-Json
$provenanceData = Get-Content -LiteralPath $Provenance -Raw | ConvertFrom-Json
if ($invocation.commit -ne $Commit -or $invocation.exitCode -ne 0 -or $invocation.timedOut -or $evidence.status -ne 'passed') { throw 'Media run did not pass for this commit' }
$expected = @('video','merge','audio','subtitles','playlist','retry','pause','cancel','shutdown')
$actual = @($evidence.scenarios | ForEach-Object { $_.scenario })
if ($actual.Count -ne $expected.Count -or (Compare-Object ($actual | Sort-Object) ($expected | Sort-Object))) { throw 'Unexpected media scenario inventory' }
if (@($evidence.scenarios | Where-Object { $_.status -ne 'passed' }).Count -ne 0) { throw 'Media scenario failed' }
$tools = [ordered]@{}
foreach ($name in @('ytdlp','ffmpeg','ffprobe')) {
    $tool = $provenanceData.tools.$name
    if ($tool.sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "Invalid $name hash" }
    $tools[$name] = @{ sha256 = $tool.sha256.ToLowerInvariant(); version = ($tool.version -split "`n")[0] }
}
$report = [ordered]@{
    schemaVersion = 1
    commit = $Commit
    startedUtc = $invocation.startedUtc
    finishedUtc = $invocation.finishedUtc
    operatingSystem = $invocation.os
    coverage = $evidence.coverage
    status = 'passed'
    scenarios = @($evidence.scenarios | ForEach-Object { @{ name = $_.scenario; status = $_.status } })
    tools = $tools
    rawEvidenceSha256 = (Get-FileHash -LiteralPath (Join-Path $Root 'evidence.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    note = 'Synthetic loopback media on hosted Windows; not interactive GUI, external service discovery, or physical GPU verification.'
}
[IO.File]::WriteAllText([IO.Path]::GetFullPath($Output), ($report | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
