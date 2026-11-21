use crate::{
    args::{Args, PhyvlabAction},
    data_freshness, emit, ensure_dataset_fresh, from_rows, now, output, read_password_input,
    runtime,
    transfer::safe_name,
};
use campus_core::{
    Error, Freshness, Result,
    phyvlab::{
        self, PhyVlabActivity, PhyVlabAssignmentDetail, PhyVlabCourse, activity_status_label,
        submission_timing_label,
    },
};
use campus_db::{Database, config::Config, paths::Paths};
use campus_net::phyvlab::{PhyVlabClient, SessionOutcome, UploadFile, capture_captcha_proof};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub fn run(
    args: &Args,
    action: Option<&PhyvlabAction>,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
) -> Result<u8> {
    let default_list = PhyvlabAction::List {
        course: None,
        pending: false,
    };
    let action = action.unwrap_or(&default_list);
    match action {
        PhyvlabAction::Status => status(args, db, config, user),
        PhyvlabAction::Logout { forget } => logout(args, db, paths, config, user, *forget),
        PhyvlabAction::Login {
            user: lab_user,
            password_stdin,
            no_remember,
            remember,
            no_browser,
        } => login(
            args,
            db,
            paths,
            config,
            user,
            lab_user.as_deref(),
            *password_stdin,
            *no_remember,
            *remember,
            *no_browser,
        ),
        PhyvlabAction::Download { id, output } => {
            if args.offline {
                return Err(Error::Usage(
                    "phyvlab download requires network access".into(),
                ));
            }
            let user = user.ok_or(Error::SessionExpired)?;
            let dir = output.as_deref().unwrap_or(Path::new("."));
            let data = download_attachments(db, paths, config, user, *id, dir)?;
            emit(args, "phyvlab download", Some(user), data)?;
            Ok(0)
        }
        PhyvlabAction::Upload { id, files, remove } => {
            if args.offline {
                return Err(Error::Usage(
                    "phyvlab upload requires network access".into(),
                ));
            }
            let user = user.ok_or(Error::SessionExpired)?;
            upload_draft(args, db, paths, config, user, *id, files, remove)
        }
        PhyvlabAction::Finalize { id, yes } => {
            if args.offline {
                return Err(Error::Usage(
                    "phyvlab finalize requires network access".into(),
                ));
            }
            let user = user.ok_or(Error::SessionExpired)?;
            finalize_submission(args, db, paths, config, user, *id, *yes)
        }
        PhyvlabAction::Courses => read_courses(args, db, paths, config, user),
        PhyvlabAction::List { course, pending } => {
            read_list(args, db, paths, config, user, course.as_deref(), *pending)
        }
        PhyvlabAction::Show { id } => read_show(args, db, paths, config, user, *id),
    }
}

fn status(args: &Args, db: &Database, _config: &Config, user: Option<&str>) -> Result<u8> {
    let account = user.map(|s| db.account(s)).transpose()?;
    let lab_user = account
        .map(|id| db.pref(&format!("phyvlab:{id}"), "username"))
        .transpose()?
        .flatten();
    let session_saved = account
        .map(|id| db.session(id, "phyvlab"))
        .transpose()?
        .flatten()
        .is_some();
    emit(
        args,
        "phyvlab status",
        user,
        json!({
            "configured": lab_user.is_some(),
            "phyvlab_user": lab_user,
            "session_saved": session_saved,
            "verified": false,
            "note": "离线状态不证明物理在线会话仍有效",
        }),
    )?;
    Ok(0)
}

fn logout(
    args: &Args,
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
    forget: bool,
) -> Result<u8> {
    let user = user.ok_or(Error::SessionExpired)?;
    let id = db.account(user)?;
    let mut warning = None;
    if !args.offline {
        match campus_sync::load_transport_jar(db, paths, config, Some(id), "phyvlab") {
            Ok((transport, _)) => {
                if runtime()?
                    .block_on(
                        PhyVlabClient {
                            transport: &transport,
                        }
                        .logout(),
                    )
                    .is_err()
                {
                    warning = Some("远端注销未确认，本地物理在线会话已清除");
                }
            }
            Err(_) => warning = Some("远端注销未确认，本地物理在线会话已清除"),
        }
    }
    db.clear_session_jar(id, "phyvlab")?;
    if forget {
        let v = campus_sync::vault(paths, config)?;
        campus_sync::clear_phyvlab_credentials(db, &v, id, user)?;
    }
    emit(
        args,
        "phyvlab logout",
        Some(user),
        json!({"cleared": true, "password_forgotten": forget, "warning": warning}),
    )?;
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
fn login(
    args: &Args,
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
    explicit_lab_user: Option<&str>,
    password_stdin: bool,
    no_remember: bool,
    remember: bool,
    no_browser: bool,
) -> Result<u8> {
    if args.offline {
        return Err(Error::Usage("phyvlab login cannot be offline".into()));
    }
    if remember && no_remember {
        return Err(Error::Usage(
            "--remember conflicts with --no-remember".into(),
        ));
    }
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let mis_user = user.ok_or(Error::SessionExpired)?;
    let account = db.account(mis_user)?;
    let (transport, _) =
        campus_sync::load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let v = campus_sync::vault(paths, config)?;
    let saved = campus_sync::load_phyvlab_credentials(db, &v, mis_user)?;

    let lab_user = explicit_lab_user
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            db.pref(&format!("phyvlab:{account}"), "username")
                .ok()
                .flatten()
        })
        .or_else(|| saved.as_ref().map(|(u, _)| u.clone()))
        .unwrap_or_else(|| mis_user.to_owned());

    let password: Zeroizing<String> = if password_stdin {
        read_password_input(true, "")?
    } else if explicit_lab_user.is_none()
        && let Some((saved_user, saved_pass)) = saved.as_ref()
        && *saved_user == lab_user
    {
        saved_pass.clone()
    } else if io::stdin().is_terminal() {
        read_password_input(false, &format!("物理在线密码 ({lab_user}): "))?
    } else {
        return Err(Error::Usage(
            "use --password-stdin for non-interactive phyvlab login".into(),
        ));
    };

    let client = PhyVlabClient {
        transport: &transport,
    };
    let open_browser = !no_browser;
    let outcome = runtime()?.block_on(client.sign_in_interactive(
        &lab_user,
        &password,
        move |captcha_id| async move {
            tokio::task::spawn_blocking(move || capture_captcha_proof(&captcha_id, open_browser))
                .await
                .map_err(|_| Error::Network)?
        },
    ))?;

    match outcome {
        SessionOutcome::Ready => {
            campus_sync::save_transport_jar(db, &transport, &v, account, "phyvlab", now())?;
            let mut password_saved = false;
            if remember || !no_remember {
                campus_sync::save_phyvlab_credentials(
                    db, &v, account, mis_user, &lab_user, &password,
                )?;
                password_saved = true;
            } else {
                db.set_pref(&format!("phyvlab:{account}"), "username", Some(&lab_user))?;
            }
            emit(
                args,
                "phyvlab login",
                Some(mis_user),
                json!({
                    "authenticated": true,
                    "phyvlab_user": lab_user,
                    "password_saved": password_saved,
                }),
            )?;
            Ok(0)
        }
        SessionOutcome::CaptchaRejected => Err(Error::Rejected("物理在线验证码未通过，请重试")),
        SessionOutcome::CredentialsRejected => {
            Err(Error::Rejected("物理在线账号或密码错误，请检查后重试"))
        }
        SessionOutcome::AccountRequired => Err(Error::ActionRequired("account_required")),
        SessionOutcome::CaptchaRequired { .. } => Err(Error::ActionRequired("captcha_required")),
    }
}

pub(crate) fn interactive_tui_login(
    args: &Args,
    db: &Database,
    paths: &Paths,
    config: &Config,
    mis_user: &str,
) -> Result<bool> {
    use std::io::Write;
    let account = db.account(mis_user)?;
    let default_user = db
        .pref(&format!("phyvlab:{account}"), "username")
        .ok()
        .flatten()
        .unwrap_or_else(|| mis_user.to_owned());
    eprintln!("物理在线登录（需校园网或学校 VPN；如遇验证码会自动打开浏览器点选）");
    eprint!("物理在线账号（直接 Enter 使用 {default_user}，输入账号将重输密码，输入 q 取消）: ");
    io::stderr().flush().map_err(campus_db::storage)?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(campus_db::storage)?;
    let trimmed = input.trim();
    if matches!(trimmed, "q" | "Q") {
        return Ok(false);
    }
    let explicit = (!trimmed.is_empty()).then_some(trimmed);
    login(
        args,
        db,
        paths,
        config,
        Some(mis_user),
        explicit,
        false,
        false,
        true,
        false,
    )?;
    Ok(true)
}

fn read_courses(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
) -> Result<u8> {
    let account = db.account(user.ok_or(Error::SessionExpired)?)?;
    let mut warnings = Vec::new();
    let (fetched, source) = ensure_dataset_fresh(
        args,
        db,
        paths,
        config,
        user,
        account,
        "phyvlab",
        "phyvlab",
        &BTreeMap::new(),
        &mut warnings,
    )?;
    let _ = fetched;
    let courses: Vec<PhyVlabCourse> = from_rows(db.rows(account, "phyvlab_courses")?)?;
    let courses_fetched = db.fetched_at(account, "phyvlab_courses")?;
    output::render(
        args,
        &output::envelope(
            "phyvlab courses",
            user,
            now(),
            (
                courses_fetched,
                data_freshness(config, "phyvlab_courses", courses_fetched, now())?
                    != Freshness::Fresh,
                source,
            ),
            json!(courses),
            &warnings,
        ),
    )?;
    Ok(0)
}

fn read_list(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
    course_filter: Option<&str>,
    pending_only: bool,
) -> Result<u8> {
    let account = db.account(user.ok_or(Error::SessionExpired)?)?;
    let mut warnings = Vec::new();
    let (fetched, source) = ensure_dataset_fresh(
        args,
        db,
        paths,
        config,
        user,
        account,
        "phyvlab",
        "phyvlab",
        &BTreeMap::new(),
        &mut warnings,
    )?;
    let mut activities: Vec<PhyVlabActivity> = from_rows(db.rows(account, "phyvlab")?)?;
    let current = now();
    if let Some(q) = course_filter {
        let q_lower = q.to_lowercase();
        activities.retain(|a| a.course_name.to_lowercase().contains(&q_lower));
    }
    if pending_only {
        activities.retain(|a| !matches!(activity_status_label(a, current), "已提交" | "逾期提交"));
    }
    let rows: Vec<Value> = activities
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "course_id": a.course_id,
                "course_name": a.course_name,
                "title": a.title,
                "open_text": a.open_text,
                "due_text": a.due_text,
                "due_timestamp": a.due_timestamp,
                "completed": a.completed,
                "status_label": activity_status_label(a, current),
                "grade_text": a.detail.as_ref().and_then(|d| d.grade_text.as_deref()),
            })
        })
        .collect();
    output::render(
        args,
        &output::envelope(
            "phyvlab list",
            user,
            current,
            (
                fetched,
                data_freshness(config, "phyvlab", fetched, current)? != Freshness::Fresh,
                source,
            ),
            json!(rows),
            &warnings,
        ),
    )?;
    Ok(0)
}

fn read_show(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
    id: i64,
) -> Result<u8> {
    let user_str = user.ok_or(Error::SessionExpired)?;
    let account = db.account(user_str)?;
    let mut warnings = Vec::new();
    let (mut fetched, mut source) = ensure_dataset_fresh(
        args,
        db,
        paths,
        config,
        user,
        account,
        "phyvlab",
        "phyvlab",
        &BTreeMap::new(),
        &mut warnings,
    )?;
    let mut activities: Vec<PhyVlabActivity> = from_rows(db.rows(account, "phyvlab")?)?;
    let idx = activities
        .iter()
        .position(|a| a.id == id)
        .ok_or(Error::NoData)?;
    if !args.offline && (args.refresh || activities[idx].detail.is_none()) {
        match fetch_and_cache_detail(db, paths, config, user_str, account, &mut activities, idx) {
            Ok(()) => {
                fetched = db.fetched_at(account, "phyvlab")?;
                source = "network";
            }
            Err(e) => warnings.push(format!("刷新作业详情失败，显示缓存结果：{e}")),
        }
    }
    let activity = &activities[idx];
    let current = now();
    let detail = activity.detail.clone().unwrap_or_default();
    let data = json!({
        "id": activity.id,
        "course_id": activity.course_id,
        "course_name": activity.course_name,
        "title": activity.title,
        "open_text": activity.open_text,
        "due_text": activity.due_text,
        "status_label": activity_status_label(activity, current),
        "submission_status": detail.submission_status,
        "submission_date_text": detail.submission_date_text,
        "timing": submission_timing_label(activity),
        "grading_status": detail.grading_status,
        "grade_text": detail.grade_text,
        "feedback_text": detail.feedback_text,
        "submitted_files": detail.submitted_files,
        "attachments": detail.attachments.iter().map(|a| &a.file_name).collect::<Vec<_>>(),
        "can_submit": detail.can_submit,
        "can_finalize": detail.can_finalize,
        "description": detail.description,
    });
    output::render(
        args,
        &output::envelope(
            "phyvlab show",
            user,
            current,
            (
                fetched,
                data_freshness(config, "phyvlab", fetched, current)? != Freshness::Fresh,
                source,
            ),
            data,
            &warnings,
        ),
    )?;
    Ok(0)
}

fn fetch_and_cache_detail(
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    account: i64,
    activities: &mut [PhyVlabActivity],
    idx: usize,
) -> Result<()> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let (transport, secret) =
        campus_sync::load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let v = match secret {
        Some(v) => v,
        None => campus_sync::vault(paths, config)?,
    };
    let creds = campus_sync::load_phyvlab_credentials(db, &v, user)?;
    let detail = runtime()?.block_on(async {
        let client = PhyVlabClient {
            transport: &transport,
        };
        let creds_ref = creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
        ensure_ready(&client, creds_ref).await?;
        let res = client.fetch_assignment_detail(&activities[idx]).await;
        if matches!(res, Err(Error::SessionExpired)) {
            transport.clear();
            ensure_ready(&client, creds_ref).await?;
            client.fetch_assignment_detail(&activities[idx]).await
        } else {
            res
        }
    })?;
    if let Some(done) = phyvlab::detail_submission_state(&detail) {
        activities[idx].completed = done;
    }
    activities[idx].detail = Some(detail);
    let rows: Vec<Value> = activities
        .iter()
        .map(|a| serde_json::to_value(a).map_err(|_| Error::Malformed("model serialization")))
        .collect::<Result<_>>()?;
    let current = now();
    campus_sync::save_transport_jar(db, &transport, &v, account, "phyvlab", current)?;
    db.save(account, "phyvlab", &rows, current)?;
    Ok(())
}

pub(crate) fn download_attachments(
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: i64,
    dir: &Path,
) -> Result<Value> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, secret) =
        campus_sync::load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let v = match secret {
        Some(v) => v,
        None => campus_sync::vault(paths, config)?,
    };
    let creds = campus_sync::load_phyvlab_credentials(db, &v, user)?;
    let cached: Vec<PhyVlabActivity> = from_rows(db.rows(account, "phyvlab")?)?;
    let activity = cached
        .into_iter()
        .find(|a| a.id == id)
        .ok_or(Error::NoData)?;
    let results = runtime()?.block_on(async {
        let client = PhyVlabClient {
            transport: &transport,
        };
        let creds_ref = creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
        ensure_ready(&client, creds_ref).await?;
        let detail = match activity.detail {
            Some(ref d) => d.clone(),
            None => client.fetch_assignment_detail(&activity).await?,
        };
        if detail.attachments.is_empty() {
            return Err(Error::NoData);
        }
        let mut out = Vec::with_capacity(detail.attachments.len());
        for attachment in &detail.attachments {
            let dest = unique_dest(dir, &safe_name(&attachment.file_name));
            let bytes = client.download_attachment(attachment, &dest).await?;
            out.push(json!({"path": dest, "bytes": bytes}));
        }
        Ok::<_, Error>(out)
    })?;
    campus_sync::save_transport_jar(db, &transport, &v, account, "phyvlab", now())?;
    Ok(json!(results))
}

#[allow(clippy::too_many_arguments)]
fn upload_draft(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: i64,
    files: &[PathBuf],
    remove: &[String],
) -> Result<u8> {
    if files.is_empty() && remove.is_empty() {
        return Err(Error::Usage(
            "phyvlab upload ID FILE... [--rm NAME...]".into(),
        ));
    }
    let mut loaded = Vec::with_capacity(files.len());
    for path in files {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Usage("invalid upload file name".into()))?
            .to_owned();
        let bytes = std::fs::read(path).map_err(campus_db::storage)?;
        let content_type = guess_content_type(&file_name);
        loaded.push((file_name, content_type, bytes));
    }
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, secret) =
        campus_sync::load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let v = match secret {
        Some(v) => v,
        None => campus_sync::vault(paths, config)?,
    };
    let creds = campus_sync::load_phyvlab_credentials(db, &v, user)?;
    let detail = runtime()?.block_on(async {
        let client = PhyVlabClient {
            transport: &transport,
        };
        let creds_ref = creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
        ensure_ready(&client, creds_ref).await?;
        let upload_refs: Vec<UploadFile<'_>> = loaded
            .iter()
            .map(|(name, ct, bytes)| UploadFile {
                file_name: name,
                content_type: ct,
                bytes,
            })
            .collect();
        client.save_files(id, &upload_refs, remove).await
    })?;
    update_cached_detail(db, account, id, &detail)?;
    campus_sync::save_transport_jar(db, &transport, &v, account, "phyvlab", now())?;
    emit(
        args,
        "phyvlab upload",
        Some(user),
        json!({
            "id": id,
            "saved": true,
            "is_draft": detail.is_draft,
            "can_finalize": detail.can_finalize,
            "submitted_files": detail.submitted_files,
            "note": "文件已保存为草稿，尚未最终提交；确认后请运行 campus phyvlab finalize",
        }),
    )?;
    Ok(0)
}

fn finalize_submission(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: i64,
    yes: bool,
) -> Result<u8> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, secret) =
        campus_sync::load_transport_jar(db, paths, config, Some(account), "phyvlab")?;
    let v = match secret {
        Some(v) => v,
        None => campus_sync::vault(paths, config)?,
    };
    let creds = campus_sync::load_phyvlab_credentials(db, &v, user)?;
    let detail = runtime()?.block_on(async {
        let client = PhyVlabClient {
            transport: &transport,
        };
        let creds_ref = creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
        ensure_ready(&client, creds_ref).await?;
        let form = client.finalization_form(id).await?;
        if let Some(statement) = form.statement.as_deref() {
            if !yes {
                if !io::stdin().is_terminal() {
                    return Err(Error::Usage(
                        "phyvlab finalize requires --yes to confirm submission statement".into(),
                    ));
                }
                eprintln!("提交声明：{statement}");
                eprint!("确认最终提交？输入 yes 继续: ");
                let mut answer = String::new();
                io::stdin()
                    .read_line(&mut answer)
                    .map_err(campus_db::storage)?;
                if !answer.trim().eq_ignore_ascii_case("yes") {
                    return Err(Error::Rejected("已取消最终提交"));
                }
            }
            client.finalize(id, Some(statement)).await
        } else {
            if !yes && !io::stdin().is_terminal() {
                return Err(Error::Usage(
                    "phyvlab finalize requires --yes in non-interactive mode".into(),
                ));
            }
            client.finalize(id, None).await
        }
    })?;
    update_cached_detail(db, account, id, &detail)?;
    campus_sync::save_transport_jar(db, &transport, &v, account, "phyvlab", now())?;
    emit(
        args,
        "phyvlab finalize",
        Some(user),
        json!({
            "id": id,
            "finalized": true,
            "submission_status": detail.submission_status,
            "submitted_files": detail.submitted_files,
        }),
    )?;
    Ok(0)
}

fn update_cached_detail(
    db: &mut Database,
    account: i64,
    id: i64,
    detail: &PhyVlabAssignmentDetail,
) -> Result<()> {
    let mut activities: Vec<PhyVlabActivity> = from_rows(db.rows(account, "phyvlab")?)?;
    if let Some(act) = activities.iter_mut().find(|a| a.id == id) {
        if let Some(done) = phyvlab::detail_submission_state(detail) {
            act.completed = done;
        }
        act.detail = Some(detail.clone());
        let rows: Vec<Value> = activities
            .into_iter()
            .map(|a| serde_json::to_value(a).map_err(|_| Error::Malformed("model serialization")))
            .collect::<Result<_>>()?;
        db.save(account, "phyvlab", &rows, now())?;
    }
    Ok(())
}

async fn ensure_ready(client: &PhyVlabClient<'_>, creds: Option<(&str, &str)>) -> Result<()> {
    match client.establish_session(creds).await? {
        SessionOutcome::Ready => Ok(()),
        SessionOutcome::AccountRequired => Err(Error::ActionRequired("account_required")),
        SessionOutcome::CaptchaRequired { .. } | SessionOutcome::CaptchaRejected => {
            Err(Error::ActionRequired("captcha_required"))
        }
        SessionOutcome::CredentialsRejected => Err(Error::ActionRequired("credentials_rejected")),
    }
}

fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    for idx in 1..1000 {
        let next = dir.join(format!("{stem}-{idx}{ext}"));
        if !next.exists() {
            return next;
        }
    }
    candidate
}

fn guess_content_type(file_name: &str) -> &'static str {
    let ext = file_name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" | "md" | "csv" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
