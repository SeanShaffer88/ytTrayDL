#requires -Version 5.1
param([int]$ProcessId,[long]$WindowHandle,[string]$ExpectedPath,[string]$ExpectedStartUtc,[ValidateSet('Snapshot','Invoke','SetValue','TypeValue','InvokeManualDownload')][string]$Action='Snapshot',[string]$Name,[string]$Value,[switch]$AllowActions)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot 'Native.cs')
$r=[ordered]@{status='Unknown';method='Owned HWND UIAutomation';processId=$ProcessId;window=$WindowHandle;action=$Action;nodes=@();error=$null}
try {
 $p=Get-Process -Id $ProcessId
 if($p.Path -ne $ExpectedPath -or $p.StartTime.ToUniversalTime() -ne [DateTime]::Parse($ExpectedStartUtc).ToUniversalTime()){throw 'Process identity mismatch'}
 if($WindowHandle -notin [FieldGuiNative]::Windows($ProcessId)){throw 'HWND not owned'}
 $root=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$WindowHandle)
 [FieldGuiNative]::RepaintOwned($WindowHandle,$ProcessId);$nodes=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition);for($retry=0;$nodes.Count-eq0-and$retry-lt20;$retry++){Start-Sleep -Milliseconds 100;[FieldGuiNative]::RepaintOwned($WindowHandle,$ProcessId);$nodes=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
 if($nodes.Count -gt 2000){throw 'UI bound exceeded'}
 foreach($n in $nodes){$c=$n.Current;if($c.ProcessId -ne $ProcessId){throw 'UI descendant ownership mismatch'};$r.nodes+=@{name=$c.Name;type=$c.ControlType.ProgrammaticName;enabled=$c.IsEnabled;x=$c.BoundingRectangle.X;y=$c.BoundingRectangle.Y;width=$c.BoundingRectangle.Width;height=$c.BoundingRectangle.Height}}
 if($Action -ne 'Snapshot'){
  if(-not $AllowActions){throw 'AllowActions required'}
  if($Action -eq 'InvokeManualDownload'){
   $edits=@($nodes|Where-Object{$_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit});if($edits.Count -ne 1){throw 'Manual edit selector ambiguous'};$e=$edits[0].Current.BoundingRectangle
   $target=@($nodes|Where-Object{$c=$_.Current;$b=$c.BoundingRectangle;$c.ControlType -eq [Windows.Automation.ControlType]::Custom -and $c.IsEnabled -and $b.X -gt $e.Right -and [Math]::Abs($b.Y-$e.Y)-lt 8 -and $b.Width -gt 70 -and $b.Width-lt 150})
  }else{$target=@($nodes|Where-Object{if($Action -in @('SetValue','TypeValue')){$_.Current.ControlType-eq[Windows.Automation.ControlType]::Edit}else{$_.Current.Name-ceq$Name}})}
  if($target.Count -ne 1){throw ('Action selector ambiguous: '+$target.Count)};$n=$target[0]
  if($Action -in @('SetValue','TypeValue')){if($n.Current.IsPassword){throw 'Password refused'};$n.SetFocus();$b=$n.Current.BoundingRectangle;if($Action-eq'TypeValue'){[FieldGuiNative]::ClickOwnedPoint($WindowHandle,$ProcessId,[int]($b.X+$b.Width/2),[int]($b.Y+$b.Height/2));[FieldGuiNative]::TypeOwnedText($WindowHandle,$ProcessId,$Value);$r.method='Owned exact UIA Edit geometry click then HWND WM_CHAR transport'}else{$n.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Value)}}elseif($Action -eq 'InvokeManualDownload'){$b=$n.Current.BoundingRectangle;[FieldGuiNative]::ClickOwnedPoint($WindowHandle,$ProcessId,[int]($b.X+$b.Width/2),[int]($b.Y+$b.Height/2));$r.method='Owned UIA exact manual-download geometry plus guarded actual mouse click'}else{$n.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()}
 }
 $r.status='Pass'
}catch{$r.error=$_.Exception.Message}
$r|ConvertTo-Json -Depth 8 -Compress
if($r.status-eq'Pass'){exit 0}else{exit 3}



