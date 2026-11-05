# Actual trusted installer execution is restricted to an ephemeral GitHub VM.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('clean','upgrade')][string]$Scenario,
    [Parameter(Mandatory)][string]$Candidate,
    [Parameter(Mandatory)][string]$CandidateExe,
    [Parameter(Mandatory)][string]$StateProbe,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [switch]$TrustIsolatedRunner
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows -or !$TrustIsolatedRunner -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Installer execution requires explicitly trusted GitHub-hosted Windows isolation.'
}
$Candidate = (Resolve-Path -LiteralPath $Candidate).Path
$CandidateExe = (Resolve-Path -LiteralPath $CandidateExe).Path
$StateProbe = (Resolve-Path -LiteralPath $StateProbe).Path
if (Test-Path -LiteralPath $EvidenceRoot) { throw 'EvidenceRoot must be new.' }
$null = New-Item -ItemType Directory -Path $EvidenceRoot
$EvidenceRoot = (Resolve-Path -LiteralPath $EvidenceRoot).Path
$install = Join-Path $env:LOCALAPPDATA 'Programs/ytTrayDL'
$exe = Join-Path $install 'yttraydl.exe'
$data = Join-Path $env:APPDATA 'ytTrayDL'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{F85DF45F-C581-47D6-8605-ADFF0A87D50A}_is1'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$shortcuts = @((Join-Path ([Environment]::GetFolderPath('Programs')) 'ytTrayDL.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'ytTrayDL.lnk'))
$events = [Collections.Generic.List[object]]::new()
function Assert([bool]$Condition, [string]$Message) {
    $events.Add(@{ assertion=$Message; passed=$Condition })
    if (!$Condition) { throw $Message }
}
function Run-Installer([string]$Path, [string[]]$Arguments, [string]$Phase) {
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    & $StateProbe --run-owned 300 $Path @Arguments *> (Join-Path $EvidenceRoot "$Phase-process.log")
    $code = $LASTEXITCODE
    $events.Add(@{ phase=$Phase; exitCode=$code; arguments=$Arguments; sha256=$hash; containment='production-owned-tree'; processEvidence="$Phase-process.log" })
    Assert ($code -eq 0) "$Phase exit code is zero"
}
function Install([string]$Path, [string]$Phase) {
    Run-Installer $Path @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=desktopicon,startup', ('/LOG="' + (Join-Path $EvidenceRoot "$Phase.log") + '"')) $Phase
}
function Hash-Data {
    $result = @{}
    foreach ($name in @('settings.json','history.json','download-archive.txt')) {
        $result[$name] = (Get-FileHash -LiteralPath (Join-Path $data $name) -Algorithm SHA256).Hash
    }
    return $result
}
function Check-Data($Expected) {
    $actual = Hash-Data
    foreach ($name in $Expected.Keys) { Assert ($actual[$name] -eq $Expected[$name]) "$name bytes preserved" }
    & $StateProbe --run-owned 30 $StateProbe *> (Join-Path $EvidenceRoot "state-probe-$($events.Count).log")
    Assert ($LASTEXITCODE -eq 0) 'actual library state loaders preserve sentinels'
}
$passed = $false
try {
    Assert (!(Test-Path $install) -and !(Test-Path $data) -and !(Test-Path $key)) 'fresh runner has no existing application state'
    foreach ($shortcut in $shortcuts) { Assert (!(Test-Path $shortcut)) 'shortcut absent before test' }
    $existingRun = Get-ItemProperty -LiteralPath $runKey -Name ytTrayDL -ErrorAction SilentlyContinue
    Assert ($null -eq $existingRun) 'no preexisting application startup value'
    $oldHash = $null
    if ($Scenario -eq 'upgrade') {
        $release = Invoke-RestMethod 'https://api.github.com/repos/SeanShaffer88/ytTrayDL/releases/tags/v0.6.1' -TimeoutSec 120
        $asset = @($release.assets | Where-Object name -eq 'ytTrayDL-Setup-v0.6.1-windows-x64.exe')
        $sums = @($release.assets | Where-Object name -eq 'SHA256SUMS.txt')
        Assert ($asset.Count -eq 1 -and $sums.Count -eq 1) 'published 0.6.1 installer and checksum assets exist'
        $old = Join-Path $EvidenceRoot $asset[0].name
        Invoke-WebRequest $asset[0].browser_download_url -OutFile $old -TimeoutSec 120
        $checksumPath = Join-Path $EvidenceRoot 'baseline-SHA256SUMS.txt'
        Invoke-WebRequest $sums[0].browser_download_url -OutFile $checksumPath -TimeoutSec 120
        $line = @(Get-Content $checksumPath | Where-Object { $_ -match ('^[0-9a-fA-F]{64}\s+\*?' + [regex]::Escape($asset[0].name) + '$') })
        Assert ($line.Count -eq 1) 'baseline checksum line is unambiguous'
        Assert ((Get-FileHash $old -Algorithm SHA256).Hash -eq $line[0].Substring(0,64)) 'baseline installer SHA256 verified'
        Install $old 'baseline-install'
        Assert ((Get-ItemProperty $key).DisplayVersion -eq '0.6.1') 'baseline registration version'
        $oldHash = (Get-FileHash $exe -Algorithm SHA256).Hash
    }
    $null = New-Item -ItemType Directory -Path $data -Force
    @{schema_version=1;monitoring_enabled=$false;filename_template='field-071-%(id)s.%(ext)s'} | ConvertTo-Json | Set-Content (Join-Path $data 'settings.json') -Encoding utf8NoBOM
    @{entries=@(@{finished_at_unix=1;url='https://example.invalid/field-071';title='field-071-preserved';status='completed';file=$null;error=$null;preset_label='Best quality'})} | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $data 'history.json') -Encoding utf8NoBOM
    [IO.File]::WriteAllText((Join-Path $data 'download-archive.txt'), "generic field-071`n")
    $hashes = Hash-Data
    Check-Data $hashes
    # Prove actual library assertions reject corrupted synthetic state, then
    # restore bytes even if the negative control itself unexpectedly succeeds.
    $settingsPath = Join-Path $data 'settings.json'
    $originalSettings = [IO.File]::ReadAllBytes($settingsPath)
    try {
        [IO.File]::WriteAllText($settingsPath, '{}')
        & $StateProbe --run-owned 30 $StateProbe *> (Join-Path $EvidenceRoot 'state-probe-negative.log')
        Assert ($LASTEXITCODE -eq 101) 'actual state probe rejects corrupted synthetic sentinel with assertion panic'
    } finally {
        [IO.File]::WriteAllBytes($settingsPath, $originalSettings)
    }
    Check-Data $hashes
    Install $Candidate 'candidate-install'
    Assert ((Get-FileHash $exe -Algorithm SHA256).Hash -eq (Get-FileHash $CandidateExe -Algorithm SHA256).Hash) 'installed executable matches candidate artifact'
    Assert ((Get-Item $exe).VersionInfo.ProductVersion -like '0.7.1*') 'candidate executable product version'
    $registration = Get-ItemProperty $key
    Assert ($registration.DisplayVersion -eq '0.7.1') 'same AppId registers candidate version'
    Assert ($registration.InstallLocation.TrimEnd('\') -eq $install.TrimEnd('\')) 'registration install path'
    Assert ((Get-ItemProperty $runKey).ytTrayDL -eq ('"' + $exe + '" --background')) 'owned startup command has exact quoted path and background argument'
    $shell = New-Object -ComObject WScript.Shell
    foreach ($shortcut in $shortcuts) {
        Assert (Test-Path $shortcut) 'requested shortcut exists'
        Assert ($shell.CreateShortcut($shortcut).TargetPath -eq $exe) 'shortcut targets candidate'
    }
    if ($Scenario -eq 'upgrade') { Assert ($oldHash -ne (Get-FileHash $exe).Hash) 'upgrade replaced old executable' }
    Check-Data $hashes
    $foreign = Join-Path $install 'user-downloads/original-user-media.txt'
    $null = New-Item -ItemType Directory -Path (Split-Path $foreign) -Force
    [IO.File]::WriteAllText($foreign, 'user-owned media sentinel')
    $foreignHash = (Get-FileHash $foreign).Hash
    Run-Installer (Join-Path $install 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="' + (Join-Path $EvidenceRoot 'uninstall.log') + '"')) 'uninstall'
    Assert (!(Test-Path $exe)) 'uninstall removed application executable'
    Assert (!(Test-Path $key)) 'uninstall removed application registration'
    foreach ($shortcut in $shortcuts) { Assert (!(Test-Path $shortcut)) 'uninstall removed owned shortcut' }
    Assert ($null -eq (Get-ItemProperty $runKey -Name ytTrayDL -ErrorAction SilentlyContinue)) 'uninstall removed owned startup value'
    Check-Data $hashes
    Assert (Test-Path $foreign) 'uninstall preserved user media inside application directory'
    Assert ((Get-FileHash $foreign).Hash -eq $foreignHash) 'preserved user media hash'
    $passed = $true
} finally {
    @{ scenario=$Scenario; passed=$passed; commit=$env:GITHUB_SHA; os=[Environment]::OSVersion.VersionString; runner=$env:RUNNER_ENVIRONMENT; events=$events.ToArray(); startupExecution='Not exercised; registry command only'; gui='Not exercised in headless installer job' } | ConvertTo-Json -Depth 9 | Set-Content (Join-Path $EvidenceRoot 'evidence.json')
}
