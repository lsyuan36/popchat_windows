[CmdletBinding()]
param([switch]$ConfirmRemoval)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$expected=Join-Path $env:LOCALAPPDATA 'Programs\PopChat'
if ($root -ne $expected) { throw 'Unexpected installation directory; nothing removed.' }
$manifestPath=Join-Path $root 'install-manifest.json'
$manifest=Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.app -ne 'PopChatWindows' -or $manifest.install_dir -ne $root -or
    $manifest.executable -ne (Join-Path $root 'popchat_windows.exe')) { throw 'Unrecognized install manifest; nothing removed.' }
if (-not $ConfirmRemoval) { 'Use -ConfirmRemoval to uninstall program files and shortcuts. User data and credentials are always retained.'; return }
if (@(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'" | Where-Object { $_.ExecutablePath -eq $manifest.executable }).Count -ne 0) {
    throw 'Close PopChat before uninstalling; no files removed.'
}
$shell=New-Object -ComObject WScript.Shell
foreach($link in $manifest.shortcuts) {
    $allowed=@(
        (Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\PopChat.lnk'),
        (Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'PopChat.lnk')
    )
    if ($link -notin $allowed) { throw 'Unexpected shortcut path; nothing further removed.' }
    if ((Test-Path -LiteralPath $link) -and $shell.CreateShortcut($link).TargetPath -eq $manifest.executable) { Remove-Item -LiteralPath $link }
}
$registration='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PopChatWindows'
if ((Get-ItemProperty -LiteralPath $registration -ErrorAction SilentlyContinue).InstallLocation -eq $root) { Remove-Item -LiteralPath $registration }
$runKey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$run=Get-ItemProperty -LiteralPath $runKey -Name PopChat -ErrorAction SilentlyContinue
if ($run -and $run.PopChat -eq ('"' + $manifest.executable + '" --login-item')) { Remove-ItemProperty -LiteralPath $runKey -Name PopChat }
# Remove only the known application files. Keep backups, unexpected files,
# all profile directories and every Windows credential untouched.
if (Test-Path -LiteralPath $manifest.executable) {
    if ((Get-FileHash -LiteralPath $manifest.executable).Hash -eq $manifest.sha256) { Remove-Item -LiteralPath $manifest.executable }
}
$updater=Join-Path $root 'popchat_updater.exe'
if ($manifest.updater_sha256 -and (Test-Path -LiteralPath $updater)) {
    if ((Get-FileHash -LiteralPath $updater).Hash -eq $manifest.updater_sha256) { Remove-Item -LiteralPath $updater }
}
Remove-Item -LiteralPath $manifestPath
'PopChat uninstalled. User data, credentials, license files and any executable backups were retained.'
