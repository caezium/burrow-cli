# Stage a binary with its complete source and Cargo dependency notices.
#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$BinaryPath,
    [string]$OutputDirectory,
    [string]$SourceRoot = (Split-Path -Parent $PSScriptRoot)
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$SourceRoot = [IO.Path]::GetFullPath($SourceRoot)
if (-not $BinaryPath) { $BinaryPath = Join-Path $SourceRoot 'target/release/burrow.exe' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $SourceRoot 'target/artifacts/burrow-windows-x86_64' }

if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) { throw 'Windows release binary is missing' }
$documents = @('LICENSE.md', 'NOTICE', 'THIRD-PARTY-LICENSES.md')
foreach ($name in $documents) {
    $path = Join-Path $SourceRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) {
        throw "Required distribution notice is missing or empty: $name"
    }
}
$pythonCommand = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'python' } else { 'python3' }
& $pythonCommand (Join-Path $PSScriptRoot 'cargo-notices.py') --check --root $SourceRoot
if ($LASTEXITCODE -ne 0) { throw 'Cargo notice verification failed; no artifact was staged' }

# A failed rerun must not overwrite or clean a caller-owned directory.
if (Test-Path -LiteralPath $OutputDirectory) { throw "Output directory already exists: $OutputDirectory" }
New-Item -ItemType Directory -Path $OutputDirectory -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $OutputDirectory 'burrow.exe')
foreach ($name in $documents) {
    Copy-Item -LiteralPath (Join-Path $SourceRoot $name) -Destination (Join-Path $OutputDirectory $name)
}
Copy-Item -LiteralPath (Join-Path $SourceRoot 'LICENSES') -Destination (Join-Path $OutputDirectory 'LICENSES') -Recurse
Write-Output "Windows binary and complete Cargo notices staged: $OutputDirectory"
