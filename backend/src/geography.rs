//! Versioned durable geography. Game state never enters this store.
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
pub const VERSION: &str = "geography-v1-equal250-gauss5000-cut20000-hollows1";
static SERIAL: AtomicU64 = AtomicU64::new(0);
static DOWNLOADS: AtomicU64 = AtomicU64::new(0);
pub struct Store {
    ram_hits: AtomicU64,
    disk_hits: AtomicU64,
    generated: AtomicU64,
    evicted: AtomicU64,
    touches: std::sync::Mutex<std::collections::HashSet<String>>,
    pub root: PathBuf,
    pub build: bool,
    db: OnceLock<Arc<std::sync::Mutex<rusqlite::Connection>>>,
    database_init: std::sync::Mutex<()>,
    cache: moka::future::Cache<String, Arc<Vec<u8>>>,
}
impl Store {
    pub fn shared() -> Arc<Self> {
        static S: OnceLock<Arc<Store>> = OnceLock::new();
        S.get_or_init(|| {
            Arc::new(Self::new(
                std::env::var_os("HUG_GEOGRAPHY_ROOT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(".geography-cache")),
                match std::env::var("HUG_GEOGRAPHY_MODE").as_deref() {
                    Ok("build") | Ok("cache") => true,
                    Ok("serve") => false,
                    Err(_) => true,
                    _ => panic!("HUG_GEOGRAPHY_MODE must be cache, build or serve"),
                },
            ))
        })
        .clone()
    }
    pub fn new(root: PathBuf, build: bool) -> Self {
        Self {
            ram_hits: AtomicU64::new(0),
            disk_hits: AtomicU64::new(0),
            generated: AtomicU64::new(0),
            evicted: AtomicU64::new(0),
            touches: Default::default(),
            root: root.join(VERSION),
            build,
            db: OnceLock::new(),
            database_init: std::sync::Mutex::new(()),
            cache: moka::future::Cache::builder()
                .max_capacity(64 * 1024 * 1024)
                .weigher(|_: &String, v: &Arc<Vec<u8>>| {
                    v.len().saturating_add(128).min(u32::MAX as usize) as u32
                })
                .build(),
        }
    }
    fn database(&self) -> Result<Arc<std::sync::Mutex<rusqlite::Connection>>, String> {
        if let Some(db) = self.db.get() {
            return Ok(db.clone());
        }
        let _initializing = self.database_init.lock().unwrap();
        if let Some(db) = self.db.get() {
            return Ok(db.clone());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let flags = if self.build {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI
        };
        let database_path = self.root.join("sources.sqlite");
        let c = if self.build {
            rusqlite::Connection::open_with_flags(&database_path, flags)
        } else {
            rusqlite::Connection::open_with_flags(
                format!("file:{}?immutable=1", database_path.display()),
                flags,
            )
        }
        .map_err(|e| e.to_string())?;
        c.busy_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| e.to_string())?;
        if self.build {
            c.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS blobs(key TEXT PRIMARY KEY,data BLOB NOT NULL) WITHOUT ROWID;").map_err(|e|e.to_string())?;
        }
        if self.build {
            let has_column = {
                let mut q = c
                    .prepare("PRAGMA table_info(blobs)")
                    .map_err(|e| e.to_string())?;
                let columns = q
                    .query_map([], |r| r.get::<_, String>(1))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                columns.iter().any(|n| n == "last_used")
            };
            if !has_column {
                c.execute_batch(
                    "ALTER TABLE blobs ADD COLUMN last_used INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| e.to_string())?;
            }
            c.execute_batch("CREATE INDEX IF NOT EXISTS blobs_last_used ON blobs(last_used);")
                .map_err(|e| e.to_string())?;
        }
        let _ = self.db.set(Arc::new(std::sync::Mutex::new(c)));
        Ok(self.db.get().unwrap().clone())
    }
    pub fn path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
    pub async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let result = if key.starts_with("dem/") || key.starts_with("water/") {
            let db = self.database()?;
            let key = key.to_owned();
            let v = tokio::task::spawn_blocking(move || {
                use rusqlite::OptionalExtension;
                db.lock()
                    .unwrap()
                    .query_row("SELECT data FROM blobs WHERE key=?1", [key], |r| {
                        r.get::<_, Vec<u8>>(0)
                    })
                    .optional()
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())??;
            v.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        } else {
            tokio::fs::read(self.path(key)).await
        };
        match result {
            Ok(b) => {
                if b.len() < 40 || &b[..8] != b"HUGGEO01" {
                    return Err(format!("Invalid geography artifact {key}"));
                }
                let digest = Sha256::digest(&b[40..]);
                if digest.as_slice() != &b[8..40] {
                    return Err(format!("Corrupt geography artifact {key}"));
                }
                self.disk_hits.fetch_add(1, Ordering::Relaxed);
                self.touch(key);
                Ok(Some(b[40..].to_vec()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub async fn put(&self, key: &str, data: &[u8]) -> Result<(), String> {
        if !self.build {
            return Err("Read-only geography dataset".into());
        }
        if key.starts_with("dem/") || key.starts_with("water/") {
            let mut b = Vec::with_capacity(40 + data.len());
            b.extend_from_slice(b"HUGGEO01");
            b.extend_from_slice(&Sha256::digest(data));
            b.extend_from_slice(data);
            let write_key = key.to_owned();
            let db = self.database()?;
            tokio::task::spawn_blocking(move || {
                db.lock()
                    .unwrap()
                    .execute(
                        "INSERT OR REPLACE INTO blobs(key,data,last_used) VALUES(?1,?2,unixepoch())",
                        rusqlite::params![write_key, b],
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())??;
            if self.cache.contains_key(key) {
                self.cache.invalidate(key).await;
            }
            return Ok(());
        }
        let path = self.path(key);
        let parent = path.parent().ok_or("Invalid artifact path")?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
        let tmp = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| e.to_string())?;
        f.write_all(b"HUGGEO01").await.map_err(|e| e.to_string())?;
        f.write_all(&Sha256::digest(data))
            .await
            .map_err(|e| e.to_string())?;
        f.write_all(data).await.map_err(|e| e.to_string())?;
        f.sync_all().await.map_err(|e| e.to_string())?;
        drop(f);
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| e.to_string())?;
        // Persist the rename as well as the payload before declaring a completed artifact.
        let p = parent.to_owned();
        tokio::task::spawn_blocking(move || std::fs::File::open(p)?.sync_all())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if self.cache.contains_key(key) {
            self.cache.invalidate(key).await;
        }
        Ok(())
    }
    pub async fn get<F, Fut>(&self, key: String, make: F) -> Result<Arc<Vec<u8>>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, String>>,
    {
        self.touch(&key);
        if let Some(value) = self.cache.get(&key).await {
            self.ram_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(value);
        }
        self.cache
            .try_get_with(key.clone(), async {
                match self.read(&key).await {
                    Ok(Some(v)) => return Ok(Arc::new(v)),
                    Ok(None) => {}
                    Err(e) if !self.build => return Err(e),
                    Err(_) => {}
                }
                if !self.build {
                    return Err(format!("Incomplete geography dataset: {key}"));
                }
                let v = make().await?;
                self.generated.fetch_add(1, Ordering::Relaxed);
                self.put(&key, &v).await?;
                Ok(Arc::new(v))
            })
            .await
            .map_err(|e: Arc<String>| (*e).clone())
    }
}
impl Store {
    pub fn touch(&self, key: &str) {
        if key.starts_with("dem/") || key.starts_with("water/") {
            self.touches.lock().unwrap().insert(key.into());
        }
    }
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({"ram_hits":self.ram_hits.load(Ordering::Relaxed),"disk_hits":self.disk_hits.load(Ordering::Relaxed),"generated":self.generated.load(Ordering::Relaxed),"external_downloads":DOWNLOADS.load(Ordering::Relaxed),"evicted_source_tiles":self.evicted.load(Ordering::Relaxed),"source_limit_mib":source_limit()/1024/1024})
    }
    pub fn start_maintenance() {
        let store = Self::shared();
        if !store.build {
            return;
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                if let Err(e) = store.maintain(source_limit()).await {
                    tracing::warn!(%e,"Source cache maintenance failed");
                }
            }
        });
    }
    pub async fn maintain(&self, budget: u64) -> Result<(), String> {
        let touches: Vec<_> = self.touches.lock().unwrap().drain().collect();
        let db = self.database()?;
        let count=tokio::task::spawn_blocking(move||{
   let mut c=db.lock().unwrap();let transaction=c.transaction().map_err(|e|e.to_string())?;
   {let mut q=transaction.prepare_cached("UPDATE blobs SET last_used=unixepoch() WHERE key=?1").map_err(|e|e.to_string())?;for key in touches{q.execute([key]).map_err(|e|e.to_string())?;}}
   let mut bytes:u64=transaction.query_row("SELECT coalesce(sum(length(data)),0) FROM blobs",[],|r|r.get(0)).map_err(|e|e.to_string())?;
   let mut removed=0;
   while bytes>budget {
    let candidates={let mut q=transaction.prepare("SELECT key,length(data) FROM blobs WHERE key!='water/source.json' ORDER BY last_used,key LIMIT 256").map_err(|e|e.to_string())?;let values=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;values};
    if candidates.is_empty(){break}for(key,size)in candidates{if bytes<=budget{break}transaction.execute("DELETE FROM blobs WHERE key=?1",[key]).map_err(|e|e.to_string())?;bytes=bytes.saturating_sub(size);removed+=1;}
   }transaction.commit().map_err(|e|e.to_string())?;Ok::<u64,String>(removed)
  }).await.map_err(|e|e.to_string())??;
        self.evicted.fetch_add(count, Ordering::Relaxed);
        Ok(())
    }
}
fn source_limit() -> u64 {
    std::env::var("HUG_SOURCE_CACHE_MIB")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8192)
        .max(1)
        * 1024
        * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn source_eviction_preserves_recent_tiles_and_derived_geography() {
        let root = std::env::temp_dir().join(format!(
            "hug-cache-budget-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::new(root.clone(), true);
        for key in ["dem/1/0/0.png", "dem/1/0/1.png", "dem/1/1/0.png"] {
            store.put(key, &[7; 100]).await.unwrap();
        }
        store
            .put("fields/example", b"keep calculated data")
            .await
            .unwrap();
        {
            let db = store.database().unwrap();
            db.lock()
                .unwrap()
                .execute("UPDATE blobs SET last_used=0", [])
                .unwrap();
        }
        assert!(store.read("dem/1/0/0.png").await.unwrap().is_some());
        store.maintain(280).await.unwrap();
        assert!(store.read("dem/1/0/0.png").await.unwrap().is_some());
        assert_eq!(store.source_counts().await.unwrap().0, 2);
        assert_eq!(
            store.read("fields/example").await.unwrap().unwrap(),
            b"keep calculated data"
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn sqlite_sources_survive_checkpoint_and_read_only_reopen() {
        let root = std::env::temp_dir().join(format!(
            "hug-geo-sqlite-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let s = Store::new(root.clone(), true);
        s.put("dem/0/0/0.png", b"encoded source").await.unwrap();
        s.checkpoint().await.unwrap();
        let t = Store::new(root.clone(), false);
        assert_eq!(
            t.read("dem/0/0/0.png").await.unwrap().unwrap(),
            b"encoded source"
        );
        assert!(t.put("dem/0/0/0.png", b"overwrite").await.is_err());
        drop(t);
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn concurrent_misses_share_one_generation_and_failures_are_retriable() {
        let root = std::env::temp_dir().join(format!(
            "hug-geo-flight-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Arc::new(Store::new(root.clone(), true));
        let calls = Arc::new(AtomicU64::new(0));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..32 {
            let s = store.clone();
            let c = calls.clone();
            tasks.spawn(async move {
                s.get("shared".into(), || async move {
                    c.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    Ok(b"once".to_vec())
                })
                .await
                .unwrap()
            });
        }
        while let Some(r) = tasks.join_next().await {
            assert_eq!(r.unwrap().as_slice(), b"once");
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(store
            .get("retry".into(), || async { Err("upstream failed".into()) })
            .await
            .is_err());
        assert_eq!(
            store
                .get("retry".into(), || async { Ok(b"recovered".to_vec()) })
                .await
                .unwrap()
                .as_slice(),
            b"recovered"
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn an_incomplete_or_corrupt_manifest_cannot_enable_serving() {
        let root = std::env::temp_dir().join(format!(
            "hug-geo-seal-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::new(root.clone(), true);
        store
            .put("complete.json", br#"{"verified":true}"#)
            .await
            .unwrap();
        assert!(!valid_dataset(&root));
        let manifest = Manifest {
            version: VERSION.into(),
            regions: 288122,
            nodes: crate::envelope::info().point_count,
            hollow_tiles: 87381,
            dem_tiles: 22369621,
            water_tiles: 22369621,
            verified: true,
        };
        store
            .put("complete.json", &serde_json::to_vec(&manifest).unwrap())
            .await
            .unwrap();
        assert!(valid_dataset(&root));
        std::fs::write(store.path("complete.json"), b"broken").unwrap();
        assert!(!valid_dataset(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn persists_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!(
            "hug-geo-test-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let s = Store::new(root.clone(), true);
        s.put("test/a", b"terrain").await.unwrap();
        let t = Store::new(root.clone(), false);
        assert_eq!(
            t.get("test/a".into(), || async { panic!("must not regenerate") })
                .await
                .unwrap()
                .as_slice(),
            b"terrain"
        );
        assert!(t
            .get("missing".into(), || async { Ok(vec![]) })
            .await
            .is_err());
        tokio::fs::write(t.path("test/a"), b"broken").await.unwrap();
        assert!(t.read("test/a").await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
pub fn region_key(id: u64) -> String {
    format!("regions/{:02x}/{id:015x}.zst", (id >> 33) & 255)
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub regions: u64,
    pub nodes: u64,
    pub hollow_tiles: u64,
    pub dem_tiles: u64,
    pub water_tiles: u64,
    pub verified: bool,
}
impl Manifest {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.regions == 288122
            && self.nodes == crate::envelope::info().point_count
            && self.hollow_tiles == 87381
            && self.dem_tiles == 22369621
            && self.water_tiles == 22369621
            && self.verified
    }
}
#[cfg(test)]
pub fn valid_dataset(root: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read(root.join(VERSION).join("complete.json")) else {
        return false;
    };
    if bytes.len() < 40
        || &bytes[..8] != b"HUGGEO01"
        || Sha256::digest(&bytes[40..]).as_slice() != &bytes[8..40]
    {
        return false;
    }
    serde_json::from_slice::<Manifest>(&bytes[40..]).is_ok_and(|m| m.valid())
}
impl Store {
    pub async fn checkpoint(&self) -> Result<(), String> {
        let db = self.database()?;
        tokio::task::spawn_blocking(move || {
            let c = db.lock().unwrap();
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub async fn verify_sources(&self) -> Result<(u64, u64), String> {
        let db = self.database()?;
        tokio::task::spawn_blocking(move || {
            let c = db.lock().unwrap();
            let mut query = c
                .prepare(
                    "SELECT key,data FROM blobs WHERE key LIKE 'dem/%' OR key LIKE 'water/%.pbf'",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = query.query([]).map_err(|e| e.to_string())?;
            let (mut dem, mut water) = (0, 0);
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let key: String = row.get(0).map_err(|e| e.to_string())?;
                let bytes: Vec<u8> = row.get(1).map_err(|e| e.to_string())?;
                if bytes.len() < 40
                    || &bytes[..8] != b"HUGGEO01"
                    || Sha256::digest(&bytes[40..]).as_slice() != &bytes[8..40]
                {
                    return Err(format!("Corrupt source {key}"));
                }
                let parts: Vec<_> = key.split('/').collect();
                if parts.len() != 4 {
                    return Err(format!("Invalid source key {key}"));
                }
                let z = parts[1].parse::<u32>().map_err(|e| e.to_string())?;
                let x = parts[2].parse::<u32>().map_err(|e| e.to_string())?;
                let y = parts[3]
                    .split('.')
                    .next()
                    .unwrap()
                    .parse::<u32>()
                    .map_err(|e| e.to_string())?;
                if z > 12 || x >= 1 << z || y >= 1 << z {
                    return Err(format!("Invalid source coordinate {key}"));
                }
                if parts[0] == "dem" {
                    dem += 1
                } else {
                    water += 1
                }
                if (dem + water) % 1_000_000 == 0 {
                    println!("verify sources {}", dem + water);
                }
            }
            Ok((dem, water))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub async fn source_counts(&self) -> Result<(u64, u64), String> {
        let db = self.database()?;
        tokio::task::spawn_blocking(move || {
            let c = db.lock().unwrap();
            let dem = c
                .query_row(
                    "SELECT count(*) FROM blobs WHERE key LIKE 'dem/%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let water = c
                .query_row(
                    "SELECT count(*) FROM blobs WHERE key LIKE 'water/%.pbf'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok((dem, water))
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

pub async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut error = String::new();
    for attempt in 0..4 {
        let result = async {
            let r = client
                .get(url)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?;
            r.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(v) => {
                DOWNLOADS.fetch_add(1, Ordering::Relaxed);
                return Ok(v);
            }
            Err(e) => error = e,
        }
        if attempt < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(250 * (1 << attempt))).await;
        }
    }
    Err(error)
}

// Bytes owns the shared cached payload until the response is sent, without copying it.
struct SharedPayload(Arc<Vec<u8>>);
impl AsRef<[u8]> for SharedPayload {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
pub fn response_bytes(value: Arc<Vec<u8>>) -> axum::body::Bytes {
    axum::body::Bytes::from_owner(SharedPayload(value))
}
