//! SQLite is only opened by commands needing state. Keyring access is separate.
pub mod config;
pub mod paths;
pub mod vault;
use campus_core::{Error, Result, detect_changes, identity};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
pub fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}
pub struct Database {
    conn: Connection,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_inner(path, None)
    }
    #[cfg(feature = "sqlcipher")]
    pub fn open_encrypted(path: &Path, key: &[u8; 32]) -> Result<Self> {
        Self::open_inner(path, Some(key))
    }
    fn open_inner(path: &Path, key: Option<&[u8; 32]>) -> Result<Self> {
        paths::private_dir(
            path.parent()
                .ok_or_else(|| storage("missing database parent"))?,
        )?;
        paths::private_file(path)?;
        let conn = Connection::open(path).map_err(storage)?;
        if let Some(key) = key {
            #[cfg(feature = "sqlcipher")]
            {
                let encoded = zeroize::Zeroizing::new(format!("x'{}'", hex::encode(key)));
                conn.pragma_update(None, "key", encoded.as_str())
                    .map_err(storage)?;
            }
            #[cfg(not(feature = "sqlcipher"))]
            {
                let _ = key;
                return Err(storage("SQLCipher was not compiled in"));
            }
        }
        conn.busy_timeout(Duration::from_millis(3000))
            .map_err(storage)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")
            .map_err(storage)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        if version == 0 {
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(storage)?;
        }
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }
    pub fn memory() -> Result<Self> {
        let mut db = Self {
            conn: Connection::open_in_memory().map_err(storage)?,
        };
        db.conn
            .execute_batch("PRAGMA foreign_keys=ON")
            .map_err(storage)?;
        db.migrate()?;
        Ok(db)
    }
    pub fn migrate(&mut self) -> Result<()> {
        let tx = self.conn.transaction().map_err(storage)?;
        let version: u32 = tx
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        if version > 1 {
            return Err(storage("database was created by a newer campus version"));
        }
        if version == 0 {
            tx.execute_batch(include_str!("../migrations/0001.sql"))
                .map_err(storage)?;
            tx.pragma_update(None, "user_version", 1).map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }
    pub fn account(&self, id: &str) -> Result<i64> {
        self.conn
            .query_row("SELECT id FROM account WHERE student_id=?", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage)?
            .ok_or(Error::SessionExpired)
    }
    pub fn accounts(&self) -> Result<Vec<Value>> {
        let mut q = self
            .conn
            .prepare_cached(
                "SELECT student_id,name,identity,department,last_login_at FROM account ORDER BY id",
            )
            .map_err(storage)?;
        q.query_map([],|r|Ok(json!({"student_id":r.get::<_,String>(0)?,"name":r.get::<_,Option<String>>(1)?,"identity":r.get::<_,Option<String>>(2)?,"department":r.get::<_,Option<String>>(3)?,"last_login_at":r.get::<_,Option<i64>>(4)?}))).map_err(storage)?.collect::<std::result::Result<_,_>>().map_err(storage)
    }
    pub fn save_account(&self, p: &campus_core::Profile, now: i64) -> Result<i64> {
        self.conn.execute("INSERT INTO account(student_id,name,identity,department,created_at,last_login_at) VALUES (?,?,?,?,?,?) ON CONFLICT(student_id) DO UPDATE SET name=excluded.name,identity=excluded.identity,department=excluded.department,last_login_at=excluded.last_login_at",params![p.student_id,p.name,p.identity,p.department,now,now]).map_err(storage)?;
        self.account(&p.student_id)
    }
    pub fn rows(&self, account: i64, dataset: &str) -> Result<Vec<Value>> {
        let mut q = self
            .conn
            .prepare_cached(
                "SELECT payload FROM dataset_row WHERE account_id=? AND dataset=? ORDER BY ordinal",
            )
            .map_err(storage)?;
        let raw: Vec<String> = q
            .query_map(params![account, dataset], |r| r.get(0))
            .map_err(storage)?
            .collect::<std::result::Result<_, _>>()
            .map_err(storage)?;
        raw.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(storage))
            .collect()
    }
    pub fn grades(&self, account: i64) -> Result<Vec<campus_core::Grade>> {
        let mut q=self.conn.prepare_cached("SELECT course_name,course_code,teacher,term,score_raw,score_letter,score_value,credits_raw,credits,detail,ordinal FROM grade WHERE account_id=? ORDER BY ordinal").map_err(storage)?;
        q.query_map([account], |r| {
            let ordinal = r.get::<_, i64>(10)?;
            let index = usize::try_from(ordinal)
                .ok()
                .and_then(|n| n.checked_add(1))
                .ok_or(rusqlite::Error::IntegralValueOutOfRange(10, ordinal))?;
            Ok(campus_core::Grade {
                index,
                course_name: r.get(0)?,
                course_code: r.get(1)?,
                teacher: r.get(2)?,
                term: r.get(3)?,
                score_raw: r.get(4)?,
                score_letter: r.get(5)?,
                score_value: r.get(6)?,
                credits_raw: r.get(7)?,
                credits: r.get(8)?,
                detail: r.get(9)?,
            })
        })
        .map_err(storage)?
        .collect::<std::result::Result<_, _>>()
        .map_err(storage)
    }
    pub fn save(&mut self, account: i64, dataset: &str, rows: &[Value], now: i64) -> Result<usize> {
        let old = self.rows(account, dataset)?;
        let changes = detect_changes(&old, rows, |v| identity(dataset, v));
        let tx = self.conn.transaction().map_err(storage)?;
        if old != rows {
            tx.execute(
                "DELETE FROM dataset_row WHERE account_id=? AND dataset=?",
                params![account, dataset],
            )
            .map_err(storage)?;
            {
                let mut q = tx
                    .prepare_cached("INSERT INTO dataset_row VALUES (?,?,?,?)")
                    .map_err(storage)?;
                for (i, v) in rows.iter().enumerate() {
                    q.execute(params![account, dataset, i as i64, v.to_string()])
                        .map_err(storage)?;
                }
            }
            for c in changes.iter().filter(|_| account > 0) {
                tx.execute("INSERT INTO change_event(account_id,dataset,kind,item_key,before_json,after_json,detected_at) VALUES (?,?,?,?,?,?,?)",params![account,dataset,c.kind,c.item_key,c.before.as_ref().map(Value::to_string),c.after.as_ref().map(Value::to_string),now]).map_err(storage)?;
            }
            // Indexed projections are updated in the same transaction as the canonical rows.
            match dataset {
                "courses" => {
                    tx.execute("DELETE FROM course WHERE account_id=?", [account])
                        .map_err(storage)?;
                    for (i, v) in rows.iter().enumerate() {
                        let c: campus_core::Course =
                            serde_json::from_value(v.clone()).map_err(storage)?;
                        tx.execute("INSERT INTO course(account_id,ordinal,course_code,name,teacher,location_index,weeks_raw,weeks_mask,place,is_selection) VALUES(?,?,?,?,?,?,?,?,?,?)",params![account,i as i64,c.course_code,c.name,c.teacher,c.location_index,c.weeks_raw,c.weeks_mask,c.place,c.is_selection]).map_err(storage)?;
                    }
                }
                "grades" => {
                    tx.execute("DELETE FROM grade WHERE account_id=?", [account])
                        .map_err(storage)?;
                    for (i, v) in rows.iter().enumerate() {
                        let g: campus_core::Grade =
                            serde_json::from_value(v.clone()).map_err(storage)?;
                        tx.execute("INSERT INTO grade(account_id,ordinal,course_name,course_code,teacher,term,score_raw,score_letter,score_value,credits_raw,credits,detail) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",params![account,i as i64,g.course_name,g.course_code,g.teacher,g.term,g.score_raw,g.score_letter,g.score_value,g.credits_raw,g.credits,g.detail]).map_err(storage)?;
                    }
                }
                "exams" => {
                    tx.execute("DELETE FROM exam WHERE account_id=?", [account])
                        .map_err(storage)?;
                    for (i, v) in rows.iter().enumerate() {
                        let e: campus_core::Exam =
                            serde_json::from_value(v.clone()).map_err(storage)?;
                        tx.execute(
                            "INSERT INTO exam VALUES(?,?,?,?,?,?,?,?,?,?)",
                            params![
                                account,
                                i as i64,
                                e.exam_type,
                                e.course_name,
                                e.time_place_raw,
                                e.status,
                                e.detail,
                                e.starts_at,
                                e.ends_at,
                                e.location
                            ],
                        )
                        .map_err(storage)?;
                    }
                }
                "grade_types" => {
                    tx.execute(
                        "DELETE FROM program_course_type WHERE account_id=?",
                        [account],
                    )
                    .map_err(storage)?;
                    for v in rows {
                        let code = v["course_code"]
                            .as_str()
                            .ok_or(Error::Malformed("course code"))?;
                        let kind = v["course_type"]
                            .as_str()
                            .ok_or(Error::Malformed("course type"))?;
                        tx.execute(
                            "INSERT INTO program_course_type VALUES(?,?,?)",
                            params![account, code, kind],
                        )
                        .map_err(storage)?;
                    }
                }
                "calendar" => {
                    for v in rows {
                        let w: campus_core::AcademicWeek =
                            serde_json::from_value(v.clone()).map_err(storage)?;
                        tx.execute("INSERT INTO academic_week VALUES(?,?,?) ON CONFLICT(term,week) DO UPDATE SET monday=excluded.monday",params![w.term,w.week,w.monday]).map_err(storage)?;
                    }
                }
                "card" => {
                    if let Some(v) = rows.first() {
                        let c: campus_core::Card =
                            serde_json::from_value(v.clone()).map_err(storage)?;
                        tx.execute(
                            "INSERT OR REPLACE INTO card_status VALUES(?,?,?,?,?)",
                            params![account, now, c.card_balance, c.net_fee, c.new_mail],
                        )
                        .map_err(storage)?;
                    }
                }
                _ => {}
            }
            tx.execute("DELETE FROM change_event WHERE account_id=? AND id NOT IN (SELECT id FROM change_event WHERE account_id=? ORDER BY id DESC LIMIT 100)",params![account,account]).map_err(storage)?;
        }
        tx.execute(
            "DELETE FROM card_status WHERE account_id=? AND fetched_at<?",
            params![account, now - 90 * 86400],
        )
        .map_err(storage)?;
        tx.execute("INSERT INTO sync_state(account_id,dataset,last_attempt_at,last_success_at,last_status,last_reason) VALUES(?,?,?,?,'success',NULL) ON CONFLICT(account_id,dataset) DO UPDATE SET last_attempt_at=excluded.last_attempt_at,last_success_at=excluded.last_success_at,last_status='success',last_reason=NULL",params![account,dataset,now,now]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(changes.len())
    }
    pub fn failure(
        &self,
        account: i64,
        dataset: &str,
        status: &str,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO sync_state(account_id,dataset,last_attempt_at,last_status,last_reason) VALUES(?,?,?,?,?) ON CONFLICT(account_id,dataset) DO UPDATE SET last_attempt_at=excluded.last_attempt_at,last_status=excluded.last_status,last_reason=excluded.last_reason",params![account,dataset,now,status,reason]).map_err(storage)?;
        Ok(())
    }
    pub fn fetched_at(&self, account: i64, dataset: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT last_success_at FROM sync_state WHERE account_id=? AND dataset=?",
                params![account, dataset],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.flatten())
            .map_err(storage)
    }
    pub fn sync_failure(&self, account: i64, dataset: &str) -> Result<Option<String>> {
        let reason: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT last_reason FROM sync_state WHERE account_id=? AND dataset=?",
                params![account, dataset],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        Ok(reason.flatten())
    }
    pub fn sync_status(&self, account: i64) -> Result<Vec<Value>> {
        let mut q=self.conn.prepare_cached("SELECT dataset,last_attempt_at,last_success_at,last_status,last_reason FROM sync_state WHERE account_id=? OR account_id=0 ORDER BY dataset").map_err(storage)?;
        q.query_map([account],|r|Ok(json!({"dataset":r.get::<_,String>(0)?,"last_attempt_at":r.get::<_,Option<i64>>(1)?,"last_success_at":r.get::<_,Option<i64>>(2)?,"status":r.get::<_,Option<String>>(3)?,"reason":r.get::<_,Option<String>>(4)?}))).map_err(storage)?.collect::<std::result::Result<_,_>>().map_err(storage)
    }
    pub fn pref(&self, scope: &str, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM pref WHERE scope=? AND key=?",
                params![scope, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }
    pub fn set_pref(&self, scope: &str, key: &str, value: Option<&str>) -> Result<()> {
        if let Some(v) = value {
            self.conn.execute("INSERT INTO pref VALUES(?,?,?) ON CONFLICT(scope,key) DO UPDATE SET value=excluded.value",params![scope,key,v]).map_err(storage)?;
        } else {
            self.conn
                .execute(
                    "DELETE FROM pref WHERE scope=? AND key=?",
                    params![scope, key],
                )
                .map_err(storage)?;
        }
        Ok(())
    }
    pub fn changes(&self, account: i64, unseen: bool) -> Result<Vec<Value>> {
        let mut q=self.conn.prepare_cached("SELECT id,dataset,kind,item_key,before_json,after_json,detected_at,seen FROM change_event WHERE account_id=? AND (?=0 OR seen=0) ORDER BY id DESC").map_err(storage)?;
        q.query_map(params![account,unseen],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"dataset":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"item_key":r.get::<_,String>(3)?,"before":r.get::<_,Option<String>>(4)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()),"after":r.get::<_,Option<String>>(5)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()),"detected_at":r.get::<_,i64>(6)?,"seen":r.get::<_,bool>(7)?}))).map_err(storage)?.collect::<std::result::Result<_,_>>().map_err(storage)
    }
    pub fn mark_seen(&self, account: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE change_event SET seen=1 WHERE account_id=?",
                [account],
            )
            .map_err(storage)?;
        Ok(())
    }
    pub fn save_session(
        &self,
        account: i64,
        jar: &str,
        cipher: &[u8],
        nonce: &[u8],
        now: i64,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO http_session(account_id,jar,cookies,nonce,saved_at) VALUES(?,?,?,?,?) ON CONFLICT(account_id,jar) DO UPDATE SET cookies=excluded.cookies,nonce=excluded.nonce,saved_at=excluded.saved_at",params![account,jar,cipher,nonce,now]).map_err(storage)?;
        Ok(())
    }
    pub fn session(&self, account: i64, jar: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        self.conn
            .query_row(
                "SELECT cookies,nonce FROM http_session WHERE account_id=? AND jar=?",
                params![account, jar],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)
    }
    pub fn clear_session(&self, account: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM http_session WHERE account_id=?", [account])
            .map_err(storage)?;
        Ok(())
    }
    pub fn clear_session_jar(&self, account: i64, jar: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM http_session WHERE account_id=? AND jar=?",
                params![account, jar],
            )
            .map_err(storage)?;
        Ok(())
    }
    pub fn secret(&self, name: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        self.conn
            .query_row(
                "SELECT ciphertext,nonce FROM secret WHERE name=?",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)
    }
    pub fn set_secret(&self, name: &str, value: Option<(&[u8], &[u8])>) -> Result<()> {
        if let Some((c, n)) = value {
            self.conn.execute("INSERT INTO secret VALUES(?,?,?) ON CONFLICT(name) DO UPDATE SET ciphertext=excluded.ciphertext,nonce=excluded.nonce",params![name,c,n]).map_err(storage)?;
        } else {
            self.conn
                .execute("DELETE FROM secret WHERE name=?", [name])
                .map_err(storage)?;
        }
        Ok(())
    }
    pub fn clear(&mut self, account: i64) -> Result<()> {
        let tx = self.conn.transaction().map_err(storage)?;
        for t in [
            "dataset_row",
            "course",
            "grade",
            "exam",
            "grade_selection",
            "program_course_type",
            "card_status",
            "sync_state",
            "change_event",
        ] {
            tx.execute(&format!("DELETE FROM {t} WHERE account_id=?"), [account])
                .map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM").map_err(storage)
    }
    pub fn info(&self) -> Result<Value> {
        let pages: i64 = self
            .conn
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(storage)?;
        let size: i64 = self
            .conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(storage)?;
        Ok(json!({"schema":1,"bytes":pages*size,"accounts":self.accounts()?.len()}))
    }
    pub fn history(&self, account: i64) -> Result<Vec<Value>> {
        let mut q=self.conn.prepare_cached("SELECT fetched_at,card_balance,net_fee,CAST(new_mail AS TEXT) FROM card_status WHERE account_id=? ORDER BY fetched_at DESC").map_err(storage)?;
        q.query_map([account],|r|Ok(json!({"fetched_at":r.get::<_,i64>(0)?,"card_balance":r.get::<_,String>(1)?,"net_fee":r.get::<_,String>(2)?,"new_mail":r.get::<_,String>(3)?}))).map_err(storage)?.collect::<std::result::Result<_,_>>().map_err(storage)
    }
}
