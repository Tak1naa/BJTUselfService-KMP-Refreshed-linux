# 交大自由行 CLI (campus) · Linux / macOS / Windows 原生终端客户端

按 `rust_refactor_plan_20261118.md` 实现的 Rust + SQLite 分支。程序名为 `campus`，不依赖 JVM、常驻后端或 PostgreSQL。原有 `multiplatform/` 保留，作为其他平台实现与协议对照；本目录不修改它的构建。

Fedora RPM 已通过安装与真实账号基本同步验收。一周定时同步、智慧教学和完整发行版矩阵的验收状态见 [docs/STATUS.md](docs/STATUS.md)。

## 全屏 TUI

在终端直接运行 `campus`，或运行 `campus tui`。首次使用可先运行 `campus auth login`。

| 按键 | 操作 |
| --- | --- |
| `Tab` / `Shift+Tab`、`←` / `→`、`1`–`9` | 切换课表、作业、成绩、考试、教室、一卡通、学年历、课件、同步 |
| `↑` / `↓`、`PageUp` / `PageDown` | 移动选择或翻页 |
| `Enter`、`Esc` | 打开、关闭所选记录的详情 |
| `/` | 筛选当前页，回车确认，Esc 清除 |
| `r` | 在后台同步当前页，查看底部结果 |
| `q`、`Ctrl+C` | 退出 |

TUI 启动时直接读缓存，按 `r` 才发起同步。用 `campus tui --offline` 禁用联网。课表页按 `v` 切换列表和周历，周历按 `+` / `-` 切换教学周，`Enter` 查看完整课程详情。教室页按 `b` / `B` 选楼宇，按 `+` / `-` 选周次，再按 `r` 查询。周占用表按星期和节次分列，用红、棕、蓝、绿、黄区分排课、调课、考试、实验和其他安排。两个周视图在窄终端都可按 `[` / `]` 横向看日期，`--color never` 为教室表显示字符图例。课件页按 `r` 更新目录，可下载的文件按 `d` 保存到当前目录的 `courseware/` 下。作业详情以纯文本保留段落和换行，绿色表示已提交，红色表示未提交。学年历从第 16 周倒排到第 1 周，以北京时间标出绿色「本周」。成绩页底部显示当前筛选范围的加权均分与 4.0 制 GPA。导出和 JSON 等脚本操作沿用对应子命令。

## 构建

使用当前稳定 Rust、Cargo 和 C 编译器。开发机验证使用 Rust 1.99 / 1.100。首次构建会下载 `Cargo.lock` 锁定的依赖。

```sh
cd linux
cargo build --locked --release -p campus-cli
./target/release/campus --help
```

静态 musl 版本还需要 Rust 的 `x86_64-unknown-linux-musl` 标准库和 `musl-gcc`。`.cargo/config.toml` 使用 Rust 自带 `rust-lld` 链接器，避免宿主 GCC 的静态 PIE 配置影响启动。

```sh
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --locked --release \
  --target x86_64-unknown-linux-musl -p campus-cli
./target/x86_64-unknown-linux-musl/release/campus --version
```

默认启用 `keyring`、`media`；可用 `--no-default-features` 构建更小的无钥匙串、无媒体版本。`--features sqlcipher` 构建全库加密版本：它需要访问 vault 解锁数据库，因此不适用默认明文业务缓存的快速读取预算。普通 SQLite 和 SQLCipher 文件不能直接互换，请使用独立 `XDG_DATA_HOME`；程序不会破坏性重建旧库。

SQLite 同时保存结构化查询表和 JSON 快照。成绩等高频数据可以精确筛选；结构变动较多的数据可以按 JSON 缓存，同步时仍能在同一事务里更新快照、查询表与同步状态。课件和录像保存在文件系统。如果将来增加课件搜索，优先试本地关键词索引；只有在需要按自然语言找内容、且样本评测显示有收益时，再增加可重建的本地向量索引。

## macOS (Apple Silicon / Intel)

macOS 支持原生构建与 Universal (`arm64` + `x86_64`) 归档：使用 macOS 钥匙串（`apple-native` / Security.framework）保存密钥、XDG 默认目录 `~/.local/share/campus-portal`（支持 `XDG_*` 覆盖）、`open` 拉起浏览器完成物理在线人机验证、`osascript` 通知，以及 `launchd`（`~/Library/LaunchAgents/team.bjtuss.campus.sync.plist`）管理定时同步。

```sh
# 在 macOS 上构建并打包
cargo build --locked --release -p campus-cli
python3 scripts/package-macos.py target/release/campus --arch arm64

# 或构建 Universal (Apple Silicon + Intel) 归档
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin -p campus-cli
cargo build --locked --release --target x86_64-apple-darwin -p campus-cli
lipo -create -output artifacts/campus-macos-universal \
  target/aarch64-apple-darwin/release/campus \
  target/x86_64-apple-darwin/release/campus
python3 scripts/package-macos.py artifacts/campus-macos-universal --arch universal
```

## Windows x64

在 PowerShell 中解压发行 ZIP 后进入目录，运行 `.\campus.exe tui` 或 `.\campus.exe --help`。首次登录用 `.\campus.exe auth login`。可选的 `.\campus.ps1` 启动脚本透传参数和退出码。默认使用 `%LOCALAPPDATA%\campus-portal` 和 Windows 凭据管理器。定时同步命令依赖 Linux systemd，Windows 通过 `campus.exe sync` 手动同步。

在 Linux 上交叉构建 Windows 版需要 Rust 的 Windows Gnullvm 标准库和 LLVM-MinGW UCRT 工具链。将 LLVM-MinGW 的 `bin` 加入 PATH 后执行以下命令。

```sh
rustup target add x86_64-pc-windows-gnullvm
CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang \
AR_x86_64_pc_windows_gnullvm=llvm-ar \
CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang \
  cargo build --locked --release --target x86_64-pc-windows-gnullvm -p campus-cli
python3 scripts/package-windows.py target/x86_64-pc-windows-gnullvm/release/campus.exe
```

## 登录与查询

默认使用系统凭据存储（Linux 钥匙串、macOS 钥匙串或 Windows 凭据管理器）。只有显式 `--remember` 才在登录成功后保存密码；Cookie 始终加密持久化。无桌面钥匙串的主机，可**自行选择** `file` 或 `passphrase`，不会自动降级。

```sh
campus config set vault.backend file       # 无钥匙串时的明确选择
campus auth login --user 学号 --remember   # 密码在终端隐藏输入
campus sync all
campus schedule today --offline
campus schedule week 12 --grid --offline
campus grades --offline --json
campus grades --type required --offline
campus grades gpa --offline                # 100 分制加权均分和 4.0 制 GPA
campus grades select add 1 3 --offline     # 使用成绩输出的原始 index
campus grades gpa --selected --offline
campus exams --upcoming --offline
campus card --offline
campus calendar weeks --offline
campus sync status --json
```

均分和 GPA 均按学分加权。百分制成绩使用原分数计算均分，纯字母成绩按项目已有折算估算均分并按成绩单等级表计算绩点。P/F 两级成绩不参与计算。接口只给出单独的 F 时无法判别评分制，该条暂不计入，`grades gpa --json` 会给出待确认条数。

需要验证码时使用 `campus auth login --user 学号 --captcha`：图片存入私有缓存目录，终端显示路径，由用户打开图片并输入字母数字验证码。没有验证码识别模型，不假设它仍是算式验证码。自动登录只允许最多三次登录页获取和一次密码 POST；失败、验证码要求或结果不明确时停止，不重发密码。

`--password-stdin` 支持外部秘密管理器输入；不提供密码命令行参数。使用 `passphrase` 后端时，从 `CAMPUS_PORTAL_PASSPHRASE` 读取口令，后台任务不弹出提示。

`auth list`、`auth switch 学号` 支持多账号。`auth status` 默认报告本地保存状态，不把它当作服务器会话仍然有效的证明。`auth logout --forget` 清除本地会话与所保存密码，远端注销失败会明确报告。

## 缓存与输出

Linux 缓存目录遵循 XDG，应用目录模式为 0700，文件模式为 0600。Windows 默认目录继承当前用户的 Local AppData 访问权限。默认业务数据明文存储；密码、Cookie 使用系统钥匙串或 XChaCha20-Poly1305 加密。`file` 后端的密钥与密文处于同一用户目录，不提供整盘离线窃取防护。

- 数据库：`$XDG_DATA_HOME/campus-portal/campus.db`，默认 `~/.local/share/campus-portal/`。
- 配置：`$XDG_CONFIG_HOME/campus-portal/config`，格式 `key = value`。
- 临时验证码：`$XDG_CACHE_HOME/campus-portal/`。
- 同步锁：`$XDG_RUNTIME_DIR/campus-portal/sync.lock`，无 runtime 目录时回退至用户缓存目录。

离线读取不会启动 Tokio、HTTP/TLS 或钥匙串。默认先使用新鲜缓存；可用但过期的缓存触发后台同步；无缓存或超过可用期时前台刷新。刷新失败保留原数据和上次成功时间。

默认表格参考 VS Code Dark Modern，命令标题、表头和成功、失败、需处理状态使用语义色。窄终端改为逐条排列。终端自动配色，`--color never` 或 `NO_COLOR=1` 可关闭。`--format tsv` 使用真实制表符，JSON、TSV、导出文件和补全脚本均不带样式。

```sh
campus config set auto_refresh off        # off | foreground | background
campus config set grades.fresh_seconds 21600
campus grades --refresh --json
campus grades --format tsv --offline
campus changes --unseen --mark-seen --json
campus export ics --weeks 1-18 -o schedule.ics --offline
campus export csv grades -o grades.csv --offline
campus cache info --json
campus cache clear --yes                  # 只清当前账号业务缓存，保留凭据
```

JSON 使用 `schema: 1`、`command/account/generated_at/freshness/data/warnings`，不是旧 JVM `agent` 协议。错误返回 `schema/error`；退出码为 0 成功、1 本地错误、2 无缓存、3 需登录、4 同步部分成功、64 参数错误、69 网络错误、75 同步锁占用。同步失败原因另见 `sync status`。

ICS 导出使用与课表缓存时间对应的学期；导出前请先同步课表和学年历。

## 教室、作业与媒体

```sh
campus rooms buildings
campus rooms free --building 11 --week 12 --at 'tue 3' --refresh
campus rooms grid 11 --week 12 --offline
campus rooms live 本部 --json
campus config set rooms.live off
campus card topup                        # 只打印原项目小程序链接与二维码
campus sync homework,courseware
campus homework list --offline
campus homework --refresh
campus homework show 作业ID --offline --json
campus homework download 作业ID -o ./attachments
campus courseware ls --offline
campus courseware ls 课程ID 文件夹ID --offline
campus courseware get 课程ID:资源ID -o ./courseware
campus courseware lessons 课程ID --json
campus courseware record 课程ID:课节ID --streams teacher,screen -o ./recordings
```

教室网格按学期、楼宇、周次分区缓存；未知颜色不会视为空闲。第三方教室人数接口使用无 Cookie 客户端，允许原项目的精确 HTTP 地址，支持关闭；离线只使用两分钟内的最后结果。

作业默认使用 HTTPS。学校跳往旧服务时，可显式开启原版的只读兼容流程。它优先尝试模块 28，再经模块 104 的本科平台入口 `jwcZhjx` 建立临时会话。旧教学端点上的票据、会话和作业数据可能经 HTTP 传输。兼容范围仅涵盖作业读取，TUI 会持续显示模式提示。

```sh
campus config set homework.legacy_http on
campus sync homework
campus homework --offline
campus config set homework.legacy_http off   # 关闭兼容模式
```

若只想启用一次，运行 `CAMPUS_PORTAL_HOMEWORK_LEGACY_HTTP=on campus sync homework`。兼容会话随同步结束清理，已同步的作业继续保存在本地缓存。

课件有独立的 `courseware.legacy_http` 设置，默认关闭。用户启用后，客户端可从旧教学端点读取目录，并在用户选择下载时用 `rpinfoDownloadUrl` 换取即时票据，再从同一端点取文件。票据、会话和文件可能经 HTTP 明文传输。TUI 在启用时持续显示提示。Linux 上单次目录同步可用 `CAMPUS_PORTAL_COURSEWARE_LEGACY_HTTP=on campus sync courseware`，随后用 `campus courseware ls --offline` 读取目录。CLI 下载入口为 `campus courseware get 课程ID:资源ID -o 输出目录`。

课件按 4 MiB Range 下载，验证 ETag 后断点续传；缺少可靠验证器时退回完整下载。HLS 支持主列表、完整 VOD TS 分片、四路下载、续传与按播放列表顺序合并，拒绝加密、直播、fMP4 和字节区间列表。`--remux` 调用用户已安装的 `ffmpeg -c copy`；不移植原视频转换器。下载不覆盖已有输出，检查点只记录 URL 摘要，不保存签名查询参数。

## 定时与发行包

只有用户主动运行以下命令才安装或启用定时任务（Linux `systemd` 用户任务或 macOS `launchd` 代理），间隔至少 30 分钟。后台不会提示输入密码或验证码。

```sh
campus sync timer install --every 1h
campus sync timer remove
campus sync all --notify
campus completions bash
python3 scripts/package.py target/x86_64-unknown-linux-musl/release/campus
```

本地 DEB/RPM 产物在 `artifacts/packages/`；脚本只构建，不安装。Arch 本地构建脚本和 Nix 表达式位于 `packaging/`，没有向 AUR 或任何包仓库发布。

## 验证与对照

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p campus-cli --no-default-features
python3 scripts/prepare-kotlin-oracle.py
bash ../multiplatform/gradlew -p tests/kotlin-oracle --no-daemon -q run \
  --args="$PWD/tests/fixtures" > artifacts/kotlin.json
cargo run --locked -q -p campus-net --example oracle_vectors -- tests/fixtures > artifacts/rust.json
python3 scripts/compare-oracle.py artifacts/kotlin.json artifacts/rust.json
python3 scripts/benchmark.py target/x86_64-unknown-linux-musl/release/campus
```

对照程序编译原 Kotlin 解析器，使用同一份虚构 HTML/JSON，并逐字段比较；SM2 使用固定随机数逐字节比较 C1C3C2 密文。fixture 测试不等于真实学校接口验收。

开发专用本地页面结构脱敏工具需 `cargo run -p campus-cli --features developer -- debug capture --redact --input page.html -o fixture.html`。它移除脚本、外部地址、隐藏值和自由文本，生成物仍需人工核对并补齐虚构值；不读取浏览器、不登录或抓取线上页面。`developer` 同时允许测试时钟 `CAMPUS_PORTAL_NOW`，禁止用于 release 构建。

`fuzz/` 提供登录页、校历、周次和 HLS 的 libFuzzer 入口。验收进度见 [docs/STATUS.md](docs/STATUS.md)。
