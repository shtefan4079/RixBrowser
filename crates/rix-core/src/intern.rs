
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock};
use std::fmt;

#[derive(Clone)]
pub struct InternedStr(Arc<str>);

pub type InternedString = InternedStr;

static POOL: OnceLock<Mutex<HashMap<String, Arc<str>>>> = OnceLock::new();

fn pool() -> &'static Mutex<HashMap<String, Arc<str>>> {
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn intern(s: &str) -> InternedStr {
    let guard = pool().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(s) {
        return InternedStr(existing.clone());
    }
    drop(guard);
    let mut guard = pool().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(s) {
        return InternedStr(existing.clone());
    }
    let arc: Arc<str> = Arc::from(s);
    guard.insert(s.to_string(), arc.clone());
    InternedStr(arc)
}

pub fn intern_str(s: &str) -> InternedStr {
    intern(s)
}

pub fn intern_string(s: String) -> InternedStr {
    intern(&s)
}

impl InternedStr {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new(s: &str) -> Self {
        intern(s)
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for InternedStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for InternedStr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for InternedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for InternedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq for InternedStr {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl PartialEq<str> for InternedStr {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for InternedStr {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for InternedStr {
    fn eq(&self, other: &String) -> bool {
        &*self.0 == other.as_str()
    }
}

impl Eq for InternedStr {}

impl Hash for InternedStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialOrd for InternedStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InternedStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl From<&str> for InternedStr {
    fn from(s: &str) -> Self {
        intern(s)
    }
}

impl From<String> for InternedStr {
    fn from(s: String) -> Self {
        intern(&s)
    }
}

impl From<Arc<str>> for InternedStr {
    fn from(s: Arc<str>) -> Self {
        intern(&s)
    }
}
