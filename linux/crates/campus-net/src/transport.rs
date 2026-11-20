use campus_core::{Error, Result};
use reqwest::{Client, Method, header};
use reqwest_cookie_store::{CookieStore, CookieStoreMutex};
use rustls::{
    CertificateError, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::ring::default_provider,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

const GEOTRUST_G2_CN_2022_CA1_DER: &[u8] = include_bytes!("geotrust_g2_cn_2022_ca1.der");

fn is_bjtu_domain(server_name: &ServerName<'_>) -> bool {
    let ServerName::DnsName(dns) = server_name else {
        return false;
    };
    let host = dns.as_ref().trim_end_matches('.').to_ascii_lowercase();
    host == "bjtu.edu.cn" || host.ends_with(".bjtu.edu.cn")
}

#[derive(Debug)]
struct BjtuIntermediateVerifier {
    inner: Arc<WebPkiServerVerifier>,
}

impl BjtuIntermediateVerifier {
    fn new() -> Result<Self> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let inner = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(default_provider()),
        )
        .build()
        .map_err(|_| Error::Network)?;
        Ok(Self { inner })
    }
}

impl ServerCertVerifier for BjtuIntermediateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => Ok(verified),
            Err(TlsError::InvalidCertificate(CertificateError::UnknownIssuer))
                if intermediates.is_empty() && is_bjtu_domain(server_name) =>
            {
                let hint = [CertificateDer::from(GEOTRUST_G2_CN_2022_CA1_DER)];
                self.inner
                    .verify_server_cert(end_entity, &hint, server_name, ocsp_response, now)
            }
            Err(err) => Err(err),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn bjtu_tls_config() -> Result<rustls::ClientConfig> {
    let verifier = Arc::new(BjtuIntermediateVerifier::new()?);
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Network)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

fn default_phyvlab() -> Url {
    "https://phyvlab.bjtu.edu.cn/".parse().unwrap()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoints {
    pub mis: Url,
    pub sso: Url,
    pub aa: Url,
    pub calendar: Url,
    pub smart: Url,
    pub headcount: Url,
    #[serde(default = "default_phyvlab")]
    pub phyvlab: Url,
    #[serde(skip)]
    pub legacy_homework: bool,
    #[serde(skip)]
    pub legacy_courseware: bool,
    #[serde(skip)]
    test_http: bool,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            mis: "https://mis.bjtu.edu.cn/".parse().unwrap(),
            sso: "https://sso.bjtu.edu.cn/".parse().unwrap(),
            aa: "https://aa.bjtu.edu.cn/".parse().unwrap(),
            calendar: "https://bksy.bjtu.edu.cn/".parse().unwrap(),
            smart: "https://bksycenter.bjtu.edu.cn/".parse().unwrap(),
            headcount: "http://yaya.csoci.com:2333/".parse().unwrap(),
            phyvlab: default_phyvlab(),
            legacy_homework: false,
            legacy_courseware: false,
            test_http: false,
        }
    }
}
impl Endpoints {
    pub fn configure(&mut self, name: &str, value: &str) -> Result<()> {
        let u = Url::parse(value).map_err(|_| Error::Usage("invalid endpoint URL".into()))?;
        if !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err(Error::Usage(
                "endpoint must be an origin without credentials".into(),
            ));
        }
        if name != "headcount" && (u.scheme() != "https" || u.port_or_known_default() != Some(443))
        {
            return Err(Error::SecureChannel);
        }
        if name == "headcount"
            && u.scheme() != "https"
            && u.as_str() != "http://yaya.csoci.com:2333/"
        {
            return Err(Error::SecureChannel);
        }
        match name {
            "mis" => self.mis = u,
            "sso" => self.sso = u,
            "aa" => self.aa = u,
            "calendar" => self.calendar = u,
            "smart" => self.smart = u,
            "headcount" => self.headcount = u,
            "phyvlab" => self.phyvlab = u,
            _ => return Err(Error::Usage("unknown endpoint".into())),
        }
        Ok(())
    }
    pub fn login_allowed(&self, u: &Url) -> bool {
        self.secure(u)
            && [&self.mis, &self.sso]
                .iter()
                .any(|base| same_origin(base, u))
    }
    pub fn school_allowed(&self, u: &Url) -> bool {
        self.secure(u)
            && ([&self.mis, &self.sso, &self.aa, &self.smart]
                .iter()
                .any(|base| same_origin(base, u))
                || u.host_str() == Some("cas.bjtu.edu.cn") && u.path().starts_with("/o/authorize/"))
    }
    pub fn secure(&self, u: &Url) -> bool {
        u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && ((u.scheme() == "https" && u.port_or_known_default() == Some(443))
                || (self.test_http && u.scheme() == "http" && u.host_str() == Some("127.0.0.1")))
    }
    pub fn smart_allowed(&self, u: &Url) -> bool {
        self.secure(u)
            && (same_origin(u, &self.smart) && u.path().starts_with("/ve/")
                || same_origin(u, &self.mis)
                    && ["/module/module/28/", "/home/", "/auth/nsso/"].contains(&u.path())
                || same_origin(u, &self.sso) && ["/o/authorize/", "/sso/login"].contains(&u.path())
                || u.host_str() == Some("cas.bjtu.edu.cn") && u.path().starts_with("/o/authorize/"))
    }
    pub fn phyvlab_allowed(&self, u: &Url) -> bool {
        self.secure(u) && same_origin(u, &self.phyvlab)
    }
    #[cfg(test)]
    pub fn for_test(base: &str) -> Self {
        let u = Url::parse(base).unwrap();
        assert_eq!(u.host_str(), Some("127.0.0.1"));
        Self {
            mis: u.clone(),
            sso: u.clone(),
            aa: u.clone(),
            calendar: u.clone(),
            smart: u.clone(),
            headcount: u.clone(),
            phyvlab: u,
            legacy_homework: false,
            legacy_courseware: false,
            test_http: true,
        }
    }
}
pub fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}
pub(crate) fn is_legacy_smart_origin(u: &Url) -> bool {
    u.scheme() == "http" && u.host_str() == Some("123.121.147.7") && u.port() == Some(88)
}
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0";
struct Gate {
    permit: Semaphore,
    last: Mutex<Option<tokio::time::Instant>>,
    gap: Duration,
    retry: bool,
}
impl Gate {
    async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            tokio::time::sleep_until(t + self.gap).await;
        }
        *last = Some(tokio::time::Instant::now());
    }
}
pub struct Reply {
    pub status: u16,
    pub url: Url,
    pub location: Option<String>,
    pub bytes: Vec<u8>,
}
impl Reply {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    pub fn data(&self, expected: &Url) -> Result<String> {
        let text = self.text();
        if campus_core::parse::session_expired(self.status, self.url.as_str(), &text) {
            return Err(Error::SessionExpired);
        }
        if !same_origin(&self.url, expected) {
            return Err(Error::SecureChannel);
        }
        if !(200..300).contains(&self.status) {
            return Err(Error::Network);
        }
        Ok(text)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyScope {
    Homework,
    Courseware,
}

pub struct SchoolTransport {
    pub endpoints: Endpoints,
    school: Client,
    public: Client,
    pub jar: Arc<CookieStoreMutex>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    legacy_scope: Option<LegacyScope>,
}
impl SchoolTransport {
    pub fn new(endpoints: Endpoints, cookies: Option<&[u8]>) -> Result<Self> {
        let store = if let Some(b) = cookies {
            let cookies: Vec<cookie_store::Cookie<'static>> =
                serde_json::from_slice(b).map_err(|_| Error::Malformed("saved cookies"))?;
            CookieStore::from_cookies(cookies.into_iter().map(Ok::<_, std::io::Error>), false)
                .map_err(|_| Error::Malformed("saved cookie jar"))?
        } else {
            CookieStore::default()
        };
        Self::build(endpoints, store, None)
    }
    pub(crate) fn new_legacy(&self, scope: LegacyScope) -> Result<Self> {
        let enabled = match scope {
            LegacyScope::Homework => self.endpoints.legacy_homework,
            LegacyScope::Courseware => self.endpoints.legacy_courseware,
        };
        if !enabled {
            return Err(Error::SmartLegacyHttp);
        }
        let store = self.jar.lock().map_err(|_| Error::Network)?;
        // Copy school-domain cookies only. The compatibility jar is never saved by sync.
        let cookies = store
            .iter_unexpired()
            .filter(|cookie| {
                cookie.domain.as_cow().is_some_and(|domain| {
                    domain == "bjtu.edu.cn" || domain.ends_with(".bjtu.edu.cn")
                })
            })
            .cloned()
            .map(Ok::<_, std::io::Error>);
        let store = CookieStore::from_cookies(cookies, false)
            .map_err(|_| Error::Malformed("school cookies"))?;
        let endpoints = Endpoints {
            smart: crate::legacy_homework::ORIGIN
                .parse()
                .expect("fixed legacy URL"),
            ..Endpoints::default()
        };
        Self::build(endpoints, store, Some(scope))
    }
    fn build(
        endpoints: Endpoints,
        store: CookieStore,
        legacy_scope: Option<LegacyScope>,
    ) -> Result<Self> {
        let jar = Arc::new(CookieStoreMutex::new(store));
        let tls = bjtu_tls_config()?;
        let builder = || {
            let client = Client::builder()
                .use_preconfigured_tls(tls.clone())
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(30));
            if legacy_scope.is_some() {
                client.no_proxy()
            } else {
                client
            }
        };
        // Redirects are followed manually: every hop passes through the host gate and allowlist.
        let school = builder()
            .cookie_provider(jar.clone())
            .build()
            .map_err(|_| Error::Network)?;
        let public = builder()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            endpoints,
            school,
            public,
            jar,
            gates: Mutex::new(HashMap::new()),
            legacy_scope,
        })
    }
    #[cfg(feature = "media")]
    pub(crate) fn media_client(&self) -> &Client {
        &self.school
    }
    pub fn cookies(&self) -> Result<Vec<u8>> {
        let store = self.jar.lock().map_err(|_| Error::Network)?;
        let mut cookies = store.iter_unexpired().collect::<Vec<_>>();
        // A stable order keeps the sealed jar byte-identical when nothing changed.
        cookies.sort_by_cached_key(|c| serde_json::to_string(c).unwrap_or_default());
        serde_json::to_vec(&cookies).map_err(|_| Error::Malformed("cookie serialization"))
    }
    pub fn clear(&self) {
        if let Ok(mut jar) = self.jar.lock() {
            jar.clear();
        }
    }
    pub fn has_cookie(&self, url: &Url, name: &str) -> bool {
        self.jar
            .lock()
            .ok()
            .is_some_and(|store| store.get_request_values(url).any(|(k, _)| k == name))
    }
    async fn gate(&self, url: &Url) -> Arc<Gate> {
        let key = url.origin().ascii_serialization();
        let mut gates = self.gates.lock().await;
        gates
            .entry(key)
            .or_insert_with(|| {
                let aa = same_origin(url, &self.endpoints.aa);
                let count = if aa || same_origin(url, &self.endpoints.sso) {
                    1
                } else if same_origin(url, &self.endpoints.smart) {
                    5
                } else {
                    2
                };
                Arc::new(Gate {
                    permit: Semaphore::new(count),
                    last: Mutex::new(None),
                    gap: Duration::from_millis(if aa { 400 } else { 0 }),
                    retry: aa,
                })
            })
            .clone()
    }
    async fn once(
        &self,
        method: Method,
        url: Url,
        form: Option<&BTreeMap<String, String>>,
        public: bool,
        session: Option<&str>,
    ) -> Result<Reply> {
        self.request(method, url, form, public, session, None).await
    }
    async fn request(
        &self,
        method: Method,
        url: Url,
        form: Option<&BTreeMap<String, String>>,
        public: bool,
        session: Option<&str>,
        referer: Option<&str>,
    ) -> Result<Reply> {
        let gate = self.gate(&url).await;
        let _permit = gate.permit.acquire().await.map_err(|_| Error::Network)?;
        let mut attempt = 0;
        loop {
            gate.wait().await;
            let client = if public { &self.public } else { &self.school };
            let mut req = client.request(method.clone(), url.clone());
            if !public {
                req = req.header(
                    "Referer",
                    referer.map_or_else(|| url.origin().ascii_serialization(), str::to_owned),
                );
            }
            let ticket = self.legacy_courseware() && method == Method::POST;
            if ticket {
                req = req.header("Origin", url.origin().ascii_serialization());
            }
            if session.is_some()
                || ticket
                || self.legacy_scope.is_some_and(|scope| {
                    crate::legacy_homework::allows_api(&url, scope == LegacyScope::Courseware)
                })
            {
                req = req
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("Accept", "application/json, text/javascript, */*; q=0.01");
            }
            if let Some(session) = session {
                req = req.header("sessionid", session);
            }
            if let Some(fields) = form {
                req = req.form(fields);
            }
            let response = req.send().await.map_err(|_| Error::Network)?;
            let status = response.status().as_u16();
            if status == 503 && method == Method::GET && gate.retry && attempt < 3 {
                drop(response);
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                attempt += 1;
                continue;
            }
            return read_reply(url, response).await;
        }
    }
    pub(crate) async fn phyvlab_send(
        &self,
        url: Url,
        build: impl FnOnce(&Client, &Url) -> reqwest::RequestBuilder,
    ) -> Result<Reply> {
        if !self.endpoints.phyvlab_allowed(&url) {
            return Err(Error::SecureChannel);
        }
        let gate = self.gate(&url).await;
        let _permit = gate.permit.acquire().await.map_err(|_| Error::Network)?;
        gate.wait().await;
        let response = build(&self.school, &url)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        read_reply(url, response).await
    }
    pub async fn get(&self, url: Url) -> Result<Reply> {
        self.follow(url, false).await
    }
    pub async fn login_get(&self, url: Url) -> Result<Reply> {
        self.follow(url, true).await
    }
    async fn follow(&self, mut url: Url, login: bool) -> Result<Reply> {
        for _ in 0..12 {
            if !(if login {
                self.endpoints.login_allowed(&url)
            } else {
                self.endpoints.school_allowed(&url)
            }) {
                return Err(Error::SecureChannel);
            }
            let r = self
                .once(Method::GET, url.clone(), None, false, None)
                .await?;
            if !(300..400).contains(&r.status) {
                return Ok(r);
            }
            url = url
                .join(
                    r.location
                        .as_deref()
                        .ok_or(Error::Malformed("redirect location"))?,
                )
                .map_err(|_| Error::SecureChannel)?;
        }
        Err(Error::Rejected("redirect limit"))
    }
    pub async fn post_once(&self, url: Url, form: &BTreeMap<String, String>) -> Result<Reply> {
        if self.legacy_scope.is_some() || !self.endpoints.school_allowed(&url) {
            return Err(Error::SecureChannel);
        }
        self.once(Method::POST, url, Some(form), false, None).await
    }
    pub async fn no_redirect(&self, url: Url) -> Result<Reply> {
        if !self.endpoints.school_allowed(&url) {
            return Err(Error::SecureChannel);
        }
        self.once(Method::GET, url, None, false, None).await
    }
    pub async fn smart_get(&self, url: Url, session: Option<&str>) -> Result<Reply> {
        if let Some(scope) = self.legacy_scope {
            if !crate::legacy_homework::allows_api(&url, scope == LegacyScope::Courseware) {
                return Err(Error::SecureChannel);
            }
            let reply = self.once(Method::GET, url, None, false, session).await?;
            return if (300..400).contains(&reply.status) {
                Err(Error::SessionExpired)
            } else {
                Ok(reply)
            };
        }
        if !self.endpoints.secure(&url)
            || !same_origin(&url, &self.endpoints.smart)
            || !url.path().starts_with("/ve/")
        {
            return Err(Error::SecureChannel);
        }
        self.once(Method::GET, url, None, false, session).await
    }
    pub(crate) fn legacy_courseware(&self) -> bool {
        self.legacy_scope == Some(LegacyScope::Courseware)
    }
    fn legacy_policy_url<'a>(&self, url: &'a Url) -> std::borrow::Cow<'a, Url> {
        #[cfg(test)]
        if self.endpoints.test_http && same_origin(url, &self.endpoints.smart) {
            let mut target = Url::parse(crate::legacy_homework::ORIGIN).unwrap();
            target.set_path(url.path());
            target.set_query(url.query());
            return std::borrow::Cow::Owned(target);
        }
        std::borrow::Cow::Borrowed(url)
    }
    pub(crate) fn legacy_download_allowed(&self, url: &Url) -> bool {
        self.legacy_courseware()
            && crate::legacy_homework::allows_download(&self.legacy_policy_url(url))
    }
    pub async fn smart_post(&self, url: Url, session: &str) -> Result<Reply> {
        if self.legacy_courseware() {
            if !crate::legacy_homework::allows_ticket(&self.legacy_policy_url(&url)) {
                return Err(Error::SecureChannel);
            }
            let referer = "http://123.121.147.7:88/ve/back/coursePlatform/coursePlatform.shtml";
            return self
                .request(
                    Method::POST,
                    url,
                    Some(&BTreeMap::new()),
                    false,
                    None,
                    Some(referer),
                )
                .await;
        }
        if !self.endpoints.secure(&url)
            || !same_origin(&url, &self.endpoints.smart)
            || !url.path().starts_with("/ve/")
        {
            return Err(Error::SecureChannel);
        }
        self.once(
            Method::POST,
            url,
            Some(&BTreeMap::new()),
            false,
            Some(session),
        )
        .await
    }
    pub async fn smart_follow(&self, mut url: Url) -> Result<Reply> {
        for _ in 0..12 {
            if is_legacy_smart_origin(&url) {
                return Err(Error::SmartLegacyHttp);
            }
            if !self.endpoints.smart_allowed(&url) {
                return Err(Error::SecureChannel);
            }
            let r = self
                .once(Method::GET, url.clone(), None, false, None)
                .await?;
            if !(300..400).contains(&r.status) {
                return Ok(r);
            }
            let location = r
                .location
                .as_deref()
                .ok_or(Error::Malformed("smart redirect"))?;
            if location.chars().any(char::is_control) {
                return Err(Error::SecureChannel);
            }
            url = url.join(location).map_err(|_| Error::SecureChannel)?;
        }
        Err(Error::Rejected("smart redirect limit"))
    }
    pub(crate) async fn legacy_handshake_get(&self, url: Url, referer: &Url) -> Result<Reply> {
        if self.legacy_scope.is_none() || !crate::legacy_homework::allows_hop(&url) {
            return Err(Error::SecureChannel);
        }
        let mut referer = referer.clone();
        referer.set_query(None);
        referer.set_fragment(None);
        self.request(Method::GET, url, None, false, None, Some(referer.as_str()))
            .await
    }
    pub async fn public(&self, url: Url) -> Result<Reply> {
        let is_calendar =
            same_origin(&url, &self.endpoints.calendar) && self.endpoints.secure(&url);
        let is_headcount = same_origin(&url, &self.endpoints.headcount)
            && url.path() == "/api/classnum/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none();
        if !is_calendar && !is_headcount {
            return Err(Error::SecureChannel);
        }
        self.once(Method::GET, url, None, true, None).await
    }
}

async fn read_reply(url: Url, response: reqwest::Response) -> Result<Reply> {
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|s| s.to_str().ok())
        .map(str::to_owned);
    let max = 16 * 1024 * 1024;
    if response.content_length().is_some_and(|n| n > max) {
        return Err(Error::Malformed("response too large"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let c = chunk.map_err(|_| Error::Network)?;
        if bytes.len() + c.len() > max as usize {
            return Err(Error::Malformed("response too large"));
        }
        bytes.extend_from_slice(&c);
    }
    Ok(Reply {
        status,
        url,
        location,
        bytes,
    })
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn aa_requests_are_serial_and_start_400ms_apart() {
        let gate = Arc::new(Gate {
            permit: Semaphore::new(1),
            last: Mutex::new(None),
            gap: Duration::from_millis(400),
            retry: true,
        });
        let start = tokio::time::Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..3 {
            let gate = gate.clone();
            tasks.push(tokio::spawn(async move {
                let _p = gate.permit.acquire().await.unwrap();
                gate.wait().await;
                tokio::time::Instant::now()
            }));
        }
        let mut times = Vec::new();
        for task in tasks {
            times.push(task.await.unwrap().duration_since(start).as_millis());
        }
        times.sort();
        assert_eq!(times, vec![0, 400, 800]);
    }
}

#[cfg(all(test, feature = "media"))]
mod download_tests {
    use super::*;
    use crate::fetch::{Fetcher, SmartContext};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    #[tokio::test]
    async fn courseware_download_uses_captured_ticket_shape_and_fresh_url() {
        let server = MockServer::start().await;
        let url = format!(
            "{}/ve/download.shtml?userId=1&id=2&p=synthetic&g=synthetic",
            server.uri()
        );
        Mock::given(method("POST"))
            .and(path("/ve/back/resourceSpace.shtml"))
            .and(query_param("method", "rpinfoDownloadUrl"))
            .and(query_param("rpId", "SyntheticFile1"))
            .and(header("x-requested-with", "XMLHttpRequest"))
            .and(header("origin", server.uri().as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "flag": true, "rpUrl": url,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ve/download.shtml"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/pdf")
                    .set_body_bytes(b"%PDF-synthetic".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let transport = SchoolTransport::build(
            Endpoints::for_test(&server.uri()),
            CookieStore::default(),
            Some(LegacyScope::Courseware),
        )
        .unwrap();
        let context = SmartContext {
            fetcher: Fetcher {
                transport: &transport,
            },
            session: zeroize::Zeroizing::new("synthetic".into()),
            courses: Vec::new(),
        };
        let ticket = context.resource_ticket("SyntheticFile1").await.unwrap();
        let target = tempfile::tempdir().unwrap();
        let file = target.path().join("synthetic.pdf");
        assert_eq!(transport.download(ticket, &file).await.unwrap(), 14);
        assert_eq!(std::fs::read(file).unwrap(), b"%PDF-synthetic");
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|r| !r.headers.contains_key("sessionid"))
        );
        let alien =
            Url::parse("http://127.0.0.2/ve/download.shtml?userId=1&id=2&p=synthetic&g=synthetic")
                .unwrap();
        assert!(!transport.legacy_download_allowed(&alien));
    }

    #[test]
    fn bjtu_domain_matcher_only_matches_bjtu_hosts() {
        let ok1 = ServerName::try_from("mis.bjtu.edu.cn").unwrap();
        let ok2 = ServerName::try_from("MIS.BJTU.EDU.CN.").unwrap();
        let ok3 = ServerName::try_from("bjtu.edu.cn").unwrap();
        let bad1 = ServerName::try_from("notbjtu.edu.cn").unwrap();
        let bad2 = ServerName::try_from("evil.example").unwrap();
        assert!(is_bjtu_domain(&ok1));
        assert!(is_bjtu_domain(&ok2));
        assert!(is_bjtu_domain(&ok3));
        assert!(!is_bjtu_domain(&bad1));
        assert!(!is_bjtu_domain(&bad2));
    }
}
