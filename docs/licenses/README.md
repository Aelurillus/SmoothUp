# 第三方许可证文本

本目录保存实际 Cargo registry 包中的许可证/notice 文本，供 `THIRD_PARTY_NOTICES.md` 引用。

## 来源与生成方式

依赖清单来自：

```text
Cargo.lock
cargo metadata --offline --locked --filter-platform x86_64-pc-windows-msvc --format-version 1
```

对 metadata 中属于 Windows 目标的每个 Cargo 包，从其 registry 解压目录按原文件名复制：

- `LICENSE*`
- `COPYING*`
- `NOTICE*`
- `UNLICENSE*`

目标路径为 `docs/licenses/third-party/<crate>-<version>/`。未对文本做改写；MIT/Apache/Unicode/Unlicense 等许可证表达式以各包 `Cargo.toml` 为准，无法用单一许可证概括。

`docs/licenses/Apache-2.0.txt` 也是从实际 registry 包（`clap-4.6.8/LICENSE-APACHE`）复制的 Apache-2.0 基准文本。

## 版权/notice

- 随附的 `LICENSE-MIT`、`LICENSE` 等文件包含上游版权行时，保留原样。
- registry 包中没有独立版权行的 MIT 文本，其 `Cargo.toml` 中的 `authors`/`repository` 元数据记录在 `THIRD_PARTY_NOTICES.md` 的 3.1 节；没有自行补写或推测版权人。
- `unicode-ident` 同时受 `(MIT OR Apache-2.0) AND Unicode-3.0` 约束，随附 `LICENSE-MIT`、`LICENSE-APACHE`、`LICENSE-UNICODE` 三份文本。
- `memchr` 为 `Unlicense OR MIT`，随附 `COPYING`、`LICENSE-MIT`、`UNLICENSE`。

## 维护

依赖升级/新增后，重新生成 `Cargo.lock`，按上述命令重新解析 Windows 依赖，并从对应 registry 包复制新的许可证文件；不要保留已移除依赖的旧目录，也不要用统一许可证标识替代实际表达式。重新打包后由 `scripts/package.ps1` 将本目录整体收入 ZIP。
