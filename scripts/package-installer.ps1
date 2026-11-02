#requires -Version 5.1
<#
.SYNOPSIS
Create the NSIS Setup.exe from a validated package-release.ps1 output directory.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [string]$MakeNsis,
    [switch]$TestInstaller
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$manifest = Get-Content -LiteralPath (Join-Path $package 'release-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release version.' }
$zip = Join-Path $package 'PopChat-Windows-x64.zip'
if ((Get-FileHash -LiteralPath $zip).Hash -ne $manifest.sha256) { throw 'Release ZIP hash mismatch.' }
$stage = Join-Path $package 'staging'
if (-not $MakeNsis) {
    $command = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($command) { $MakeNsis = $command.Source }
    else { $MakeNsis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe' }
}
if (-not (Test-Path -LiteralPath $MakeNsis -PathType Leaf)) { throw 'NSIS 3 compiler missing. Install NSIS or pass -MakeNsis.' }
$files = @(Get-ChildItem -LiteralPath $stage -Recurse -File)
if ($files.Count -lt 6) { throw 'Incomplete release staging directory.' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    if ($archive.Entries.Count -ne $files.Count) { throw 'Staging/ZIP file count mismatch.' }
    foreach ($entry in $archive.Entries) {
        if ($entry.FullName -match '(^/|\\|:|(^|/)\.\.(/|$))') { throw 'Unsafe archive path.' }
        $path = Join-Path $stage $entry.FullName
        $source = $entry.Open()
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $digest = ([BitConverter]::ToString($sha.ComputeHash($source))).Replace('-','') }
        finally { $sha.Dispose(); $source.Dispose() }
        if ((Get-FileHash -LiteralPath $path).Hash -ne $digest) { throw "Staging differs from validated ZIP: $($entry.FullName)" }
    }
} finally { $archive.Dispose() }
foreach ($item in @(Get-Item -LiteralPath $stage) + @(Get-ChildItem -LiteralPath $stage -Recurse -Force)) {
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Staging links are forbidden.' }
}
$out = Join-Path $package ('nsis-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($out)
$suffix = if ($TestInstaller) { '-TEST' } else { '' }
$name = "PopChat-$($manifest.version)-Windows-x64-Setup$suffix.exe"
$setup = Join-Path $out $name
$utf8 = New-Object Text.UTF8Encoding($false)
function Escape-Nsis([string]$Value) {
    if ($Value -match '[\r\n]') { throw 'Newlines are forbidden in NSIS paths.' }
    return $Value.Replace('$','$$').Replace('"','$\"')
}
$install = New-Object 'Collections.Generic.List[string]'
$uninstall = New-Object 'Collections.Generic.List[string]'
$dirs = @{}
foreach ($file in ($files | Sort-Object FullName)) {
    $relative = $file.FullName.Substring($stage.Length + 1)
    $parent = Split-Path -Parent $relative
    $dest = if ($parent) { '\' + (Escape-Nsis $parent) } else { '' }
    $install.Add('SetOutPath "$INSTDIR' + $dest + '"')
    $install.Add('File "' + (Escape-Nsis $file.FullName) + '"')
    $uninstall.Add('Delete "$INSTDIR\' + (Escape-Nsis $relative) + '"')
    while ($parent) { $dirs[$parent] = $true; $parent = Split-Path -Parent $parent }
}
foreach ($dir in ($dirs.Keys | Sort-Object Length -Descending)) {
    $uninstall.Add('RMDir "$INSTDIR\' + (Escape-Nsis $dir) + '"')
}
$installFile = Join-Path $out 'install-files.nsh'
$uninstallFile = Join-Path $out 'uninstall-files.nsh'
[IO.File]::WriteAllLines($installFile, $install, $utf8)
[IO.File]::WriteAllLines($uninstallFile, $uninstall, $utf8)
$defs = [ordered]@{
    APP_VERSION = $manifest.version; SETUP_PATH = $setup; STAGE_DIR = $stage
    INSTALL_FILES = $installFile; UNINSTALL_FILES = $uninstallFile
    APP_SHA256 = (Get-FileHash -LiteralPath (Join-Path $stage 'popchat_windows.exe')).Hash.ToLowerInvariant()
    HELPER_SHA256 = (Get-FileHash -LiteralPath (Join-Path $stage 'popchat_updater.exe')).Hash.ToLowerInvariant()
    SIZE_KB = [string][Math]::Ceiling(($files | Measure-Object Length -Sum).Sum / 1KB)
}
$nsisLicense = Join-Path (Split-Path -Parent $MakeNsis) 'COPYING'
if (-not (Test-Path -LiteralPath $nsisLicense)) { throw 'NSIS compiler COPYRIGHT/COPYING notice missing.' }
$defs['NSIS_LICENSE'] = $nsisLicense
$config = @($defs.Keys | ForEach-Object { '!define ' + $_ + ' "' + (Escape-Nsis $defs[$_]) + '"' })
if ($TestInstaller) { $config += '!define TEST_INSTALLER' }
$include = Join-Path $out 'package.nsh'
[IO.File]::WriteAllLines($include, [string[]]$config, $utf8)
& $MakeNsis /INPUTCHARSET UTF8 /V3 /WX "/DPACKAGE_INCLUDE=$include" (Join-Path $repo 'packaging\windows\installer.nsi')
if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed.' }
$hash = (Get-FileHash -LiteralPath $setup).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $out 'SHA256SUMS-Setup.txt'), "$hash  $name`n", $utf8)
[IO.File]::WriteAllText((Join-Path $out 'installer-manifest.json'), (([ordered]@{
    version=$manifest.version; author='Evis Lai'; channel='LOCAL BUILD'; format='NSIS'
    assetname=$name; sha256=$hash; source_commit=$manifest.source_commit
    packaging_commit=((& git -C $repo rev-parse HEAD) | Out-String).Trim()
    source_dirty=$manifest.source_dirty; test_installer=[bool]$TestInstaller; signed=$false
} | ConvertTo-Json) + "`n"), $utf8)
Write-Output "NSIS Setup: $setup"
if ($env:GITHUB_OUTPUT) { "installer_dir=$out" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8 }
