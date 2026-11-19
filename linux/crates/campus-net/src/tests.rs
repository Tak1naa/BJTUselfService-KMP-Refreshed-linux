use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
const PUBLIC: &str =
    "BCNfHMe8JNA+F0xYeDk+8jpQX/u7gIRDbmOvxyzq5cUyaEGp88qjQT78wi2lAzmdPM6OVMEwDiqrRWqeeO+789s=";
const PRIVATE: &str = "xvTitqraFE8IaR0A1mRBX7OD024adxJl3Z6tn7UGSlI=";
const NODE_CIPHER: &str = "UIEP+/t+eev0UNqTlUNIFZzUsaqjQaWmNg7WNOVByPF2hzrndStvuEfJcFqvcv4T8E8AKLVcvt9n6tYkjPrw0kvbNI8rtpRtVfKVv8OUhOPSb0Viq61tu82u2xeJXT1QqnwnhCgIiaF/ThHqUsS2Tsnsg9rn";

#[tokio::test]
async fn smart_portal_identifies_and_stops_at_legacy_http_redirect() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/module/module/28/"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "http://123.121.147.7:88/?ticket=synthetic"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let transport = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let fetcher = fetch::Fetcher {
        transport: &transport,
    };
    assert!(matches!(
        fetcher.smart_handshake().await,
        Err(campus_core::Error::SmartLegacyHttp)
    ));
}
#[tokio::test]
async fn courseware_accepts_non_json_article_after_cookie_handshake() {
    use wiremock::matchers::{header, query_param};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/module/module/28/"))
        .respond_with(ResponseTemplate::new(200).insert_header(
            "Set-Cookie",
            "JSESSIONID=fixture-cookie; Path=/ve; HttpOnly",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let article = Mock::given(method("GET"))
        .and(path("/ve/back/coursePlatform/message.shtml"))
        .and(query_param("method", "getArticleList"))
        .and(header("sessionid", "fixture-cookie"));
    article
        .respond_with(ResponseTemplate::new(200).set_body_string("<p>Fixture notices</p>"))
        .expect(1)
        .mount(&server)
        .await;
    for (route, action, data) in [
        (
            "../rp/common/teachCalendar.shtml",
            "queryCurrentXq",
            serde_json::json!({"result":[{"xqCode":"fixture-term"}]}),
        ),
        (
            "course.shtml",
            "getCourseList",
            serde_json::json!({"courseList":[{"id":"fixture-course","course_num":"fixture-num","fz_id":"fixture-group","xq_code":"fixture-term"}]}),
        ),
        (
            "courseResource.shtml",
            "stuQueryUploadResourceForCourseList",
            serde_json::json!({"bagList": [],"resList":[{"rpId":7,"rpName":"Fixture PDF"}]}),
        ),
    ] {
        let prefix = url::Url::parse("http://localhost/ve/back/coursePlatform/").unwrap();
        let endpoint = prefix.join(route).unwrap();
        Mock::given(method("GET"))
            .and(path(endpoint.path()))
            .and(query_param("method", action))
            .and(header("sessionid", "fixture-cookie"))
            .respond_with(ResponseTemplate::new(200).set_body_json(data))
            .expect(1)
            .mount(&server)
            .await;
    }
    let transport = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let fetcher = fetch::Fetcher {
        transport: &transport,
    };
    let mut fallback = None;
    let context = fetcher.courseware_context(&mut fallback).await.unwrap();
    let nodes = context.walk(&context.courses[0]).await.unwrap();
    assert_eq!(nodes[0]["rpName"], "Fixture PDF");
    server.verify().await;
}

fn login_page() -> String {
    "<form id=loginForm><input name=flowId value=fixture-flow><input name=username><input name=password type=password></form>".into()
}
#[test]
fn sm2_matches_upstream_sm_crypto_vector() {
    let key = sm2::pke::DecryptingKey::from_slice(&STANDARD.decode(PRIVATE).unwrap()).unwrap();
    let mut cipher = vec![4];
    cipher.extend(STANDARD.decode(NODE_CIPHER).unwrap());
    assert_eq!(key.decrypt(&cipher).unwrap(), b"fixture-Password_123!");
    let encoded = auth::encrypt_password("fixture-Password_123!", PUBLIC).unwrap();
    let mut bytes = vec![4];
    bytes.extend(STANDARD.decode(&*encoded).unwrap());
    assert_eq!(bytes.len(), 97 + b"fixture-Password_123!".len());
    assert_eq!(key.decrypt(&bytes).unwrap(), b"fixture-Password_123!");
}
#[tokio::test]
async fn session_cookies_persist_including_nonpersistent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cookie"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "fixture_session=fixture; Path=/; HttpOnly"),
        )
        .mount(&server)
        .await;
    let ep = Endpoints::for_test(&server.uri());
    let t = SchoolTransport::new(ep.clone(), None).unwrap();
    t.get(format!("{}/cookie", server.uri()).parse().unwrap())
        .await
        .unwrap();
    let b = t.cookies().unwrap();
    assert!(String::from_utf8_lossy(&b).contains("fixture_session"));
    let reloaded = SchoolTransport::new(ep, Some(&b)).unwrap();
    assert_eq!(reloaded.cookies().unwrap(), b);
}
#[tokio::test]
async fn wrong_password_sends_one_post() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sso/login"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            "{}<script>var error = {{\"code\":40400}};\n</script>",
            login_page()
        )))
        .expect(1)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let challenge = auth::Challenge {
        page: campus_core::parse::login(&login_page()).unwrap(),
        page_url: format!("{}/o/authorize/", server.uri()).parse().unwrap(),
        captcha_id: String::new(),
        image: Vec::new(),
    };
    let result = auth::SsoLogin { transport: &t }
        .authenticate("fixture-user", "fixture-password", challenge, "")
        .await;
    assert!(result.is_err());
    server.verify().await;
}
#[tokio::test]
async fn post_503_does_not_retry_and_redirects_are_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://evil.example/collect"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    assert_eq!(
        t.post_once(
            format!("{}/post", server.uri()).parse().unwrap(),
            &BTreeMap::new()
        )
        .await
        .unwrap()
        .status,
        503
    );
    assert!(matches!(
        t.login_get(format!("{}/redirect", server.uri()).parse().unwrap())
            .await,
        Err(campus_core::Error::SecureChannel)
    ));
    server.verify().await;
}
#[tokio::test]
async fn full_sso_flow() {
    let server = MockServer::start().await;
    // One home entry and one final confirmation, using a cookie to distinguish them.
    Mock::given(method("GET"))
        .and(path("/home/"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/o/authorize/"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/o/authorize/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(login_page()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sso/login"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/home/"))
        .expect(1)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let gateway = auth::SsoLogin { transport: &t };
    let auth::LoginState::Challenge(c) = gateway.challenge("fixture-user", false).await.unwrap()
    else {
        panic!("expected challenge")
    };
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/sso/login"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/home/"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/home/")).respond_with(ResponseTemplate::new(200).set_body_string("<div class=name_right><h3><a>测试，欢迎</a></h3><div class=nr_con><span>身份：学生</span><span>部门：测试学院</span></div></div>")).mount(&server).await;
    assert_eq!(
        gateway
            .authenticate("fixture-user", "fixture-password", c, "")
            .await
            .unwrap()
            .name,
        "测试"
    );
    server.verify().await;
}
#[test]
fn production_endpoint_policy() {
    let ep = Endpoints::default();
    for u in [
        "http://sso.bjtu.edu.cn/sso/login",
        "https://sso.bjtu.edu.cn:444/sso/login",
        "https://sso.bjtu.edu.cn.evil.example/sso/login",
        "https://user@sso.bjtu.edu.cn/sso/login",
    ] {
        assert!(!ep.login_allowed(&u.parse().unwrap()));
    }
}
#[tokio::test]
async fn only_get_retries_503_with_bounded_backoff() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    Mock::given(method("GET"))
        .and(path("/retry"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(if seen.fetch_add(1, Ordering::SeqCst) < 3 {
                503
            } else {
                200
            })
        })
        .expect(4)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        t.get(format!("{}/retry", server.uri()).parse().unwrap())
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(count.load(Ordering::SeqCst), 4);
    assert!(start.elapsed() >= std::time::Duration::from_secs(7));
    server.verify().await;
}

#[tokio::test]
async fn ambiguous_password_transport_failure_is_not_replayed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sso/login"))
        .respond_with_err(|_: &wiremock::Request| {
            std::io::Error::from(std::io::ErrorKind::ConnectionReset)
        })
        .expect(1)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let c = auth::Challenge {
        page: campus_core::parse::login(&login_page()).unwrap(),
        page_url: format!("{}/o/authorize/", server.uri()).parse().unwrap(),
        captcha_id: String::new(),
        image: Vec::new(),
    };
    assert!(
        auth::SsoLogin { transport: &t }
            .authenticate("fixture-user", "fixture-password", c, "")
            .await
            .is_err()
    );
    server.verify().await;
}
#[tokio::test]
async fn captcha_is_explicit_and_missing_answer_never_posts_password() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/home/"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/o/authorize/"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/o/authorize/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(login_page()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/captcha/getCaptcha"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"data":{"captchaId":"fixture-captcha","captchaImg":"aW1hZ2U="}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sso/login"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let gateway = auth::SsoLogin { transport: &t };
    let auth::LoginState::Challenge(c) = gateway.challenge("fixture-user", true).await.unwrap()
    else {
        panic!("expected challenge")
    };
    assert!(!c.image.is_empty());
    assert!(
        gateway
            .authenticate("fixture-user", "fixture-password", c, "")
            .await
            .is_err()
    );
    server.verify().await;
}
#[tokio::test]
async fn already_logged_in_needs_no_password_post() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/home/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../tests/fixtures/profile.html")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    assert!(matches!(
        auth::SsoLogin { transport: &t }
            .challenge("fixture-user", false)
            .await
            .unwrap(),
        auth::LoginState::Active(_)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn public_client_never_sends_school_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cookie"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "fixture_session=fixture; Path=/"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/classnum/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;
    let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    t.get(format!("{}/cookie", server.uri()).parse().unwrap())
        .await
        .unwrap();
    t.public(format!("{}/api/classnum/", server.uri()).parse().unwrap())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.last().unwrap().headers.contains_key("cookie"));
}

#[tokio::test]
async fn room_view_uses_default_semester_and_first_page() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/classroom/timeholdresult/room_view/"))
        .and(query_param("page", "1"))
        .and(query_param("zc", "10"))
        .and(query_param("jxlh", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../tests/fixtures/rooms.html")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let transport = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
    let options = [("term", "current"), ("building", "1"), ("week", "10")]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    let rows = fetch::Fetcher {
        transport: &transport,
    }
    .fetch("rooms", &options)
    .await
    .unwrap();
    assert!(!rows.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].url.query_pairs().all(|(k, _)| k != "zxjxjhh"));
}
