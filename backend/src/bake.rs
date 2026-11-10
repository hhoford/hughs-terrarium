use crate::{
    envelope::{Envelope, Node},
    geography::{self, Store},
    hollows,
    topo::TerrainSource,
};
use h3o::{CellIndex, LatLng, Resolution};
use std::sync::Arc;
pub struct Block {
    pub id: u64,
    pub samples: Vec<(Node, f64, f64)>,
}
impl Block {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut b = Vec::with_capacity(16 + 24 * self.samples.len());
        b.extend_from_slice(&self.id.to_le_bytes());
        b.extend_from_slice(&(self.samples.len() as u64).to_le_bytes());
        for (n, h, k) in &self.samples {
            b.extend_from_slice(&n.row.to_le_bytes());
            b.extend_from_slice(&n.col.to_le_bytes());
            b.extend_from_slice(&h.to_le_bytes());
            b.extend_from_slice(&k.to_le_bytes());
        }
        zstd::stream::encode_all(b.as_slice(), 3).map_err(|e| e.to_string())
    }
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        use std::io::Read;
        let mut raw = Vec::new();
        zstd::stream::read::Decoder::new(b)
            .map_err(|e| e.to_string())?
            .take(16 * 1024 * 1024)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
        if raw.len() < 16 {
            return Err("Short region".into());
        }
        let u64at = |i| u64::from_le_bytes(raw[i..i + 8].try_into().unwrap());
        let id = u64at(0);
        let n = u64at(8) as usize;
        if n > 500000 || raw.len() != 16 + n * 24 {
            return Err("Invalid region length".into());
        }
        let mut samples = Vec::with_capacity(n);
        for b in raw[16..].chunks_exact(24) {
            let node = Node {
                row: u32::from_le_bytes(b[..4].try_into().unwrap()),
                col: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            };
            let h = f64::from_le_bytes(b[8..16].try_into().unwrap());
            let k = f64::from_le_bytes(b[16..24].try_into().unwrap());
            if !h.is_finite() || !k.is_finite() {
                return Err("Invalid region values".into());
            }
            samples.push((node, h, k));
        }
        if samples.windows(2).any(|s| s[0].0 >= s[1].0) {
            return Err("Unordered region nodes".into());
        }
        Ok(Self { id, samples })
    }
    pub fn value(&self, n: Node) -> Option<f64> {
        self.samples
            .binary_search_by_key(&n, |v| v.0)
            .ok()
            .map(|i| self.samples[i].2)
    }
}
pub fn owner(lat: f64, lon: f64) -> u64 {
    u64::from(
        LatLng::new(lat, lon)
            .expect("finite coordinates")
            .to_cell(Resolution::Four),
    )
}
pub async fn build(cell: CellIndex, source: &Arc<TerrainSource>) -> Result<Block, String> {
    crate::land::Land::default().prepare(cell).await?;
    let nodes = crate::envelope::region_nodes(cell);
    let mesh = Envelope::builder();
    let raw = mesh.region_inputs(cell, source).await?;
    if raw.values().all(|h| *h == 0.) {
        return Ok(Block {
            id: u64::from(cell),
            samples: nodes.into_iter().map(|n| (n, 0., 0.)).collect(),
        });
    }
    let samples = tokio::task::spawn_blocking(move || crate::envelope::evaluate_region(nodes, raw))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Block {
        id: u64::from(cell),
        samples,
    })
}
pub async fn run() -> Result<(), String> {
    let action = std::env::args().nth(2).unwrap_or_else(|| "status".into());
    let store = Store::shared();
    let source = Arc::new(TerrainSource::new());
    match action.as_str() {
        "pilot" => {
            let sites = [
                (0., -140.),
                (40., -100.),
                (37.7, -122.4),
                (28., 86.),
                (70., 20.),
                (-55., -70.),
                (10., 179.),
            ];
            let limit = std::env::var("HUG_PILOT_REGIONS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(sites.len());
            let mut results = Vec::new();
            for &(lat, lon) in sites.iter().take(limit) {
                let cell = CellIndex::try_from(owner(lat, lon)).map_err(|e| e.to_string())?;
                let t = std::time::Instant::now();
                let existing = if std::env::var_os("HUG_PILOT_REBUILD").is_some() {
                    None
                } else {
                    store.read(&geography::region_key(u64::from(cell))).await?
                };
                let b = if let Some(v) = existing {
                    Block::decode(&v)?
                } else {
                    build(cell, &source).await?
                };
                let encoded = b.encode()?;
                store.put(&geography::region_key(b.id), &encoded).await?;
                let result = serde_json::json!({"region":cell.to_string(),"nodes":b.samples.len(),"bytes":encoded.len(),"seconds":t.elapsed().as_secs_f64()});
                println!("{result}");
                results.push(result);
            }
            store
                .put("pilot.json", &serde_json::to_vec(&results).unwrap())
                .await?;
        }
        "pilot-sources" => source_pilot(&source).await?,
        "radar" => crate::radar_field::build().await?,
        "verify" => verify(&store).await?,
        "atlas" => atlas(&store, source).await?,
        "regions" => regions(&store, &source).await?,
        "all" => {
            atlas(&store, source.clone()).await?;
            regions(&store, &source).await?;
            visual(&store, &source).await?;
            crate::radar_field::build().await?;
            verify(&store).await?;
        }
        "status" => {
            println!("cached sources: {:?}", store.source_counts().await?);
            println!(
                "dataset={} root={} sealed={}",
                geography::VERSION,
                store.root.display(),
                store.path("complete.json").exists()
            );
        }
        _ => return Err("Use --geography pilot|atlas|regions|all|radar|verify|status".into()),
    };
    Ok(())
}
async fn regions(store: &Store, source: &Arc<TerrainSource>) -> Result<(), String> {
    let workers = std::env::var("HUG_BAKE_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(4)
        .clamp(1, 12);
    let mut cells = CellIndex::base_cells().flat_map(|c| c.children(Resolution::Four));
    let mut tasks = tokio::task::JoinSet::new();
    let mut done = 0;
    loop {
        while tasks.len() < workers {
            let Some(cell) = cells.next() else { break };
            let source = source.clone();
            tasks.spawn(async move {
                let store = Store::shared();
                let key = geography::region_key(u64::from(cell));
                if let Some(b) = store.read(&key).await? {
                    Block::decode(&b)?;
                } else {
                    let b = build(cell, &source).await?;
                    store.put(&key, &b.encode()?).await?;
                }
                Ok::<_, String>(cell)
            });
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        let cell = result.map_err(|e| e.to_string())??;
        done += 1;
        println!("regions {done}/288122 {cell}");
    }
    store.put("regions.complete", b"288122").await
}
async fn atlas(store: &Store, source: Arc<TerrainSource>) -> Result<(), String> {
    let tiles = Arc::new(hollows::Tiles::new());
    let mut tasks = tokio::task::JoinSet::new();
    let mut count = 0;
    for z in 0..=8 {
        for x in 0..1u32 << z {
            for y in 0..1u32 << z {
                let tiles = tiles.clone();
                let source = source.clone();
                tasks.spawn(async move { tiles.get(source, z, x, y).await.map(|_| ()) });
                if tasks.len() >= 4 {
                    tasks
                        .join_next()
                        .await
                        .unwrap()
                        .map_err(|e| e.to_string())??;
                    count += 1;
                    if count % 16 == 0 {
                        println!("atlas {count}/87381");
                    }
                }
            }
        }
    }
    while let Some(r) = tasks.join_next().await {
        r.map_err(|e| e.to_string())??;
        count += 1;
    }
    println!("atlas {count}/87381");
    store.put("atlas.complete", b"87381").await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn block_roundtrip() {
        let b = Block {
            id: owner(0., 0.),
            samples: vec![(Node { row: 1, col: 2 }, 1.25, 500.75)],
        };
        let c = Block::decode(&b.encode().unwrap()).unwrap();
        assert_eq!(c.id, b.id);
        assert_eq!(c.value(b.samples[0].0), Some(500.75));
        assert!(Block::decode(b"broken").is_err());
    }
}

async fn verify(store: &Store) -> Result<(), String> {
    let mut count = 0;
    let mut nodes = 0;
    for c in CellIndex::base_cells().flat_map(|c| c.children(Resolution::Four)) {
        let b = store
            .read(&geography::region_key(u64::from(c)))
            .await?
            .ok_or("Incomplete regional coverage")?;
        let b = Block::decode(&b)?;
        if b.id != u64::from(c) {
            return Err("Bad region identity".into());
        }
        nodes += b.samples.len() as u64;
        count += 1;
    }
    if nodes != crate::envelope::info().point_count {
        return Err(format!(
            "Regional node coverage {nodes} differs from canonical lattice"
        ));
    }
    let mut tiles = 0;
    for z in 0..=8 {
        for x in 0..1u32 << z {
            for y in 0..1u32 << z {
                let b = store
                    .read(&format!("hollows/{z}/{x}/{y}.png"))
                    .await?
                    .ok_or("Incomplete hollow atlas")?;
                image::load_from_memory(&b).map_err(|e| e.to_string())?;
                tiles += 1;
            }
        }
    }
    if store.read("visual.complete").await?.is_none() {
        return Err("Visual pyramid incomplete".into());
    }
    crate::radar_field::load().await?;
    let (dem_tiles, water_tiles) = store.verify_sources().await?;
    let manifest = geography::Manifest {
        version: geography::VERSION.into(),
        regions: count,
        nodes,
        hollow_tiles: tiles,
        dem_tiles,
        water_tiles,
        verified: true,
    };
    if !manifest.valid() {
        return Err("Coverage counts do not match the complete dataset".into());
    }
    store.checkpoint().await?;
    store
        .put("complete.json", &serde_json::to_vec(&manifest).unwrap())
        .await
}

async fn visual(store: &Store, source: &Arc<TerrainSource>) -> Result<(), String> {
    let land = Arc::new(crate::land::Land::default());
    let mut tasks = tokio::task::JoinSet::new();
    let mut count = 0u64;
    for z in 0..=12 {
        for x in 0..1u32 << z {
            for y in 0..1u32 << z {
                let land = land.clone();
                let source = source.clone();
                tasks.spawn(async move {
                    land.vector_tile(z, x, y).await?;
                    source.encoded(z, x, y).await?;
                    Ok::<_, String>(())
                });
                if tasks.len() >= 12 {
                    tasks
                        .join_next()
                        .await
                        .unwrap()
                        .map_err(|e| e.to_string())??;
                    count += 1;
                    if count % 4096 == 0 {
                        println!("visual {count}/22369621");
                    }
                }
            }
        }
    }
    while let Some(r) = tasks.join_next().await {
        r.map_err(|e| e.to_string())??;
    }
    store.put("visual.complete", b"complete").await
}

async fn source_pilot(source: &Arc<TerrainSource>) -> Result<(), String> {
    let land = Arc::new(crate::land::Land::default());
    let mut jobs = tokio::task::JoinSet::new();
    let mut results = Vec::new();
    for iy in 0..16u32 {
        for ix in 0..16u32 {
            let source = source.clone();
            let land = land.clone();
            jobs.spawn(async move {
                let x = ix * 256 + 127;
                let y = iy * 256 + 127;
                let d = source.encoded(12, x, y).await?;
                let w = land.vector_tile(12, x, y).await?;
                Ok::<_, String>(
                    serde_json::json!({"x":x,"y":y,"dem_bytes":d.len(),"water_bytes":w.len()}),
                )
            });
            if jobs.len() >= 8 {
                results.push(
                    jobs.join_next()
                        .await
                        .unwrap()
                        .map_err(|e| e.to_string())??,
                );
            }
        }
    }
    while let Some(r) = jobs.join_next().await {
        results.push(r.map_err(|e| e.to_string())??);
    }
    let summary = serde_json::json!({"samples":results,"note":"Equal Mercator tile strata, z12. Source payload sizes, before SQLite overhead."});
    Store::shared()
        .put("source-pilot.json", &serde_json::to_vec(&summary).unwrap())
        .await?;
    println!("{summary}");
    Ok(())
}
