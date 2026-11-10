//! Partially populated H3 regions: mutable shared RAM, coalesced durable snapshots.
use crate::{
    bake::{owner, Block},
    envelope::Node,
    geography::Store,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Region {
    block: Block,
    revision: u64,
}
type SharedRegion = Arc<Mutex<Region>>;
struct Encoder {
    raw: Vec<u8>,
    encoded: Vec<u8>,
    compressor: zstd::bulk::Compressor<'static>,
}
impl Encoder {
    fn new() -> Self {
        Self {
            raw: Vec::with_capacity(64 * 1024),
            encoded: Vec::with_capacity(64 * 1024),
            compressor: zstd::bulk::Compressor::new(3).expect("zstd"),
        }
    }
}
pub struct HexCache {
    store: Arc<Store>,
    blocks: moka::sync::Cache<u64, SharedRegion>,
    locks: moka::sync::Cache<u64, Arc<tokio::sync::Mutex<()>>>,
    pinned: Mutex<HashMap<u64, (SharedRegion, Option<Instant>)>>,
    // Dirty regions stay alive even if the weighted cache evicts them.
    dirty: Mutex<HashMap<u64, SharedRegion>>,
    flush_lock: tokio::sync::Mutex<()>,
    encoder: Arc<Mutex<Encoder>>,
}
fn id(n: Node) -> u64 {
    let (a, b) = n.coords();
    owner(a.to_degrees(), b.to_degrees())
}
fn key(id: u64) -> String {
    format!("fields/{:02x}/{id:x}.zst", (id >> 33) & 255)
}
impl HexCache {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            blocks: moka::sync::Cache::builder()
                .max_capacity(128 * 1024 * 1024)
                .weigher(|_: &u64, b: &SharedRegion| {
                    (b.lock().unwrap().block.samples.capacity() * 24 + 128) as u32
                })
                .build(),
            locks: moka::sync::Cache::builder().max_capacity(4096).build(),
            pinned: Mutex::new(HashMap::new()),
            dirty: Mutex::new(HashMap::new()),
            flush_lock: tokio::sync::Mutex::new(()),
            encoder: Arc::new(Mutex::new(Encoder::new())),
        }
    }
    fn resident(&self, id: u64) -> Option<SharedRegion> {
        self.blocks
            .get(&id)
            .or_else(|| self.dirty.lock().unwrap().get(&id).cloned())
            .or_else(|| {
                self.pinned
                    .lock()
                    .unwrap()
                    .get(&id)
                    .filter(|(_, t)| t.is_none_or(|t| t > Instant::now()))
                    .map(|(b, _)| b.clone())
            })
    }
    pub fn value(&self, n: Node) -> Option<f64> {
        self.resident(id(n))?.lock().unwrap().block.value(n)
    }
    async fn load_one(&self, id: u64) -> Result<SharedRegion, String> {
        if let Some(b) = self.resident(id) {
            return Ok(b);
        }
        let lock = self
            .locks
            .get_with(id, || Arc::new(tokio::sync::Mutex::new(())));
        let _guard = lock.lock().await;
        if let Some(b) = self.resident(id) {
            return Ok(b);
        }
        let mut block = match self.store.read(&key(id)).await? {
            Some(bytes) => tokio::task::spawn_blocking(move || Block::decode(&bytes))
                .await
                .map_err(|e| e.to_string())??,
            None => Block {
                id,
                samples: Vec::with_capacity(256),
            },
        };
        if block.id != id {
            return Err("Hexagon identity mismatch".into());
        }
        block.samples.reserve(256);
        let block = Arc::new(Mutex::new(Region { block, revision: 0 }));
        self.blocks.insert(id, block.clone());
        Ok(block)
    }
    pub async fn load(&self, nodes: &[Node]) -> Result<(), String> {
        let ids: HashSet<_> = nodes.iter().copied().map(id).collect();
        for id in ids {
            self.load_one(id).await?;
        }
        Ok(())
    }
    pub async fn merge(&self, values: &[(Node, f64, f64)]) -> Result<(), String> {
        for &(n, h, k) in values {
            let id = id(n);
            let shared = self.load_one(id).await?;
            let grew = {
                let mut region = shared.lock().unwrap();
                let capacity = region.block.samples.capacity();
                match region.block.samples.binary_search_by_key(&n, |v| v.0) {
                    Ok(i) => region.block.samples[i] = (n, h, k),
                    Err(i) => region.block.samples.insert(i, (n, h, k)),
                }
                region.revision += 1;
                capacity != region.block.samples.capacity()
            };
            self.dirty.lock().unwrap().insert(id, shared.clone());
            if grew {
                self.blocks.insert(id, shared);
            }
        }
        Ok(())
    }
    pub async fn flush(&self) -> Result<usize, String> {
        let _guard = self.flush_lock.lock().await;
        let pending: Vec<_> = self
            .dirty
            .lock()
            .unwrap()
            .iter()
            .map(|(&id, b)| (id, b.clone()))
            .collect();
        let mut saved = 0;
        for (id, region) in pending {
            let shared = region.clone();
            let encoder = self.encoder.clone();
            let (revision, bytes) = tokio::task::spawn_blocking(move || {
                let mut encoder = encoder.lock().unwrap();
                encoder.raw.clear();
                let revision = {
                    let region = shared.lock().unwrap();
                    encoder.raw.extend_from_slice(&id.to_le_bytes());
                    encoder
                        .raw
                        .extend_from_slice(&(region.block.samples.len() as u64).to_le_bytes());
                    for (n, h, k) in &region.block.samples {
                        encoder.raw.extend_from_slice(&n.row.to_le_bytes());
                        encoder.raw.extend_from_slice(&n.col.to_le_bytes());
                        encoder.raw.extend_from_slice(&h.to_le_bytes());
                        encoder.raw.extend_from_slice(&k.to_le_bytes());
                    }
                    region.revision
                };
                let Encoder {
                    raw,
                    encoded,
                    compressor,
                } = &mut *encoder;
                encoded.clear();
                encoded.reserve(zstd::zstd_safe::compress_bound(raw.len()));
                compressor
                    .compress_to_buffer(raw, encoded)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((revision, std::mem::take(encoded)))
            })
            .await
            .map_err(|e| e.to_string())??;
            let result = self.store.put(&key(id), &bytes).await;
            self.encoder.lock().unwrap().encoded = bytes;
            result?;
            // Lock order is dirty then region. A concurrent merge either has a
            // newer revision here, or re-inserts itself after we remove this one.
            let mut dirty = self.dirty.lock().unwrap();
            if region.lock().unwrap().revision == revision {
                dirty.remove(&id);
            }
            saved += 1;
        }
        Ok(saved)
    }
    pub fn pin(&self, nodes: &[Node]) {
        let now = Instant::now();
        let ids: HashSet<_> = nodes.iter().copied().map(id).collect();
        let regions: Vec<_> = ids
            .iter()
            .filter_map(|&id| self.resident(id).map(|b| (id, b)))
            .collect();
        let mut pinned = self.pinned.lock().unwrap();
        for (id, (_, expiry)) in pinned.iter_mut() {
            if ids.contains(id) {
                *expiry = None;
            } else if expiry.is_none() {
                *expiry = Some(now + Duration::from_secs(5));
            }
        }
        pinned.retain(|_, (_, expiry)| expiry.is_none_or(|t| t > now));
        for (id, b) in regions {
            pinned.insert(id, (b, None));
        }
    }
    pub fn stats(&self) -> serde_json::Value {
        self.blocks.run_pending_tasks();
        serde_json::json!({"resident_hexagons":self.blocks.entry_count(),"resident_bytes":self.blocks.weighted_size(),"retained_hexagons":self.pinned.lock().unwrap().len(),"dirty_hexagons":self.dirty.lock().unwrap().len()})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dirty_eviction_and_concurrent_flushes_do_not_lose_samples() {
        let root = std::env::temp_dir().join(format!("hug-dirty-{}", std::process::id()));
        let store = Arc::new(Store::new(root.clone(), true));
        let a = Arc::new(HexCache::new(store.clone()));
        let node = Node {
            row: 40030,
            col: 10,
        };
        a.merge(&[(node, 1., 2.)]).await.unwrap();
        a.blocks.invalidate_all();
        a.blocks.run_pending_tasks();
        assert_eq!(a.value(node), Some(2.));
        let writer = a.clone();
        let task = tokio::spawn(async move {
            for i in 0..100 {
                writer
                    .merge(&[(node, i as f64, i as f64 + 10.)])
                    .await
                    .unwrap();
                tokio::task::yield_now().await;
            }
        });
        for _ in 0..5 {
            a.flush().await.unwrap();
        }
        task.await.unwrap();
        a.flush().await.unwrap();
        let b = HexCache::new(store);
        b.load(&[node]).await.unwrap();
        assert_eq!(b.value(node), Some(109.));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn partial_hexagons_merge_and_survive_a_new_cache_instance() {
        let root = std::env::temp_dir().join(format!("hug-hex-{}", std::process::id()));
        let store = Arc::new(Store::new(root.clone(), true));
        let a = HexCache::new(store.clone());
        let nodes = [Node { row: 40030, col: 0 }, Node { row: 40030, col: 1 }];
        a.merge(&[(nodes[0], 10., 100.)]).await.unwrap();
        assert_eq!(a.value(nodes[0]), Some(100.));
        assert_eq!(a.value(nodes[1]), None);
        a.merge(&[(nodes[1], 20., 200.)]).await.unwrap();
        assert_eq!(a.flush().await.unwrap(), 1);
        assert_eq!(a.flush().await.unwrap(), 0);
        drop(a);
        let b = HexCache::new(store);
        b.load(&nodes).await.unwrap();
        assert_eq!(b.value(nodes[0]), Some(100.));
        assert_eq!(b.value(nodes[1]), Some(200.));
        b.pin(&nodes);
        assert!(b.pinned.lock().unwrap().len() > 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}

static PREPARE_MICROS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(250_000);
pub fn lookahead_seconds() -> f64 {
    (PREPARE_MICROS.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6 * 2.).clamp(0.25, 8.)
}
pub fn observe_preparation(elapsed: Duration) {
    let old = PREPARE_MICROS.load(std::sync::atomic::Ordering::Relaxed);
    let sample = elapsed.as_micros().min(4_000_000) as u64;
    PREPARE_MICROS.store((old * 3 + sample) / 4, std::sync::atomic::Ordering::Relaxed);
}
