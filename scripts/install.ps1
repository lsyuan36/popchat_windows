<#!
Per-user PopChat installer. Without -Apply it only reports the installation plan.
Only the explicitly selected source profile's custom services are imported;
general preferences, history, OAuth/search credentials and source files stay put.
#>
[CmdletBinding()]
param(
    [string]$SourceProfile = '',
    [int]$ExpectedPid = 0,
    [switch]$Apply,
    [switch]$SkipBuild,
    [switch]$UpdateOnly
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\PopChat'
$profile = Join-Path $env:APPDATA 'PopChat\PopChat\config'
$targetDir = Join-Path $repo 'target\button-review'
$releaseDir = Join-Path $targetDir 'x86_64-pc-windows-msvc\release'
$sourceExe = Join-Path $releaseDir 'popchat_windows.exe'
$sourceUpdater = Join-Path $releaseDir 'popchat_updater.exe'
$importer = Join-Path $releaseDir 'examples\import_custom_providers.exe'
$installedExe = Join-Path $installDir 'popchat_windows.exe'
$installedUpdater = Join-Path $installDir 'popchat_updater.exe'
$manifestPath = Join-Path $installDir 'install-manifest.json'
$startMenu = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\PopChat.lnk'
$desktop = Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'PopChat.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PopChatWindows'
$oldProfileEnv = $env:POPCHAT_DATA_DIR
$oldPinnedEnv = $env:POPCHAT_PINNED
$installGuard = $null

if ($env:OS -ne 'Windows_NT') { throw 'This installer requires Windows.' }
if ($UpdateOnly) {
    if ($SourceProfile) { throw 'UpdateOnly cannot import a source profile.' }
    if (-not (Test-Path -LiteralPath $manifestPath)) { throw 'UpdateOnly requires an existing recognized installation.' }
} else {
    if (-not [IO.Path]::IsPathRooted($SourceProfile)) { throw 'SourceProfile must be absolute, matching the original profile spelling.' }
    if (-not (Test-Path -LiteralPath (Join-Path $SourceProfile 'provider_store.json'))) { throw 'Source provider catalog missing.' }
}
if (-not (Test-Path -LiteralPath (Join-Path $profile 'provider_store.json'))) { throw 'Existing default provider catalog required; no implicit replacement.' }
if (Test-Path -LiteralPath $installDir) {
    if (-not (Test-Path -LiteralPath $manifestPath)) { throw 'Installation directory already exists without a PopChat install manifest; refusing to overwrite it.' }
    $oldManifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($oldManifest.app -ne 'PopChatWindows' -or $oldManifest.install_dir -ne $installDir) { throw 'Unrecognized existing installation.' }
}
$shell = New-Object -ComObject WScript.Shell
foreach ($link in @($startMenu,$desktop)) {
    if ((Test-Path -LiteralPath $link) -and $shell.CreateShortcut($link).TargetPath -ne $installedExe) {
        throw "An unrelated shortcut already exists: $link"
    }
}
if (Test-Path -LiteralPath $uninstallKey) {
    $registration = Get-ItemProperty -LiteralPath $uninstallKey
    if ($registration.InstallLocation -ne $installDir) { throw 'Existing uninstall registration belongs to another location.' }
}
function Assert-ExpectedApp {
    $apps = @(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'")
    if ($ExpectedPid -eq 0) {
        if ($apps.Count -ne 0) { throw 'PopChat is running; supply its sole current PID, do not start a second UI.' }
    } else {
        if ($apps.Count -ne 1 -or $apps[0].ProcessId -ne $ExpectedPid) { throw 'Sole expected app changed; no application closed.' }
        if (-not $apps[0].ExecutablePath.StartsWith((Join-Path $repo 'target\'),[StringComparison]::OrdinalIgnoreCase) -and
            $apps[0].ExecutablePath -ne $installedExe) { throw 'Unexpected running executable; nothing closed.' }
    }
    return $apps
}
function Get-PreservedDataFingerprint {
    $state = [ordered]@{}
    $names=@('settings.json','prompt_history.json','shortcuts.json')
    if ($UpdateOnly) { $names += 'provider_store.json' }
    foreach ($name in $names) {
        $file = Join-Path $profile $name
        $state[$name] = if (Test-Path -LiteralPath $file) { (Get-FileHash -LiteralPath $file).Hash } else { 'ABSENT' }
    }
    $history = Join-Path $profile 'conversations'
    if (Test-Path -LiteralPath $history) {
        foreach ($file in @(Get-ChildItem -LiteralPath $history -File -Recurse | Sort-Object FullName)) {
            $state['conversations/' + $file.FullName.Substring($history.Length)] = (Get-FileHash -LiteralPath $file.FullName).Hash
        }
    }
    return ($state | ConvertTo-Json -Compress)
}
function Get-ExecutableSubsystem([string]$File) {
    $stream=[IO.File]::OpenRead($File)
    $reader=[IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw 'Invalid Windows executable.' }
        $stream.Position=0x3C
        $offset=$reader.ReadInt32()
        if ($offset -lt 0 -or $offset + 94 -gt $stream.Length) { throw 'Invalid PE header bounds.' }
        $stream.Position=$offset
        if ($reader.ReadUInt32() -ne 0x4550) { throw 'Invalid PE signature.' }
        $stream.Position=$offset+24
        if ($reader.ReadUInt16() -notin @(0x10B,0x20B)) { throw 'Unsupported PE optional header.' }
        $stream.Position=$offset+24+68
        return $reader.ReadUInt16()
    } finally { $reader.Dispose(); $stream.Dispose() }
}
$null = Assert-ExpectedApp
[pscustomobject]@{
    InstallDirectory=$installDir
    DefaultDataDirectory=$profile
    CustomServiceSource=$SourceProfile
    StartMenuShortcut=$startMenu
    DesktopShortcut=$desktop
    GeneralPreferencesAndHistory='preserved'
    CredentialScope=$(if ($UpdateOnly) { 'none; no credential or profile import' } else { 'only imported custom provider UUIDs; no plaintext export' })
    EnableLoginStartup=$false
    Apply=[bool]$Apply
    UpdateOnly=[bool]$UpdateOnly
} | Format-List
if (-not $Apply) { return }

try {
    # The formal/default profile must use its original unprefixed credentials.
    Remove-Item Env:POPCHAT_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:POPCHAT_PINNED -ErrorAction SilentlyContinue
    # Only cooperating installers use this lock. Keep the OS handle held through
    # import, installation and launch; never truncate/delete the lock file.
    $installGuard = [IO.File]::Open((Join-Path $profile '.popchat-install.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    if (-not $SkipBuild) {
        Push-Location $repo
        try {
            cargo build --locked --release --bins --target x86_64-pc-windows-msvc --target-dir $targetDir
            if ($LASTEXITCODE -ne 0) { throw 'Application build failed; nothing closed.' }
            if (-not $UpdateOnly) {
                cargo build --locked --release --target x86_64-pc-windows-msvc --target-dir $targetDir --example import_custom_providers
                if ($LASTEXITCODE -ne 0) { throw 'Importer build failed; nothing closed.' }
            }
        } finally { Pop-Location }
    }
    if (-not (Test-Path -LiteralPath $sourceExe)) { throw 'Required application build missing; nothing closed.' }
    if ((Get-ExecutableSubsystem $sourceExe) -ne 2) { throw 'Application must use the Windows GUI subsystem; nothing closed.' }
    if (-not (Test-Path -LiteralPath $sourceUpdater) -or (Get-ExecutableSubsystem $sourceUpdater) -ne 2) {
        throw 'Required Windows GUI updater helper missing or invalid; nothing closed.'
    }
    $fontLicenseSource = Join-Path $repo 'assets\fonts\swei-gothic'
    foreach ($notice in @('SIL_Open_Font_License_1.1.txt','COPYRIGHT.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $fontLicenseSource $notice))) {
            throw 'Bundled Swei Gothic font license/copyright missing; nothing closed.'
        }
    }
    foreach ($notice in @('README.md','LICENSE','NOTICE.md','THIRD_PARTY_NOTICES.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $repo $notice))) {
            throw 'Release documentation/license notice missing; nothing closed.'
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $repo 'assets\licenses\dependencies'))) {
        throw 'Dependency license notices missing; nothing closed.'
    }
    if (-not $UpdateOnly) {
        if (-not (Test-Path -LiteralPath $importer)) { throw 'Required importer build missing; nothing closed.' }
        # Dry-run reads provider metadata only, never a credential.
        & $importer --source-profile $SourceProfile
        if ($LASTEXITCODE -ne 0) { throw 'Import plan validation failed; nothing closed.' }
        $sourceSettingsHash = (Get-FileHash -LiteralPath (Join-Path $SourceProfile 'settings.json')).Hash
        $sourceProviderHash = (Get-FileHash -LiteralPath (Join-Path $SourceProfile 'provider_store.json')).Hash
    }
    $defaultSettingsHash = (Get-FileHash -LiteralPath (Join-Path $profile 'settings.json')).Hash
    $preservedData = Get-PreservedDataFingerprint
    $null = New-Item -ItemType Directory -Path $installDir -Force
    $stagedExe = Join-Path $installDir ('.popchat-install-' + [Guid]::NewGuid().ToString('N') + '.exe')
    [IO.File]::Copy($sourceExe,$stagedExe,$false)
    $stagedUpdater = Join-Path $installDir ('.popchat-updater-install-' + [Guid]::NewGuid().ToString('N') + '.exe')
    [IO.File]::Copy($sourceUpdater,$stagedUpdater,$false)
    $buildHash = (Get-FileHash -LiteralPath $sourceExe).Hash
    if ((Get-FileHash -LiteralPath $stagedExe).Hash -ne $buildHash) { throw 'Staged executable hash mismatch; nothing closed.' }
    $updaterHash = (Get-FileHash -LiteralPath $sourceUpdater).Hash
    if ((Get-FileHash -LiteralPath $stagedUpdater).Hash -ne $updaterHash) { throw 'Staged updater hash mismatch; nothing closed.' }
    $null = Assert-ExpectedApp
    if ($ExpectedPid -ne 0) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PopChatInstallClose {
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr value);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder text, int count);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h,uint msg,IntPtr w,IntPtr l);
    public static bool Close(uint pid) {
        IntPtr window=IntPtr.Zero;
        EnumWindows((h,p)=> {
            uint owner; GetWindowThreadProcessId(h,out owner);
            if(owner!=pid) return true;
            var title=new StringBuilder(256); GetWindowTextW(h,title,256);
            if(title.ToString()!="PopChat") return true;
            window=h; return false;
        },IntPtr.Zero);
        return window!=IntPtr.Zero && PostMessageW(window,0x10,IntPtr.Zero,IntPtr.Zero);
    }
}
'@
        $oldApp = Get-Process -Id $ExpectedPid
        if (-not [PopChatInstallClose]::Close([uint32]$ExpectedPid)) { throw 'WM_CLOSE failed; no migration or launch.' }
        if (-not $oldApp.WaitForExit(10000)) { throw 'Old app did not exit; no migration or launch.' }
    }
    if (@(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'").Count -ne 0) { throw 'Another app remains; no migration or launch.' }
    if (-not $UpdateOnly) {
        & $importer --source-profile $SourceProfile --apply
        if ($LASTEXITCODE -ne 0) { throw 'Custom import failed; sources retained. No new UI launched.' }
        if ((Get-FileHash -LiteralPath (Join-Path $SourceProfile 'settings.json')).Hash -ne $sourceSettingsHash -or
            (Get-FileHash -LiteralPath (Join-Path $SourceProfile 'provider_store.json')).Hash -ne $sourceProviderHash) {
            throw 'An unrelated source profile file changed; pause before launching.'
        }
    }
    if ((Get-FileHash -LiteralPath (Join-Path $profile 'settings.json')).Hash -ne $defaultSettingsHash) { throw 'Default settings changed; pause before launching.' }
    if ((Get-PreservedDataFingerprint) -cne $preservedData) { throw 'Default preferences or history changed; pause before launching.' }
    'DEFAULT_PREFERENCES_COMMANDS_AND_HISTORY_HASHES_PRESERVED'
    if (Test-Path -LiteralPath $installedExe) {
        $backup = Join-Path $installDir ('popchat_windows.before-update-' + [Guid]::NewGuid().ToString('N') + '.exe')
        [IO.File]::Copy($installedExe,$backup,$false)
    }
    Move-Item -LiteralPath $stagedExe -Destination $installedExe -Force
    if (Test-Path -LiteralPath $installedUpdater) {
        [IO.File]::Copy($installedUpdater,(Join-Path $installDir ('popchat_updater.before-update-' + [Guid]::NewGuid().ToString('N') + '.exe')),$false)
    }
    Move-Item -LiteralPath $stagedUpdater -Destination $installedUpdater -Force
    if ((Get-FileHash -LiteralPath $installedExe).Hash -ne $buildHash) { throw 'Installed executable hash mismatch; no launch.' }
    if ((Get-FileHash -LiteralPath $installedUpdater).Hash -ne $updaterHash) { throw 'Installed updater hash mismatch; no launch.' }
    $licenses = Join-Path $installDir 'licenses\font-awesome'
    $null = New-Item -ItemType Directory -Path $licenses -Force
    Copy-Item -Path (Join-Path $repo 'assets\font-awesome\*') -Destination $licenses -Recurse -Force
    $fontLicenses = Join-Path $installDir 'licenses\swei-gothic'
    $null = New-Item -ItemType Directory -Path $fontLicenses -Force
    Copy-Item -Path (Join-Path $fontLicenseSource '*.txt'),(Join-Path $fontLicenseSource '*.md') -Destination $fontLicenses -Force
    $dependencyNotices = Join-Path $repo 'assets\licenses\dependencies'
    if (Test-Path -LiteralPath $dependencyNotices) {
        $dependencyDestination = Join-Path $installDir 'licenses\dependencies'
        $null = New-Item -ItemType Directory -Path $dependencyDestination -Force
        Copy-Item -Path (Join-Path $dependencyNotices '*') -Destination $dependencyDestination -Recurse -Force
    }
    foreach ($notice in @('README.md','LICENSE','NOTICE.md','THIRD_PARTY_NOTICES.md')) {
        Copy-Item -LiteralPath (Join-Path $repo $notice) -Destination (Join-Path $installDir $notice) -Force
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'uninstall.ps1') -Destination (Join-Path $installDir 'uninstall.ps1') -Force
    foreach ($link in @($startMenu,$desktop)) {
        $shortcut = $shell.CreateShortcut($link)
        $shortcut.TargetPath = $installedExe
        $shortcut.Arguments = ''
        $shortcut.WorkingDirectory = $installDir
        $shortcut.IconLocation = "$installedExe,0"
        $shortcut.Description = 'PopChat Windows'
        $shortcut.Save()
        if ($shell.CreateShortcut($link).TargetPath -ne $installedExe) { throw 'Shortcut verification failed.' }
    }
    $versionMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw),'(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) { throw 'Package version missing.' }
    $version = $versionMatch.Groups[1].Value
    $uninstall = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $installDir 'uninstall.ps1') + '" -ConfirmRemoval'
    $null = New-Item -Path $uninstallKey -Force
    foreach ($entry in @{
        DisplayName='PopChat Windows'; DisplayVersion=$version; Publisher='PopChat Windows';
        InstallLocation=$installDir; DisplayIcon=$installedExe; UninstallString=$uninstall
    }.GetEnumerator()) {
        $null = New-ItemProperty -LiteralPath $uninstallKey -Name $entry.Key -Value $entry.Value -PropertyType String -Force
    }
    $null = New-ItemProperty -LiteralPath $uninstallKey -Name NoModify -Value 1 -PropertyType DWord -Force
    $null = New-ItemProperty -LiteralPath $uninstallKey -Name NoRepair -Value 1 -PropertyType DWord -Force
    $manifest = [ordered]@{
        app='PopChatWindows'; version=$version; install_dir=$installDir; executable=$installedExe;
        sha256=$buildHash; data_profile=$profile;
        updater_sha256=$updaterHash; build_channel='stable'; author='Evis Lai';
        imported_custom_source=$(if ($UpdateOnly) { $oldManifest.imported_custom_source } else { $SourceProfile });
        installed_utc=[DateTime]::UtcNow.ToString('o'); shortcuts=@($startMenu,$desktop)
    }
    [IO.File]::WriteAllText($manifestPath,($manifest | ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
    # Only retarget an already-enabled entry for this exact prior executable.
    # Never enable startup as a side effect of installing.
    $runKey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $run = Get-ItemProperty -LiteralPath $runKey -Name PopChat -ErrorAction SilentlyContinue
    if ($run -and $ExpectedPid -ne 0 -and $run.PopChat -eq ('"' + $oldApp.Path + '" --login-item')) {
        Set-ItemProperty -LiteralPath $runKey -Name PopChat -Value ('"' + $installedExe + '" --login-item')
    }
    if (@(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'").Count -ne 0) { throw 'App appeared before launch; no second UI launched.' }
    $app = Start-Process -FilePath $installedExe -ArgumentList '--settings' -WorkingDirectory $installDir -PassThru
    if ($app.WaitForExit(1500)) { throw 'Installed app exited early; no second launch attempted.' }
    $apps = @(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'")
    if ($apps.Count -ne 1 -or $apps[0].ProcessId -ne $app.Id -or $apps[0].ExecutablePath -ne $installedExe) { throw 'Installed sole-process verification failed.' }
    [pscustomobject]@{Installed=$installedExe;Sha256=$buildHash;SolePid=$app.Id;DefaultProfile=$profile;StartupEnabledByInstaller=$false} | Format-List
} finally {
    if ($installGuard) { $installGuard.Dispose() }
    $env:POPCHAT_DATA_DIR = $oldProfileEnv
    $env:POPCHAT_PINNED = $oldPinnedEnv
}
