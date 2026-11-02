#requires -Version 5.1
[CmdletBinding()]
param([string]$Destination = (Join-Path $env:TEMP 'popchat-nsis-3.11'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$zip = "$Destination.zip"
$expected = 'c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1'
if (-not (Test-Path -LiteralPath $zip)) {
    & curl.exe --fail --location --silent --show-error --max-time 180 --proto '=https' --proto-redir '=https' --output $zip 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.11/nsis-3.11.zip'
    if ($LASTEXITCODE -ne 0) { throw 'NSIS download failed.' }
}
if ((Get-FileHash -LiteralPath $zip).Hash -ne $expected) { throw 'NSIS compiler archive SHA-256 mismatch.' }
$compiler = Join-Path $Destination 'nsis-3.11\makensis.exe'
if (-not (Test-Path -LiteralPath $compiler)) { Expand-Archive -LiteralPath $zip -DestinationPath $Destination }
if (-not (Test-Path -LiteralPath $compiler)) { throw 'NSIS compiler not found after extraction.' }
Write-Output "NSIS compiler: $compiler"
if ($env:GITHUB_OUTPUT) { "compiler=$compiler" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8 }
