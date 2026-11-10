//! Durable world checkpoints and cursor-readable game events. No disk work in physics.
use super::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const VERSION: u32 = 1;
const MAX_PENDING: usize = 10_000;
const MAX_EVENTS: i64 = 1_000_000;
const RETENTION_MS: u64 = 7 * 24 * 3600 * 1000;
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at_ms: u64,
    pub epoch: u64,
    pub sim_time: f64,
    pub kind: String,
    pub hive: Option<u32>,
    pub unit: Option<String>,
    pub data: Value,
}
#[derive(Default)]
pub struct Journal {
    next: AtomicU64,
    dropped: AtomicU64,
    pending: Mutex<VecDeque<Event>>,
}
impl Journal {
    pub fn emit(
        &self,
        epoch: u64,
        sim_time: f64,
        kind: &str,
        hive: Option<u32>,
        unit: Option<&str>,
        data: Value,
    ) {
        let mut q = self.pending.lock().unwrap();
        if q.len() == MAX_PENDING {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        q.push_back(Event {
            seq: self.next.fetch_add(1, Ordering::Relaxed) + 1,
            at_ms: now_ms(),
            epoch,
            sim_time,
            kind: kind.into(),
            hive,
            unit: unit.map(str::to_owned),
            data,
        });
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct RuntimeBody {
    hold: Option<Pos>,
    input: Input,
    heartbeat: f64,
    reload: f64,
    timer: f64,
    salvaged: bool,
    succession_done: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedBody {
    #[serde(flatten)]
    body: Body,
    #[serde(default, rename = "_runtime")]
    runtime: RuntimeBody,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedBullet {
    #[serde(flatten)]
    body: Bullet,
    #[serde(default, rename = "_p")]
    p: Option<[f64; 3]>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedMissile {
    #[serde(flatten)]
    body: Missile,
    #[serde(default, rename = "_velocity")]
    velocity: Option<[f64; 3]>,
    #[serde(default, rename = "_p")]
    p: Option<[f64; 3]>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct SavedWorld {
    epoch: u64,
    time: f64,
    units: Vec<SavedBody>,
    resources: Vec<Resource>,
    #[serde(default)]
    bullets: Vec<SavedBullet>,
    #[serde(default)]
    missiles: Vec<SavedMissile>,
    #[serde(default)]
    bursts: Vec<Burst>,
    #[serde(default)]
    marks: Vec<Mark>,
    #[serde(default)]
    tags: Vec<model::RadarTag>,
    #[serde(default)]
    next: u64,
    #[serde(default)]
    commands: VecDeque<String>,
    #[serde(default)]
    ai: Option<ai::SavedSwarm>,
    #[serde(default)]
    ai_hives: Vec<u32>,
    #[serde(default)]
    patch_centres: Vec<Pos>,
}
fn array(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}
fn vector(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
impl SavedWorld {
    fn capture(g: &Lab) -> Self {
        Self {
            epoch: g.epoch,
            time: g.time,
            units: g
                .units
                .iter()
                .map(|u| SavedBody {
                    body: u.clone(),
                    runtime: RuntimeBody {
                        hold: Some(u.hold),
                        input: u.input.clone(),
                        heartbeat: u.heartbeat,
                        reload: u.reload,
                        timer: u.timer,
                        salvaged: u.salvaged,
                        succession_done: u.succession_done,
                    },
                })
                .collect(),
            resources: g.resources.clone(),
            bullets: g
                .bullets
                .iter()
                .map(|b| SavedBullet {
                    body: b.clone(),
                    p: Some(array(b.p)),
                })
                .collect(),
            missiles: g
                .missiles
                .iter()
                .map(|m| SavedMissile {
                    body: m.clone(),
                    p: Some(array(m.p)),
                    velocity: Some(array(m.velocity)),
                })
                .collect(),
            bursts: g.bursts.clone(),
            marks: g.marks.clone(),
            tags: g.tags.clone(),
            next: g.next,
            commands: g.commands.clone(),
            ai: Some(g.ai.saved()),
            ai_hives: vec![],
            patch_centres: g
                .patches
                .iter()
                .map(|t| Pos {
                    lat: t.lat,
                    lon: t.lon,
                    alt: 0.,
                })
                .collect(),
        }
    }
    fn restore(self) -> Result<(Lab, Vec<Pos>), String> {
        if !self.time.is_finite() || self.time < 0. {
            return Err("Invalid saved world".into());
        }
        let mut g = new();
        g.epoch = self.epoch;
        g.time = self.time;
        let full = self.ai.is_some();
        g.ai = self.ai.map(ai::Swarm::from_saved).unwrap_or_default();
        for h in self.ai_hives {
            g.ai.brains.entry(h).or_insert_with(|| ai::Brain::new(h));
        }
        for saved in self.units {
            let mut u = saved.body;
            let r = saved.runtime;
            if !u.pos.lat.is_finite()
                || !u.pos.lon.is_finite()
                || !u.pos.alt.is_finite()
                || u.pos.lat.abs() > 90.
                || u.pos.lon.abs() > 180.
                || !u.energy.is_finite()
                || !u.v.is_finite()
            {
                return Err("Invalid saved creature".into());
            }
            u.hold = r.hold.unwrap_or(u.pos);
            u.input = r.input;
            u.heartbeat = r.heartbeat;
            u.reload = r.reload;
            u.timer = r.timer;
            u.salvaged = if full { r.salvaged } else { !u.alive };
            u.succession_done = if full { r.succession_done } else { !u.alive };
            // Browser ownership does not survive a server restart; AI-controlled motion does.
            if u.mode == "manual" && !g.ai.brains.contains_key(&u.hive) {
                u.hold_here();
            }
            g.units.push(u);
        }
        for saved in self.bullets {
            let mut b = saved.body;
            b.p = saved.p.map(vector).unwrap_or_else(|| b.pos.ecef());
            g.bullets.push(b);
        }
        for saved in self.missiles {
            let mut m = saved.body;
            m.p = saved.p.map(vector).unwrap_or_else(|| m.pos.ecef());
            m.velocity = saved.velocity.map(vector).unwrap_or_else(|| {
                super::combat::strike_state(m.start, m.target, m.age, m.duration).1
            });
            g.missiles.push(m);
        }
        g.resources = self.resources;
        g.bursts = self.bursts;
        g.marks = self.marks;
        g.tags = self.tags;
        g.commands = self.commands;
        let largest = g
            .units
            .iter()
            .filter_map(|u| u.id.get(1..)?.parse::<u64>().ok())
            .chain(g.resources.iter().map(|r| r.id))
            .chain(g.bullets.iter().map(|b| b.id))
            .chain(g.missiles.iter().map(|m| m.id))
            .chain(g.tags.iter().map(|t| t.id))
            .chain(g.marks.iter().map(|m| m.id))
            .chain(g.bursts.iter().map(|b| b.id))
            .max()
            .unwrap_or(0);
        g.next = self.next.max(largest + 1);
        let mut centres = self.patch_centres;
        if centres.is_empty() {
            centres.extend(g.units.iter().filter(|u| u.alive).map(|u| u.pos));
            centres.extend(
                g.resources
                    .iter()
                    .filter(|r| r.fall_speed > 0.)
                    .map(|r| r.pos),
            );
        }
        Ok((g, centres))
    }
}
#[derive(Clone, Serialize, Default)]
struct SaveStatus {
    saved_at_ms: Option<u64>,
    saved_epoch: Option<u64>,
    saved_sim_time: Option<f64>,
    last_event_seq: u64,
    checkpoint_bytes: usize,
    save_duration_ms: u64,
    last_error: Option<String>,
}
pub struct Persistence {
    path: PathBuf,
    db: Mutex<Connection>,
    operation: Mutex<()>,
    status: Mutex<SaveStatus>,
    pub journal: Arc<Journal>,
}
impl Persistence {
    pub fn open_env() -> Result<Self, String> {
        let root = std::env::var("HUG_WORLD_ROOT").unwrap_or_else(|_| "data/world".into());
        Self::open(Path::new(&root))
    }
    fn open(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let path = root.join("world.sqlite3");
        let db = Connection::open(&path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS checkpoints (id INTEGER PRIMARY KEY, version INTEGER NOT NULL, saved_at INTEGER NOT NULL, epoch INTEGER NOT NULL, sim_time REAL NOT NULL, payload BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, epoch INTEGER NOT NULL, sim_time REAL NOT NULL, kind TEXT NOT NULL, hive INTEGER, unit TEXT, data TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS events_at_ms ON events(at_ms);
            CREATE INDEX IF NOT EXISTS events_hive_seq ON events(hive,seq);
            CREATE INDEX IF NOT EXISTS events_kind_seq ON events(kind,seq);") .map_err(|e|e.to_string())?;
        let last: u64 = db
            .query_row("SELECT coalesce(max(seq),0) FROM events", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let journal = Arc::new(Journal::default());
        journal.next.store(last, Ordering::Relaxed);
        Ok(Self {
            path,
            db: Mutex::new(db),
            operation: Mutex::new(()),
            status: Mutex::new(SaveStatus {
                last_event_seq: last,
                ..Default::default()
            }),
            journal,
        })
    }
    pub fn restore(&self) -> Result<Option<(Lab, Vec<Pos>)>, String> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT version,saved_at,epoch,sim_time,payload FROM checkpoints ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut errors = vec![];
        for row in rows {
            let (version, at, epoch, time, bytes) = row.map_err(|e| e.to_string())?;
            if version != VERSION {
                return Err(format!(
                    "Unsupported world schema {version}; refusing to start blank"
                ));
            }
            let result = (|| {
                let json = zstd::decode_all(bytes.as_slice()).map_err(|e| e.to_string())?;
                let saved: SavedWorld = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
                saved.restore()
            })();
            match result {
                Ok(world) => {
                    *self.status.lock().unwrap() = SaveStatus {
                        saved_at_ms: Some(at),
                        saved_epoch: Some(epoch),
                        saved_sim_time: Some(time),
                        checkpoint_bytes: bytes.len(),
                        last_event_seq: self.journal.next.load(Ordering::Relaxed),
                        ..Default::default()
                    };
                    if !errors.is_empty() {
                        tracing::error!(?errors, "Recovered previous valid world checkpoint");
                    }
                    return Ok(Some(world));
                }
                Err(e) => errors.push(e),
            }
        }
        if !errors.is_empty() {
            return Err(format!(
                "No valid checkpoint; refusing to start blank: {errors:?}"
            ));
        }
        let bootstrap=self.path.parent().unwrap().join("bootstrap.json");
        let import=std::env::var_os("HUG_WORLD_IMPORT").map(PathBuf::from)
            .or_else(||bootstrap.exists().then_some(bootstrap));
        if let Some(path)=import {
            let bytes=std::fs::read(path).map_err(|e|e.to_string())?;
            let saved:SavedWorld=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
            return saved.restore().map(Some);
        }
        Ok(None)
    }
    pub fn save(&self, lab: &Store, summary: bool) -> Result<Value, String> {
        let _op = self.operation.lock().unwrap();
        let started = std::time::Instant::now();
        let (saved, events) = {
            let g = lab.lock().unwrap();
            if summary {
                self.journal
                    .emit(g.epoch, g.time, "world_summary", None, None, summary_of(&g));
            }
            (
                SavedWorld::capture(&g),
                self.journal
                    .pending
                    .lock()
                    .unwrap()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        let result = (|| {
            let payload = zstd::encode_all(
                serde_json::to_vec(&saved)
                    .map_err(|e| e.to_string())?
                    .as_slice(),
                3,
            )
            .map_err(|e| e.to_string())?;
            let at = now_ms();
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT OR REPLACE INTO checkpoints SELECT 2,version,saved_at,epoch,sim_time,payload FROM checkpoints WHERE id=1",[]).map_err(|e|e.to_string())?;
            tx.execute(
                "INSERT OR REPLACE INTO checkpoints VALUES(1,?1,?2,?3,?4,?5)",
                params![VERSION, at, saved.epoch, saved.time, payload],
            )
            .map_err(|e| e.to_string())?;
            {
                let mut stmt = tx
                    .prepare_cached("INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")
                    .map_err(|e| e.to_string())?;
                for e in &events {
                    stmt.execute(params![
                        e.seq,
                        e.at_ms,
                        e.epoch,
                        e.sim_time,
                        e.kind,
                        e.hive,
                        e.unit,
                        e.data.to_string()
                    ])
                    .map_err(|e| e.to_string())?;
                }
            }
            tx.execute("DELETE FROM events WHERE seq <= (SELECT coalesce(max(seq),0)-?1 FROM events) OR at_ms < ?2",params![MAX_EVENTS,at.saturating_sub(RETENTION_MS)]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            let last = events.last().map(|e| e.seq);
            if let Some(last) = last {
                self.journal
                    .pending
                    .lock()
                    .unwrap()
                    .retain(|e| e.seq > last);
            }
            let mut status = self.status.lock().unwrap();
            status.saved_at_ms = Some(at);
            status.saved_epoch = Some(saved.epoch);
            status.saved_sim_time = Some(saved.time);
            status.last_event_seq = last.unwrap_or(status.last_event_seq);
            status.checkpoint_bytes = payload.len();
            status.save_duration_ms = started.elapsed().as_millis() as u64;
            status.last_error = None;
            if summary {
                tracing::info!(
                    epoch = saved.epoch,
                    sim_time = saved.time,
                    bytes = payload.len(),
                    events = events.len(),
                    duration_ms = status.save_duration_ms,
                    "World checkpoint saved"
                );
            }
            Ok::<(), String>(())
        })();
        if let Err(e) = result {
            self.status.lock().unwrap().last_error = Some(e.clone());
            return Err(e);
        }
        Ok(self.status())
    }
    pub fn status(&self) -> Value {
        let status = self.status.lock().unwrap().clone();
        json!({"database":self.path,"checkpoint_interval_seconds":5,"retention_days":7,"event_limit":MAX_EVENTS,
            "saved":status,"pending_events":self.journal.pending.lock().unwrap().len(),"dropped_events":self.journal.dropped.load(Ordering::Relaxed)})
    }
    fn events(&self, q: &EventQuery) -> Result<Value, String> {
        let db = self.db.lock().unwrap();
        let limit = q.limit.unwrap_or(100).clamp(1, 1000);
        let earliest: u64 = db
            .query_row("SELECT coalesce(min(seq),0) FROM events", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut stmt=db.prepare("SELECT seq,at_ms,epoch,sim_time,kind,hive,unit,data FROM events WHERE seq>?1 AND (?2 IS NULL OR hive=?2) AND (?3 IS NULL OR kind=?3) ORDER BY seq LIMIT ?4").map_err(|e|e.to_string())?;
        let mut rows = stmt
            .query(params![q.after, q.hive, q.kind, limit])
            .map_err(|e| e.to_string())?;
        let mut events = Vec::with_capacity(limit as usize);
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let data: String = r.get(7).map_err(|e| e.to_string())?;
            events.push(Event {
                seq: r.get(0).map_err(|e| e.to_string())?,
                at_ms: r.get(1).map_err(|e| e.to_string())?,
                epoch: r.get(2).map_err(|e| e.to_string())?,
                sim_time: r.get(3).map_err(|e| e.to_string())?,
                kind: r.get(4).map_err(|e| e.to_string())?,
                hive: r.get(5).map_err(|e| e.to_string())?,
                unit: r.get(6).map_err(|e| e.to_string())?,
                data: serde_json::from_str(&data).map_err(|e| e.to_string())?,
            });
        }
        Ok(
            json!({"next_cursor":events.last().map_or(q.after,|e|e.seq),"earliest_available":earliest,"retention_gap":q.after>0&&q.after+1<earliest,"events":events}),
        )
    }
}
fn compact_ai(g: &Lab, h: u32) -> Value {
    let mut value = g.ai.status(h);
    if let Some(obj) = value.as_object_mut() {
        for field in ["contacts", "deposits"] {
            if let Some(v) = obj.remove(field) {
                obj.insert(
                    format!("{field}_count"),
                    json!(v.as_object().map_or(0, |x| x.len())),
                );
            }
        }
        if let Some(trace) = obj.get_mut("trace").and_then(Value::as_array_mut) {
            let excess = trace.len().saturating_sub(3);
            trace.drain(..excess);
        }
    }
    value
}
pub(super) fn summary_of(g: &Lab) -> Value {
    let mut hives = std::collections::BTreeMap::<u32, Vec<Value>>::new();
    for u in &g.units {
        hives.entry(u.hive).or_default().push(json!({"id":u.id,"kind":u.kind,"alive":u.alive,"founder":u.founder,"lineage":u.lineage,"mode":u.mode,"grounded":u.grounded,"pos":u.pos,"speed_m_s":u.v.length(),"energy_j":u.energy,"cargo_energy_j":u.cargo_energy,"minerals_kg":u.minerals,"cargo_minerals_kg":u.cargo_minerals,"power_w":u.power,"radar_visible":u.radar_visible,"sheltered":u.nuclear_sheltered,"charge":u.charge,"brood_remaining":u.brood_remaining}));
    }
    json!({"epoch":g.epoch,"sim_time":g.time,"loading":g.loading,"tutorial":g.tutorial,"units":g.units.len(),
        "resource_count":g.resources.len(),"resource_energy_j":g.resources.iter().map(|r|r.energy).sum::<f64>(),"resource_minerals_kg":g.resources.iter().map(|r|r.minerals).sum::<f64>(),
        "active_missiles":g.missiles,"hives":hives.into_iter().map(|(id,members)|json!({"hive":id,"members":members,"ai":compact_ai(g,id)})).collect::<Vec<_>>()})
}
#[derive(Deserialize, Default)]
pub struct EventQuery {
    #[serde(default)]
    after: u64,
    limit: Option<u32>,
    hive: Option<u32>,
    kind: Option<String>,
}
pub async fn monitor(State(app): State<AppState>) -> Json<Value> {
    let summary = {
        let g = app.lab.lock().unwrap();
        summary_of(&g)
    };
    Json(json!({"world":summary,"persistence":app.persistence.status()}))
}
pub async fn events(
    State(app): State<AppState>,
    Query(q): Query<EventQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || app.persistence.events(&q))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}
pub async fn save_now(State(app): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || app.persistence.save(&app.lab, true))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}
pub fn start(app: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            n += 1;
            let app = app.clone();
            match tokio::task::spawn_blocking(move || app.persistence.save(&app.lab, n % 6 == 0))
                .await
            {
                Ok(Ok(_)) => {}
                result => tracing::error!(?result, "World checkpoint failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "hug-world-{}-{}",
                std::process::id(),
                {
                    static NEXT: AtomicU64 = AtomicU64::new(0);
                    NEXT.fetch_add(1, Ordering::Relaxed)
                } + now_ms() * 1000
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(p: &Persistence) -> Store {
        let mut g = new();
        g.epoch = 123;
        g.time = 42.;
        g.journal = p.journal.clone();
        let mut u = Body::new(
            "q7".into(),
            3,
            true,
            Pos {
                lat: 1.,
                lon: 2.,
                alt: 3000.,
            },
        );
        u.energy = 123456.;
        u.hold = u.pos.offset(10., 20., 30.);
        u.timer = 301.;
        u.reload = 0.2;
        u.input.alt_hold = true;
        u.charge = Some(Charging {
            precision: true,
            target: Pos {
                lat: 2.,
                lon: 3.,
                alt: 0.,
            },
            remaining: 8.,
        });
        u.brood_remaining = 17.;
        u.salvaged = true;
        u.succession_done = true;
        u.lineage = vec![1];
        g.ai.brains.insert(3, ai::Brain::new(3));
        g.units.push(u);
        g.resources.push(Resource {
            id: 9,
            pos: Pos::default(),
            energy: 123.,
            minerals: 45.,
            discovered: vec![3],
            fall_speed: 6.,
        });
        let p0 = Pos {
            lat: 1.,
            lon: 2.,
            alt: 5000.,
        };
        let target = p0.offset(10000., 0., 0.);
        let (v, duration, path) = super::super::combat::plan(p0, target).unwrap();
        let mut m = Missile {
            id: 10,
            hive: 3,
            pos: p0,
            start: p0,
            target,
            age: 0.,
            duration,
            path,
            p: p0.ecef(),
            velocity: v,
            precision: true,
        };
        super::super::combat::step_strike(&mut m, 0.025);
        g.missiles.push(m);
        g.bullets.push(Bullet {
            id: 11,
            hive: 3,
            source: "d2".into(),
            pos: p0,
            velocity: Vec3::new(1., 2., 3.),
            age: 0.3,
            p: p0.ecef(),
        });
        g.next = 30;
        g.commands.push_back("already-applied".into());
        Arc::new(Mutex::new(g))
    }
    #[test]
    fn world_and_projectile_runtime_survive_database_reopen() {
        let dir = Dir::new();
        let p = Persistence::open(&dir.0).unwrap();
        let lab = fixture(&p);
        lab.lock().unwrap().journal.emit(
            123,
            42.,
            "spit_hit",
            Some(3),
            Some("q7"),
            json!({"fatal":false}),
        );
        let expected = serde_json::to_value(SavedWorld::capture(&lab.lock().unwrap())).unwrap();
        p.save(&lab, true).unwrap();
        drop(p);
        let p = Persistence::open(&dir.0).unwrap();
        let (g, _) = p.restore().unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(SavedWorld::capture(&g)).unwrap(),
            expected
        );
        assert!(g.units[0].salvaged && g.units[0].succession_done);
        assert_eq!(g.units[0].charge.as_ref().unwrap().remaining, 8.);
        assert!(g.tutorial.is_none());
        assert_eq!(g.next, 30);
        let result = p
            .events(&EventQuery {
                hive: Some(3),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(result["events"][0]["kind"], "spit_hit");
        assert!(p.journal.next.load(Ordering::Relaxed) > 0);
    }
    #[test]
    fn corrupt_latest_uses_previous_and_never_silently_resets() {
        let dir = Dir::new();
        let p = Persistence::open(&dir.0).unwrap();
        let lab = fixture(&p);
        p.save(&lab, false).unwrap();
        lab.lock().unwrap().time = 99.;
        p.save(&lab, false).unwrap();
        p.db.lock()
            .unwrap()
            .execute("UPDATE checkpoints SET payload=x'00' WHERE id=1", [])
            .unwrap();
        assert_eq!(p.restore().unwrap().unwrap().0.time, 42.);
        p.db.lock()
            .unwrap()
            .execute("UPDATE checkpoints SET payload=x'00'", [])
            .unwrap();
        assert!(p.restore().err().unwrap().contains("No valid checkpoint"));
    }
    #[test]
    fn failed_transaction_keeps_previous_checkpoint_and_pending_events() {
        let dir = Dir::new();
        let p = Persistence::open(&dir.0).unwrap();
        let lab = fixture(&p);
        p.save(&lab, false).unwrap();
        lab.lock().unwrap().time = 99.;
        p.journal.emit(123, 99., "test", None, None, json!({}));
        p.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(p.save(&lab, false).is_err());
        assert!(p.status()["saved"]["last_error"].is_string());
        assert_eq!(p.journal.pending.lock().unwrap().len(), 1);
        assert_eq!(p.restore().unwrap().unwrap().0.time, 42.);
        p.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_save")
            .unwrap();
        p.save(&lab, false).unwrap();
        assert_eq!(p.journal.pending.lock().unwrap().len(), 0);
        assert_eq!(p.restore().unwrap().unwrap().0.time, 99.);
    }
    #[test]
    fn event_cursors_filters_and_retention_gaps_are_explicit() {
        let dir = Dir::new();
        let p = Persistence::open(&dir.0).unwrap();
        let lab = fixture(&p);
        for i in 0..5 {
            p.journal.emit(
                123,
                42.,
                if i % 2 == 0 { "spit_hit" } else { "command" },
                Some(i % 2),
                None,
                json!({"i":i}),
            );
        }
        p.save(&lab, false).unwrap();
        let a = p
            .events(&EventQuery {
                limit: Some(2),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a["events"].as_array().unwrap().len(), 2);
        assert_eq!(a["next_cursor"], 2);
        let b = p
            .events(&EventQuery {
                after: 2,
                hive: Some(0),
                kind: Some("spit_hit".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(b["events"].as_array().unwrap().len(), 2);
        p.db.lock()
            .unwrap()
            .execute("DELETE FROM events WHERE seq<4", [])
            .unwrap();
        assert_eq!(
            p.events(&EventQuery {
                after: 1,
                ..Default::default()
            })
            .unwrap()["retention_gap"],
            true
        );
    }
}
