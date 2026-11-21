//! Network work is explicit. The offline CLI never constructs a runtime or vault.
use campus_core::{Error, Result};
use campus_db::{
    Database,
    config::Config,
    paths::{Paths, SyncLock},
    vault::Vault,
};
use campus_net::{
    Endpoints, SchoolTransport,
    auth::SsoLogin,
    fetch::Fetcher,
    phyvlab::{PhyVlabClient, SessionOutcome},
};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub fn endpoints(config: &Config) -> Result<Endpoints> {
    let mut e = Endpoints::default();
    e.legacy_homework = config.get("homework.legacy_http").as_deref() != Some("off");
    e.legacy_courseware = config.get("courseware.legacy_http").as_deref() != Some("off");
    for key in [
        "mis",
        "sso",
        "aa",
        "calendar",
        "smart",
        "headcount",
        "phyvlab",
        "mail",
    ] {
        if let Some(v) = config.get(&format!("endpoints.{key}")) {
            e.configure(key, &v)?;
        }
    }
    Ok(e)
}
pub fn vault(paths: &Paths, config: &Config) -> Result<Vault> {
    let pass = std::env::var("CAMPUS_PORTAL_PASSPHRASE")
        .ok()
        .map(zeroize::Zeroizing::new);
    Vault::open(
        paths,
        &config
            .get("vault.backend")
            .unwrap_or_else(|| "keyring".into()),
        pass.as_deref().map(String::as_str),
    )
}
pub fn load_transport_jar(
    db: &Database,
    paths: &Paths,
    config: &Config,
    account: Option<i64>,
    jar: &str,
) -> Result<(SchoolTransport, Option<Vault>)> {
    let stored = account.map(|id| db.session(id, jar)).transpose()?.flatten();
    let (bytes, v) = if let Some((c, n)) = stored {
        let v = vault(paths, config)?;
        let b = v.open_secret(&format!("session:{}:{jar}", account.unwrap()), &c, &n)?;
        (Some(b), Some(v))
    } else {
        (None, None)
    };
    Ok((
        SchoolTransport::new(endpoints(config)?, bytes.as_deref().map(Vec::as_slice))?,
        v,
    ))
}
pub fn save_transport_jar(
    db: &Database,
    transport: &SchoolTransport,
    vault: &Vault,
    account: i64,
    jar: &str,
    now: i64,
) -> Result<()> {
    let b = transport.cookies()?;
    let name = format!("session:{account}:{jar}");
    if let Some((c, n)) = db.session(account, jar)?
        && vault.open_secret(&name, &c, &n)?.as_slice() == b.as_slice()
    {
        return Ok(());
    }
    let (c, n) = vault.seal(&name, &b)?;
    db.save_session(account, jar, &c, &n, now)
}
pub fn load_transport(
    db: &Database,
    paths: &Paths,
    config: &Config,
    account: Option<i64>,
) -> Result<(SchoolTransport, Option<Vault>)> {
    load_transport_jar(db, paths, config, account, "school")
}
pub fn save_transport(
    db: &Database,
    transport: &SchoolTransport,
    vault: &Vault,
    account: i64,
    now: i64,
) -> Result<()> {
    save_transport_jar(db, transport, vault, account, "school", now)
}
pub fn load_phyvlab_credentials(
    db: &Database,
    vault: &Vault,
    user: &str,
) -> Result<Option<(String, zeroize::Zeroizing<String>)>> {
    let name = format!("phyvlab:cred:{user}");
    let Some(raw) = vault.secret_string(db, &name)? else {
        return Ok(None);
    };
    let parsed: Value =
        serde_json::from_str(&raw).map_err(|_| Error::Malformed("saved phyvlab credentials"))?;
    let Some(u) = parsed["username"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Some(p) = parsed["password"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    Ok(Some((u.to_owned(), zeroize::Zeroizing::new(p.to_owned()))))
}
pub fn save_phyvlab_credentials(
    db: &Database,
    vault: &Vault,
    account: i64,
    user: &str,
    lab_user: &str,
    lab_pass: &str,
) -> Result<()> {
    let payload =
        zeroize::Zeroizing::new(json!({"username": lab_user, "password": lab_pass}).to_string());
    vault.save_secret_string(db, &format!("phyvlab:cred:{user}"), Some(&payload))?;
    db.set_pref(&format!("phyvlab:{account}"), "username", Some(lab_user))
}
pub fn clear_phyvlab_credentials(
    db: &Database,
    vault: &Vault,
    account: i64,
    user: &str,
) -> Result<()> {
    vault.save_secret_string(db, &format!("phyvlab:cred:{user}"), None)?;
    db.set_pref(&format!("phyvlab:{account}"), "username", None)
}
pub async fn sync(
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
    datasets: &[String],
    options: &BTreeMap<String, String>,
    now: i64,
) -> Result<Vec<Value>> {
    let _lock = SyncLock::acquire(paths)?;
    let private = datasets.iter().any(|s| s != "calendar");
    let account = if private {
        Some(db.account(user.ok_or(Error::SessionExpired)?)?)
    } else {
        None
    };
    let mut reports = Vec::new();
    let other_datasets: Vec<String> = datasets
        .iter()
        .filter(|s| s.as_str() != "phyvlab")
        .cloned()
        .collect();
    if !other_datasets.is_empty() {
        let (transport, mut secret) = load_transport(db, paths, config, account)?;
        let source = NetworkSource {
            fetcher: Fetcher {
                transport: &transport,
            },
            db,
            paths,
            config,
            user: user.unwrap_or(""),
            now,
        };
        let results = run_batch(&source, &other_datasets, options).await;
        if results.iter().any(|(d, r)| d == "courses" && r.is_ok())
            && let Ok(week) = (Fetcher {
                transport: &transport,
            })
            .current_week()
            .await
        {
            db.set_pref(
                &format!("courses:{}", account.unwrap_or(0)),
                "current_week",
                Some(&week.to_string()),
            )?;
        }
        for (d, result) in results {
            let id = if d == "calendar" {
                0
            } else {
                account.unwrap_or(0)
            };
            match result {
                Ok(rows) => {
                    let key = cache_key(&d, options);
                    let changes = db.save(id, &key, &rows, now)?;
                    reports.push(
                        json!({"dataset":d,"status":"success","count":rows.len(),"changes":changes}),
                    );
                }
                Err(e) => {
                    let (status, reason) = classify_sync_error(&e);
                    db.failure(id, &cache_key(&d, options), status, reason, now)?;
                    reports.push(json!({"dataset":d,"status":status,"reason":reason}));
                }
            }
        }
        if let Some(id) = account {
            if secret.is_none() {
                secret = Some(vault(paths, config)?);
            }
            save_transport(db, &transport, secret.as_ref().unwrap(), id, now)?;
        }
    }
    if datasets.iter().any(|s| s == "phyvlab") {
        let id = account.ok_or(Error::SessionExpired)?;
        let u = user.ok_or(Error::SessionExpired)?;
        reports.push(sync_phyvlab(db, paths, config, u, id, now).await?);
    }
    reports.sort_by_key(|v| v["dataset"].as_str().unwrap_or("").to_owned());
    Ok(reports)
}

fn classify_sync_error(e: &Error) -> (&'static str, &'static str) {
    match e {
        Error::SessionExpired => ("needs_user_action", "session_expired"),
        Error::ActionRequired(reason) => ("needs_user_action", *reason),
        Error::Rejected(_) => ("needs_user_action", "login_required"),
        Error::SecureChannel => ("failed", "secure_channel"),
        Error::SmartLegacyHttp => ("failed", "smart_legacy_http"),
        Error::Malformed(_) => ("failed", "malformed_response"),
        _ => ("failed", "network"),
    }
}

async fn sync_phyvlab(
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    account: i64,
    now: i64,
) -> Result<Value> {
    let (transport, mut secret) = load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let creds = match secret.as_ref() {
        Some(v) => load_phyvlab_credentials(db, v, user).ok().flatten(),
        None => vault(paths, config).ok().and_then(|v| {
            let res = load_phyvlab_credentials(db, &v, user).ok().flatten();
            secret = Some(v);
            res
        }),
    };
    let client = PhyVlabClient {
        transport: &transport,
    };
    let creds_ref = creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
    let outcome = match client.establish_session(creds_ref).await {
        Ok(outcome) => outcome,
        Err(e) => {
            if std::env::var_os("CAMPUS_PHYVLAB_DEBUG").is_some() {
                eprintln!("[phyvlab-debug] establish_session err={e:?}");
            }
            let (status, reason) = classify_sync_error(&e);
            db.failure(account, "phyvlab", status, reason, now)?;
            return Ok(json!({"dataset":"phyvlab","status":status,"reason":reason}));
        }
    };
    match outcome {
        SessionOutcome::Ready => {
            let res = client.fetch_all(true).await;
            if let Some(v) = secret.as_ref() {
                let _ = save_transport_jar(db, &transport, v, account, "phyvlab", now);
            } else if let Ok(v) = vault(paths, config) {
                let _ = save_transport_jar(db, &transport, &v, account, "phyvlab", now);
            }
            match res {
                Ok((courses, activities)) => {
                    let course_rows: Vec<Value> = courses
                        .into_iter()
                        .map(|c| {
                            serde_json::to_value(c)
                                .map_err(|_| Error::Malformed("model serialization"))
                        })
                        .collect::<Result<_>>()?;
                    let activity_rows: Vec<Value> = activities
                        .into_iter()
                        .map(|a| {
                            serde_json::to_value(a)
                                .map_err(|_| Error::Malformed("model serialization"))
                        })
                        .collect::<Result<_>>()?;
                    db.save(account, "phyvlab_courses", &course_rows, now)?;
                    let changes = db.save(account, "phyvlab", &activity_rows, now)?;
                    Ok(
                        json!({"dataset":"phyvlab","status":"success","count":activity_rows.len(),"changes":changes}),
                    )
                }
                Err(e) => {
                    if std::env::var_os("CAMPUS_PHYVLAB_DEBUG").is_some() {
                        eprintln!("[phyvlab-debug] fetch_all err={e:?}");
                    }
                    let (status, reason) = classify_sync_error(&e);
                    db.failure(account, "phyvlab", status, reason, now)?;
                    Ok(json!({"dataset":"phyvlab","status":status,"reason":reason}))
                }
            }
        }
        SessionOutcome::AccountRequired => {
            db.failure(
                account,
                "phyvlab",
                "needs_user_action",
                "account_required",
                now,
            )?;
            Ok(
                json!({"dataset":"phyvlab","status":"needs_user_action","reason":"account_required"}),
            )
        }
        SessionOutcome::CaptchaRequired { .. } | SessionOutcome::CaptchaRejected => {
            db.failure(
                account,
                "phyvlab",
                "needs_user_action",
                "captcha_required",
                now,
            )?;
            Ok(
                json!({"dataset":"phyvlab","status":"needs_user_action","reason":"captcha_required"}),
            )
        }
        SessionOutcome::CredentialsRejected => {
            db.failure(
                account,
                "phyvlab",
                "needs_user_action",
                "credentials_rejected",
                now,
            )?;
            Ok(
                json!({"dataset":"phyvlab","status":"needs_user_action","reason":"credentials_rejected"}),
            )
        }
    }
}

pub fn cache_key(dataset: &str, options: &BTreeMap<String, String>) -> String {
    if dataset == "rooms" {
        format!(
            "rooms:{}:{}:{}",
            options.get("term").map(String::as_str).unwrap_or("current"),
            options.get("building").map(String::as_str).unwrap_or("1"),
            options.get("week").map(String::as_str).unwrap_or("1")
        )
    } else {
        dataset.into()
    }
}

trait SyncSource {
    async fn fetch(&self, dataset: &str, options: &BTreeMap<String, String>) -> Result<Vec<Value>>;
    async fn recover(&self) -> Result<()>;
}
struct NetworkSource<'a> {
    fetcher: Fetcher<'a>,
    db: &'a Database,
    paths: &'a Paths,
    config: &'a Config,
    user: &'a str,
    now: i64,
}
impl SyncSource for NetworkSource<'_> {
    async fn fetch(&self, dataset: &str, options: &BTreeMap<String, String>) -> Result<Vec<Value>> {
        self.fetcher.fetch(dataset, options).await
    }
    async fn recover(&self) -> Result<()> {
        let v = vault(self.paths, self.config)?;
        let password = v
            .password(self.db, self.user)?
            .ok_or(Error::SessionExpired)?;
        let login = SsoLogin {
            transport: self.fetcher.transport,
        };
        let profile = login.automatic(self.user, &password).await?;
        self.db.save_account(&profile, self.now)?;
        login.link_academic().await
    }
}
async fn run_batch(
    source: &impl SyncSource,
    datasets: &[String],
    options: &BTreeMap<String, String>,
) -> Vec<(String, Result<Vec<Value>>)> {
    let first = stream::iter(datasets.iter().cloned().map(|d| async move {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            source.fetch(&d, options),
        )
        .await
        .unwrap_or(Err(Error::Network));
        (d, result)
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    let expired = first
        .iter()
        .any(|(_, r)| matches!(r, Err(Error::SessionExpired)));
    let recovered = expired && source.recover().await.is_ok();
    stream::iter(first.into_iter().map(|(d, result)| async move {
        let result = if recovered && matches!(result, Err(Error::SessionExpired)) {
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                source.fetch(&d, options),
            )
            .await
            .unwrap_or(Err(Error::Network))
        } else {
            result
        };
        (d, result)
    }))
    .buffer_unordered(8)
    .collect()
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Fake {
        logins: Cell<usize>,
        fetches: Cell<usize>,
        accept_login: bool,
    }
    impl SyncSource for Fake {
        async fn fetch(&self, _: &str, _: &BTreeMap<String, String>) -> Result<Vec<Value>> {
            self.fetches.set(self.fetches.get() + 1);
            tokio::task::yield_now().await;
            if self.logins.get() > 0 && self.accept_login {
                Ok(Vec::new())
            } else {
                Err(Error::SessionExpired)
            }
        }
        async fn recover(&self) -> Result<()> {
            self.logins.set(self.logins.get() + 1);
            if self.accept_login {
                Ok(())
            } else {
                Err(Error::Rejected("captcha required"))
            }
        }
    }
    #[tokio::test]
    async fn concurrent_expiry_causes_one_relogin() {
        let source = Fake {
            logins: Cell::new(0),
            fetches: Cell::new(0),
            accept_login: true,
        };
        let results = run_batch(
            &source,
            &["courses".into(), "grades".into(), "exams".into()],
            &BTreeMap::new(),
        )
        .await;
        assert_eq!(source.logins.get(), 1);
        assert_eq!(source.fetches.get(), 6);
        assert!(results.iter().all(|(_, r)| r.is_ok()));
    }
    #[tokio::test]
    async fn rejected_login_is_not_replayed() {
        let source = Fake {
            logins: Cell::new(0),
            fetches: Cell::new(0),
            accept_login: false,
        };
        let results = run_batch(
            &source,
            &["courses".into(), "grades".into()],
            &BTreeMap::new(),
        )
        .await;
        assert_eq!(source.logins.get(), 1);
        assert_eq!(source.fetches.get(), 2);
        assert!(results.iter().all(|(_, r)| r.is_err()));
    }
    #[test]
    fn rooms_have_distinct_cache_keys() {
        let mut a = BTreeMap::new();
        a.insert("building".into(), "1".into());
        let mut b = a.clone();
        b.insert("week".into(), "2".into());
        assert_ne!(cache_key("rooms", &a), cache_key("rooms", &b));
    }

    #[test]
    fn phyvlab_credentials_session_jar_and_error_classification_are_isolated() {
        assert_eq!(
            classify_sync_error(&Error::ActionRequired("captcha_required")),
            ("needs_user_action", "captcha_required")
        );
        assert_eq!(
            classify_sync_error(&Error::ActionRequired("account_required")),
            ("needs_user_action", "account_required")
        );

        let root = std::env::temp_dir().join(format!(
            "campus-sync-phyvlab-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = Paths {
            data: root.join("data"),
            config: root.join("config"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        };
        let mut config = Config::default();
        config.values.insert("vault.backend".into(), "file".into());
        let db = Database::open(&paths.database()).unwrap();
        let account = db
            .save_account(
                &campus_core::Profile {
                    student_id: "u1".into(),
                    name: "测试".into(),
                    identity: "学生".into(),
                    department: "学院".into(),
                },
                1,
            )
            .unwrap();
        let v = vault(&paths, &config).unwrap();
        save_phyvlab_credentials(&db, &v, account, "u1", "lab-u1", "lab-p1").unwrap();
        let creds = load_phyvlab_credentials(&db, &v, "u1").unwrap().unwrap();
        assert_eq!(creds.0, "lab-u1");
        assert_eq!(creds.1.as_str(), "lab-p1");
        assert_eq!(
            db.pref(&format!("phyvlab:{account}"), "username")
                .unwrap()
                .as_deref(),
            Some("lab-u1")
        );

        let school_transport = SchoolTransport::new(Endpoints::default(), None).unwrap();
        let phy_transport = SchoolTransport::new(Endpoints::default(), None).unwrap();
        save_transport(&db, &school_transport, &v, account, 10).unwrap();
        save_transport_jar(&db, &phy_transport, &v, account, "phyvlab", 10).unwrap();
        assert!(db.session(account, "school").unwrap().is_some());
        assert!(db.session(account, "phyvlab").unwrap().is_some());

        db.clear_session_jar(account, "phyvlab").unwrap();
        assert!(db.session(account, "school").unwrap().is_some());
        assert!(db.session(account, "phyvlab").unwrap().is_none());

        clear_phyvlab_credentials(&db, &v, account, "u1").unwrap();
        assert!(load_phyvlab_credentials(&db, &v, "u1").unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_http_defaults_on_and_respects_explicit_off() {
        let mut config = Config::default();
        let default_ep = endpoints(&config).unwrap();
        assert!(default_ep.legacy_homework);
        assert!(default_ep.legacy_courseware);

        config
            .values
            .insert("homework.legacy_http".into(), "off".into());
        config
            .values
            .insert("courseware.legacy_http".into(), "off".into());
        let off_ep = endpoints(&config).unwrap();
        assert!(!off_ep.legacy_homework);
        assert!(!off_ep.legacy_courseware);

        config
            .values
            .insert("homework.legacy_http".into(), "on".into());
        let mixed_ep = endpoints(&config).unwrap();
        assert!(mixed_ep.legacy_homework);
        assert!(!mixed_ep.legacy_courseware);
    }
}
