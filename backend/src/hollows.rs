//! Cached, resolution-dependent globe overview. Authoritative shelter remains
//! envelope::height; this cartographic raster uses a metric Gaussian DEM blur.
use crate::{topo::TerrainSource, AppState};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use image::{ImageBuffer, ImageEncoder, Luma, Rgba, RgbaImage};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Semaphore;

pub struct Tiles {
    slots: Semaphore,
}
impl Tiles {
    pub fn new() -> Self {
        Self {
            slots: Semaphore::new(2),
        }
    }
    pub(crate) async fn get(
        &self,
        source: Arc<TerrainSource>,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        crate::geography::Store::shared()
            .get(format!("hollows/{z}/{x}/{y}.png"), || async {
                self.generate(source, z, x, y).await.map(|b| (*b).clone())
            })
            .await
    }
    async fn generate(
        &self,
        source: Arc<TerrainSource>,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        let _slot = self.slots.acquire().await.map_err(|e| e.to_string())?;
        let lat = (std::f64::consts::PI * (1. - 2. * (y as f64 + 0.5) / (1u32 << z) as f64))
            .sinh()
            .atan();
        let mut sz = (z + 3).min(10);
        let metres = |s: u32| 40030173.6 * lat.cos() / (256. * (1u32 << s) as f64);
        while sz > 0 && 5000. / metres(sz) > 40. {
            sz -= 1;
        }
        let sigma = (5000. / metres(sz)) as f32;
        let factor = 2f64.powi(sz as i32 - z as i32);
        let core = (256. * factor).round().max(1.) as u32;
        let pad = (4. * sigma).ceil() as u32 + 2;
        let side = core + 2 * pad;
        let start_x = (x as f64 * 256. * factor) as i64 - pad as i64;
        let start_y = (y as f64 * 256. * factor) as i64 - pad as i64;
        let n = (1u32 << sz) as i64;
        let mut tasks = tokio::task::JoinSet::new();
        let fetch_slots = Arc::new(Semaphore::new(8));
        for ty in start_y.div_euclid(256)..=(start_y + side as i64 - 1).div_euclid(256) {
            for tx in start_x.div_euclid(256)..=(start_x + side as i64 - 1).div_euclid(256) {
                let sx = tx.rem_euclid(n) as u32;
                let sy = ty.clamp(0, n - 1) as u32;
                let source = source.clone();
                let slots = fetch_slots.clone();
                tasks.spawn(async move {
                    let _permit = slots.acquire().await.map_err(|e| e.to_string())?;
                    source.dem(sz, sx, sy).await.map(|v| ((tx, ty), v))
                });
            }
        }
        let mut data = HashMap::new();
        while let Some(t) = tasks.join_next().await {
            let (k, v) = t.map_err(|e| e.to_string())??;
            data.insert(k, v);
        }
        let bytes = tokio::task::spawn_blocking(move || {
            let raw = ImageBuffer::from_fn(side, side, |i, j| {
                let gx = start_x + i as i64;
                let gy = start_y + j as i64;
                let tile = &data[&(gx.div_euclid(256), gy.div_euclid(256))];
                let py = if gy < 0 {
                    0
                } else if gy >= n * 256 {
                    255
                } else {
                    gy.rem_euclid(256) as usize
                };
                Luma([tile[py * 256 + gx.rem_euclid(256) as usize].max(0.) / 12000.])
            });
            render(&raw, core, pad, sigma)
        })
        .await
        .map_err(|e| e.to_string())??;
        let bytes = Arc::new(bytes);
        Ok(bytes)
    }
}
fn render(
    raw: &ImageBuffer<Luma<f32>, Vec<f32>>,
    core: u32,
    pad: u32,
    sigma: f32,
) -> Result<Vec<u8>, String> {
    let mean = image::imageops::blur(raw, sigma);
    let mut out = RgbaImage::new(256, 256);
    for y in 0..256 {
        for x in 0..256 {
            let lo_x = x * core / 256;
            let hi_x = ((x + 1) * core / 256).max(lo_x + 1).min(core);
            let lo_y = y * core / 256;
            let hi_y = ((y + 1) * core / 256).max(lo_y + 1).min(core);
            let mut depth = 0f32;
            for j in lo_y..hi_y {
                for i in lo_x..hi_x {
                    depth = depth.max(
                        (mean.get_pixel(i + pad, j + pad)[0] - raw.get_pixel(i + pad, j + pad)[0])
                            * 12000.
                            - 500.,
                    );
                }
            }
            if depth > 0. {
                let t = (depth / 2500.).clamp(0., 1.);
                out.put_pixel(
                    x,
                    y,
                    Rgba([
                        (57. + 198. * t) as u8,
                        (255. - 163. * t) as u8,
                        (138. - 46. * t) as u8,
                        210,
                    ]),
                );
            }
        }
    }
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(out.as_raw(), 256, 256, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
pub async fn tile(State(app): State<AppState>, Path((z, x, y)): Path<(u32, u32, u32)>) -> Response {
    if z > 8 || x >= (1u32 << z) || y >= (1u32 << z) {
        return (StatusCode::BAD_REQUEST, "Invalid tile").into_response();
    }
    match app.hollows.get(app.renderer, z, x, y).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "public,max-age=31536000,immutable"),
            ],
            crate::geography::response_bytes(bytes),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flat_has_no_hollows_and_depression_is_visible() {
        let mut raw = ImageBuffer::from_pixel(96, 96, Luma([1000f32 / 12000.]));
        let flat = image::load_from_memory(&render(&raw, 64, 16, 5.).unwrap())
            .unwrap()
            .into_rgba8();
        assert!(flat.pixels().all(|p| p[3] == 0));
        for y in 45..51 {
            for x in 45..51 {
                raw.put_pixel(x, y, Luma([0.]));
            }
        }
        let bowl = image::load_from_memory(&render(&raw, 64, 16, 5.).unwrap())
            .unwrap()
            .into_rgba8();
        assert!(bowl.pixels().any(|p| p[3] > 0));
        assert_eq!(bowl.get_pixel(0, 0)[3], 0);
    }
}
