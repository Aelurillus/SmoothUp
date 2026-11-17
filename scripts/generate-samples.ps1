<#
.SYNOPSIS
生成用于人工平台验证的 30/60/120fps 编号运动样片。

.DESCRIPTION
每个样片包含：
- 每帧递增的帧号与时间戳，便于慢动作录像后数唯一帧；
- 每 2 秒一次硬切场景，检验插帧场景保护；
- 每秒一次音频提示与画面闪烁，切点额外双重提示，便于检查音画同步；
- 底部横向扫描条，便于观察运动连续性。

输出默认写入 .work/samples，不进入仓库。需要 FFmpeg/FFprobe。

.PARAMETER Ffmpeg
ffmpeg.exe 路径；默认读取 SMOOTHUP_FFMPEG 或 PATH。

.PARAMETER Ffprobe
ffprobe.exe 路径；默认读取 SMOOTHUP_FFPROBE 或 PATH。

.PARAMETER Force
覆盖目录中已存在的 smoothup-sample-*.mp4；不会递归清空目录。

.PARAMETER KeepFrames
保留 PNG 帧目录用于诊断；默认在编码成功后删除本次唯一的临时帧目录。

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\generate-samples.ps1
#>
[CmdletBinding()]
param(
    [string]$OutDir = '',
    [int]$Seconds = 6,
    [int]$Width = 1280,
    [int]$Height = 720,
    [int[]]$Fps = @(30, 60, 120),
    [string]$Ffmpeg = $env:SMOOTHUP_FFMPEG,
    [string]$Ffprobe = $env:SMOOTHUP_FFPROBE,
    [switch]$KeepFrames,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $RepoRoot '.work\samples'
}
if (-not [System.IO.Path]::IsPathRooted($OutDir)) {
    $OutDir = Join-Path $RepoRoot $OutDir
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
    throw "找不到 $Name。请先运行 smoothup setup，或通过 -Ffmpeg/-Ffprobe 指定路径。"
}

function Format-Bytes([long]$Bytes) {
    if ($Bytes -ge 1MB) { return ('{0:N2} MiB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N2} KiB' -f ($Bytes / 1KB)) }
    return ('{0} B' -f $Bytes)
}

function New-MarkWav([string]$Path, [int]$DurationSeconds, [int]$SampleRate = 48000) {
    $total = [int]($DurationSeconds * $SampleRate)
    $data = New-Object 'System.Int16[]' -ArgumentList $total

    function Add-Tone([double]$StartSec, [double]$Frequency, [double]$DurationSec, [double]$Amplitude) {
        $start = [int]($StartSec * $SampleRate)
        $count = [int]($DurationSec * $SampleRate)
        $fade = [int]($SampleRate * 0.005)
        if ($fade -lt 1) { $fade = 1 }
        for ($i = 0; $i -lt $count; $i++) {
            $index = $start + $i
            if ($index -lt 0 -or $index -ge $total) { continue }
            $env = 1.0
            if ($i -lt $fade) { $env = $i / [double]$fade }
            elseif ($i -gt ($count - $fade)) { $env = ($count - $i) / [double]$fade }
            $value = $Amplitude * $env * [math]::Sin(2.0 * [math]::PI * $Frequency * ($i / [double]$SampleRate))
            $mixed = ($data[$index] / 32767.0) + $value
            if ($mixed -gt 1.0) { $mixed = 1.0 }
            if ($mixed -lt -1.0) { $mixed = -1.0 }
            $data[$index] = [int16][math]::Round($mixed * 32767.0)
        }
    }

    for ($second = 0; $second -lt $DurationSeconds; $second++) {
        Add-Tone $second 1000.0 0.040 0.65
    }
    for ($second = 2; $second -lt $DurationSeconds; $second += 2) {
        Add-Tone ($second + 0.25) 2000.0 0.040 0.65
    }

    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create)
    $writer = New-Object System.IO.BinaryWriter -ArgumentList $stream
    try {
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes('RIFF'))
        $writer.Write([int](36 + $total * 2))
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes('WAVE'))
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes('fmt '))
        $writer.Write([int]16)
        $writer.Write([int16]1)
        $writer.Write([int16]1)
        $writer.Write([int]$SampleRate)
        $writer.Write([int]($SampleRate * 2))
        $writer.Write([int16]2)
        $writer.Write([int16]16)
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes('data'))
        $writer.Write([int]($total * 2))
        for ($i = 0; $i -lt $total; $i++) {
            $writer.Write([int16]$data[$i])
        }
    }
    finally {
        $writer.Dispose()
        $stream.Dispose()
    }
}

function New-FrameSet([int]$TargetFps, [string]$FrameDir) {
    if (Test-Path -LiteralPath $FrameDir) {
        throw "帧目录已存在，拒绝删除或复用：$FrameDir"
    }
    New-Item -ItemType Directory -Path $FrameDir -Force | Out-Null

    $totalFrames = [int]($Seconds * $TargetFps)
    $backgrounds = @(
        [System.Drawing.Color]::FromArgb(18, 32, 58),
        [System.Drawing.Color]::FromArgb(24, 56, 40),
        [System.Drawing.Color]::FromArgb(64, 28, 38)
    )
    $accent = [System.Drawing.Color]::FromArgb(250, 250, 250)
    $muted = [System.Drawing.Color]::FromArgb(190, 205, 220)
    $barColor = [System.Drawing.Color]::FromArgb(80, 200, 255)
    $cutColor = [System.Drawing.Color]::FromArgb(255, 80, 80)

    $bigFont = New-Object System.Drawing.Font -ArgumentList @('Arial', [float]([math]::Max(48, [int]($Width * 0.085))), [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $midFont = New-Object System.Drawing.Font -ArgumentList @('Arial', [float]([math]::Max(20, [int]($Width * 0.026))), [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
    $smallFont = New-Object System.Drawing.Font -ArgumentList @('Arial', [float]([math]::Max(16, [int]($Width * 0.019))), [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $whiteBrush = New-Object System.Drawing.SolidBrush -ArgumentList $accent
    $mutedBrush = New-Object System.Drawing.SolidBrush -ArgumentList $muted
    $barBrush = New-Object System.Drawing.SolidBrush -ArgumentList $barColor
    $cutBrush = New-Object System.Drawing.SolidBrush -ArgumentList $cutColor
    $panelBrush = New-Object System.Drawing.SolidBrush -ArgumentList ([System.Drawing.Color]::FromArgb(120, 0, 0, 0))
    $center = New-Object System.Drawing.StringFormat
    $center.Alignment = [System.Drawing.StringAlignment]::Center
    $center.LineAlignment = [System.Drawing.StringAlignment]::Center

    try {
        for ($n = 0; $n -lt $totalFrames; $n++) {
            $time = $n / [double]$TargetFps
            $scene = [int][math]::Floor($time / 2.0)
            $bg = $backgrounds[$scene % $backgrounds.Count]
            $bitmap = New-Object System.Drawing.Bitmap -ArgumentList $Width, $Height
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
                $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
                $graphics.Clear($bg)

                for ($band = 0; $band -lt 8; $band++) {
                    $x = (($n * 3) + ($band * 160)) % ($Width + 200) - 200
                    $graphics.FillRectangle($panelBrush, $x, ($band * [int]($Height / 8)), 120, [int]($Height / 8))
                }

                $phase = $n % $TargetFps
                $flashFrames = [math]::Max(1, [int]($TargetFps * 0.06))
                if ($phase -lt $flashFrames) {
                    $flashBrush = $whiteBrush
                    if (($n % ($TargetFps * 2)) -lt $flashFrames -and $scene -gt 0) {
                        $flashBrush = $cutBrush
                    }
                    $graphics.FillRectangle($flashBrush, 24, 24, 96, 96)
                }

                $sceneTime = $time % 2.0
                if ($scene -eq 0) {
                    $graphics.DrawString('START', $smallFont, $whiteBrush, 140, 40)
                }
                elseif ($sceneTime -lt 0.30) {
                    $graphics.DrawString('CUT', $smallFont, $cutBrush, 140, 40)
                }
                else {
                    $graphics.DrawString(('SCENE {0}' -f $scene), $smallFont, $whiteBrush, 140, 40)
                }

                $fpsText = 'FPS {0}  |  {1:0.000} s' -f $TargetFps, $time
                $graphics.DrawString($fpsText, $midFont, $mutedBrush, 24, 140)
                $graphics.DrawString(('AUDIO TICK {0} / s' -f 1), $smallFont, $mutedBrush, 24, 180)

                $numberText = 'FRAME {0:D6}' -f $n
                $numberRect = New-Object System.Drawing.RectangleF -ArgumentList @([float]0, [float]($Height * 0.40), [float]$Width, [float]($Height * 0.18))
                $graphics.DrawString($numberText, $bigFont, $whiteBrush, $numberRect, $center)

                $barX = [int](($n % $TargetFps) / [double]$TargetFps * ($Width - 96))
                $graphics.FillRectangle($barBrush, $barX, [int]($Height - 140), 72, 72)
                $graphics.FillRectangle($barBrush, 24, [int]($Height - 60), [int](($n / [double]$totalFrames) * ($Width - 48)), 16)

                $label = 'SmoothUp manual playback sample  |  {0}x{1}  |  {2}fps' -f $Width, $Height, $TargetFps
                $graphics.DrawString($label, $smallFont, $mutedBrush, 24, [int]($Height - 40))

                $framePath = Join-Path $FrameDir ('frame_{0:D6}.png' -f $n)
                $bitmap.Save($framePath, [System.Drawing.Imaging.ImageFormat]::Png)
            }
            finally {
                $graphics.Dispose()
                $bitmap.Dispose()
            }
        }
    }
    finally {
        $bigFont.Dispose()
        $midFont.Dispose()
        $smallFont.Dispose()
        $whiteBrush.Dispose()
        $mutedBrush.Dispose()
        $barBrush.Dispose()
        $cutBrush.Dispose()
        $panelBrush.Dispose()
        $center.Dispose()
    }
}

function Invoke-Ffmpeg([string[]]$Arguments, [string]$LogPath) {
    & $script:FfmpegPath @Arguments 2>&1 | Out-File -FilePath $LogPath -Encoding utf8
    if ($LASTEXITCODE -ne 0) {
        if (Test-Path -LiteralPath $LogPath) {
            Get-Content -LiteralPath $LogPath -Tail 40 | ForEach-Object { Write-Host $_ }
        }
        throw "FFmpeg 失败，退出码 $LASTEXITCODE；日志：$LogPath"
    }
}

try {
    if ($Seconds -lt 2) { throw 'Seconds 至少为 2。' }
    if ($Width -lt 320 -or $Height -lt 240) { throw 'Width/Height 太小。' }
    if (($Width % 2) -ne 0 -or ($Height % 2) -ne 0) { throw 'Width/Height 必须为偶数。' }
    if ($Fps.Count -eq 0) { throw '至少指定一个帧率。' }
    foreach ($rate in $Fps) {
        if ($rate -le 0) { throw "无效帧率：$rate" }
    }

    $script:FfmpegPath = Resolve-Tool $Ffmpeg 'ffmpeg'
    $ffprobePath = Resolve-Tool $Ffprobe 'ffprobe'
    try {
        Add-Type -AssemblyName System.Drawing
    }
    catch {
        Add-Type -AssemblyName System.Drawing.Common
    }

    if (-not (Test-Path -LiteralPath $OutDir)) {
        New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    }
    $OutDir = (Resolve-Path -LiteralPath $OutDir).Path

    $existing = Get-ChildItem -LiteralPath $OutDir -Filter 'smoothup-sample-*.mp4' -File -ErrorAction SilentlyContinue
    if ($existing -and -not $Force) {
        throw "输出目录已有样片；使用 -Force 覆盖：$OutDir"
    }

    $wavPath = Join-Path $OutDir 'smoothup-sample-marks.wav'
    if ((Test-Path -LiteralPath $wavPath) -and -not $Force) {
        throw "提示音文件已存在；使用 -Force 覆盖：$wavPath"
    }
    Write-Host ('[audio] 写入提示音：{0}' -f $wavPath)
    New-MarkWav -Path $wavPath -DurationSeconds $Seconds

    $ffmpegVersion = (& $script:FfmpegPath -version 2>&1 | Select-Object -First 1)
    $records = New-Object System.Collections.Generic.List[object]
    $runId = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)

    foreach ($rate in $Fps) {
        $frameDir = Join-Path $OutDir ('.smoothup-frames-{0}-{1}fps' -f $runId, $rate)
        $logPath = Join-Path $OutDir ('encode-{0}fps.log' -f $rate)
        $outputName = 'smoothup-sample-{0}fps.mp4' -f $rate
        $outputPath = Join-Path $OutDir $outputName

        Write-Host ('[draw] {0}fps：绘制 {1} 帧' -f $rate, ($Seconds * $rate))
        New-FrameSet -TargetFps $rate -FrameDir $frameDir

        Write-Host ('[encode] {0}fps -> {1}' -f $rate, $outputPath)
        $ffmpegArgs = @(
            '-y', '-hide_banner', '-loglevel', 'warning',
            '-framerate', ([string]$rate),
            '-i', (Join-Path $frameDir 'frame_%06d.png'),
            '-i', $wavPath,
            '-map', '0:v:0', '-map', '1:a:0',
            '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '18', '-pix_fmt', 'yuv420p',
            '-r', ([string]$rate), '-fps_mode', 'cfr',
            '-c:a', 'aac', '-b:a', '192k',
            '-movflags', '+faststart',
            '-shortest',
            $outputPath
        )
        Invoke-Ffmpeg -Arguments $ffmpegArgs -LogPath $logPath

        $probeText = & $ffprobePath -v error -show_entries 'stream=codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,nb_frames,duration' -show_entries 'format=duration,size' -of json $outputPath | Out-String
        if ($LASTEXITCODE -ne 0) { throw "ffprobe 无法读取 $outputPath" }
        $probe = $probeText | ConvertFrom-Json
        $video = @($probe.streams | Where-Object { $_.codec_type -eq 'video' })[0]
        $audio = @($probe.streams | Where-Object { $_.codec_type -eq 'audio' })[0]
        if (-not $video) { throw "$outputPath 缺少视频流" }
        if (-not $audio) { throw "$outputPath 缺少音频流" }
        $expectedRate = '{0}/1' -f $rate
        if ($video.avg_frame_rate -ne $expectedRate) {
            throw "$outputPath 帧率异常：$($video.avg_frame_rate)，期望 $expectedRate"
        }
        $duration = [double]($probe.format.duration)
        if ([math]::Abs($duration - $Seconds) -gt 0.25) {
            throw "$outputPath 时长异常：$duration，期望约 $Seconds 秒"
        }
        $videoBytes = (Get-Item -LiteralPath $outputPath).Length
        $hash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $records.Add([ordered]@{
            file = $outputName
            fps = $rate
            width = [int]($video.width)
            height = [int]($video.height)
            seconds = [math]::Round($duration, 3)
            video_codec = $video.codec_name
            audio_codec = $audio.codec_name
            bytes = $videoBytes
            sha256 = $hash
            ffmpeg = $ffmpegVersion
        })

        if (-not $KeepFrames) {
            Remove-Item -LiteralPath $frameDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        Write-Host ('[ok] {0}  {1}  {2}  {3}' -f $outputName, (Format-Bytes $videoBytes), $hash.Substring(0, 16), $duration)
    }

    $manifest = [ordered]@{
        generated_at = (Get-Date).ToString('o')
        width = $Width
        height = $Height
        seconds = $Seconds
        audio_marks = '1 kHz every second; 2 kHz double mark at cut +0.25s; 2s scene cuts'
        ffmpeg = $ffmpegVersion
        samples = $records
    }
    $manifestPath = Join-Path $OutDir 'samples-manifest.json'
    $json = $manifest | ConvertTo-Json -Depth 6
    $utf8NoBom = New-Object System.Text.UTF8Encoding -ArgumentList $false
    [System.IO.File]::WriteAllText($manifestPath, $json + "`n", $utf8NoBom)

    Write-Host ''
    Write-Host '样片生成完成'
    Write-Host ('  输出目录：{0}' -f $OutDir)
    Write-Host ('  清单    ：{0}' -f $manifestPath)
    Write-Host '  请用 smoothup verify 检查后再上传；平台播放仍需人工测量。'
}
catch {
    Write-Host ('错误：{0}' -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
