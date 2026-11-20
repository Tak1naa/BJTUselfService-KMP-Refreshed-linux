# 交大自由行 CLI (`campus`)

北京交通大学校园服务跨平台原生终端客户端（Rust + SQLite，当前版本 `0.1.9`）。  
离线优先、低内存、单二进制分发，无需 JVM、常驻后端或外置数据库；同时提供全屏工作台（TUI）与可脚本化的 CLI / JSON / TSV 输出。

> 原 Kotlin Multiplatform（Android / iOS / macOS / Windows 桌面端）工程仍完整保留于上级 [`multiplatform/`](../multiplatform) 目录，作为客户端与解析对照基准；[`linux/`](.) 目录为其原生终端实现，两者互不影响。实际验收记录与各平台测试状态见 [docs/STATUS.md](docs/STATUS.md)。

---

## 目录

- [特性概览](#特性概览)
- [平台支持与安装路径](#平台支持与安装路径)
- [快速上手](#快速上手)
- [全屏工作台 (TUI)](#全屏工作台-tui)
- [CLI 命令速查](#cli-命令速查)
  - [1. 账号与认证 (`auth`)](#1-账号与认证-auth)
  - [2. 课表、成绩、考试、一卡通与校历](#2-课表成绩考试一卡通与校历)
  - [3. 教室查询 (`rooms`)](#3-教室查询-rooms)
  - [4. 作业与课件下载 (`homework` / `courseware`)](#4-作业与课件下载-homework--courseware)
  - [5. 物理在线 (`phyvlab`)](#5-物理在线-phyvlab)
  - [6. 同步、定时任务与导出 (`sync` / `changes` / `export`)](#6-同步定时任务与导出-sync--changes--export)
- [输出格式、退出码与配置](#输出格式退出码与配置)
- [数据目录、凭据与安全模型](#数据目录凭据与安全模型)
- [从源码构建与打包](#从源码构建与打包)
- [测试、对照验证与性能基准](#测试对照验证与性能基准)

---

## 特性概览

- **离线优先**：所有本地查询（课表、成绩、考试、一卡通、学年历、作业、物理在线等）直接读取 SQLite 缓存，不启动 Tokio / TLS / 钥匙串；有缓存但过期时可按配置触发后台静默同步。
- **全屏交互工作台**：终端直接运行 `campus` 进入「交大自由行 CLI」TUI，集成 10 个视图页（支持列表/周历切换、实时筛选、详情弹窗、附件下载与即时同步）。
- **物理在线完整闭环**：支持独立登录（通过本机 `127.0.0.1` 临时回环页拉起系统浏览器完成阿里云点选验证码）、课程/作业列表与详情、老师附件下载、草稿文件增删与最终提交确认。
- **多端密钥集成与隐私边界**：默认接入系统钥匙串（Linux Secret Service / macOS Keychain / Windows Credential Manager），显式 `--remember` 才保存密码；会话 Cookie 加密保存；不内置任何验证码自动识别或绕过逻辑。
- **脚本与 AI 友好**：原生支持 `--json`（统一 `schema: 1` 信封格式）与 `--format tsv`，方便管道处理、Shell 自动补全或接入本地自动化工具。

---

## 平台支持与安装路径

| 平台                           | 构建产物 / 包形式                                            | 系统钥匙串                                                   | 定时同步后端                         | 状态说明                                           |
| ------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------ | ------------------------------------ | -------------------------------------------------- |
| **Linux x86_64**               | 静态 musl 二进制、`.deb`、`.rpm`、`PKGBUILD`、`default.nix`  | `linux-native` / Secret Service（无钥匙串可选 `file` / `passphrase`） | `systemd --user`                     | 本地构建、解包与真机实测通过                       |
| **macOS** (`arm64` / `x86_64`) | Universal / 单架构 `.tar.gz`                                 | Apple Keychain (`apple-native`)                              | `launchd` (`~/Library/LaunchAgents`) | 已完成源码与 CI 适配，待 `macos-latest` / 真机实测 |
| **Windows x64**                | `campus-bjtu-<version>-windows-x64.zip`（含 `campus.exe` + `campus.ps1`） | Windows 凭据管理器                                           | 需手动或任务计划运行 `campus sync`   | 支持交叉构建（需 LLVM-MinGW 工具链）               |

> 安装/使用本地 DEB/RPM 包不会自动向远端包仓库发布；macOS 如从浏览器直接下载未公证归档，首次运行前可执行 `xattr -d com.apple.quarantine ./campus`。

---

## 快速上手

```sh
# 1. （仅无桌面钥匙串的无头服务器需要）显式选择加密文件或口令后端
campus config set vault.backend file

# 2. 登录统一认证（仅传 --remember 时保存密码，密码在终端隐藏输入）
campus auth login --user 学号 --remember

# 3. 同步全部数据到本地缓存
campus sync all

# 4. 启动「交大自由行 CLI」全屏工作台（非交互管道环境下可使用 campus tui）
campus
```

首次同步后，常用离线查询无需联网即可毫秒级返回：

```sh
campus schedule today --offline
campus grades gpa --offline
campus exams --upcoming --offline
campus card --offline
```

---

## 全屏工作台 (TUI)

在交互终端直接运行 `campus` 或 `campus tui`（加 `--offline` 可禁止联网同步）：

| 标签页 (`1`–`9`, `0`) | 数据源       | 专属功能                                                     |
| --------------------- | ------------ | ------------------------------------------------------------ |
| `1` 课表              | `courses`    | 按 `v` 切换列表/周历网格；`+` / `-` 切换教学周；`[` / `]` 窄屏横向翻页 |
| `2` 作业              | `homework`   | `Enter` 查看纯文本详情；绿/红区分已提交与未提交              |
| `3` 成绩              | `grades`     | 底部展示当前范围加权均分与 4.0 制 GPA                        |
| `4` 考试              | `exams`      | 考场、时间与状态一览                                         |
| `5` 教室              | `rooms`      | `b` / `B` 选楼宇，`+` / `-` 选周次，`r` 查询，红/棕/蓝/绿/黄区分占用状态 |
| `6` 一卡通            | `card`       | 校园卡余额、网费与未读邮件统计                               |
| `7` 学年历            | `calendar`   | 第 16 周倒排至第 1 周，北京时间当前周标为绿色「本周」        |
| `8` 课件              | `courseware` | `r` 同步目录，`d` 下载所选资源到 `./courseware/`             |
| `9` 同步              | `sync`       | 查看各模块最近同步状态、时间与失败原因                       |
| `0` 物理在线          | `phyvlab`    | `Enter` 查看实验要求与评语，`d` 下载老师附件，`r` 同步（支持草稿/逾期颜色标注） |

### 通用快捷键

| 按键                                                        | 操作                                                         |
| ----------------------------------------------------------- | ------------------------------------------------------------ |
| `Tab` / `Shift+Tab`、`←` / `→`、`1`–`9`、`0`                | 切换页面                                                     |
| `↑` / `↓`、`j` / `k`、`PageUp` / `PageDown`、`Home` / `End` | 移动选中行或翻页                                             |
| `Enter` / `Esc`                                             | 打开或关闭当前记录详情弹窗（弹窗内支持 `↑↓` / `PageUp` / `PageDown` 滚动） |
| `/`                                                         | 进入当前页实时关键词筛选（`Enter` 锁定，`Esc` 清除）         |
| `r`                                                         | 在后台刷新当前页数据并在底栏显示结果                         |
| `q`、`Ctrl+C`                                               | 退出工作台                                                   |

> 颜色自动跟随终端状态，可通过 `--color never` 或环境变量 `NO_COLOR=1` 关闭；禁用颜色时教室占用表会附带字符图例。

---

## CLI 命令速查

全局选项（所有子命令均可使用）：

- `-a, --account <学号>`：临时指定操作账号
- `--offline`：严格离线模式，仅读取本地缓存
- `--refresh`：强制联网刷新后再输出（与 `--offline` 互斥）
- `--json` / `--format <table|tsv|json>`：切换输出格式
- `--color <auto|always|never>`：控制 ANSI 着色
- `--tz <时区>`：覆盖时间显示时区
- `-q` / `-v`：静默模式或输出详细日志

### 1. 账号与认证 (`auth`)

```sh
campus auth login --user 学号 --remember          # 隐藏提示输入密码并记住
campus auth login --user 学号 --password-stdin    # 从标准输入读取密码（适合脚本/秘密管理器）
campus auth login --user 学号 --captcha           # 需要图形验证码时保存图片路径并交互输入
campus auth list                                  # 列出本地已保存账号
campus auth switch 学号                           # 切换默认账号
campus auth status                                # 查看本地会话与凭据保存状态
campus auth logout --forget                       # 注销并清除本地凭据与缓存会话
```

- 自动登录单次最多发起 3 次登录页 GET 与 1 次密码 POST；遇到密码错误、验证码要求或非预期响应即刻停止，绝不盲目重试导致锁号。

### 2. 课表、成绩、考试、一卡通与校历

```sh
campus schedule today --offline                   # today | tomorrow | now | week | all
campus schedule week 12 --grid --offline          # 周课表网格视图
campus grades --term 2025-2026-2 --offline
campus grades --type required --sort desc --detail --offline
campus grades gpa --offline                       # 100 分制加权均分与 4.0 制 GPA
campus grades select add 1 3 --offline            # 按成绩输出 index 自选计算范围（add/rm/clear）
campus grades gpa --selected --offline
campus exams --upcoming --offline                 # --upcoming | --all | --course 关键字
campus card --history --offline                   # 一卡通与网费
campus card topup                                 # 打印缴费二维码与链接
campus calendar now --offline                     # now | weeks（可加 --term）
```

- **均分与 GPA 规则**：按学分加权；百分制课程采用原分计算均分，纯字母等级课程采用项目既定折算表；`P/F` 两级制不计入均值，接口仅返回单独 `F` 无法判别评分制时暂不计入并在 `grades gpa --json` 中标出待确认数。

### 3. 教室查询 (`rooms`)

```sh
campus rooms buildings                            # 列出教学楼 ID
campus rooms free --building 11 --week 12 --at 'tue 3' --refresh
campus rooms grid 11 --week 12 --room SY201 --offline
campus rooms live 本部 --free --min-cap 40 --json # 实时教室人数估计
campus config set rooms.live off                  # 关闭第三方人数估计请求
```

- 教室网格按「学期 + 楼宇 + 周次」隔离缓存；未知占用状态不视为空闲。

### 4. 作业与课件下载 (`homework` / `courseware`)

```sh
campus homework list --offline
campus homework show <作业ID> --offline --json
campus homework download <作业ID> -o ./attachments

campus courseware ls --offline                    # 列出课程目录
campus courseware ls <课程ID> <文件夹ID> --offline
campus courseware get <课程ID>:<资源ID> -o ./courseware
campus courseware lessons <课程ID> --json
campus courseware record <课程ID>:<课节ID> --streams teacher,screen --remux -o ./recordings
```

- **旧版教学平台只读兼容**：当学校入口跳转到旧版教学端点时，默认 HTTPS 模式会明确提示而不自动降级明文传输。如需启用原版只读兼容流程，可显式开启：

  ```sh
  campus config set homework.legacy_http on       # 持久化开关（courseware.legacy_http 同理）
  CAMPUS_PORTAL_COURSEWARE_LEGACY_HTTP=on campus sync courseware  # 单次环境变量启用
  ```

- **下载与录像**：课件按 4 MiB Range 校验 `ETag` 断点续传；录像支持 VOD HLS 播放列表 4 路并发与顺序合并，拒绝加密/直播/fMP4 流；`--remux` 调用本机 `ffmpeg -c copy`。下载永不覆盖已有文件，断点记录仅保留 URL 摘要。

### 5. 物理在线 (`phyvlab`)

物理在线（`phyvlab.bjtu.edu.cn`）使用独立账号凭据与 `MoodleSession` 会话，与统一认证隔离保存：

```sh
campus phyvlab login --user 学号 --remember       # 遇阿里云验证码时拉起浏览器 127.0.0.1 回环页完成校验
campus phyvlab login --user 学号 --no-browser     # 仅打印本地验证 URL（SSH / 无桌面环境）
campus phyvlab status                             # 检查配置状态与会话可用性
campus phyvlab courses --offline                  # 已选物理实验课程与进度
campus phyvlab list --pending --offline           # 待完成作业（可加 --course 筛选）
campus phyvlab show <作业ID> --offline            # 作业说明、状态、批改成绩与附件列表
campus phyvlab download <作业ID> -o ./lab_files   # 下载老师附件
campus phyvlab upload <作业ID> report.pdf         # 上传草稿（不直接最终提交）
campus phyvlab upload <作业ID> --rm old.pdf       # 从草稿中移除文件
campus phyvlab finalize <作业ID> --yes            # 确认提交声明并最终提交
campus phyvlab logout --forget
```

- 遇验证码时，后台同步流程（`campus sync phyvlab`）绝不自动提交密码，而是返回需人机验证提示；若账号首次登录被服务端跳往 `/user/edit.php` 完善资料，会话仍保持已保存并在获取列表时给出 `phyvlab_profile_incomplete` 引导。

### 6. 同步、定时任务与导出 (`sync` / `changes` / `export`)

```sh
campus sync all                                   # 支持逗号分隔模块，如 sync grades,phyvlab
campus sync all --notify                          # 成绩或考试变动时发送系统通知（notify-send / osascript）
campus sync status --json                         # 各数据集最近同步结果与原因
campus sync timer install --every 1h              # 安装定时后台同步（30m..168h，Linux systemd / macOS launchd）
campus sync timer remove                          # 卸载定时任务
campus changes --unseen --mark-seen --json        # 查看并标记缓存数据变更
campus export ics --weeks 1-18 -o schedule.ics --offline
campus export csv grades -o grades.csv --offline
campus completions zsh                            # 生成 bash / zsh / fish 补全脚本
```

---

## 输出格式、退出码与配置

### JSON 协议与退出码

`--json` 输出统一信封结构（非旧版 JVM `agent` 协议）：

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

| 退出码 | 含义               | 常见原因                                       |
| ------ | ------------------ | ---------------------------------------------- |
| `0`    | 成功               | 查询或同步全部成功                             |
| `1`    | 本地错误           | 数据库/文件系统/配置异常                       |
| `2`    | 无可用缓存         | 离线模式无缓存且无网络更新                     |
| `3`    | 需要认证或人工处理 | 会话过期、密码错误、需人机验证或完善资料       |
| `4`    | 部分同步成功       | 一次同步多个模块时部分成功、部分失败           |
| `64`   | 参数错误           | 命令用法、取值范围或缺少确认标志（如 `--yes`） |
| `69`   | 网络错误           | 上游不可达或超时                               |
| `75`   | 同步锁占用         | 已有另一个同步进程持有 `sync.lock`             |

### 配置与缓存管理

配置文件采用 `key = value` 格式，支持同名的 `CAMPUS_PORTAL_<KEY_UPPER>` 环境变量覆盖：

```sh
campus config list
campus config get auto_refresh
campus config set auto_refresh off                # off | background | foreground
campus config set grades.fresh_seconds 21600      # 自定义新鲜度阈值（秒）
campus cache info --json
campus cache clear --yes                          # 清空当前账号业务缓存（保留凭据与配置）
campus cache vacuum
```

支持的配置键包括：`auto_refresh`、`vault.backend`、`rooms.live`、`homework.legacy_http`、`courseware.legacy_http`、`grades.fresh_seconds` 以及用于测试/私有镜像覆盖的 `endpoints.{mis,sso,aa,calendar,smart,headcount,phyvlab}`。

---

## 数据目录、凭据与安全模型

- **Linux / macOS（遵循 XDG 规范，目录强制 `0700`、文件 `0600`）**：
  - 数据库：`$XDG_DATA_HOME/campus-portal/campus.db`（默认 `~/.local/share/campus-portal/`）
  - 配置文件：`$XDG_CONFIG_HOME/campus-portal/config`（默认 `~/.config/campus-portal/`）
  - 临时验证码缓存：`$XDG_CACHE_HOME/campus-portal/`（默认 `~/.cache/campus-portal/`）
  - 进程同步锁：`$XDG_RUNTIME_DIR/campus-portal/sync.lock`（无则回退至缓存目录）
- **Windows**：默认全部置于 `%LOCALAPPDATA%\campus-portal\`。
- **密钥与加密存储**：
  - 密码（仅 `--remember` 时）与会话 Cookie 始终经 `XChaCha20-Poly1305` 加密后存储，默认主密钥托管在系统钥匙串；无钥匙串环境可选用 `file`（本地密钥文件）或 `passphrase`（由 `CAMPUS_PORTAL_PASSPHRASE` 解锁的 Argon2 密钥）。
  - 如需将课表、成绩等业务表本身也落盘加密，可启用 `sqlcipher` 特性构建全库加密版本；注意普通 SQLite 与 SQLCipher 库文件不可混用，请配合独立 `XDG_DATA_HOME`。

---

## 从源码构建与打包

### Linux（动态链接与静态 musl / DEB / RPM）

```sh
cd linux
cargo build --locked --release -p campus-cli
./target/release/campus --help

# 静态 musl 构建（要求 rust-lld 与 musl-gcc）+ 打包 DEB/RPM
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --locked --release \
  --target x86_64-unknown-linux-musl -p campus-cli
python3 scripts/package.py target/x86_64-unknown-linux-musl/release/campus
```

- 可选特性：`--no-default-features`（移除钥匙串与媒体模块进一步缩减体积）、`--features sqlcipher`（全库加密）。
- Arch Linux `PKGBUILD` 与 Nix `default.nix` 模板位于 [packaging/](packaging/)。

### macOS（Apple Silicon / Intel / Universal）

```sh
cd linux
# 本机单架构构建与打包
cargo build --locked --release -p campus-cli
python3 scripts/package-macos.py target/release/campus --arch arm64

# Universal 通用包 (arm64 + x86_64)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin -p campus-cli
cargo build --locked --release --target x86_64-apple-darwin -p campus-cli
lipo -create -output artifacts/campus-macos-universal \
  target/aarch64-apple-darwin/release/campus \
  target/x86_64-apple-darwin/release/campus
python3 scripts/package-macos.py artifacts/campus-macos-universal --arch universal
```

### Windows x64（Linux 交叉构建）

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

## 测试、对照验证与性能基准

```sh
cd linux
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p campus-cli --no-default-features
cargo test --locked -p campus-db --features sqlcipher

# Kotlin <-> Rust 解析与 SM2 密文逐字节对照
python3 scripts/prepare-kotlin-oracle.py
bash ../multiplatform/gradlew -p tests/kotlin-oracle --no-daemon -q run \
  --args="$PWD/tests/fixtures" > artifacts/kotlin.json
cargo run --locked -q -p campus-net --example oracle_vectors -- tests/fixtures > artifacts/rust.json
python3 scripts/compare-oracle.py artifacts/kotlin.json artifacts/rust.json

# 体积与离线启动延迟门禁检查（静态二进制 <= 10 MiB）
python3 scripts/benchmark.py target/x86_64-unknown-linux-musl/release/campus
```

- 开发专用页面脱敏工具（需启用 `developer` 特性，禁止用于 release 构建）：  
  `cargo run -p campus-cli --features developer -- debug capture --redact --input page.html -o fixture.html`
- [`fuzz/`](fuzz/) 提供登录页、校历、教学周与 HLS 播放列表解析的 `libFuzzer` 目标。详细测试与发布状态见 [docs/STATUS.md](docs/STATUS.md)。
