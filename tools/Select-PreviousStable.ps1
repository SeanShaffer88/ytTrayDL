#requires -Version 7.0
# Pure selector: callers supply published release metadata; no network here.
function Select-PreviousStable {
    param([Parameter(Mandatory)][object[]]$Releases, [Parameter(Mandatory)][string]$CandidateVersion)
    if ($CandidateVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid candidate version' }
    $candidate = [version]$CandidateVersion
    $best = $null
    foreach ($release in $Releases) {
        if ($release.draft -or $release.prerelease -or $release.tag_name -notmatch '^v\d+\.\d+\.\d+$') { continue }
        try { $version = [version]($release.tag_name.Substring(1)) } catch { continue }
        if ($version -lt $candidate -and ($null -eq $best -or $version -gt $best)) { $best = $version }
    }
    if ($null -eq $best) { throw 'No previous stable release among supplied metadata' }
    return $best.ToString()
}
