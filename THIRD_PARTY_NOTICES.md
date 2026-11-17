# 第三方组件与许可证边界

本文档说明 SmoothUp 自身代码、编译进 Windows 二进制的 Rust 依赖，以及由 `setup` 从上游下载的外部程序的许可证边界。它不是法律意见；对外分发二进制或安装包前，请按各上游许可证核对义务。

## 1. SmoothUp 自身代码

SmoothUp 仓库内由本项目编写的代码与文档采用 **MIT License**，全文见 [`LICENSE`](LICENSE)。版权声明使用 “SmoothUp contributors”，不虚构个人姓名。

## 2. 外部程序：不随发布包分发

发布包（`dist/SmoothUp-*-windows-x64.zip`）**只包含** `smoothup.exe`、文档、`setup.ps1` 与校验清单，不包含 `ffmpeg.exe`、`ffprobe.exe`、`ffplay.exe`、`rife-ncnn-vulkan.exe` 或模型文件。`smoothup setup` 在用户显式执行时从上游固定版本下载并校验；SmoothUp 不以 MIT 许可证对这些外部程序再授权。

### 2.1 FFmpeg（Windows，GyanD/codexffmpeg）

- 上游：<https://github.com/GyanD/codexffmpeg>
- 固定版本：9.0.2，`ffmpeg-9.0.2-essentials_build.zip`
- 下载地址：<https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip>
- 下载大小：114,768,076 字节
- SHA256：`60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba`
- 上游许可证说明：<https://www.gyan.dev/ffmpeg/builds/> 明确说明其 Windows 构建为 **GPLv3**；FFmpeg 项目本身的许可证说明见 <https://ffmpeg.org/legal.html>。
- 对应源码参考：<https://github.com/FFmpeg/FFmpeg/commit/946fcce07b>（Gyan 页面为 9.0.2 标注的源码提交）。
- 该 essentials 构建包含 libx264 等 GPL 组件，因此安装后的 `ffmpeg.exe`/`ffprobe.exe` 按 GPLv3 分发，而不是 LGPL。若你再次分发这些二进制，需要自行满足 GPLv3 的源代码与通知义务；SmoothUp 的 MIT 许可证不覆盖它们。
- SmoothUp 仅以独立进程方式调用这些程序，不链接 FFmpeg 库。FFmpeg 是 Fabrice Bellard 的商标；本项目与 FFmpeg 项目无隶属关系。

### 2.2 RIFE ncnn Vulkan（可选 GPU 引擎）

- 上游：<https://github.com/nihui/rife-ncnn-vulkan>（MIT License）
- 固定发行版：`20221029` Windows ZIP，下载大小 431,540,241 字节
- 下载地址：<https://github.com/nihui/rife-ncnn-vulkan/releases/download/20221029/rife-ncnn-vulkan-20221029-windows.zip>
- SHA256：`d8e4d772d26cd8006ef0ad0bc82eb191b53c68677d1ae2f42506d74cbbbea606`
- 安装内容：`setup --rife` 只从该 ZIP 保留运行所需的引擎、DLL 与 `rife-v4.6` 模型，其余模型不解压安装。已实测保留文件约为：
  - `rife-ncnn-vulkan.exe`：6,974,464 字节
  - 随附 DLL：181,680 字节
  - `rife-v4.6` 的 `flownet.bin`：10,614,320 字节
  - `rife-v4.6` 的 `flownet.param`：16,749 字节
  - `LICENSE`：1,093 字节
  - 合计约 17 MiB（**这是安装后保留的体积，不是 ZIP 下载体积**）
- 上述 ZIP 中的 `vcomp140.dll` 是 Microsoft Visual C++ 运行库组件，适用 Microsoft 自身的许可，SmoothUp 不对其再授权；`setup` 仅原样保留该文件以运行 RIFE 引擎。
- 推理框架 ncnn：<https://github.com/Tencent/ncnn>，BSD-3-Clause。
- RIFE 论文与上游实现：<https://github.com/hzwer/ECCV2022-RIFE>，MIT License。
- 该 ZIP 不随 SmoothUp 发布包分发；`setup --rife` 下载后按上述上游许可证使用。若再次分发该 ZIP 或其内容，请保留上游 `LICENSE` 与版权声明。

## 3. 编译进 Windows 二进制的 Rust 依赖

以下清单按 `Cargo.lock` 与 `cargo metadata --offline --locked --filter-platform x86_64-pc-windows-msvc` 的实际解析结果生成，覆盖 Windows 目标使用的依赖（不含 `Cargo.lock` 中仅为其他平台解析的条件依赖）。Cargo 包名与版本如下；实际许可证表达式并不全是 MIT/Apache：其中包含 MIT-only、Unlicense OR MIT、Apache-2.0 OR MIT，以及 `unicode-ident` 的 `(MIT OR Apache-2.0) AND Unicode-3.0`。因此本项目**不将全部 Rust 依赖统一声明为 Apache-2.0**。

每个包在 Cargo registry 中的许可证/notice 文件已按原文件名逐字复制到 `docs/licenses/third-party/<crate>-<version>/`；`docs/licenses/Apache-2.0.txt` 为从实际 registry 包复制的 Apache-2.0 基准文本。表中链接即为随发布包附带的原始文本所在目录。

| Crate | 版本 | Cargo.toml 许可证表达式 | 随附原始许可文本 |
| --- | --- | --- | --- |
| `anstream` | 1.0.0 | MIT OR Apache-2.0 | [docs/licenses/third-party/anstream-1.0.0/](docs/licenses/third-party/anstream-1.0.0/) |
| `anstyle` | 1.0.14 | MIT OR Apache-2.0 | [docs/licenses/third-party/anstyle-1.0.14/](docs/licenses/third-party/anstyle-1.0.14/) |
| `anstyle-parse` | 1.0.0 | MIT OR Apache-2.0 | [docs/licenses/third-party/anstyle-parse-1.0.0/](docs/licenses/third-party/anstyle-parse-1.0.0/) |
| `anstyle-query` | 1.1.5 | MIT OR Apache-2.0 | [docs/licenses/third-party/anstyle-query-1.1.5/](docs/licenses/third-party/anstyle-query-1.1.5/) |
| `anstyle-wincon` | 3.0.11 | MIT OR Apache-2.0 | [docs/licenses/third-party/anstyle-wincon-3.0.11/](docs/licenses/third-party/anstyle-wincon-3.0.11/) |
| `clap` | 4.6.8 | MIT OR Apache-2.0 | [docs/licenses/third-party/clap-4.6.8/](docs/licenses/third-party/clap-4.6.8/) |
| `clap_builder` | 4.6.7 | MIT OR Apache-2.0 | [docs/licenses/third-party/clap_builder-4.6.7/](docs/licenses/third-party/clap_builder-4.6.7/) |
| `clap_derive` | 4.6.8 | MIT OR Apache-2.0 | [docs/licenses/third-party/clap_derive-4.6.8/](docs/licenses/third-party/clap_derive-4.6.8/) |
| `clap_lex` | 1.1.2 | MIT OR Apache-2.0 | [docs/licenses/third-party/clap_lex-1.1.2/](docs/licenses/third-party/clap_lex-1.1.2/) |
| `colorchoice` | 1.0.5 | MIT OR Apache-2.0 | [docs/licenses/third-party/colorchoice-1.0.5/](docs/licenses/third-party/colorchoice-1.0.5/) |
| `ctrlc` | 3.5.2 | MIT/Apache-2.0 | [docs/licenses/third-party/ctrlc-3.5.2/](docs/licenses/third-party/ctrlc-3.5.2/) |
| `heck` | 0.5.0 | MIT OR Apache-2.0 | [docs/licenses/third-party/heck-0.5.0/](docs/licenses/third-party/heck-0.5.0/) |
| `is_terminal_polyfill` | 1.70.2 | MIT OR Apache-2.0 | [docs/licenses/third-party/is_terminal_polyfill-1.70.2/](docs/licenses/third-party/is_terminal_polyfill-1.70.2/) |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 | [docs/licenses/third-party/itoa-1.0.19/](docs/licenses/third-party/itoa-1.0.19/) |
| `memchr` | 2.8.3 | Unlicense OR MIT | [docs/licenses/third-party/memchr-2.8.3/](docs/licenses/third-party/memchr-2.8.3/) |
| `once_cell_polyfill` | 1.70.2 | MIT OR Apache-2.0 | [docs/licenses/third-party/once_cell_polyfill-1.70.2/](docs/licenses/third-party/once_cell_polyfill-1.70.2/) |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | [docs/licenses/third-party/proc-macro2-1.0.107/](docs/licenses/third-party/proc-macro2-1.0.107/) |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | [docs/licenses/third-party/quote-1.0.47/](docs/licenses/third-party/quote-1.0.47/) |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | [docs/licenses/third-party/serde-1.0.229/](docs/licenses/third-party/serde-1.0.229/) |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | [docs/licenses/third-party/serde_core-1.0.229/](docs/licenses/third-party/serde_core-1.0.229/) |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | [docs/licenses/third-party/serde_derive-1.0.229/](docs/licenses/third-party/serde_derive-1.0.229/) |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | [docs/licenses/third-party/serde_json-1.0.154/](docs/licenses/third-party/serde_json-1.0.154/) |
| `strsim` | 0.11.1 | MIT | [docs/licenses/third-party/strsim-0.11.1/](docs/licenses/third-party/strsim-0.11.1/) |
| `syn` | 3.0.10 | MIT OR Apache-2.0 | [docs/licenses/third-party/syn-3.0.10/](docs/licenses/third-party/syn-3.0.10/) |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | [docs/licenses/third-party/unicode-ident-1.0.27/](docs/licenses/third-party/unicode-ident-1.0.27/) |
| `utf8parse` | 0.2.2 | Apache-2.0 OR MIT | [docs/licenses/third-party/utf8parse-0.2.2/](docs/licenses/third-party/utf8parse-0.2.2/) |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 | [docs/licenses/third-party/windows-link-0.2.1/](docs/licenses/third-party/windows-link-0.2.1/) |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 | [docs/licenses/third-party/windows-sys-0.61.2/](docs/licenses/third-party/windows-sys-0.61.2/) |
| `zmij` | 1.0.23 | MIT | [docs/licenses/third-party/zmij-1.0.23/](docs/licenses/third-party/zmij-1.0.23/) |

### 3.1 上游版权/来源元数据

多数随附 `LICENSE-MIT` 已含独立版权行。以下 registry 包的 MIT 文本本身未写独立版权行；此处记录 registry `Cargo.toml` 的实际 `authors`/`repository` 元数据，未自行补写或推测版权人。其他包的版权信息以其随附许可文本为准。

| Crate | 上游作者（registry Cargo.toml） | 上游仓库 |
| --- | --- | --- |
| `ctrlc` | `Antti Keränen <detegr@gmail.com>` | <https://github.com/Detegr/rust-ctrlc.git> |
| `itoa` | `David Tolnay <dtolnay@gmail.com>` | <https://github.com/dtolnay/itoa> |
| `proc-macro2` | `David Tolnay <dtolnay@gmail.com>`; `Alex Crichton <alex@alexcrichton.com>` | <https://github.com/dtolnay/proc-macro2> |
| `quote` | `David Tolnay <dtolnay@gmail.com>` | <https://github.com/dtolnay/quote> |
| `serde` | `Erick Tryzelaar <erick.tryzelaar@gmail.com>`; `David Tolnay <dtolnay@gmail.com>` | <https://github.com/serde-rs/serde> |
| `serde_core` | `Erick Tryzelaar <erick.tryzelaar@gmail.com>`; `David Tolnay <dtolnay@gmail.com>` | <https://github.com/serde-rs/serde> |
| `serde_derive` | `Erick Tryzelaar <erick.tryzelaar@gmail.com>`; `David Tolnay <dtolnay@gmail.com>` | <https://github.com/serde-rs/serde> |
| `serde_json` | registry `Cargo.toml` 的 `authors` 为空 | <https://github.com/serde-rs/json> |
| `syn` | `David Tolnay <dtolnay@gmail.com>` | <https://github.com/dtolnay/syn> |
| `unicode-ident` | `David Tolnay <dtolnay@gmail.com>` | <https://github.com/dtolnay/unicode-ident> |
| `zmij` | `David Tolnay <dtolnay@gmail.com>` | <https://github.com/dtolnay/zmij> |

若新增/升级依赖，应重新生成 `Cargo.lock`、更新本节清单，并从对应 registry 包复制新的许可证/notice 文件；不要用统一许可证标识替代实际依赖的许可证表达式。

## 4. 平台与商标

抖音、哔哩哔哩及其标识属于各自权利人。SmoothUp 不隶属于这些平台，也不代表平台官方。`--platform douyin|bilibili` 只影响输出文件名与预设参数，不包含上传、登录或绕过平台审核的功能。平台可能对上传文件转码、降帧或限制清晰度；平台行为不属于本项目的许可证授权范围。
