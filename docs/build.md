# 构建、测试与打包

本文面向从源码构建 Windows 发布包。普通用户只需要下载 ZIP 并运行 `setup.ps1`。

## 1. 环境

- Windows 10/11 x64。
- Rust 1.85 或更新（edition 2021）。CI 使用当前 stable。
- Git 与 PowerShell 5.1+（CI 使用 PowerShell 7）。
- Rust 工具链二选一：
  - MSVC：Visual Studio Build Tools 的“使用 C++ 的桌面开发”；
  - GNU：`x86_64-pc-windows-gnu` 工具链。
- 编译**不需要** FFmpeg 或 RIFE；它们是运行时依赖，由 `setup` 安装。

检查：

```powershell
rustc --version
cargo --version
```

## 2. 常用命令

```powershell
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

- `cargo build --release` 生成 `target\release\smoothup.exe`。
- 提交了 `Cargo.lock` 时，CI 与打包脚本使用 `--locked`，避免依赖漂移。
- 格式化不是提交门槛的一部分；如本地使用 `cargo fmt`，请单独执行，不要把它混入功能提交。

发布配置在 `Cargo.toml`：`opt-level = "s"`、LTO、单 codegen unit、strip、`panic = "abort"`。目标是保持单文件、轻量、启动快；不要为了体积引入不必要的依赖或运行时服务。

## 3. 打包

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
```

可选参数：

```powershell
# 已经构建过，跳过 cargo build
.\scripts\package.ps1 -SkipBuild

# 指定版本号（默认读取 Cargo.toml）
.\scripts\package.ps1 -Version 0.1.0

# 指定输出目录（默认 dist）
.\scripts\package.ps1 -OutputDir .\dist
```

脚本会：

1. 读取 `Cargo.toml` 版本，构建 release 可执行文件（除非 `-SkipBuild`）。
2. 检查 `smoothup.exe` 不超过 **10 MiB**（10 × 1024 × 1024 字节）工程目标；超过即失败。
3. 在 `dist` 下的唯一临时目录中组装 `SmoothUp-<版本>-windows-x64/`（完成后只清理本次临时目录，不删除 `dist` 中其他内容）：
   - `smoothup.exe`
   - `README.md`、`CHANGELOG.md`、`LICENSE`、`THIRD_PARTY_NOTICES.md`
   - `docs/`（含 CLI、平台验证、故障排查、构建文档与逐包第三方许可证文本）
   - `setup.ps1`
4. 拒绝把 `ffmpeg.exe`、`ffprobe.exe`、`ffplay.exe`、`rife-ncnn-vulkan.exe` 等外部二进制打进包。
5. 生成 `SHA256SUMS.txt` 与 `MANIFEST.txt`（记录每个文件的字节数与 SHA256、exe 大小与上限）。
6. 生成 `dist/SmoothUp-<版本>-windows-x64.zip`，并写出对应 `.sha256` 文件。

发布包未签名。对外分发时同时提供 ZIP 与 `.sha256`，让用户先校验再解压。打包脚本拒绝任何 `.dll`；如果 GNU 工具链构建出的 exe 依赖额外运行时 DLL，请改用 MSVC（CI 使用静态 CRT）或调整链接方式，不要把 DLL 塞进发布包。

当前 Windows 本机可能是 GNU target。打包后应检查 PE 导入（例如 `objdump -p target\release\smoothup.exe | Select-String "DLL Name"`）；若出现 `libgcc_s_seh-1.dll`、`libwinpthread-1.dll` 等 GCC 运行库依赖，应通过静态链接解决，而不是把运行库放进发布包。MSVC CI 已配置静态 CRT。

### 为什么强制 10 MiB

这是主程序的工程约束，不是平台要求。它迫使实现保持单文件、无重型框架、无捆绑模型。`setup` 安装的 FFmpeg/RIFE 不计入该上限，也不进入 ZIP。若构建超过上限，应先排查依赖与构建配置，不要通过删除文档或跳过检查来“通过”。

## 4. CI

`.github/workflows/ci.yml` 只在 **push 到 main 或推送 `v*` 标签** 时运行，不配置 `pull_request` 事件，符合本项目“不使用 PR 流程”的约定。

任务：

- `check`（Windows runner）：
  - `cargo build --locked --all-targets`
  - `cargo test --locked`
  - `cargo clippy --locked --all-targets -- -D warnings`
  - 使用静态 MSVC CRT（`-C target-feature=+crt-static`），减少对额外 VC++ 运行库的依赖。
- `package`：依赖 `check`，运行 `scripts/package.ps1`，上传 ZIP 与 `.sha256` 为构建产物。
- `release`：仅 `v*` 标签触发，下载打包产物并用 `gh release create --verify-tag` 创建 GitHub Release；普通 main push 不会发布 Release。

CI 不执行真实平台上传、不生成平台播放证据，也不应被描述为“平台验证通过”。

## 5. 本地集成检查

以下脚本只存在于源码仓库的 `scripts/`，**不进入发布 ZIP**；发布包用户不需要它们。在具备 FFmpeg 的 Windows 机器上，可执行：

```powershell
# 生成 30/60/120fps 带帧号、切点、音画提示的样片
powershell -ExecutionPolicy Bypass -File .\scripts\generate-samples.ps1

# 用真实 CLI 与 ffprobe 检查转换、时长、帧率、音频、源文件不被覆盖
powershell -ExecutionPolicy Bypass -File .\scripts\media-smoke.ps1
```

这两个脚本会真实调用 `smoothup.exe` 与 `ffprobe`。它们不是单元测试，不替代平台人工验证；运行结果应保存日志，不要在没有执行的情况下宣称通过。CI 不运行它们，以免在缺少真实媒体工具链的环境中伪造结论。

两个脚本都不会递归清空用户指定目录：样例脚本把帧文件写入带随机后缀的临时子目录，媒体冒烟脚本在 `-WorkDir` 下创建唯一的 `run-*` 子目录并保留证据。需要清理时请自行确认路径后删除这些专属子目录。

本次本机实际结果（2026-11-18，Windows GNU 构建）：

- `cargo test --locked`：六项安全回归通过；另有三个由测试启动的子进程 fixture 标记为 ignored，不是待补测试。
- `cargo clippy --locked --all-targets -- -D warnings`：通过。
- `scripts/media-smoke.ps1`：12 项通过，0 失败；证据位于 `.work/smoke/run-20261118-045448-75042de5/`，包含源文件哈希、转换与 verify 报告。
- 媒体矩阵：24→120fps 的 120 帧/1 秒与音轨偏移、VFR/SAR→60000/1001、原生 120fps 流复制、PQ HDR→SDR、旋转归一化、RIFE 跨分块有理帧率均已通过；RIFE 额外覆盖含中文、空格和单引号的输出目录。
- 平台成品上传已验证到 B 站文件接收与草稿；公开投稿、平台转码分发、真实设备高帧播放未验证，详见平台记录。

这些结果不代表已在干净 Windows 用户配置、所有 GPU 或平台播放端通过测试；CI 结果单独查看。

## 6. 发布检查清单

- [ ] `Cargo.toml` 版本与 `CHANGELOG.md` 一致。
- [ ] `Cargo.lock` 已提交，CI 使用 `--locked` 成功。
- [ ] `cargo test`、`cargo clippy --all-targets -- -D warnings` 通过。
- [ ] `scripts/package.ps1` 成功，`smoothup.exe` ≤10 MiB。
- [ ] ZIP 内没有 FFmpeg/RIFE 二进制；`SHA256SUMS.txt` 与 `MANIFEST.txt` 已生成。
- [ ] 在干净 Windows 用户环境解压，运行 `setup.ps1` 与 `doctor`。
- [ ] 至少完成一次本地真实转换与 `verify`；如涉及平台结论，按 [平台上传与人工播放验证](platform-verification.md) 留证。
- [ ] 记录实际执行过的检查；未执行的项目明确写“未运行”。

## 7. 版本与许可证

- 自身代码：MIT，见 `LICENSE`。
- 编译进 Windows 二进制的 Rust 依赖按各包实际许可证使用（MIT、Apache-2.0、Unicode-3.0、Unlicense 等），不能概括为全部 Apache。逐包原始许可文本见 `docs/licenses/third-party/<crate>-<version>/`；清单与版权/来源元数据见 `THIRD_PARTY_NOTICES.md`。
- FFmpeg（GyanD essentials）为 GPLv3，RIFE/ncnn 为 MIT/BSD-3；它们不随发布包分发，详见 `THIRD_PARTY_NOTICES.md`。
- 新增依赖时同步更新 `THIRD_PARTY_NOTICES.md` 与打包清单。
