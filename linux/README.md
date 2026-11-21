# 交大自由行 CLI (`campus`)

**不到 10 MiB 的单二进制跨平台交大校园客户端** —— 用 Rust + SQLite 从底层重构，告别 JVM 与重型桌面壳，打开即秒开本地缓存、全屏工作台查课算分、终端一键下载课件与提交物理在线实验。

> 仓库首页与三分钟快速上手请看 **[README.md](../README.md)**；开发者编译、Crate 模块架构与基准测试请看 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) 与 [docs/STATUS.md](docs/STATUS.md)。

---

## 核心亮点

- **极小体积**：静态编译二进制严格小于 **10 MiB**（musl 单文件实测约 `9.0 MiB` 级，`.deb` / `.rpm` 约 `3–4 MiB`），零 JVM、零 Electron、零外置数据库依赖，一个文件拷走就能跑。
- **低内存与毫秒级响应**：离线查询与 TUI 启动时**不初始化 Tokio 异步运行时、TLS 网络栈与系统钥匙串**，直接同步读取本地 SQLite；100 次预热实测 `--version` p95 `2.69 ms`，课表/成绩/考试/一卡通离线读取 p95 `8.70–15.05 ms`；大课件采用 4 MiB 分片断点续传，凭据内存随用随擦除（`zeroize`）。
- **全屏工作台 (TUI)**：原生 `ratatui` 终端界面内置 **10 个功能页**（课表列表/周历网格、作业、成绩与 GPA、考试、五色教室周占用、校内邮箱、学年历、课件、同步状态、物理在线），启动秒开缓存、按 `r` 后台同步不卡界面。
- **五层架构重构**：将原客户端拆解为 `campus-core`（纯规则与解析）→ `campus-db`（SQLite + 加密钥匙串）& `campus-net`（`rustls` + 本地 SM2 + 回环验证码 + 流式下载）→ `campus-sync`（重登去重与隔离编排）→ `campus-cli`（CLI / TUI / 定时任务），离线与联网职责完全解耦。

---

## 一、安装指南（普通用户 To C）

### 下载安装包

前往仓库的 **[GitHub Releases](https://github.com/Tak1naa/BJTUselfService-Lite/releases)**（或开发分支最新构建 **[Actions → Linux Rust CLI → Artifacts](https://github.com/Tak1naa/BJTUselfService-Lite/actions/workflows/linux-rust.yml)**）下载对应系统压缩包/安装包：

### 1. Linux（Ubuntu / Debian / Fedora / Arch / 通用发行版）

- **Ubuntu / Debian / Linux Mint（`.deb`）**：
  ```sh
  sudo apt install ./campus-bjtu_1.0.0_amd64.deb
  campus --version
  # 卸载：sudo apt remove campus-bjtu
  ```
- **Fedora / RHEL / CentOS / AlmaLinux（`.rpm`）**：
  ```sh
  sudo dnf install ./campus-bjtu-1.0.0-1.*.x86_64.rpm
  campus --version
  # 卸载：sudo dnf remove campus-bjtu
  ```
- **任意 Linux 发行版 / WSL（便携 `.tar.gz`）**：
  ```sh
  tar -xzf campus-bjtu-1.0.0-linux-x86_64.tar.gz
  sudo install -m 755 ./campus /usr/local/bin/campus
  # 无 root 权限也可安装到用户目录：install -Dm755 ./campus ~/.local/bin/campus
  ```
- **Arch Linux**：可在 [`packaging/`](packaging/) 目录使用 `makepkg -si` 从源码打包安装。

> 提示：`.deb` 与 `.rpm` 已自动包含 `bash` / `zsh` / `fish` 命令补全及 `man campus` 手册。便携包用户可手动将压缩包内 `completions/` 复制到对应 Shell 补全目录。

### 2. macOS（Apple Silicon M 系列 & Intel）

> 说明：GitHub Releases 已自动构建并提供 macOS Universal / `arm64` / `x64` `.tar.gz` 归档（由 `macos-latest` CI 编译并跑通全部 Rust 单测；真实 macOS 桌面钥匙串弹窗与 `launchctl` 运行效果待真机实测）。

拿到 `campus-bjtu-1.0.0-macos-universal.tar.gz` 后：
```sh
tar -xzf campus-bjtu-1.0.0-macos-universal.tar.gz

# 若直接下载未经 Apple 公证的二进制，先解除系统隔离标记
xattr -d com.apple.quarantine ./campus

# 安装到 PATH（Apple Silicon Homebrew 环境可用 /opt/homebrew/bin/campus）
sudo install -m 755 ./campus /usr/local/bin/campus
campus --version
```
首次保存或读取密码时，macOS 会弹出「钥匙串访问」授权提示，点击 **「始终允许」** 即可。

### 3. Windows 10 / 11（64 位）

1. 解压 `campus-bjtu-1.0.0-windows-x64.zip` 到固定文件夹（例如 `D:\Tools\campus`）。
2. 直接双击 **`campus.exe`**（或在文件夹空白处右键 → **「在终端中打开」** 运行 `.\campus.exe`）：
   - 初次未登录时会自动进入**首次登录向导**；
   - 登录成功后会询问是否在桌面创建 **「交大自由行 CLI」** 快捷方式（含专属终端图标，也可用 `campus auth login --shortcut` / `--no-shortcut` 控制）；
   - 初次登录完成后会自动同步所有页面并刷新工作台。
3. （推荐）将 `D:\Tools\campus` 添加到系统环境变量 **`Path`** 后，以后可在任意终端直接输入 `campus` 运行。
4. 若弹出 Windows SmartScreen「已保护你的电脑」，点击 **「更多信息」 → 「仍要运行」** 。

---

## 二、首次使用（3 步跑起来）

### 第 1 步：登录校园账号

```sh
campus auth login --user 你的学号 --remember
```
- 终端会**隐藏输入**提示输入 MIS / 统一认证密码（不会回显字符）。
- 只有加了 `--remember` 才会将密码加密存入系统钥匙串；不加则只保存加密后的登录会话 Cookie。
- **如果学校要求输入图形验证码**，加 `--captcha` 参数：
  ```sh
  campus auth login --user 你的学号 --captcha --remember
  ```
  程序会把验证码图片保存到本地私有缓存并打印图片路径，打开图片查看后在终端输入即可。
- **如果是在没有桌面钥匙串的 Linux 服务器 / WSL / SSH 终端上使用**，先执行一次显式配置：
  ```sh
  campus config set vault.backend file
  ```

### 第 2 步：首次全量同步

```sh
campus sync all
```
程序会依次拉取你的课表、作业、成绩、考试、一卡通、校历与课件目录，写入本地 SQLite 缓存。

### 第 3 步：打开全屏工作台

```sh
campus
```
在交互终端直接输入 `campus`（或 `campus tui`）即可进入**「交大自由行 CLI」**全屏工作台。如果只想纯离线查看、不联网更新，运行 `campus tui --offline`。

---

## 三、全屏工作台 (TUI) 操作说明

工作台顶部共有 **10 个标签页**，可用数字键直达，或用 `Tab` / `←` `→` 切换：

| 快捷键 | 页面 | 你能做什么 |
| --- | --- | --- |
| `1` | 课表 | 默认列表；按 `v` 切换**周课表网格**，按 `+` / `-` 切教学周，按 `Enter` 看教室与老师详情 |
| `2` | 作业 | 未提交置顶并按截止时间排序；绿=已提交、红=未提交、灰=逾期未提交；按 `Enter` 看详情，按 `d` 下载老师附件至系统下载文件夹，按 `u` 交互式上传提交作业 |
| `3` | 成绩 | 查看各科成绩与学分，底栏实时显示当前筛选范围的**百分制均分与 4.0 GPA** |
| `4` | 考试 | 查看考试时间、地点与状态 |
| `5` | 教室 | 按 `b` / `B` 切换教学楼，`+` / `-` 选周次，按 `r` 刷新五色周占用网格 |
| `6` | 邮箱 | 查看校内邮箱收件箱（红=未读、绿=已读），按 `Enter` 读正文，按 `d` 下载邮件附件至系统下载文件夹 |
| `7` | 学年历 | 第 16 周至第 1 周倒排，以绿色标出北京时间「本周」 |
| `8` | 课件 | 按 `r` 同步目录，选中文件按 `d` 保存到系统下载文件夹 |
| `9` | 同步 | 一览各模块最近成功时间、同步状态及失败原因 |
| `0` | 物理在线 | 查看物理实验作业（绿=已提交、黄=草稿待提交、红=未提交/逾期），按 `l` 登录物理在线，按 `d` 下载老师附件 |

**通用快捷键**：
- `↑` / `↓` 或 `j` / `k` 移动选中行，`PageUp` / `PageDown` / `Home` / `End` 快速翻页
- `Enter` 打开详情弹窗，`Esc` 关闭弹窗（弹窗内也可用方向键上下滚动）
- `/` 在当前页按关键词实时过滤（`Enter` 锁定筛选，`Esc` 清空）
- `[` / `]` 在窄屏终端横向滚动周课表或教室网格
- `l` 启动交互式登录向导（在「物理在线」页按 `l` 登录物理在线独立账号，其它页面登录 MIS 并自动同步全部页面）
- `r` 后台刷新当前页数据（在「同步」页按 `r` 同步所有页面，不卡界面）
- `d` 在「作业」「邮箱」「课件」「物理在线」页下载选中项的附件或文件（前 3 项默认保存至系统「下载」文件夹）
- `u` 在「作业」页打开交互式上传向导（支持拖入多文件与作答说明，按平台规则覆盖提交）
- `U` 检查并从 GitHub 拉取后续版本更新（顶栏出现新版提示时也可直接按 `U` 升级）
- `q` 或 `Ctrl+C` 退出工作台

---

## 四、日常场景命令手册

### 1. 查课表 & 导出手机日历

```sh
campus schedule today --offline                   # 今天课表（也支持 tomorrow / now / week / all）
campus schedule week 12 --grid --offline          # 第 12 周网格课表
campus export ics --weeks 1-18 -o bjtu.ics --offline
```
生成的 `bjtu.ics` 可直接导入 iPhone 日历、macOS 日历、Google Calendar 或安卓日历应用（导出前请先同步课表与学年历）。

### 2. 查成绩、算均分与自选 GPA

```sh
campus grades --offline                           # 全部成绩
campus grades --term 2025-2026-2 --offline        # 按学期筛选
campus grades --type required --sort desc --offline
campus grades gpa --offline                       # 全部课程学分加权均分 + 4.0 GPA
campus grades select add 1 3 --offline            # 按输出编号挑选课程（add / rm / clear）
campus grades gpa --selected --offline            # 只计算已选课程 GPA
campus export csv grades -o grades.csv --offline  # 导出为 CSV 表格
```
> 说明：`P/F` 两级制课程不计入均分与 GPA；若接口仅返回单独 `F` 无法判别评分制，会暂不计入并在 `--json` 中给出待确认条数。

### 3. 查考试、校内邮箱、一卡通与缴费码

```sh
campus exams --upcoming --offline                 # 即将到来的考试（--all 查全部，--course 搜课程）
campus mail --offline                             # 查看校内邮箱收件箱
campus mail show <邮件ID> --refresh               # 拉取最新邮件正文与附件列表
campus mail download <邮件ID>                     # 下载邮件附件（默认保存至系统下载文件夹，可用 -o 指定目录）
campus card --history --offline                   # 校园卡余额与网费（命令行继续保留）
campus card topup                                 # 打印校园卡/网费充值二维码与链接
```

### 4. 找空教室 & 看实时人数

```sh
campus rooms buildings                            # 查看教学楼编号
campus rooms free --building 11 --week 12 --at 'tue 3' --refresh
campus rooms grid 11 --week 12 --offline          # 查整栋楼周占用网格
campus rooms live 本部 --free --min-cap 40        # 查看实时人数估计（可 config set rooms.live off 关闭）
```

### 5. 下载课件、作业附件与提交作业

```sh
campus homework list --offline
campus homework show <作业ID> --refresh           # 拉取作业正文、老师附件与已交附件清单
campus homework download <作业ID>                 # 默认下载至系统下载文件夹（可用 -o 指定目录）
campus homework download <作业ID> --submitted     # 同时下载自己已提交的文件
campus homework submit <作业ID> --file 报告.pdf --content "实验说明"   # 上传并覆盖提交作业

campus courseware ls --offline                    # 浏览课程列表
campus courseware ls <课程ID> <文件夹ID> --offline
campus courseware get <课程ID>:<资源ID>           # 默认下载至系统下载文件夹，支持 4 MiB 分片断点续传
campus courseware lessons <课程ID>
campus courseware record <课程ID>:<课节ID> --streams teacher,screen --remux -o ./videos
```
> 若学校跳转至旧版 HTTP 教学服务（`123.121.147.7:88`），默认自动启用受限的只读兼容会话；如需强制仅走 HTTPS，可关闭对应兼容：
> `campus config set homework.legacy_http off` / `campus config set courseware.legacy_http off`

### 6. 物理在线实验作业（独立账号）

物理在线需要校园网（或学校 VPN）及物理在线独立密码：

```sh
# 1. 登录物理在线：遇到阿里云点选验证码会自动打开浏览器本机页面供你点选完成
# （SSH 无桌面环境可加 --no-browser，手动复制终端打印的 http://127.0.0.1:... 地址到浏览器）
campus phyvlab login --user 你的学号 --remember

# 2. 查看课程与未完成实验
campus phyvlab courses --refresh
campus phyvlab list --pending --offline

# 3. 查看详情 & 下载老师附件
campus phyvlab show <作业ID> --offline
campus phyvlab download <作业ID> -o ./lab_attachments

# 4. 上传实验报告草稿（支持增删，不会立刻提交）
campus phyvlab upload <作业ID> report.pdf
campus phyvlab upload <作业ID> --rm old.pdf

# 5. 确认无误后最终提交（必须显式传 --yes）
campus phyvlab finalize <作业ID> --yes
```

### 7. 后台定时自动同步 & 系统通知

```sh
campus sync all --notify                          # 成绩/考试有变动时发送桌面通知
campus sync timer install --every 1h              # 每小时后台自动同步（支持 30m..168h）
campus sync timer remove                          # 取消定时任务
campus changes --unseen --mark-seen               # 查看上次同步以来有哪些数据变动
```
- **Linux** 使用用户级 `systemd --user`；**macOS** 使用用户级 `launchd`；**Windows** 请用系统「任务计划程序」定期调用 `campus.exe sync all`。

### 8. 多账号、自更新与脚本 JSON 输出

```sh
campus update --check                             # 仅检查 GitHub 是否有新版本
campus update                                     # 校验 SHA-256 后从 GitHub 拉取更新
campus update --yes                               # 跳过确认直接拉取更新
campus config set update.check off                # 关闭 TUI 后台自动检查更新提示

campus auth list                                  # 查看已保存账号
campus auth switch 另一个学号                     # 切换默认账号
campus -a 另一个学号 grades --offline             # 单条命令临时切换账号
campus grades --offline --json                    # 输出稳定 schema: 1 JSON 供脚本/AI 调用
```

---

## 五、常见问题（FAQ）与卸载清理

1. **提示找不到钥匙串（Linux 无桌面 / SSH / Docker 环境）**：  
   程序不会静默降级明文保存密钥。请手动二选一：`campus config set vault.backend file`（本地加密文件）或 `campus config set vault.backend passphrase`（配合环境变量 `CAMPUS_PORTAL_PASSPHRASE` 使用口令加密）。
2. **物理在线提示 `phyvlab_profile_incomplete`（需处理）**：  
   表示你的物理在线账号认证已保存成功，但学校网站将首次访问强制跳转到了个人资料修改页（`/user/edit.php`）。用浏览器打开 `https://phyvlab.bjtu.edu.cn/user/edit.php` 补全必填项并保存后，再执行 `campus sync phyvlab` 即可正常拉取作业。
3. **为什么部分课件显示「不可下载」？**  
   课程平台接口返回的 `stu_download` 字段仅在值为 `"2"` 时允许学生下载；其余值表示授课教师将该课件设为仅在线查看、未开放学生下载，因此 CLI 与 TUI 会保留「不可下载」状态并拒绝下载。
4. **不想显示终端彩色文字**：  
   在任意命令后加 `--color never`，或在环境中设置 `NO_COLOR=1`。
5. **如何完全退出并清除个人数据 / 卸载？**
   - 先注销并删除本地凭据：`campus auth logout --forget`、`campus phyvlab logout --forget`
   - 仅清空业务缓存保留账号：`campus cache clear --yes`
   - 彻底清理本地缓存目录（删除前确认无重要下载文件）：
     - **Linux / macOS**：`~/.local/share/campus-portal`、`~/.config/campus-portal`、`~/.cache/campus-portal`
     - **Windows**：`%LOCALAPPDATA%\campus-portal`
   - macOS 还可在「钥匙串访问」中删除服务名为 `campus-portal` 的条目；Linux 可通过 `apt remove` / `dnf remove` 卸载软件包。

---

## 六、更多文档

- 开发者架构细节、源码编译与接口契约：[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)
- 各版本真机测试、包体指标与当前状态记录：[docs/STATUS.md](docs/STATUS.md)
