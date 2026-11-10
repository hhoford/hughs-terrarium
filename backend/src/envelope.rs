//! Fixed, approximately equal-area spherical lattice. Nodes have stable keys;
//! the billions of possible points are not materialised. Real elevations are
//! sampled only for requested Gaussian neighbourhoods and shared by all actors.
use crate::topo::TerrainSource;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    f64::consts::PI,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
};

const R: f64 = 6_371_000.;
pub const SPACING: f64 = 250.;
pub const SIGMA: f64 = 5000.;
pub const CUTOFF: f64 = 4. * SIGMA;
const ROWS: u32 = 80_060; // round(pi * R / SPACING)
const DEM_Z: u32 = 11; // ~76 m at the equator, finer than the 250 m lattice
const RAW_CAP: usize = 1_000_000;
const FIELD_CAP: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Node {
    pub row: u32,
    pub col: u32,
}
fn latitude(row: u32) -> f64 {
    -PI / 2. + row as f64 * PI / ROWS as f64
}
fn columns(row: u32) -> u32 {
    if row == 0 || row == ROWS {
        1
    } else {
        (2. * PI * R * latitude(row).cos() / SPACING)
            .round()
            .max(1.) as u32
    }
}
fn phase(row: u32) -> f64 {
    if row == 0 || row == ROWS {
        0.
    } else {
        (row % 2) as f64 * 0.5
    }
}
impl Node {
    pub(crate) fn coords(self) -> (f64, f64) {
        (
            latitude(self.row),
            (self.col as f64 + phase(self.row)) / columns(self.row) as f64 * 2. * PI - PI,
        )
    }
}
#[derive(Clone, Copy)]
struct Stencil {
    nodes: [Node; 4],
    weights: [f64; 4],
}
fn stencil(lat: f64, lon: f64) -> Stencil {
    let y = ((lat.clamp(-90., 90.) + 90.) / 180. * ROWS as f64).clamp(0., ROWS as f64);
    let row = (y.floor() as u32).min(ROWS - 1);
    let v = y - row as f64;
    let pair = |r| {
        let n = columns(r);
        let x = (((lon + 180.).rem_euclid(360.) / 360.) * n as f64 - phase(r)).rem_euclid(n as f64);
        let a = x.floor() as u32;
        (
            [
                Node { row: r, col: a },
                Node {
                    row: r,
                    col: (a + 1) % n,
                },
            ],
            x - a as f64,
        )
    };
    let (a, u) = pair(row);
    let (b, t) = pair(row + 1);
    Stencil {
        nodes: [a[0], a[1], b[0], b[1]],
        weights: [(1. - v) * (1. - u), (1. - v) * u, v * (1. - t), v * t],
    }
}
// Find just the latitude bands and longitude intervals intersecting the
// spherical cap. No scan of the global hash table or all possible nodes.
fn visit_neighbourhood(q: Node, mut emit: impl FnMut(Node, f64)) {
    let (lat, lon) = q.coords();
    let angular = CUTOFF / R;
    let lo =
        (((lat - angular + PI / 2.) / PI * ROWS as f64).ceil() as i64).clamp(0, ROWS as i64) as u32;
    let hi = (((lat + angular + PI / 2.) / PI * ROWS as f64).floor() as i64).clamp(0, ROWS as i64)
        as u32;
    for row in lo..=hi {
        let a = latitude(row);
        let n = columns(row);
        let denom = lat.cos() * a.cos();
        let boundary = if denom.abs() < 1e-15 {
            -1.
        } else {
            (angular.cos() - lat.sin() * a.sin()) / denom
        };
        if boundary > 1. + 1e-12 {
            continue;
        }
        let extent = boundary.clamp(-1., 1.).acos();
        let centre = (lon + PI) / (2. * PI) * n as f64 - phase(row);
        let width = extent / (2. * PI) * n as f64;
        let (start, end) = if 2. * width + 2. >= n as f64 {
            (0, n as i64 - 1)
        } else {
            (
                (centre - width).floor() as i64,
                (centre + width).ceil() as i64,
            )
        };
        let lat_term = ((a - lat) / 2.).sin().powi(2);
        let cos_term = lat.cos() * a.cos();
        let row_phase = phase(row);
        for i in start..=end {
            let node = Node {
                row,
                col: i.rem_euclid(n as i64) as u32,
            };
            let b = (node.col as f64 + row_phase) / n as f64 * 2. * PI - PI;
            // Haversine stays accurate for adjacent 250 m samples, poles and
            // the date line. Distances ignore creature altitude.
            let hav = lat_term + cos_term * ((b - lon) / 2.).sin().powi(2);
            let d = 2. * R * hav.clamp(0., 1.).sqrt().asin();
            if d <= CUTOFF + 1e-6 {
                emit(node, (-0.5 * (d / SIGMA).powi(2)).exp());
            }
        }
    }
}
#[cfg(test)]
fn neighbourhood(q: Node) -> Vec<(Node, f64)> {
    let mut v = Vec::new();
    visit_neighbourhood(q, |n, w| v.push((n, w)));
    v
}

#[derive(Serialize)]
pub struct MeshInfo {
    pub point_count: u64,
    pub spacing_m: f64,
    pub sigma_m: f64,
    pub cutoff_m: f64,
    pub version: &'static str,
}
pub fn info() -> MeshInfo {
    static N: OnceLock<u64> = OnceLock::new();
    MeshInfo {
        point_count: *N.get_or_init(|| (0..=ROWS).map(|r| columns(r) as u64).sum()),
        spacing_m: SPACING,
        sigma_m: SIGMA,
        cutoff_m: CUTOFF,
        version: "equal-area-250m-gaussian-5km-v1",
    }
}
#[derive(Default)]
struct Cache {
    raw: HashMap<Node, f64>,
    raw_order: VecDeque<Node>,
    field: HashMap<Node, f64>,
    field_order: VecDeque<Node>,
    computed: u64,
}
#[derive(Default)]
struct RawScratch {
    missing: Vec<Node>,
    pixels: Vec<(Node, (u32, u32), f64, f64)>,
    tiles: HashSet<(u32, u32)>,
    grids: HashMap<(u32, u32), Arc<Vec<f32>>>,
}
impl RawScratch {
    fn preallocated() -> Self {
        Self {
            missing: Vec::with_capacity(24000),
            pixels: Vec::with_capacity(24000),
            tiles: HashSet::with_capacity(64),
            grids: HashMap::with_capacity(64),
        }
    }
}
#[derive(Default)]
struct Workspace {
    nodes: HashSet<Node>,
    required: HashSet<Node>,
    raw: HashMap<Node, f64>,
    samples: Vec<(Node, f64, f64)>,
    inputs: RawScratch,
}
impl Workspace {
    fn preallocated() -> Self {
        Self {
            nodes: HashSet::with_capacity(64),
            required: HashSet::with_capacity(24000),
            raw: HashMap::with_capacity(24000),
            samples: Vec::with_capacity(64),
            inputs: RawScratch::preallocated(),
        }
    }
}
pub struct Envelope {
    hexes: Option<crate::hex_cache::HexCache>,
    cache: Mutex<Cache>,
    // Serialises expensive misses, never the hot read path. Network waits are
    // outside the simulation lock and the shared sample-cache mutex.
    prepare: tokio::sync::Mutex<Workspace>,
    #[cfg(test)]
    test_surface: Option<Arc<dyn Fn(f64, f64) -> f64 + Send + Sync>>,
}
#[derive(Clone)]
pub struct Cell {
    nodes: [Node; 4],
    heights: [f64; 4],
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct CacheStats {
    pub raw_points: usize,
    pub field_points: usize,
    pub computed_fields: u64,
}
impl Envelope {
    pub fn shared() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<Envelope>> = OnceLock::new();
        GLOBAL
            .get_or_init(|| {
                Arc::new(Self {
                    hexes: if cfg!(test) {
                        None
                    } else {
                        Some(crate::hex_cache::HexCache::new(
                            crate::geography::Store::shared(),
                        ))
                    },
                    cache: Mutex::new(Cache::default()),
                    prepare: tokio::sync::Mutex::new(Workspace::preallocated()),
                    #[cfg(test)]
                    test_surface: None,
                })
            })
            .clone()
    }
    pub(crate) fn builder() -> Arc<Self> {
        Arc::new(Self {
            hexes: None,
            cache: Mutex::new(Cache::default()),
            prepare: tokio::sync::Mutex::new(Workspace::preallocated()),
            #[cfg(test)]
            test_surface: None,
        })
    }
    pub(crate) async fn region_inputs(
        &self,
        cell: h3o::CellIndex,
        source: &Arc<TerrainSource>,
    ) -> Result<HashMap<Node, f64>, String> {
        let c = h3o::LatLng::from(cell);
        let radius = cell
            .boundary()
            .iter()
            .map(|p| c.distance_rads(*p))
            .fold(0f64, f64::max)
            + (CUTOFF + 1000.) / R;
        let nodes = cap_nodes(c.lat_radians(), c.lng_radians(), radius)
            .into_iter()
            .collect();
        let values = self.raw_values(&nodes, source).await?;
        Ok(values)
    }
    pub fn stats(&self) -> CacheStats {
        let c = self.cache.lock().unwrap();
        CacheStats {
            raw_points: c.raw.len(),
            field_points: c.field.len(),
            computed_fields: c.computed,
        }
    }
    pub fn ready(&self, lat: f64, lon: f64) -> bool {
        #[cfg(test)]
        if self.test_surface.is_some() {
            return true;
        }
        let s = stencil(lat, lon);
        let c = self.cache.lock().unwrap();
        s.nodes.iter().all(|n| {
            c.field.contains_key(n) || self.hexes.as_ref().is_some_and(|h| h.value(*n).is_some())
        })
    }
    pub fn height(&self, lat: f64, lon: f64, cell: &mut Option<Cell>) -> Option<f64> {
        #[cfg(test)]
        if let Some(f) = &self.test_surface {
            return Some(f(lat, lon));
        }
        let s = stencil(lat, lon);
        if cell.as_ref().is_none_or(|c| c.nodes != s.nodes) {
            let c = self.cache.lock().unwrap();
            let mut heights = [0.; 4];
            for (i, n) in s.nodes.iter().enumerate() {
                heights[i] = c
                    .field
                    .get(n)
                    .copied()
                    .or_else(|| self.hexes.as_ref().and_then(|h| h.value(*n)))?;
            }
            *cell = Some(Cell {
                nodes: s.nodes,
                heights,
            });
        }
        Some(
            cell.as_ref()
                .unwrap()
                .heights
                .iter()
                .zip(s.weights)
                .map(|(h, w)| h * w)
                .sum(),
        )
    }
    pub fn pin_regions(&self, points: &[(f64, f64)]) {
        if let Some(hexes) = &self.hexes {
            let nodes = points
                .iter()
                .flat_map(|&(a, b)| stencil(a, b).nodes)
                .collect::<Vec<_>>();
            hexes.pin(&nodes);
        }
    }
    pub async fn flush_cache(&self) -> Result<usize, String> {
        match &self.hexes {
            Some(h) => h.flush().await,
            None => Ok(0),
        }
    }
    pub fn hex_stats(&self) -> serde_json::Value {
        self.hexes
            .as_ref()
            .map(|h| h.stats())
            .unwrap_or(serde_json::Value::Null)
    }
    pub async fn ensure(
        &self,
        points: &[(f64, f64)],
        renderer: &Arc<TerrainSource>,
    ) -> Result<(), String> {
        if points.iter().all(|(a, b)| self.ready(*a, *b)) {
            return Ok(());
        }
        let mut preparing = self.prepare.lock().await;
        if let Some(hexes) = &self.hexes {
            let required = points
                .iter()
                .flat_map(|&(a, b)| stencil(a, b).nodes)
                .collect::<Vec<_>>();
            hexes.load(&required).await?;
        }
        preparing.nodes.clear();
        {
            let c = self.cache.lock().unwrap();
            for &(a, b) in points {
                for node in stencil(a, b).nodes {
                    if !c.field.contains_key(&node)
                        && !self.hexes.as_ref().is_some_and(|h| h.value(node).is_some())
                    {
                        preparing.nodes.insert(node);
                    }
                }
            }
        }
        if preparing.nodes.is_empty() {
            return Ok(());
        }
        // A cheap bounding-cap pass gathers inputs. The exact geodesic kernel
        // is evaluated once below, in its original summation order.
        let workspace = std::mem::take(&mut *preparing);
        let mut workspace = tokio::task::spawn_blocking(move || {
            let mut w = workspace;
            w.required.clear();
            for &n in &w.nodes {
                let (a, b) = n.coords();
                visit_cap_nodes(a, b, (CUTOFF + 0.001) / R, |n| {
                    w.required.insert(n);
                });
            }
            w
        })
        .await
        .map_err(|e| e.to_string())?;
        if let Err(e) = self
            .raw_values_into(
                &workspace.required,
                renderer,
                &mut workspace.raw,
                &mut workspace.inputs,
            )
            .await
        {
            *preparing = workspace;
            return Err(e);
        }
        let workspace = tokio::task::spawn_blocking(move || {
            let mut w = workspace;
            w.samples.clear();
            for &node in &w.nodes {
                let (mut weighted, mut total) = (0., 0.);
                visit_neighbourhood(node, |n, k| {
                    weighted += w.raw[&n] * k;
                    total += k;
                });
                w.samples.push((node, w.raw[&node], weighted / total));
            }
            w
        })
        .await
        .map_err(|e| e.to_string())?;
        *preparing = workspace;
        let samples = &preparing.samples;
        if let Some(hexes) = &self.hexes {
            hexes.merge(samples).await?;
        }
        let mut c = self.cache.lock().unwrap();
        for &(node, _, h) in samples {
            if let std::collections::hash_map::Entry::Vacant(e) = c.field.entry(node) {
                e.insert(h);
                c.field_order.push_back(node);
                c.computed += 1;
            }
        }
        while c.field.len() > FIELD_CAP {
            let node = c.field_order.pop_front().unwrap();
            c.field.remove(&node);
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn for_test(surface: impl Fn(f64, f64) -> f64 + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            hexes: None,
            cache: Mutex::new(Cache::default()),
            prepare: tokio::sync::Mutex::new(Workspace::preallocated()),
            test_surface: Some(Arc::new(surface)),
        })
    }
    async fn raw_values(
        &self,
        nodes: &HashSet<Node>,
        renderer: &Arc<TerrainSource>,
    ) -> Result<HashMap<Node, f64>, String> {
        let mut values = HashMap::with_capacity(nodes.len());
        self.raw_values_into(
            nodes,
            renderer,
            &mut values,
            &mut RawScratch::preallocated(),
        )
        .await?;
        Ok(values)
    }
    async fn raw_values_into(
        &self,
        nodes: &HashSet<Node>,
        renderer: &Arc<TerrainSource>,
        values: &mut HashMap<Node, f64>,
        scratch: &mut RawScratch,
    ) -> Result<(), String> {
        values.clear();
        values.reserve(nodes.len());

        let RawScratch {
            missing,
            pixels,
            tiles,
            grids,
        } = scratch;
        missing.clear();
        pixels.clear();
        tiles.clear();
        grids.clear();
        {
            let c = self.cache.lock().unwrap();
            for n in nodes {
                if let Some(h) = c.raw.get(n) {
                    values.insert(*n, *h);
                } else {
                    missing.push(*n);
                }
            }
        }
        for node in missing.iter() {
            let (lat, lon) = node.coords();
            let (lat, lon) = (lat.to_degrees(), lon.to_degrees());
            if lat.abs() >= 84.5 {
                values.insert(*node, polar(lat, lon));
                continue;
            }
            let (x, y) = mercator(lat, lon);
            let key = (x.floor() as u32, y.floor() as u32);
            tiles.insert(key);
            pixels.push((*node, key, x.fract() * 256., y.fract() * 256.));
        }
        let mut todo = tiles.drain();
        let mut tasks = tokio::task::JoinSet::new();
        let spawn = |tasks: &mut tokio::task::JoinSet<_>, (x, y)| {
            let r = renderer.clone();
            tasks.spawn(async move { r.dem(DEM_Z, x, y).await.map(|g| ((x, y), g)) });
        };
        for tile in todo.by_ref().take(12) {
            spawn(&mut tasks, tile);
        }
        while let Some(result) = tasks.join_next().await {
            let (key, g) = result.map_err(|e| e.to_string())??;
            grids.insert(key, g);
            if let Some(tile) = todo.next() {
                spawn(&mut tasks, tile);
            }
        }
        for (node, key, x, y) in pixels.drain(..) {
            let grid = &grids[&key];
            let (i, j) = (x.floor() as usize, y.floor() as usize);
            let (a, b) = (x.fract(), y.fract());
            let h = |i: usize, j: usize| grid[j.min(255) * 256 + i.min(255)] as f64;
            let value = (h(i, j) * (1. - a) + h(i + 1, j) * a) * (1. - b)
                + (h(i, j + 1) * (1. - a) + h(i + 1, j + 1) * a) * b;
            if !value.is_finite() {
                return Err("Non-finite envelope elevation".into());
            }
            values.insert(node, value.max(0.));
        }
        grids.clear();
        let mut c = self.cache.lock().unwrap();
        for node in missing.drain(..) {
            if let std::collections::hash_map::Entry::Vacant(e) = c.raw.entry(node) {
                e.insert(values[&node]);
                c.raw_order.push_back(node);
            }
        }
        while c.raw.len() > RAW_CAP {
            let node = c.raw_order.pop_front().unwrap();
            c.raw.remove(&node);
        }
        Ok(())
    }
}
fn mercator(lat: f64, lon: f64) -> (f64, f64) {
    let n = (1u32 << DEM_Z) as f64;
    (
        ((lon + 180.).rem_euclid(360.)) / 360. * n,
        (1. - lat.to_radians().tan().asinh() / PI) / 2. * n,
    )
}
fn polar(lat: f64, lon: f64) -> f64 {
    static DATA: OnceLock<Vec<i16>> = OnceLock::new();
    let data = DATA.get_or_init(|| {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(&include_bytes!("../data/polar-heights.i16.gz")[..])
            .read_to_end(&mut bytes)
            .expect("bundled polar terrain");
        assert_eq!(bytes.len(), 2 * 330 * 21600 * 2);
        bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    });
    let x = (((lon + 180.).rem_euclid(360.)) * 60. - 0.5).rem_euclid(21600.);
    let y = ((90. - lat.abs()) * 60. - 0.5).clamp(0., 329.);
    let (i, j) = (x.floor() as usize, y.floor() as usize);
    let (a, b) = (x.fract(), y.fract());
    let h = |i: usize, j: usize| {
        data[(if lat >= 0. { 0 } else { 330 }) * 21600 + j.min(329) * 21600 + i % 21600] as f64
    };
    (h(i, j) * (1. - a) + h(i + 1, j) * a) * (1. - b)
        + (h(i, j + 1) * (1. - a) + h(i + 1, j + 1) * a) * b
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> Arc<Envelope> {
        Arc::new(Envelope {
            hexes: None,
            cache: Mutex::new(Cache::default()),
            prepare: tokio::sync::Mutex::new(Workspace::preallocated()),
            test_surface: None,
        })
    }
    async fn fill(mesh: &Envelope, points: &[(f64, f64)], f: impl Fn(f64, f64) -> f64) {
        // Synthetic terrain is the input fixture. The actual production
        // preparation, spatial cutoff, normalisation and interpolation run.
        let nodes: HashSet<_> = points
            .iter()
            .flat_map(|(a, b)| stencil(*a, *b).nodes)
            .collect();
        let raw: HashSet<_> = nodes
            .into_iter()
            .flat_map(|n| {
                let (a, b) = n.coords();
                cap_nodes(a, b, (CUTOFF + 0.001) / R)
            })
            .collect();
        {
            let mut c = mesh.cache.lock().unwrap();
            for n in raw {
                if !c.raw.contains_key(&n) {
                    let (a, b) = n.coords();
                    c.raw.insert(n, f(a, b));
                    c.raw_order.push_back(n);
                }
            }
        }
        mesh.ensure(points, &Arc::new(TerrainSource::new()))
            .await
            .unwrap();
    }
    #[test]
    fn enormous_global_lattice_uses_only_a_local_geodesic_cap() {
        let meta = info();
        assert!(meta.point_count > 8_000_000_000 && meta.point_count < 8_300_000_000);
        for (a, b) in [(0., 0.), (60., 179.999), (89.99, -180.), (-89.99, 90.)] {
            let q = stencil(a, b).nodes[0];
            let ns = neighbourhood(q);
            let unique: HashSet<_> = ns.iter().map(|(n, _)| *n).collect();
            assert_eq!(ns.len(), unique.len());
            assert!(
                ns.len() > 18_000 && ns.len() < 23_000,
                "cap size {}",
                ns.len()
            );
            assert!(ns
                .iter()
                .all(|(_, w)| *w >= (-8_f64).exp() - 1e-10 && *w <= 1.));
            assert!(ns.iter().any(|(n, w)| *n == q && (*w - 1.).abs() < 1e-12));
            println!(
                "point_count={} at ({a},{b}) local_samples={}",
                meta.point_count,
                ns.len()
            );
        }
        let ns = neighbourhood(stencil(0., -180.).nodes[0]);
        assert!(ns.iter().any(|(n, _)| n.col == 0));
        assert!(ns.iter().any(|(n, _)| n.col == columns(n.row) - 1));
    }
    #[test]
    fn regional_cores_cover_interpolation_and_coarse_radar_at_pentagons_and_seams() {
        use h3o::{CellIndex, LatLng, Resolution};
        let mut points = vec![
            (90., 180.),
            (-90., -180.),
            (0., 179.999),
            (89.999, -179.999),
            (-89.999, 179.999),
        ];
        for base in CellIndex::base_cells().filter(|c| c.is_pentagon()) {
            let c = base.center_child(Resolution::Seven).unwrap();
            let ll = LatLng::from(c);
            points.push((ll.lat(), ll.lng()));
            assert_eq!(c.grid_disk::<Vec<_>>(1).len(), 6);
        }
        for i in 0..1000 {
            let r = crate::splitmix64(i);
            let a = ((r >> 32) as f64 / u32::MAX as f64 * 2. - 1.)
                .asin()
                .to_degrees();
            let b = (r as u32) as f64 / u32::MAX as f64 * 360. - 180.;
            points.push((a, b));
        }
        for (a, b) in points {
            let c = LatLng::new(a, b).unwrap().to_cell(Resolution::Seven);
            let support = c.grid_disk::<Vec<_>>(2);
            for n in stencil(a, b).nodes {
                let (a, b) = n.coords();
                assert!(support.contains(
                    &LatLng::from_radians(a, b)
                        .unwrap()
                        .to_cell(Resolution::Seven)
                ));
            }
        }
        for (a, b) in [(90., 0.), (-90., 0.), (28., 86.), (0., 179.999)] {
            for n in stencil(a, b).nodes {
                let (a, b) = n.coords();
                let c = LatLng::from_radians(a, b)
                    .unwrap()
                    .to_cell(Resolution::Four);
                assert!(region_nodes(c).binary_search(&n).is_ok());
            }
        }
    }
    #[test]
    fn offline_dense_kernel_matches_reference_at_seams_and_poles() {
        for (a, b) in [(28., 86.), (0., 179.999), (89.99, -179.9), (-89.99, 90.)] {
            let nodes = stencil(a, b).nodes.to_vec();
            let mut raw = HashMap::new();
            for q in &nodes {
                for (n, _) in neighbourhood(*q) {
                    let (a, b) = n.coords();
                    raw.insert(n, 1800. + 500. * (a * 29.).sin() * (b * 17.).cos());
                }
            }
            let expected: Vec<_> = nodes
                .iter()
                .map(|q| {
                    let mut sum = 0.;
                    let mut total = 0.;
                    for (n, w) in neighbourhood(*q) {
                        sum += raw[&n] * w;
                        total += w;
                    }
                    sum / total
                })
                .collect();
            let result = evaluate_region(nodes, raw).unwrap();
            for ((_, _, actual), expected) in result.iter().zip(expected) {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
    }
    #[tokio::test]
    async fn normalised_kernel_preserves_metres_and_detects_a_deep_bowl() {
        let flat = empty();
        fill(&flat, &[(0., 0.)], |_, _| 2345.).await;
        assert!((flat.height(0., 0., &mut None).unwrap() - 2345.).abs() < 1e-8);
        let bowl = empty();
        fill(&bowl, &[(0., 0.)], |a, b| {
            let r = R * (a * a + b * b).sqrt();
            2500. - 2000. * (-0.5 * (r / 1000.).powi(2)).exp()
        })
        .await;
        let h = bowl.height(0., 0., &mut None).unwrap();
        let expected = 2500. - 2000. * 1e6 / (1e6 + SIGMA * SIGMA);
        assert!(
            (h - expected).abs() < 2.,
            "bowl height {h} expected {expected}"
        );
        assert!(500. < h - 500.);
        assert!(2500. > h - 500.);
    }
    #[tokio::test]
    async fn field_is_continuous_across_cells_dateline_and_poles() {
        let mesh = empty();
        let points = [
            (0.000001, 179.999999),
            (-0.000001, -179.999999),
            (-90., -180.),
            (-90., 135.),
        ];
        fill(&mesh, &points, |a, b| 2000. + 500. * a.cos() * b.cos()).await;
        let a = mesh.height(points[0].0, points[0].1, &mut None).unwrap();
        let b = mesh.height(points[1].0, points[1].1, &mut None).unwrap();
        assert!((a - b).abs() < 0.01);
        let a = mesh.height(-90., -180., &mut None).unwrap();
        let b = mesh.height(-90., 135., &mut None).unwrap();
        assert!((a - b).abs() < 1e-9);
    }
    #[tokio::test]
    async fn cached_cell_serves_many_actors_without_recalculating_kernels() {
        let mesh = empty();
        fill(&mesh, &[(12.345, 67.89)], |a, b| {
            2500. + a.sin() * 100. + b.cos() * 100.
        })
        .await;
        let before = mesh.stats();
        let expected = mesh.height(12.345, 67.89, &mut None).unwrap();
        let start = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let mesh = &mesh;
                scope.spawn(move || {
                    let mut cell = None;
                    for _ in 0..100_000 {
                        assert_eq!(mesh.height(12.345, 67.89, &mut cell).unwrap(), expected);
                    }
                });
            }
        });
        mesh.ensure(&[(12.345, 67.89)], &Arc::new(TerrainSource::new()))
            .await
            .unwrap();
        assert_eq!(mesh.stats(), before);
        println!(
            "800000 cached actor queries: {:?}; raw={} field={} computed={}",
            start.elapsed(),
            before.raw_points,
            before.field_points,
            before.computed_fields
        );
        assert!(Arc::ptr_eq(&Envelope::shared(), &Envelope::shared()));
    }
    #[test]
    fn measured_polar_data_covers_the_non_mercator_caps() {
        assert_eq!(polar(89., 0.), 0.);
        let south = polar(-89.99, 0.);
        assert!(south > 2000. && south < 4000.);
        assert!((polar(-89., -180.) - polar(-89., 180.)).abs() < 1e-9);
    }
}

pub(crate) fn region_nodes(cell: h3o::CellIndex) -> Vec<Node> {
    let centre = h3o::LatLng::from(cell);
    let radius = cell
        .boundary()
        .iter()
        .map(|p| centre.distance_rads(*p))
        .fold(0f64, f64::max)
        + 500. / R;
    cap_nodes(centre.lat_radians(), centre.lng_radians(), radius)
        .into_iter()
        .filter(|n| {
            let (a, b) = n.coords();
            h3o::LatLng::from_radians(a, b)
                .unwrap()
                .to_cell(h3o::Resolution::Four)
                == cell
        })
        .collect()
}
fn visit_cap_nodes(lat: f64, lon: f64, radius: f64, mut emit: impl FnMut(Node)) {
    let lo =
        (((lat - radius + PI / 2.) / PI * ROWS as f64).floor() as i64).clamp(0, ROWS as i64) as u32;
    let hi =
        (((lat + radius + PI / 2.) / PI * ROWS as f64).ceil() as i64).clamp(0, ROWS as i64) as u32;
    for row in lo..=hi {
        let a = latitude(row);
        let n = columns(row);
        let denom = lat.cos() * a.cos();
        let boundary = if denom.abs() < 1e-15 {
            -1.
        } else {
            (radius.cos() - lat.sin() * a.sin()) / denom
        };
        if boundary > 1. {
            continue;
        }
        let width = boundary.clamp(-1., 1.).acos() / (2. * PI) * n as f64;
        let c = (lon + PI) / (2. * PI) * n as f64 - phase(row);
        let (start, end) = if width * 2. + 2. >= n as f64 {
            (0, n as i64 - 1)
        } else {
            ((c - width).floor() as i64, (c + width).ceil() as i64)
        };
        for i in start..=end {
            emit(Node {
                row,
                col: i.rem_euclid(n as i64) as u32,
            });
        }
    }
}

fn cap_nodes(lat: f64, lon: f64, radius: f64) -> Vec<Node> {
    let mut nodes = Vec::new();
    visit_cap_nodes(lat, lon, radius, |n| nodes.push(n));
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

pub(crate) fn evaluate_region(
    nodes: Vec<Node>,
    raw: HashMap<Node, f64>,
) -> Result<Vec<(Node, f64, f64)>, String> {
    struct Row {
        start: u32,
        n: u32,
        values: Vec<f64>,
    }
    let mut grouped: HashMap<u32, Vec<(u32, f64)>> = HashMap::new();
    for (n, h) in raw {
        grouped.entry(n.row).or_default().push((n.col, h));
    }
    let mut rows = HashMap::new();
    for (row, mut points) in grouped {
        points.sort_unstable_by_key(|p| p.0);
        let n = columns(row);
        let mut start = points[0].0;
        let mut gap = 0;
        for i in 0..points.len() {
            let previous = points[(i + points.len() - 1) % points.len()].0;
            let current = points[i].0;
            let d = if current > previous {
                current - previous
            } else {
                current + n - previous
            };
            if d > gap {
                gap = d;
                start = current;
            }
        }
        let mut values = vec![f64::NAN; (n - gap + 1) as usize];
        for (col, h) in points {
            let i = if col >= start {
                col - start
            } else {
                col + n - start
            };
            values[i as usize] = h;
        }
        rows.insert(row, Row { start, n, values });
    }
    let get = |node: Node| -> Option<f64> {
        let r = rows.get(&node.row)?;
        let i = if node.col >= r.start {
            node.col - r.start
        } else {
            node.col + r.n - r.start
        };
        r.values.get(i as usize).copied().filter(|h| h.is_finite())
    };
    let mut output = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut weighted = 0.;
        let mut total = 0.;
        let mut missing = false;
        // Build row access once per latitude band, avoiding random point hash lookups.
        visit_neighbourhood(node, |n, w| {
            if let Some(h) = get(n) {
                weighted += h * w;
                total += w
            } else {
                missing = true
            }
        });
        if missing || total == 0. {
            return Err("Incomplete Gaussian support".into());
        }
        output.push((
            node,
            get(node).ok_or("Missing raw centre")?,
            weighted / total,
        ));
    }
    Ok(output)
}
