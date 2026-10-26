
use crate::cookie_crypto::{cookie_key, derive_profile_cookie_key, random, CookieCipher};
use crate::error::Result;
use crate::models::{current_time_secs, Bookmark, Cookie, HistoryEntry, SessionState};
use crate::secrets::{profile_key, SecretStoreError};
use crate::StorageError;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use rix_core::url::RixUrl;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const TABLE_METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const TABLE_HISTORY: TableDefinition<u64, &[u8]> = TableDefinition::new("history");
const TABLE_COOKIES: TableDefinition<&str, &[u8]> = TableDefinition::new("cookies");
const TABLE_PROTECTED_COOKIES: TableDefinition<&str, &[u8]> =
    TableDefinition::new("protected_cookies_v1");
const TABLE_SECRET_METADATA: TableDefinition<&str, &[u8]> =
    TableDefinition::new("secret_metadata_v1");
const TABLE_BOOKMARKS: TableDefinition<u64, &[u8]> = TableDefinition::new("bookmarks");
const TABLE_SESSION: TableDefinition<&str, &[u8]> = TableDefinition::new("session");
const TABLE_LOCAL_STORAGE: TableDefinition<&str, &[u8]> = TableDefinition::new("local_storage_v1");

#[derive(Clone)]
pub struct StorageEngine {
    db: Arc<Database>,
    pub(crate) profile_id: [u8; 16],
    cookies: Arc<CookieState>,
}

struct CookieState {
    cipher: Option<CookieCipher>,
    memory: Mutex<HashMap<String, Cookie>>,
}

impl StorageEngine {
    pub fn in_memory() -> Result<Self> {
        let db = Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())?;
        let mut engine = Self {
            db: Arc::new(db),
            profile_id: [0; 16],
            cookies: Arc::new(CookieState {
                cipher: None,
                memory: Mutex::new(HashMap::new()),
            }),
        };
        engine.init_tables()?;
        engine.profile_id = engine.load_profile_id()?;
        Ok(engine)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let db = Database::create(path)?;
        let mut engine = Self {
            db: Arc::new(db),
            profile_id: [0; 16],
            cookies: Arc::new(CookieState {
                cipher: None,
                memory: Mutex::new(HashMap::new()),
            }),
        };
        engine.init_tables()?;
        engine.profile_id = engine.load_profile_id()?;
        Ok(engine)
    }

    pub async fn open_with_system_secrets(path: &Path) -> Result<(Self, Option<SecretStoreError>)> {
        let mut engine = Self::open(path)?;
        let protected_rows_exist = {
            let tx = engine.db.begin_read()?;
            !tx.open_table(TABLE_PROTECTED_COOKIES)?.is_empty()?
        };
        match profile_key(&engine.profile_id, protected_rows_exist).await {
            Ok(key) => match engine.enable_cookie_key(key) {
                Ok(()) => Ok((engine, None)),
                Err(_) => {
                    let fallback_key = derive_profile_cookie_key(&engine.profile_id);
                    if engine.enable_cookie_key(fallback_key).is_ok() {
                        Ok((engine, None))
                    } else {
                        Ok((engine, Some(SecretStoreError::InvalidKey)))
                    }
                }
            },
            Err(SecretStoreError::Locked) => Ok((engine, Some(SecretStoreError::Locked))),
            Err(error) => {
                let fallback_key = derive_profile_cookie_key(&engine.profile_id);
                match engine.enable_cookie_key(fallback_key) {
                    Ok(()) => Ok((engine, None)),
                    Err(_) => Ok((engine, Some(error))),
                }
            }
        }
    }

    pub fn open_persistent(path: &Path) -> Result<Self> {
        let mut engine = Self::open(path)?;
        engine.enable_profile_key()?;
        Ok(engine)
    }

    pub fn enable_profile_key(&mut self) -> Result<()> {
        let fallback_key = derive_profile_cookie_key(&self.profile_id);
        self.enable_cookie_key(fallback_key)
    }

    pub fn cookies_are_protected(&self) -> bool {
        self.cookies.cipher.is_some()
    }

    fn load_profile_id(&self) -> Result<[u8; 16]> {
        let tx = self.db.begin_write()?;
        let id = {
            let mut table = tx.open_table(TABLE_SECRET_METADATA)?;
            let existing = table.get("profile-id")?.map(|value| value.value().to_vec());
            match existing {
                Some(bytes) => bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| StorageError::CookieProtection)?,
                None => {
                    let id = random::<16>()?;
                    table.insert("profile-id", id.as_slice())?;
                    id
                }
            }
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn enable_cookie_key(&mut self, key: Zeroizing<[u8; 32]>) -> Result<()> {
        let cipher = CookieCipher::new(key, self.profile_id);
        let tx = self.db.begin_write()?;
        let mut memory = HashMap::new();
        {
            let mut protected = tx.open_table(TABLE_PROTECTED_COOKIES)?;
            for entry in protected.iter()? {
                let (row, bytes) = entry?;
                memory.insert(
                    row.value().to_owned(),
                    cipher.decode(row.value(), bytes.value())?,
                );
            }
            let mut legacy = tx.open_table(TABLE_COOKIES)?;
            let mut migrated = Vec::new();
            for entry in legacy.iter()? {
                let (old_row, bytes) = entry?;
                if bytes.value().len() > 64 * 1024 {
                    return Err(StorageError::CookieProtection);
                }
                let plain = Zeroizing::new(bytes.value().to_vec());
                let cookie: Cookie =
                    serde_json::from_slice(&plain).map_err(|_| StorageError::CookieProtection)?;
                let row = cookie_key(&cookie)?;
                if !memory.contains_key(&row) {
                    if cookie.expires_at.is_some() {
                        let encrypted = cipher.encode(&row, &cookie)?;
                        if cipher.decode(&row, &encrypted)? != cookie {
                            return Err(StorageError::CookieProtection);
                        }
                        migrated.push((Some((row.clone(), encrypted)), old_row.value().to_owned()));
                    } else {
                        migrated.push((None, old_row.value().to_owned()));
                    }
                    memory.insert(row, cookie);
                } else {
                    migrated.push((None, old_row.value().to_owned()));
                }
            }
            for (replacement, old_row) in migrated {
                if let Some((row, encrypted)) = replacement {
                    protected.insert(row.as_str(), encrypted.as_slice())?;
                }
                legacy.remove(old_row.as_str())?;
            }
        }
        tx.commit()?;
        self.cookies = Arc::new(CookieState {
            cipher: Some(cipher),
            memory: Mutex::new(memory),
        });
        Ok(())
    }

    fn init_tables(&self) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let _ = write_txn.open_table(TABLE_METADATA)?;
            let _ = write_txn.open_table(TABLE_HISTORY)?;
            let _ = write_txn.open_table(TABLE_COOKIES)?;
            let _ = write_txn.open_table(TABLE_PROTECTED_COOKIES)?;
            let _ = write_txn.open_table(TABLE_SECRET_METADATA)?;
            let _ = write_txn.open_table(TABLE_BOOKMARKS)?;
            let _ = write_txn.open_table(TABLE_SESSION)?;
            let _ = write_txn.open_table(TABLE_LOCAL_STORAGE)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    fn next_sequence_id(&self, key: &str) -> Result<u64> {
        let write_txn = self.db.begin_write()?;
        let next_id = {
            let mut table = write_txn.open_table(TABLE_METADATA)?;
            let current = table.get(key)?.map(|v| v.value()).unwrap_or(0);
            let next = current + 1;
            table.insert(key, next)?;
            next
        };
        write_txn.commit()?;
        Ok(next_id)
    }

    pub fn add_history_entry(&self, url: &str, title: &str) -> Result<u64> {
        let now = current_time_secs();
        let write_txn = self.db.begin_write()?;
        let id = {
            let mut meta_table = write_txn.open_table(TABLE_METADATA)?;
            let mut hist_table = write_txn.open_table(TABLE_HISTORY)?;

            let next_id = meta_table
                .get("history_seq")?
                .map(|v| v.value())
                .unwrap_or(0)
                + 1;
            meta_table.insert("history_seq", next_id)?;

            let entry = HistoryEntry {
                id: next_id,
                url: url.to_string(),
                title: title.to_string(),
                visited_at: now,
                visit_count: 1,
            };

            let bytes = serde_json::to_vec(&entry)?;
            hist_table.insert(next_id, bytes.as_slice())?;
            next_id
        };
        write_txn.commit()?;
        Ok(id)
    }

    pub fn get_history(&self, offset: usize, limit: usize) -> Result<Vec<HistoryEntry>> {
        let read_txn = self.db.begin_read()?;
        let hist_table = read_txn.open_table(TABLE_HISTORY)?;

        let mut entries = Vec::new();
        let iter = hist_table.iter()?;

        for item in iter.rev().skip(offset).take(limit) {
            let (_id, bytes) = item?;
            let entry: HistoryEntry = serde_json::from_slice(bytes.value())?;
            entries.push(entry);
        }

        Ok(entries)
    }

    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let read_txn = self.db.begin_read()?;
        let hist_table = read_txn.open_table(TABLE_HISTORY)?;

        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for item in hist_table.iter()?.rev() {
            let (_id, bytes) = item?;
            let entry: HistoryEntry = serde_json::from_slice(bytes.value())?;

            if entry.url.to_lowercase().contains(&query_lower)
                || entry.title.to_lowercase().contains(&query_lower)
            {
                results.push(entry);
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    pub fn clear_history(&self) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut hist_table = write_txn.open_table(TABLE_HISTORY)?;
            let keys: Vec<u64> = hist_table
                .iter()?
                .map(|res| res.map(|(k, _)| k.value()))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for key in keys {
                hist_table.remove(key)?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn set_cookie(&self, cookie: Cookie) -> Result<()> {
        self.update_cookie(cookie, None)
    }

    fn update_cookie(&self, cookie: Cookie, secure_origin: Option<bool>) -> Result<()> {
        if secure_origin == Some(false) && cookie.secure {
            return Ok(());
        }
        let row = cookie_key(&cookie)?;
        let encrypted = match &self.cookies.cipher {
            Some(cipher) if cookie.expires_at.is_some() => cipher.encode(&row, &cookie).map(Some),
            _ => Ok(None),
        };
        let mut memory = self
            .cookies
            .memory
            .lock()
            .map_err(|_| StorageError::CookieProtection)?;
        if secure_origin == Some(false)
            && memory.values().any(|existing| {
                existing.secure
                    && existing.name == cookie.name
                    && existing.domain.trim_start_matches('.')
                        == cookie.domain.trim_start_matches('.')
                    && (cookie.path == existing.path
                        || cookie
                            .path
                            .starts_with(&format!("{}/", existing.path.trim_end_matches('/'))))
            })
        {
            return Ok(());
        }
        if !memory.contains_key(&row) && memory.len() >= 4096 {
            return Err(StorageError::CookieProtection);
        }
        memory.insert(row.clone(), cookie);
        let encrypted = encrypted?;
        if self.cookies.cipher.is_some() {
            let tx = self.db.begin_write()?;
            {
                let mut table = tx.open_table(TABLE_PROTECTED_COOKIES)?;
                match encrypted {
                    Some(bytes) => {
                        table.insert(row.as_str(), bytes.as_slice())?;
                    }
                    None => {
                        table.remove(row.as_str())?;
                    }
                }
            }
            tx.commit()?;
        }
        Ok(())
    }

    pub fn set_cookie_from_response(&self, cookie: Cookie, secure_origin: bool) -> Result<()> {
        self.update_cookie(cookie, Some(secure_origin))
    }

    pub fn get_cookies_for_url(&self, url: &RixUrl) -> Result<Vec<Cookie>> {
        let now = current_time_secs();
        let is_secure = url.scheme() == rix_core::url::RixScheme::Https;
        Ok(self
            .cookies
            .memory
            .lock()
            .map_err(|_| StorageError::CookieProtection)?
            .values()
            .filter(|cookie| cookie.matches_request(url, is_secure, now))
            .cloned()
            .collect())
    }

    pub fn get_all_cookies(&self) -> Result<Vec<Cookie>> {
        let now = current_time_secs();
        Ok(self
            .cookies
            .memory
            .lock()
            .map_err(|_| StorageError::CookieProtection)?
            .values()
            .filter(|cookie| !cookie.is_expired(now))
            .cloned()
            .collect())
    }

    pub fn set_cookies(&self, cookies: impl IntoIterator<Item = Cookie>) -> Result<()> {
        for cookie in cookies {
            self.set_cookie(cookie)?;
        }
        Ok(())
    }

    pub fn delete_expired_cookies(&self) -> Result<usize> {
        let now = current_time_secs();
        let mut memory = self
            .cookies
            .memory
            .lock()
            .map_err(|_| StorageError::CookieProtection)?;
        let expired: Vec<String> = memory
            .iter()
            .filter(|(_, cookie)| cookie.is_expired(now))
            .map(|(row, _)| row.clone())
            .collect();
        if self.cookies.cipher.is_some() {
            let tx = self.db.begin_write()?;
            {
                let mut table = tx.open_table(TABLE_PROTECTED_COOKIES)?;
                for row in &expired {
                    table.remove(row.as_str())?;
                }
            }
            tx.commit()?;
        }
        for row in &expired {
            memory.remove(row);
        }
        Ok(expired.len())
    }

    pub fn clear_cookies(&self) -> Result<()> {
        let mut memory = self
            .cookies
            .memory
            .lock()
            .map_err(|_| StorageError::CookieProtection)?;
        let tx = self.db.begin_write()?;
        for definition in [TABLE_COOKIES, TABLE_PROTECTED_COOKIES] {
            let mut table = tx.open_table(definition)?;
            let keys: Vec<String> = table
                .iter()?
                .map(|item| item.map(|(row, _)| row.value().to_owned()))
                .collect::<std::result::Result<_, _>>()?;
            for row in keys {
                table.remove(row.as_str())?;
            }
        }
        tx.commit()?;
        memory.clear();
        Ok(())
    }

    pub fn add_bookmark(
        &self,
        title: &str,
        url: Option<&str>,
        parent_id: Option<u64>,
        is_folder: bool,
    ) -> Result<u64> {
        let id = self.next_sequence_id("bookmark_seq")?;
        let now = current_time_secs();

        let bookmark = Bookmark {
            id,
            parent_id,
            title: title.to_string(),
            url: url.map(|u| u.to_string()),
            is_folder,
            created_at: now,
        };

        let bytes = serde_json::to_vec(&bookmark)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_BOOKMARKS)?;
            table.insert(id, bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(id)
    }

    pub fn get_bookmarks_in_folder(&self, parent_id: Option<u64>) -> Result<Vec<Bookmark>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_BOOKMARKS)?;

        let mut list = Vec::new();
        for item in table.iter()? {
            let (_id, bytes) = item?;
            let bm: Bookmark = serde_json::from_slice(bytes.value())?;
            if bm.parent_id == parent_id {
                list.push(bm);
            }
        }

        Ok(list)
    }

    pub fn delete_bookmark(&self, id: u64) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_BOOKMARKS)?;
            table.remove(id)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn save_session(&self, session: &SessionState) -> Result<()> {
        let bytes = serde_json::to_vec(session)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_SESSION)?;
            table.insert("active_session", bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn restore_session(&self) -> Result<Option<SessionState>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_SESSION)?;

        match table.get("active_session")? {
            Some(bytes) => {
                let state: SessionState = serde_json::from_slice(bytes.value())?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    fn storage_composite_key(origin: &str, key: &str) -> String {
        format!("{origin}\0{key}")
    }

    pub fn set_local_storage(&self, origin: &str, key: &str, value: &str) -> Result<()> {
        let composite = Self::storage_composite_key(origin, key);
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_LOCAL_STORAGE)?;
            table.insert(composite.as_str(), value.as_bytes())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_local_storage(&self, origin: &str, key: &str) -> Result<Option<String>> {
        let composite = Self::storage_composite_key(origin, key);
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_LOCAL_STORAGE)?;
        let val = table.get(composite.as_str())?.map(|guard| {
            String::from_utf8_lossy(guard.value()).into_owned()
        });
        Ok(val)
    }

    pub fn remove_local_storage(&self, origin: &str, key: &str) -> Result<()> {
        let composite = Self::storage_composite_key(origin, key);
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_LOCAL_STORAGE)?;
            let _ = table.remove(composite.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn clear_local_storage(&self, origin: &str) -> Result<()> {
        let prefix = format!("{origin}\0");
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_LOCAL_STORAGE)?;
            let keys_to_delete: Vec<String> = table
                .iter()?
                .filter_map(|item| item.ok())
                .filter(|(k, _)| k.value().starts_with(&prefix))
                .map(|(k, _)| k.value().to_string())
                .collect();

            for k in keys_to_delete {
                let _ = table.remove(k.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_all_local_storage(&self, origin: &str) -> Result<HashMap<String, String>> {
        let prefix = format!("{origin}\0");
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_LOCAL_STORAGE)?;
        let mut map = HashMap::new();
        for item in table.iter()? {
            let (k, v) = item?;
            let k_str = k.value();
            if k_str.starts_with(&prefix) {
                let key = &k_str[prefix.len()..];
                let val = String::from_utf8_lossy(v.value()).into_owned();
                map.insert(key.to_string(), val);
            }
        }
        Ok(map)
    }
}
