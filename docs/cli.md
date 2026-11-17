# CLI 参考

本文描述 `smoothup` 的命令行界面与可预期行为。以 `smoothup --help`、`smoothup <子命令> --help` 为当前构建的最终准绳。

## 调用形式

```text
smoothup INPUT --platform douyin|bilibili [选项]
smoothup doctor [--verbose]
smoothup setup [--rife] [--verbose]
smoothup inspect FILE [--json]
smoothup verify FILE [--fps RATE] [--json] [--verbose]
```

不带子命令且给出输入文件时执行转换。所有子命令在启动时都不会自动联网；只有 `setup` 会下载依赖。

## 转换

```text
smoothup INPUT --platform douyin|bilibili
                [--fps 60|120|60000/1001|120000/1001]
                [--output FILE]
                [--engine auto|cpu|rife]
                [--quality balanced|high]
                [--preview SECONDS]
                [--report FILE]
                [--verbose]
                [--overwrite]
```

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `INPUT` | 必填 | 输入视频路径。 |
| `--platform` | 必填 | `douyin` 或 `bilibili`。影响命名与平台预设，输出均为 MP4，不执行上传。 |
| `--fps` | `60` | 只接受 `60`、`120`、`60000/1001`、`120000/1001` 四种目标帧率。 |
| `--output` | 自动命名 | 输出文件路径。自动命名为 `<stem>.<platform>.<rate>fps.mp4`。 |
| `--engine` | `auto` | `auto`：恒定帧率且找到 RIFE 引擎与模型时优先，否则 CPU；`cpu`：只用 CPU；`rife`：强制 RIFE，需恒定帧率输入。 |
| `--quality` | `balanced` | `balanced` 或 `high`。`high` 更慢、输出更大。 |
| `--preview` | 无 | 只处理输入开头 N 秒，输出一段独立预览短视频（不启动播放器）；N 必须是有限正数。 |
| `--report` | 无 | 额外写出 JSON 报告。 |
| `--verbose` | 关闭 | 输出更多阶段、命令与进度信息。 |
| `--overwrite` | 关闭 | 允许替换已存在的**输出**文件。源文件永远不能被覆盖。 |

### 默认输出名

输入 `/videos/clip.mp4`：

| 平台 | 帧率 | 默认输出 |
| --- | --- | --- |
| `bilibili` | `60` | `/videos/clip.bilibili.60fps.mp4` |
| `douyin` | `120` | `/videos/clip.douyin.120fps.mp4` |
| `bilibili` | `60000/1001` | `/videos/clip.bilibili.60000-1001fps.mp4` |
| `bilibili` | `120000/1001` | `/videos/clip.bilibili.120000-1001fps.mp4` |

有理帧率在文件名中把 `/` 换成 `-`，避免路径分隔符；容器内仍使用精确有理数，不把 59.94 或 119.88 四舍五入成 60/120。

带 `--preview N` 且未指定 `--output` 时，默认输出为 `<stem>.<platform>.<rate>fps.preview.mp4`，不会占用全片默认输出名。显式 `--output` 时按指定路径写出。

### 输出格式

- 容器：MP4。
- 视频：H.264 / libx264，`yuv420p`，SDR。
- 音频：AAC；保留源音频时序与起始偏移。
- 分辨率与画幅：跟随源，不强制放大；必要时按旋转/SAR 归一化并把尺寸补到偶数。
- 帧率：目标有理帧率，恒定帧率输出。
- HDR 源：实际做 HDR→SDR 色调映射，而不是只改标签。

### 引擎选择

- `auto` 不会因为 RIFE 存在就无条件使用；源帧率已不低于目标时不会做无意义插帧。
- `--engine rife` 在缺少 Vulkan、引擎或模型时**报错退出**，不会静默降级。这样便于确认产出到底由哪个引擎生成。
- CPU 路径使用运动补偿插帧并做场景切换保护；仍可能在快速运动、遮挡、字幕、闪白处产生伪影。先用 `--preview` 检查。

### 安全与发布

- 输出先写入同目录临时文件；只有完整成功并通过输出验证后才发布为最终文件。
- 默认拒绝覆盖任何已存在的输出；`--overwrite` 只放开输出，不放开源文件。
- 即使 `--output` 指向源文件，也会拒绝。
- Ctrl+C 会设置取消标志，终止子进程并清理临时文件；退出状态为 `130`。
- 失败时不会把未完成文件当作成功结果。

## `doctor`

```powershell
smoothup doctor
smoothup doctor --verbose
```

显示 FFmpeg、FFprobe、RIFE 引擎与模型路径，以及编码能力检查结果。RIFE 路径存在不代表 Vulkan/GPU 推理通过；请用 `--engine rife --preview 1` 做实际转换确认。RIFE 缺失不是致命错误，只要 CPU 路径可用。`--verbose` 显示诊断细节。

## `setup`

```powershell
smoothup setup
smoothup setup --rife
smoothup setup --rife --verbose
```

- 不带 `--rife`：安装固定版本的 FFmpeg（Windows essentials 构建）。
- 带 `--rife`：额外安装固定的 RIFE/ncnn/Vulkan 引擎，并且只保留 v4.6 模型。
- 安装到当前用户的管理目录（Windows 默认 `%LOCALAPPDATA%\SmoothUp`，依赖在 `deps\` 下）；可用 `SMOOTHUP_HOME` 覆盖。不需要管理员权限。
- 下载后校验固定 SHA256；校验失败不会安装。
- 部分安装失败不会删除已有的可用组件。
- 只有显式执行 `setup` 才联网；转换与 `doctor` 不会自动下载。

第三方版本、下载大小、安装后体积与许可证见 [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md)。

## `inspect`

```powershell
smoothup inspect .\output.mp4
smoothup inspect .\output.mp4 --json
```

读取容器与流信息。默认打印“媒体信息：<路径>”及人类可读摘要字段；`--json` 输出完整、格式化的 JSON，便于脚本处理。

## `verify`

```powershell
smoothup verify .\output.mp4
smoothup verify .\output.mp4 --fps 60
smoothup verify .\output.mp4 --fps 60000/1001 --json
smoothup verify .\output.mp4 --fps 60 --verbose
```

- 完整解码并检查容器、视频/音频流、时长、帧率与时序；给 `--fps` 时校验目标帧率。
- 默认打印“验证完成：<路径>”及人类可读摘要；`--json` 输出完整 JSON。
- 失败时非零退出；成功只说明文件结构与时序通过检查，**不证明**插帧观感、运动质量或平台播放帧率。
- `--verbose` 显示检查细节。

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `SMOOTHUP_HOME` | 覆盖当前用户的管理依赖目录。 |
| `SMOOTHUP_FFMPEG` | 显式指定 `ffmpeg` 可执行文件。 |
| `SMOOTHUP_FFPROBE` | 显式指定 `ffprobe` 可执行文件。 |
| `SMOOTHUP_RIFE` | 显式指定 RIFE 引擎可执行文件。 |
| `SMOOTHUP_RIFE_MODEL` | 显式指定 RIFE 模型目录。 |

发现顺序：显式环境变量 → 管理依赖目录 → `PATH`。显式环境变量无效时直接报错，不会静默回退到其他路径。

## 退出状态

| 状态 | 含义 |
| --- | --- |
| `0` | 成功。 |
| `1` | 依赖、媒体、验证或写出失败。 |
| `130` | 被 Ctrl+C 取消。 |
| `2` | 命令行语法或参数值无效。 |

## 示例

```powershell
# 默认 60fps，B 站命名
smoothup .\clip.mp4 --platform bilibili

# 抖音 60fps，先出 10 秒预览
smoothup .\clip.mp4 --platform douyin --fps 60 --preview 10 --output .\preview.mp4

# 强制 CPU，避免依赖 GPU
smoothup .\clip.mp4 --platform bilibili --fps 120 --engine cpu --report .\rife-not-used.json

# 检查并留档
smoothup verify .\clip.bilibili.60fps.mp4 --fps 60 --json --verbose
```
