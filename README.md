# SmoothUp

Windows 优先的轻量命令行工具：把普通视频转换为适合抖音 / 哔哩哔哩上传的 60fps 或 120fps MP4。没有 GUI、没有后台服务、不自动上传。

> **验证阶段（请先读）**
>
> 1. **导出成功**：本地文件通过 `smoothup verify` / `ffprobe` 检查。
> 2. **上传入口**：平台页面完成文件接收或保存草稿，尚未提交发布。
> 3. **发布/转码分发**：提交发布（可仅自己可见）成功，平台完成转码并给出可访问稿件/分发结果。
> 4. **实际显示**：在真实设备与客户端上测得目标帧率及音画同步。
>
> 当前：原生 **3840×2160、30fps** 动态片段已由 SmoothUp RIFE 补帧为 **4K120、8 秒、960 帧**，两平台私密投稿成功。B 站所选“4K 超高清”正式流保持 4K120 和原时长，浏览器 1 倍速计数约 120.59fps，测量窗口掉帧 4；抖音同片与 4K60 对照当前均为 720p30，仍处于官方所述高清转码窗口内。手机 App、物理屏幕唯一帧数与音画同步尚未测量。完整解码、校验值、复现命令及限制见 [平台上传与人工播放验证](docs/platform-verification.md) 第 8.6 节。
>
> 4K 验收必须使用原生 4K 素材，转换保持输入分辨率；不把 1080p 放大或原生高帧率合成片当作实际补帧成品。此前“1K 高帧率”按 1920×1080、60/120fps 验收，其中 B 站已测得 1080p60；历史记录见上述文档第 8.2–8.5 节。
>
> 抖音补充对照：12 秒的原生 1080p60 对照在公开审核完成后仍只返回最高 720p30，补帧对照的所选正式流也为 720p30；`-itsscale 2` 对照则变成 24 秒慢放，未恢复原速度。尚无通过验证的抖音 1080p60 发布路径，编码预设不变；测试作品最终均已恢复或保持私密。证据与方法边界见上述记录第 8.4 节。
>
> 后续直式 H.264（1080×1920）与横式 HEVC（1920×1080）的 60fps、12 秒私密对照也都被分发为 720p30；正式流完整解码均为 361 帧，浏览器约 29.78 / 29.77fps。仍未取得抖音 1080p60 路径；详见上述记录第 8.5 节。

## 功能

- CPU 运动补偿插帧，带场景切换保护；源帧率已不低于目标时不做无意义插帧。
- 可选 RIFE / ncnn / Vulkan GPU 引擎；不可用时明确报错或按 `auto` 使用 CPU，不伪造插帧。
- 精确有理帧率：仅支持 `60`、`120`、`60000/1001`、`120000/1001`，避免可变帧率漂移。
- 输出 H.264/libx264、AAC、yuv420p、SDR；保持原画幅、旋转与音频时序，不强制放大。
- 默认不覆盖源文件；输出发布前验证；支持 `--preview`、`--report`、Ctrl+C 清理。
- `doctor` 检查依赖，`inspect` 查看媒体信息，`verify` 做完整解码与时序检查。

## 快速开始（Windows）

### 1. 获取程序

从发布页下载 `SmoothUp-<版本>-windows-x64.zip` 与对应的 `.zip.sha256`，先核对 ZIP 的 SHA256，再解压并用包内 `SHA256SUMS.txt` 核对各文件。发布包不包含 FFmpeg 或 RIFE。

也可以从源码构建，见 [构建与打包](docs/build.md)。

### 2. 安装运行依赖

在解压目录打开 PowerShell：

```powershell
powershell -ExecutionPolicy Bypass -File .\setup.ps1
.\smoothup.exe doctor
```

`setup` 从上游固定版本下载 FFmpeg（发布包不内置），校验 SHA256 后安装到当前用户的管理目录（Windows 默认 `%LOCALAPPDATA%\SmoothUp`）；只有显式执行安装时才联网。需要 GPU 插帧时：

```powershell
.\smoothup.exe setup --rife
.\smoothup.exe doctor
```

也可以直接运行随包脚本：`.\setup.ps1 -Rife`。

RIFE 发行 ZIP 约 431.5 MB，但只保留引擎、DLL 与 v4.6 模型，安装后约 17 MiB。FFmpeg 压缩包约 114.8 MB；两者都不随 SmoothUp 发布包分发。可用 `SMOOTHUP_HOME` 指定管理目录。

如果你已经有可信的 FFmpeg，可设置 `SMOOTHUP_FFMPEG`、`SMOOTHUP_FFPROBE`（RIFE 对应 `SMOOTHUP_RIFE`、`SMOOTHUP_RIFE_MODEL`）指向实际文件。显式路径无效时会报错，不会静默改用 PATH。

### 3. 转换为 60fps

```powershell
.\smoothup.exe .\input.mp4 --platform bilibili --fps 60
```

默认输出与源文件同目录，命名为 `<原名>.bilibili.60fps.mp4`，例如 `input.bilibili.60fps.mp4`。有理帧率把 `/` 写成 `-`，例如 `input.bilibili.60000-1001fps.mp4`。预览默认输出为 `<原名>.<平台>.<帧率>fps.preview.mp4`。源文件不会被覆盖。

常用变体：

```powershell
# 抖音，60fps，显式输出
.\smoothup.exe .\input.mp4 --platform douyin --fps 60 --output .\input.douyin.60fps.mp4

# 只转换开头 10 秒为预览，输出 input.bilibili.60fps.preview.mp4
.\smoothup.exe .\input.mp4 --platform bilibili --fps 60 --preview 10

# 120fps 实验输出，并写 JSON 报告
.\smoothup.exe .\input.mp4 --platform bilibili --fps 120 --report .\report.json

# NTSC 有理帧率
.\smoothup.exe .\input.mp4 --platform bilibili --fps 60000/1001

# 指定 CPU 引擎；auto 会在 RIFE 可用时优先使用 RIFE
.\smoothup.exe .\input.mp4 --platform douyin --fps 60 --engine cpu

# 覆盖已有输出（仍拒绝覆盖源文件）
.\smoothup.exe .\input.mp4 --platform douyin --fps 60 --overwrite
```

### 4. 本地验收

```powershell
.\smoothup.exe inspect .\input.bilibili.60fps.mp4
.\smoothup.exe verify .\input.bilibili.60fps.mp4 --fps 60
.\smoothup.exe verify .\input.bilibili.60fps.mp4 --fps 60 --json
```

`verify` 成功只表示文件可完整解码、容器/流时序与目标帧率一致；它**不证明**运动估计质量，也不证明平台会以高帧率播放。`--json` 输出适合留档。

### 5. 上传与播放验证

按 [平台上传与人工播放验证](docs/platform-verification.md) 操作：记录源文件 SHA256、上传时间、作品 URL、设备型号、屏幕刷新率、App/浏览器版本，并用慢动作录像或逐帧检查确认实际播放帧率与音画同步。没有证据就保持“未验证”。

## CLI 速查

```text
smoothup INPUT --platform douyin|bilibili [--fps RATE] [--output FILE]
                [--engine auto|cpu|rife] [--quality balanced|high]
                [--preview SECONDS] [--report FILE] [--verbose] [--overwrite]
smoothup doctor [--verbose]
smoothup setup [--rife]
smoothup inspect FILE [--json]
smoothup verify FILE [--fps RATE] [--json]
```

| 选项 | 说明 |
| --- | --- |
| `--platform` | 必填。只影响命名与预设，不执行上传。 |
| `--fps` | 目标帧率，默认 `60`；只接受 `60`、`120`、`60000/1001`、`120000/1001`。 |
| `--output` | 输出文件。默认 `<stem>.<platform>.<rate>fps.mp4`；拒绝指向源文件。 |
| `--engine` | `auto`（默认）、`cpu`、`rife`。`rife` 不可用时不会静默退回 CPU。 |
| `--quality` | `balanced`（默认）或 `high`；高质量更慢、文件更大。 |
| `--preview` | 只转换开头 N 秒为独立预览，默认输出加 `.preview`；适合上传前快速检查。 |
| `--report` | 额外写出 JSON 报告；写入失败不会留下半截报告。 |
| `--verbose` | 显示阶段、命令与进度细节。 |
| `--overwrite` | 允许替换已有输出；不改变“绝不覆盖源文件”的规则。 |

完整语义、环境变量与退出状态见 [CLI 参考](docs/cli.md)。

## 引擎怎么选

- `auto`：恒定帧率输入且已找到 RIFE 引擎与模型时优先使用，否则用 CPU；找到文件不表示 GPU/Vulkan 已实测。
- `cpu`：只用 FFmpeg 的 CPU 运动补偿插帧。无需 GPU，速度较慢，兼容性最好。
- `rife`：强制 RIFE；缺少 Vulkan、驱动、引擎或模型时直接失败，便于确认到底用了什么。

CPU 与 RIFE 都是运动补偿插帧，不是简单复制帧，也不是用 `-r` 改标签。快速运动、遮挡、字幕、场景切换处仍可能出现伪影；请用 `--preview` 检查。源帧率已不低于所选目标帧率时，程序不会为了“凑”目标帧率而做无意义插帧。

## 平台现实与限制

截至 2026-11-18 查阅的公开资料：

- 抖音创作者中心当前说明可上传任意分辨率与帧率，但播放最高为 **4K60**；其表内 1080p 及以下为最高 30fps。高清转码一般在两小时内完成，首次低清流不代表最终结果。本轮尚未取得抖音 4K60 分发，不承诺 120fps 播放；页面文字与实测见验证记录第 8.6 节。
- B 站本轮实际补帧成品已在“4K 超高清”（quality 120）档分发为 **4K120**；“1080P 60帧”档仍为 60fps。测量账号具备大会员播放权益，该结果不能外推到所有账号、视频、客户端或物理屏幕。
- 60fps 与 120fps 都需按阶段实测；平台可能转码/降帧，60Hz 屏幕无法展示 120fps。

平台入口、证据模板与判定标准见 [平台上传与人工播放验证](docs/platform-verification.md)。

## 常见问题

- **找不到 FFmpeg**：先运行 `smoothup doctor`；再运行 `smoothup setup`，或设置 `SMOOTHUP_FFMPEG` / `SMOOTHUP_FFPROBE`。不要把无关的 `ffmpeg.exe` 改名为系统目录里的文件。
- **RIFE 不可用 / Vulkan 报错**：先用 `--engine cpu` 完成导出；更新显卡驱动，或运行 `smoothup setup --rife`。`--engine rife` 失败是预期行为，不是静默降级。
- **源文件会被覆盖吗**：不会。即使 `--output` 指向源文件或 `--overwrite` 打开，也会拒绝。输出先写临时文件，验证通过后才发布。
- **ffmpeg 显示成功但文件坏了/变短**：SmoothUp 在发布前会 `verify`；失败时不会把半成品当作成功输出。加 `--verbose` 保留诊断。
- **发布后播放不够流畅**：先确认本地 `verify` 与 `ffprobe` 的帧率正确，再按平台验证流程检查平台是否降帧；不要用平台播放结果反推本地导出一定有问题。
- **HDR / 颜色不对**：输出固定为 SDR，HDR 源会做色调映射，颜色与原始 HDR 不会逐像素一致。
- **杀毒软件拦截 / 未签名**：发布包未签名；用 SHA256 核对来源。企业环境可能需要白名单。

更多问题见 [故障排查](docs/troubleshooting.md)。

## 构建与打包

需要 Rust 1.85+ 与 Windows 10/11 x64。FFmpeg 不是编译依赖。

```powershell
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
```

打包产物在 `dist/`：包含 `smoothup.exe`、README/CHANGELOG/LICENSE/第三方通知、`docs/` 与 `setup.ps1` 的 ZIP，以及大小/SHA256 清单。脚本强制检查主程序不超过 **10 MiB** 工程目标，并拒绝把 FFmpeg/RIFE 二进制打进包。CI 工作流配置为在 Windows runner 上执行构建、测试、clippy 与打包；只在 push 时运行，不使用 PR 流程。详见 [构建与打包](docs/build.md)。

## 仓库文档

- [CLI 参考](docs/cli.md)
- [平台上传与人工播放验证](docs/platform-verification.md)
- [故障排查](docs/troubleshooting.md)
- [构建与打包](docs/build.md)
- [第三方组件与许可证边界](THIRD_PARTY_NOTICES.md)

## 许可证

本项目自身代码采用 MIT License，见 [LICENSE](LICENSE)。FFmpeg、RIFE、ncnn 等外部程序不随发布包分发，其许可证与再分发边界见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。抖音、哔哩哔哩为各自权利人的商标，本项目与平台无隶属关系。
