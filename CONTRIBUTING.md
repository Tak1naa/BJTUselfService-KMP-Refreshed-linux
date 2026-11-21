# 贡献指南 (Contributing)

感谢你对 **交大自由行（BJTU Self Service）** 的关注与支持！  
本仓库主分支维护 **`linux/`** 下基于 Rust 构建的跨平台终端 & TUI 客户端「交大自由行 CLI」（`campus`），支持 Linux / macOS / Windows（原 Kotlin Multiplatform 图形端代码已归档至 Git 标签 `archive/kmp-upstream`）。

---

## 一、快速环境准备（Rust CLI）

1. 安装最新稳定版 Rust 工具链（包含 `rustfmt` 与 `clippy`）：
   ```sh
   rustup component add rustfmt clippy
   ```
2. （可选）Linux 静态构建额外需要：
   ```sh
   rustup target add x86_64-unknown-linux-musl
   ```
   Ubuntu / Debian 安装 `musl-tools`、`rpm`、`python3`（用于静态打包）。

---

## 二、构建与运行测试

所有 CLI 命令均在 `linux/` 目录下执行：

```sh
cd linux

# 1. 代码格式化与严格静态检查（CI 必跑）
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings

# 2. 工作区全部单元测试与集成测试（均基于合成数据与 wiremock，不依赖真实账号）
cargo test --locked --workspace

# 3. 无媒体功能精简编译检查 & SQLCipher 加密数据库测试
cargo check --locked -p campus-cli --no-default-features
cargo test --locked -p campus-db --features sqlcipher
```

### 静态发布构建与体积门禁

`campus` 二进制严格控制依赖与产物体积（Linux 静态 musl 单文件要求 **`<= 10 MiB`**）：

```sh
cargo build --locked --release --target x86_64-unknown-linux-musl -p campus-cli
python3 scripts/benchmark.py target/x86_64-unknown-linux-musl/release/campus
```

构建系统安装包：
- **Linux**（`.deb` / `.rpm` / `.tar.gz`）：`python3 scripts/package.py target/x86_64-unknown-linux-musl/release/campus --format all`
- **Windows x64**（`.zip`）：`python scripts/package-windows.py target/release/campus.exe`
- **macOS**（`.tar.gz`）：`python3 scripts/package-macos.py ...`

更多五层 Crate 架构说明与开发基准规范，请参阅 [linux/docs/DEVELOPMENT.md](linux/docs/DEVELOPMENT.md)。

---

## 三、报告问题（Bug Report / 功能建议）

提交 Issue 前请先：
1. 运行 `campus --version` 与 `campus sync status`（或 `campus cache info`）确认当前版本与同步状态。
2. **严禁上传任何隐私或敏感信息**：提交截图、日志或报错前，务必移除学号、姓名、密码、`Cookie`、`sessionId`、票据链接与真实成绩截图。
3. 清晰描述：
   - 操作系统与终端环境（如 Windows 11 PowerShell、Ubuntu 24.04、macOS Sequoia）
   - 复现步骤与预期/实际输出
   - 是否处于校园网或学校 VPN 环境

---

## 四、提交 Pull Request 规范

1. **安全与隐私边界**：
   - 绝不记录、打印或上传用户账号、密码、`Cookie`、`sessionId` 等凭据。
   - 不得引入静默 TLS 降级或全局禁用证书校验；任何学校域名兼容改动须先分析原因并在最小白名单内隔离。
   - 离线命令与 TUI 启动应避免不必要地阻塞或拉起无关重量级依赖。
2. **依赖与体积**：新增任何第三方依赖前，请先评估静态二进制体积影响，并通过 `scripts/benchmark.py` 确认未超出 `10 MiB` 门禁。
3. **测试与提交**：
   - 新增解析或行为变更须附带可离线复现的单元测试或集成测试（位于 `crates/*/tests/` 或模块内 `#[cfg(test)]`）。
   - 确保 GitHub Actions 工作流 `.github/workflows/linux-rust.yml` 中的所有检查全部通过。
