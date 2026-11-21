# 交大自由行 CLI (`campus`) 架构、编译与验证指南

本文档面向开发者与贡献者，说明 Rust 终端客户端的五层 Crate 架构重构、小体积与内存优化措施、跨平台构建打包方法及自动化对照验证流程。  
普通终端用户的安装与使用教程请见 [../README.md](../README.md)。

---

## 1. 五层 Crate 架构重构

为彻底脱离 JVM 运行时与跨平台 UI 框架开销，原 KMP 业务逻辑按职责边界重构为 5 个严格单向依赖的 Rust Crate：

```
                     ┌──────────────────────────────────┐
                     │            campus-cli            │
                     │  clap CLI · ratatui TUI · 输出   │
                     │  systemd / launchd · 补全生成    │
                     └───────────┬────────────┬─────────┘
                                 │            │
                     ┌───────────▼──────┐     │
                     │   campus-sync    │     │
                     │ 编排 · 重登去重  │     │
                     └───┬──────────┬───┘     │
                         │          │         │
           ┌─────────────▼──┐   ┌───▼─────────▼──┐
           │   campus-net   │   │   campus-db    │
           │ rustls · SM2   │   │ SQLite · Vault │
           │ Range/HLS · Lab│   │ Keyring · 路径 │
           └───────┬────────┘   └───────┬────────┘
                   │                    │
                   └──────────┬─────────┘
                              ▼
                     ┌──────────────────┐
                     │   campus-core    │
                     │ 纯领域规则与解析 │
                     │ 零 IO / 零异步   │
                     └──────────────────┘
```

| Crate | 职责与边界 |
| --- | --- |
| [`campus-core`](../crates/campus-core) | 纯函数领域层（零网络、零文件系统、零异步运行时）：课表/成绩/考试/校历/物理在线/教室 HTML 与 JSON 解析、学分加权 GPA 计算、教学周映射、ICS 日历生成与页面脱敏 |
| [`campus-db`](../crates/campus-db) | 存储与凭据层：`rusqlite` (bundled) 事务缓存、结构化表 + JSON 快照同事务写入、XDG/Windows `0700`/`0600`/`O_NOFOLLOW`/`flock` 文件锁、`XChaCha20-Poly1305` 凭据加密、系统钥匙串（Linux Secret Service / macOS `apple-native` Keychain / Windows Credential Manager）与可选 `sqlcipher` 全库加密 |
| [`campus-net`](../crates/campus-net) | 网络协议层：`reqwest` + `rustls-tls` (`webpki-roots`，无 OpenSSL)、本地 SM2 密码加密、单次登录请求上限与教务接口串行节流、物理在线本机 `127.0.0.1` 回环验证码捕获、4 MiB Range 断点续传与 HLS VOD 下载 |
| [`campus-sync`](../crates/campus-sync) | 同步协调层：多模块调度、并发过期单次重登去重、主账号与物理在线凭据/Cookie 隔离及变更追踪 |
| [`campus-cli`](../crates/campus-cli) | 终端交互层：`clap` 子命令、VS Code Dark Modern 风格表格 / TSV / `schema: 1` JSON 输出、`ratatui` + `crossterm` 全屏 10 页 TUI、Linux `systemd` 与 macOS `launchd` 定时同步代理 |

---

## 2. 小体积与低内存工程实践

- **静态零依赖分发（< 10 MiB 硬门禁）**：
  - `Cargo.toml` 释放配置采用 `opt-level = 3`（主代码）、`profile.release.package."*".opt-level = "s"`（控制第三方 crate 与 bundled SQLite 体积）、`lto = "fat"`、`codegen-units = 1`、`panic = "abort"`、`strip = true`。
  - `.cargo/config.toml` 在 Linux `x86_64-unknown-linux-musl` 目标使用 Rust 自带 `rust-lld` 静态链接，`0.1.9` 静态 musl 可执行文件为 `10,225,056` 字节（`9.75 MiB`）；`.deb` 约 `3.27 MiB`、`.rpm` 约 `3.51 MiB`。
  - 若需要更小体积，可通过 `--no-default-features` 裁剪掉系统钥匙串与媒体下载模块。
- **启动与运行时内存优化**：
  - **按需延迟初始化**：所有 `--offline` 查询与 `campus tui` 启动读缓存时，完全跳过 `tokio` 多线程运行时、`rustls`/HTTP 客户端与系统钥匙串调用，直接打开本地 SQLite 读取返回；100 次预热基准显示 `--version` p95 `2.69 ms`，离线查询 p95 `8.70–15.05 ms`。
  - **流式传输不驻留整包**：课件下载按 `4 MiB` Range 分片校验 `ETag` 落盘；教学录像解析主列表与 VOD TS 分片后 4 路并发拉取并按顺序流式拼接，避免大文件缓冲占用内存。
  - **敏感内存主动擦除**：口令、私钥材料与解密缓冲统一使用 `zeroize::Zeroizing` 在作用域结束时清零；`sync --background` 使用独立 `process_group(0)` / `CREATE_NO_WINDOW` 子进程执行并受 `sync.lock` 文件锁互斥保护。
  - **无重型系统库**：通过 `cargo tree -e normal` 门禁确保默认 Linux 构建不依赖 `OpenSSL`、`libdbus-sys` 或 `dbus` C 库。

---

## 3. 从源码构建与跨平台打包

### Linux（动态构建、静态 musl 与 DEB / RPM / 通用 tar.gz）

```sh
cd linux
cargo build --locked --release -p campus-cli

# 静态 musl 版本 + 打包全部 Linux 产物（.deb / .rpm / .tar.gz）
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --locked --release \
  --target x86_64-unknown-linux-musl -p campus-cli
python3 scripts/package.py target/x86_64-unknown-linux-musl/release/campus --format all
```

- Arch Linux 本地构建脚本：[packaging/PKGBUILD](../packaging/PKGBUILD)（可在 `packaging/` 目录运行 `makepkg -si`）。
- Nix 表达式：[packaging/default.nix](../packaging/default.nix)。
- 全库加密变体：`cargo build --locked --release -p campus-cli --features sqlcipher`（普通 SQLite 与 SQLCipher 库文件不兼容，请使用独立 `XDG_DATA_HOME`）。

### macOS（Apple Silicon / Intel / Universal）

```sh
cd linux
# 单架构
cargo build --locked --release -p campus-cli
python3 scripts/package-macos.py target/release/campus --arch arm64

# Universal 通用二进制 (arm64 + x86_64)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin -p campus-cli
cargo build --locked --release --target x86_64-apple-darwin -p campus-cli
lipo -create -output artifacts/campus-macos-universal \
  target/aarch64-apple-darwin/release/campus \
  target/x86_64-apple-darwin/release/campus
python3 scripts/package-macos.py artifacts/campus-macos-universal --arch universal
```

### Windows x64（Linux 交叉编译）

需准备 Rust `x86_64-pc-windows-gnullvm` 标准库与 LLVM-MinGW UCRT 工具链：

```sh
cd linux
rustup target add x86_64-pc-windows-gnullvm
CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang \
AR_x86_64_pc_windows_gnullvm=llvm-ar \
CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang \
  cargo build --locked --release --target x86_64-pc-windows-gnullvm -p campus-cli
python3 scripts/package-windows.py target/x86_64-pc-windows-gnullvm/release/campus.exe
```

---

## 4. 机器输出契约与退出码

`--json` 与 `--format json` 统一输出 `schema: 1` 信封：

```json
{
  "schema": 1,
  "command": "grades",
  "account": "学号",
  "generated_at": 1763560000,
  "freshness": { "source": "cache", "stale": false },
  "data": [],
  "warnings": []
}
```

错误时输出 `{"schema":1,"error":{"code":<exit_code>,"message":"..."}}`：

| 退出码 | 错误类型 | 含义 |
| --- | --- | --- |
| `0` | — | 成功 |
| `1` | `Storage` / `Malformed` / `SecureChannel` 等 | 本地存储、解析或安全通道异常 |
| `2` | `NoData` / `CacheUnavailable` | 无可用离线缓存 |
| `3` | `SessionExpired` / `Rejected` / `ActionRequired` | 需重新登录、认证被拒或需人工交互（如完善个人资料） |
| `4` | 部分同步成功 | 一次同步多个数据集时部分成功、部分失败 |
| `64` | `Usage` / CLI 解析错误 | 参数不合法或缺少显式确认标志（如 `--yes`） |
| `69` | `Network` | 网络请求失败或超时 |
| `75` | `Busy` | 另一个进程正持有 `sync.lock` |

---

## 5. 测试与基准门禁

```sh
cd linux
# 1. 代码风格、静态检查与全量单元/集成测试
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p campus-cli --no-default-features
cargo test --locked -p campus-db --features sqlcipher

# 2. 二进制体积 (<= 10 MiB) 与离线延迟门禁
python3 scripts/benchmark.py target/x86_64-unknown-linux-musl/release/campus
```

- **脱敏采集工具**（仅限开发排查，禁止在 `release` 构建中启用）：  
  `cargo run -p campus-cli --features developer -- debug capture --redact --input page.html -o fixture.html`
- **模糊测试**：[`fuzz/`](../fuzz) 提供登录页、校历、教学周与 HLS 播放列表的 `libFuzzer` 入口。
- 详细实测结果与持续验收状态见 [STATUS.md](STATUS.md)。
