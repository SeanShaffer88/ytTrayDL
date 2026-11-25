#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:RUNNER_TEMP "media-summary-test-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory -Path $root
try {
    $commit = 'a' * 40
    $invocation = @{ commit=$commit; exitCode=0; timedOut=$false; startedUtc='start'; finishedUtc='end'; os='test' }
    $names = @('video','merge','audio','subtitles','playlist','retry','pause','cancel','shutdown')
    $evidence = @{ status='passed'; coverage='test'; scenarios=@($names | ForEach-Object { @{scenario=$_; status='passed'; assertions=@{filename='PRIVATE-PATH'}} }) }
    $tool = @{ sha256=('b' * 64); version='synthetic' }
    $provenance = @{ tools=@{ ytdlp=$tool; ffmpeg=$tool; ffprobe=$tool } }
    $invocation | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $root 'invocation.json')
    $evidence | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $root 'evidence.json')
    $provenance | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $root 'provenance.json')
    $export = Join-Path $PSScriptRoot 'Export-ReleaseMediaEvidence.ps1'
    $output = Join-Path $root 'public.json'
    & $export -Root $root -Provenance (Join-Path $root 'provenance.json') -Output $output -Commit $commit
    $report = Get-Content -LiteralPath $output -Raw
    if ($report.Contains('PRIVATE-PATH')) { throw 'Private assertion leaked' }
    $parsed = $report | ConvertFrom-Json
    if ($parsed.status -ne 'passed' -or $parsed.scenarios.Count -ne 9 -or $parsed.commit -ne $commit) { throw 'Invalid summary' }
    $invocation.exitCode = 1
    $invocation | ConvertTo-Json | Set-Content (Join-Path $root 'invocation.json')
    $rejected = $false
    try { & $export -Root $root -Provenance (Join-Path $root 'provenance.json') -Output (Join-Path $root 'failed.json') -Commit $commit } catch { $rejected = $true }
    if (-not $rejected -or (Test-Path (Join-Path $root 'failed.json'))) { throw 'Failed run accepted' }
    $invocation.exitCode = 0
    $invocation | ConvertTo-Json | Set-Content (Join-Path $root 'invocation.json')
    $evidence.scenarios = @($evidence.scenarios | Select-Object -First 8)
    $evidence | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $root 'evidence.json')
    $rejected = $false
    try { & $export -Root $root -Provenance (Join-Path $root 'provenance.json') -Output (Join-Path $root 'missing.json') -Commit $commit } catch { $rejected = $true }
    if (-not $rejected -or (Test-Path (Join-Path $root 'missing.json'))) { throw 'Incomplete run accepted' }
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force
}
