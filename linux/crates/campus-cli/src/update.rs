use crate::{Args, emit, now, open_database, runtime};
use campus_core::update::{ReleaseInfo, checksums_asset, is_newer, select_asset};
use campus_core::{Error, Result};
use campus_db::{Database, config::Config, paths::Paths, storage};
use serde_json::json;
use std::io::IsTerminal;
use std::path::Path;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const UPDATE_CHECK_TTL_SECS: i64 = 86_400;

pub(crate) fn cleanup_stale_old_exe() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_extension("exe.old");
        let _ = std::fs::remove_file(old);
    }
}

pub(crate) fn cached_newer_tag(db: &Database) -> Option<String> {
    let tag = db.pref("", "update_latest_tag").ok().flatten()?;
    is_newer(CURRENT_VERSION, &tag).then_some(tag)
}

pub(crate) fn should_background_check(db: &Database, current_ts: i64) -> bool {
    let last = db
        .pref("", "update_checked_at")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    current_ts.saturating_sub(last) >= UPDATE_CHECK_TTL_SECS
}

pub(crate) fn background_check(paths: &Paths, config: &Config) -> Option<String> {
    let rt = runtime().ok()?;
    let latest = rt
        .block_on(campus_net::update::check_latest(CURRENT_VERSION))
        .ok()
        .flatten();
    if let Ok(db) = open_database(paths, config) {
        let _ = save_check_state(&db, latest.as_ref());
    }
    latest.map(|r| r.tag)
}

fn save_check_state(db: &Database, latest: Option<&ReleaseInfo>) -> Result<()> {
    db.set_pref("", "update_checked_at", Some(&now().to_string()))?;
    let tag = latest.map(|r| r.tag.as_str()).filter(|s| !s.is_empty());
    db.set_pref("", "update_latest_tag", tag)
}

pub(crate) fn run(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    check: bool,
    yes: bool,
) -> Result<u8> {
    if args.offline {
        return Err(Error::Usage(
            "检查或拉取更新需要联网，请移除 --offline 后重试".into(),
        ));
    }
    if !args.json && !args.quiet {
        eprintln!("正在检查 GitHub 最新版本（当前 {CURRENT_VERSION}）…");
    }
    let latest = runtime()?.block_on(campus_net::update::check_latest(CURRENT_VERSION))?;
    save_check_state(db, latest.as_ref())?;

    let Some(release) = latest else {
        emit(
            args,
            "update",
            None,
            json!({
                "current": CURRENT_VERSION,
                "latest": CURRENT_VERSION,
                "update_available": false,
                "updated": false,
            }),
        )?;
        return Ok(0);
    };

    if check {
        emit(
            args,
            "update",
            None,
            json!({
                "current": CURRENT_VERSION,
                "latest": release.version,
                "tag": release.tag,
                "url": release.html_url,
                "update_available": true,
                "updated": false,
            }),
        )?;
        return Ok(0);
    }

    let payload = apply_release_update(db, paths, &release, yes, args.json)?;
    emit(args, "update", None, payload)?;
    Ok(0)
}

pub(crate) fn interactive_tui_update(db: &Database, paths: &Paths) -> Result<Option<String>> {
    use std::io::Write;
    eprintln!("正在检查 GitHub 最新版本（当前 {CURRENT_VERSION}）…");
    let latest = runtime()?.block_on(campus_net::update::check_latest(CURRENT_VERSION))?;
    save_check_state(db, latest.as_ref())?;

    let Some(release) = latest else {
        eprintln!("当前已是最新版本 campus {CURRENT_VERSION}。");
        wait_enter();
        return Ok(None);
    };

    match apply_release_update(db, paths, &release, false, false) {
        Ok(result) => {
            if result["updated"] == true {
                eprintln!(
                    "更新完成！请退出并重新启动 campus 以运行新版本 {}。",
                    release.tag
                );
            } else if let Some(cmd) = result["install_command"].as_str() {
                eprintln!("已校验下载新版本包，请按上方提示命令安装：{cmd}");
            }
            eprint!("按 Enter 返回工作台…");
            let _ = std::io::stderr().flush();
            let mut pause = String::new();
            let _ = std::io::stdin().read_line(&mut pause);
            Ok(Some(release.tag))
        }
        Err(Error::Rejected("update cancelled")) => Ok(Some(String::new())),
        Err(err) => Err(err),
    }
}

fn wait_enter() {
    use std::io::Write;
    eprint!("按 Enter 返回工作台…");
    let _ = std::io::stderr().flush();
    let mut pause = String::new();
    let _ = std::io::stdin().read_line(&mut pause);
}

fn apply_release_update(
    db: &Database,
    paths: &Paths,
    release: &ReleaseInfo,
    yes: bool,
    json_mode: bool,
) -> Result<serde_json::Value> {
    let exe = std::env::current_exe().map_err(storage)?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let writable = exe.parent().is_some_and(dir_is_writable);

    let prefer_pkg = if cfg!(target_os = "linux") && !writable {
        detect_linux_pkg_kind()
    } else {
        None
    };

    let asset = select_asset(
        release,
        std::env::consts::OS,
        std::env::consts::ARCH,
        prefer_pkg,
    )
    .ok_or(Error::NoData)?;
    let sums_asset = checksums_asset(release).ok_or(Error::Malformed("SHA256SUMS.txt missing"))?;

    if !yes {
        use std::io::Write;
        if !std::io::stdin().is_terminal() {
            return Err(Error::Usage("non-interactive update requires --yes".into()));
        }
        let action_desc = if writable {
            format!("下载并更新本地程序 {}", exe.display())
        } else {
            format!("下载安装包 {} 到缓存目录", asset.name)
        };
        eprint!(
            "发现新版本 {}（当前 {}），确认{action_desc}？[y/N]: ",
            release.tag, CURRENT_VERSION
        );
        std::io::stderr().flush().map_err(storage)?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).map_err(storage)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(Error::Rejected("update cancelled"));
        }
    }

    if !json_mode {
        eprintln!("正在下载 {} 并校验 SHA-256…", asset.name);
    }
    let bytes = runtime()?.block_on(campus_net::update::download_verified(asset, sums_asset))?;

    if writable {
        install_in_place(&exe, &bytes)?;
        let _ = db.set_pref("", "update_latest_tag", None);
        return Ok(json!({
            "current": CURRENT_VERSION,
            "latest": release.version,
            "tag": release.tag,
            "asset": asset.name,
            "path": exe,
            "update_available": true,
            "updated": true,
        }));
    }

    let stage_dir = paths.cache.join("updates");
    campus_db::paths::private_dir(&stage_dir)?;
    let dest = stage_dir.join(&asset.name);
    std::fs::write(&dest, &bytes).map_err(storage)?;

    let install_command = if asset.name.ends_with(".deb") {
        format!("sudo apt install {}", dest.display())
    } else if asset.name.ends_with(".rpm") {
        format!("sudo dnf install {}", dest.display())
    } else {
        let bin_out = stage_dir.join("campus");
        let extracted = campus_net::update::extract_tar_gz_binary(&bytes, "campus")?;
        std::fs::write(&bin_out, &extracted).map_err(storage)?;
        format!(
            "sudo install -m 755 {} {}",
            bin_out.display(),
            exe.display()
        )
    };
    if !json_mode {
        eprintln!(
            "当前程序目录无写权限（如系统包安装），请运行以下命令完成升级：\n  {install_command}"
        );
    }
    Ok(json!({
        "current": CURRENT_VERSION,
        "latest": release.version,
        "tag": release.tag,
        "asset": asset.name,
        "path": dest,
        "install_command": install_command,
        "update_available": true,
        "updated": false,
    }))
}

fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".campus-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn detect_linux_pkg_kind() -> Option<&'static str> {
    if Path::new("/etc/debian_version").exists() {
        Some("deb")
    } else if Path::new("/etc/redhat-release").exists() || Path::new("/etc/fedora-release").exists()
    {
        Some("rpm")
    } else {
        None
    }
}

fn install_in_place(exe: &Path, archive_bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let binary = campus_net::update::extract_tar_gz_binary(archive_bytes, "campus")?;
        let parent = exe
            .parent()
            .ok_or_else(|| storage("missing executable directory"))?;
        let tmp = parent.join(format!(".campus.update.{}", std::process::id()));
        let write_result = (|| -> std::io::Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(&binary)?;
            f.sync_all()?;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&tmp, exe)?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(storage(err));
        }
        Ok(())
    }

    #[cfg(windows)]
    {
        let parent = exe
            .parent()
            .ok_or_else(|| storage("missing executable directory"))?;
        let stage = parent.join(format!(".campus-update-{}", std::process::id()));
        std::fs::create_dir_all(&stage).map_err(storage)?;
        let zip_path = stage.join("release.zip");
        let extract_dir = stage.join("extracted");
        let res = (|| -> Result<()> {
            std::fs::write(&zip_path, archive_bytes).map_err(storage)?;
            std::fs::create_dir_all(&extract_dir).map_err(storage)?;
            let status = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Expand-Archive -LiteralPath $env:CAMPUS_ZIP -DestinationPath $env:CAMPUS_OUT -Force",
                ])
                .env("CAMPUS_ZIP", &zip_path)
                .env("CAMPUS_OUT", &extract_dir)
                .status()
                .map_err(storage)?;
            if !status.success() {
                return Err(Error::Storage(
                    "failed to extract Windows update zip".into(),
                ));
            }
            let new_exe = find_file_named(&extract_dir, "campus.exe")
                .ok_or(Error::Malformed("update zip missing campus.exe"))?;
            let old_exe = exe.with_extension("exe.old");
            let _ = std::fs::remove_file(&old_exe);
            std::fs::rename(exe, &old_exe).map_err(storage)?;
            if let Err(err) = std::fs::copy(&new_exe, exe) {
                let _ = std::fs::rename(&old_exe, exe);
                return Err(storage(err));
            }
            if let Some(new_ico) = find_file_named(&extract_dir, "campus.ico") {
                let _ = std::fs::copy(new_ico, parent.join("campus.ico"));
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&stage);
        res
    }
}

#[cfg(windows)]
fn find_file_named(dir: &Path, target: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file_named(&p, target) {
                return Some(found);
            }
        } else if p.file_name().and_then(|s| s.to_str()) == Some(target) {
            return Some(p);
        }
    }
    None
}
