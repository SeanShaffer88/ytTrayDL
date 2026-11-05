#requires -Version 7.0
function Confirm-FieldGuiFocus {
 param([Parameter(Mandatory)][Diagnostics.Process]$Process,[Parameter(Mandatory)][long]$Window,[Parameter(Mandatory)][string]$Stage,[Parameter(Mandatory)][string]$RequestPath,[switch]$AllowAssistance,[switch]$RequireOperatorAcknowledgement,[ValidateRange(1,180)][int]$TimeoutSeconds=180)
 $r=[ordered]@{schemaVersion=1;stage=$Stage;pid=$Process.Id;path=$Process.Path;startUtc=$Process.StartTime.ToUniversalTime().ToString('o');hwnd=$Window;title=[FieldGuiNative]::Title($Window);metrics=[FieldGuiNative]::Metrics($Window,$Process.Id);assisted=$false;foregroundVerified=$false;requestedUtc=$null;verifiedUtc=$null;status='Unknown';scope='Focus activation only; no business control action requested'}
 if([FieldGuiNative]::FocusOwned($Window,$Process.Id)-and-not$RequireOperatorAcknowledgement){$r.foregroundVerified=$true;$r.status='Pass';return [pscustomobject]$r}
 if(-not$AllowAssistance){throw 'Owned foreground activation unavailable; bounded explicit -AllowFocusHandoff is required for operator/CUA focus assistance'}
 if(Test-Path -LiteralPath $RequestPath){throw 'Focus handoff evidence path must be new'}
 $r.acknowledgementPath=$RequestPath+'.activated';$r.operatorAcknowledgementRequired=[bool]$RequireOperatorAcknowledgement;$r.assisted=$true;$r.requestedUtc=[DateTime]::UtcNow.ToString('o');$r.status='NeedsFocus';$r|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $RequestPath
 $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
 do{$Process.Refresh();if($Process.HasExited){throw 'Owned app exited during focus-only handoff'};if([FieldGuiNative]::IsForegroundOwned($Process.Id)-and(-not$RequireOperatorAcknowledgement-or(Test-Path -LiteralPath $r.acknowledgementPath))){$r.foregroundVerified=$true;$r.verifiedUtc=[DateTime]::UtcNow.ToString('o');$r.status='Pass';$r|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $RequestPath;return [pscustomobject]$r};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow-lt$deadline)
 $r.status='Blocked';$r|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $RequestPath;throw ('Owned focus assistance deadline exceeded: '+$RequestPath)
}

