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
# Hosted Windows runners may expose TEMP through an 8.3 path (RUNNER~1),
# while ShellLink resolves the same directory to its long spelling.
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
public static class PopChatInstallerTestPath {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll")]
    static extern IntPtr LockResource(IntPtr resource);
    [DllImport("kernel32.dll")]
    static extern bool FreeLibrary(IntPtr module);
    public static int AppIconFrames(string path) {
        // LOAD_LIBRARY_AS_DATAFILE: inspect resources; never execute app code.
        IntPtr module = LoadLibraryEx(path, IntPtr.Zero, 2);
        if (module == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            IntPtr group = FindResource(module, new IntPtr(1), new IntPtr(14));
            if (group == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            IntPtr loaded = LoadResource(module, group);
            IntPtr data = loaded == IntPtr.Zero ? IntPtr.Zero : LockResource(loaded);
            if (data == IntPtr.Zero || Marshal.ReadInt16(data, 0) != 0 || Marshal.ReadInt16(data, 2) != 1)
                throw new InvalidOperationException("Invalid app icon resource");
            return (ushort)Marshal.ReadInt16(data, 4);
        } finally { FreeLibrary(module); }
    }
    [ComImport, Guid("000214F9-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellLinkW {
        void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int count, IntPtr findData, uint flags);
        void GetIDList(out IntPtr list);
        void SetIDList(IntPtr list);
        void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder text, int count);
        void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string text);
        void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int count);
    }
    public static string[] ReadShortcut(string path) {
        object obj = Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("00021401-0000-0000-C000-000000000046")));
        try {
            ((IPersistFile)obj).Load(path, 0);
            var link = (IShellLinkW)obj;
            var target = new StringBuilder(32768);
            var working = new StringBuilder(32768);
            link.GetPath(target, target.Capacity, IntPtr.Zero, 0);
            link.GetWorkingDirectory(working, working.Capacity);
            return new string[] { target.ToString(), working.ToString() };
        } finally { Marshal.FinalReleaseComObject(obj); }
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetLongPathName(string path, StringBuilder buffer, uint length);
    public static string Normalize(string path) {
        var buffer = new StringBuilder(32768);
        uint length = GetLongPathName(path, buffer, (uint)buffer.Capacity);
        if (length == 0 || length >= buffer.Capacity)
            throw new Win32Exception(Marshal.GetLastWin32Error());
        return buffer.ToString().TrimEnd('\\');
    }
}
'@
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
$leaf = 'PopChat-NSIS-' + [Guid]::NewGuid().ToString('N').Substring(0,12)
$root = Join-Path ([IO.Path]::GetTempPath()) $leaf
$dir = Join-Path $root (([string][char]0x6E2C + [string][char]0x8A66) + '\PopChat')
if ((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $dir)) { throw 'Fresh-install fixture must not exist.' }
$sentinel = Join-Path $dir 'user-file-do-not-delete.txt'
$beforeApps = @(Get-CimInstance Win32_Process -Filter "Name='popchat_windows.exe'" | ForEach-Object ProcessId) -join ','
# Invalid destination and invalid parent are both owned files, not real drive
# roots/system folders. Failure must not redirect installation elsewhere.
$blocked = Join-Path ([IO.Path]::GetTempPath()) ("$leaf-blocked.txt")
[IO.File]::WriteAllText($blocked, 'preserve blocked target')
$tooLong = Join-Path ([IO.Path]::GetTempPath()) ('PopChat-' + ('x' * 190))
foreach ($invalid in @($blocked, (Join-Path $blocked 'PopChat'), $tooLong, '')) {
    $process = Start-Process -FilePath $installerPath -ArgumentList "/S /D=$invalid" -Wait -PassThru
    if ($process.ExitCode -eq 0 -or (Test-Path -LiteralPath $reg)) { throw "Installer accepted an invalid destination: path=$invalid exit=$($process.ExitCode) registered=$(Test-Path -LiteralPath $reg)" }
    if ([IO.File]::ReadAllText($blocked) -ne 'preserve blocked target') { throw 'Invalid-target guard changed the blocking file.' }
}
# A reparse-point ancestor must not redirect the payload to another folder.
$junctionTarget = Join-Path ([IO.Path]::GetTempPath()) ("$leaf-junction-target")
$junction = Join-Path ([IO.Path]::GetTempPath()) ("$leaf-junction")
[void][IO.Directory]::CreateDirectory($junctionTarget)
$junctionSentinel = Join-Path $junctionTarget 'keep.txt'
[IO.File]::WriteAllText($junctionSentinel, 'preserve junction target')
$null = New-Item -ItemType Junction -Path $junction -Target $junctionTarget
$process = Start-Process -FilePath $installerPath -ArgumentList ("/S /D=" + (Join-Path $junction 'PopChat')) -Wait -PassThru
if ($process.ExitCode -eq 0 -or (Test-Path -LiteralPath $reg) -or (Test-Path -LiteralPath (Join-Path $junctionTarget 'PopChat')) -or
    [IO.File]::ReadAllText($junctionSentinel) -ne 'preserve junction target') { throw 'Reparse-point destination was not rejected safely.' }
for ($pass = 0; $pass -lt 2; $pass++) {
    # /D must be last, without quotes (NSIS consumes the remainder verbatim).
    $process = Start-Process -FilePath $installerPath -ArgumentList "/S /D=$dir" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer exited $($process.ExitCode) on pass $pass." }
    if ($pass -eq 0) {
        [IO.File]::WriteAllText($sentinel, 'preserve unknown files')
    }
    $manifestPath = Join-Path $dir 'install-manifest.json'
    $bytes = [IO.File]::ReadAllBytes($manifestPath)
    $utf8 = New-Object Text.UTF8Encoding($false, $true)
    $manifest = $utf8.GetString($bytes) | ConvertFrom-Json
    $markerBytes = [IO.File]::ReadAllBytes((Join-Path $dir 'nsis-install.ini'))
    if ($markerBytes.Length -lt 2 -or $markerBytes[0] -ne 255 -or $markerBytes[1] -ne 254 -or
        -not ([Text.Encoding]::Unicode.GetString($markerBytes).Contains("InstallDir=$dir"))) {
        throw 'Uninstall marker must preserve the Unicode installation path.'
    }
    if ($manifest.app -ne 'PopChatWindows' -or $manifest.version -ne $release.version -or
        $manifest.install_dir -ne $dir -or $manifest.executable -ne 'popchat_windows.exe' -or
        $manifest.author -ne 'Evis Lai' -or $manifest.installer -ne 'NSIS') { throw 'UTF-8 updater manifest mismatch.' }
    foreach ($file in @('popchat_windows.exe','popchat_updater.exe')) {
        $expected = (Get-FileHash -LiteralPath (Join-Path $package "staging\$file")).Hash
        if ((Get-FileHash -LiteralPath (Join-Path $dir $file)).Hash -ne $expected) { throw 'Installed PE hash mismatch.' }
        $recorded = if ($file -eq 'popchat_windows.exe') { $manifest.sha256 } else { $manifest.updater_sha256 }
        if ($recorded -ne $expected) { throw 'Updater manifest hash mismatch.' }
        $pe = Get-Item -LiteralPath (Join-Path $dir $file)
        if ($pe.VersionInfo.ProductVersion -ne $release.version -or $pe.VersionInfo.CompanyName -ne 'Evis Lai' -or
            [PopChatInstallerTestPath]::AppIconFrames($pe.FullName) -ne 7) { throw 'Installed app icon/version/author resource mismatch.' }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $dir 'licenses\dependencies\index.json'))) { throw 'Missing licenses.' }
    $registration = Get-ItemProperty -LiteralPath $reg
    if ($registration.InstallLocation -ne $dir -or $registration.Publisher -ne 'Evis Lai') { throw 'Installed apps registration mismatch.' }
    foreach ($shortcut in $shortcuts) {
        $link = $shell.CreateShortcut($shortcut)
        Write-Output ("NSIS_SHORTCUT_RAW " + (@{
            shortcut=$shortcut; target=$link.TargetPath; working=$link.WorkingDirectory
            expected_target=(Join-Path $dir 'popchat_windows.exe'); expected_working=$dir
        } | ConvertTo-Json -Compress))
        # Read the actual Unicode ShellLink, without WSH's legacy ANSI path
        # conversion. This is the same native interface Windows Explorer uses.
        $wide = [PopChatInstallerTestPath]::ReadShortcut($shortcut)
        Write-Output ("NSIS_SHORTCUT_UNICODE " + (@{ target=$wide[0]; working=$wide[1] } | ConvertTo-Json -Compress))
        $target = [PopChatInstallerTestPath]::Normalize($wide[0])
        $working = [PopChatInstallerTestPath]::Normalize($wide[1])
        $expectedTarget = [PopChatInstallerTestPath]::Normalize((Join-Path $dir 'popchat_windows.exe'))
        $expectedWorking = [PopChatInstallerTestPath]::Normalize($dir)
        if ($target -ne $expectedTarget -or $working -ne $expectedWorking) {
            throw "Shortcut target/working directory mismatch: shortcut=$shortcut target=$target expected_target=$expectedTarget working=$working expected_working=$expectedWorking"
        }
    }
}
$backups = @(Get-ChildItem -LiteralPath $dir -Directory -Filter '.popchat-before-nsis-*')
if ($backups.Count -ne 1) { throw 'Reinstall did not retain the previous binaries.' }
$uninstaller = Join-Path $dir 'Uninstall.exe'
# A tiny no-UI/no-profile process exercises the real process guard without
# starting either PopChat executable or forcibly terminating a process.
$probeDir = Join-Path $dir 'process-guard-probe'
[void][IO.Directory]::CreateDirectory($probeDir)
$movedUninstaller = Join-Path $probeDir 'Uninstall.exe'
[IO.File]::Copy($uninstaller, $movedUninstaller, $false)
$moved = Start-Process -FilePath $movedUninstaller -ArgumentList "/S _?=$probeDir" -Wait -PassThru
if ($moved.ExitCode -ne 10 -or -not (Test-Path -LiteralPath $reg) -or -not (Test-Path -LiteralPath (Join-Path $dir 'popchat_windows.exe'))) {
    throw 'Relocated uninstaller did not refuse an unverified directory.'
}
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
Write-Output "NSIS_SMOKE_PASSED nonexistent_directory_and_parents=verified invalid_empty_long_file_and_junction_destinations=blocked unicode_manifest_and_marker=verified app_icon_resources=verified relocated_uninstaller=blocked reinstall=verified shortcuts=verified running_process_guards=verified uninstall=verified unknown_files_and_backups=preserved no_chat_launched=true directory=$dir"
