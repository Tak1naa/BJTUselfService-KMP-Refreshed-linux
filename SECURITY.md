# 安全策略 (Security Policy)

感谢关注 **`BJTUselfService-Lite`** 的安全与隐私保护。本文件说明本项目受安全维护的版本范围、漏洞私密反馈渠道以及安全边界约定。

---

## 支持维护的版本 (Supported Versions)

目前仅对 **`1.0.x`** 稳定主线分支提供安全补丁与依赖漏洞修复（对应 `main` 分支及最新 `cli-v1.0.x` Release）：

| 版本系列 | 安全支持状态 | 说明 |
| :--- | :--- | :--- |
| **`1.0.x`**（当前最新：`1.0.1`） | **支持** | 持续包含 `cargo audit` 安全审计修复、协议白名单更新与漏洞补丁 |
| `0.1.x`（早期预览版） | 不再支持 | 请通过 `campus update --yes` 或前往 Releases 升级至最新 `1.0.x` |
| `archive/kmp-upstream` | 已归档 | 上游 Kotlin Multiplatform 历史只读归档快照，不再维护 |

---

## 如何报告安全漏洞 (Reporting a Vulnerability)

**请勿在公开 Issue 或讨论区直接披露尚未修复的安全漏洞细节**（尤其是涉及凭据提取、本地权限越权、路径穿越、证书校验绕过或命令注入的利用方式）。

1. **推荐渠道（GitHub 私有漏洞报告）**：
   本项目已启用 GitHub **Private Vulnerability Reporting**，请直接前往以下地址提交加密私密报告：
   - **[提交私有安全报告 (GitHub Security Advisory)](https://github.com/Tak1naa/BJTUselfService-Lite/security/advisories/new)**
2. **建议在报告中附带的信息**：
   - 受影响的版本号（`campus --version`）与操作系统平台（Linux / macOS / Windows）；
   - 触发场景、影响范围及最小化脱敏复现步骤（请务必去除真实学号、密码、Cookie 与校内敏感正文）；
   - 如有建议的修复思路或补丁方向，也欢迎一并说明。
3. **响应说明**：
   - 收到私有报告后会在确认可复现后尽快评估优先级并准备修复补丁，修复发布前不对外公开具体利用细节；
   - 漏洞修复并发布新版本后，将视需要发布 Security Advisory 并向报告者致谢。

---

## 安全审计范围与已知边界 (Scope & Known Limitations)

为避免安全研究员或自动化扫描误解，以下列出本项目的保护边界：

### 重点关注的安全范围 (In Scope)
- **本地凭据与数据库保护**：系统钥匙串集成（`keyring`）、`vault.backend = file` / `passphrase` 私有权限（`0600`）与 `SQLCipher` 加密数据库中的密钥/Cookie 越权泄露风险。
- **文件系统与路径隔离**：课件、作业与邮箱附件下载名清洗（防止路径穿越覆盖系统或用户关键文件）与压缩包安全解压。
- **更新与传输安全**：`campus update` 的 GitHub 官方域名/逐跳重定向白名单限制、`SHA256SUMS.txt` 强制核验，以及主 HTTPS 通道的 `rustls` 证书校验（仅对学校部分缺少中间证书的域名补充内置官方 `GeoTrust G2 TLS CN RSA4096 SHA256 2022 CA1` 中间证书，不放宽信任锚）。
- **注入与敏感日志**：终端转义序列注入过滤、`cscript`/系统命令调用环境变量隔离、以及日志/报错输出中的凭据零泄露。

### 已知受限边界与扫描误报 (Out of Scope / Expected Behavior)
1. **学校旧版系统的上游 HTTP 明文**：
   校方旧版智慧教学平台（`http://123.121.147.7:88`，用于作业列表/附件/提交与课件）与第三方教室人数接口（`http://yaya.csoci.com:2333/`）本身仅提供 HTTP 接口，客户端无法单方面启用 TLS。本项目通过独立临时传输、`no_proxy()`、严格 URL/参数白名单与**明文会话 Cookie 绝不落盘**将影响控制在最小面，并支持通过 `campus config set homework.legacy_http off` / `courseware.legacy_http off` 完全禁用；该上游明文特性本身不属于客户端代码漏洞。
2. **单元测试中的合成测试常量**：
   `linux/crates/campus-net/src/tests.rs` 与 `linux/crates/campus-net/src/phyvlab.rs` 内仅在 `#[cfg(test)]` 作用域出现的固定字符串（如 `"fixture-password"`、`"fixture-Password_123!"`、`"lab-pass"`）均为配合 `wiremock` 与 SM2 密文比对的合成测试夹具，不代表任何真实账号凭据或生产硬编码密钥。

---

## 构建与供应链安全保障

- 每次 CI 与发版均通过 `cargo audit` 检查 `linux/Cargo.lock` 与 `linux/fuzz/Cargo.lock`（历史示例：在 `1.0.1` 中第一时间修复了上游 `RUSTSEC-2026-0592` / `url 2.5.9`）。
- 禁止引入 `openssl`、`libdbus-sys`、`dbus` 等系统动态链接安全面；Linux 发布产物为全静态 musl 单二进制。
- 全部 GitHub Release 产物均提供 `SHA256SUMS.txt` 与 GitHub SLSA Build Provenance 构建溯源证明，安装与更新前均可核验完整性。
