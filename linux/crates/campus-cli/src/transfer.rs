use crate::args::*;
#[cfg(feature = "media")]
use crate::{emit, now, runtime};
#[cfg(feature = "media")]
use campus_core::parse;
use campus_core::{Error, Result};
use campus_db::{Database, config::Config, paths::Paths};
#[cfg(feature = "media")]
use serde_json::json;

#[cfg(feature = "media")]
pub fn run(
    args: &Args,
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
) -> Result<u8> {
    if args.offline {
        return Err(Error::Usage(
            "download and recording commands require network access".into(),
        ));
    }
    if let Command::Courseware {
        action, id, output, ..
    } = &args.command
        && action == "get"
    {
        let id = id
            .as_deref()
            .ok_or_else(|| Error::Usage("courseware get COURSE:RESOURCE".into()))?;
        let output = output.as_deref().unwrap_or(std::path::Path::new("."));
        let data = courseware_download(db, paths, config, user, id, output)?;
        emit(args, "transfer", Some(user), data)?;
        return Ok(0);
    }
    if let Command::Homework {
        action,
        id,
        output,
        submitted,
        ..
    } = &args.command
        && action == "download"
    {
        let id = id
            .as_deref()
            .ok_or_else(|| Error::Usage("homework download ID".into()))?;
        let output = output.as_deref().unwrap_or(std::path::Path::new("."));
        let data = homework_download(db, paths, config, user, id, output, *submitted)?;
        emit(args, "transfer", Some(user), data)?;
        return Ok(0);
    }
    if let Command::Homework {
        action,
        id,
        files,
        content,
        yes,
        ..
    } = &args.command
        && action == "submit"
    {
        let id = id
            .as_deref()
            .ok_or_else(|| Error::Usage("homework submit ID --file PATH...".into()))?;
        let data = homework_submit(
            db,
            paths,
            config,
            user,
            id,
            files,
            content,
            *yes || args.json,
        )?;
        emit(args, "transfer", Some(user), data)?;
        return Ok(0);
    }
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, vault) = campus_sync::load_transport(db, paths, config, Some(account))?;
    let rt = runtime()?;
    let data =
        rt.block_on(async {
            let fetcher = campus_net::fetch::Fetcher {
                transport: &transport,
            };
            let context = fetcher.smart_context().await?;
            match &args.command {
                Command::Courseware {
                    action,
                    id,
                    output,
                    remux,
                    streams,
                    ..
                } => {
                    let id = id
                        .as_deref()
                        .ok_or_else(|| Error::Usage("courseware action requires ID".into()))?;
                    if action == "lessons" {
                        return Ok(json!(context.lessons(id).await?));
                    }
                    let (course_id, item) = id
                        .split_once(':')
                        .map(|(a, b)| (Some(a), b))
                        .unwrap_or((None, id));
                    if action == "record" {
                        let mut course = None;
                        for c in context.courses.iter().filter(|c| {
                            course_id.is_none_or(|id| {
                                c.get("id").and_then(parse::scalar).as_deref() == Some(id)
                            })
                        }) {
                            let cid = c
                                .get("id")
                                .and_then(parse::scalar)
                                .ok_or(Error::Malformed("course ID"))?;
                            if context.lessons(&cid).await?.iter().any(|l| {
                                l.get("id").and_then(parse::scalar).as_deref() == Some(item)
                            }) {
                                course = Some(c);
                                break;
                            }
                        }
                        let detail = context
                            .playback_detail(course.ok_or(Error::NoData)?, item)
                            .await?;
                        let map = &detail["res"]["streamMap"];
                        if map.get("haveStream").and_then(parse::scalar).as_deref() == Some("0") {
                            return Err(Error::Rejected("recording is unavailable"));
                        }
                        let dir = output.as_deref().unwrap_or(std::path::Path::new("."));
                        std::fs::create_dir_all(dir).map_err(campus_db::storage)?;
                        let mut results = Vec::new();
                        for kind in streams.split(',') {
                            let field = match kind {
                                "teacher" => "teaStreamHlsUrl",
                                "screen" => "vgaStreamHlsUrl",
                                "student" => "stuStreamHlsUrl",
                                _ => {
                                    return Err(Error::Usage(
                                        "--streams teacher,screen,student".into(),
                                    ));
                                }
                            };
                            let Some(url) = map[field].as_str().filter(|s| !s.is_empty()) else {
                                continue;
                            };
                            let url = url.parse().map_err(|_| Error::SecureChannel)?;
                            let path = dir.join(format!("{}-{kind}.ts", safe_name(item)));
                            let count = transport.record(url, &path, *remux).await?;
                            results.push(json!({"path":path,"segments":count}));
                        }
                        if results.is_empty() {
                            return Err(Error::NoData);
                        }
                        Ok(json!(results))
                    } else {
                        Err(Error::Usage("courseware ls|get|lessons|record".into()))
                    }
                }
                Command::Homework { action, .. } => Err(Error::Usage(format!(
                    "unsupported homework transfer action: {action}"
                ))),
                _ => Err(Error::Usage("invalid transfer command".into())),
            }
        });
    let vault = match vault {
        Some(v) => v,
        None => campus_sync::vault(paths, config)?,
    };
    campus_sync::save_transport(db, &transport, &vault, account, now())?;
    emit(args, "transfer", Some(user), data?)?;
    Ok(0)
}
#[cfg(feature = "media")]
pub(crate) fn courseware_download(
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: &str,
    dir: &std::path::Path,
) -> Result<serde_json::Value> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, _) = campus_sync::load_transport(db, paths, config, Some(account))?;
    let (course_id, item) = id.split_once(':').map_or((None, id), |(c, r)| (Some(c), r));
    let matches_course = |course: &serde_json::Value| {
        course_id.is_none_or(|c| parse::scalar(&course["id"]).as_deref() == Some(c))
    };
    let matches_resource = |node: &serde_json::Value| {
        node["kind"] == "resource"
            && ["resId", "rpId"]
                .iter()
                .any(|key| parse::scalar(&node[key]).as_deref() == Some(item))
    };
    let cached = db.rows(account, "courseware")?;
    let cached_node = cached
        .iter()
        .filter(|course| matches_course(course))
        .flat_map(|course| course["nodes"].as_array().into_iter().flatten())
        .find(|node| matches_resource(node))
        .cloned();
    runtime()?.block_on(async {
        let fetcher = campus_net::fetch::Fetcher {
            transport: &transport,
        };
        let mut fallback = None;
        let session = fetcher.courseware_context(&mut fallback).await?;
        let node = if let Some(node) = cached_node {
            node
        } else {
            let mut found = None;
            for course in session
                .courses
                .iter()
                .filter(|course| matches_course(course))
            {
                found = session
                    .walk(course)
                    .await?
                    .into_iter()
                    .find(&matches_resource);
                if found.is_some() {
                    break;
                }
            }
            found.ok_or(Error::NoData)?
        };
        if parse::scalar(&node["stu_download"]).is_some_and(|v| v.trim() != "2") {
            return Err(Error::Rejected("teacher has not enabled downloads"));
        }
        let rp = parse::scalar(&node["rpId"]).ok_or(Error::Malformed("resource ID"))?;
        let url = session.resource_ticket(&rp).await?;
        std::fs::create_dir_all(dir).map_err(campus_db::storage)?;
        let mut name = safe_name(node["rpName"].as_str().unwrap_or("resource"));
        if let Some(extension) = node["extName"]
            .as_str()
            .map(|s| s.trim_start_matches('.'))
            .filter(|s| !s.is_empty())
        {
            let extension = safe_name(extension);
            if !name
                .to_lowercase()
                .ends_with(&format!(".{}", extension.to_lowercase()))
            {
                name = format!("{name}.{extension}");
            }
        }
        let path = dir.join(name);
        let size = session.fetcher.transport.download(url, &path).await?;
        Ok(json!({"path":path,"bytes":size}))
    })
}

#[cfg(feature = "media")]
fn unique_dest(
    dir: &std::path::Path,
    used: &mut std::collections::BTreeSet<String>,
    raw_name: &str,
) -> std::path::PathBuf {
    let base = safe_name(raw_name);
    if used.insert(base.clone()) {
        return dir.join(base);
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e)),
        _ => (base.as_str(), None),
    };
    for idx in 2u32.. {
        let candidate = match ext {
            Some(e) => format!("{stem} ({idx}).{e}"),
            None => format!("{stem} ({idx})"),
        };
        if used.insert(candidate.clone()) {
            return dir.join(candidate);
        }
    }
    unreachable!()
}

#[cfg(feature = "media")]
pub(crate) fn refresh_homework_detail(
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: &str,
) -> Result<serde_json::Value> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, _) = campus_sync::load_transport(db, paths, config, Some(account))?;
    let mut cached = db.rows(account, "homework")?;
    let idx = cached
        .iter()
        .position(|v| v.get("id").and_then(parse::scalar).as_deref() == Some(id))
        .ok_or(Error::NoData)?;
    let course = cached[idx]
        .get("course_id")
        .and_then(parse::scalar)
        .ok_or(Error::Malformed("homework course"))?;
    let fallback_content = cached[idx]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let sn_id =
        parse::scalar(&cached[idx]["snId"]).filter(|s| !matches!(s.trim(), "" | "0" | "null"));
    let score = parse::scalar(&cached[idx]["stu_score"]).unwrap_or_default();
    let user_id = parse::scalar(&cached[idx]["user_id"]).unwrap_or_else(|| "0".into());

    let (content, attachments, submitted) = runtime()?.block_on(async {
        let fetcher = campus_net::fetch::Fetcher {
            transport: &transport,
        };
        let mut fallback = None;
        let context = fetcher.homework_context(&mut fallback).await?;
        let raw_detail = context.homework_detail(id, &course).await?;
        let (content, attachments) =
            parse::homework_detail(&raw_detail.to_string(), &fallback_content)?;
        let submitted = if let Some(sn_id) = sn_id.as_deref() {
            context
                .submitted_attachments(id, sn_id, &score, &user_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok::<_, Error>((content, attachments, submitted))
    })?;

    if let Some(obj) = cached[idx].as_object_mut() {
        obj.insert("content".into(), json!(content));
        obj.insert("attachments".into(), json!(attachments));
        obj.insert("submitted_attachments".into(), json!(submitted));
    }
    let updated = cached[idx].clone();
    db.save(account, "homework", &cached, now())?;
    Ok(updated)
}

#[cfg(feature = "media")]
pub(crate) fn homework_download(
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: &str,
    dir: &std::path::Path,
    include_submitted: bool,
) -> Result<serde_json::Value> {
    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let (transport, _) = campus_sync::load_transport(db, paths, config, Some(account))?;
    let cached = db.rows(account, "homework")?;
    let hw = cached
        .iter()
        .find(|v| v.get("id").and_then(parse::scalar).as_deref() == Some(id))
        .ok_or(Error::NoData)?;
    let course = hw
        .get("course_id")
        .and_then(parse::scalar)
        .ok_or(Error::Malformed("homework course"))?;
    let sn_id = parse::scalar(&hw["snId"]).filter(|s| !matches!(s.trim(), "" | "0" | "null"));
    let score = parse::scalar(&hw["stu_score"]).unwrap_or_default();
    let user_id = parse::scalar(&hw["user_id"]).unwrap_or_else(|| "0".into());

    runtime()?.block_on(async {
        let fetcher = campus_net::fetch::Fetcher {
            transport: &transport,
        };
        let mut fallback = None;
        let context = fetcher.homework_context(&mut fallback).await?;
        let detail = context.homework_detail(id, &course).await?;
        let (_, teacher_files) = parse::homework_detail(&detail.to_string(), "")?;
        let submitted_files = if include_submitted && let Some(sn_id) = sn_id.as_deref() {
            context
                .submitted_attachments(id, sn_id, &score, &user_id)
                .await?
        } else {
            Vec::new()
        };
        if teacher_files.is_empty() && submitted_files.is_empty() {
            return Err(Error::NoData);
        }
        std::fs::create_dir_all(dir).map_err(campus_db::storage)?;
        let mut used = std::collections::BTreeSet::new();
        let mut results = Vec::new();
        for f in teacher_files {
            let mut url = context
                .fetcher
                .transport
                .endpoints
                .smart
                .join("/ve/back/coursePlatform/dataSynAction.shtml")
                .unwrap();
            url.query_pairs_mut()
                .append_pair("method", "downLoadPic")
                .append_pair("id", &f.id)
                .append_pair("noteId", id);
            let path = unique_dest(dir, &mut used, &f.file_name);
            let bytes = context.fetcher.transport.download(url, &path).await?;
            results.push(json!({"kind":"teacher","path":path,"bytes":bytes}));
        }
        for f in submitted_files {
            let mut url = context
                .fetcher
                .transport
                .endpoints
                .smart
                .join("/ve/downloadZyFj.shtml")
                .unwrap();
            url.query_pairs_mut()
                .append_pair("path", &f.source_path)
                .append_pair("filename", &f.file_name)
                .append_pair("id", &f.id);
            let path = unique_dest(dir, &mut used, &f.file_name);
            let bytes = context.fetcher.transport.download(url, &path).await?;
            results.push(json!({"kind":"submitted","path":path,"bytes":bytes}));
        }
        Ok(json!(results))
    })
}

#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn homework_submit(
    db: &mut Database,
    paths: &Paths,
    config: &Config,
    user: &str,
    id: &str,
    files: &[std::path::PathBuf],
    content: &str,
    skip_prompt: bool,
) -> Result<serde_json::Value> {
    if files.is_empty() {
        return Err(Error::Usage("homework submit ID --file PATH ...".into()));
    }
    let mut payloads = Vec::with_capacity(files.len());
    for file_path in files {
        let meta = std::fs::metadata(file_path).map_err(campus_db::storage)?;
        if !meta.is_file() {
            return Err(Error::Usage(format!(
                "not a regular file: {}",
                file_path.display()
            )));
        }
        if meta.len() > 100 * 1024 * 1024 {
            return Err(Error::Usage(format!(
                "file exceeds 100 MiB limit: {}",
                file_path.display()
            )));
        }
        let name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Usage("invalid attachment filename".into()))?
            .to_owned();
        let bytes = std::fs::read(file_path).map_err(campus_db::storage)?;
        payloads.push((name, bytes));
    }

    let _lock = campus_db::paths::SyncLock::acquire(paths)?;
    let account = db.account(user)?;
    let mut cached = db.rows(account, "homework")?;
    let idx = cached
        .iter()
        .position(|v| v.get("id").and_then(parse::scalar).as_deref() == Some(id))
        .ok_or(Error::NoData)?;
    let hw = cached[idx].clone();

    if !skip_prompt {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(Error::Usage(
                "non-interactive homework submit requires --yes".into(),
            ));
        }
        let title = hw["title"].as_str().unwrap_or(id);
        eprint!(
            "确认上传 {} 个文件并覆盖提交作业「{title}」？[y/N]: ",
            payloads.len()
        );
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(campus_db::storage)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(Error::Rejected("homework submission cancelled"));
        }
    }

    let (transport, _) = campus_sync::load_transport(db, paths, config, Some(account))?;
    runtime()?.block_on(async {
        let fetcher = campus_net::fetch::Fetcher {
            transport: &transport,
        };
        let mut fallback = None;
        let context = fetcher.homework_context(&mut fallback).await?;
        let refs: Vec<(&str, &[u8])> = payloads
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        context.submit_homework(&hw, content, &refs).await
    })?;

    if let Some(obj) = cached[idx].as_object_mut() {
        obj.insert("subStatus".into(), json!("已提交"));
    }
    db.save(account, "homework", &cached, now())?;
    let uploaded_names: Vec<&str> = payloads.iter().map(|(n, _)| n.as_str()).collect();
    Ok(json!({
        "id": id,
        "submitted": true,
        "files": uploaded_names,
    }))
}

#[cfg(not(feature = "media"))]
pub(crate) fn refresh_homework_detail(
    _db: &mut Database,
    _paths: &Paths,
    _config: &Config,
    _user: &str,
    _id: &str,
) -> Result<serde_json::Value> {
    Err(Error::Usage("this build has media disabled".into()))
}

#[cfg(not(feature = "media"))]
pub(crate) fn homework_download(
    _db: &Database,
    _paths: &Paths,
    _config: &Config,
    _user: &str,
    _id: &str,
    _dir: &std::path::Path,
    _include_submitted: bool,
) -> Result<serde_json::Value> {
    Err(Error::Usage("this build has media disabled".into()))
}

#[cfg(not(feature = "media"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn homework_submit(
    _db: &mut Database,
    _paths: &Paths,
    _config: &Config,
    _user: &str,
    _id: &str,
    _files: &[std::path::PathBuf],
    _content: &str,
    _skip_prompt: bool,
) -> Result<serde_json::Value> {
    Err(Error::Usage("this build has media disabled".into()))
}

#[cfg(not(feature = "media"))]
pub(crate) fn courseware_download(
    _db: &Database,
    _paths: &Paths,
    _config: &Config,
    _user: &str,
    _id: &str,
    _dir: &std::path::Path,
) -> Result<serde_json::Value> {
    Err(Error::Usage("this build has media disabled".into()))
}

#[cfg(not(feature = "media"))]
pub fn run(
    _args: &Args,
    _db: &mut Database,
    _paths: &Paths,
    _config: &Config,
    _user: &str,
) -> Result<u8> {
    Err(Error::Usage("this build has media disabled".into()))
}
pub(crate) fn safe_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_control()
                || ['/', '\\', ':'].contains(&c)
                || (cfg!(windows) && ['<', '>', '"', '|', '?', '*'].contains(&c))
            {
                '_'
            } else {
                c
            }
        })
        .take(160)
        .collect();
    let s = s.trim_matches('.').trim();
    #[cfg(windows)]
    let s = s.trim_end_matches([' ', '.']);
    #[cfg(windows)]
    {
        let stem = s
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        let port = stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"));
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || port.is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        }) {
            return format!("_{s}");
        }
    }
    if s.is_empty() {
        "attachment".into()
    } else {
        s.into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_cannot_escape() {
        assert_eq!(safe_name("../../secret"), "_.._secret");
        assert!(!safe_name("a/b\\c").contains('/'));
    }
}
