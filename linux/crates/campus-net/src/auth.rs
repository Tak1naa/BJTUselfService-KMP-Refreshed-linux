use crate::transport::{SchoolTransport, same_origin};
use base64::{Engine, engine::general_purpose::STANDARD};
use campus_core::{Error, Profile, Result, parse};
use std::collections::BTreeMap;
use zeroize::Zeroizing;
pub struct Challenge {
    pub page: parse::LoginPage,
    pub page_url: url::Url,
    pub captcha_id: String,
    pub image: Vec<u8>,
}
pub enum LoginState {
    Active(Profile),
    Challenge(Challenge),
}
pub fn encrypt_password(password: &str, key: &str) -> Result<Zeroizing<String>> {
    use sm2::{
        PublicKey,
        elliptic_curve::common::getrandom::SysRng,
        pke::{EncryptingKey, Mode},
    };
    let bytes = STANDARD
        .decode(key)
        .map_err(|_| Error::Malformed("SM2 public key encoding"))?;
    let public =
        PublicKey::from_sec1_bytes(&bytes).map_err(|_| Error::Malformed("SM2 public key"))?;
    let encryptor = EncryptingKey::new_with_mode(public, Mode::C1C3C2);
    let encrypted = encryptor
        .encrypt(&mut SysRng, password.as_bytes())
        .map_err(|_| Error::Malformed("SM2 encryption"))?;
    if encrypted.first() != Some(&4) || encrypted.len() != password.len() + 97 {
        return Err(Error::Malformed("SM2 layout"));
    }
    Ok(Zeroizing::new(STANDARD.encode(&encrypted[1..])))
}
pub struct SsoLogin<'a> {
    pub transport: &'a SchoolTransport,
}
impl SsoLogin<'_> {
    pub async fn challenge(&self, user: &str, force_captcha: bool) -> Result<LoginState> {
        let ep = &self.transport.endpoints;
        let home = ep.mis.join("home/").unwrap();
        let r = self.transport.login_get(home.clone()).await?;
        if same_origin(&r.url, &home)
            && r.url.path().trim_end_matches('/') == home.path().trim_end_matches('/')
        {
            return parse::profile(&r.text(), user).map(LoginState::Active);
        }
        if !same_origin(&r.url, &ep.sso) || r.url.path() != "/o/authorize/" {
            return Err(Error::Malformed("unexpected login landing"));
        }
        let page = parse::login(&r.text())?;
        let mut challenge = Challenge {
            page,
            page_url: r.url,
            captcha_id: String::new(),
            image: Vec::new(),
        };
        if force_captcha || challenge.page.requires_captcha {
            let response = self
                .transport
                .post_once(
                    ep.sso.join("api/captcha/getCaptcha").unwrap(),
                    &BTreeMap::new(),
                )
                .await?;
            let v = parse::json_object(&response.text())?;
            challenge.captcha_id = v["data"]["captchaId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Error::Malformed("captcha ID"))?
                .into();
            let image = v["data"]["captchaImg"]
                .as_str()
                .ok_or(Error::Malformed("captcha image"))?;
            challenge.image = STANDARD
                .decode(image.split("base64,").last().unwrap_or(image))
                .map_err(|_| Error::Malformed("captcha encoding"))?;
        }
        Ok(LoginState::Challenge(challenge))
    }
    pub async fn authenticate(
        &self,
        user: &str,
        password: &str,
        challenge: Challenge,
        answer: &str,
    ) -> Result<Profile> {
        if user.trim().is_empty() || password.is_empty() {
            return Err(Error::Rejected("empty credentials"));
        }
        if !challenge.image.is_empty() && answer.trim().is_empty() {
            return Err(Error::Rejected("captcha required"));
        }
        let mut fields = challenge.page.fields;
        let encrypted = if challenge.page.sm2_public_key.is_empty() {
            Zeroizing::new(password.to_string())
        } else {
            encrypt_password(password, &challenge.page.sm2_public_key)?
        };
        fields.insert("username".into(), user.into());
        fields.insert("password".into(), encrypted.to_string());
        fields.insert("loginType".into(), "username_password".into());
        fields.insert(
            "captcha".into(),
            if challenge.captcha_id.is_empty() {
                String::new()
            } else {
                serde_json::json!({"captchaId":challenge.captcha_id,"captchaValue":answer.trim()})
                    .to_string()
            },
        );
        let ep = &self.transport.endpoints;
        let login_url = ep.sso.join("sso/login").unwrap();
        // Exactly one password POST: failures and ambiguous transport errors are final.
        let result = self.transport.post_once(login_url.clone(), &fields).await;
        use zeroize::Zeroize;
        for value in fields.values_mut() {
            value.zeroize();
        }
        let submitted = result?;
        let landing = if (300..400).contains(&submitted.status) {
            let target = login_url
                .join(
                    submitted
                        .location
                        .as_deref()
                        .ok_or(Error::Malformed("login redirect"))?,
                )
                .map_err(|_| Error::SecureChannel)?;
            self.transport.login_get(target).await?
        } else {
            submitted
        };
        if same_origin(&landing.url, &ep.sso)
            && !(same_origin(&landing.url, &ep.mis) && landing.url.path() == "/home/")
        {
            let p = parse::login(&landing.text())?;
            return Err(if p.requires_captcha {
                Error::Rejected("captcha required; run campus auth login --captcha")
            } else if p.error_code == Some(40400) {
                Error::Rejected("credentials rejected")
            } else {
                Error::Rejected("login response not accepted")
            });
        }
        let home = ep.mis.join("home/").unwrap();
        let r = if same_origin(&landing.url, &home) && landing.url.path() == home.path() {
            landing
        } else {
            self.transport.login_get(home.clone()).await?
        };
        if !same_origin(&r.url, &home) || r.url.path() != home.path() {
            return Err(Error::Rejected("login not confirmed"));
        }
        parse::profile(&r.text(), user)
    }
    pub async fn automatic(&self, user: &str, password: &str) -> Result<Profile> {
        let mut last = Error::Network;
        for _ in 0..3 {
            match self.challenge(user, false).await {
                Ok(LoginState::Active(p)) => return Ok(p),
                Ok(LoginState::Challenge(c)) => {
                    if !c.image.is_empty() {
                        return Err(Error::Rejected("captcha needs user action"));
                    }
                    return self.authenticate(user, password, c, "").await;
                }
                Err(e @ Error::Network) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
    pub async fn link_academic(&self) -> Result<()> {
        let ep = &self.transport.endpoints;
        let module = self
            .transport
            .get(ep.mis.join("module/module/10/").unwrap())
            .await?;
        let action = parse::redirect(&module.text())?;
        let url = module.url.join(&action).map_err(|_| Error::SecureChannel)?;
        if !same_origin(&url, &ep.aa) {
            return Err(Error::SecureChannel);
        }
        let r = self.transport.get(url).await?;
        if !same_origin(&r.url, &ep.aa) || r.url.path().trim_end_matches('/') != "/notice/item" {
            return Err(Error::SessionExpired);
        }
        Ok(())
    }
    pub async fn probe(&self) -> Result<bool> {
        let home = self.transport.endpoints.mis.join("home/").unwrap();
        let r = self.transport.login_get(home.clone()).await?;
        Ok(same_origin(&r.url, &home)
            && r.url.path() == home.path()
            && !parse::session_expired(r.status, r.url.as_str(), &r.text()))
    }
    pub async fn logout(&self) -> Result<()> {
        let ep = &self.transport.endpoints;
        let a = self
            .transport
            .no_redirect(ep.mis.join("auth/slogout/").unwrap())
            .await;
        let b = self
            .transport
            .no_redirect(ep.sso.join("sso/logout").unwrap())
            .await;
        self.transport.clear();
        a?;
        b?;
        Ok(())
    }
}
