[CmdletBinding()]
param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Destination) { throw 'Destination must be new.' }
$null = New-Item -ItemType Directory -Path $Destination
$Destination = (Resolve-Path -LiteralPath $Destination).Path
$messages = & cargo build --lib --locked --message-format=json
if ($LASTEXITCODE -ne 0) { throw "Library build failed: $LASTEXITCODE" }
$records = @($messages | ForEach-Object { $_ | ConvertFrom-Json })
$libraries = @($records | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'yttraydl' -and $_.target.kind -contains 'lib' } | ForEach-Object { $_.filenames | Where-Object { $_ -like '*.rlib' } })
if ($libraries.Count -ne 1) { throw 'Cannot identify unique actual yttraydl library artifact.' }
$libraryDirectory = Split-Path $libraries[0]
$linkArguments = @('-L', "dependency=$libraryDirectory", '-L', "dependency=$(Join-Path $libraryDirectory 'deps')")
# Newer Cargo build-dir layouts give every crate its own output directory
# (target/<profile>/build/<pkg>/<hash>/out) instead of one shared deps/.
# Proc-macro crates (e.g. serde_derive) are dynamic libraries, so take every
# artifact's directory rather than only .rlib/.rmeta ones.
$crateDirectories = @($records | Where-Object reason -eq 'compiler-artifact' | ForEach-Object { $_.filenames } | ForEach-Object { Split-Path $_ } | Sort-Object -Unique)
foreach ($directory in $crateDirectories) {
    $linkArguments += @('-L', "dependency=$directory")
}
foreach ($path in @($records | Where-Object reason -eq 'build-script-executed' | ForEach-Object linked_paths | Sort-Object -Unique)) {
    $linkArguments += @('-L', $path)
}
& rustc --edition=2021 (Join-Path $PSScriptRoot 'InstallerStateProbe.rs') --extern "yttraydl=$($libraries[0])" @linkArguments -o (Join-Path $Destination 'InstallerStateProbe.exe')
if ($LASTEXITCODE -ne 0) { throw "State probe compile failed: $LASTEXITCODE" }
