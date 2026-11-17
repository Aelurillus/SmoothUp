[CmdletBinding()]
param(
    [switch] $Rife,
    [switch] $VerboseOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 6) {
    $windowsPowerShellModules = Join-Path $PSHOME 'Modules'
    $modulePaths = @($env:PSModulePath -split [System.IO.Path]::PathSeparator | Where-Object { $_ -and $_ -ne $windowsPowerShellModules })
    $env:PSModulePath = (@($windowsPowerShellModules) + $modulePaths) -join [System.IO.Path]::PathSeparator
    Import-Module (Join-Path $windowsPowerShellModules 'Microsoft.PowerShell.Utility') -ErrorAction Stop
    Import-Module (Join-Path $windowsPowerShellModules 'Microsoft.PowerShell.Management') -ErrorAction Stop
}

$ffmpegVersion = '9.0.2'
$ffmpegSha256 = '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba'
$ffmpegUrls = @(
    'https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-9.0.2-essentials_build.zip',
    'https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip'
)
$rifeVersion = '20221029'
$rifeSha256 = 'd8e4d772d26cd8006ef0ad0bc82eb191b53c68677d1ae2f42506d74cbbbea606'
$rifeUrl = 'https://github.com/nihui/rife-ncnn-vulkan/releases/download/20221029/rife-ncnn-vulkan-20221029-windows.zip'
$rifeFiles = @(
    'rife-ncnn-vulkan-20221029-windows/rife-ncnn-vulkan.exe',
    'rife-ncnn-vulkan-20221029-windows/vcomp140.dll',
    'rife-ncnn-vulkan-20221029-windows/LICENSE',
    'rife-ncnn-vulkan-20221029-windows/rife-v4.6/flownet.bin',
    'rife-ncnn-vulkan-20221029-windows/rife-v4.6/flownet.param'
)
$maxZipEntries = 256
$maxExpandedBytes = 536870912L
$maxFileBytes = 134217728L

function Get-ManagedHome {
    if (-not [string]::IsNullOrWhiteSpace($env:SMOOTHUP_HOME)) {
        return [System.IO.Path]::GetFullPath($env:SMOOTHUP_HOME)
    }
    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        return Join-Path $env:LOCALAPPDATA 'SmoothUp'
    }
    return Join-Path $HOME '.smoothup'
}

function Test-Cancelled {
    if (-not [Console]::IsInputRedirected -and [Console]::KeyAvailable -and [Console]::ReadKey($true).Key -eq [ConsoleKey]::Escape) {
        throw [System.OperationCanceledException]::new('Installation cancelled by Escape.')
    }
}

function Get-Sha256Hex {
    param([string] $Path)
    $stream = [System.IO.File]::OpenRead($Path)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
        $stream.Dispose()
    }
}

function Get-Archive {
    param([string[]] $Urls, [string] $Destination, [string] $ExpectedHash)

    $lastError = $null
    foreach ($url in $Urls) {
        Test-Cancelled
        $partial = "$Destination.partial"
        Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue
        try {
            if ($VerboseOutput) { Write-Host "Downloading $url" }
            Invoke-WebRequest -Uri $url -OutFile $partial -UseBasicParsing
            $actualHash = Get-Sha256Hex -Path $partial
            if ($actualHash -ne $ExpectedHash) {
                throw "SHA256 mismatch for $url (expected $ExpectedHash, got $actualHash)."
            }
            [System.IO.File]::Move($partial, $Destination)
            return
        }
        catch {
            $lastError = $_
            Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue
            if ($VerboseOutput) { Write-Warning "Download source failed: $($_.Exception.Message)" }
        }
    }
    throw "All pinned download sources failed or failed SHA256 verification. $lastError"
}

function Get-ArchiveEntries {
    param([System.IO.Compression.ZipArchive] $Archive)
    if ($Archive.Entries.Count -gt $maxZipEntries) {
        throw "Archive contains too many entries ($($Archive.Entries.Count))."
    }
    foreach ($entry in $Archive.Entries) {
        if ($entry.FullName.Contains([char]92) -or $entry.FullName.StartsWith('/') -or $entry.FullName -match '(^|/)\.\.(?:/|$)') {
            throw "Unsafe archive path: $($entry.FullName)"
        }
        if ($entry.Length -gt $maxFileBytes) {
            throw "Archive member exceeds size limit: $($entry.FullName)"
        }
    }
}

function Expand-SelectedZip {
    param(
        [string] $ZipPath,
        [string] $Destination,
        [ValidateSet('ffmpeg', 'rife')] [string] $Kind
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        Get-ArchiveEntries $archive
        $selected = @{}
        if ($Kind -eq 'rife') {
            foreach ($name in $rifeFiles) { $selected[$name] = $true }
        }
        else {
            foreach ($entry in $archive.Entries) {
                if ($entry.FullName -match '^[^/]+/bin/[^/]+$' -and $entry.Name -match '^(ffmpeg|ffprobe)\.exe$') {
                    $selected[$entry.FullName] = $true
                }
            }
        }
        if ($selected.Count -eq 0) { throw "No expected $Kind files found in archive." }
        $seen = @{}
        foreach ($entry in $archive.Entries) {
            if (-not $selected.ContainsKey($entry.FullName)) { continue }
            $relative = if ($Kind -eq 'rife') {
                $entry.FullName.Substring('rife-ncnn-vulkan-20221029-windows/'.Length)
            }
            else {
                $entry.Name
            }
            if ([string]::IsNullOrWhiteSpace($relative) -or $relative -match '(^|/)\.\.(?:/|$)' -or $relative.StartsWith('/') -or $relative.Contains([char]92) -or ($Kind -eq 'ffmpeg' -and $relative.Contains('/'))) {
                throw "Unexpected selected archive path: $($entry.FullName)"
            }
            $target = Join-Path $Destination $relative
            $rootPath = [System.IO.Path]::GetFullPath($Destination).TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
            $targetPath = [System.IO.Path]::GetFullPath($target)
            if (-not $targetPath.StartsWith($rootPath, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Archive member escaped staging directory: $($entry.FullName)"
            }
            $destinationParent = Split-Path -Parent $target
            if (-not (Test-Path -LiteralPath $destinationParent)) { New-Item -ItemType Directory -Path $destinationParent -Force | Out-Null }
            $input = $entry.Open()
            try {
                $output = [System.IO.File]::Open($target, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
                try { $input.CopyTo($output) } finally { $output.Dispose() }
            }
            finally { $input.Dispose() }
            $seen[$entry.FullName] = $true
        }
        if ($Kind -eq 'rife') {
            foreach ($name in $rifeFiles) {
                if (-not $seen.ContainsKey($name)) { throw "Required RIFE archive member is missing: $name" }
            }
        }
        else {
            foreach ($name in @('ffmpeg.exe', 'ffprobe.exe')) {
                if (-not (Test-Path -LiteralPath (Join-Path $Destination $name) -PathType Leaf)) {
                    throw "Required FFmpeg executable is missing: $name"
                }
            }
        }
    }
    finally { $archive.Dispose() }
}

function Install-Atomic {
    param(
        [string] $Name,
        [string] $ZipPath,
        [string] $Target,
        [ValidateSet('ffmpeg', 'rife')] [string] $Kind
    )

    $parent = Split-Path -Parent $Target
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $staging = Join-Path $parent (".$Name.stage." + [Guid]::NewGuid().ToString('N'))
    $backup = Join-Path $parent (".$Name.backup." + [Guid]::NewGuid().ToString('N'))
    $oldMoved = $false
    try {
        New-Item -ItemType Directory -Path $staging | Out-Null
        Expand-SelectedZip -ZipPath $ZipPath -Destination $staging -Kind $Kind
        Test-Cancelled
        if (Test-Path -LiteralPath $Target) {
            [System.IO.Directory]::Move($Target, $backup)
            $oldMoved = $true
        }
        try {
            [System.IO.Directory]::Move($staging, $Target)
        }
        catch {
            if ($oldMoved -and -not (Test-Path -LiteralPath $Target)) {
                [System.IO.Directory]::Move($backup, $Target)
                $oldMoved = $false
            }
            throw
        }
        if ($oldMoved) {
            Remove-Item -LiteralPath $backup -Recurse -Force
            $oldMoved = $false
        }
    }
    finally {
        if ($oldMoved -and (Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $Target)) {
            [System.IO.Directory]::Move($backup, $Target)
        }
        if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
    }
}

$managedHome = Get-ManagedHome
$deps = Join-Path $managedHome 'deps'
$cache = Join-Path $managedHome 'cache'
New-Item -ItemType Directory -Path $deps, $cache -Force | Out-Null
$ffmpegArchive = Join-Path $cache "ffmpeg-$ffmpegVersion.zip"
$rifeArchive = Join-Path $cache "rife-$rifeVersion-windows.zip"

try {
    if (-not (Test-Path -LiteralPath $ffmpegArchive -PathType Leaf) -or
        (Get-Sha256Hex -Path $ffmpegArchive) -ne $ffmpegSha256) {
        Remove-Item -LiteralPath $ffmpegArchive -Force -ErrorAction SilentlyContinue
        Get-Archive -Urls $ffmpegUrls -Destination $ffmpegArchive -ExpectedHash $ffmpegSha256
    }
    $ffmpegTarget = Join-Path $deps 'ffmpeg'
    Install-Atomic -Name 'ffmpeg' -ZipPath $ffmpegArchive -Target $ffmpegTarget -Kind 'ffmpeg'
    Write-Host "FFmpeg $ffmpegVersion installed under $ffmpegTarget"

    if ($Rife) {
        if (-not (Test-Path -LiteralPath $rifeArchive -PathType Leaf) -or
            (Get-Sha256Hex -Path $rifeArchive) -ne $rifeSha256) {
            Remove-Item -LiteralPath $rifeArchive -Force -ErrorAction SilentlyContinue
            Get-Archive -Urls @($rifeUrl) -Destination $rifeArchive -ExpectedHash $rifeSha256
        }
        $rifeTarget = Join-Path $deps 'rife'
        Install-Atomic -Name 'rife' -ZipPath $rifeArchive -Target $rifeTarget -Kind 'rife'
        Write-Host "RIFE Vulkan $rifeVersion with only model v4.6 installed under $rifeTarget"
    }
}
catch {
    [Console]::Error.WriteLine("SmoothUp setup failed: $($_.Exception.Message)")
    exit 1
}
