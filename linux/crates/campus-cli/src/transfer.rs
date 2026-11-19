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
                Command::Homework { action, id, output } if action == "download" => {
                    let id = id
                        .as_deref()
                        .ok_or_else(|| Error::Usage("homework download ID".into()))?;
                    let cached = db.rows(account, "homework")?;
                    let hw = cached
                        .iter()
                        .find(|v| v.get("id").and_then(parse::scalar).as_deref() == Some(id))
                        .ok_or(Error::NoData)?;
                    let course = hw
                        .get("course_id")
                        .and_then(parse::scalar)
                        .ok_or(Error::Malformed("homework course"))?;
                    let detail = context.homework_detail(id, &course).await?;
                    let files = campus_net::fetch::smart_array(&detail, "picList")?;
                    let dir = output.as_deref().unwrap_or(std::path::Path::new("."));
                    std::fs::create_dir_all(dir).map_err(campus_db::storage)?;
                    let mut results = Vec::new();
                    for f in files {
                        let attachment = f
                            .get("id")
                            .and_then(parse::scalar)
                            .ok_or(Error::Malformed("attachment ID"))?;
                        let mut url = transport
                            .endpoints
                            .smart
                            .join("/ve/back/coursePlatform/dataSynAction.shtml")
                            .unwrap();
                        url.query_pairs_mut()
                            .append_pair("method", "downLoadPic")
                            .append_pair("id", &attachment)
                            .append_pair("noteId", id);
                        let path =
                            dir.join(safe_name(f["file_name"].as_str().unwrap_or(&attachment)));
                        let bytes = transport.download(url, &path).await?;
                        results.push(json!({"path":path,"bytes":bytes}));
                    }
                    Ok(json!(results))
                }
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
