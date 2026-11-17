<#
.SYNOPSIS
用真实 CLI 与 ffprobe 执行媒体集成冒烟检查。

.DESCRIPTION
覆盖：doctor、inspect、CPU 60fps 转换、输出编码/帧率/尺寸/音频/时长、
verify 成功与错误帧率拒绝、默认命名、特殊字符路径、既有输出拒绝、
--overwrite 仅覆盖输出、拒绝以源文件为输出、preview 命名与时长、
以及全程源文件哈希不变。

脚本会真实调用 smoothup.exe 与 ffmpeg/ffprobe，不是单元测试。
运行结果必须来自实际执行；不要在没有运行的情况下宣称通过。

.PARAMETER Smoothup
smoothup.exe 路径；默认 target\release\smoothup.exe 或 PATH。

.PARAMETER Ffmpeg
ffmpeg.exe 路径；默认 SMOOTHUP_FFMPEG 或 PATH。

.PARAMETER Ffprobe
ffprobe.exe 路径；默认 SMOOTHUP_FFPROBE 或 PATH。

.PARAMETER WorkDir
证据根目录，默认 .work\smoke。每次运行在其中创建唯一的 run-* 子目录；
不会清空或删除该目录下已有的用户内容。

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\media-smoke.ps1
#>
[CmdletBinding()]
param(
    [string]$Smoothup = '',
    [string]$Ffmpeg = $env:SMOOTHUP_FFMPEG,
    [string]$Ffprobe = $env:SMOOTHUP_FFPROBE,
    [string]$WorkDir = ''
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($WorkDir)) {
    $WorkDir = Join-Path $RepoRoot '.work\smoke'
}
if (-not [System.IO.Path]::IsPathRooted($WorkDir)) {
    $WorkDir = Join-Path $RepoRoot $WorkDir
}

function Resolve-Tool([string]$ExplicitPath, [string]$Name) {
    if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
        if (-not (Test-Path -LiteralPath $ExplicitPath)) {
            throw "指定的 $Name 不存在：$ExplicitPath"
        }
        return (Resolve-Path -LiteralPath $ExplicitPath).Path
    }
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    throw "找不到 $Name。请先运行 smoothup setup，或通过参数指定路径。"
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Equal($Actual, $Expected, [string]$Label) {
    if ([string]$Actual -ne [string]$Expected) {
        throw ("{0}：实际 '{1}'，期望 '{2}'" -f $Label, $Actual, $Expected)
    }
}

function Assert-Near([double]$Actual, [double]$Expected, [double]$Tolerance, [string]$Label) {
    if ([math]::Abs($Actual - $Expected) -gt $Tolerance) {
        throw ("{0}：实际 {1}，期望 {2} ± {3}" -f $Label, $Actual, $Expected, $Tolerance)
    }
}

function Invoke-Smoothup([string[]]$Arguments) {
    $errFile = [System.IO.Path]::GetTempFileName()
    try {
        $stdout = & $script:SmoothupPath @Arguments 2> $errFile | Out-String
        $exitCode = $LASTEXITCODE
        $stderr = ''
        if (Test-Path -LiteralPath $errFile) {
            $stderr = Get-Content -LiteralPath $errFile -Raw
            if ($null -eq $stderr) { $stderr = '' }
        }
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output = $stdout
            Error = $stderr
        }
    }
    finally {
        Remove-Item -LiteralPath $errFile -Force -ErrorAction SilentlyContinue
    }
}

function Get-Probe([string]$File) {
    $errFile = [System.IO.Path]::GetTempFileName()
    try {
        $json = & $script:FfprobePath -v error -show_entries 'stream=codec_type,codec_name,width,height,pix_fmt,avg_frame_rate,r_frame_rate,duration,nb_frames' -show_entries 'format=format_name,duration,size' -of json $File 2> $errFile | Out-String
        $code = $LASTEXITCODE
        if ($code -ne 0) {
            $err = Get-Content -LiteralPath $errFile -Raw -ErrorAction SilentlyContinue
            throw "ffprobe 失败（$File）：$err"
        }
        return ($json | ConvertFrom-Json)
    }
    finally {
        Remove-Item -LiteralPath $errFile -Force -ErrorAction SilentlyContinue
    }
}

function Get-Stream($Probe, [string]$Type) {
    return @($Probe.streams | Where-Object { $_.codec_type -eq $Type })[0]
}

function Invoke-Case([string]$Name, [scriptblock]$Body) {
    Write-Host ('[case] {0}' -f $Name)
    try {
        . $Body
        $script:CaseResults.Add([pscustomobject]@{ name = $Name; pass = $true; detail = '' })
        Write-Host '  PASS'
    }
    catch {
        $message = $_.Exception.Message
        $script:CaseResults.Add([pscustomobject]@{ name = $Name; pass = $false; detail = $message })
        Write-Host ('  FAIL：{0}' -f $message) -ForegroundColor Red
    }
}

try {
    if ($WorkDir -eq $RepoRoot) {
        throw 'WorkDir 不能是仓库根目录。'
    }
    if (Test-Path -LiteralPath $WorkDir) {
        if (-not (Test-Path -LiteralPath $WorkDir -PathType Container)) {
            throw "WorkDir 不是目录：$WorkDir"
        }
    }
    else {
        New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
    }
    $workRoot = (Resolve-Path -LiteralPath $WorkDir).Path
    if ($workRoot -eq $RepoRoot) {
        throw 'WorkDir 不能解析为仓库根目录。'
    }
    $runId = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $WorkDir = Join-Path $workRoot ('run-' + $runId)
    if (Test-Path -LiteralPath $WorkDir) {
        throw "本次运行目录已存在，拒绝复用：$WorkDir"
    }
    New-Item -ItemType Directory -Path $WorkDir | Out-Null

    if ([string]::IsNullOrWhiteSpace($Smoothup)) {
        $candidate = Join-Path $RepoRoot 'target\release\smoothup.exe'
        if (Test-Path -LiteralPath $candidate) {
            $Smoothup = $candidate
        }
    }
    $script:SmoothupPath = Resolve-Tool $Smoothup 'smoothup'
    $script:FfmpegPath = Resolve-Tool $Ffmpeg 'ffmpeg'
    $script:FfprobePath = Resolve-Tool $Ffprobe 'ffprobe'
    $env:SMOOTHUP_FFMPEG = $script:FfmpegPath
    $env:SMOOTHUP_FFPROBE = $script:FfprobePath

    $sourceDir = Join-Path $WorkDir 'input dir with spaces'
    New-Item -ItemType Directory -Path $sourceDir -Force | Out-Null
    $source = Join-Path $sourceDir 'clip & test.mp4'
    $sourceLog = Join-Path $WorkDir 'source-ffmpeg.log'
    $sourceArgs = @(
        '-y', '-hide_banner', '-loglevel', 'error',
        '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=30:duration=2',
        '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=2',
        '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', '-pix_fmt', 'yuv420p',
        '-c:a', 'aac', '-b:a', '128k', '-shortest',
        $source
    )
    & $script:FfmpegPath @sourceArgs 2> $sourceLog | Out-Null
    if ($LASTEXITCODE -ne 0) {
        $tail = Get-Content -LiteralPath $sourceLog -Tail 30 -ErrorAction SilentlyContinue
        throw ("生成源视频失败：" + ($tail -join "`n"))
    }

    $sourceHashBefore = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    $sourceProbe = Get-Probe $source
    $sourceVideo = Get-Stream $sourceProbe 'video'
    if (-not $sourceVideo) { throw '源视频缺少视频流。' }
    Assert-Equal $sourceVideo.avg_frame_rate '30/1' '源视频帧率'

    $out60 = Join-Path $WorkDir 'converted-60fps.mp4'
    $report = Join-Path $WorkDir 'report.json'
    $verifyJson = Join-Path $WorkDir 'verify.json'
    $script:CaseResults = New-Object System.Collections.Generic.List[object]

    Invoke-Case 'doctor 可执行且不因缺少 RIFE 失败' {
        $result = Invoke-Smoothup @('doctor', '--verbose')
        [System.IO.File]::WriteAllText((Join-Path $WorkDir 'doctor.log'), $result.Output + "`n" + $result.Error)
        Assert-True ($result.ExitCode -eq 0) ("doctor 退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        Assert-True ($result.Output.Length -gt 0) 'doctor 没有输出'
    }

    Invoke-Case 'inspect --json 输出可解析 JSON' {
        $result = Invoke-Smoothup @('inspect', $source, '--json')
        Assert-True ($result.ExitCode -eq 0) ("inspect 退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        $parsed = $result.Output | ConvertFrom-Json
        Assert-True ($null -ne $parsed) 'inspect JSON 为空'
    }

    Invoke-Case 'CPU 60fps 转换成功并写报告' {
        $result = Invoke-Smoothup @(
            $source, '--platform', 'bilibili', '--fps', '60',
            '--engine', 'cpu', '--output', $out60, '--report', $report, '--verbose'
        )
        Assert-True ($result.ExitCode -eq 0) ("转换退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        Assert-True (Test-Path -LiteralPath $out60) '转换没有生成输出文件'
        Assert-True ((Get-Item -LiteralPath $out60).Length -gt 0) '输出文件为空'
        Assert-True (Test-Path -LiteralPath $report) '没有生成 report.json'
        $reportJson = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        Assert-True ($null -ne $reportJson) 'report.json 无法解析'
    }

    Invoke-Case 'ffprobe 检查输出编码、帧率、尺寸、音频与时长' {
        $probe = Get-Probe $out60
        $video = Get-Stream $probe 'video'
        $audio = Get-Stream $probe 'audio'
        Assert-True ($null -ne $video) '输出缺少视频流'
        Assert-True ($null -ne $audio) '输出缺少音频流'
        Assert-Equal $video.codec_name 'h264' '视频编码'
        Assert-Equal $video.pix_fmt 'yuv420p' '像素格式'
        Assert-Equal $video.avg_frame_rate '60/1' '输出帧率'
        Assert-Equal ([int]($video.width)) ([int]($sourceVideo.width)) '输出宽度'
        Assert-Equal ([int]($video.height)) ([int]($sourceVideo.height)) '输出高度'
        Assert-True ((([int]($video.width) % 2) -eq 0) -and (([int]($video.height) % 2) -eq 0)) '输出尺寸不是偶数'
        Assert-Equal $audio.codec_name 'aac' '音频编码'
        Assert-True ($probe.format.format_name -match 'mp4') '输出不是 MP4 容器'
        Assert-Near ([double]($probe.format.duration)) ([double]($sourceProbe.format.duration)) 0.25 '输出总时长'
        $videoDuration = if ($null -ne $video.duration) { [double]($video.duration) } else { [double]($probe.format.duration) }
        $audioDuration = if ($null -ne $audio.duration) { [double]($audio.duration) } else { [double]($probe.format.duration) }
        Assert-Near $audioDuration $videoDuration 0.25 '音视频时长'
        [System.IO.File]::WriteAllText((Join-Path $WorkDir 'output-probe.json'), ($probe | ConvertTo-Json -Depth 8))
    }

    Invoke-Case 'verify 接受正确帧率并输出 JSON' {
        $result = Invoke-Smoothup @('verify', $out60, '--fps', '60', '--json')
        [System.IO.File]::WriteAllText($verifyJson, $result.Output + "`n" + $result.Error)
        Assert-True ($result.ExitCode -eq 0) ("verify 退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        $parsed = $result.Output | ConvertFrom-Json
        Assert-True ($null -ne $parsed) 'verify JSON 为空'
    }

    Invoke-Case 'verify 拒绝错误帧率' {
        $result = Invoke-Smoothup @('verify', $source, '--fps', '60', '--json')
        Assert-True ($result.ExitCode -ne 0) '30fps 源视频在期望 60fps 时应失败'
    }

    Invoke-Case '默认命名、特殊字符路径且不覆盖源文件' {
        $defaultOut = Join-Path $sourceDir 'clip & test.bilibili.60fps.mp4'
        $result = Invoke-Smoothup @($source, '--platform', 'bilibili', '--fps', '60', '--engine', 'cpu')
        Assert-True ($result.ExitCode -eq 0) ("默认命名转换退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        Assert-True (Test-Path -LiteralPath $defaultOut) ("默认输出不存在：{0}" -f $defaultOut)
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $hash $sourceHashBefore '源文件哈希'
    }

    Invoke-Case '既有输出默认拒绝覆盖' {
        $defaultOut = Join-Path $sourceDir 'clip & test.bilibili.60fps.mp4'
        $before = (Get-FileHash -LiteralPath $defaultOut -Algorithm SHA256).Hash
        $result = Invoke-Smoothup @($source, '--platform', 'bilibili', '--fps', '60', '--engine', 'cpu')
        Assert-True ($result.ExitCode -ne 0) '已存在输出时应失败'
        $after = (Get-FileHash -LiteralPath $defaultOut -Algorithm SHA256).Hash
        Assert-Equal $after $before '拒绝覆盖时输出文件不应改变'
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $hash $sourceHashBefore '源文件哈希'
    }

    Invoke-Case '--overwrite 允许替换输出但不改源文件' {
        $defaultOut = Join-Path $sourceDir 'clip & test.bilibili.60fps.mp4'
        $result = Invoke-Smoothup @($source, '--platform', 'bilibili', '--fps', '60', '--engine', 'cpu', '--overwrite')
        Assert-True ($result.ExitCode -eq 0) ("--overwrite 退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $hash $sourceHashBefore '源文件哈希'
    }

    Invoke-Case '拒绝把源文件作为输出' {
        $result = Invoke-Smoothup @($source, '--platform', 'bilibili', '--fps', '60', '--output', $source, '--overwrite', '--engine', 'cpu')
        Assert-True ($result.ExitCode -ne 0) '--output 指向源文件时应失败'
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $hash $sourceHashBefore '源文件哈希'
        $probe = Get-Probe $source
        $sourceNowVideo = Get-Stream $probe 'video'
        Assert-Equal $sourceNowVideo.avg_frame_rate '30/1' '源文件帧率仍应为 30'
    }

    Invoke-Case 'preview 默认命名且时长约为 1 秒' {
        $previewOut = Join-Path $sourceDir 'clip & test.bilibili.60fps.preview.mp4'
        $result = Invoke-Smoothup @($source, '--platform', 'bilibili', '--fps', '60', '--preview', '1', '--engine', 'cpu')
        Assert-True ($result.ExitCode -eq 0) ("preview 退出码 {0}：{1}" -f $result.ExitCode, $result.Error.Trim())
        Assert-True (Test-Path -LiteralPath $previewOut) ("预览输出不存在：{0}" -f $previewOut)
        $defaultOut = Join-Path $sourceDir 'clip & test.bilibili.60fps.mp4'
        Assert-True (Test-Path -LiteralPath $defaultOut) '预览不应占用或删除全片默认输出'
        $probe = Get-Probe $previewOut
        $video = Get-Stream $probe 'video'
        $audio = Get-Stream $probe 'audio'
        Assert-True ($null -ne $video) '预览缺少视频流'
        Assert-True ($null -ne $audio) '预览缺少音频流'
        Assert-Equal $video.avg_frame_rate '60/1' '预览帧率'
        Assert-True (([double]($probe.format.duration)) -le 1.35) ("预览时长过长：{0}" -f $probe.format.duration)
        Assert-True (([double]($probe.format.duration)) -ge 0.5) ("预览时长过短：{0}" -f $probe.format.duration)
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $hash $sourceHashBefore '源文件哈希'
    }

    Invoke-Case '全程源文件未被修改' {
        $sourceHashAfter = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Equal $sourceHashAfter $sourceHashBefore '源文件最终哈希'
        $probe = Get-Probe $source
        $finalVideo = Get-Stream $probe 'video'
        Assert-Equal $finalVideo.avg_frame_rate '30/1' '源文件帧率'
    }

    $passed = @($script:CaseResults | Where-Object { $_.pass }).Count
    $failed = @($script:CaseResults | Where-Object { -not $_.pass }).Count
    $summary = [ordered]@{
        generated_at = (Get-Date).ToString('o')
        smoothup = $script:SmoothupPath
        ffmpeg = $script:FfmpegPath
        ffprobe = $script:FfprobePath
        work_dir = $WorkDir
        passed = $passed
        failed = $failed
        cases = $script:CaseResults
    }
    $summaryPath = Join-Path $WorkDir 'smoke-summary.json'
    $utf8NoBom = New-Object System.Text.UTF8Encoding -ArgumentList $false
    [System.IO.File]::WriteAllText($summaryPath, (($summary | ConvertTo-Json -Depth 6) + "`n"), $utf8NoBom)

    Write-Host ''
    Write-Host ('冒烟检查完成：通过 {0}，失败 {1}' -f $passed, $failed)
    Write-Host ('工作目录与证据：{0}' -f $WorkDir)
    Write-Host ('汇总：{0}' -f $summaryPath)
    if ($failed -gt 0) { exit 1 }
}
catch {
    Write-Host ('错误：{0}' -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
