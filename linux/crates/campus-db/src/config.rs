use crate::{paths, storage};
use campus_core::{Error, Result};
use std::{collections::BTreeMap, io::Write, path::Path};
#[derive(Default)]
pub struct Config {
    pub values: BTreeMap<String, String>,
}
const KEYS: &[&str] = &[
    "auto_refresh",
    "vault.backend",
    "rooms.live",
    "homework.legacy_http",
    "courseware.legacy_http",
    "endpoints.mis",
    "endpoints.sso",
    "endpoints.aa",
    "endpoints.calendar",
    "endpoints.smart",
    "endpoints.headcount",
    "endpoints.phyvlab",
    "grades.fresh_seconds",
    "update.check",
];
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path).map_err(storage)?;
        let mut values = BTreeMap::new();
        for line in data
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
        {
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| Error::Usage("config must contain key = value".into()))?;
            Self::validate(k.trim(), v.trim())?;
            values.insert(k.trim().into(), v.trim().into());
        }
        Ok(Self { values })
    }
    pub fn get(&self, key: &str) -> Option<String> {
        let env = format!("CAMPUS_PORTAL_{}", key.replace('.', "_").to_uppercase());
        std::env::var(env)
            .ok()
            .or_else(|| self.values.get(key).cloned())
    }
    pub fn validate(k: &str, v: &str) -> Result<()> {
        if !KEYS.contains(&k) || v.contains(['\n', '\r', '\0']) {
            return Err(Error::Usage("unknown or unsafe config setting".into()));
        }
        match k {
            "vault.backend" if !["keyring", "file", "passphrase"].contains(&v) => Err(
                Error::Usage("vault.backend: keyring|file|passphrase".into()),
            ),
            "auto_refresh" if !["off", "background", "foreground"].contains(&v) => Err(
                Error::Usage("auto_refresh: off|background|foreground".into()),
            ),
            "rooms.live" | "homework.legacy_http" | "courseware.legacy_http" | "update.check"
                if !["on", "off"].contains(&v) =>
            {
                Err(Error::Usage(format!("{k}: on|off")))
            }
            _ => Ok(()),
        }
    }
    pub fn set(&mut self, path: &Path, key: &str, value: Option<&str>) -> Result<()> {
        if let Some(v) = value {
            Self::validate(key, v)?;
            self.values.insert(key.into(), v.into());
        } else {
            self.values.remove(key);
        }
        paths::private_dir(path.parent().ok_or_else(|| storage("config parent"))?)?;
        let tmp = path.with_extension("tmp");
        let mut f = paths::private_file(&tmp)?;
        f.set_len(0).map_err(storage)?;
        for (k, v) in &self.values {
            writeln!(f, "{k} = {v}").map_err(storage)?;
        }
        f.sync_all().map_err(storage)?;
        std::fs::rename(tmp, path).map_err(storage)
    }
}
