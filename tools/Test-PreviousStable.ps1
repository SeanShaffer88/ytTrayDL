#requires -Version 7.0
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Select-PreviousStable.ps1')
$releases = @(
    @{tag_name='v0.9.0'; draft=$false; prerelease=$false},
    @{tag_name='v0.13.2'; draft=$false; prerelease=$false},
    @{tag_name='v0.13.1'; draft=$false; prerelease=$false},
    @{tag_name='v0.13.3'; draft=$false; prerelease=$false},
    @{tag_name='v0.13.1-rc'; draft=$false; prerelease=$true},
    @{tag_name='v0.13.1'; draft=$true; prerelease=$false},
    @{tag_name='v0.10.0'; draft=$false; prerelease=$false}
)
if ((Select-PreviousStable -Releases $releases -CandidateVersion '0.13.2') -ne '0.13.1') { throw 'Incorrect previous stable selection' }
if ((Select-PreviousStable -Releases $releases -CandidateVersion '0.13.1') -ne '0.10.0') { throw 'Numeric ordering or prerelease exclusion failed' }
$refused=$false
try { Select-PreviousStable -Releases $releases -CandidateVersion '0.1.0' } catch { $refused=$true }
if (-not $refused) { throw 'Missing previous stable was accepted' }
