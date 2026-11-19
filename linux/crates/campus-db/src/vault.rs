//! Secrets are never logged; cached academic reads do not construct this type.
use crate::{
    Database,
    paths::{self, Paths},
    storage,
};
use campus_core::{Error, Result};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use std::io::{Read, Write};
#[cfg(feature = "keyring")]
use zeroize::Zeroize;
use zeroize::Zeroizing;
pub struct Vault {
    key: Zeroizing<[u8; 32]>,
    #[cfg(feature = "keyring")]
    backend: String,
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0; N];
    getrandom::getrandom(&mut b).map_err(|_| storage("OS random source unavailable"))?;
    Ok(b)
}
impl Vault {
    #[cfg(feature = "sqlcipher")]
    pub fn database_key(&self) -> &[u8; 32] {
        &self.key
    }
    pub fn open(paths: &Paths, backend: &str, passphrase: Option<&str>) -> Result<Self> {
        paths::private_dir(&paths.data)?;
        // Key material is written straight into a zeroizing buffer so no stack copy outlives it.
        let mut key = Zeroizing::new([0u8; 32]);
        match backend {
            "file" => {
                let (bytes, created) = load_or_create::<32>(&paths.data.join("vault.key"))?;
                key.copy_from_slice(&*bytes);
                if created {
                    eprintln!(
                        "campus: file vault selected; key and encrypted secrets share the same user directory"
                    );
                }
            }
            "passphrase" => {
                let pass = passphrase.filter(|p| !p.is_empty()).ok_or(Error::Rejected(
                    "a non-empty passphrase is required; set CAMPUS_PORTAL_PASSPHRASE",
                ))?;
                let (salt, _) = load_or_create::<16>(&paths.data.join("vault.salt"))?;
                argon2::Argon2::default()
                    .hash_password_into(pass.as_bytes(), &*salt, &mut *key)
                    .map_err(|_| storage("passphrase derivation failed"))?;
            }
            "keyring" => Self::keyring_key(&mut key)?,
            _ => return Err(Error::Usage("unknown vault backend".into())),
        }
        Ok(Self {
            key,
            #[cfg(feature = "keyring")]
            backend: backend.into(),
        })
    }
    #[cfg(feature = "keyring")]
    fn keyring_key(key: &mut [u8; 32]) -> Result<()> {
        let entry = keyring::Entry::new("campus-portal", "encryption-key").map_err(|_| {
            storage("keyring unavailable; explicitly choose vault.backend=file or passphrase")
        })?;
        let decode = |mut s: String, key: &mut [u8; 32]| -> Result<()> {
            let decoded =
                Zeroizing::new(hex::decode(&s).map_err(|_| storage("invalid keyring key"))?);
            s.zeroize();
            if decoded.len() != 32 {
                return Err(storage("invalid keyring key length"));
            }
            key.copy_from_slice(&decoded);
            Ok(())
        };
        match entry.get_password() {
            Ok(s) => decode(s, key),
            Err(keyring::Error::NoEntry) => {
                let fresh = Zeroizing::new(random::<32>()?);
                let encoded = Zeroizing::new(hex::encode(*fresh));
                entry
                    .set_password(&encoded)
                    .map_err(|_| storage("keyring could not save encryption key"))?;
                // Two first runs can race to create the key; whichever write landed last wins.
                // Re-read so this process never seals secrets under a key that was overwritten.
                match entry.get_password() {
                    Ok(s) => decode(s, key),
                    Err(_) => Err(storage("keyring could not confirm encryption key")),
                }
            }
            Err(_) => Err(storage(
                "keyring locked or unavailable; no fallback was performed",
            )),
        }
    }
    #[cfg(not(feature = "keyring"))]
    fn keyring_key(_key: &mut [u8; 32]) -> Result<()> {
        Err(storage("this build has no keyring support"))
    }
    pub fn seal(&self, context: &str, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let nonce = random::<24>()?;
        let cipher = XChaCha20Poly1305::new((&*self.key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plain,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| storage("secret encryption failed"))?;
        Ok((cipher, nonce.to_vec()))
    }
    pub fn open_secret(
        &self,
        context: &str,
        cipher: &[u8],
        nonce: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        if nonce.len() != 24 {
            return Err(storage("invalid secret nonce"));
        }
        XChaCha20Poly1305::new((&*self.key).into())
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: cipher,
                    aad: context.as_bytes(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| storage("secret authentication failed"))
    }
    pub fn secret_string(&self, db: &Database, name: &str) -> Result<Option<Zeroizing<String>>> {
        #[cfg(feature = "keyring")]
        if self.backend == "keyring" {
            return match keyring::Entry::new("campus-portal", name)
                .map_err(|_| storage("keyring unavailable"))?
                .get_password()
            {
                Ok(s) => Ok(Some(Zeroizing::new(s))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(storage("keyring read failed")),
            };
        }
        db.secret(name)?
            .map(|(c, n)| {
                let b = self.open_secret(name, &c, &n)?;
                String::from_utf8(b.to_vec())
                    .map(Zeroizing::new)
                    .map_err(|_| storage("invalid secret encoding"))
            })
            .transpose()
    }
    pub fn save_secret_string(&self, db: &Database, name: &str, value: Option<&str>) -> Result<()> {
        #[cfg(feature = "keyring")]
        if self.backend == "keyring" {
            let e = keyring::Entry::new("campus-portal", name)
                .map_err(|_| storage("keyring unavailable"))?;
            return match value {
                Some(p) => e
                    .set_password(p)
                    .map_err(|_| storage("keyring write failed")),
                None => match e.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(_) => Err(storage("keyring delete failed")),
                },
            };
        }
        if let Some(p) = value {
            let (c, n) = self.seal(name, p.as_bytes())?;
            db.set_secret(name, Some((&c, &n)))
        } else {
            db.set_secret(name, None)
        }
    }
    pub fn password(&self, db: &Database, user: &str) -> Result<Option<Zeroizing<String>>> {
        self.secret_string(db, &format!("cred:{user}"))
    }
    pub fn save_password(&self, db: &Database, user: &str, pass: Option<&str>) -> Result<()> {
        self.save_secret_string(db, &format!("cred:{user}"), pass)
    }
}

fn load_or_create<const N: usize>(path: &std::path::Path) -> Result<(Zeroizing<[u8; N]>, bool)> {
    match paths::private_options().create_new(true).open(path) {
        Ok(mut f) => {
            let bytes = Zeroizing::new(random::<N>()?);
            f.write_all(&*bytes).map_err(storage)?;
            f.sync_all().map_err(storage)?;
            Ok((bytes, true))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut f = paths::private_file(path)?;
            let mut bytes = Zeroizing::new([0; N]);
            f.read_exact(&mut *bytes)
                .map_err(|_| storage("vault file incomplete or busy"))?;
            if f.metadata().map_err(storage)?.len() != N as u64 {
                return Err(storage("invalid vault file length"));
            }
            Ok((bytes, false))
        }
        Err(e) => Err(storage(e)),
    }
}
