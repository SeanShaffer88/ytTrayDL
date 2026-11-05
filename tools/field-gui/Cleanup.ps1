#requires -Version 7.0
function Stop-FieldGuiResource {
 [CmdletBinding()]param([Parameter(Mandatory)][Diagnostics.Process]$Process,[Parameter(Mandatory)][string]$Role,[Parameter(Mandatory)][string]$ExpectedPath,[Parameter(Mandatory)][DateTime]$ExpectedStartUtc,[ValidateRange(100,10000)][int]$TimeoutMilliseconds=5000,[switch]$InjectPrimaryFailure)
 $clock=[Diagnostics.Stopwatch]::StartNew();$r=[ordered]@{role=$Role;pid=$Process.Id;path=$ExpectedPath;expectedStartUtc=$ExpectedStartUtc.ToUniversalTime().ToString('o');status='Fail';primarySucceeded=$false;cleanupVerified=$false;error=$null;emergencyCleanupVerified=$false;emergencyError=$null;exitCode=$null;elapsedMilliseconds=$null;timeoutMilliseconds=$TimeoutMilliseconds}
 function Assert-Identity {
  $Process.Refresh();if($Process.HasExited){return}
  if($Process.MainModule.FileName-ne$ExpectedPath-or$Process.StartTime.ToUniversalTime().Ticks-ne$ExpectedStartUtc.ToUniversalTime().Ticks){throw 'Owned cleanup process identity changed; termination refused'}
 }
 try {
  Assert-Identity
  if($InjectPrimaryFailure){throw 'Controlled primary cleanup failure for acknowledged negative control'}
  if(-not$Process.HasExited){$Process.Kill($true);if(-not$Process.WaitForExit($TimeoutMilliseconds)){throw 'Owned resource cleanup acknowledgement deadline exceeded'}}
  $r.primarySucceeded=$true;$r.cleanupVerified=$Process.HasExited;$r.status=if($r.cleanupVerified){'Pass'}else{'Fail'}
 }catch{
  $r.error=$_.Exception.Message
  # A failed primary cleanup remains nonpassing even if emergency cleanup works.
  try{Assert-Identity;if(-not$Process.HasExited){$Process.Kill($true);if(-not$Process.WaitForExit($TimeoutMilliseconds)){throw 'Emergency owned cleanup acknowledgement deadline exceeded'}};$r.emergencyCleanupVerified=$Process.HasExited;$r.cleanupVerified=$r.emergencyCleanupVerified}catch{$r.emergencyError=$_.Exception.Message}
 }finally{
  try{if($Process.HasExited){$r.exitCode=$Process.ExitCode}}catch{$r.exitCodeUnavailable=$_.Exception.Message}
  $clock.Stop();$r.elapsedMilliseconds=$clock.ElapsedMilliseconds
 }
 [pscustomobject]$r
}
