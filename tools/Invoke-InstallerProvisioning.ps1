# Real installer execution is confined to disposable GitHub-hosted Windows VMs.
[CmdletBinding()]
param([Parameter(Mandatory)][ValidateSet('latest','upgrade-latest','offline-failure','skip','existing-both','existing-yt-only')][string]$Scenario,
      [Parameter(Mandatory)][string]$Candidate,
      [Parameter(Mandatory)][string]$CandidateExe,
      [Parameter(Mandatory)][string]$StateProbe,
      [Parameter(Mandatory)][string]$EvidenceRoot,
      [switch]$TrustIsolatedRunner)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if(-not $IsWindows -or -not $TrustIsolatedRunner -or $env:GITHUB_ACTIONS-ne'true' -or $env:RUNNER_ENVIRONMENT-ne'github-hosted'){throw 'Installer execution requires trusted disposable GitHub-hosted Windows isolation.'}
$Candidate=(Resolve-Path -LiteralPath $Candidate).Path
$CandidateExe=(Resolve-Path -LiteralPath $CandidateExe).Path
$StateProbe=(Resolve-Path -LiteralPath $StateProbe).Path
if(Test-Path -LiteralPath $EvidenceRoot){throw 'EvidenceRoot must be new.'}
$null=New-Item -ItemType Directory -Path $EvidenceRoot
$EvidenceRoot=(Resolve-Path -LiteralPath $EvidenceRoot).Path
$install=Join-Path $env:LOCALAPPDATA 'Programs/ytTrayDL'
$data=Join-Path $env:APPDATA 'ytTrayDL'
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{F85DF45F-C581-47D6-8605-ADFF0A87D50A}_is1'
$events=[Collections.Generic.List[object]]::new()
$passed=$false
function Assert([bool]$Condition,[string]$Message){$events.Add(@{assertion=$Message;passed=$Condition});if(-not$Condition){throw $Message}}
function Run-Owned([string]$Path,[string[]]$Arguments,[string]$Phase){
 & $StateProbe --run-owned 1800 $Path @Arguments *> (Join-Path $EvidenceRoot ($Phase+'-process.log'))
 $code=$LASTEXITCODE
 $events.Add(@{phase=$Phase;exitCode=$code;arguments=$Arguments;sha256=(Get-FileHash -LiteralPath $Path).Hash})
 return $code
}
try {
 Assert (-not(Test-Path $install)-and-not(Test-Path $data)-and-not(Test-Path $key)) 'disposable runner starts without app/state/registration'
 if($Scenario-eq'upgrade-latest'){
  $release=Invoke-RestMethod 'https://api.github.com/repos/SeanShaffer88/ytTrayDL/releases/tags/v0.7.0' -TimeoutSec 120
  $asset=@($release.assets|Where-Object name -eq 'ytTrayDL-Setup-v0.7.0-windows-x64.exe')
  $sums=@($release.assets|Where-Object name -eq 'SHA256SUMS.txt')
  Assert ($asset.Count-eq1-and$sums.Count-eq1) 'published upgrade baseline has installer and checksums'
  $old=Join-Path $EvidenceRoot $asset[0].name
  Invoke-WebRequest $asset[0].browser_download_url -OutFile $old -TimeoutSec 120
  $sumPath=Join-Path $EvidenceRoot 'baseline-SHA256SUMS.txt'
  Invoke-WebRequest $sums[0].browser_download_url -OutFile $sumPath -TimeoutSec 120
  $line=@(Get-Content $sumPath|Where-Object{$_-match('^[0-9a-fA-F]{64}\s+\*?'+[regex]::Escape($asset[0].name)+'$')})
  Assert ($line.Count-eq1-and(Get-FileHash -LiteralPath $old).Hash-eq$line[0].Substring(0,64)) 'baseline installer verified before execution'
  $baseline=Run-Owned $old @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=','/TOOLSDOWNLOAD=skip') 'baseline'
  Assert ($baseline-eq0-and(Test-Path $key)) 'actual published baseline installed'
 }
 $null=New-Item -ItemType Directory -Path (Join-Path $data 'bin')
 $settings=Join-Path $data 'settings.json'
 [IO.File]::WriteAllText($settings,'{"monitoring_enabled":false,"filename_template":"preserved-installer-sentinel"}')
 if($Scenario-in@('existing-both','existing-yt-only')){
  # Inert local files prove detection without executing or trusting user tools.
  $ytExisting=Join-Path $data 'existing-yt-dlp.exe';Copy-Item -LiteralPath $StateProbe -Destination $ytExisting
  $fixture=@{monitoring_enabled=$false;filename_template='preserved-installer-sentinel';ytdlp_path=$ytExisting}
  if($Scenario-eq'existing-both'){$ffExisting=Join-Path $data 'existing-ffmpeg.exe';Copy-Item -LiteralPath $StateProbe -Destination $ffExisting;$fixture.ffmpeg_path=$ffExisting}
  $fixture|ConvertTo-Json|Set-Content -LiteralPath $settings
 }
 $settingsHash=(Get-FileHash -LiteralPath $settings).Hash
 $legacy=Join-Path $data 'bin/user-owned-tool.txt'
 [IO.File]::WriteAllText($legacy,'existing user tool sentinel')
 $legacyHash=(Get-FileHash -LiteralPath $legacy).Hash
 $manifest=Join-Path $data 'bin/current-tools.json'
 if($Scenario-in@('offline-failure','skip')){
  # Inert original bytes exercise pointer/file preservation, not media behavior.
  $prior=Join-Path $data 'bin/generations/generation-prior-fixture';$null=New-Item -ItemType Directory -Path $prior
  $priorTool=Join-Path $prior 'yt-dlp.exe';Copy-Item -LiteralPath $StateProbe -Destination $priorTool
  $priorHash=(Get-FileHash -LiteralPath $priorTool).Hash
  @{schemaVersion=1;tools=@(@{name='yt-dlp';version='prior-fixture';path='generations/generation-prior-fixture/yt-dlp.exe';sha256=$priorHash;sourceUrl='https://example.invalid/inert-fixture'})}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $manifest
  $priorManifestHash=(Get-FileHash -LiteralPath $manifest).Hash
 }
 # Do not let tools preinstalled on the hosted runner alter scenario outcomes.
 $env:PATH="$env:SystemRoot\System32;$env:SystemRoot"
 if($Scenario-notlike'*latest'){
  # Exercise real production download failures without a test-only transport.
  # Closed loopback proxy makes curl fail immediately; skip must never invoke it.
  $env:HTTPS_PROXY='http://127.0.0.1:1';$env:HTTP_PROXY=$env:HTTPS_PROXY;$env:ALL_PROXY=$env:HTTPS_PROXY;$env:NO_PROXY=''
 }
 $mode=if($Scenario-eq'skip'){'skip'}else{'required'}
 $installerArgs=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=',('/LOG='+(Join-Path $EvidenceRoot 'install.log')))
 # The clean live case deliberately omits the parameter to test the default.
 if($Scenario-ne'latest'){$installerArgs+=('/TOOLSDOWNLOAD='+$mode)}
 $code=Run-Owned $Candidate $installerArgs 'install'
 if($Scenario-in@('offline-failure','existing-yt-only')){
  Assert ($code-ne0) 'required download failure returns nonzero'
  Assert (Select-String -LiteralPath (Join-Path $EvidenceRoot 'install.log') -Pattern 'curl: \(7\)|Failed to connect|Could not connect' -Quiet) 'required failure reached real network transport through closed proxy'
  Assert (-not(Test-Path $install)-and-not(Test-Path $key)) 'failed prerequisite does not install or register app'
 }else{
  Assert ($code-eq0) 'installer succeeded'
  Assert ((Get-FileHash -LiteralPath (Join-Path $install 'yttraydl.exe')).Hash-eq(Get-FileHash -LiteralPath $CandidateExe).Hash) 'installed app matches candidate'
  Assert ((Get-FileHash -LiteralPath (Join-Path $install 'THIRD-PARTY-NOTICES.txt')).Hash-eq(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot '../THIRD-PARTY-NOTICES.txt')).Hash) 'installed dependency notices match reviewed source'
  Assert (Test-Path $key) 'candidate registered'
  Assert (@(Get-Process -Name yttraydl -ErrorAction SilentlyContinue).Count-eq0) 'silent install did not launch app'
  if($Scenario-eq'existing-both'){
   Assert (-not(Test-Path -LiteralPath $manifest)) 'existing tools reused without managed downloads'
   Assert (Select-String -LiteralPath (Join-Path $EvidenceRoot 'install.log') -Pattern 'Media tool provisioning succeeded' -Quiet) 'existing tools satisfy setup with blocked network'
   Assert ((Get-FileHash -LiteralPath $ytExisting).Hash-eq(Get-FileHash -LiteralPath $StateProbe).Hash) 'existing yt-dlp bytes unchanged'
   Assert ((Get-FileHash -LiteralPath $ffExisting).Hash-eq(Get-FileHash -LiteralPath $StateProbe).Hash) 'existing ffmpeg bytes unchanged'
  }elseif($Scenario-eq'skip'){
   Assert ((Get-FileHash -LiteralPath $manifest).Hash-eq$priorManifestHash) 'offline skip leaves managed pointer unchanged'
   Assert (-not(Select-String -LiteralPath (Join-Path $EvidenceRoot 'install.log') -Pattern 'Media tool provisioning succeeded' -Quiet)) 'offline skip did not provision tools'
  }else{
   # Inspect the installer's result immediately; a second provisioning call
   # could hide an installer integration failure and is deliberately avoided.
   Assert (Test-Path -LiteralPath $manifest) 'installer created canonical managed pointer'
   $tools=Get-Content -LiteralPath $manifest -Raw|ConvertFrom-Json
   Assert ($tools.schemaVersion-eq1-and$tools.tools.Count-eq3) 'installer provisioned yt-dlp, ffmpeg, and ffprobe'
   Copy-Item -LiteralPath $manifest -Destination (Join-Path $EvidenceRoot 'installed-tools.json')
   foreach($tool in $tools.tools){
    $toolPath=[IO.Path]::GetFullPath((Join-Path (Join-Path $data 'bin') $tool.path))
    Assert ($toolPath.StartsWith(([IO.Path]::GetFullPath((Join-Path $data 'bin/generations'))+[IO.Path]::DirectorySeparatorChar),[StringComparison]::OrdinalIgnoreCase)) 'tool path stays inside managed generations'
    Assert (Test-Path -LiteralPath $toolPath -PathType Leaf) ('reported tool exists: '+$tool.name)
    Assert ((Get-FileHash -LiteralPath $toolPath).Hash-eq$tool.sha256) ('reported tool hash matches: '+$tool.name)
    $tool.path=$toolPath
   }
   $toolSnapshots=@($tools.tools|ForEach-Object{@{path=$_.path;sha256=$_.sha256}})
   Assert ((Run-Owned $StateProbe @('--tool-status') 'actual-tool-resolvers')-eq0) 'actual application resolvers consume installer-produced managed tools'
   if($Scenario-eq'latest'){
    # A subsequent upgrade must work with blocked network and leave the exact
    # managed pointer and tool generations untouched, not silently replace them.
    $manifestBefore=(Get-FileHash -LiteralPath $manifest).Hash
    $env:HTTPS_PROXY='http://127.0.0.1:1';$env:HTTP_PROXY=$env:HTTPS_PROXY;$env:ALL_PROXY=$env:HTTPS_PROXY;$env:NO_PROXY=''
    $againArgs=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=',('/LOG='+(Join-Path $EvidenceRoot 'reinstall.log')))
    Assert ((Run-Owned $Candidate $againArgs 'offline-reinstall')-eq0) 'reinstall reuses managed tools without network'
    Assert ((Get-FileHash -LiteralPath $manifest).Hash-eq$manifestBefore) 'reinstall preserves managed pointer bytes'
    foreach($tool in $toolSnapshots){Assert ((Get-FileHash -LiteralPath $tool.path).Hash-eq$tool.sha256) 'reinstall preserves managed tool bytes'}
   }
  }
  $uninstall=Run-Owned (Join-Path $install 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG='+(Join-Path $EvidenceRoot 'uninstall.log'))) 'uninstall'
  Assert ($uninstall-eq0) 'uninstall succeeded'
  Assert (-not(Test-Path (Join-Path $install 'yttraydl.exe'))-and-not(Test-Path $key)) 'uninstall removed owned app and registration'
  Assert (-not(Test-Path (Join-Path $install 'THIRD-PARTY-NOTICES.txt'))) 'uninstall removed owned dependency notices'
  if($Scenario-like'*latest'){
   foreach($tool in $toolSnapshots){Assert ((Get-FileHash -LiteralPath $tool.path).Hash-eq$tool.sha256) 'uninstall preserved managed tool bytes'}
   Assert ((Run-Owned $StateProbe @('--tool-status') 'post-uninstall-tool-resolvers')-eq0) 'actual managed resolver still works after uninstall'
  }
 }
 if($Scenario-in@('offline-failure','skip')){
  Assert ((Get-FileHash -LiteralPath $manifest).Hash-eq$priorManifestHash) 'existing managed manifest bytes preserved'
  Assert ((Get-FileHash -LiteralPath $priorTool).Hash-eq$priorHash) 'existing managed tool fixture bytes preserved'
 }
 Assert ((Get-FileHash -LiteralPath $settings).Hash-eq$settingsHash) 'settings bytes preserved'
 Assert ((Get-FileHash -LiteralPath $legacy).Hash-eq$legacyHash) 'preexisting tool sentinel bytes preserved'
 $passed=$true
}finally{
 @{scenario=$Scenario;passed=$passed;commit=$env:GITHUB_SHA;events=$events.ToArray();scope='Actual silent installer on disposable hosted Windows; interactive Retry/Skip/Cancel UI not exercised.'}|ConvertTo-Json -Depth 9|Set-Content -LiteralPath (Join-Path $EvidenceRoot 'evidence.json')
}
# The offline scenario intentionally observes a nonzero native installer exit.
# Successful assertions must not leak that expected code into GitHub's wrapper.
if($passed){exit 0}
