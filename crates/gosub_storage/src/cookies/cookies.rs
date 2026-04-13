use crate::cookies::cookie_jar::CookieJar;
use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::store::CookieStore;
use gosub_net::types::ZoneId;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Clone, Debug)]
pub struct CookieJarHandle(Arc<RwLock<Box<dyn CookieJar + Send + Sync>>>);

impl Debug for dyn CookieJar + Send + Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieJar {{ ... }}")
    }
}

impl CookieJarHandle {
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        Arc::ptr_eq(&this.0, &other.0)
    }
}

impl PartialEq for CookieJarHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for CookieJarHandle {}

impl CookieJarHandle {
    pub fn new<T>(jar: T) -> Self
    where
        T: CookieJar + Send + Sync + 'static,
    {
        Self(Arc::new(RwLock::new(Box::new(jar))))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Box<dyn CookieJar + Send + Sync>> {
        self.0.read().expect("poisoned CookieJarHandle")
    }
    pub fn write(&self) -> RwLockWriteGuard<'_, Box<dyn CookieJar + Send + Sync>> {
        self.0.write().expect("poisoned CookieJarHandle")
    }
}

impl Deref for CookieJarHandle {
    type Target = RwLock<Box<dyn CookieJar + Send + Sync>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Box<dyn CookieJar + Send + Sync>> for CookieJarHandle {
    fn from(jar: Box<dyn CookieJar + Send + Sync>) -> Self {
        Self(Arc::new(RwLock::new(jar)))
    }
}

impl<T> From<T> for CookieJarHandle
where
    T: CookieJar + Send + Sync + 'static,
{
    fn from(jar: T) -> Self {
        Self::new(jar)
    }
}

pub struct CookieStoreHandle(Arc<dyn CookieStore + Send + Sync>);

impl Clone for CookieStoreHandle {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }

    fn clone_from(&mut self, source: &Self)
    where
        Self:,
    {
        self.0.clone_from(&source.0);
    }
}

impl<T> From<Arc<T>> for CookieStoreHandle
where
    T: CookieStore + Send + Sync + 'static,
{
    fn from(a: Arc<T>) -> Self {
        Self(a)
    }
}

impl Debug for CookieStoreHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieStore {{ ... }}")
    }
}

impl CookieStoreHandle {
    pub fn persist_zone_from_snapshot(&self, zone: ZoneId, snap: &DefaultCookieJar) {
        self.0.persist_zone_from_snapshot(zone, snap);
    }
    pub fn remove_zone(&self, zone: ZoneId) {
        self.0.remove_zone(zone);
    }
    pub fn persist_all(&self) {
        self.0.persist_all();
    }
    pub fn jar_for(&self, zone: ZoneId) -> Option<CookieJarHandle> {
        self.0.jar_for(zone)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub path: Option<String>,
    pub domain: Option<String>,
    pub secure: bool,
    pub expires: Option<String>,
    pub same_site: Option<String>,
    pub http_only: bool,
}
