use crate::cookie_crypto::hex;
use rand::{rngs::OsRng, RngCore};
use secret_service::{EncryptionType, SecretService};
use std::{collections::HashMap, time::Duration};
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    #[error("Clavis or another Secret Service provider is unavailable; cookies remain in memory")]
    Unavailable,
    #[error("Unlock Clavis and restart RixBrowser to restore persistent cookies; new cookies remain in memory")]
    Locked,
    #[error("The profile cookie key is missing or invalid; existing protected data was preserved")]
    InvalidKey,
    #[error("Create a persistent default collection in Clavis; cookies remain in memory")]
    NotConfigured,
    #[error("The system secret store could not complete the operation; cookies remain in memory")]
    OperationFailed,
    #[error("The system secret store did not respond in time; cookies remain in memory")]
    TimedOut,
}

fn provider_error(error: secret_service::Error) -> SecretStoreError {
    match error {
        secret_service::Error::Locked
        | secret_service::Error::Prompt
        | secret_service::Error::PromptDisconnected => SecretStoreError::Locked,
        secret_service::Error::Unavailable => SecretStoreError::Unavailable,
        secret_service::Error::NoResult => SecretStoreError::NotConfigured,
        secret_service::Error::Zbus(zbus::Error::MethodError(name, _, _)) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner" => SecretStoreError::Unavailable,
            "org.freedesktop.Secret.Error.IsLocked" => SecretStoreError::Locked,
            "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.TimedOut"
            | "org.freedesktop.DBus.Error.Timeout" => SecretStoreError::TimedOut,
            _ => SecretStoreError::OperationFailed,
        },
        secret_service::Error::ZbusFdo(
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_),
        ) => SecretStoreError::Unavailable,
        _ => SecretStoreError::OperationFailed,
    }
}

fn attributes(profile: &str) -> HashMap<&str, &str> {
    HashMap::from([
        ("application", "org.rix.RixBrowser"),
        ("purpose", "cookie-encryption"),
        ("schema", "1"),
        ("profile", profile),
    ])
}

pub(crate) async fn profile_key(
    profile: &[u8; 16],
    protected_rows_exist: bool,
) -> std::result::Result<Zeroizing<[u8; 32]>, SecretStoreError> {
    tokio::time::timeout(
        Duration::from_secs(8),
        acquire_key(profile, protected_rows_exist),
    )
    .await
    .map_err(|_| SecretStoreError::TimedOut)?
}

async fn acquire_key(
    profile: &[u8; 16],
    protected_rows_exist: bool,
) -> std::result::Result<Zeroizing<[u8; 32]>, SecretStoreError> {
    let service = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(provider_error)?;
    let profile_id = hex(profile);
    let found = service
        .search_items(attributes(&profile_id))
        .await
        .map_err(provider_error)?;
    if !found.locked.is_empty() {
        return Err(SecretStoreError::Locked);
    }
    if found.unlocked.len() > 1 {
        return Err(SecretStoreError::InvalidKey);
    }
    if let Some(item) = found.unlocked.first() {
        let bytes = Zeroizing::new(item.get_secret().await.map_err(provider_error)?);
        if bytes.len() != 32 {
            return Err(SecretStoreError::InvalidKey);
        }
        if let Ok(session) = service.get_collection_by_alias("session").await {
            let temporary = session
                .search_items(attributes(&profile_id))
                .await
                .map_err(provider_error)?;
            if temporary
                .iter()
                .any(|temporary| temporary.item_path == item.item_path)
            {
                return Err(SecretStoreError::NotConfigured);
            }
        }
        let mut key = Zeroizing::new([0; 32]);
        key.copy_from_slice(&bytes);
        return Ok(key);
    }
    if protected_rows_exist {
        return Err(SecretStoreError::InvalidKey);
    }
    let collection = service
        .get_default_collection()
        .await
        .map_err(provider_error)?;
    if collection.collection_path.as_str() == "/org/freedesktop/secrets/collection/session"
        || service
            .get_collection_by_alias("session")
            .await
            .ok()
            .is_some_and(|session| session.collection_path == collection.collection_path)
    {
        return Err(SecretStoreError::NotConfigured);
    }
    if collection.is_locked().await.map_err(provider_error)? {
        return Err(SecretStoreError::Locked);
    }
    let mut key = Zeroizing::new([0; 32]);
    OsRng
        .try_fill_bytes(key.as_mut())
        .map_err(|_| SecretStoreError::OperationFailed)?;
    let item = collection
        .create_item(
            "RixBrowser — protected profile cookies",
            attributes(&profile_id),
            key.as_ref(),
            false,
            "application/octet-stream",
        )
        .await
        .map_err(provider_error)?;
    let read_back = Zeroizing::new(item.get_secret().await.map_err(provider_error)?);
    if read_back.as_slice() != key.as_ref() {
        return Err(SecretStoreError::OperationFailed);
    }
    Ok(key)
}
