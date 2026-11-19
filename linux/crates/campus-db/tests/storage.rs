use campus_db::{
    Database,
    paths::{Paths, SyncLock},
    vault::Vault,
};
use serde_json::json;
fn account(db: &Database) -> i64 {
    db.save_account(
        &campus_core::Profile {
            student_id: "fixture-user".into(),
            name: "测试".into(),
            identity: "学生".into(),
            department: "测试学院".into(),
        },
        1,
    )
    .unwrap()
}
#[test]
fn migrations_are_idempotent_and_refresh_rolls_back() {
    let mut db = Database::memory().unwrap();
    db.migrate().unwrap();
    let id = account(&db);
    let grade = json!({"course_name":"测试课程","course_code":null,"teacher":"测试教师","term":"2026-1","score_raw":"90","score_letter":"A","score_value":90,"credits_raw":"2","credits":2,"detail":""});
    db.save(id, "grades", std::slice::from_ref(&grade), 10)
        .unwrap();
    db.set_pref("grades:1", "selection", Some("stable-key"))
        .unwrap();
    let mut changed = grade.clone();
    changed["score_value"] = 95.into();
    assert!(
        db.save(id, "grades", &[changed, json!({"invalid":true})], 20)
            .is_err()
    );
    assert_eq!(db.rows(id, "grades").unwrap(), vec![grade]);
    assert_eq!(db.fetched_at(id, "grades").unwrap(), Some(10));
    assert_eq!(db.changes(id, false).unwrap().len(), 1);
    assert_eq!(
        db.pref("grades:1", "selection").unwrap().as_deref(),
        Some("stable-key")
    );
    db.failure(id, "grades", "failed", "network", 30).unwrap();
    assert_eq!(db.fetched_at(id, "grades").unwrap(), Some(10));
    assert_eq!(db.rows(id, "grades").unwrap().len(), 1);
}
#[test]
fn credentials_and_cookies_encrypted_and_bound_to_account() {
    let temp = tempfile::tempdir().unwrap();
    let paths = Paths {
        data: temp.path().join("data"),
        config: temp.path().join("config"),
        cache: temp.path().join("cache"),
        state: temp.path().join("state"),
        runtime: temp.path().join("runtime"),
    };
    let db = Database::open(&paths.database()).unwrap();
    let id = account(&db);
    let vault = Vault::open(&paths, "file", None).unwrap();
    let plain = b"fixture-only-session-cookie";
    let context = format!("session:{id}:school");
    let (c, n) = vault.seal(&context, plain).unwrap();
    db.save_session(id, "school", &c, &n, 2).unwrap();
    assert!(!c.windows(plain.len()).any(|w| w == plain));
    assert_eq!(
        vault.open_secret(&context, &c, &n).unwrap().as_slice(),
        plain
    );
    assert!(vault.open_secret("session:2:school", &c, &n).is_err());
    vault
        .save_password(&db, "fixture-user", Some("fixture-only-password"))
        .unwrap();
    assert_eq!(
        vault
            .password(&db, "fixture-user")
            .unwrap()
            .unwrap()
            .as_str(),
        "fixture-only-password"
    );
    vault.save_password(&db, "fixture-user", None).unwrap();
    assert!(vault.password(&db, "fixture-user").unwrap().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(paths.database())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&paths.data).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let lock = SyncLock::acquire(&paths).unwrap();
    assert!(matches!(
        SyncLock::acquire(&paths),
        Err(campus_core::Error::Busy)
    ));
    drop(lock);
    assert!(SyncLock::acquire(&paths).is_ok());
}
#[test]
#[cfg(unix)]
fn symlink_database_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("unrelated");
    std::fs::write(&target, b"keep me").unwrap();
    let private = t.path().join("private");
    campus_db::paths::private_dir(&private).unwrap();
    let link = private.join("campus.db");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(Database::open(&link).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"keep me");
}
#[test]
fn passphrase_vault_authenticates() {
    let t = tempfile::tempdir().unwrap();
    let p = Paths {
        data: t.path().join("d"),
        config: t.path().join("c"),
        cache: t.path().join("x"),
        state: t.path().join("s"),
        runtime: t.path().join("r"),
    };
    let a = Vault::open(&p, "passphrase", Some("test phrase")).unwrap();
    let (c, n) = a.seal("test", b"fixture").unwrap();
    let b = Vault::open(&p, "passphrase", Some("wrong phrase")).unwrap();
    assert!(b.open_secret("test", &c, &n).is_err());
}
#[test]
fn public_calendar_needs_no_private_account() {
    let mut db = Database::memory().unwrap();
    let rows = [json!({"term":"2026-2027-1","week":1,"monday":"2026-08-31"})];
    db.save(0, "calendar", &rows, 100).unwrap();
    assert_eq!(db.rows(0, "calendar").unwrap(), rows);
    assert_eq!(db.fetched_at(0, "calendar").unwrap(), Some(100));
    assert!(db.accounts().unwrap().is_empty());
}
#[test]
fn newer_schema_is_rejected_without_rebuilding() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("data/campus.db");
    let db = Database::open(&p).unwrap();
    account(&db);
    drop(db);
    let c = rusqlite::Connection::open(&p).unwrap();
    c.pragma_update(None, "user_version", 999).unwrap();
    drop(c);
    assert!(Database::open(&p).is_err());
    let c = rusqlite::Connection::open(p).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM account", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[cfg(feature = "sqlcipher")]
#[test]
fn sqlcipher_database_requires_the_key() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("private/encrypted.db");
    let key = [42; 32];
    let db = Database::open_encrypted(&path, &key).unwrap();
    account(&db);
    drop(db);
    assert_ne!(&std::fs::read(&path).unwrap()[..16], b"SQLite format 3\0");
    assert!(Database::open(&path).is_err());
    let db = Database::open_encrypted(&path, &key).unwrap();
    assert_eq!(db.accounts().unwrap().len(), 1);
}
