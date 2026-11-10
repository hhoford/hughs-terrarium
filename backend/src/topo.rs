//! Terrarium elevation data and decoded-tile cache. MapLibre renders relief and shading.
use std::{sync::Arc, time::Duration};
const TILE: usize = 256;
const DEM_TILE: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";
type Grid = Vec<f32>;
pub struct TerrainSource {
    client: reqwest::Client,
    dem: moka::future::Cache<(u32, u32, u32), Arc<Grid>>,
}
impl TerrainSource {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(25))
                .user_agent("hug-gamer-terrain/0.1")
                .build()
                .expect("HTTP client"),
            dem: moka::future::Cache::builder()
                .max_capacity(96 * 1024 * 1024)
                .weigher(|_: &(u32, u32, u32), v: &Arc<Grid>| (v.len() * 4) as u32)
                .build(),
        }
    }
    pub(crate) async fn dem(&self, z: u32, x: u32, y: u32) -> Result<Arc<Grid>, String> {
        crate::geography::Store::shared().touch(&format!("dem/{z}/{x}/{y}.png"));
        self.dem
            .try_get_with((z, x, y), async {
                let bytes = self.encoded(z, x, y).await?;
                tokio::task::spawn_blocking(move || {
                    let img = image::load_from_memory(&bytes)
                        .map_err(|e| format!("decode: {e}"))?
                        .to_rgb8();

                    if img.dimensions() != (256, 256) {
                        return Err("Invalid DEM dimensions".into());
                    }
                    let mut grid = Grid::with_capacity(TILE * TILE);
                    for p in img.pixels() {
                        // Terrarium encoding: height = (R*256 + G + B/256) - 32768 meters.
                        let r = p[0] as f32;
                        let g = p[1] as f32;
                        let b = p[2] as f32;
                        grid.push(r * 256.0 + g + b / 256.0 - 32768.0);
                    }
                    let arc = Arc::new(grid);
                    Ok::<_, String>(arc)
                })
                .await
                .map_err(|e| e.to_string())?
            })
            .await
            .map_err(|e: Arc<String>| (*e).clone())
    }

    pub(crate) async fn encoded(&self, z: u32, x: u32, y: u32) -> Result<Arc<Vec<u8>>, String> {
        crate::geography::Store::shared()
            .get(format!("dem/{z}/{x}/{y}.png"), || async {
                let url = DEM_TILE
                    .replace("{z}", &z.to_string())
                    .replace("{x}", &x.to_string())
                    .replace("{y}", &y.to_string());
                let bytes = crate::geography::download(&self.client, &url).await?;
                let dimensions = image::ImageReader::with_format(
                    std::io::Cursor::new(bytes.as_slice()),
                    image::ImageFormat::Png,
                )
                .into_dimensions()
                .map_err(|e| e.to_string())?;
                if dimensions != (256, 256) {
                    return Err("Invalid DEM dimensions".into());
                }
                Ok(bytes)
            })
            .await
    }
    /// Raw (unsmoothed) elevation at an arbitrary lat/lon, bilinearly sampled
    /// from the z11 terrarium tiles. This is the counterpart to the Gaussian
    /// `Envelope::height`: subtracting it from the envelope gives the "hollow
    /// depth" that the H-500 shelter rule turns into a hiding place.
    pub async fn elevation(&self, lat: f64, lon: f64) -> Result<f64, String> {
        if !lat.is_finite() || !lon.is_finite() {
            return Err("Invalid coordinates".into());
        }
        if lat.abs() >= 84.5 {
            return Ok(0.); // polar caps carry no playable hollows
        }
        let z = 11u32;
        let n = (1u32 << z) as f64;
        let x = (lon + 180.).rem_euclid(360.) / 360. * n;
        let y = (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * n;
        let (tx, ty) = (x.floor() as u32, y.floor() as u32);
        let grid = self.dem(z, tx, ty).await?;
        let (px, py) = (x.fract() * 256., y.fract() * 256.);
        let (i, j) = (px.floor() as usize, py.floor() as usize);
        let (a, b) = (px.fract(), py.fract());
        let h = |i: usize, j: usize| grid[j.min(255) * 256 + i.min(255)] as f64;
        let v = (h(i, j) * (1. - a) + h(i + 1, j) * a) * (1. - b)
            + (h(i, j + 1) * (1. - a) + h(i + 1, j + 1) * a) * b;
        Ok(v.max(0.))
    }
}
