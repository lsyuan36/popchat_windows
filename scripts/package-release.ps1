#requires -Version 5.1
<#
.SYNOPSIS
Build and package the unsigned, portable Windows x64 release.
.EXAMPLE
powershell -NoProfile -File scripts/package-release.ps1
.EXAMPLE
powershell -NoProfile -File scripts/package-release.ps1 -SkipBuild -AllowDirty
.NOTES
SkipBuild uses the caller's existing binaries; it does not establish their source
provenance. AllowDirty is for pre-commit previews, not published releases. Every
invocation uses a new GUID directory and never overwrites or deletes old output.
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$AllowDirty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$target = 'x86_64-pc-windows-msvc'
$targetDir = Join-Path $repo 'target/button-review'
$binaryDir = Join-Path $targetDir "$target/release"
$assetName = 'PopChat-Windows-x64.zip'
$maxBytes = 256MB
$utf8 = New-Object Text.UTF8Encoding($false)

function Invoke-GitText([string[]]$Arguments) {
    $result = & git -C $repo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "git $($Arguments -join ' ') failed." }
    return ($result -join "`n").Trim()
}

function Assert-RegularSource([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Source outside repository: $full"
    }
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { throw "Required release file is missing: $full" }
    $item = Get-Item -LiteralPath $full -Force
    if ($item.PSIsContainer) { throw "Expected a file: $full" }
    while ($null -ne $item -and $item.FullName.Length -ge $repo.Length) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Links/reparse points are not allowed: $($item.FullName)"
        }
        if ($item -is [IO.FileInfo]) { $item = $item.Directory } else { $item = $item.Parent }
    }
}

function Assert-X64GuiPE([string]$Path) {
    Assert-RegularSource $Path
    $stream = [IO.File]::OpenRead($Path)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        if ($stream.Length -lt 128 -or $reader.ReadUInt16() -ne 0x5A4D) { throw 'Missing DOS header' }
        $stream.Position = 0x3c
        $pe = $reader.ReadUInt32()
        if ($pe -gt ($stream.Length - 96)) { throw 'Invalid PE offset' }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x00004550) { throw 'Missing PE signature' }
        if ($reader.ReadUInt16() -ne 0x8664) { throw 'Not AMD64/x64' }
        $stream.Position = $pe + 20
        if ($reader.ReadUInt16() -lt 70) { throw 'Truncated optional header' }
        $flags = $reader.ReadUInt16()
        if (($flags -band 2) -eq 0 -or ($flags -band 0x2000) -ne 0) { throw 'Not an executable image' }
        if ($reader.ReadUInt16() -ne 0x20b) { throw 'Not PE32+' }
        $stream.Position = $pe + 24 + 68
        if ($reader.ReadUInt16() -ne 2) { throw 'Not the Windows GUI subsystem' }
    } catch {
        throw "Invalid x64 GUI binary '$Path': $_"
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Read-Metadata([switch]$NoDeps) {
    $args = @('metadata', '--locked', '--offline', '--format-version', '1', '--manifest-path', (Join-Path $repo 'Cargo.toml'), '--filter-platform', $target)
    if ($NoDeps) { $args += '--no-deps' }
    $json = & cargo @args
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed; populate the locked dependency cache first.' }
    return (($json -join "`n") | ConvertFrom-Json)
}

$sourceCommit = Invoke-GitText @('rev-parse', '--verify', 'HEAD')
if ($sourceCommit -notmatch '^[0-9a-f]{40,64}$') { throw 'A valid source Git commit is required.' }
$dirty = -not [string]::IsNullOrWhiteSpace((Invoke-GitText @('status', '--porcelain', '--untracked-files=all')))
if ($dirty -and -not $AllowDirty) { throw 'Working tree is dirty. Commit release sources first, or use -AllowDirty for an unpublished preview.' }
if ($dirty) { Write-Warning 'Dirty-source preview: do not publish these artifacts as a clean release.' }

$metadata = Read-Metadata
$package = @($metadata.packages | Where-Object { $_.manifest_path -eq (Join-Path $repo 'Cargo.toml') })
if ($package.Count -ne 1) { throw 'Could not uniquely identify the root Cargo package.' }
$version = [string]$package[0].version
if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw "Unsafe release version: $version" }

$licenseRoot = Join-Path $repo 'assets/licenses/dependencies'
$indexPath = Join-Path $licenseRoot 'index.json'
Assert-RegularSource $indexPath
$licenseIndex = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
$covered = @{}
foreach ($entry in $licenseIndex.packages) { $covered["$($entry.name)@$($entry.version)"] = $true }
$nodes = @{}
foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
$byId = @{}
foreach ($pkg in $metadata.packages) { $byId[$pkg.id] = $pkg }
$seen = @{}
$pending = New-Object 'Collections.Generic.Queue[string]'
$pending.Enqueue($metadata.resolve.root)
while ($pending.Count -gt 0) {
    $id = $pending.Dequeue()
    if ($seen.ContainsKey($id)) { continue }
    $seen[$id] = $true
    $pkg = $byId[$id]
    if ($null -ne $pkg.source -and -not $covered.ContainsKey("$($pkg.name)@$($pkg.version)")) {
        throw "Missing dependency notices for $($pkg.name) $($pkg.version). Refresh assets/licenses/dependencies/index.json."
    }
    foreach ($dep in $nodes[$id].deps) {
        if (@($dep.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) { $pending.Enqueue($dep.pkg) }
    }
}

# Explicit source-to-archive mapping: no project tree, profiles, installers,
# installed fonts or raw Swei TTF are swept into the portable ZIP.
$files = New-Object 'Collections.Generic.List[object]'
foreach ($name in @('popchat_windows.exe', 'popchat_updater.exe')) {
    $files.Add([pscustomobject]@{ Source = Join-Path $binaryDir $name; Entry = $name })
}
foreach ($name in @('README.md', 'LICENSE', 'NOTICE.md', 'THIRD_PARTY_NOTICES.md')) {
    $files.Add([pscustomobject]@{ Source = Join-Path $repo $name; Entry = $name })
}
foreach ($name in @('LICENSE.txt', 'NOTICE.md', 'README.md')) {
    $files.Add([pscustomobject]@{ Source = Join-Path $repo "assets/font-awesome/$name"; Entry = "licenses/font-awesome/$name" })
}
$iconDir = Join-Path $repo 'assets/font-awesome/solid'
foreach ($name in @(
    'arrow-up.svg', 'check.svg', 'clock-rotate-left.svg', 'code-branch.svg',
    'comment-medical.svg', 'copy.svg', 'gear.svg', 'pen.svg', 'plus.svg',
    'rotate-right.svg', 'stop.svg', 'thumbtack.svg',
    'up-right-and-down-left-from-center.svg', 'xmark.svg'
)) {
    $files.Add([pscustomobject]@{ Source = Join-Path $iconDir $name; Entry = "licenses/font-awesome/solid/$name" })
}
foreach ($name in @('SIL_Open_Font_License_1.1.txt', 'COPYRIGHT.txt', 'SOURCE.md')) {
    $files.Add([pscustomobject]@{ Source = Join-Path $repo "assets/fonts/swei-gothic/$name"; Entry = "licenses/swei-gothic/$name" })
}
$files.Add([pscustomobject]@{ Source = $indexPath; Entry = 'licenses/dependencies/index.json' })
foreach ($entry in $licenseIndex.packages) {
    if (@($entry.files).Count -eq 0) {
        # A newly added dependency can keep its full license in the existing
        # bundled notices, without changing the portable ZIP entry contract.
        $notice = $entry.PSObject.Properties['bundled_notice']
        if ($null -eq $notice -or $notice.Value.path -ne 'THIRD_PARTY_NOTICES.md' -or $notice.Value.sha256 -notmatch '^[0-9a-f]{64}$') {
            throw "Empty or invalid bundled license inventory: $($entry.name)"
        }
        $source = Join-Path $repo 'THIRD_PARTY_NOTICES.md'
        Assert-RegularSource $source
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $notice.Value.sha256) {
            throw "Bundled dependency notice hash mismatch: $($entry.name)"
        }
        if ($notice.Value.section -ne "$($entry.name) $($entry.version)" -or -not [IO.File]::ReadAllText($source).Contains("#### $($notice.Value.section)")) {
            throw "Bundled dependency notice section missing: $($entry.name)"
        }
        continue
    }
    foreach ($relative in $entry.files) {
        if ($relative -match '(^|[\\/])\.\.([\\/]|$)|^[\\/]|:|[\x00-\x1f]' -or $relative -match '\.(exe|dll|ttf|otf|woff2?|ps1|rs|c|h)$') {
            throw "Unsafe dependency license path: $relative"
        }
        $source = Join-Path $licenseRoot $relative
        Assert-RegularSource $source
        $expectedHash = $entry.file_sha256.PSObject.Properties[$relative]
        if ($null -eq $expectedHash -or (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $expectedHash.Value) {
            throw "Dependency license inventory hash mismatch: $relative"
        }
        $files.Add([pscustomobject]@{ Source = $source; Entry = 'licenses/dependencies/' + $relative.Replace('\', '/') })
    }
}
$unique = @{}
foreach ($file in $files) {
    if ($unique.ContainsKey($file.Entry)) { throw "Duplicate ZIP entry: $($file.Entry)" }
    $unique[$file.Entry] = $true
    if ($file.Entry -notlike '*.exe') { Assert-RegularSource $file.Source }
}

if (-not $SkipBuild) {
    & cargo build --manifest-path (Join-Path $repo 'Cargo.toml') --locked --release --bins --target $target --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
} else {
    Write-Warning 'SkipBuild: packaging existing binaries; their source provenance is the caller''s responsibility.'
}
foreach ($name in @('popchat_windows.exe', 'popchat_updater.exe')) { Assert-X64GuiPE (Join-Path $binaryDir $name) }
if ((Invoke-GitText @('rev-parse', 'HEAD')) -ne $sourceCommit) { throw 'Git HEAD changed during packaging.' }
$dirtyAfter = -not [string]::IsNullOrWhiteSpace((Invoke-GitText @('status', '--porcelain', '--untracked-files=all')))
if ($dirtyAfter -and -not $AllowDirty) { throw 'Working tree changed during the build. Commit sources and rebuild.' }

$expandedBytes = [long]0
foreach ($file in $files) { $expandedBytes += (Get-Item -LiteralPath $file.Source).Length }
if ($expandedBytes -gt $maxBytes) { throw 'Expanded archive exceeds the updater 256 MiB limit.' }
$runDir = Join-Path $repo ("target/releases/v{0}/{1}" -f $version, [Guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $runDir) { throw "Output already exists: $runDir" }
$ancestor = Split-Path -Parent $runDir
while ($ancestor.Length -ge $repo.Length) {
    if (Test-Path -LiteralPath $ancestor) {
        if (((Get-Item -LiteralPath $ancestor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Output ancestor is a link/reparse point: $ancestor"
        }
    }
    $ancestor = Split-Path -Parent $ancestor
}
[void][IO.Directory]::CreateDirectory($runDir)
$stage = Join-Path $runDir 'staging'
[void][IO.Directory]::CreateDirectory($stage)
foreach ($file in $files) {
    $destination = Join-Path $stage $file.Entry
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
    [IO.File]::Copy($file.Source, $destination, $false)
}

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipPath = Join-Path $runDir $assetName
$zipStream = [IO.File]::Open($zipPath, [IO.FileMode]::CreateNew)
$archive = New-Object IO.Compression.ZipArchive($zipStream, [IO.Compression.ZipArchiveMode]::Create, $false)
try {
    foreach ($file in ($files | Sort-Object Entry)) {
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, (Join-Path $stage $file.Entry), $file.Entry, [IO.Compression.CompressionLevel]::Optimal)
    }
} finally {
    $archive.Dispose()
    $zipStream.Dispose()
}
$zipBytes = (Get-Item -LiteralPath $zipPath).Length
if ($zipBytes -gt $maxBytes) { throw 'Compressed archive exceeds the updater 256 MiB limit; do not publish.' }
$sha256 = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $runDir 'SHA256SUMS.txt'), "$sha256  $assetName`n", $utf8)
$manifest = [ordered]@{
    version = $version
    channel = 'stable'
    author = 'Evis Lai'
    source_commit = $sourceCommit
    source_dirty = [bool]($dirty -or $dirtyAfter)
    skip_build = [bool]$SkipBuild
    target = $target
    assetname = $assetName
    sha256 = $sha256
    archive_bytes = $zipBytes
    expanded_bytes = $expandedBytes
    created_utc = [DateTime]::UtcNow.ToString('o')
    signed = $false
}
[IO.File]::WriteAllText((Join-Path $runDir 'release-manifest.json'), (($manifest | ConvertTo-Json -Depth 5) + "`n"), $utf8)
Write-Output "Archive: $zipPath"
Write-Output "SHA256:  $sha256"
Write-Output "Manifest and SHA256SUMS.txt: $runDir"
