<#
.SYNOPSIS
构建并打包 SmoothUp Windows 发布 ZIP。

.DESCRIPTION
生成 dist/SmoothUp-<版本>-windows-x64.zip，包含主程序、文档、setup.ps1、
SHA256SUMS.txt 与 MANIFEST.txt。禁止打包 FFmpeg/RIFE 外部二进制，并强制
检查 smoothup.exe 不超过 10 MiB 工程目标。

.PARAMETER OutputDir
输出目录，默认 dist（相对仓库根目录）。

.PARAMETER Version
覆盖版本号；默认读取 Cargo.toml。

.PARAMETER SkipBuild
跳过 cargo build --release，直接使用 target/release/smoothup.exe。

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
#>
[CmdletBinding()]
param(
    [string]$OutputDir = 'dist',
    [string]$Version = '',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$SizeLimit = 10 * 1024 * 1024
$stageRoot = $null

function Format-Bytes([long]$Bytes) {
    if ($Bytes -ge 1MB) { return ('{0:N2} MiB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N2} KiB' -f ($Bytes / 1KB)) }
    return ('{0} B' -f $Bytes)
}

try {
    Push-Location $RepoRoot

    $manifestPath = Join-Path $RepoRoot 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        throw "找不到 Cargo.toml：$manifestPath"
    }

    if ([string]::IsNullOrWhiteSpace($Version)) {
        $cargoText = Get-Content -LiteralPath $manifestPath -Raw
        $match = [regex]::Match($cargoText, '(?m)^version\s*=\s*"([^"]+)"')
        if (-not $match.Success) {
            throw '无法从 Cargo.toml 读取版本号；请使用 -Version 指定。'
        }
        $Version = $match.Groups[1].Value
    }
    if ($Version -notmatch '^[A-Za-z0-9._-]+$') {
        throw "版本号包含不安全字符：$Version"
    }

    $required = @(
        'README.md',
        'CHANGELOG.md',
        'LICENSE',
        'THIRD_PARTY_NOTICES.md',
        'docs',
        'docs\cli.md',
        'docs\platform-verification.md',
        'docs\troubleshooting.md',
        'docs\build.md',
        'scripts\setup.ps1'
    )
    foreach ($item in $required) {
        if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot $item))) {
            throw "缺少发布必需文件：$item"
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot 'docs\licenses\Apache-2.0.txt'))) {
        throw '缺少 docs\licenses\Apache-2.0.txt'
    }

    if (-not $SkipBuild) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
        if (-not $cargo) {
            throw '找不到 cargo；请安装 Rust 工具链，或使用 -SkipBuild 复用已有构建。'
        }
        $buildArgs = @('build', '--release')
        if (Test-Path -LiteralPath (Join-Path $RepoRoot 'Cargo.lock')) {
            $buildArgs += '--locked'
        }
        Write-Host ('[build] cargo {0}' -f ($buildArgs -join ' '))
        & $cargo.Source @buildArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build 失败，退出码 $LASTEXITCODE"
        }
    }

    $targetDir = if ($env:CARGO_TARGET_DIR) {
        if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
            $env:CARGO_TARGET_DIR
        } else {
            Join-Path $RepoRoot $env:CARGO_TARGET_DIR
        }
    } else {
        Join-Path $RepoRoot 'target'
    }
    $exePath = Join-Path $targetDir 'release\smoothup.exe'
    if (-not (Test-Path -LiteralPath $exePath)) {
        throw "找不到构建产物：$exePath"
    }

    $exeSize = (Get-Item -LiteralPath $exePath).Length
    if ($exeSize -gt $SizeLimit) {
        throw ("smoothup.exe 为 {0}，超过 10 MiB 工程目标（{1} 字节）；请先优化构建，不要跳过检查。" -f (Format-Bytes $exeSize), $SizeLimit)
    }

    if (-not [System.IO.Path]::IsPathRooted($OutputDir)) {
        $OutputDir = Join-Path $RepoRoot $OutputDir
    }
    if (-not (Test-Path -LiteralPath $OutputDir)) {
        New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    }

    $stageName = "SmoothUp-$Version-windows-x64"
    $stageRoot = Join-Path $OutputDir ('.smoothup-package-' + [Guid]::NewGuid().ToString('N'))
    if (Test-Path -LiteralPath $stageRoot) {
        throw "临时打包目录已存在，拒绝复用：$stageRoot"
    }
    $stagePath = Join-Path $stageRoot $stageName
    New-Item -ItemType Directory -Path $stagePath -Force | Out-Null

    Copy-Item -LiteralPath $exePath -Destination (Join-Path $stagePath 'smoothup.exe')
    foreach ($doc in @('README.md', 'CHANGELOG.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md')) {
        Copy-Item -LiteralPath (Join-Path $RepoRoot $doc) -Destination (Join-Path $stagePath $doc)
    }
    Copy-Item -LiteralPath (Join-Path $RepoRoot 'docs') -Destination (Join-Path $stagePath 'docs') -Recurse
    Copy-Item -LiteralPath (Join-Path $RepoRoot 'scripts\setup.ps1') -Destination (Join-Path $stagePath 'setup.ps1')

    $forbidden = Get-ChildItem -LiteralPath $stagePath -Recurse -File | Where-Object {
        $_.Name -match '^(ffmpeg|ffprobe|ffplay|rife-ncnn-vulkan)(\.exe)?$' -or $_.Extension -ieq '.dll'
    }
    if ($forbidden) {
        $names = ($forbidden | ForEach-Object { $_.FullName }) -join '; '
        throw "发布包中发现不允许捆绑的外部二进制/动态库：$names"
    }

    $payloadFiles = @(Get-ChildItem -LiteralPath $stagePath -Recurse -File | Sort-Object FullName)
    $sumLines = New-Object System.Collections.Generic.List[string]
    $manifestLines = New-Object System.Collections.Generic.List[string]
    $manifestLines.Add("SmoothUp $Version Windows x64 package manifest")
    $manifestLines.Add(("Built: {0:o}" -f (Get-Date)))
    $manifestLines.Add("Executable limit: $SizeLimit bytes (10 MiB)")
    $manifestLines.Add("Executable size: $exeSize bytes")
    $manifestLines.Add("Executable within limit: yes")
    $manifestLines.Add("Bundled FFmpeg/RIFE: no; install with setup.ps1 or 'smoothup setup'.")
    $manifestLines.Add("")
    $manifestLines.Add('BYTES  SHA256  PATH')

    foreach ($file in $payloadFiles) {
        $relative = $file.FullName.Substring($stagePath.Length).TrimStart('\', '/').Replace('\', '/')
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $sumLines.Add(('{0}  {1}' -f $hash, $relative))
        $manifestLines.Add(('{0}  {1}  {2}' -f $file.Length, $hash, $relative))
    }

    $utf8NoBom = New-Object System.Text.UTF8Encoding -ArgumentList $false
    [System.IO.File]::WriteAllText((Join-Path $stagePath 'SHA256SUMS.txt'), ($sumLines -join "`n") + "`n", $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $stagePath 'MANIFEST.txt'), ($manifestLines -join "`n") + "`n", $utf8NoBom)

    $zipPath = Join-Path $OutputDir ($stageName + '.zip')
    if (Test-Path -LiteralPath $zipPath) {
        if (Test-Path -LiteralPath $zipPath -PathType Container) {
            throw "ZIP 输出路径是目录，拒绝覆盖：$zipPath"
        }
        Remove-Item -LiteralPath $zipPath -Force
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $stagePath,
        $zipPath,
        [System.IO.Compression.CompressionLevel]::Optimal,
        $true
    )

    $zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $zipName = Split-Path -Leaf $zipPath
    $shaPath = $zipPath + '.sha256'
    [System.IO.File]::WriteAllText($shaPath, ('{0}  {1}' -f $zipHash, $zipName) + "`n", $utf8NoBom)

    $allFiles = Get-ChildItem -LiteralPath $stagePath -Recurse -File
    $payloadBytes = ($allFiles | Measure-Object -Property Length -Sum).Sum
    $zipSize = (Get-Item -LiteralPath $zipPath).Length

    Write-Host ''
    Write-Host '打包完成'
    Write-Host ('  版本          : {0}' -f $Version)
    Write-Host ('  主程序        : {0} ({1} 字节，上限 {2} 字节)' -f (Format-Bytes $exeSize), $exeSize, $SizeLimit)
    Write-Host ('  包内文件      : {0} 个，未压缩 {1}' -f $allFiles.Count, (Format-Bytes $payloadBytes))
    Write-Host ('  ZIP           : {0} ({1})' -f $zipPath, (Format-Bytes $zipSize))
    Write-Host ('  ZIP SHA256    : {0}' -f $zipHash)
    Write-Host ('  校验文件      : {0}' -f $shaPath)
    Write-Host '  外部 FFmpeg/RIFE: 未捆绑（符合预期）'
    Write-Host ''
}
catch {
    Write-Host ('错误：{0}' -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
finally {
    if ($stageRoot -and (Test-Path -LiteralPath $stageRoot)) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    Pop-Location
}
