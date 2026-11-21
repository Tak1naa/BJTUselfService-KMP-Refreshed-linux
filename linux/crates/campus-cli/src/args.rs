use clap::{Parser, Subcommand, ValueEnum};
#[derive(Parser, Debug)]
#[command(
    name = "campus",
    version,
    about = "交大自由行 CLI（BJTU 跨平台离线优先校园服务）"
)]
pub struct Args {
    #[arg(short = 'a', long, global = true)]
    pub account: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true, value_enum, default_value = "table")]
    pub format: Format,
    #[arg(long, global = true, conflicts_with = "refresh")]
    pub offline: bool,
    #[arg(long, global = true)]
    pub refresh: bool,
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub color: Color,
    #[arg(long, global = true)]
    pub tz: Option<String>,
    #[arg(short = 'q', global = true)]
    pub quiet: bool,
    #[arg(short='v',action=clap::ArgAction::Count,global=true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Format {
    Table,
    Tsv,
    Json,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Color {
    Auto,
    Always,
    Never,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 打开全屏校园服务界面
    Tui,
    #[cfg(feature = "developer")]
    /// 处理本地调试样例
    Debug {
        #[command(subcommand)]
        action: Debug,
    },
    /// 登录、切换账号或查看会话
    Auth {
        #[command(subcommand)]
        action: Auth,
    },
    /// 查看课表或周课表网格
    Schedule {
        #[arg(default_value = "today",value_parser=["today","tomorrow","now","week","all"])]
        view: String,
        #[arg(value_parser=clap::value_parser!(u8).range(1..=30))]
        week: Option<u8>,
        #[arg(long,value_parser=clap::value_parser!(u8).range(1..=7))]
        day: Option<u8>,
        #[arg(long)]
        grid: bool,
    },
    /// 查看成绩、课程类别和加权均分
    Grades {
        #[arg(long)]
        term: Option<String>,
        #[arg(long = "type")]
        kind: Option<String>,
        #[arg(long, default_value = "orig",value_parser=["orig","asc","desc"])]
        sort: String,
        #[arg(long)]
        detail: bool,
        #[command(subcommand)]
        action: Option<Grades>,
    },
    /// 查看考试安排
    Exams {
        #[arg(long)]
        upcoming: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        course: Option<String>,
    },
    /// 查空闲教室、周占用或实时人数
    Rooms {
        #[command(subcommand)]
        action: Rooms,
    },
    /// 查看一卡通余额或打开充值入口
    Card {
        #[arg(long)]
        history: bool,
        #[command(subcommand)]
        action: Option<Card>,
    },
    /// 查看校内邮箱收件箱、邮件正文或下载附件
    Mail {
        #[arg(default_value = "list", value_parser = ["list", "show", "download"])]
        action: String,
        id: Option<String>,
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
    },
    /// 查看教学周与学年历
    Calendar {
        #[arg(default_value = "now",value_parser=["now","weeks"])]
        view: String,
        #[arg(long)]
        term: Option<String>,
    },
    /// 更新缓存、查看同步状态或管理定时器
    Sync {
        #[arg(default_value = "all")]
        datasets: String,
        #[arg(long, hide = true)]
        background: bool,
        #[arg(long)]
        notify: bool,
        #[arg(long, hide = true)]
        datasets_override: Option<String>,
        #[command(subcommand)]
        action: Option<Sync>,
    },
    /// 导出日历、JSON 或 CSV
    Export {
        #[arg(value_name = "FORMAT",value_parser=["ics","json","csv"])]
        kind: String,
        dataset: Option<String>,
        #[arg(long, default_value = "1-30")]
        weeks: String,
        #[arg(long)]
        no_exams: bool,
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
    },
    /// 查看缓存中的数据变动
    Changes {
        #[arg(long)]
        unseen: bool,
        #[arg(long)]
        mark_seen: bool,
    },
    /// 浏览课件、下载资源或保存录像
    Courseware {
        #[arg(default_value = "ls",value_parser=["ls","get","lessons","record"])]
        action: String,
        id: Option<String>,
        path: Option<String>,
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
        #[arg(long)]
        remux: bool,
        #[arg(long, default_value = "teacher,screen")]
        streams: String,
    },
    /// 查看作业、详情、下载附件或上传提交
    Homework {
        #[arg(default_value = "list",value_parser=["list","show","download","submit"])]
        action: String,
        id: Option<String>,
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
        /// 同时下载已提交附件（download 时生效）
        #[arg(long)]
        submitted: bool,
        /// 待提交附件路径，可重复指定（submit 时生效）
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<std::path::PathBuf>,
        /// 提交说明正文（submit 时生效）
        #[arg(long, default_value = "")]
        content: String,
        /// 跳过覆盖提交确认（submit 时生效）
        #[arg(long)]
        yes: bool,
    },
    /// 查看物理在线课程、作业、下载附件或提交
    Phyvlab {
        #[command(subcommand)]
        action: Option<PhyvlabAction>,
    },
    /// 查看或修改本地设置
    Config {
        #[arg(value_parser=["get","set","unset","list"])]
        action: String,
        key: Option<String>,
        value: Option<String>,
    },
    /// 查看或维护本地缓存
    Cache {
        #[arg(default_value="info",value_parser=["info","clear","vacuum"])]
        action: String,
        #[arg(long)]
        yes: bool,
    },
    /// 输出 Shell 补全脚本
    Completions {
        #[arg(value_parser=["bash","zsh","fish"])]
        shell: String,
    },
    /// 检查或从 GitHub 拉取后续版本更新
    Update {
        /// 仅检查是否有新版本，不下载安装
        #[arg(long)]
        check: bool,
        /// 跳过交互确认直接下载安装
        #[arg(long)]
        yes: bool,
    },
}
#[cfg(feature = "developer")]
#[derive(Subcommand, Debug)]
pub enum Debug {
    Capture {
        #[arg(long, required = true)]
        redact: bool,
        #[arg(long)]
        input: std::path::PathBuf,
        #[arg(short = 'o', long)]
        output: std::path::PathBuf,
    },
}
#[derive(Subcommand, Debug)]
pub enum Auth {
    Login {
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        password_stdin: bool,
        #[arg(long)]
        no_remember: bool,
        #[arg(long)]
        remember: bool,
        #[arg(long)]
        captcha: bool,
        /// 登录后在 Windows 桌面创建快捷方式
        #[arg(long, conflicts_with = "no_shortcut")]
        shortcut: bool,
        /// 登录后跳过 Windows 桌面快捷方式创建提示
        #[arg(long)]
        no_shortcut: bool,
    },
    Logout {
        #[arg(long)]
        forget: bool,
    },
    Status,
    Switch {
        id: String,
    },
    List,
}
#[derive(Subcommand, Debug)]
pub enum Grades {
    Gpa {
        #[arg(long)]
        term: Option<String>,
        #[arg(long)]
        selected: bool,
    },
    Select {
        #[arg(value_parser=["add","rm","clear"])]
        action: String,
        indices: Vec<usize>,
    },
}
#[derive(Subcommand, Debug)]
pub enum Rooms {
    Free {
        #[arg(long)]
        building: Option<String>,
        #[arg(long, default_value = "now")]
        at: String,
        #[arg(long,value_parser=clap::value_parser!(u8).range(1..=53))]
        week: Option<u8>,
    },
    Grid {
        building: String,
        #[arg(long,value_parser=clap::value_parser!(u8).range(1..=53))]
        week: Option<u8>,
        #[arg(long)]
        room: Option<String>,
    },
    Live {
        building: String,
        #[arg(long)]
        free: bool,
        #[arg(long)]
        min_cap: Option<u32>,
    },
    Buildings,
}
#[derive(Subcommand, Debug)]
pub enum Card {
    Topup,
}
#[derive(Subcommand, Debug)]
pub enum Sync {
    Status,
    Timer {
        #[arg(value_parser=["install","remove"])]
        action: String,
        #[arg(long, default_value = "1h")]
        every: String,
    },
}
#[derive(Subcommand, Debug)]
pub enum PhyvlabAction {
    /// 列出物理在线作业（默认）
    List {
        #[arg(long)]
        course: Option<String>,
        #[arg(long)]
        pending: bool,
    },
    /// 列出已选物理在线课程
    Courses,
    /// 查看单个物理在线作业详情
    Show { id: i64 },
    /// 下载作业说明中的老师附件
    Download {
        id: i64,
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
    },
    /// 上传或移除草稿文件（不直接最终提交）
    Upload {
        id: i64,
        files: Vec<std::path::PathBuf>,
        #[arg(long = "rm")]
        remove: Vec<String>,
    },
    /// 确认并最终提交物理在线作业草稿
    Finalize {
        id: i64,
        #[arg(long)]
        yes: bool,
    },
    /// 配置并登录物理在线独立账号（支持点选验证码）
    Login {
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        password_stdin: bool,
        #[arg(long)]
        no_remember: bool,
        #[arg(long)]
        remember: bool,
        #[arg(long)]
        no_browser: bool,
    },
    /// 退出物理在线会话
    Logout {
        #[arg(long)]
        forget: bool,
    },
    /// 查看物理在线账号与会话状态
    Status,
}
