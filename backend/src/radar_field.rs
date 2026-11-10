//! Coarse, globally resident conservative ranges; ambiguous contacts use fine H.
use h3o::{CellIndex, LatLng, Resolution};
use std::sync::{Arc, OnceLock};
struct Field {
    lo: Vec<i32>,
    hi: Vec<i32>,
}
static FIELD: OnceLock<Arc<Field>> = OnceLock::new();
fn offsets() -> &'static Vec<usize> {
    static O: OnceLock<Vec<usize>> = OnceLock::new();
    O.get_or_init(|| {
        let mut v = vec![0];
        for c in CellIndex::base_cells() {
            v.push(v.last().unwrap() + c.children_count(Resolution::Seven) as usize);
        }
        v
    })
}
fn index(c: CellIndex) -> usize {
    offsets()[usize::from(c.base_cell())] + c.child_position(Resolution::Zero).unwrap() as usize
}
pub fn bounds(lat: f64, lon: f64) -> Option<(f64, f64)> {
    let f = FIELD.get()?;
    let c = LatLng::new(lat, lon).ok()?.to_cell(Resolution::Seven);
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for n in c.grid_disk::<Vec<_>>(2) {
        let i = index(n);
        lo = lo.min(f.lo[i]);
        hi = hi.max(f.hi[i]);
    }
    if lo == i32::MAX {
        None
    } else {
        Some((lo as f64 / 4., hi as f64 / 4.))
    }
}
pub async fn load() -> Result<(), String> {
    let b = crate::geography::Store::shared()
        .read("radar/ranges.zst")
        .await?
        .ok_or("Missing global radar")?;
    let f = tokio::task::spawn_blocking(move || {
        let raw = zstd::decode_all(b.as_slice()).map_err(|e| e.to_string())?;
        let n = *offsets().last().unwrap();
        if raw.len() != n * 8 {
            return Err("Invalid radar size".to_string());
        }
        let mut lo = Vec::with_capacity(n);
        let mut hi = Vec::with_capacity(n);
        for r in raw.chunks_exact(8) {
            let a = i32::from_le_bytes(r[..4].try_into().unwrap());
            let b = i32::from_le_bytes(r[4..].try_into().unwrap());
            if a > b {
                return Err("Unbuilt radar cell".into());
            }
            lo.push(a);
            hi.push(b);
        }
        Ok(Field { lo, hi })
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = FIELD.set(Arc::new(f));
    Ok(())
}
pub async fn build() -> Result<(), String> {
    let n = *offsets().last().unwrap();
    let mut lo = vec![i32::MAX; n];
    let mut hi = vec![i32::MIN; n];
    let store = crate::geography::Store::shared();
    for (i, c) in CellIndex::base_cells()
        .flat_map(|c| c.children(Resolution::Four))
        .enumerate()
    {
        let b = store
            .read(&crate::geography::region_key(u64::from(c)))
            .await?
            .ok_or("Missing region for radar")?;
        let b = crate::bake::Block::decode(&b)?;
        for (node, _, h) in b.samples {
            let (a, b) = node.coords();
            let cell = LatLng::from_radians(a, b)
                .unwrap()
                .to_cell(Resolution::Seven);
            let j = index(cell);
            if !h.is_finite() || h.abs() > 100_000. {
                return Err("Radar height outside lossless range bounds".into());
            }
            lo[j] = lo[j].min((h * 4.).floor() as i32);
            hi[j] = hi[j].max((h * 4.).ceil() as i32);
        }
        if i % 128 == 0 {
            println!("radar regions {i}/288122")
        }
    }
    if lo.iter().any(|v| *v == i32::MAX) {
        return Err("Radar coverage is incomplete".into());
    }
    let mut raw = Vec::with_capacity(n * 8);
    for (a, b) in lo.into_iter().zip(hi) {
        raw.extend_from_slice(&a.to_le_bytes());
        raw.extend_from_slice(&b.to_le_bytes());
    }
    let encoded = tokio::task::spawn_blocking(move || zstd::encode_all(raw.as_slice(), 3))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    store.put("radar/ranges.zst", &encoded).await
}
