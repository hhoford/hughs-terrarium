use super::*;
pub(super) async fn load_patch(app: &AppState, at: Pos) -> Result<Terrain, String> {
    let hex = crate::bake::owner(at.lat, at.lon);
    let key = format!(
        "collision/{hex:x}/{:016x}-{:016x}.zst",
        at.lat.to_bits(),
        at.lon.to_bits()
    );
    let data = crate::geography::Store::shared()
        .get(key, || async {
            let t = sample_patch(app, at).await?;
            let mut data = Vec::with_capacity(32 + t.heights.len() * 8);
            for v in [t.lat, t.lon, t.step, t.size as f64]
                .into_iter()
                .chain(t.heights)
            {
                data.extend_from_slice(&v.to_le_bytes());
            }
            tokio::task::spawn_blocking(move || {
                zstd::encode_all(data.as_slice(), 3).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        })
        .await?;
    let data = zstd::decode_all(data.as_slice()).map_err(|e| e.to_string())?;
    if data.len() != 32 + 65 * 65 * 8 {
        return Err("Invalid cached collision patch".into());
    }
    let values: Vec<f64> = data
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    if values.iter().any(|v| !v.is_finite())
        || values[0].to_bits() != at.lat.to_bits()
        || values[1].to_bits() != at.lon.to_bits()
        || values[2] != 250.
        || values[3] != 65.
    {
        return Err("Invalid collision patch metadata".into());
    }
    let terrain = Terrain {
        lat: values[0],
        lon: values[1],
        step: values[2],
        size: 65,
        heights: values[4..].to_vec(),
        shape_cache: Default::default(),
    };
    tokio::task::spawn_blocking(move || crate::physics::prepare_terrain(terrain))
        .await
        .map_err(|e| e.to_string())
}
async fn sample_patch(app: &AppState, at: Pos) -> Result<Terrain, String> {
    let mut t = Terrain {
        lat: at.lat,
        lon: at.lon,
        step: 250.,
        size: 65,
        heights: Vec::with_capacity(65 * 65),
        shape_cache: Default::default(),
    };
    let mut pixels = vec![];
    let mut keys = std::collections::HashSet::new();
    let n = (1u64 << 12) as f64;
    for j in 0..65 {
        for i in 0..65 {
            let lat = at.lat + (j as f64 - 32.) * 250. / 111320.;
            let lon =
                (at.lon + (i as f64 - 32.) * 250. / (111320. * at.lat.to_radians().cos()) + 540.)
                    .rem_euclid(360.)
                    - 180.;
            let x = (lon + 180.) / 360. * n;
            let y = (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * n;
            let k = (x.floor() as u32, y.floor() as u32);
            keys.insert(k);
            pixels.push((k, (x.fract() * 256.), (y.fract() * 256.)));
        }
    }
    let mut tasks = tokio::task::JoinSet::new();
    for (x, y) in keys {
        let r = app.renderer.clone();
        tasks.spawn(async move { r.dem(12, x, y).await.map(|d| ((x, y), d)) });
    }
    let mut tiles = HashMap::new();
    while let Some(r) = tasks.join_next().await {
        let (k, v) = r.map_err(|e| e.to_string())??;
        tiles.insert(k, v);
    }
    for (k, x, y) in pixels {
        let d = &tiles[&k];
        let (i, j) = (x.floor() as usize, y.floor() as usize);
        let (a, b) = (x.fract(), y.fract());
        let h = |i: usize, j: usize| d[j.min(255) * 256 + i.min(255)] as f64;
        t.heights.push(
            (h(i, j) * (1. - a) + h(i + 1, j) * a) * (1. - b)
                + (h(i, j + 1) * (1. - a) + h(i + 1, j + 1) * a) * b,
        );
    }
    for h in &mut t.heights {
        *h = h.max(0.);
    }
    Ok(t)
}
#[derive(Deserialize, Default)]
pub struct ViewQuery {
    unit: Option<String>,
    hive: Option<u32>,
}
pub async fn state(State(app): State<AppState>, Query(q): Query<ViewQuery>) -> Json<View> {
    let g = app.lab.lock().unwrap();
    let hive = q
        .unit
        .as_ref()
        .and_then(|id| g.units.iter().find(|u| &u.id == id))
        .map(|u| u.hive)
        .or(q.hive);
    Json(g.view(hive))
}
#[derive(Deserialize)]
pub struct EnvelopeQuery {
    lat: f64,
    lon: f64,
}
#[derive(Serialize)]
pub struct EnvelopeProbe {
    lat: f64,
    lon: f64,
    terrain_mean: f64,
    radar_altitude: f64,
    shelter_altitude: f64,
    mesh: envelope::MeshInfo,
    cache: envelope::CacheStats,
}
pub async fn envelope_at(
    State(app): State<AppState>,
    Query(q): Query<EnvelopeQuery>,
) -> Result<Json<EnvelopeProbe>, Failure> {
    if !q.lat.is_finite() || !q.lon.is_finite() || q.lat.abs() > 90. || q.lon.abs() > 180. {
        return Err(fail("Invalid coordinates"));
    }
    let mesh = app.lab.lock().unwrap().envelope.clone();
    mesh.ensure(&[(q.lat, q.lon)], &app.renderer)
        .await
        .map_err(fail)?;
    let h = mesh
        .height(q.lat, q.lon, &mut None)
        .ok_or_else(|| fail("Terrain envelope unavailable"))?;
    Ok(Json(EnvelopeProbe {
        lat: q.lat,
        lon: q.lon,
        terrain_mean: h,
        radar_altitude: h + 1000.,
        shelter_altitude: h - 500.,
        mesh: envelope::info(),
        cache: mesh.stats(),
    }))
}
#[derive(Deserialize)]
pub struct HollowsQuery {
    lat: f64,
    lon: f64,
    #[serde(default = "default_radius")]
    radius_km: f64,
    #[serde(default = "default_cells")]
    cells: usize,
}
fn default_radius() -> f64 {
    20.
}
fn default_cells() -> usize {
    65
}
#[derive(Serialize)]
pub struct Hollow {
    lat: f64,
    lon: f64,
    /// Envelope (Gaussian-smoothed) height minus raw terrain height, metres.
    depth: f64,
    /// `depth - 500`: positive exactly where the H-500 shelter rule allows a
    /// body to fly below the envelope shelter altitude while staying airborne.
    shelter: f64,
}
#[derive(Serialize)]
pub struct Hollows {
    lat: f64,
    lon: f64,
    radius_km: f64,
    cells: usize,
    threshold: f64,
    points: Vec<Hollow>,
}
pub async fn hollows(
    State(app): State<AppState>,
    Query(q): Query<HollowsQuery>,
) -> Result<Json<Hollows>, Failure> {
    if !q.lat.is_finite() || !q.lon.is_finite() || q.lat.abs() > 90. || q.lon.abs() > 180. {
        return Err(fail("Invalid coordinates"));
    }
    let radius_km = q.radius_km.clamp(1., 500.);
    let cells = q.cells.clamp(3, 129);
    let n = cells;
    let mut pts = Vec::with_capacity(n * n);
    let deg_lat = radius_km / 111.32;
    for j in 0..n {
        let dlat = (j as f64 / (n - 1) as f64 - 0.5) * 2. * deg_lat;
        let this_lat = (q.lat + dlat).clamp(-90., 90.);
        let deg_lon = radius_km / (111.32 * this_lat.to_radians().cos().max(1e-3));
        for i in 0..n {
            let dlon = (i as f64 / (n - 1) as f64 - 0.5) * 2. * deg_lon;
            let this_lon = (q.lon + dlon + 540.).rem_euclid(360.) - 180.;
            pts.push((this_lat, this_lon));
        }
    }
    let mesh = app.lab.lock().unwrap().envelope.clone();
    // One ensure() over the whole grid runs the Gaussian kernel for every
    // 250 m lattice node in a single synchronous pass — large enough to block
    // the whole server (health/liveness probes time out and the pod restarts).
    // Chunk by grid row so each pass is short and the runtime yields between
    // rows; the field is incremental, so the result is identical.
    for chunk in pts.chunks(n) {
        mesh.ensure(chunk, &app.renderer).await.map_err(fail)?;
    }
    let mut points = Vec::with_capacity(pts.len());
    let mut cell = None;
    for (la, lo) in &pts {
        let h = mesh
            .height(*la, *lo, &mut cell)
            .ok_or_else(|| fail("Terrain envelope unavailable"))?;
        let raw = app.renderer.elevation(*la, *lo).await.map_err(fail)?;
        points.push(Hollow {
            lat: *la,
            lon: *lo,
            depth: h - raw,
            shelter: h - raw - 500.,
        });
    }
    Ok(Json(Hollows {
        lat: q.lat,
        lon: q.lon,
        radius_km,
        cells: n,
        threshold: 500.,
        points,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Reset {
    seed: Option<u64>,
    start: crate::distribute::StartConfig,
}
type Failure = (StatusCode, String);
fn fail(s: impl Into<String>) -> Failure {
    (StatusCode::CONFLICT, s.into())
}
impl Reset {
    pub(super) fn config(self) -> Result<crate::distribute::StartConfig, Failure> {
        let mut cfg = self.start;
        if let Some(seed) = self.seed {
            cfg.seed = seed;
        }
        if cfg.hives > 100
            || cfg.drones_per_hive > 99
            || cfg.deposits > 1000
            || cfg
                .hives
                .checked_mul(cfg.drones_per_hive + 1)
                .is_none_or(|n| n > 100)
        {
            return Err(fail("Starting population/deposit limit exceeded"));
        }
        for (value, cap) in [
            (cfg.queen_energy, QUEEN_E),
            (cfg.queen_minerals, QUEEN_MINERALS),
            (cfg.energy_per_deposit, 1e16),
            (cfg.minerals_per_deposit, 1e9),
        ] {
            if !value.is_finite() || value < 0. || value > cap {
                return Err(fail("Invalid starting resources"));
            }
        }
        Ok(cfg)
    }
}
pub async fn reset(
    State(app): State<AppState>,
    Json(r): Json<Reset>,
) -> Result<Json<View>, Failure> {
    let cfg = r.config()?;
    let mut distribution = crate::distribute::plan(&cfg);
    let started = std::time::Instant::now();
    // Independent deterministic placement searches can wait on map tiles together.
    let mut searches = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(8));
    let jobs = distribution
        .hives
        .iter()
        .enumerate()
        .map(|(i, h)| (true, i, h.at, h.drones))
        .chain(
            distribution
                .deposits
                .iter()
                .enumerate()
                .map(|(i, d)| (false, i, d.at, 0)),
        );
    for (hive, i, at, drones) in jobs {
        let a = app.clone();
        let slots = slots.clone();
        let mut seed = if hive {
            crate::splitmix64(cfg.seed.wrapping_add(i as u64) ^ 0x68697665)
        } else {
            cfg.seed.wrapping_add(i as u64)
        };
        searches.spawn(async move {
            let _permit = slots.acquire().await.map_err(|e| e.to_string())?;
            let mut at = at;
            for _ in 0..128 {
                if a.land.is_land(at.lat, at.lon).await? {
                    let p = Pos {
                        lat: at.lat,
                        lon: at.lon,
                        alt: 1600.,
                    };
                    let mut eligible = true;
                    for n in 0..drones {
                        let d = member_position(p, n);
                        if !a.land.is_land(d.lat, d.lon).await? {
                            eligible = false;
                            break;
                        }
                    }
                    if eligible {
                        return Ok((hive, i, at));
                    }
                }
                if !hive {
                    seed = crate::splitmix64(seed);
                }
                at = crate::distribute::positions(seed, 1)[0];
                if hive {
                    seed = crate::splitmix64(seed);
                }
            }
            Err::<_, String>("No eligible land found; world unchanged".into())
        });
    }
    while let Some(result) = searches.join_next().await {
        let (hive, i, at) = result.map_err(|e| fail(e.to_string()))?.map_err(fail)?;
        if hive {
            distribution.hives[i].at = at;
        } else {
            distribution.deposits[i].at = at;
        }
    }
    let land_ms = started.elapsed().as_millis() as u64;
    let poses: Vec<Pos> = distribution
        .hives
        .iter()
        .map(|h| Pos {
            lat: h.at.lat,
            lon: h.at.lon,
            alt: 0.,
        })
        .collect();
    let epoch = {
        let mut g = app.lab.lock().unwrap();
        if g.loading {
            return Err(fail("World generation already in progress"));
        }
        g.loading = true;
        g.epoch
    };
    let mut tasks = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(4));
    for p in poses
        .iter()
        .copied()
        .chain(distribution.deposits.iter().map(|d| Pos {
            lat: d.at.lat,
            lon: d.at.lon,
            alt: 0.,
        }))
    {
        let a = app.clone();
        let slots = slots.clone();
        tasks.spawn(async move {
            let _permit = slots.acquire().await.map_err(|e| e.to_string())?;
            load_patch(&a, p).await
        });
    }
    let mesh = app.lab.lock().unwrap().envelope.clone();
    let queen_points: Vec<_> = poses.iter().map(|p| (p.lat, p.lon)).collect();
    let patches = async {
        let mut patches = Vec::with_capacity(poses.len() + distribution.deposits.len());
        while let Some(result) = tasks.join_next().await {
            patches.push(result.map_err(|e| e.to_string())??);
        }
        Ok::<_, String>(patches)
    };
    let (patches, fields) = tokio::join!(patches, mesh.ensure(&queen_points, &app.renderer));
    let patches = match patches.and_then(|p| fields.map(|_| p)) {
        Ok(patches) => patches,
        Err(error) => {
            let mut g = app.lab.lock().unwrap();
            if g.epoch == epoch {
                g.loading = false;
            }
            return Err(fail(error));
        }
    };
    let terrain_ms = started.elapsed().as_millis() as u64 - land_ms;
    let mut points = Vec::new();
    for (p, hive) in poses.iter().zip(&distribution.hives) {
        let pos = Pos {
            alt: (elevation(&patches, *p).ok_or_else(|| fail("Terrain unavailable"))? + 1600.)
                .max(mesh.height(p.lat, p.lon, &mut None).unwrap() + 1600.),
            ..*p
        };
        points.push((pos.lat, pos.lon));
        for i in 0..hive.drones {
            let p = member_position(pos, i);
            points.push((p.lat, p.lon));
        }
    }
    let mesh = app.lab.lock().unwrap().envelope.clone();
    if let Err(e) = mesh.ensure(&points, &app.renderer).await {
        let mut g = app.lab.lock().unwrap();
        if g.epoch == epoch {
            g.loading = false;
        }
        return Err(fail(e));
    }
    let mut g = app.lab.lock().unwrap();
    if g.epoch != epoch {
        return Err(fail("World replaced"));
    }
    let journal = g.journal.clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    *g = new();
    g.journal = journal;
    g.epoch = (epoch + 1).max(now);
    g.patches = patches;
    {
        let mut index = crate::physics::TerrainIndex::default();
        index.rebuild(&g.patches);
        g.terrain_index = index;
    }
    g.apply_distribution(distribution).map_err(fail)?;
    g.loading = false;
    g.journal.emit(
        g.epoch,
        g.time,
        "world_reset",
        None,
        None,
        serde_json::json!({"seed":cfg.seed,"hives":cfg.hives,"deposits":cfg.deposits}),
    );
    tracing::info!(
        hives = cfg.hives,
        deposits = cfg.deposits,
        land_ms,
        terrain_ms,
        member_fields_ms = started.elapsed().as_millis() as u64 - land_ms - terrain_ms,
        total_ms = started.elapsed().as_millis() as u64,
        "World generation complete"
    );
    Ok(Json(g.view(None)))
}
#[derive(Deserialize)]
pub struct Cmd {
    pub epoch: u64,
    pub id: String,
    pub action: String,
    pub unit: Option<String>,
    pub target: Option<Pos>,
    pub hive: Option<u32>,
    pub amount: Option<f64>,
    pub mark: Option<u64>,
}
pub async fn command(
    State(app): State<AppState>,
    Json(mut c): Json<Cmd>,
) -> Result<Json<View>, Failure> {
    if app.lab.lock().unwrap().tutorial.is_some() && c.action != "tutorial_end" {
        return Err(fail("Tutorial: controls explain only"));
    }
    if matches!(
        c.action.as_str(),
        "add_queen"
            | "add_dragonfly"
            | "drop_energy"
            | "drop_minerals"
            | "found"
            | "grow"
            | "reproduce"
    ) {
        let p = if let Some(p) = c.target {
            p
        } else {
            let g = app.lab.lock().unwrap();
            g.units
                .iter()
                .find(|u| Some(&u.id) == c.unit.as_ref())
                .ok_or_else(|| fail("Unit unavailable"))?
                .pos
        };
        if !app.land.is_land(p.lat, p.lon).await.map_err(fail)? {
            return Err(fail("Land required"));
        }
    }
    if c.action == "strike" {
        let g = app.lab.lock().unwrap();
        let u = g
            .units
            .iter()
            .find(|u| Some(&u.id) == c.unit.as_ref() && u.alive)
            .ok_or_else(|| fail("Unit unavailable"))?;
        let m = g
            .marks
            .iter()
            .find(|m| Some(m.id) == c.mark && m.hive == u.hive && m.expires > g.time)
            .ok_or_else(|| fail("Choose a fresh dragonfly mark"))?;
        c.target = Some(m.pos);
    }
    if c.action == "add_queen"
        || c.action == "add_dragonfly"
        || c.action == "strike"
        || c.action == "drop_energy"
        || c.action == "drop_minerals"
    {
        let at = c.target.ok_or_else(|| fail("Choose a position"))?;
        if !at.valid() {
            return Err(fail("Invalid position"));
        }
        let needed = {
            let g = app.lab.lock().unwrap();
            if g.epoch != c.epoch {
                return Err(fail("World replaced"));
            }
            !g.patches.iter().any(|t| {
                let p = local(t, at);
                p.x.abs() < 7000. && p.y.abs() < 7000.
            })
        };
        if needed {
            let patch = load_patch(&app, at).await.map_err(fail)?;
            let mut g = app.lab.lock().unwrap();
            if g.epoch != c.epoch {
                return Err(fail("World replaced"));
            }
            g.remember_patch(patch);
        }
    }
    if matches!(c.action.as_str(), "add_queen" | "add_dragonfly") {
        let p = c.target.ok_or_else(|| fail("Choose a position"))?;
        let mesh = app.lab.lock().unwrap().envelope.clone();
        mesh.ensure(&[(p.lat, p.lon)], &app.renderer)
            .await
            .map_err(fail)?;
    }
    let mut g = app.lab.lock().unwrap();
    g.command(&c).map_err(fail)?;
    Ok(Json(g.view(c.hive)))
}
#[derive(Deserialize)]
pub struct Pilot {
    epoch: u64,
    unit: String,
    input: Input,
}
pub async fn input(
    State(app): State<AppState>,
    Json(p): Json<Pilot>,
) -> Result<Json<Body>, Failure> {
    let mut g = app.lab.lock().unwrap();
    if p.epoch != g.epoch {
        return Err(fail("World replaced"));
    }
    if g.tutorial.is_some() {
        return Err(fail("Tutorial: movement disabled"));
    }
    if g.ai.manual.as_deref() != Some(p.unit.as_str()) {
        return Err(fail("Unit not piloted"));
    }
    let time = g.time;
    let mesh = g.envelope.clone();
    let u = g
        .units
        .iter_mut()
        .find(|u| u.id == p.unit && u.alive && u.mode != "hold")
        .ok_or_else(|| fail("Unit not piloted"))?;
    let fire = u.input.fire || p.input.fire;
    u.input = p.input;
    u.input.fire = fire;
    u.heartbeat = time;
    let mut sample = u.clone();
    refresh_envelope(&mesh, &mut sample);
    sample.sample_time = time;
    Ok(Json(sample))
}
#[derive(Deserialize)]
pub struct TerrainQuery {
    unit: String,
}
pub async fn terrain(
    State(app): State<AppState>,
    Query(q): Query<TerrainQuery>,
) -> Result<Json<Terrain>, Failure> {
    let g = app.lab.lock().unwrap();
    let u = g
        .units
        .iter()
        .find(|u| u.id == q.unit)
        .ok_or_else(|| fail("Unit unavailable"))?;
    g.patches
        .iter()
        .filter(|t| {
            let p = local(t, u.pos);
            p.x.abs() < 7900. && p.y.abs() < 7900.
        })
        .min_by(|a, b| {
            let a = local(a, u.pos);
            let b = local(b, u.pos);
            a.x.hypot(a.y).total_cmp(&b.x.hypot(b.y))
        })
        .cloned()
        .map(Json)
        .ok_or_else(|| fail("Terrain loading"))
}
pub async fn dem(State(app): State<AppState>, Path((z, x, y)): Path<(u32, u32, u32)>) -> Response {
    if z > 15 || x >= (1u64 << z) as u32 || y >= (1u64 << z) as u32 {
        return (StatusCode::BAD_REQUEST, "Invalid tile").into_response();
    }
    match app.renderer.encoded(z, x, y).await {
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

pub fn start(app: AppState) -> [tokio::task::JoinHandle<()>; 3] {
    let land_app = app.clone();
    let land_task = tokio::spawn(async move {
        loop {
            let request = {
                let mut g = land_app.lab.lock().unwrap();
                g.ai.land_requests.pop_front().map(|p| (g.epoch, p))
            };
            if let Some((epoch, p)) = request {
                if let Ok(land) = land_app.land.is_land(p.lat, p.lon).await {
                    let mut g = land_app.lab.lock().unwrap();
                    if g.epoch == epoch {
                        g.ai.record_land(p, land);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });
    let prefetch = app.clone();
    let prepare_task = tokio::spawn(async move {
        let mut locations = Vec::with_capacity(512);
        let mut points = Vec::with_capacity(512);
        loop {
            let (epoch, mesh, patch_at) = {
                let g = prefetch.lab.lock().unwrap();
                locations.clear();
                points.clear();
                for u in g
                    .units
                    .iter()
                    .filter(|u| u.alive && u.pos.alt < 60000. + u.v.z.min(0.).powi(2) / 8.)
                {
                    locations.push(u.pos);
                    let seconds = crate::hex_cache::lookahead_seconds();
                    let speed = u.v.x.hypot(u.v.y);
                    if speed > 1. {
                        let steps = (speed * seconds / 4000.).ceil().clamp(1., 8.) as usize;
                        for i in 1..=steps {
                            let t = seconds * i as f64 / steps as f64;
                            let p = u.pos.offset(u.v.x * t, u.v.y * t, 0.);
                            locations.extend([
                                p,
                                p.offset(500., 500., 0.),
                                p.offset(-500., 500., 0.),
                                p.offset(500., -500., 0.),
                                p.offset(-500., -500., 0.),
                            ]);
                        }
                    }
                }
                locations.extend(g.bullets.iter().map(|b| b.pos));
                locations.extend(
                    g.missiles
                        .iter()
                        .filter(|m| m.pos.alt < 15000.)
                        .flat_map(|m| [m.pos, Pos::from_ecef(m.p.add(m.velocity.mul(0.25)))]),
                );
                locations.extend(
                    g.resources
                        .iter()
                        .filter(|r| r.fall_speed > 0.)
                        .map(|r| r.pos),
                );
                locations.extend(g.ai.prepare.iter().copied());
                let patch_at = if g.loading {
                    None
                } else {
                    locations
                        .iter()
                        .find(|p| {
                            !g.patches.iter().any(|t| {
                                let v = local(t, **p);
                                v.x.abs() < 6000. && v.y.abs() < 6000.
                            })
                        })
                        .copied()
                };
                points.extend(locations.iter().map(|p| (p.lat, p.lon)));
                (g.epoch, g.envelope.clone(), patch_at)
            };
            let preparing_since = std::time::Instant::now();
            let before_fields = mesh.stats().computed_fields;
            if let Some(at) = patch_at {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    load_patch(&prefetch, at),
                )
                .await
                {
                    Ok(Ok(patch)) => {
                        let mut g = prefetch.lab.lock().unwrap();
                        if g.epoch == epoch && !g.loading {
                            g.remember_patch(patch);
                        }
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "terrain prefetch unavailable; other bodies keep ticking")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "terrain prefetch timed out; other bodies keep ticking")
                    }
                }
            }
            match tokio::time::timeout(
                std::time::Duration::from_secs(8),
                mesh.ensure(&points, &prefetch.renderer),
            )
            .await
            {
                Ok(Ok(())) => mesh.pin_regions(&points),
                result => tracing::warn!(
                    ?result,
                    "envelope prefetch unavailable; other bodies keep ticking"
                ),
            }
            if patch_at.is_some() || mesh.stats().computed_fields != before_fields {
                crate::hex_cache::observe_preparation(preparing_since.elapsed());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });
    let tick_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(25));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            app.lab.lock().unwrap().tick(DT);
        }
    });
    [prepare_task, tick_task, land_task]
}

pub async fn retired() -> (StatusCode, &'static str) {
    (StatusCode::GONE, "Match API retired; use the globe sandbox")
}

#[derive(Deserialize)]
pub struct Feed {
    epoch: u64,
    seed: u64,
    count: usize,
    energy: f64,
    minerals: f64,
}
pub async fn feed(State(app): State<AppState>, Json(f): Json<Feed>) -> Result<Json<View>, Failure> {
    if f.count == 0
        || f.count > 1000
        || !f.energy.is_finite()
        || !f.minerals.is_finite()
        || f.energy < 0.
        || f.minerals < 0.
        || f.energy > 1e16
        || f.minerals > 1e9
        || f.energy + f.minerals <= 0.
    {
        return Err(fail("Invalid feeding request"));
    }
    {
        let g = app.lab.lock().unwrap();
        if g.epoch != f.epoch || g.tutorial.is_some() || g.resources.len() + f.count > 1000 {
            return Err(fail("Feeding unavailable"));
        }
    }
    let mut pending = Vec::new();
    for i in 0..f.count {
        let mut seed = crate::splitmix64(f.seed.wrapping_add(i as u64));
        let mut accepted = None;
        for _ in 0..128 {
            let p = crate::distribute::positions(seed, 1)[0];
            if app.land.is_land(p.lat, p.lon).await.map_err(fail)? {
                accepted = Some(p);
                break;
            }
            seed = crate::splitmix64(seed);
        }
        let p = accepted.ok_or_else(|| fail("No eligible land; no resources added"))?;
        let alt = app.renderer.elevation(p.lat, p.lon).await.map_err(fail)?;
        pending.push(Pos {
            lat: p.lat,
            lon: p.lon,
            alt,
        });
    }
    let mut g = app.lab.lock().unwrap();
    if g.epoch != f.epoch || g.tutorial.is_some() || g.resources.len() + f.count > 1000 {
        return Err(fail("World changed; no resources added"));
    }
    for pos in pending {
        let id = g.id();
        g.resources.push(Resource {
            id,
            pos,
            energy: f.energy,
            minerals: f.minerals,
            discovered: vec![],
            fall_speed: 0.,
        });
    }
    g.journal.emit(g.epoch,g.time,"resource_feed",None,None,serde_json::json!({"count":f.count,"energy_per_deposit_j":f.energy,"minerals_per_deposit_kg":f.minerals}));
    Ok(Json(g.view(None)))
}

#[derive(Deserialize)]
pub struct AiQuery {
    hive: u32,
}
pub async fn ai_status(
    State(app): State<AppState>,
    Query(q): Query<AiQuery>,
) -> Json<serde_json::Value> {
    let g = app.lab.lock().unwrap();
    Json(g.ai.status(q.hive))
}

#[derive(Deserialize)]
pub struct TagInput {
    epoch: u64,
    unit: String,
    label: String,
    pos: Option<Pos>,
    id: Option<u64>,
    #[serde(default)]
    remove: bool,
}
impl Lab {
    fn radar_tag(&mut self, t: TagInput) -> Result<View, String> {
        if t.epoch != self.epoch {
            return Err("World replaced".into());
        }
        if self.tutorial.is_some() {
            return Err("Tutorial: controls explain only".into());
        }
        let (hive, position) = if t.unit == "god" {
            (None, t.pos.ok_or("Choose a position")?)
        } else {
            let u = self
                .units
                .iter()
                .find(|u| u.alive && u.id == t.unit)
                .ok_or("Creature unavailable")?;
            if self.ai.manual.as_deref() != Some(u.id.as_str()) {
                return Err("Unit not piloted".into());
            }
            (Some(u.hive), t.pos.unwrap_or(u.pos))
        };
        if !position.valid() {
            return Err("Invalid position".into());
        }
        let label = t.label.trim();
        if !t.remove
            && (label.is_empty()
                || label.chars().count() > 48
                || label.chars().any(char::is_control))
        {
            return Err("Use a label of 1 to 48 characters".into());
        }
        if let Some(id) = t.id {
            let index = self
                .tags
                .iter()
                .position(|x| x.id == id && (hive.is_none() || x.hive == hive))
                .ok_or("Tag unavailable")?;
            if t.remove {
                self.tags.remove(index);
            } else {
                self.tags[index].label = label.into();
                self.tags[index].pos = position;
            }
        } else {
            if t.remove
                || self.tags.len() >= 1024
                || self.tags.iter().filter(|x| x.hive == hive).count() >= 128
            {
                return Err("Tag capacity".into());
            }
            let id = self.id();
            self.tags.push(model::RadarTag {
                id,
                hive,
                source: t.unit,
                pos: position,
                label: label.into(),
            });
        }
        Ok(self.view(hive))
    }
}
pub async fn tag(
    State(app): State<AppState>,
    Json(t): Json<TagInput>,
) -> Result<Json<View>, Failure> {
    app.lab.lock().unwrap().radar_tag(t).map(Json).map_err(fail)
}

#[cfg(test)]
mod tag_tests {
    use super::*;
    fn request(unit: &str, label: &str, id: Option<u64>) -> TagInput {
        TagInput {
            epoch: 1,
            unit: unit.into(),
            label: label.into(),
            pos: Some(Pos {
                lat: 0.,
                lon: 0.,
                alt: 1000.,
            }),
            id,
            remove: false,
        }
    }
    #[test]
    fn radar_tags_are_hive_private_editable_and_epoch_scoped() {
        let mut g = new();
        g.epoch = 1;
        g.envelope = Envelope::for_test(|_, _| 0.);
        for h in 0..2 {
            g.units.push(Body::new(
                format!("d{h}"),
                h,
                false,
                Pos {
                    lat: 0.,
                    lon: 0.,
                    alt: 1000.,
                },
            ));
        }
        g.ai.manual = Some("d0".into());
        let view = g.radar_tag(request("d0", "food <safe>", None)).unwrap();
        let id = view.tags[0].id;
        assert_eq!(g.view(Some(0)).tags.len(), 1);
        assert!(g.view(Some(1)).tags.is_empty());
        g.ai.manual = Some("d1".into());
        assert!(g.radar_tag(request("d1", "steal", Some(id))).is_err());
        g.radar_tag(request("god", "observer", None)).unwrap();
        assert_eq!(g.view(None).tags.len(), 2);
        assert_eq!(g.view(Some(0)).tags.len(), 1);
        g.ai.manual = Some("d0".into());
        g.radar_tag(request("d0", "renamed", Some(id))).unwrap();
        assert_eq!(g.tags[0].label, "renamed");
        assert!(g.radar_tag(request("d0", "", None)).is_err());
        assert!(g.radar_tag(request("d0", &"x".repeat(49), None)).is_err());
        let mut old = request("d0", "old", None);
        old.epoch = 0;
        assert!(g.radar_tag(old).is_err());
        g.tutorial = Some("d0".into());
        assert!(g.radar_tag(request("d0", "paused", None)).is_err());
        g.tutorial = None;
        let mut remove = request("d0", "", Some(id));
        remove.remove = true;
        g.radar_tag(remove).unwrap();
        assert!(g.view(Some(0)).tags.is_empty());
        assert!(new().tags.is_empty());
    }
}

pub async fn restore_geography(app: &AppState, centres: Vec<Pos>) -> Result<(), String> {
    let mut jobs = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(8));
    for pos in centres {
        let app = app.clone();
        let slots = slots.clone();
        jobs.spawn(async move {
            let _permit = slots.acquire().await.map_err(|e| e.to_string())?;
            load_patch(&app, pos).await
        });
    }
    while let Some(result) = jobs.join_next().await {
        app.lab
            .lock()
            .unwrap()
            .remember_patch(result.map_err(|e| e.to_string())??);
    }
    let (mesh, points) = {
        let g = app.lab.lock().unwrap();
        (
            g.envelope.clone(),
            g.units
                .iter()
                .filter(|u| u.alive)
                .map(|u| (u.pos.lat, u.pos.lon))
                .collect::<Vec<_>>(),
        )
    };
    mesh.ensure(&points, &app.renderer).await?;
    Ok(())
}
