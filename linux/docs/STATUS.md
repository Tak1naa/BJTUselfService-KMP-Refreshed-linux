# Linux Rust 本地验证记录

日期：2026-11-20。分支：`main`（由 `feat/linux-rust-cli` 合入），基于实际 BJTU 仓库。Kotlin Multiplatform 源码保留，可继续使用。

## 0.1.12 作业附件下载与提交、首次登录向导及 Windows 桌面快捷方式

- 新增作业详情刷新（`campus homework show <ID> --refresh`）、老师与已交附件下载（`campus homework download <ID> [--submitted] -o ./homework` 及 TUI `d` 键）以及作业上传提交（`campus homework submit <ID> --file ... [--content ...] [--yes]`），按 KMP 协议对齐双层正文编码、精简 `23 KiB` GBK 查表解码已交附件列表与严格上传/提交回执校验，仅在显式用户传输/提交作用域开放 `HomeworkAction`。
- 新增首次启动监测与引导：双击/运行 `campus` 且未登录时进入首次登录向导，TUI 亦支持按 `l` 登录或检测其他终端登录成功，首次登录完成后自动同步所有常用页面并刷新界面；`campus auth login` 首次登录无缓存时同样自动完成首轮全量同步。
- 新增 Windows 登录成功后可选自动创建桌面快捷方式（`交大自由行 CLI.lnk`，含透明圆角多尺寸 `campus.ico` 图标，支持 `--shortcut` / `--no-shortcut`，图标仅在 `#[cfg(windows)]` 嵌入，不增加 Linux 静态二进制体积）。
- 更新面向普通用户的 To-C 首页 [README.md](../../README.md)、[linux/README.md](../README.md) 与 [CONTRIBUTING.md](../../CONTRIBUTING.md)，附项目截图与常见 FAQ。
- 83 项工作区测试（含 GBK 查表与作业下载/提交 mock 测试）、`sqlcipher` 7 项测试、`--no-default-features` 编译、Clippy 与格式检查全部通过；`[profile.release]` 使用 `opt-level = "s"`，静态 musl 二进制体积 `9,409,184` 字节（8.97 MiB），100 次预热 `--version` p95 `2.69 ms`、离线查询 p95 `8.98–15.96 ms`。

## 0.1.11 作业列表排序与逾期未提交灰显

- 作业列表与 TUI「作业」页改为将 `未提交` 与 `已提交` 分别按截止时间（北京时间）升序排列，并将 `未提交` 置于上方，缺少截止时间的条目置于所属分组末尾。
- 已过截止时间的 `未提交` 作业在终端表格/详情与 TUI 表格/详情弹窗中以灰色（`MUTED`）显示提交状态，未逾期仍为红色，`已提交` 为绿色；`--json`、`--format tsv` 与 `--color never` 输出保持不变。
- 80 项工作区测试（含新增排序/逾期单测与 CLI 集成测试）、`sqlcipher` 7 项测试、`--no-default-features` 编译、Clippy 与格式检查全部通过；静态 musl 二进制体积 `10,295,840` 字节（9.82 MiB）。

## 0.1.10 BJTU TLS 中间证书补全与跨平台修复

- 定位并修复 `mis.bjtu.edu.cn` 自 2026-11-17 更换 `*.bjtu.edu.cn` 新证书后服务端漏发 `GeoTrust G2 TLS CN RSA4096 SHA256 2022 CA1` 中间证书导致 `rustls` 报错 `network request failed`（退出码 `69`）的问题（`0.1.9` 在 Wine 与本机 Linux 均已复现该问题）。
- 在 `campus-net` 中嵌入该 1,482 字节公开中间证书（与 `sso.bjtu.edu.cn` 及 DigiCert AIA 逐字节一致），仅当 `*.bjtu.edu.cn` 服务端未下发任何中间证书且触发 `UnknownIssuer` 时作为非信任中间提示参与链校验，仍严格校验完整证书链至 `DigiCert Global Root G2` 根证书与主机名；ALPN 保持 `http/1.1`。
- 修复 Windows 终端（GBK/CP936）下 `campus auth login` / `campus phyvlab login` 密码提示词中文乱码：由 `rpassword::prompt_password` 直接写 UTF-8 到 `CONOUT$` 改为标准库 `eprint!`（Windows 走 UTF-16 `WriteConsoleW`）+ `rpassword::read_password()`。
- 默认启用 `homework.legacy_http` 与 `courseware.legacy_http` 的受限只读兼容（仅在 MIS 跳转到 `http://123.121.147.7:88/` 时生效，可通过 `campus config set ... off` 关闭），并统一旧版作业与课件会话的 `article["sessionId"]` / `JSESSIONID` 读取逻辑及终端/TUI 报错引导。
- 78 项工作区测试（含 `campus-net` 29 项、`campus-sync` 5 项、`campus-cli` 13 项等）、`--no-default-features` 编译、`sqlcipher` 7 项测试、Clippy 与格式检查全部通过；静态 musl 二进制体积 `10,242,592` 字节（9.77 MiB）。
- 发布静态 musl 二进制与 Linux DEB / RPM / `tar.gz`、macOS Universal / `arm64` / `x64`、Windows x64 ZIP。

## 0.1.9 物理在线与本地打包

- 新增物理在线（`phyvlab`）模块：支持课程与作业列表/详情解析、老师 `/pluginfile.php/` 附件下载、草稿文件上传/移除与提交声明最终提交；独立凭据和 `MoodleSession` 会话按主账号隔离并加密保存。
- 支持零新增依赖的本机 `127.0.0.1` 一次性回环服务拉起浏览器完成阿里云图形点选验证，先校验 `/login/captcha_verify.php` 再单次提交密码；后台同步遇验证码时绝不提交密码。
- 真机实测已完成物理在线登录与会话持久化保存，并确认当账号被服务端重定向至 `/user/edit.php` 时保持已认证状态且返回 `需处理`（`phyvlab_profile_incomplete`）引导至网页完善个人资料。
- CLI 新增 `campus phyvlab`（`list`、`courses`、`show`、`download`、`upload`、`finalize`、`login`、`logout`、`status`），全屏 TUI（标题已更新为「交大自由行 CLI」）新增「物理在线」页（快捷键 `0`，支持详情查看、按 `d` 下载附件与 `r` 同步）。
- 76 项工作区测试（含 `campus-cli` 13 项、`campus-core` 25 项、`campus-db` 6 项、`campus-net` 28 项、`campus-sync` 4 项）、Clippy 与格式检查全部通过（`--no-default-features` 编译与 `sqlcipher` 7 项测试在同版本 `0.1.9` 更名前通过）。
- 静态 musl 二进制体积 `10,225,056` 字节（9.75 MiB，低于 10 MiB 门禁），100 次预热离线基准测试通过（`--version` p95 `2.69 ms`，离线各查询 p95 `8.70–15.05 ms`）。
- 已打包并解包校验 `artifacts/packages/campus-bjtu_0.1.9_amd64.deb`（`3,426,708` 字节）、`artifacts/packages/campus-bjtu-0.1.9-1.fc44.x86_64.rpm`（`3,676,903` 字节）与 `artifacts/packages/campus-bjtu-0.1.9-linux-x86_64.tar.gz`（`4,402,564` 字节），包内 `/usr/bin/campus`（或便携包 `campus`）与静态二进制 SHA-256 逐字节一致；校验值已写入 `artifacts/SHA256SUMS`。当前环境未安装 LLVM-MinGW，本轮未构建 Windows x64 ZIP。
- 已完成 macOS 源码与 CI 适配：`campus-db` 拆分 `target_os = "macos"`（`keyring` `apple-native` 接入系统钥匙串）与 `target_os = "linux"`（`linux-native-async-persistent`），`campus timer` 新增 macOS `launchd`（`~/Library/LaunchAgents/team.bjtuss.campus.sync.plist`）定时任务与 `plutil -lint` 校验测试，`sync --notify` 新增 macOS `osascript` 系统通知，并新增 `scripts/package-macos.py` 与 `.github/workflows/linux-rust.yml` 的 `macos-latest` Universal 构建测试流水线（本机为 Linux x86_64，macOS 目标交由 GitHub Actions `macos-latest` 验证）。

## 0.1.8 课件目录同步

- 旧服务的 `getCourseList` 在收到 Cookie 对应的会话 ID 时返回 HTTP 200 和 GBK 编码的登录提示页。公告接口提供了不同的 API `sessionId`。旧版课件模式现在优先使用公告提供的 API 会话 ID，拿不到时用握手 Cookie。
- 同一真实会话改用 API 会话 ID 后，`getCourseList` 返回 UTF-8 JSON。成功同步和离线读回 7 门课、43 个文件，43 个均有下载选项。兼容只在同步进程中启用，持久配置保持关闭。 解包后的 0.1.8 RPM 再次同步成功，目录内容一致。伪终端验证了离线课件页和下载选项。
- 65 项 Rust 测试及 Clippy、格式和包完整性检查通过。DEB 和 RPM 解包后与静态二进制一致。Windows ZIP 为 3,504,785 字节，`campus.exe` 为 7,792,640 字节，Wine 中的解包程序已通过版本和离线 GPA 检查。包位于 `linux/artifacts/courseware-0.1.8/packages/`。

## 0.1.7 作业、成绩、学年历和 Windows 构建

- 作业详情清理 HTML 标记和实体，保留文本段落。TUI 与 CLI 分别以绿、红显示已提交和未提交。学年历 TUI 显示当前学期的第 16 周至第 1 周，北京时间当前周显示「本周」。
- 加权均分和 4.0 制 GPA 按学分计算。绩点使用成绩单等级对照，字母成绩的百分制均分采用项目原有折算。P/F 两级成绩不参与计算，评分制未知的单独 F 暂时排除。
- 课件会话优先使用登录跳转设置的 JSESSIONID，补齐原版首个列表请求的会话头。模拟服务验证了课程与目录读取，也回归了下载票据。真实账号仅 HTTPS 的目录请求返回 `secure_channel`。随后用既有会话单次开启旧版兼容执行目录同步，返回 `session_expired`。本地课件目录仍为空，兼容设置保持关闭。11 月 19 日重新登录后，HTTPS 查询返回 `smart_legacy_http`，确认课件入口仍指向旧版服务。使用新会话单次开启兼容查询后返回 `malformed_response`（JSON），课件缓存仍为空。源码随后补齐 Kotlin 对已有 Cookie 的处理，当会话 Cookie 存在时校验公告响应状态和会话，只在缺少 Cookie 时解析公告 JSON。模拟服务器返回 HTML 公告时已验证能继续读取目录，64 项测试通过。11 月 19 日用修补后的程序单次实测，已通过会话初始化和当前学期解析，课程列表 `getCourseList` 响应仍无法解析为 JSON 对象，课件缓存为 0 条，旧版兼容设置保持关闭。现有抓包只覆盖下载票据，无法据此确认课程列表响应格式。原发布包未更新。
- 64 项 Rust 测试、Clippy、格式检查通过。合成数据在 108 列和 42 列伪终端通过文字、颜色、成绩计算和无色模式验证。RPM 与 DEB 解包后与静态二进制一致。
- 产物在 `linux/artifacts/updates-0.1.7/packages/`。Windows x64 ZIP 为 3,503,388 字节，解包后的 `campus.exe` 为 7,790,592 字节。Wine 中已运行解包后的 PE，完成版本检查、本地配置改写与 SQLite 离线 GPA 读取。Wine 下 PowerShell 7 的运行时未通过启动检查，实际 Windows 终端与 PowerShell 启动脚本仍待真机验证。

## 0.1.6 课表合并与周历

- 正式课表与选课课表按课程编号、去掉校方本科标记的名称、星期节次、地点、教学周合并。正式记录优先，保留不同班号、时段、地点和周次。新同步、旧缓存读取、TUI 与导出共用规则。
- 课表页按 `v` 在列表与周历之间切换。周历默认当前教学周，支持换周、横向查看日期、筛选和打开课程详情。
- 旧缓存中实际存在 22 条记录，来自两种来源的 11 组重复。新版读取旧缓存得到 11 条，使用真实教务会话重新同步后也保存并读回 11 条。
- 61 项 Rust 测试、Clippy 和性能检查通过。解包后的 RPM 通过 42 列和 110 列终端的周历切换、换周、课程详情与教室网格回归。

## 0.1.5 教室周占用表

- 全屏 TUI 固定左列显示教室与容量，表头显示星期、可用的校历日期和节次。占用状态由独立色块表示，支持无色字符图例。
- 窄终端用 `[` / `]` 查看其他日期，教室列表可竖向滚动，楼宇、周次、查询和筛选按键继续可用。
- 42 列和 110 列伪终端验证了颜色、日期表头、横向和纵向移动，以及无色字符模式。60 项 Rust 测试、Clippy 与格式检查通过。

## 0.1.4 教室和课件

- 教室页可直接选择楼宇与周次，并发起查询。首次打开以缓存的当前教学周或第 1 周为起点。教务请求省略内部的 `current` 学期占位符，显式查询第一页。
- 课件页列出文件及下载状态。按 `d` 获取即时票据并保存到 `./courseware/`。兼容模式单独启用，限定到现有教学服务及抓包核对的票据和下载路径。
- 使用合成票据验证了 POST 请求、文件下载和禁止跨源。真实课件验证仍待授权，避免在未经明确许可时通过 HTTP 使用登录会话。
- 真实教务会话查询思源楼 37 间，离线缓存同为 37 间。60 项测试、Clippy、格式和性能检查通过。RPM 与 DEB 已解包运行，TUI 楼宇、周次和下载操作通过 42–110 列终端验收。

## 0.1.3 作业兼容

- 默认采用 HTTPS。用户明确选择兼容模式时，使用原版模块 28 与本科服务平台 `jwcZhjx` 回退，逐跳验证固定的学校端点。
- 真实账号通过模块 104 回退同步 34 条作业。本次仅单次启用兼容模式。会话使用独立 Cookie 存储，SQLite 只保存筛选后的作业字段。
- 旧 HTTP 传输只用于作业查询，限定已核对的教学端点和路径。CLI 与 TUI 显示明文提示。
- 关闭兼容模式后确认 34 条作业均可离线读取。58 项 Rust 测试、Clippy 与格式检查通过。RPM 和 DEB 已独立解包并核对二进制，TUI 明文提示在 42 列和 110 列终端通过验证。

## 0.1.2 全屏 TUI

- 在交互终端运行 `campus` 或 `campus tui` 可切换九个缓存页面、滚动选行、查看详情、筛选和后台同步。教室页使用最近缓存的查询条件。
- 用真实会话复测，MIS 的智慧教学入口跳到旧版 `http://123.121.147.7:88`。该 IP 的公开登录页返回 HTTP 200，HTTPS 域名的同名页面返回 404。Rust 在发出明文会话请求前退出，并在作业页解释原因。
- 公开页面连通性和合成缓存界面已验收，作业的在线同步仍缺少可用的 HTTPS 入口。
- 56 项测试通过，Clippy 与格式检查通过。RPM 和 DEB 均已解包并运行。用解包的 RPM 在 42 列及 110 列伪终端验证了切页、详情、筛选、同步反馈和退出恢复。升级包在本地 `artifacts/tui-0.1.2/packages/`。

## 0.1.1 终端界面

- 终端输出新增 VS Code 风格的颜色、中文标签、对齐表格、周课表和教室占用视图。窄终端显示逐条记录。JSON 和 TSV 保留机器可读格式。
- 用户的真实作业同步记录为 `secure_channel`，且没有成功快照。新版本把失败原因和操作提示显示在作业命令里。智慧教学的安全通道仍待打通。
- 55 项 Rust 测试通过，Clippy 和格式检查通过。用真实作业失败记录确认了新版输出。解包 RPM 后在 38 至 110 列伪终端验证了配色、表格、周视图、失败提示、帮助及无色输出。
- 本地 RPM 和 DEB 均已解包、运行和核对 SHA-256。升级包在本机 `artifacts/ui-0.1.1/packages/`，现有系统安装由用户通过 PackPort 升级。

## 0.1.0 已验证

- Rust workspace 54 项测试通过：覆盖命令契约、课程与成绩、会话恢复、防止重复登录提交、会话持久化、同步失败保留快照、课表学期选择、课件断点续传与 HLS 顺序合并。学校业务请求由 mock server 模拟。
- Kotlin 对照工具编译原仓库的实现：使用同一组合成样例，逐字段比对课程、成绩、考试、个人信息、一卡通、教室、学年历、课程类型；固定随机标量的 SM2 加密还逐字节匹配。
- 默认库与 SQLCipher 分别测试。SQLCipher 的 7 项存储测试通过，包括数据库重新打开、错误密钥和无密钥不能读取。file 与 passphrase 凭据后端通过本地测试。
- 格式、Clippy、无默认功能的 developer 构建通过。systemd 单元通过 `systemd-analyze verify`，未写入或启用用户服务。
- 默认依赖树未引入 OpenSSL 或原生 libdbus。RustSec 主锁文件审计为 0 项已知漏洞，系统钥匙串依赖链仍有 3 项未维护警告（`zbus_names`、`zvariant`、`zvariant_derive`）。
- 静态版 CLI 导出的 ICS 已由独立的 libical 解析：无解析错误，学期、UTC 开始时刻与中文标题均与样例一致。
- 登录页、学年历及 HLS 解析的 libFuzzer 入口完成 162,529 次冒烟迭代，无崩溃；该次运行未开启覆盖率或 sanitizer。独立 fuzz 锁文件审计为 0 项已知漏洞、无警告。
- PackPort 安装了 Fedora RPM。`/usr/bin/campus` 与已测试的静态二进制 SHA-256 一致，版本、帮助和空缓存初始化均可运行。
- 用户登录后，用已安装的 CLI 复用真实会话。课表、成绩、课程类型、考试、学年历首次同步成功。一卡通首次报网络错误，单独复测后同步成功。课表、成绩、考试、一卡通和学年历均可离线读取。真实缓存导出的 ICS 由 libical 复核，有事件且无解析错误。

## 性能与体积

Fedora 44 x86-64 上以 Rust 1.100 构建了 musl 和 glibc release。musl ELF 由 `file` 确认为静态链接、已剥离符号，体积 9,035,680 字节（8.62 MiB）。

静态版分别预热后各运行 100 次，使用独立的合成 SQLite 缓存，耗时包括新进程启动、数据库读取和 JSON 序列化：

| 命令 | p95 |
| --- | ---: |
| `--version` | 2.35 ms |
| 今日课表（100 条） | 15.11 ms |
| 成绩（1,000 条） | 14.62 ms |
| 考试 | 8.99 ms |
| 一卡通 | 7.93 ms |

数据见本地 `artifacts/benchmark.json`；该结果不代表联网同步或其他机器的耗时。

已生成本地 AMD64 DEB 和 x86-64 RPM（约 2.92 MiB 和 3.14 MiB）。两个包均已独立解包并运行 `campus --version`。包内可执行文件与已测试的静态产物逐字节一致，附三种 shell 补全和 man 页。交付文件与校验值在本地 `artifacts/`、`artifacts/packages/` 和 `artifacts/SHA256SUMS`。

## 尚待验收

- 真实联调从用户已登录后的会话开始，未独立观察验证码交互与会话过期后的自动恢复，未验证智慧教学回跳或下载。未采集真实页面 fixture。开发版脱敏工具仅处理用户提供的本地文件。
- 未单独验收 Secret Service 读写，未启用或验证连续一周的定时同步，也未在 GNOME Calendar 或 Thunderbird 实际导入。
- P7 的邮箱、图书馆、物理实验、CITEL、成绩单下载和提交作业属于可选后续范围。未实现可选的终端内验证码预览；可用手工验证码流程。没有移植原 Kotlin 视频转换器。
- Arch 提供本地 `PKGBUILD`，Nix 提供打包表达式；本机缺少对应工具，未构建，亦未向 AUR 或其他渠道发布。带覆盖率和 sanitizer 的 fuzz 留待 CI，尚未在这里运行。
