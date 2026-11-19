//! Original BJTU compatibility flow, scoped to a disposable smart platform session.
use crate::transport::{Reply, SchoolTransport, same_origin};
use campus_core::{Error, Result, parse};
use url::Url;

pub(crate) const ORIGIN: &str = "http://123.121.147.7:88/";
const MIS: &str = "https://mis.bjtu.edu.cn";
const HOME: &str = "https://bksy.bjtu.edu.cn/login_introduce_t.html";
const BRIDGE: &str =
    "https://bksycenter.bjtu.edu.cn/NoMasterJumpPage.aspx?URL=jwcZhjx&FPC=page:jwcZhjx";
const BOOTSTRAP: &str = "/ve/back/core/main/qx_kl.jsp";
const INDEX: &str = "/ve/back/core/main/index.shtml?method=index&type=qxkt";
const COURSE_PLATFORM: &str = "/ve/back/coursePlatform/coursePlatform.shtml";

fn safe_parts(url: &Url) -> bool {
    url.username().is_empty() && url.password().is_none() && url.fragment().is_none()
}

fn is_legacy(url: &Url) -> bool {
    crate::transport::is_legacy_smart_origin(url) && safe_parts(url)
}

pub(crate) fn allows_hop(url: &Url) -> bool {
    if is_legacy(url) {
        return [
            "/oauth/api/user/thirdLogin",
            "/oauth/token/callBack",
            "/ve",
            "/ve/",
            "/ve/Login_2.jsp",
            BOOTSTRAP,
            INDEX.split('?').next().unwrap(),
            COURSE_PLATFORM,
        ]
        .contains(&url.path());
    }
    if !safe_parts(url) || url.scheme() != "https" || url.port_or_known_default() != Some(443) {
        return false;
    }
    let path = url.path();
    match url.host_str() {
        Some("mis.bjtu.edu.cn") => matches!(
            path,
            "/home/"
                | "/auth/nsso/"
                | "/auth/ncallback/"
                | "/module/module/28/"
                | "/module/module/104/"
        ),
        Some("sso.bjtu.edu.cn" | "cas.bjtu.edu.cn") => {
            matches!(path, "/o/authorize" | "/o/authorize/" | "/sso/login")
        }
        Some("bksy.bjtu.edu.cn") => path == "/login_introduce_t.html",
        Some("bksycenter.bjtu.edu.cn") => matches!(
            path,
            "/oauth2.ashx" | "/NoMasterJumpPage.aspx" | "/CommonLogin.aspx"
        ),
        _ => false,
    }
}

pub(crate) fn allows_api(url: &Url, courseware: bool) -> bool {
    if !is_legacy(url) {
        return false;
    }
    let mut methods = url.query_pairs().filter(|(key, _)| key == "method");
    let Some((_, method)) = methods.next() else {
        return false;
    };
    methods.next().is_none()
        && (matches!(
            (url.path(), method.as_ref()),
            ("/ve/back/coursePlatform/message.shtml", "getArticleList")
                | ("/ve/back/rp/common/teachCalendar.shtml", "queryCurrentXq")
                | ("/ve/back/coursePlatform/course.shtml", "getCourseList")
        ) || if courseware {
            (url.path(), method.as_ref())
                == (
                    "/ve/back/coursePlatform/courseResource.shtml",
                    "stuQueryUploadResourceForCourseList",
                )
        } else {
            (url.path(), method.as_ref())
                == ("/ve/back/coursePlatform/homeWork.shtml", "getHomeWorkList")
        })
}

pub(crate) fn allows_ticket(url: &Url) -> bool {
    if !is_legacy(url) || url.path() != "/ve/back/resourceSpace.shtml" {
        return false;
    }
    let query: Vec<_> = url.query_pairs().collect();
    query.len() == 2
        && query
            .iter()
            .any(|(k, v)| k == "method" && v == "rpinfoDownloadUrl")
        && query.iter().any(|(k, v)| {
            k == "rpId"
                && !v.is_empty()
                && v.len() <= 128
                && v.bytes().all(|c| c.is_ascii_alphanumeric())
        })
}

pub(crate) fn allows_download(url: &Url) -> bool {
    is_legacy(url)
        && url.path() == "/ve/download.shtml"
        && url.query_pairs().count() == 4
        && ["userId", "id", "p", "g"]
            .iter()
            .all(|key| url.query_pairs().any(|(k, v)| k == *key && !v.is_empty()))
}

fn next_target(base: &Url, location: &str) -> Result<Url> {
    if location.trim().is_empty() || location.chars().any(char::is_control) {
        return Err(Error::SecureChannel);
    }
    let mut next = base.join(location).map_err(|_| Error::SecureChannel)?;
    // Kotlin upgrades only the verified undergraduate portal OAuth callback.
    if next.scheme() == "http"
        && next.host_str() == Some("bksycenter.bjtu.edu.cn")
        && next.path() == "/oauth2.ashx"
        && next.port_or_known_default() == Some(80)
        && safe_parts(&next)
    {
        next.set_scheme("https").map_err(|_| Error::SecureChannel)?;
    }
    if allows_hop(&next) {
        Ok(next)
    } else {
        Err(Error::SecureChannel)
    }
}

async fn follow(transport: &SchoolTransport, mut url: Url, mut referer: Url) -> Result<Reply> {
    for _ in 0..12 {
        let reply = transport.legacy_handshake_get(url, &referer).await?;
        if !(300..400).contains(&reply.status) {
            return Ok(reply);
        }
        let location = reply.location.as_deref().ok_or(Error::SecureChannel)?;
        url = next_target(&reply.url, location)?;
        referer = reply.url;
    }
    Err(Error::SecureChannel)
}

fn validate_landing(reply: &Reply, transport: &SchoolTransport) -> Result<()> {
    if !same_origin(&reply.url, &transport.endpoints.smart) {
        return Err(Error::SecureChannel);
    }
    if reply.url.path() == "/ve/Login_2.jsp" {
        return Err(Error::SessionExpired);
    }
    reply.data(&transport.endpoints.smart).map(|_| ())
}

pub(crate) async fn handshake(transport: &SchoolTransport) -> Result<()> {
    let mis_home = Url::parse(&format!("{MIS}/home/")).expect("fixed MIS home URL");
    let primary = Url::parse(&format!("{MIS}/module/module/28/")).expect("fixed MIS module URL");
    if let Ok(reply) = follow(transport, primary, mis_home.clone()).await {
        if same_origin(&reply.url, &mis_home)
            && parse::session_expired(reply.status, reply.url.as_str(), &reply.text())
        {
            return Err(Error::SessionExpired);
        }
        if validate_landing(&reply, transport).is_ok() {
            return Ok(());
        }
    }
    let module = Url::parse(&format!("{MIS}/module/module/104/")).expect("fixed MIS module URL");
    let home = Url::parse(HOME).expect("fixed portal home URL");
    let portal = follow(transport, module, mis_home).await?;
    if parse::session_expired(portal.status, portal.url.as_str(), &portal.text()) {
        return Err(Error::SessionExpired);
    }
    portal.data(&home)?;
    if portal.url != home {
        return Err(Error::SecureChannel);
    }
    let mut landing = follow(
        transport,
        Url::parse(BRIDGE).expect("fixed portal bridge URL"),
        home,
    )
    .await?;
    if landing.url.path() == "/CommonLogin.aspx" && landing.url.scheme() == "https" {
        return Err(Error::SessionExpired);
    }
    if same_origin(&landing.url, &transport.endpoints.smart)
        && (200..300).contains(&landing.status)
        && landing.url.path() == BOOTSTRAP
    {
        let index = transport
            .endpoints
            .smart
            .join(INDEX)
            .expect("fixed legacy index URL");
        landing = follow(transport, index, landing.url).await?;
    }
    validate_landing(&landing, transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_portal_upgrades_only_verified_oauth_and_bounds_http_paths() {
        let base = Url::parse(BRIDGE).unwrap();
        let callback = next_target(
            &base,
            "http://bksycenter.bjtu.edu.cn/oauth2.ashx?code=synthetic",
        )
        .unwrap();
        assert_eq!(callback.scheme(), "https");
        assert_eq!(
            next_target(&base, "http://123.121.147.7:88/ve?ID=synthetic")
                .unwrap()
                .path(),
            "/ve"
        );
        for target in [
            "http://bksycenter.bjtu.edu.cn/CommonLogin.aspx",
            "https://bksycenter.bjtu.edu.cn:9443/oauth2.ashx",
            "https://mis.bjtu.edu.cn.evil.test/home/",
            "http://123.121.147.7:88/ve/admin",
            "http://123.121.147.7:88/ve#fragment",
            "http://123.121.147.7:1936/ve",
        ] {
            assert!(next_target(&base, target).is_err(), "{target}");
        }
    }

    #[test]
    fn legacy_api_accepts_only_read_only_homework_methods() {
        let homework = Url::parse(
            "http://123.121.147.7:88/ve/back/coursePlatform/homeWork.shtml?method=getHomeWorkList",
        )
        .unwrap();
        assert!(allows_api(&homework, false));
        assert!(!allows_hop(&homework));
        for query in [
            "method=submit",
            "method=getHomeWorkList&method=submit",
            "method=queryStudentCourseNote",
        ] {
            let mut target = homework.clone();
            target.set_query(Some(query));
            assert!(!allows_api(&target, false));
        }
    }
}
