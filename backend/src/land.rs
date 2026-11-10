//! Shared public OpenMapTiles water classification, independent of elevation.
use geo::{Geometry, Intersects, Point};
use geozero::ToGeo;
use prost::Message;
use std::sync::Arc;

pub struct Land {
    tiles: moka::sync::Cache<(u32, u32), Arc<(f64, Vec<Geometry>)>>,
}
impl Default for Land {
    fn default() -> Self {
        Self {
            tiles: moka::sync::Cache::builder()
                .max_capacity(64 * 1024 * 1024)
                .weigher(|_: &(u32, u32), v: &Arc<(f64, Vec<Geometry>)>| {
                    use geo::CoordsIter;
                    v.1.iter()
                        .map(|g| g.coords_count() * 16)
                        .sum::<usize>()
                        .saturating_add(128)
                        .min(u32::MAX as usize) as u32
                })
                .build(),
        }
    }
}
impl Land {
    pub async fn is_land(&self, lat: f64, lon: f64) -> Result<bool, String> {
        match self.classify(lat, lon).await {
            Ok(v) => Ok(v),
            Err(e) if e == "Invalid position" => Err(e),
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                self.classify(lat, lon).await
            }
        }
    }
    pub(crate) async fn encoded(&self, x: u32, y: u32) -> Result<Arc<Vec<u8>>, String> {
        self.vector_tile(12, x, y).await
    }
    pub(crate) async fn vector_tile(&self, z: u32, x: u32, y: u32) -> Result<Arc<Vec<u8>>, String> {
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let client = CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("hug-gamer-terrain/0.1")
                .timeout(std::time::Duration::from_secs(25))
                .build()
                .expect("Map HTTP client")
        });
        crate::geography::Store::shared()
            .get(format!("water/{z}/{x}/{y}.pbf"), || async {
                let meta = crate::geography::Store::shared()
                    .get("water/source.json".into(), || async {
                        let bytes = crate::geography::download(
                            client,
                            "https://tiles.openfreemap.org/planet/latest",
                        )
                        .await?;
                        let _: serde_json::Value =
                            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                        Ok(bytes)
                    })
                    .await?;
                let meta: serde_json::Value =
                    serde_json::from_slice(&meta).map_err(|e| e.to_string())?;
                let url = meta["tiles"][0]
                    .as_str()
                    .ok_or("Missing water source")?
                    .replace("{z}", &z.to_string())
                    .replace("{x}", &x.to_string())
                    .replace("{y}", &y.to_string());
                let bytes = crate::geography::download(client, &url).await?;
                let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
                    use std::io::Read;
                    let mut out = Vec::new();
                    flate2::read::GzDecoder::new(bytes.as_slice())
                        .read_to_end(&mut out)
                        .map_err(|e| e.to_string())?;
                    out
                } else {
                    bytes
                };
                let mut tile =
                    geozero::mvt::Tile::decode(raw.as_slice()).map_err(|e| e.to_string())?;
                tile.layers
                    .retain(|l| l.name == "water" || l.name == "waterway");
                Ok(tile.encode_to_vec())
            })
            .await
    }
    pub(crate) async fn prepare(&self, cell: h3o::CellIndex) -> Result<(), String> {
        let c = h3o::LatLng::from(cell);
        let to_xy = |p: h3o::LatLng| {
            (
                (p.lng() + 180.) / 360. * 4096.,
                (1. - p.lat_radians().tan().asinh() / std::f64::consts::PI) * 2048.,
            )
        };
        let (cx, _) = to_xy(c);
        let mut minx = cx;
        let mut maxx = cx;
        let mut miny = 4095f64;
        let mut maxy = 0f64;
        for p in cell.boundary().iter() {
            let (x, y) = to_xy(*p);
            let x = cx + (x - cx + 2048.).rem_euclid(4096.) - 2048.;
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
        if c.lat().abs() > 83. {
            return Ok(());
        }
        let mut tasks = tokio::task::JoinSet::new();
        let slots = Arc::new(tokio::sync::Semaphore::new(8));
        for y in ((miny.floor() as i32) - 1).max(0)..=((maxy.ceil() as i32) + 1).min(4095) {
            for x in (minx.floor() as i32) - 1..=(maxx.ceil() as i32) + 1 {
                let x = x.rem_euclid(4096) as u32;
                let slots = slots.clone();
                tasks.spawn(async move {
                    let _p = slots.acquire().await.map_err(|e| e.to_string())?;
                    Self::default().encoded(x, y as u32).await.map(|_| ())
                });
            }
        }
        while let Some(r) = tasks.join_next().await {
            r.map_err(|e| e.to_string())??;
        }
        Ok(())
    }
    async fn classify(&self, lat: f64, lon: f64) -> Result<bool, String> {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 80. || lon.abs() > 180. {
            return Err("Invalid position".into());
        }
        let n = 4096.;
        let x = (lon + 180.).rem_euclid(360.) / 360. * n;
        let y = (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * n;
        let key = (x.floor() as u32, y.floor() as u32);
        let cached = self.tiles.get(&key);
        let tile = if let Some(t) = cached {
            t
        } else {
            let raw = self.encoded(key.0, key.1).await?;
            let decoded = geozero::mvt::Tile::decode(raw.as_slice()).map_err(|e| e.to_string())?;
            let layer = decoded.layers.iter().find(|l| l.name == "water");
            let extent = layer.and_then(|l| l.extent).unwrap_or(4096) as f64;
            let shapes = if let Some(l) = layer {
                l.features
                    .iter()
                    .map(|f| f.to_geo().map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![]
            };
            let t = Arc::new((extent, shapes));
            self.tiles.insert(key, t.clone());
            t
        };
        let p = Point::new(x.fract() * tile.0, y.fract() * tile.0);
        Ok(!tile.1.iter().any(|g| g.intersects(&p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "explicit public map-service integration check"]
    async fn live_land_water_and_below_sea_level() {
        let land = Land::default();
        for (name, lat, lon, expected) in [
            ("San Francisco", 37.7749, -122.4194, true),
            ("Pacific", 0., -140., false),
            ("Lake Tahoe", 39.0968, -120.0324, false),
            ("Jericho below sea level", 31.87, 35.45, true),
        ] {
            assert_eq!(land.is_land(lat, lon).await.unwrap(), expected, "{name}");
        }
    }
}
