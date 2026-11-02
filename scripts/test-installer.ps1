#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Installer,
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [switch]$TestInstaller
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $TestInstaller -and $env:GITHUB_ACTIONS -ne 'true') {
    throw 'Production installer smoke tests are only allowed on an empty CI runner. Use -TestInstaller locally.'
}
$name = if ($TestInstaller) { 'PopChat Installer Test' } else { 'PopChat' }
$keyName = if ($TestInstaller) { 'PopChatWindowsInstallerTest' } else { 'PopChatWindows' }
$reg = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$keyName"
$shell = New-Object -ComObject WScript.Shell
$shortcuts = @(
    (Join-Path ([Environment]::GetFolderPath('StartMenu')) "Programs\$name.lnk"),
    (Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) "$name.lnk")
)
if (Test-Path -LiteralPath $reg) { throw 'Refusing to overwrite existing installation registration.' }
foreach ($shortcut in $shortcuts) { if (Test-Path -LiteralPath $shortcut) { throw 'Refusing to overwrite existing shortcut.' } }
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$metadata = Get-Content -LiteralPath (Join-Path (Split-Path -Parent $installerPath) 'installer-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ([bool]$metadata.test_installer -ne [bool]$TestInstaller -or
    (Get-FileHash -LiteralPath $installerPath).Hash -ne $metadata.sha256) {
    throw 'Installer checksum or test/production scope mismatch; nothing installed.'
}
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$release = Get-Content -LiteralPath (Join-Path $package 'release-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
# Construct Unicode explicitly so Windows PowerShell 5.1 need not infer UTF-8.
$leaf = 'PopChat NSIS ' + [char]0x6E2C + [char]0x8A66 + ' ' + [Guid]::NewGuid().ToString('N')
$dir = Join-Path ([IO.Path]::GetTempPath()) $leaf
[void][IO.Directory]::CreateDirectory($dir)
$sentinel = Join-Path $dir 'user-file-do-not-delete.txt'
[IO.File]::WriteAllText($sentinel, 'preserve unknown files')
$beforeApps = @(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'" | ForEach-Object ProcessId) -join ','
for ($pass = 0; $pass -lt 2; $pass++) {
    # /D must be last, without quotes (NSIS consumes the remainder verbatim).
    $process = Start-Process -FilePath $installerPath -ArgumentList "/S /D=$dir" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer exited $($process.ExitCode) on pass $pass." }
    $manifestPath = Join-Path $dir 'install-manifest.json'
    $bytes = [IO.File]::ReadAllBytes($manifestPath)
    $utf8 = New-Object Text.UTF8Encoding($false, $true)
    $manifest = $utf8.GetString($bytes) | ConvertFrom-Json
    if ($manifest.app -ne 'PopChatWindows' -or $manifest.version -ne $release.version -or
        $manifest.install_dir -ne $dir -or $manifest.executable -ne 'popchat_windows.exe' -or
        $manifest.author -ne 'Evis Lai' -or $manifest.installer -ne 'NSIS') { throw 'UTF-8 updater manifest mismatch.' }
    foreach ($file in @('popchat_windows.exe','popchat_updater.exe')) {
        $expected = (Get-FileHash -LiteralPath (Join-Path $package "staging\$file")).Hash
        if ((Get-FileHash -LiteralPath (Join-Path $dir $file)).Hash -ne $expected) { throw 'Installed PE hash mismatch.' }
        $recorded = if ($file -eq 'popchat_windows.exe') { $manifest.sha256 } else { $manifest.updater_sha256 }
        if ($recorded -ne $expected) { throw 'Updater manifest hash mismatch.' }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $dir 'licenses\dependencies\index.json'))) { throw 'Missing licenses.' }
    $registration = Get-ItemProperty -LiteralPath $reg
    if ($registration.InstallLocation -ne $dir -or $registration.Publisher -ne 'Evis Lai') { throw 'Installed apps registration mismatch.' }
    foreach ($shortcut in $shortcuts) {
        if ($shell.CreateShortcut($shortcut).TargetPath -ne (Join-Path $dir 'popchat_windows.exe')) { throw 'Shortcut target mismatch.' }
    }
}
$backups = @(Get-ChildItem -LiteralPath $dir -Directory -Filter '.popchat-before-nsis-*')
if ($backups.Count -ne 1) { throw 'Reinstall did not retain the previous binaries.' }
$uninstaller = Join-Path $dir 'Uninstall.exe'
# A tiny no-UI/no-profile process exercises the real process guard without
# starting either PopChat executable or forcibly terminating a process.
$probeDir = Join-Path $dir 'process-guard-probe'
[void][IO.Directory]::CreateDirectory($probeDir)
$probeSource = Join-Path $probeDir 'probe.rs'
$probeExe = Join-Path $probeDir 'popchat_updater.exe'
[IO.File]::WriteAllText($probeSource, '#![windows_subsystem = "windows"] fn main() { let _ = std::io::stdin().read_line(&mut String::new()); }')
& rustc --edition 2021 --crate-name nsis_guard_probe $probeSource -o $probeExe
if ($LASTEXITCODE -ne 0) { throw 'Guard probe compilation failed.' }
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $probeExe
$start.UseShellExecute = $false
$start.RedirectStandardInput = $true
$start.CreateNoWindow = $true
$probe = [Diagnostics.Process]::Start($start)
try {
    $blockedInstall = Start-Process -FilePath $installerPath -ArgumentList "/S /D=$dir" -Wait -PassThru
    if ($blockedInstall.ExitCode -ne 2) { throw 'Setup did not reject a running updater.' }
    $blockedUninstall = Start-Process -FilePath $uninstaller -ArgumentList "/S _?=$dir" -Wait -PassThru
    if ($blockedUninstall.ExitCode -ne 2 -or -not (Test-Path -LiteralPath $reg)) { throw 'Uninstall did not reject a running updater.' }
} finally {
    $probe.StandardInput.Close()
    if (-not $probe.WaitForExit(10000)) { throw 'Guard probe did not exit normally.' }
    $probe.Dispose()
}
$process = Start-Process -FilePath $uninstaller -ArgumentList "/S _?=$dir" -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Uninstaller exited $($process.ExitCode)." }
if ((Test-Path -LiteralPath $reg) -or (Test-Path -LiteralPath (Join-Path $dir 'popchat_windows.exe'))) { throw 'Uninstall incomplete.' }
foreach ($shortcut in $shortcuts) { if (Test-Path -LiteralPath $shortcut) { throw 'Shortcut was not removed.' } }
if (-not (Test-Path -LiteralPath $sentinel) -or -not (Test-Path -LiteralPath $backups[0].FullName)) { throw 'Uninstall removed unknown files or backups.' }
$afterApps = @(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'" | ForEach-Object ProcessId) -join ','
if ($beforeApps -ne $afterApps) { throw 'Smoke test changed running chat processes.' }
Write-Output "NSIS_SMOKE_PASSED fresh_install=verified unicode_manifest=verified reinstall=verified shortcuts=verified running_process_guards=verified uninstall=verified unknown_files_and_backups=preserved no_chat_launched=true directory=$dir"
