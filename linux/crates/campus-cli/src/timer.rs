use campus_core::{Error, Result};
use campus_db::paths::Paths;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use campus_db::storage;
use serde_json::Value;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use serde_json::json;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::Path;

pub fn manage(_paths: &Paths, action: &str, every: &str) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        manage_linux(action, every)
    }
    #[cfg(target_os = "macos")]
    {
        manage_macos(action, every)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (action, every);
        Err(Error::Usage(
            "campus timer requires Linux systemd or macOS launchd; use campus sync on Windows"
                .into(),
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_interval_minutes(every: &str) -> Result<u64> {
    if let Some(v) = every.strip_suffix('h') {
        v.parse::<u64>().ok().and_then(|v| v.checked_mul(60))
    } else if let Some(v) = every.strip_suffix('m') {
        v.parse().ok()
    } else {
        None
    }
    .filter(|n| *n >= 30 && *n <= 10080)
    .ok_or_else(|| Error::Usage("--every must be 30m..168h".into()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_private_atomic(
    dir: &Path,
    name: &str,
    ownership_marker: &str,
    contents: &str,
) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let path = dir.join(name);
    if let Ok(m) = std::fs::symlink_metadata(&path)
        && (m.file_type().is_symlink()
            || !std::fs::read_to_string(&path)
                .map_err(storage)?
                .contains(ownership_marker))
    {
        return Err(storage("existing timer file is not owned by campus"));
    }
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(storage)?;
    file.write_all(contents.as_bytes()).map_err(storage)?;
    file.sync_all().map_err(storage)?;
    std::fs::rename(tmp, path).map_err(storage)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn manage_linux(action: &str, every: &str) -> Result<Value> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|s| std::path::PathBuf::from(s).join(".config")))
        .ok_or_else(|| storage("HOME missing"))?;
    let dir = home.join("systemd/user");
    if action == "remove" {
        systemctl(&["disable", "--now", "campus-sync.timer"])?;
        for name in ["campus-sync.service", "campus-sync.timer"] {
            let path = dir.join(name);
            if path.exists() {
                std::fs::remove_file(path).map_err(storage)?;
            }
        }
        systemctl(&["daemon-reload"])?;
        return Ok(json!({"removed":true}));
    }
    let mins = parse_interval_minutes(every)?;
    let exe = std::env::current_exe()
        .map_err(storage)?
        .to_string_lossy()
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::create_dir_all(&dir).map_err(storage)?;
    let (service, timer) = units(&exe, mins);
    for (name, contents) in [
        ("campus-sync.service", &service),
        ("campus-sync.timer", &timer),
    ] {
        write_private_atomic(&dir, name, "Description=BJTU campus", contents)?;
    }
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", "campus-sync.timer"])?;
    Ok(json!({"installed":true,"every_minutes":mins}))
}

#[cfg(target_os = "linux")]
pub fn units(exe: &str, mins: u64) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=BJTU campus cache refresh\n[Service]\nType=oneshot\nExecStart=\"{exe}\" sync all --background\nUMask=0077\nNoNewPrivileges=true\n"
    );
    let timer = format!(
        "[Unit]\nDescription=BJTU campus periodic refresh\n[Timer]\nOnBootSec=5m\nOnUnitActiveSec={mins}m\nRandomizedDelaySec=60\n[Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

#[cfg(target_os = "linux")]
fn systemctl(args: &[&str]) -> Result<()> {
    let ok = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .map_err(storage)?
        .success();
    if ok {
        Ok(())
    } else {
        Err(storage("systemctl --user failed"))
    }
}

#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "team.bjtuss.campus.sync";
#[cfg(target_os = "macos")]
const LAUNCHD_PLIST_NAME: &str = "team.bjtuss.campus.sync.plist";
#[cfg(target_os = "macos")]
const LAUNCHD_MARKER: &str = "<!-- BJTU campus periodic refresh -->";

#[cfg(target_os = "macos")]
fn manage_macos(action: &str, every: &str) -> Result<Value> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| storage("HOME missing"))?;
    let dir = home.join("Library/LaunchAgents");
    let plist_path = dir.join(LAUNCHD_PLIST_NAME);
    let domain = format!("gui/{}", campus_db::paths::current_uid());
    if action == "remove" {
        if let Ok(m) = std::fs::symlink_metadata(&plist_path)
            && (m.file_type().is_symlink()
                || !std::fs::read_to_string(&plist_path)
                    .map_err(storage)?
                    .contains(LAUNCHD_MARKER))
        {
            return Err(storage("existing launchd plist is not owned by campus"));
        }
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &domain])
            .arg(&plist_path)
            .status();
        if plist_path.exists() {
            std::fs::remove_file(&plist_path).map_err(storage)?;
        }
        return Ok(json!({"removed": true}));
    }
    let mins = parse_interval_minutes(every)?;
    let exe = std::env::current_exe()
        .map_err(storage)?
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(&dir).map_err(storage)?;
    let plist = launchd_plist(&exe, mins);
    write_private_atomic(&dir, LAUNCHD_PLIST_NAME, LAUNCHD_MARKER, &plist)?;
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &domain])
        .arg(&plist_path)
        .status();
    let ok = std::process::Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&plist_path)
        .status()
        .map_err(storage)?
        .success();
    if !ok {
        return Err(storage("launchctl bootstrap failed"));
    }
    Ok(json!({"installed": true, "every_minutes": mins, "backend": "launchd"}))
}

#[cfg(target_os = "macos")]
pub fn launchd_plist(exe: &str, mins: u64) -> String {
    let escaped = exe
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let seconds = mins * 60;
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         {LAUNCHD_MARKER}\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{LAUNCHD_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{escaped}</string>\n\
         \t\t<string>sync</string>\n\
         \t\t<string>all</string>\n\
         \t\t<string>--background</string>\n\
         \t</array>\n\
         \t<key>StartInterval</key>\n\
         \t<integer>{seconds}</integer>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         \t<key>StandardOutPath</key>\n\
         \t<string>/dev/null</string>\n\
         \t<key>StandardErrorPath</key>\n\
         \t<string>/dev/null</string>\n\
         </dict>\n\
         </plist>\n"
    )
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn user_units_validate_without_installing() {
        let temp = tempfile::tempdir().unwrap();
        let (service, timer) = super::units("/usr/bin/true", 60);
        std::fs::write(temp.path().join("campus-sync.service"), service).unwrap();
        std::fs::write(temp.path().join("campus-sync.timer"), timer).unwrap();
        if std::process::Command::new("systemd-analyze")
            .arg("--version")
            .output()
            .is_ok()
        {
            let result = std::process::Command::new("systemd-analyze")
                .arg("verify")
                .arg(temp.path().join("campus-sync.service"))
                .arg(temp.path().join("campus-sync.timer"))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_validates_without_installing() {
        let temp = tempfile::tempdir().unwrap();
        let plist = super::launchd_plist("/Applications/campus & tools/campus", 60);
        let path = temp.path().join(super::LAUNCHD_PLIST_NAME);
        std::fs::write(&path, plist).unwrap();
        let result = std::process::Command::new("plutil")
            .args(["-lint", "-s"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
