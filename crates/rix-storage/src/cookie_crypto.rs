use crate::{Cookie, Result, StorageError};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const PREFIX: &[u8] = b"RC1";
const NONCE_LEN: usize = 24;
const MAX_COOKIE_BYTES: usize = 64 * 1024;

pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| StorageError::CookieProtection)?;
    Ok(bytes)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

pub(crate) fn cookie_key(cookie: &Cookie) -> Result<String> {
    let identity = serde_json::to_vec(&(&cookie.domain, &cookie.path, &cookie.name))?;
    Ok(hex(&Sha256::digest(identity)))
}

pub(crate) fn derive_profile_cookie_key(profile_id: &[u8; 16]) -> Zeroizing<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(b"org.rix.RixBrowser.cookie.fallback-key.v1\0");
    hasher.update(profile_id);
    let key: [u8; 32] = hasher.finalize().into();
    Zeroizing::new(key)
}

pub(crate) struct CookieCipher {
    key: Zeroizing<[u8; 32]>,
    profile: [u8; 16],
}

impl CookieCipher {
    pub(crate) fn new(key: Zeroizing<[u8; 32]>, profile: [u8; 16]) -> Self {
        Self { key, profile }
    }

    fn aad(&self, row: &str) -> Vec<u8> {
        let mut aad = b"org.rix.RixBrowser.cookie.v1\0".to_vec();
        aad.extend_from_slice(&self.profile);
        aad.extend_from_slice(row.as_bytes());
        aad
    }

    pub(crate) fn encode(&self, row: &str, cookie: &Cookie) -> Result<Vec<u8>> {
        let plain = Zeroizing::new(serde_json::to_vec(cookie)?);
        if plain.len() > MAX_COOKIE_BYTES {
            return Err(StorageError::CookieProtection);
        }
        let nonce = random::<NONCE_LEN>()?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| StorageError::CookieProtection)?;
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &self.aad(row),
                },
            )
            .map_err(|_| StorageError::CookieProtection)?;
        let mut bytes = Vec::with_capacity(PREFIX.len() + nonce.len() + encrypted.len());
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&encrypted);
        Ok(bytes)
    }

    pub(crate) fn decode(&self, row: &str, bytes: &[u8]) -> Result<Cookie> {
        if !bytes.starts_with(PREFIX)
            || bytes.len() < PREFIX.len() + NONCE_LEN + 16
            || bytes.len() > MAX_COOKIE_BYTES + PREFIX.len() + NONCE_LEN + 16
        {
            return Err(StorageError::CookieProtection);
        }
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| StorageError::CookieProtection)?;
        let nonce = &bytes[PREFIX.len()..PREFIX.len() + NONCE_LEN];
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(nonce),
                    Payload {
                        msg: &bytes[PREFIX.len() + NONCE_LEN..],
                        aad: &self.aad(row),
                    },
                )
                .map_err(|_| StorageError::CookieProtection)?,
        );
        let cookie: Cookie =
            serde_json::from_slice(&plain).map_err(|_| StorageError::CookieProtection)?;
        if cookie_key(&cookie)? != row {
            return Err(StorageError::CookieProtection);
        }
        Ok(cookie)
    }
}
