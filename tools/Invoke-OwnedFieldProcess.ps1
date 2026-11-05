#requires -Version 7.0
<#
.SYNOPSIS
Run an explicitly trusted field helper through the application's owned tree runner.
.NOTES
Does not build or discover executables. EvidencePath must be a new absolute JSON
path in an existing directory. Returns evidence; callers must require passed=true.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ProcessRunner,
    [Parameter(Mandatory)][string]$Executable,
    [string[]]$Arguments=@(),
    [ValidateRange(1,300)][int]$TimeoutSeconds=30,
    [Parameter(Mandatory)][string]$EvidencePath
)
$ErrorActionPreference='Stop'
foreach($path in @($ProcessRunner,$Executable)) {
    if(-not [IO.Path]::IsPathFullyQualified($path) -or -not (Test-Path -LiteralPath $path -PathType Leaf)){throw "Explicit absolute executable required: $path"}
}
if(-not [IO.Path]::IsPathFullyQualified($EvidencePath) -or -not (Test-Path -LiteralPath (Split-Path $EvidencePath -Parent) -PathType Container)){throw 'EvidencePath requires an existing absolute parent directory.'}
foreach($path in @($EvidencePath,"$EvidencePath.stdout.log","$EvidencePath.stderr.log")){if(Test-Path -LiteralPath $path){throw "Evidence must be new: $path"}}
$clock=[Diagnostics.Stopwatch]::StartNew()
$r=[ordered]@{elapsedMilliseconds=$null;cleanupErrors=@();schemaVersion=1;startedUtc=[DateTime]::UtcNow.ToString('o');finishedUtc=$null;passed=$false;exitCode=125;timedOut=$false;outerTimedOut=$false;rootPid=$null;runnerPid=$null;cleanupVerified=$false;executable=$Executable;arguments=$Arguments;timeoutSeconds=$TimeoutSeconds;processRunner=$ProcessRunner;stdout='';stderr='';error=$null;evidencePath=$EvidencePath}
$p=[Diagnostics.Process]::new();$launched=$false;$stdoutTask=$null;$stderrTask=$null
try {
    $p.StartInfo=[Diagnostics.ProcessStartInfo]::new($ProcessRunner)
    $p.StartInfo.UseShellExecute=$false;$p.StartInfo.CreateNoWindow=$true
    $p.StartInfo.RedirectStandardOutput=$true;$p.StartInfo.RedirectStandardError=$true
    foreach($arg in @('--run-owned',"$TimeoutSeconds",$Executable)+$Arguments){$p.StartInfo.ArgumentList.Add($arg)}
    if(-not $p.Start()){throw 'Owned runner did not start.'}
    $launched=$true;$r.runnerPid=$p.Id
    $stdoutTask=$p.StandardOutput.ReadToEndAsync();$stderrTask=$p.StandardError.ReadToEndAsync()
    # Production cleanup has a five-second acknowledgement limit. This outer
    # watchdog is a last-resort failure path, never evidence of verified cleanup.
    if(-not $p.WaitForExit(($TimeoutSeconds+15)*1000)){
        $r.outerTimedOut=$true;$r.timedOut=$true;$r.exitCode=124
        $p.Kill($true)
        if(-not $p.WaitForExit(5000)){throw 'Owned runner failed to acknowledge watchdog termination.'}
    }else{$r.exitCode=$p.ExitCode;$r.timedOut=($p.ExitCode -eq 124)}
    if(-not [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($stdoutTask,$stderrTask),5000)){throw 'Owned runner output did not close within deadline.'}
    $r.stdout=$stdoutTask.GetAwaiter().GetResult();$r.stderr=$stderrTask.GetAwaiter().GetResult()
    if($r.stderr -match '(?m)^ownedRootPid=(\d+) '){$r.rootPid=[int]$Matches[1]}
    $r.cleanupVerified=(-not $r.outerTimedOut -and $null -ne $r.rootPid -and $r.stderr -match '(?m)^ownedTreeCleanup=verified(?: |\r?$)')
    $r.passed=($r.exitCode -eq 0 -and $r.cleanupVerified -and -not $r.timedOut)
    if(-not $r.passed){$r.error='Owned helper failed, timed out, or cleanup was not verified; inspect stderr.'}
}catch{$r.error=$_.Exception.Message;$r.passed=$false}finally{
    if($launched -and -not $p.HasExited){
        try{$p.Kill($true);if(-not $p.WaitForExit(5000)){throw 'Final owned-runner cleanup timed out.'}}catch{$r.cleanupErrors+= $_.Exception.Message;$r.error="$( $r.error ); cleanup: $($_.Exception.Message)"}
        $r.cleanupVerified=$false;$r.passed=$false
    }
    if($null -ne $stdoutTask -and $stdoutTask.IsCompletedSuccessfully){$r.stdout=$stdoutTask.GetAwaiter().GetResult()}
    if($null -ne $stderrTask -and $stderrTask.IsCompletedSuccessfully){$r.stderr=$stderrTask.GetAwaiter().GetResult()}
    $p.Dispose();$clock.Stop();$r.elapsedMilliseconds=$clock.ElapsedMilliseconds;$r.finishedUtc=[DateTime]::UtcNow.ToString('o')
    [IO.File]::WriteAllText("$EvidencePath.stdout.log",$r.stdout)
    [IO.File]::WriteAllText("$EvidencePath.stderr.log",$r.stderr)
    $r|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $EvidencePath -Encoding utf8NoBOM
}
[pscustomobject]$r
