use super::*;
#[derive(Clone, Copy, Serialize, Deserialize, Debug, Default)]
pub struct Pos {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
}
pub type Vec3 = rapier3d_f64::math::Vector;

pub(super) fn serialize_vector<S: serde::Serializer>(
    v: &Vec3,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut value = serializer.serialize_struct("Vector", 3)?;
    value.serialize_field("x", &v.x)?;
    value.serialize_field("y", &v.y)?;
    value.serialize_field("z", &v.z)?;
    value.end()
}
pub(super) fn deserialize_vector<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
    #[derive(Deserialize)]
    struct Components {
        x: f64,
        y: f64,
        z: f64,
    }
    let v = Components::deserialize(d)?;
    Ok(Vec3::new(v.x, v.y, v.z))
}
impl Pos {
    pub(crate) fn ecef(self) -> Vec3 {
        let (a, b) = (self.lat.to_radians(), self.lon.to_radians());
        Vec3::new(a.cos() * b.cos(), a.cos() * b.sin(), a.sin()).mul(R + self.alt)
    }
    pub(crate) fn from_ecef(v: Vec3) -> Self {
        Self {
            lat: (v.z / v.length()).clamp(-1., 1.).asin().to_degrees(),
            lon: v.y.atan2(v.x).to_degrees(),
            alt: v.length() - R,
        }
    }
    pub(crate) fn offset(self, e: f64, n: f64, u: f64) -> Self {
        let b = self.basis();
        Self::from_ecef(
            self.ecef()
                .add(b[0].mul(e))
                .add(b[1].mul(n))
                .add(b[2].mul(u)),
        )
    }
    pub(super) fn basis(self) -> [Vec3; 3] {
        let (a, b) = (self.lat.to_radians(), self.lon.to_radians());
        [
            Vec3::new(-b.sin(), b.cos(), 0.),
            Vec3::new(-a.sin() * b.cos(), -a.sin() * b.sin(), a.cos()),
            Vec3::new(a.cos() * b.cos(), a.cos() * b.sin(), a.sin()),
        ]
    }
    pub(super) fn distance(self, b: Self) -> f64 {
        self.ecef().sub(b.ecef()).length()
    }
    pub(super) fn valid(self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && self.alt.is_finite()
            && self.lat.abs() <= 80.
            && self.lon.abs() <= 180.
            && self.alt.abs() < 1e7
    }
}
pub(crate) fn local(t: &Terrain, p: Pos) -> V {
    V {
        x: ((p.lon - t.lon + 540.).rem_euclid(360.) - 180.) * 111320. * t.lat.to_radians().cos(),
        y: (p.lat - t.lat) * 111320.,
        z: p.alt,
    }
}
pub(crate) fn elevation(patches: &[Terrain], p: Pos) -> Option<f64> {
    patches.iter().find_map(|t| {
        let v = local(t, p);
        if v.x.abs() < 7900. && v.y.abs() < 7900. {
            Some(t.ground(v).max(0.))
        } else {
            None
        }
    })
}
#[cfg(test)]
pub(super) fn body_ground(patches: &[Terrain], u: &Body) -> Option<f64> {
    let mut ground = elevation(patches, u.pos)?;
    if u.queen() {
        // Four coarse feet protect a broad body from landing through a ridge.
        for (e, n) in [(60., 60.), (-60., 60.), (60., -60.), (-60., -60.)] {
            ground = ground.max(elevation(
                patches,
                u.pos.offset(e * u.size(), n * u.size(), 0.),
            )?);
        }
    }
    Some(ground)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Body {
    pub dry_mass: f64,
    pub founder: bool,
    pub construction_remaining: f64,
    pub growth_target: f64,
    #[serde(skip)]
    pub(super) succession_done: bool,
    pub id: String,
    pub sample_time: f64,
    pub hive: u32,
    pub lineage: Vec<u32>,
    pub kind: String,
    pub pos: Pos,
    #[serde(
        serialize_with = "serialize_vector",
        deserialize_with = "deserialize_vector"
    )]
    pub v: Vec3,
    #[serde(
        serialize_with = "serialize_vector",
        deserialize_with = "deserialize_vector"
    )]
    pub thrust: Vec3,
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    pub energy: f64,
    pub capacity: f64,
    pub power: f64,
    pub alive: bool,
    pub grounded: bool,
    pub mode: String,
    pub agl: f64,
    pub terrain_mean: Option<f64>,
    pub radar_altitude: Option<f64>,
    pub shelter_altitude: Option<f64>,
    pub radar_visible: bool,
    pub nuclear_sheltered: bool,
    pub brood_remaining: f64,
    #[serde(skip)]
    pub(super) envelope_cell: Option<Cell>,
    #[serde(skip)]
    pub(super) salvaged: bool,
    pub cooldown: f64,
    pub charge: Option<Charging>,
    pub minerals: f64,
    pub cargo_energy: f64,
    pub cargo_minerals: f64,
    pub scan_until: f64,
    #[serde(skip)]
    pub(super) hold: Pos,
    #[serde(skip)]
    pub(super) input: Input,
    #[serde(skip)]
    pub(super) heartbeat: f64,
    #[serde(skip)]
    pub(super) reload: f64,
    #[serde(skip)]
    pub(super) timer: f64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Charging {
    pub precision: bool,
    pub target: Pos,
    pub remaining: f64,
}
impl Body {
    pub(super) fn new(id: String, hive: u32, queen: bool, pos: Pos) -> Self {
        let cap = if queen { QUEEN_E } else { DRONE_E };
        Self {
            dry_mass: if queen { 1e7 } else { DRONE_MASS },
            founder: false,
            construction_remaining: 0.,
            growth_target: 0.,
            succession_done: false,
            id,
            sample_time: 0.,
            hive,
            lineage: Vec::new(),
            kind: if queen { "queen" } else { "dragonfly" }.into(),
            pos,
            v: Vec3::default(),
            thrust: Vec3::new(0., 0., gravity(pos)),
            heading: 0.,
            pitch: 0.,
            roll: 0.,
            energy: cap,
            capacity: cap,
            power: 0.,
            alive: true,
            grounded: false,
            mode: "hold".into(),
            agl: 1600.,
            terrain_mean: None,
            radar_altitude: None,
            shelter_altitude: None,
            radar_visible: false,
            nuclear_sheltered: false,
            brood_remaining: 0.,
            envelope_cell: None,
            salvaged: false,
            cooldown: 0.,
            charge: None,
            minerals: if queen { QUEEN_MINERALS } else { 0. },
            cargo_energy: 0.,
            cargo_minerals: 0.,
            scan_until: 0.,
            hold: pos,
            input: Input::default(),
            heartbeat: 0.,
            reload: 0.,
            timer: 0.,
        }
    }
    pub(crate) fn queen(&self) -> bool {
        self.kind == "queen"
    }
    pub(crate) fn size(&self) -> f64 {
        (self.dry_mass / 1e7).cbrt().clamp(0.12, 1.)
    }
    pub(super) fn mineral_capacity(&self) -> f64 {
        (self.dry_mass * 0.1).max(2000.)
    }
    pub(super) fn mass(&self) -> f64 {
        if self.queen() {
            self.dry_mass + self.minerals + self.cargo_minerals
        } else {
            self.dry_mass + self.cargo_minerals
        }
    }
    pub(super) fn hold_here(&mut self) {
        self.hold = self.pos;
        self.mode = "hold".into();
        self.input = Input::default();
    }
    pub(super) fn hull(&self) -> (Pos, f64) {
        if self.queen() {
            (
                self.pos.offset(0., 0., QUEEN_HULL_HEIGHT * self.size()),
                QUEEN_HULL_RADIUS * self.size(),
            )
        } else {
            (self.pos, 8.)
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Bullet {
    pub id: u64,
    pub source: String,
    pub hive: u32,
    pub pos: Pos,
    #[serde(
        serialize_with = "serialize_vector",
        deserialize_with = "deserialize_vector"
    )]
    pub velocity: Vec3,
    pub age: f64,
    #[serde(skip)]
    pub(super) p: Vec3,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Missile {
    pub precision: bool,
    pub id: u64,
    pub hive: u32,
    pub pos: Pos,
    pub start: Pos,
    pub target: Pos,
    pub age: f64,
    pub duration: f64,
    pub path: Vec<Pos>,
    #[serde(skip)]
    pub(super) p: Vec3,
    #[serde(skip)]
    pub(super) velocity: Vec3,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Burst {
    pub precision: bool,
    pub id: u64,
    pub pos: Pos,
    pub age: f64,
    pub radius: f64,
    pub kind: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: u64,
    pub pos: Pos,
    pub energy: f64,
    pub minerals: f64,
    pub discovered: Vec<u32>,
    pub fall_speed: f64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Mark {
    pub id: u64,
    pub hive: u32,
    pub source: String,
    pub pos: Pos,
    pub expires: f64,
    pub accuracy_m: f64,
}
#[derive(Serialize)]
pub struct Balance {
    pub blast_radius: f64,
    pub precision_radius: f64,
    pub radar_offset: f64,
    pub close_mark_altitude: f64,
    pub strike_energy: f64,
    pub birth_energy: f64,
    pub birth_minerals: f64,
}
pub(super) struct TickScratch {
    pub hatches: Vec<(u32, Pos)>,
    pub fire: Vec<(String, u32, Pos, Vec3)>,
    pub launch: Vec<(u32, Pos, Pos, bool)>,
    pub active: Vec<(Pos, bool)>,
    pub melt: Vec<Pos>,
    pub blasts: Vec<(Pos, bool)>,
}
impl Default for TickScratch {
    fn default() -> Self {
        Self {
            hatches: Vec::with_capacity(16),
            fire: Vec::with_capacity(64),
            launch: Vec::with_capacity(16),
            active: Vec::with_capacity(16),
            melt: Vec::with_capacity(64),
            blasts: Vec::with_capacity(16),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RadarTag {
    pub id: u64,
    pub hive: Option<u32>,
    pub source: String,
    pub pos: Pos,
    pub label: String,
}
#[derive(Serialize)]
pub struct Pheromone {
    pub hive: u32,
    pub cell: u64,
    pub pos: Pos,
    pub trail: f64,
    pub danger: f64,
}
pub struct Lab {
    pub(crate) journal: Arc<persistence::Journal>,
    pub(super) ai: ai::Swarm,
    pub(super) scratch: Option<TickScratch>,
    pub epoch: u64,
    pub time: f64,
    pub units: Vec<Body>,
    pub patches: Vec<Terrain>,
    pub(super) terrain_revision: usize,
    pub(super) terrain_index: crate::physics::TerrainIndex,
    pub(super) envelope: Arc<Envelope>,
    pub bullets: Vec<Bullet>,
    pub missiles: Vec<Missile>,
    pub bursts: Vec<Burst>,
    pub loading: bool,
    pub tutorial: Option<String>,
    pub resources: Vec<Resource>,
    pub tags: Vec<RadarTag>,
    pub marks: Vec<Mark>,
    pub(super) next: u64,
    pub(super) commands: VecDeque<String>,
}
pub fn new() -> Lab {
    Lab {
        journal: Arc::new(persistence::Journal::default()),
        ai: ai::Swarm::default(),
        scratch: Some(TickScratch::default()),
        epoch: 0,
        time: 0.,
        units: Vec::with_capacity(100),
        patches: Vec::with_capacity(257),
        terrain_revision: 0,
        terrain_index: Default::default(),
        envelope: Envelope::shared(),
        bullets: Vec::with_capacity(256),
        missiles: Vec::with_capacity(16),
        bursts: Vec::with_capacity(64),
        loading: false,
        tutorial: None,
        resources: vec![],
        tags: Vec::with_capacity(128),
        marks: Vec::with_capacity(100),
        next: 1,
        commands: VecDeque::new(),
    }
}
#[derive(Serialize)]
pub struct View {
    pub all_hives_active: bool,
    pub pheromones: Vec<Pheromone>,
    pub ai_hives: Vec<u32>,
    pub balance: Balance,
    pub terrain_envelope: envelope::MeshInfo,
    pub epoch: u64,
    pub time: f64,
    pub units: Vec<Body>,
    pub bullets: Vec<Bullet>,
    pub missiles: Vec<Missile>,
    pub bursts: Vec<Burst>,
    pub loading: bool,
    pub revision: usize,
    pub tutorial: Option<String>,
    pub resources: Vec<Resource>,
    pub tags: Vec<RadarTag>,
    pub marks: Vec<Mark>,
}
