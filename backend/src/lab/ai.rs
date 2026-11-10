//! Conventional hive control. No learned model and no world mutation outside
//! existing commands/actuators. The planner sees a projected private observation.
use super::*;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};
const MEMORY: usize = 4096;
const ROUTE_NODES: usize = 256;
const PROBES: usize = 2048;
const RETAINED_NODES: usize = 1024;
#[derive(Default)]
pub(super) struct Swarm {
    pub brains: BTreeMap<u32, Brain>,
    pub manual: Option<String>,
    pub tick: u64,
    max_tick_micros: u64,
    pub prepare: Vec<Pos>,
    pub land_requests: VecDeque<Pos>,
    land: BTreeMap<(u64, u64), bool>,
}
fn land_key(p: Pos) -> (u64, u64) {
    (p.lat.to_bits(), p.lon.to_bits())
}
impl Swarm {
    pub fn record_land(&mut self, p: Pos, land: bool) {
        if self.land.len() >= 512 {
            self.land.pop_first();
        }
        self.land.insert(land_key(p), land);
    }
    fn land_ready(&mut self, p: Pos) -> Option<bool> {
        let result = self.land.get(&land_key(p)).copied();
        if result.is_none()
            && self.land_requests.len() < 32
            && !self
                .land_requests
                .iter()
                .any(|x| land_key(*x) == land_key(p))
        {
            self.land_requests.push_back(p);
        }
        result
    }
    pub fn pheromones(&self, hive: Option<u32>) -> Vec<model::Pheromone> {
        let mut values = Vec::new();
        for (&h, brain) in &self.brains {
            if hive.is_some_and(|wanted| h != wanted) {
                continue;
            }
            for (&key, m) in &brain.memory {
                let trail = decayed(m.trail, m.trail_at, self.tick, 4800);
                let danger = decayed(m.danger, m.danger_at, self.tick, 2400);
                if trail <= 0. && danger <= 0. {
                    continue;
                }
                let Ok(cell) = h3o::CellIndex::try_from(key) else {
                    continue;
                };
                let p = h3o::LatLng::from(cell);
                values.push(model::Pheromone {
                    hive: h,
                    cell: key,
                    pos: Pos {
                        lat: p.lat(),
                        lon: p.lng(),
                        alt: 0.,
                    },
                    trail,
                    danger,
                });
            }
        }
        values
    }
    pub fn status(&self, h: u32) -> serde_json::Value {
        match self.brains.get(&h) {
            Some(b) => {
                serde_json::json!({"enabled":true,"tick":self.tick,"tasks":b.tasks,"memory_entries":b.memory.len(),"contacts":b.contacts,"deposits":b.deposits,"trace":b.trace,"max_tick_micros":self.max_tick_micros,"max_plan_micros":b.max_plan_micros,"max_route_probes":b.max_route_probes,"deliveries":b.deliveries,"net_delivered_joules":b.delivered,"births":b.births,"shots":b.shots})
            }
            None => serde_json::json!({"enabled":false}),
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Job {
    Hold,
    Haul,
    Return,
    Survey,
    Inspect,
    Intercept,
    Escort,
    Queen,
    Founder,
    Withdraw,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Task {
    pub job: Job,
    target: u64,
    goal: Pos,
    since: u64,
    progress: u64,
    #[serde(deserialize_with = "saved_distance")]
    distance: f64,
    score: i32,
    #[serde(skip)]
    route: Vec<Pos>,
    route_tick: u64,
    action_tick: u64,
}
impl Task {
    fn new(job: Job, target: u64, goal: Pos, tick: u64, score: i32) -> Self {
        Self {
            job,
            target,
            goal,
            since: tick,
            progress: tick,
            distance: f64::INFINITY,
            score,
            route: Vec::with_capacity(32),
            route_tick: 0,
            action_tick: 0,
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Contact {
    pos: Pos,
    #[serde(
        serialize_with = "model::serialize_vector",
        deserialize_with = "model::deserialize_vector"
    )]
    v: Vec3,
    queen: bool,
    grounded: bool,
    radius: f64,
    seen: u64,
    alive: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Deposit {
    pos: Pos,
    energy: f64,
    minerals: f64,
    seen: u64,
}
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct Memory {
    trail: u16,
    trail_at: u64,
    danger: u16,
    danger_at: u64,
    visited: u64,
}
fn decayed(value: u16, at: u64, now: u64, lifetime: u64) -> f64 {
    (value as f64 - 1000. * now.saturating_sub(at) as f64 / lifetime as f64).max(0.) / 1000.
}
fn sector(p: Pos) -> u64 {
    u64::from(
        h3o::LatLng::new(p.lat, p.lon)
            .unwrap()
            .to_cell(h3o::Resolution::Seven),
    )
}
fn body_key(s: &str) -> u64 {
    s.get(1..).and_then(|x| x.parse().ok()).unwrap_or(0)
}
#[derive(Clone, Copy)]
struct Member {
    index: usize,
    pos: Pos,
    v: Vec3,
    energy: f64,
    capacity: f64,
    cargo: f64,
    minerals: f64,
    queen: bool,
    founder: bool,
}
#[derive(Default)]
struct HiveObservation {
    members: Vec<Member>,
}
#[derive(Clone, Copy)]
struct Bid {
    unit: usize,
    job: Job,
    target: u64,
    goal: Pos,
    score: i32,
}
#[derive(Clone, Copy)]
struct Node {
    x: i16,
    y: i16,
    level: u8,
    pos: Pos,
    cost: u32,
    parent: usize,
    closed: bool,
    edge: u8,
}
struct Search {
    unit: String,
    origin: Pos,
    goal: Pos,
}
#[derive(Clone, Serialize, Deserialize)]
struct Trace {
    tick: u64,
    unit: String,
    job: Job,
    reason: String,
    score: i32,
}
#[derive(Clone, Serialize, Deserialize)]
struct Drive {
    throttle: f64,
    scan: f64,
    path: VecDeque<u64>,
}
impl Default for Drive {
    fn default() -> Self {
        Self {
            throttle: 0.,
            scan: 0.,
            path: VecDeque::with_capacity(32),
        }
    }
}
pub(super) struct Brain {
    drives: BTreeMap<usize, Drive>,
    route_window: u64,
    nodes_left: usize,
    probes_left: usize,
    blocked: BTreeMap<u64, u64>,
    pub tasks: BTreeMap<usize, Task>,
    contacts: BTreeMap<u64, Contact>,
    deposits: BTreeMap<u64, Deposit>,
    memory: BTreeMap<u64, Memory>,
    obs: HiveObservation,
    bids: Vec<Bid>,
    assigned: Vec<usize>,
    reserved: BTreeMap<u64, (f64, f64)>,
    trace: VecDeque<Trace>,
    search: Option<Search>,
    nodes: Vec<Node>,
    open: BinaryHeap<Reverse<(u32, usize)>>,
    max_route_probes: usize,
    max_plan_micros: u64,
    deliveries: u64,
    delivered: f64,
    births: u64,
    shots: u64,
    personality: u32,
}
fn offer(bids: &mut Vec<Bid>, bid: Bid) {
    let mut count = 0;
    let mut worst = None;
    // Candidates are emitted contiguously per member. Inspect only this
    // member's bounded tail, rather than all earlier members on every offer.
    for (i, b) in bids
        .iter()
        .enumerate()
        .rev()
        .take_while(|(_, b)| b.unit == bid.unit)
    {
        count += 1;
        if worst.is_none_or(|j: usize| (b.score, i) < (bids[j].score, j)) {
            worst = Some(i);
        }
    }
    if count < 12 {
        bids.push(bid);
    } else if let Some(i) = worst {
        if bid.score > bids[i].score {
            bids[i] = bid;
        }
    }
}
impl Brain {
    #[cfg(test)]
    pub fn status_deliveries(&self) -> u64 {
        self.deliveries
    }
    pub(super) fn offspring(&self, h: u32) -> Self {
        let mut b = Self::new(h);
        b.deposits = self.deposits.clone();
        b.contacts = self.contacts.clone();
        b.memory = self.memory.clone();
        b
    }
    pub fn new(h: u32) -> Self {
        Self {
            drives: BTreeMap::new(),
            route_window: 0,
            nodes_left: ROUTE_NODES,
            probes_left: PROBES,
            blocked: BTreeMap::new(),
            tasks: BTreeMap::new(),
            contacts: BTreeMap::new(),
            deposits: BTreeMap::new(),
            memory: BTreeMap::new(),
            obs: HiveObservation {
                members: Vec::with_capacity(100),
            },
            bids: Vec::with_capacity(1200),
            assigned: Vec::with_capacity(100),
            reserved: BTreeMap::new(),
            trace: VecDeque::with_capacity(256),
            search: None,
            nodes: Vec::with_capacity(RETAINED_NODES),
            open: BinaryHeap::with_capacity(PROBES * 2),
            max_route_probes: 0,
            max_plan_micros: 0,
            deliveries: 0,
            delivered: 0.,
            births: 0,
            shots: 0,
            personality: h % 3,
        }
    }
    fn trace(&mut self, g: &Lab, i: usize, t: &Task, tick: u64, reason: &'static str) {
        if self.trace.len() == 256 {
            self.trace.pop_front();
        }
        g.journal.emit(
            g.epoch,
            g.time,
            "ai_task",
            Some(g.units[i].hive),
            Some(&g.units[i].id),
            serde_json::json!({"job":t.job,"reason":reason,"score":t.score,"tick":tick}),
        );
        self.trace.push_back(Trace {
            tick,
            unit: g.units[i].id.clone(),
            job: t.job,
            reason: reason.into(),
            score: t.score,
        });
    }
    fn memory(&mut self, key: u64) -> &mut Memory {
        if !self.memory.contains_key(&key) && self.memory.len() >= MEMORY {
            if let Some((&old, _)) = self.memory.iter().min_by_key(|(key, m)| (m.visited, **key)) {
                self.memory.remove(&old);
            }
        }
        self.memory.entry(key).or_default()
    }
    fn observe(&mut self, g: &Lab, h: u32, tick: u64) {
        self.obs.members.clear();
        for (i, u) in g.units.iter().enumerate() {
            if u.alive && u.hive == h {
                self.obs.members.push(Member {
                    index: i,
                    pos: u.pos,
                    v: u.v,
                    energy: u.energy,
                    capacity: u.capacity,
                    cargo: u.cargo_energy,
                    minerals: u.cargo_minerals,
                    queen: u.queen(),
                    founder: u.founder,
                });
            } else if u.hive != h && g.unit_visible(h, u) {
                let key = body_key(&u.id);
                self.contacts.insert(
                    key,
                    Contact {
                        pos: u.pos,
                        v: u.v,
                        queen: u.queen(),
                        grounded: u.grounded,
                        radius: u.hull().1,
                        seen: tick,
                        alive: u.alive,
                    },
                );
                if u.alive {
                    let m = self.memory(sector(u.pos));
                    m.danger = 500;
                    m.danger_at = tick;
                    m.visited = tick;
                }
            }
        }
        self.contacts.retain(|_, c| {
            tick.saturating_sub(c.seen) < if c.queen { 40 * 43200 } else { 40 * 120 }
        });
        let scanned = |p: Pos| {
            self.obs.members.iter().any(|m| {
                let u = &g.units[m.index];
                let fraction = ((u.pos.alt - 20000.) / 60000.).clamp(0., 1.);
                m.pos.distance(p) < 120.
                    || (!u.queen()
                        && u.scan_until > g.time
                        && u.pos.distance(p) <= 8000. + 992000. * fraction
                        && segment_sphere(
                            u.pos.ecef(),
                            p.offset(0., 0., 5.).ecef(),
                            Vec3::ZERO,
                            R - 1.,
                        )
                        .is_none()
                        && (fraction > 0.
                            || !segment_ground(
                                &g.patches,
                                u.pos.ecef(),
                                p.offset(0., 0., 5.).ecef(),
                            )))
            })
        };
        self.deposits
            .retain(|id, d| !scanned(d.pos) || g.resources.iter().any(|r| r.id == *id));
        for r in &g.resources {
            if r.discovered.contains(&h) && scanned(r.pos) {
                self.deposits.insert(
                    r.id,
                    Deposit {
                        pos: r.pos,
                        energy: r.energy,
                        minerals: r.minerals,
                        seen: tick,
                    },
                );
            }
        }
        for n in 0..self.obs.members.len() {
            let m = self.obs.members[n];
            let u = &g.units[m.index];
            if u.queen()
                || u.scan_until <= g.time
                || self.drives.entry(m.index).or_default().scan == u.scan_until
            {
                continue;
            }
            self.drives.get_mut(&m.index).unwrap().scan = u.scan_until;
            for j in 0..13 {
                let angle = j as f64 * std::f64::consts::TAU / 12.;
                let sample = m.pos.offset(
                    if j == 12 { 0. } else { angle.sin() * 5000. },
                    if j == 12 { 0. } else { angle.cos() * 5000. },
                    0.,
                );
                let cell = h3o::CellIndex::try_from(sector(sample)).unwrap();
                let centre = h3o::LatLng::from(cell);
                let mut p = Pos {
                    lat: centre.lat(),
                    lon: centre.lng(),
                    alt: 0.,
                };
                let Some(height) = g.terrain_index.elevation(&g.patches, p) else {
                    continue;
                };
                p.alt = height + 5.;
                if m.pos.distance(p) <= 8000.
                    && segment_sphere(m.pos.ecef(), p.ecef(), Vec3::ZERO, R - 1.).is_none()
                    && !segment_ground(&g.patches, m.pos.ecef(), p.ecef())
                {
                    self.memory(u64::from(cell)).visited = tick;
                }
            }
        }
        while self.deposits.len() > 1000 {
            self.deposits.pop_first();
        }
        self.tasks
            .retain(|&i, _| g.units.get(i).is_some_and(|u| u.alive && u.hive == h));
    }
    fn cost(&self, p: Pos, tick: u64) -> f64 {
        self.memory.get(&sector(p)).map_or(0., |m| {
            decayed(m.danger, m.danger_at, tick, 2400)
                * (if self.personality == 0 { 2. } else { 1. })
                - 0.2 * decayed(m.trail, m.trail_at, tick, 4800)
        })
    }
    fn reserve(&self, _g: &Lab, q: &Body) -> f64 {
        let own = self
            .obs
            .members
            .iter()
            .filter(|m| !m.queen)
            .map(|m| (m.capacity - m.energy).max(0.))
            .sum::<f64>();
        let hover = q.mass() * (1. + 4. * gravity(q.pos));
        (hover * (TAKEOFF_PREP + 120.) + own) * 1.25
    }
    fn fuel_ok(&self, m: &Member, p: Pos) -> bool {
        let home = self
            .obs
            .members
            .iter()
            .find(|m| m.queen)
            .map_or(m.pos, |q| q.pos);
        let estimate = |mass: f64, speed: f64, seconds: f64| {
            let v = Vec3::new(speed, 0., 0.);
            let thrust =
                drag_acceleration(false, mass, m.pos.alt, v) + Vec3::new(0., 0., gravity(m.pos));
            actuator_power(mass, thrust, v).min(DRONE_POWER) * (seconds + 60.)
        };
        m.energy
            > 1.25
                * (estimate(DRONE_MASS + m.minerals, 35., m.pos.distance(p) / 35.)
                    + estimate(
                        DRONE_MASS + DRONE_CARGO_MINERALS,
                        55.,
                        p.distance(home) / 55.,
                    ))
    }
    fn allocate(&mut self, g: &Lab, _h: u32, tick: u64, manual: Option<&str>) {
        self.bids.clear();
        self.assigned.clear();
        self.reserved.clear();
        let queen = self.obs.members.iter().find(|m| m.queen).copied();
        let qbody = queen.map(|m| &g.units[m.index]);
        // A hive shares a bounded frontier set; do not regenerate the same
        // geographic candidates separately for every member.
        let mut frontiers = [None; 24];
        if let Some(q) = queen {
            let lead = self
                .obs
                .members
                .iter()
                .filter(|m| !m.queen)
                .max_by(|a, b| a.pos.distance(q.pos).total_cmp(&b.pos.distance(q.pos)))
                .filter(|m| m.pos.distance(q.pos) > 2000.)
                .map(|m| m.pos);
            for (a, anchor) in [Some(q.pos), lead].into_iter().enumerate() {
                let Some(anchor) = anchor else {
                    continue;
                };
                for n in 0..12 {
                    let angle = n as f64 * std::f64::consts::TAU / 12.;
                    let p = anchor.offset(angle.sin() * 5000., angle.cos() * 5000., 0.);
                    if !p.valid() {
                        continue;
                    }
                    let key = sector(p);
                    let age = self
                        .memory
                        .get(&key)
                        .map_or(12000, |m| tick.saturating_sub(m.visited));
                    let score =
                        100. + 200. * (age as f64 / 12000.).min(1.) - 700. * self.cost(p, tick);
                    frontiers[a * 12 + n] = Some((p, key, score as i32));
                }
            }
        }
        for m in &self.obs.members {
            let u = &g.units[m.index];
            if manual == Some(u.id.as_str()) {
                continue;
            }
            if m.queen || m.founder {
                continue;
            }
            let receive = qbody.map_or(0., |q| (q.capacity - q.energy).max(0.));
            if let Some(t) = self.tasks.get(&m.index) {
                let valid = match t.job {
                    Job::Survey | Job::Withdraw => true,
                    Job::Inspect | Job::Intercept => {
                        self.contacts.get(&t.target).is_some_and(|c| c.alive)
                    }
                    Job::Escort => self
                        .obs
                        .members
                        .iter()
                        .any(|m| m.index == t.target as usize && m.cargo > 0.),
                    _ => false,
                };
                if valid {
                    offer(
                        &mut self.bids,
                        Bid {
                            unit: m.index,
                            job: t.job,
                            target: t.target,
                            goal: t.goal,
                            score: t.score,
                        },
                    );
                }
            }

            if let Some(q) = queen {
                if (m.cargo > 0. && receive > 1.)
                    || (m.minerals > 0. && qbody.is_some_and(|q| q.minerals < q.mineral_capacity()))
                    || m.energy < m.capacity * 0.25
                {
                    offer(
                        &mut self.bids,
                        Bid {
                            unit: m.index,
                            job: Job::Return,
                            target: 0,
                            goal: q.pos,
                            score: if m.energy < m.capacity * 0.25 {
                                30000
                            } else {
                                2200
                            },
                        },
                    );
                }
            }
            if let Some(q) = qbody {
                for (&id, d) in self.deposits.iter().take(32) {
                    if (d.energy > 0. && receive > 1e7 && m.cargo < DRONE_CARGO_ENERGY)
                        || (d.minerals > 0.
                            && q.minerals < q.mineral_capacity()
                            && m.minerals < DRONE_CARGO_MINERALS)
                    {
                        let gain = (d.energy.min(receive) / DRONE_CARGO_ENERGY).min(1.)
                            + (d.minerals.min((q.mineral_capacity() - q.minerals).max(0.))
                                / DRONE_CARGO_MINERALS)
                                .min(1.);
                        if self.fuel_ok(m, d.pos) {
                            let seconds =
                                m.pos.distance(d.pos) / 55. + d.pos.distance(q.pos) / 55. + 10.;
                            let score = 1000.
                                * (gain * (if self.personality == 1 { 2. } else { 1. })
                                    / (1. + seconds / 300.)
                                    - self.cost(d.pos, tick));
                            offer(
                                &mut self.bids,
                                Bid {
                                    unit: m.index,
                                    job: Job::Haul,
                                    target: id,
                                    goal: Pos {
                                        alt: d.pos.alt + 80.,
                                        ..d.pos
                                    },
                                    score: score as i32,
                                },
                            );
                        }
                    }
                }
            }
            for (&id, c) in self.contacts.iter().filter(|(_, c)| c.alive).take(8) {
                if self.fuel_ok(m, c.pos) {
                    let near = m.pos.distance(c.pos);
                    let fresh = tick.saturating_sub(c.seen) < 80;
                    let job = if c.queen {
                        Job::Inspect
                    } else {
                        Job::Intercept
                    };
                    let benefit = if c.queen {
                        1.3
                    } else if near < 3000. && fresh {
                        1.5
                    } else {
                        0.3
                    };
                    let score = 1000.
                        * (benefit * (if self.personality == 2 { 2. } else { 1. })
                            / (1. + (near / 20000.).min(1.))
                            - 0.25 * self.cost(c.pos, tick));
                    offer(
                        &mut self.bids,
                        Bid {
                            unit: m.index,
                            job,
                            target: id,
                            goal: c.pos,
                            score: score as i32,
                        },
                    );
                }
            }
            if queen.is_some() {
                let loaded = self
                    .obs
                    .members
                    .iter()
                    .filter(|x| x.cargo > 0. && x.index != m.index)
                    .min_by(|a, b| a.pos.distance(m.pos).total_cmp(&b.pos.distance(m.pos)));
                if let Some(friend) = loaded {
                    if self.cost(friend.pos, tick) > 0. {
                        offer(
                            &mut self.bids,
                            Bid {
                                unit: m.index,
                                job: Job::Escort,
                                target: friend.index as u64,
                                goal: friend.pos,
                                score: 800,
                            },
                        );
                    }
                }
                for &(p, key, score) in frontiers.iter().flatten() {
                    if self.fuel_ok(m, p) {
                        offer(
                            &mut self.bids,
                            Bid {
                                unit: m.index,
                                job: Job::Survey,
                                target: key,
                                goal: p,
                                score,
                            },
                        );
                    }
                }
            }
        }
        self.blocked.retain(|_, until| *until > tick);
        self.bids.retain(|b| !self.blocked.contains_key(&b.target));
        let local = self
            .bids
            .iter()
            .filter(|b| matches!(b.job, Job::Survey | Job::Haul | Job::Inspect))
            .max_by_key(|b| (b.score, Reverse(b.unit)))
            .map(|b| b.unit)
            .filter(|_| {
                self.obs
                    .members
                    .iter()
                    .filter(|m| !m.queen && !m.founder)
                    .count()
                    > 1
            });
        self.bids.sort_by_key(|b| {
            let old = self
                .tasks
                .get(&b.unit)
                .filter(|t| t.job == b.job && t.target == b.target);
            let lease = old.is_some_and(|t| tick - t.since < 200);
            let score = b.score + if old.is_some() { 150 } else { 0 };
            (
                Reverse(b.job == Job::Return),
                Reverse(
                    local == Some(b.unit)
                        && matches!(b.job, Job::Survey | Job::Haul | Job::Inspect),
                ),
                Reverse(lease),
                Reverse(score),
                b.unit,
                b.target,
                b.job as u8,
            )
        });
        let mut receiving = qbody.map_or(0., |q| (q.capacity - q.energy).max(0.));
        let mut receiving_minerals =
            qbody.map_or(0., |q| (q.mineral_capacity() - q.minerals).max(0.));
        for bid in &self.bids {
            if bid.score <= 0 || self.assigned.contains(&bid.unit) {
                continue;
            }
            if matches!(bid.job, Job::Survey | Job::Inspect | Job::Escort)
                && self.assigned.iter().any(|i| {
                    self.tasks
                        .get(i)
                        .is_some_and(|t| t.job == bid.job && t.target == bid.target)
                })
            {
                continue;
            }
            if self.tasks.get(&bid.unit).is_some_and(|t| {
                (t.job != bid.job || t.target != bid.target)
                    && bid.job != Job::Return
                    && (tick - t.since < 200 || t.score + 150 > bid.score)
            }) {
                continue;
            }
            if bid.job == Job::Haul {
                let d = self.deposits[&bid.target];
                let (energy, minerals) =
                    self.reserved.get(&bid.target).copied().unwrap_or_default();
                if d.energy - energy <= 0. && d.minerals - minerals <= 0. {
                    continue;
                }
                let e = (d.energy - energy)
                    .min((DRONE_CARGO_ENERGY - g.units[bid.unit].cargo_energy).max(0.))
                    .max(0.);
                let m = (d.minerals - minerals)
                    .min((DRONE_CARGO_MINERALS - g.units[bid.unit].cargo_minerals).max(0.))
                    .max(0.);
                let useful = e.min(receiving);
                let useful_m = m.min(receiving_minerals);
                if useful == 0. && useful_m == 0. {
                    continue;
                }
                self.reserved.insert(bid.target, (energy + e, minerals + m));
                receiving -= useful;
                receiving_minerals -= useful_m;
            }
            if let Some(t) = self
                .tasks
                .get_mut(&bid.unit)
                .filter(|t| t.job == bid.job && t.target == bid.target)
            {
                if t.job != Job::Survey {
                    t.goal = bid.goal;
                }
                t.score = bid.score;
            } else {
                self.tasks.insert(
                    bid.unit,
                    Task::new(bid.job, bid.target, bid.goal, tick, bid.score),
                );
            }
            if self.tasks[&bid.unit].since == tick {
                if self.trace.len() == 256 {
                    self.trace.pop_front();
                }
                g.journal.emit(g.epoch,g.time,"ai_task",Some(g.units[bid.unit].hive),Some(&g.units[bid.unit].id),serde_json::json!({"job":bid.job,"reason":"allocated","score":bid.score,"tick":tick}));
                self.trace.push_back(Trace {
                    tick,
                    unit: g.units[bid.unit].id.clone(),
                    job: bid.job,
                    reason: "allocated".into(),
                    score: bid.score,
                });
            }
            self.assigned.push(bid.unit);
        }
        self.tasks.retain(|i, t| {
            matches!(t.job, Job::Queen | Job::Founder)
                || g.units[*i].founder
                || g.units[*i].queen()
                || self.assigned.contains(i)
        });
    }
}

fn request(world: &mut Swarm, p: Pos) {
    if p.valid() && !world.prepare.iter().any(|x| x.distance(p) < 150.) {
        if world.prepare.len() == 64 {
            world.prepare.remove(0);
        }
        world.prepare.push(p);
    }
}
fn ground(g: &Lab, world: &mut Swarm, p: Pos) -> Option<(f64, f64)> {
    let a = g.terrain_index.elevation(&g.patches, p);
    let h = g.envelope.height(p.lat, p.lon, &mut None);
    match (a, h) {
        (Some(a), Some(h)) => Some((a, h)),
        _ => {
            request(world, p);
            None
        }
    }
}
fn enu(a: Pos, b: Pos) -> Vec3 {
    let delta = b.ecef() - a.ecef();
    let basis = a.basis();
    Vec3::new(delta.dot(basis[0]), delta.dot(basis[1]), b.alt - a.alt)
}
fn clear(g: &Lab, world: &mut Swarm, a: Pos, b: Pos, radius: f64) -> bool {
    let steps = (a.distance(b) / 250.).ceil().clamp(1., 40.) as usize;
    for i in 0..=steps {
        let p = Pos::from_ecef(a.ecef().lerp(b.ecef(), i as f64 / steps as f64));
        let Some((h, _)) = ground(g, world, p) else {
            return false;
        };
        if p.alt < h + radius + 5. {
            return false;
        }
    }
    g.terrain_index.clearance(&g.patches, a, b, radius)
}
impl Brain {
    fn route(&mut self, g: &Lab, world: &mut Swarm, u: &Body, t: &mut Task, tick: u64) {
        if self.probes_left == 0 {
            return;
        }
        let mut goal = t.goal;
        let delta = enu(u.pos, goal);
        let distance = delta.x.hypot(delta.y);
        if distance > 4000. {
            goal = u
                .pos
                .offset(delta.x * 4000. / distance, delta.y * 4000. / distance, 0.);
        }
        let Some((h, mean)) = ground(g, world, goal) else {
            return;
        };
        let radius = if u.queen() {
            QUEEN_HULL_RADIUS * u.size() + 5.
        } else {
            12.
        };
        let centre = if u.queen() {
            QUEEN_HULL_HEIGHT * u.size()
        } else {
            0.
        };
        let low = (h + radius + 25. - centre).max(0.);
        goal.alt = if matches!(
            t.job,
            Job::Haul
                | Job::Return
                | Job::Inspect
                | Job::Intercept
                | Job::Escort
                | Job::Founder
                | Job::Queen
                | Job::Withdraw
        ) && distance < 4000.
        {
            goal.alt.max(low)
        } else {
            (mean + 800.).max(low)
        };
        if t.job == Job::Survey && distance < 4000. {
            t.goal.alt = goal.alt;
        }
        let start = Pos {
            alt: u.pos.alt + centre,
            ..u.pos
        };
        let end = Pos {
            alt: goal.alt + centre,
            ..goal
        };
        t.route.clear();
        t.route_tick = tick;
        self.probes_left -= 1;
        if clear(g, world, start, end, radius) {
            self.max_route_probes = self.max_route_probes.max(PROBES - self.probes_left);
            if self.search.as_ref().is_some_and(|s| s.unit == u.id) {
                self.search = None;
            }
            t.route.push(goal);
            return;
        }
        // One retained search per hive; other bodies still use validated direct
        // corridors. Waiting never discards unfinished work on each refresh.
        if self
            .search
            .as_ref()
            .is_some_and(|s| !g.units.iter().any(|b| b.alive && b.id == s.unit))
        {
            self.search = None;
        }
        if self.search.as_ref().is_some_and(|s| s.unit != u.id) {
            return;
        }
        let resume = self
            .search
            .as_ref()
            .is_some_and(|s| s.goal.distance(goal) < 100. && s.origin.distance(u.pos) < 50.);
        if !resume {
            self.nodes.clear();
            self.open.clear();
            self.search = Some(Search {
                unit: u.id.clone(),
                origin: u.pos,
                goal,
            });
            self.nodes.push(Node {
                x: 0,
                y: 0,
                level: 0,
                pos: u.pos,
                cost: 0,
                parent: 0,
                closed: false,
                edge: 0,
            });
            self.open.push(Reverse((0, 0)));
        }
        let origin = self.search.as_ref().unwrap().origin;
        let mut found = None;
        while self.nodes_left > 0 && self.probes_left > 0 {
            let Some(Reverse((_, index))) = self.open.pop() else {
                break;
            };
            if self.nodes[index].closed {
                continue;
            }
            let n = self.nodes[index];
            self.nodes_left -= 1;
            if n.edge == 0 && n.pos.distance(goal) < 1600. {
                self.probes_left -= 1;
                if clear(
                    g,
                    world,
                    Pos {
                        alt: n.pos.alt + centre,
                        ..n.pos
                    },
                    end,
                    radius,
                ) {
                    found = Some(index);
                    break;
                }
            }
            while self.nodes[index].edge < 24 && self.probes_left > 0 {
                let edge = self.nodes[index].edge;
                self.nodes[index].edge += 1;
                let (dx, dy) = [
                    (1, 0),
                    (0, 1),
                    (-1, 0),
                    (0, -1),
                    (1, 1),
                    (-1, 1),
                    (1, -1),
                    (-1, -1),
                ][edge as usize / 3];
                let level = edge % 3;
                let (x, y) = (n.x + dx, n.y + dy);
                if x.abs() > 8 || y.abs() > 8 {
                    continue;
                }
                let mut p = origin.offset(x as f64 * 1000., y as f64 * 1000., 0.);
                let Some((h, mean)) = ground(g, world, p) else {
                    continue;
                };
                p.alt = match level {
                    0 => (h + radius + 25. - centre).max(goal.alt.min(mean + 800.)),
                    1 => (mean + 950.).max(h + radius + 25. - centre),
                    _ => (origin.alt + 1000.).max(h + radius + 25. - centre),
                };
                let cost = n.cost + (n.pos.distance(p) * (1. + self.cost(p, tick).max(0.))) as u32;
                let existing = self
                    .nodes
                    .iter()
                    .position(|v| v.x == x && v.y == y && v.level == level);
                if existing.is_some_and(|i| self.nodes[i].cost <= cost || self.nodes[i].closed) {
                    continue;
                }
                self.probes_left -= 1;
                if !clear(
                    g,
                    world,
                    Pos {
                        alt: n.pos.alt + centre,
                        ..n.pos
                    },
                    Pos {
                        alt: p.alt + centre,
                        ..p
                    },
                    radius,
                ) {
                    continue;
                }
                if self.open.len() >= PROBES * 2 {
                    continue;
                }
                let next = if let Some(i) = existing {
                    self.nodes[i].cost = cost;
                    self.nodes[i].parent = index;
                    i
                } else {
                    if self.nodes.len() >= RETAINED_NODES {
                        continue;
                    }
                    let i = self.nodes.len();
                    self.nodes.push(Node {
                        x,
                        y,
                        level,
                        pos: p,
                        cost,
                        parent: index,
                        closed: false,
                        edge: 0,
                    });
                    i
                };
                self.open
                    .push(Reverse((cost + p.distance(goal) as u32, next)));
            }
            if self.nodes[index].edge == 24 {
                self.nodes[index].closed = true;
            } else {
                self.open
                    .push(Reverse((n.cost + n.pos.distance(goal) as u32, index)));
            }
        }
        self.max_route_probes = self.max_route_probes.max(PROBES - self.probes_left);
        if let Some(mut i) = found {
            t.route.push(goal);
            for _ in 0..31 {
                if i == 0 {
                    break;
                }
                t.route.push(self.nodes[i].pos);
                i = self.nodes[i].parent;
            }
            if i != 0
                || t.route.last().is_some_and(|p| {
                    !clear(
                        g,
                        world,
                        start,
                        Pos {
                            alt: p.alt + centre,
                            ..*p
                        },
                        radius,
                    )
                })
            {
                t.route.clear();
            }
            self.search = None;
        } else if self.open.is_empty() {
            self.search = None;
        }
    }
    fn command(
        &mut self,
        g: &mut Lab,
        world: &mut Swarm,
        i: usize,
        action: &str,
        mark: Option<u64>,
        tick: u64,
    ) -> bool {
        if matches!(action, "reproduce" | "found" | "grow")
            && world.land_ready(g.units[i].pos) != Some(true)
        {
            return false;
        }
        let c = Cmd {
            epoch: g.epoch,
            id: format!("ai-{}-{tick}-{}-{action}", g.epoch, g.units[i].id),
            action: action.into(),
            unit: Some(g.units[i].id.clone()),
            target: None,
            hive: Some(g.units[i].hive),
            amount: if action == "grow" {
                let q = &g.units[i];
                let reserve = self.reserve(g, q);
                Some(
                    (q.dry_mass * 2.).min(1e7).min(
                        q.dry_mass
                            + (q.minerals - BIRTH_M)
                                .max(0.)
                                .min(((q.energy - BIRTH_E - reserve) / 5e6).max(0.)),
                    ),
                )
            } else {
                None
            },
            mark,
        };
        let ok = g.command(&c).is_ok();
        if ok {
            match action {
                "reproduce" => self.births += 1,
                "strike" => self.shots += 1,
                _ => {}
            }
        }
        ok
    }
    fn queen(&mut self, g: &mut Lab, world: &mut Swarm, i: usize, tick: u64) {
        let q = &g.units[i];
        let reserve = self.reserve(g, q);
        let p = q.pos;
        if q.mode == "preparing" || q.mode == "rising" {
            g.units[i].heartbeat = g.time;
            return;
        }
        let warning = g.missiles.iter().find(|m| {
            m.target.distance(p) < BLAST_R + QUEEN_HULL_RADIUS * q.size() + 300.
                && (!q.nuclear_sheltered || m.precision)
        });
        let threat = warning.map(|m| m.target);
        if let Some(impact) = threat {
            if q.charge.is_some() {
                self.command(g, world, i, "cancel", None, tick);
            }
            if g.units[i].grounded {
                self.command(g, world, i, "lift", None, tick);
                return;
            }
            if g.units[i].mode == "landing" {
                g.units[i].hold_here();
            }
            let d = enu(impact, p);
            let norm = d.x.hypot(d.y);
            let (e, n) = if norm < 1. {
                (1., 0.)
            } else {
                (d.x / norm, d.y / norm)
            };
            self.tasks.insert(
                i,
                Task::new(
                    Job::Queen,
                    0,
                    p.offset(e * 2500., n * 2500., 0.),
                    tick,
                    10000,
                ),
            );
            return;
        }
        let mut mark = None;
        if q.energy > STRIKE_E + reserve && q.cooldown == 0. && q.charge.is_none() {
            for m in &g.marks {
                if m.hive != q.hive || m.expires <= g.time || m.accuracy_m > 10. {
                    continue;
                }
                let valid = self.contacts.values().any(|c| {
                    c.alive
                        && c.queen
                        && tick.saturating_sub(c.seen) <= if c.grounded { 2400 } else { 80 }
                        && c.grounded
                        && c.pos.offset(0., 0., c.radius * 0.4).distance(m.pos)
                            < PRECISION_R + c.radius - 10.
                        && c.v.length() < 3.
                });
                let friendly = self
                    .obs
                    .members
                    .iter()
                    .any(|f| f.pos.distance(m.pos) < BLAST_R + 150.);
                if valid && !friendly {
                    mark = Some(m.id);
                    break;
                }
            }
        }
        if let Some(mark) = mark {
            if q.grounded {
                self.command(g, world, i, "lift", None, tick);
                return;
            }
            if !q.radar_visible {
                let Some((_, h)) = ground(g, world, p) else {
                    return;
                };
                self.tasks.insert(
                    i,
                    Task::new(
                        Job::Queen,
                        mark,
                        Pos {
                            alt: h + 1100.,
                            ..p
                        },
                        tick,
                        4000,
                    ),
                );
            } else {
                self.command(g, world, i, "strike", Some(mark), tick);
                self.tasks.remove(&i);
            }
            return;
        }
        if q.charge.is_some() {
            return;
        }
        if !q.grounded
            && q.energy > STRIKE_E + reserve
            && self.contacts.values().any(|c| {
                c.alive
                    && c.queen
                    && c.grounded
                    && self
                        .obs
                        .members
                        .iter()
                        .any(|m| !m.queen && m.pos.distance(c.pos) < 3000.)
            })
        {
            let Some((_, h)) = ground(g, world, p) else {
                return;
            };
            let goal = Pos {
                alt: p.alt.max(h + 1100.),
                ..p
            };
            self.tasks
                .entry(i)
                .or_insert_with(|| Task::new(Job::Queen, 1, goal, tick, 2000));
            return;
        }
        let ready = q.energy > q.capacity * 0.85
            && q.energy > STRIKE_E + reserve
            && self.obs.members.iter().filter(|m| !m.queen).count() >= 6
            && self.contacts.values().any(|c| {
                c.alive
                    && c.queen
                    && tick.saturating_sub(c.seen) < 40 * 1200
                    && c.pos.distance(p) < 100000.
            });
        if ready && q.brood_remaining == 0. {
            if q.grounded {
                self.command(g, world, i, "lift", None, tick);
            } else if let Some((_, h)) = ground(g, world, p) {
                self.tasks.insert(
                    i,
                    Task::new(
                        Job::Queen,
                        1,
                        Pos {
                            alt: p.alt.max(h + 1100.),
                            ..p
                        },
                        tick,
                        2000,
                    ),
                );
            }
            return;
        }
        if q.grounded {
            self.tasks.remove(&i);
            if self.deposits.values().any(|d| {
                d.pos.distance(q.pos) < 120.
                    && ((d.energy > 0. && q.energy < q.capacity)
                        || (d.minerals > 0. && q.minerals < q.mineral_capacity()))
            }) {
                self.command(g, world, i, "collect", None, tick);
                return;
            }

            if q.dry_mass < q.growth_target {
                return;
            }
            if q.dry_mass < 1e7 && q.minerals > BIRTH_M + 1. && q.energy > reserve + BIRTH_E + 1e8 {
                self.command(g, world, i, "grow", None, tick);
                return;
            }
            let workers = self.obs.members.iter().filter(|m| !m.queen).count();
            // Value accessible finite stock in full-load equivalents. Object
            // count alone must never make splitting a pile profitable.
            let mut loads = 0.;
            let mut useful_energy = 0.;
            let mut useful_minerals = 0.;
            for d in self.deposits.values() {
                let trip = q.pos.distance(d.pos) / 50. * 2. + 30.;
                if trip > 600. {
                    continue;
                }
                useful_energy += d.energy;
                useful_minerals += d.minerals;
                loads += (d.energy / DRONE_CARGO_ENERGY).max(d.minerals / DRONE_CARGO_MINERALS)
                    * (300. / trip.max(30.)).min(1.);
            }
            let demand = (q.capacity - q.energy).max(0.) + BIRTH_E;
            let useful =
                useful_energy.min(demand) + useful_minerals.min(BIRTH_M) / BIRTH_M * BIRTH_E;
            let productive = workers == 0 || (loads > workers as f64 && useful > BIRTH_E);
            if workers < 24
                && productive
                && q.energy > BIRTH_E + reserve
                && q.minerals >= BIRTH_M
                && q.brood_remaining == 0.
                && g.population_used() < 100
            {
                self.command(g, world, i, "reproduce", None, tick);
            }
            return;
        }
        let mut site = p;
        let mut best = 0.;
        for n in 0..=16 {
            let a = n as f64 * std::f64::consts::TAU / 16.;
            let candidate = if n == 0 {
                p
            } else {
                p.offset(a.sin() * 2000., a.cos() * 2000., 0.)
            };
            if let Some((h, mean)) = ground(g, world, candidate) {
                let value = mean
                    - h
                    - 500.
                    - p.distance(candidate) * 0.05
                    - self.cost(candidate, tick) * 1000.;
                if value > best {
                    best = value;
                    site = Pos {
                        alt: h + 140.,
                        ..candidate
                    };
                }
            }
        }
        // Land is checked by the existing map service off the tick path. Keep
        // the site stable while that check completes and while approaching it.
        let t = self
            .tasks
            .entry(i)
            .or_insert_with(|| Task::new(Job::Queen, 0, site, tick, 500));
        if t.target == 0 && tick - t.since > 400 {
            t.goal = site;
            t.since = tick;
        }
        let goal = t.goal;
        if enu(p, goal).x.hypot(enu(p, goal).y) < 80.
            && g.units[i].v.length() < 5.
            && world.land_ready(goal) == Some(true)
        {
            self.tasks.remove(&i);
            self.command(g, world, i, "land", None, tick);
        } else {
            world.land_ready(goal);
        }
    }
    fn founder(&mut self, g: &mut Lab, world: &mut Swarm, i: usize, tick: u64) {
        let u = &g.units[i];
        if u.construction_remaining > 0. {
            g.units[i].heartbeat = g.time;
            return;
        }
        if u.energy < u.capacity * 0.5 && u.cargo_energy > 0. {
            self.command(g, world, i, "refuel_self", None, tick);
        }
        let u = &g.units[i];
        if u.cargo_minerals >= FOUNDER_MINERALS && u.energy + u.cargo_energy >= FOUNDER_E + 4.5e9 {
            if u.grounded {
                self.command(g, world, i, "found", None, tick);
            } else if u.v.length() < 10.
                && self.tasks.get(&i).is_some_and(|t| t.job == Job::Founder)
                && world.land_ready(self.tasks[&i].goal) == Some(true)
            {
                self.command(g, world, i, "land", None, tick);
            } else {
                let p = u.pos;
                if self.tasks.get(&i).is_none_or(|t| t.job != Job::Founder) {
                    self.tasks
                        .insert(i, Task::new(Job::Founder, 0, p, tick, 1000));
                }
                world.land_ready(p);
            }
            return;
        }
        if u.grounded {
            self.command(g, world, i, "lift_founder", None, tick);
            return;
        }
        let candidate = self
            .deposits
            .iter()
            .filter(|(_, d)| d.energy > 0. || d.minerals > 0.)
            .min_by(|a, b| u.pos.distance(a.1.pos).total_cmp(&u.pos.distance(b.1.pos)));
        if let Some((&id, d)) = candidate {
            let goal = Pos {
                alt: d.pos.alt + 80.,
                ..d.pos
            };
            self.tasks
                .entry(i)
                .or_insert_with(|| Task::new(Job::Haul, id, goal, tick, 1500));
        } else {
            let p = u.pos.offset(3000., 2000., 0.);
            self.tasks
                .entry(i)
                .or_insert_with(|| Task::new(Job::Survey, sector(p), p, tick, 500));
        }
    }
    fn execute(
        &mut self,
        g: &mut Lab,
        world: &mut Swarm,
        i: usize,
        t: &mut Task,
        tick: u64,
    ) -> bool {
        let u = &g.units[i];
        if u.grounded || matches!(u.mode.as_str(), "preparing" | "rising" | "landing") {
            g.units[i].heartbeat = g.time;
            return false;
        }
        match t.job {
            Job::Haul => {
                let Some(d) = self.deposits.get(&t.target).copied() else {
                    return true;
                };
                t.goal = Pos {
                    alt: d.pos.alt + 80.,
                    ..d.pos
                };
                if u.pos.distance(d.pos) < 115.
                    && u.v.length() <= 8.
                    && u.v.z.abs() < 2.
                    && tick - t.action_tick >= 10
                {
                    t.action_tick = tick;
                    self.command(g, world, i, "scan", None, tick);
                    if self.command(g, world, i, "collect", None, tick) {
                        self.trace(g, i, t, tick, "collected");
                        return true;
                    }
                    self.deposits.remove(&t.target);
                    return true;
                }
            }
            Job::Return => {
                let Some(q) = self.obs.members.iter().find(|m| m.queen).copied() else {
                    return true;
                };
                let d = enu(q.pos, u.pos);
                let n = d.x.hypot(d.y).max(1.);
                t.goal = q.pos.offset(d.x / n * 190., d.y / n * 190., 30.);
                if u.pos.distance(q.pos) < 270. && u.v.length() < 15. && tick - t.action_tick >= 10
                {
                    let before = g.units[i].cargo_energy;
                    let before_minerals = g.units[i].cargo_minerals;
                    let qbefore = g.units[q.index].energy;
                    t.action_tick = tick;
                    if self.command(g, world, i, "deliver", None, tick) {
                        let delivered = before - g.units[i].cargo_energy;
                        let minerals = before_minerals - g.units[i].cargo_minerals;
                        self.deliveries += u64::from(delivered > 0. || minerals > 0.);
                        self.delivered += (g.units[q.index].energy - qbefore).max(0.);
                        if delivered > 0. || minerals > 0. {
                            let strength = ((delivered / DRONE_CARGO_ENERGY)
                                .max(minerals / DRONE_CARGO_MINERALS)
                                .clamp(0., 1.)
                                * 1000.) as u16;
                            let count = self.drives.get(&i).map_or(0, |d| d.path.len());
                            for n in 0..=count {
                                let key = if n == count {
                                    sector(t.goal)
                                } else {
                                    self.drives[&i].path[n]
                                };
                                let m = self.memory(key);
                                m.trail = ((decayed(m.trail, m.trail_at, tick, 4800) * 1000.)
                                    as u16)
                                    .saturating_add(strength)
                                    .min(1000);
                                m.trail_at = tick;
                                m.visited = tick;
                            }
                            if let Some(d) = self.drives.get_mut(&i) {
                                d.path.clear();
                            }
                            self.trace(g, i, t, tick, "delivered");
                        }
                        // One recheck after refuelling can use newly freed capacity.
                        if g.units[i].cargo_energy > 0.
                            && g.units[q.index].energy < g.units[q.index].capacity
                        {
                            self.command(g, world, i, "deliver", None, tick);
                        }
                    }
                    return true;
                }
            }
            Job::Survey => {
                if tick - t.action_tick >= 120 {
                    t.action_tick = tick;
                    self.command(g, world, i, "scan", None, tick);
                }
                if g.units[i].pos.distance(t.goal) < 250. {
                    let m = self.memory(t.target);
                    m.visited = tick;
                    self.trace(g, i, t, tick, "surveyed");
                    return true;
                }
            }
            Job::Inspect | Job::Intercept => {
                let Some(c) = self.contacts.get(&t.target).copied() else {
                    return true;
                };
                if !c.alive {
                    return true;
                }
                t.goal = c.pos;
                if c.queen {
                    let d = enu(c.pos, g.units[i].pos);
                    let n = d.x.hypot(d.y).max(1.);
                    let e = if n <= 1. { 1. } else { d.x / n };
                    let north = if n <= 1. { 0. } else { d.y / n };
                    t.goal = c.pos.offset(
                        e * (c.radius + 25.),
                        north * (c.radius + 25.),
                        c.radius * 0.4,
                    );
                    if g.units[i].pos.distance(c.pos) < c.radius + PRECISION_R - 5.
                        && g.units[i].v.length() <= 15.
                        && tick.saturating_sub(c.seen) < 80
                        && tick - t.action_tick >= 10
                    {
                        t.action_tick = tick;
                        if self.command(g, world, i, "mark", None, tick) {
                            let d = enu(c.pos, g.units[i].pos);
                            let n = d.x.hypot(d.y).max(1.);
                            let e = if n <= 1. { 1. } else { d.x / n };
                            let north = if n <= 1. { 0. } else { d.y / n };
                            t.job = Job::Withdraw;
                            t.goal =
                                c.pos
                                    .offset(e * (BLAST_R + 350.), north * (BLAST_R + 350.), 100.);
                            t.score = 10000;
                            t.since = tick;
                            t.route_tick = 0;
                            self.trace(g, i, t, tick, "marked_and_withdrawing");
                        }
                    }
                } else {
                    let lead = (g.units[i].pos.distance(c.pos) / 150.).clamp(0., 2.);
                    t.goal = c.pos.offset(c.v.x * lead, c.v.y * lead, c.v.z * lead);
                }
            }
            Job::Withdraw => {
                if u.pos.distance(t.goal) < 100.
                    && tick - t.since > 2400
                    && !g.units.iter().any(|q| {
                        q.alive
                            && q.hive == u.hive
                            && q.charge
                                .as_ref()
                                .is_some_and(|c| c.target.distance(t.goal) < 2000.)
                    })
                    && !g
                        .missiles
                        .iter()
                        .any(|m| m.hive == u.hive && m.target.distance(t.goal) < 2000.)
                    && !g
                        .bursts
                        .iter()
                        .any(|b| b.kind == "nuclear" && b.pos.distance(t.goal) < 2000.)
                {
                    return true;
                }
            }
            Job::Escort => {
                let Some(friend) = self
                    .obs
                    .members
                    .iter()
                    .find(|m| m.index == t.target as usize)
                else {
                    return true;
                };
                if friend.cargo <= 0. {
                    return true;
                }
                t.goal = friend
                    .pos
                    .offset(150. + friend.v.x * 2., -150. + friend.v.y * 2., 100.);
            }
            _ => {}
        }
        let u = &g.units[i];
        if matches!(t.job, Job::Haul | Job::Return) {
            let path = &mut self.drives.entry(i).or_default().path;
            let key = sector(u.pos);
            if path.back() != Some(&key) {
                if path.len() == 32 {
                    path.pop_front();
                }
                path.push_back(key);
            }
        }
        let distance = u.pos.distance(t.goal);
        if distance + 10. < t.distance {
            t.distance = distance;
            t.progress = tick;
        }
        if tick - t.progress > 1200 && !matches!(t.job, Job::Queen | Job::Founder | Job::Withdraw) {
            self.trace(g, i, t, tick, "no_progress");
            if self.blocked.len() >= 128 {
                self.blocked.pop_first();
            }
            self.blocked.insert(t.target, tick + 1200);
            return true;
        }
        if t.route_tick == 0 || tick.saturating_sub(t.route_tick) >= 40 {
            self.route(g, world, u, t, tick);
        }
        while t.route.last().is_some_and(|p| u.pos.distance(*p) < 15.) {
            t.route.pop();
        }
        let destination = t.route.last().copied().unwrap_or(u.pos);
        let safe = if t.route.is_empty() {
            None
        } else {
            Some(destination)
        };
        steer(
            g,
            world,
            i,
            safe,
            self.obs.members.as_slice(),
            self.drives.entry(i).or_default(),
        );
        if t.job == Job::Intercept && (tick + i as u64) % 10 == 0 {
            if let Some(c) = self.contacts.get(&t.target).copied() {
                if tick.saturating_sub(c.seen) < 20 && g.units[i].pos.distance(c.pos) < 350. {
                    aim(g, i, c, &self.obs.members);
                }
            }
        }
        false
    }
}
fn steer(
    g: &mut Lab,
    world: &mut Swarm,
    i: usize,
    goal: Option<Pos>,
    friends: &[Member],
    drive: &mut Drive,
) {
    let u = &g.units[i];
    let p = u.pos;
    let mut d = goal.map(|q| enu(p, q)).unwrap_or(Vec3::ZERO);
    // A prepared stopping corridor is required before accelerating out of the
    // current safe region. Missing terrain never becomes a fictional surface.
    let stop = u.v.x.hypot(u.v.y).powi(2) / 4. + 100.;
    let speed = u.v.x.hypot(u.v.y);
    let ahead = if speed > 1. {
        p.offset(
            u.v.x / speed * stop.min(4000.),
            u.v.y / speed * stop.min(4000.),
            0.,
        )
    } else {
        p
    };
    if ground(g, world, ahead).is_none() {
        d.x = 0.;
        d.y = 0.;
    }
    let mut desired = Vec3::new(d.x * 0.2, d.y * 0.2, d.z * 0.8);
    let cruise = (if u.queen() { 55f64 } else { 70f64 }).min((4. * d.x.hypot(d.y)).sqrt());
    let length = desired.x.hypot(desired.y);
    if length > cruise {
        desired.x *= cruise / length;
        desired.y *= cruise / length;
    }
    desired.z = desired.z.clamp(
        if u.queen() { -30. } else { -15. },
        if u.queen() { 30. } else { 15. },
    );
    for f in friends {
        if f.index == i {
            continue;
        }
        let delta = enu(f.pos, p);
        let distance = delta.length();
        let spacing = if f.queen {
            QUEEN_HULL_RADIUS + 35.
        } else {
            30.
        };
        if distance > 0.1 && distance < spacing {
            desired += delta / distance * (spacing - distance) * 0.5;
        }
    }
    if let Some((h, _)) = ground(g, world, p) {
        let minimum = h + if u.queen() { 100. * u.size() } else { 25. };
        if p.alt < minimum {
            desired.z = 15.;
        }
    }
    let gravity = gravity(p);
    let lift_margin = 3. * gravity * DRONE_MASS / u.mass();
    if !u.queen() && desired.z < 0. {
        // A loaded worker has little excess lift. Its commanded descent must
        // leave enough distance to arrest through the actual finite actuator.
        desired.z = desired.z.max(-(lift_margin * (-d.z).max(0.)).sqrt());
    }
    let mut a = (desired - u.v) * 1.5;
    let max = if u.queen() {
        2.5
    } else {
        (lift_margin * 0.6).min(5.)
    };
    let horizontal = a.x.hypot(a.y);
    if horizontal > max {
        a.x *= max / horizontal;
        a.y *= max / horizontal;
    }
    let heading = if desired.x.hypot(desired.y) > 2. {
        desired.x.atan2(desired.y)
    } else {
        u.heading
    };
    let yaw = (heading - u.heading)
        .sin()
        .atan2((heading - u.heading).cos());
    let forward = a.x * u.heading.sin() + a.y * u.heading.cos();
    let right = a.x * u.heading.cos() - a.y * u.heading.sin();
    let pitch = -forward.atan2(gravity);
    let roll = right.atan2(gravity);
    let margin = if u.queen() { 0.004 } else { 0.025 };
    let mut input = Input {
        forward: u.pitch > pitch + margin,
        back: u.pitch < pitch - margin,
        strafe_left: u.roll > roll + margin,
        strafe_right: u.roll < roll - margin,
        yaw_left: yaw < -0.025,
        yaw_right: yaw > 0.025,
        ..Default::default()
    };
    if u.queen() {
        input.alt_hold = true;
        input.thrust_up = u.v.z < desired.z - 1.;
        input.thrust_down = u.v.z > desired.z + 1.;
    } else {
        let drag = drag_acceleration(false, u.mass(), p.alt, u.v);
        let target = (gravity + drag.z + (desired.z - u.v.z) * 2.).max(0.);
        let full = gravity + 3. * gravity * DRONE_MASS / u.mass();
        let vertical = (u.pitch.cos() * u.roll.cos()).max(0.1);
        drive.throttle += (target / (full * vertical)).clamp(0., 1.);
        input.thrust_up = drive.throttle >= 1.;
        if input.thrust_up {
            drive.throttle -= 1.;
        }
    }
    let time = g.time;
    let u = &mut g.units[i];
    u.mode.clear();
    u.mode.push_str("manual");
    u.input = input;
    u.heartbeat = time;
}
fn aim(g: &mut Lab, i: usize, c: Contact, friends: &[Member]) {
    let u = &g.units[i];
    let d = enu(u.pos, c.pos);
    let time = (d.length() / 300.).clamp(0.01, 1.2);
    let relative = Vec3::new(
        d.x + (c.v.x - u.v.x) * time,
        d.y + (c.v.y - u.v.y) * time,
        d.z + (c.v.z - u.v.z) * time + 0.5 * gravity(u.pos) * time * time,
    );
    let heading = relative.x.atan2(relative.y);
    let pitch = relative.z.atan2(relative.x.hypot(relative.y));
    let yaw = (heading - u.heading)
        .sin()
        .atan2((heading - u.heading).cos());
    // Roll out the real relative launch, gravity and drag through Rapier.
    // Test moving friendly hulls against each curved-flight segment.
    let aligned = yaw.abs() < 0.04 && (u.pitch - pitch).abs() < 0.04;
    let safe = aligned && u.reload == 0. && shot_is_safe(g, i, c, friends);
    let current_pitch = u.pitch;
    let input = &mut g.units[i].input;
    input.yaw_left = yaw < -0.02;
    input.yaw_right = yaw > 0.02;
    input.forward = current_pitch > pitch + 0.02;
    input.back = current_pitch < pitch - 0.02;
    input.fire = safe && yaw.abs() < 0.04 && (current_pitch - pitch).abs() < 0.04;
}
fn shot_is_safe(g: &Lab, i: usize, c: Contact, friends: &[Member]) -> bool {
    let u = &g.units[i];
    let basis = u.pos.basis();
    let to_world = |v: Vec3| basis[0] * v.x + basis[1] * v.y + basis[2] * v.z;
    let direction = Vec3::new(
        u.heading.sin() * u.pitch.cos(),
        u.heading.cos() * u.pitch.cos(),
        u.pitch.sin(),
    );
    let mut p = u.pos.ecef();
    let mut v = to_world(u.v + direction * 300.);
    let target_basis = c.pos.basis();
    let target_v = target_basis[0] * c.v.x + target_basis[1] * c.v.y + target_basis[2] * c.v.z;
    let mut hit = false;
    for step in 0..48 {
        let old = p;
        (p, v) = crate::physics::advance(p, v, spit_acceleration(p, v), DT);
        let t0 = step as f64 * DT;
        let t1 = t0 + DT;
        for f in friends {
            if f.index == i {
                continue;
            }
            let (hull, radius) = g.units[f.index].hull();
            let b = f.pos.basis();
            let fv = b[0] * f.v.x + b[1] * f.v.y + b[2] * f.v.z;
            if segment_sphere(old - fv * t0, p - fv * t1, hull.ecef(), radius + 5.).is_some() {
                return false;
            }
        }
        if g.terrain_index.hit(&g.patches, old, p).is_some() {
            return false;
        }
        if segment_sphere(
            old - target_v * t0,
            p - target_v * t1,
            c.pos.ecef(),
            c.radius,
        )
        .is_some()
        {
            hit = true;
            break;
        }
    }
    hit
}
impl Lab {
    pub(super) fn ai_step(&mut self) {
        self.ai.tick += 1;
        if self.ai.brains.is_empty() {
            return;
        }
        let started = std::time::Instant::now();
        let mut world = std::mem::take(&mut self.ai);
        let tick = world.tick;
        if world.manual.as_ref().is_some_and(|id| {
            self.units
                .iter()
                .find(|u| &u.id == id)
                .is_none_or(|u| !u.alive || self.time - u.heartbeat > 2.)
        }) {
            world.manual = None;
        }
        world.prepare.retain(|p| {
            !self.envelope.ready(p.lat, p.lon)
                || self.terrain_index.elevation(&self.patches, *p).is_none()
        });
        let mut brains = std::mem::take(&mut world.brains);
        for (&h, b) in &mut brains {
            if b.route_window != tick / 40 {
                b.route_window = tick / 40;
                b.nodes_left = ROUTE_NODES;
                b.probes_left = PROBES;
            }
            b.drives
                .retain(|&i, _| self.units.get(i).is_some_and(|u| u.alive && u.hive == h));
            if tick % 10 == h as u64 % 10 || b.obs.members.is_empty() {
                let start = std::time::Instant::now();
                b.observe(self, h, tick);
                b.allocate(self, h, tick, world.manual.as_deref());
                b.max_plan_micros = b.max_plan_micros.max(start.elapsed().as_micros() as u64);
            }
            if b.search.as_ref().is_some_and(|s| {
                !b.tasks.keys().any(|&i| self.units[i].id == s.unit)
                    || world.manual.as_deref() == Some(s.unit.as_str())
            }) {
                b.search = None;
            }
            for member in 0..b.obs.members.len() {
                let i = b.obs.members[member].index;
                if !self.units[i].alive
                    || self.units[i].hive != h
                    || world.manual.as_deref() == Some(self.units[i].id.as_str())
                {
                    b.tasks.remove(&i);
                    continue;
                }
                if tick % 10 == h as u64 % 10 {
                    if self.units[i].queen() {
                        b.queen(self, &mut world, i, tick);
                    } else if self.units[i].founder {
                        b.founder(self, &mut world, i, tick);
                    }
                }
                if let Some(mut task) = b.tasks.get_mut(&i).map(|t| {
                    std::mem::replace(
                        t,
                        Task {
                            job: Job::Hold,
                            target: 0,
                            goal: Pos::default(),
                            since: 0,
                            progress: 0,
                            distance: 0.,
                            score: 0,
                            route: Vec::new(),
                            route_tick: 0,
                            action_tick: 0,
                        },
                    )
                }) {
                    let done = b.execute(self, &mut world, i, &mut task, tick);
                    if done {
                        b.tasks.remove(&i);
                    } else {
                        *b.tasks.get_mut(&i).unwrap() = task;
                    }
                } else if self.units[i].mode == "manual" {
                    steer(
                        self,
                        &mut world,
                        i,
                        None,
                        &b.obs.members,
                        b.drives.entry(i).or_default(),
                    );
                }
                if tick % 120 == h as u64 % 120
                    && !self.units[i].queen()
                    && self.units[i].scan_until <= self.time
                {
                    b.command(self, &mut world, i, "scan", None, tick);
                }
            }
        }
        brains.retain(|h, _| self.units.iter().any(|u| u.alive && u.hive == *h));
        world.brains = brains;
        world.max_tick_micros = world
            .max_tick_micros
            .max(started.elapsed().as_micros() as u64);
        self.ai = world;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world(workers: usize) -> Lab {
        let mut g = new();
        g.epoch = 7;
        g.envelope = Envelope::for_test(|_, _| 0.);
        g.patches.push(Terrain {
            lat: 0.,
            lon: 0.,
            step: 250.,
            size: 65,
            heights: vec![0.; 65 * 65],
            ..Default::default()
        });
        g.terrain_index.ensure(&g.patches);
        g.add_hive(
            Pos {
                lat: 0.,
                lon: 0.,
                alt: 0.,
            },
            workers,
            START_QUEEN_E,
            START_QUEEN_MINERALS,
        );
        g.units[0].pos.alt = 0.;
        g.units[0].grounded = true;
        g.units[0].hold = g.units[0].pos;
        g
    }
    fn enable(g: &mut Lab, h: u32) {
        let c = Cmd {
            epoch: g.epoch,
            id: format!("enable-{h}"),
            action: "ai_enable".into(),
            unit: None,
            target: None,
            hive: Some(h),
            amount: None,
            mark: None,
        };
        g.command(&c).unwrap();
    }
    fn run(g: &mut Lab, seconds: f64) {
        for _ in 0..(seconds / DT) as usize {
            while let Some(p) = g.ai.land_requests.pop_front() {
                g.ai.record_land(p, true);
            }
            g.tick(DT);
        }
    }
    #[test]
    fn hidden_world_changes_cannot_change_observations_or_jobs() {
        let mut g = world(3);
        let mut enemy = Body::new(
            "q999".into(),
            1,
            true,
            Pos {
                lat: 30.,
                lon: 120.,
                alt: 100.,
            },
        );
        enemy.energy = 1.;
        g.units.push(enemy);
        g.resources.push(Resource {
            id: 900,
            pos: Pos {
                lat: 30.,
                lon: 120.,
                alt: 0.,
            },
            energy: 1.,
            minerals: 1.,
            fall_speed: 0.,
            discovered: vec![],
        });
        let mut a = Brain::new(0);
        a.observe(&g, 0, 1);
        a.allocate(&g, 0, 1, None);
        g.units.last_mut().unwrap().pos.lon = 130.;
        g.units.last_mut().unwrap().energy = 1e14;
        g.resources[0].energy = 1e14;
        let mut b = Brain::new(0);
        b.observe(&g, 0, 1);
        b.allocate(&g, 0, 1, None);
        assert!(a.contacts.is_empty());
        assert!(a.deposits.is_empty());
        assert_eq!(
            serde_json::to_value(&a.tasks).unwrap(),
            serde_json::to_value(&b.tasks).unwrap()
        );
    }
    #[test]
    fn partial_stock_is_reserved_once_and_commitment_does_not_leave_duplicate_jobs() {
        let g = world(3);
        let mut b = Brain::new(1);
        b.observe(&g, 0, 1);
        b.deposits.insert(
            99,
            Deposit {
                pos: Pos {
                    lat: 0.,
                    lon: 0.,
                    alt: 0.,
                },
                energy: DRONE_CARGO_ENERGY / 2.,
                minerals: 0.,
                seen: 1,
            },
        );
        for tick in [1, 10, 200, 400] {
            b.allocate(&g, 0, tick, None);
            assert_eq!(b.tasks.values().filter(|t| t.job == Job::Haul).count(), 1);
            assert_eq!(b.reserved[&99].0, DRONE_CARGO_ENERGY / 2.);
        }
    }
    #[test]
    fn manual_takeover_tutorial_and_disable_are_immediate() {
        let mut g = world(3);
        assert!(g.view(None).ai_hives.is_empty());
        enable(&mut g, 0);
        run(&mut g, 1.);
        let id = g.units[1].id.clone();
        g.command(&Cmd {
            epoch: g.epoch,
            id: "manual".into(),
            action: "pilot".into(),
            unit: Some(id.clone()),
            target: None,
            hive: Some(0),
            amount: None,
            mark: None,
        })
        .unwrap();
        run(&mut g, 0.5);
        assert_eq!(g.ai.manual.as_deref(), Some(id.as_str()));
        assert!(!g.ai.brains[&0].tasks.contains_key(&1));
        assert!(g.ai.brains[&0].tasks.contains_key(&2));
        let c = Cmd {
            epoch: g.epoch,
            id: "lesson".into(),
            action: "tutorial".into(),
            unit: Some(id.clone()),
            target: None,
            hive: Some(0),
            amount: None,
            mark: None,
        };
        g.command(&c).unwrap();
        let tick = g.ai.tick;
        let p = g.units[2].pos;
        run(&mut g, 2.);
        assert_eq!(g.ai.tick, tick);
        assert_eq!(g.units[2].pos.lon, p.lon);
        g.command(&Cmd {
            id: "finish".into(),
            action: "tutorial_end".into(),
            ..c
        })
        .unwrap();
        g.command(&Cmd {
            epoch: g.epoch,
            id: "off".into(),
            action: "ai_disable".into(),
            unit: None,
            target: None,
            hive: Some(0),
            amount: None,
            mark: None,
        })
        .unwrap();
        assert!(g.ai.brains.is_empty());
        assert_eq!(g.units[2].mode, "hold");
    }
    #[test]
    fn scout_marks_withdraws_and_queen_launches_a_legal_precision_strike() {
        let mut g = world(1);
        g.units[0].grounded = false;
        g.units[0].pos.alt = 1100.;
        g.units[0].hold = g.units[0].pos;
        g.units[0].energy = QUEEN_E;
        let enemy = Pos {
            lat: 0.,
            lon: 0.012,
            alt: 0.,
        };
        let mut q = Body::new("q999".into(), 1, true, enemy);
        q.grounded = true;
        q.hold = enemy;
        g.units.push(q);
        g.units[1].pos = enemy.offset(-170., 0., 45.);
        g.units[1].hold = g.units[1].pos;
        enable(&mut g, 0);
        run(&mut g, 180.);
        eprintln!("precision chain {}", g.ai.status(0));
        assert!(
            g.ai.brains[&0].shots > 0,
            "queen never launched a marked strike"
        );
        assert!(!g.units[2].alive, "precision did not kill grounded enemy");
        assert!(g.units[1].alive, "scout failed to withdraw");
        assert!(g.units[0].alive);
    }
    #[test]
    fn succession_preserves_stores_and_physically_founds_a_queen() {
        let mut g = world(1);
        g.units[1].pos = Pos {
            lat: 0.,
            lon: 0.002,
            alt: 60.,
        };
        g.units[1].hold = g.units[1].pos;
        g.units[1].cargo_energy = 200e9;
        g.units[1].cargo_minerals = FOUNDER_MINERALS;
        enable(&mut g, 0);
        let energy = g.units[1].energy + g.units[1].cargo_energy;
        g.units[0].alive = false;
        g.settle_deaths();
        g.succession();
        assert_eq!(g.units[1].energy + g.units[1].cargo_energy, energy);
        run(&mut g, 250.);
        eprintln!(
            "founder: kind={} grounded={} construction={} energy={} cargo={} ai={:?}",
            g.units[1].kind,
            g.units[1].grounded,
            g.units[1].construction_remaining,
            g.units[1].energy,
            g.units[1].cargo_energy,
            g.view(None).ai_hives
        );
        assert!(g.units[1].queen(), "orphan never completed legal founding");
        assert!(g.units[1].dry_mass >= 1000.);
        assert!(
            g.units[1].brood_remaining > 0.,
            "new colony must preserve resources and begin its first brood"
        );
    }
    #[test]
    fn route_waits_for_missing_data_and_budgets_are_shared_per_hive() {
        let mut g = world(3);
        g.patches.clear();
        g.terrain_index.rebuild(&g.patches);
        enable(&mut g, 0);
        let p = g.units[1].pos;
        run(&mut g, 5.);
        assert_eq!(g.units[1].pos.lon, p.lon);
        assert!(!g.ai.prepare.is_empty());
        let b = &g.ai.brains[&0];
        assert!(b.max_route_probes <= PROBES);
        assert!(b.bids.len() <= 12 * 3);
        assert!(g.units.iter().all(|u| u.alive));
    }
    #[test]
    fn blocked_corridor_search_resumes_and_returns_hull_clear_edges() {
        let mut g = world(1);
        for y in 0..65 {
            for x in 34..=36 {
                g.patches[0].heights[y * 65 + x] = 1200.;
            }
        }
        g.patches[0].shape_cache = std::sync::OnceLock::new();
        g.terrain_index.rebuild(&g.patches);
        g.units[1].pos = Pos {
            lat: 0.,
            lon: 0.,
            alt: 300.,
        };
        let u = &g.units[1];
        let mut world = Swarm::default();
        let mut b = Brain::new(0);
        let mut task = Task::new(Job::Haul, 42, u.pos.offset(3000., 0., -250.), 1, 1000);
        b.nodes_left = 1;
        b.probes_left = 3;
        b.route(&g, &mut world, u, &mut task, 1);
        assert!(task.route.is_empty());
        assert!(b.search.is_some(), "unfinished search must persist");
        let mut retained = b.nodes.len();
        for tick in 1..20 {
            b.nodes_left = ROUTE_NODES;
            b.probes_left = PROBES;
            b.route(&g, &mut world, u, &mut task, tick * 40);
            retained = retained.max(b.nodes.len());
            if !task.route.is_empty() {
                break;
            }
        }
        assert!(
            !task.route.is_empty(),
            "prepared ridge route never completed"
        );
        assert!(retained > 1 && retained <= RETAINED_NODES);
        let mut previous = u.pos;
        for &next in task.route.iter().rev() {
            assert!(clear(&g, &mut world, previous, next, 12.));
            previous = next;
        }
    }
    #[test]
    fn spit_rollout_respects_ballistics_and_moving_friendlies() {
        let mut g = world(2);
        g.units[1].pos = Pos {
            lat: 0.,
            lon: 0.,
            alt: 300.,
        };
        g.units[1].heading = 0.;
        g.units[1].pitch = 0.005;
        g.units[1].v = Vec3::ZERO;
        g.units[2].pos = g.units[1].pos.offset(200., 0., 0.);
        let c = Contact {
            pos: g.units[1].pos.offset(0., 100., 0.),
            v: Vec3::ZERO,
            queen: false,
            grounded: false,
            radius: 8.,
            seen: 1,
            alive: true,
        };
        let mut b = Brain::new(0);
        b.observe(&g, 0, 1);
        assert!(shot_is_safe(&g, 1, c, &b.obs.members));
        g.units[2].pos = g.units[1].pos.offset(0., 50., 0.);
        b.observe(&g, 0, 1);
        assert!(!shot_is_safe(&g, 1, c, &b.obs.members));
        g.units[2].pos = g.units[1].pos.offset(25., 50., 0.);
        g.units[2].v = Vec3::new(-150., 0., 0.);
        b.observe(&g, 0, 1);
        assert!(!shot_is_safe(&g, 1, c, &b.obs.members));
    }
    #[test]
    fn integrated_interceptor_kills_an_observed_enemy_worker() {
        let mut g = world(1);
        let p = Pos {
            lat: 0.,
            lon: 0.,
            alt: 300.,
        };
        g.units[1].pos = p;
        g.units[1].hold = p;
        let mut enemy = Body::new("d999".into(), 1, false, p.offset(0., 150., 0.));
        enemy.hold = enemy.pos;
        g.units.push(enemy);
        enable(&mut g, 0);
        run(&mut g, 30.);
        assert!(
            !g.units[2].alive,
            "autonomous interceptor never landed a real shot: {}",
            g.ai.status(0)
        );
        assert!(g.units[1].alive);
    }
    #[test]
    fn identical_inputs_produce_identical_tasks_and_trajectories() {
        let mut a = world(3);
        let mut b = world(3);
        enable(&mut a, 0);
        enable(&mut b, 0);
        run(&mut a, 20.);
        run(&mut b, 20.);
        assert_eq!(
            serde_json::to_value(&a.ai.brains[&0].tasks).unwrap(),
            serde_json::to_value(&b.ai.brains[&0].tasks).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&a.ai.brains[&0].trace).unwrap(),
            serde_json::to_value(&b.ai.brains[&0].trace).unwrap()
        );
        for (a, b) in a.units.iter().zip(&b.units) {
            assert_eq!(a.pos.ecef(), b.pos.ecef());
            assert_eq!(a.energy, b.energy);
        }
    }
    #[test]
    fn empty_terrarium_and_population_cap_keep_planning_bounded() {
        for workers in [11, 99] {
            let mut g = world(workers);
            let patch = g.patches.remove(0);
            g.patches.push(crate::physics::prepare_terrain(patch));
            enable(&mut g, 0);
            run(&mut g, 5.);
            let mut samples = Vec::with_capacity(400);
            let allocations = super::super::tests::heap::count(|| {
                // Controller-only cost with warm static geography and fixed
                // observed poses. It excludes rendering, I/O and flight solver.
                for _ in 0..400 {
                    let start = std::time::Instant::now();
                    g.ai_step();
                    samples.push(start.elapsed().as_nanos() as u64);
                    g.time += DT;
                }
            });
            samples.sort_unstable();
            let b = &g.ai.brains[&0];
            println!("{} bodies controller-only/debug: p50={}us p95={}us max={}us allocations={} over 400 ticks; bids={} memory={} route_probes={}",
                workers+1,samples[200]/1000,samples[380]/1000,samples[399]/1000,allocations,b.bids.len(),b.memory.len(),b.max_route_probes);
            assert!(b.bids.len() <= 12 * workers);
            assert!(b.nodes.len() <= RETAINED_NODES && b.open.len() <= PROBES * 2);
            assert!(
                b.memory.len() <= MEMORY && b.trace.len() <= 256 && b.max_route_probes <= PROBES
            );
            assert_eq!(
                b.births, 0,
                "no productive stock means no speculative brood"
            );
            assert!(g.units.iter().all(|u| u.alive));
        }
        let mut b = Brain::new(0);
        for key in 0..5000 {
            b.memory(key).visited = key;
        }
        assert_eq!(b.memory.len(), MEMORY);
        assert!(!b.memory.contains_key(&0));
        assert_eq!(decayed(1000, 0, 2400, 4800), 0.5);
        assert_eq!(
            decayed(1000, 0, 2400, 4800),
            0.5,
            "reads must not restart decay"
        );
    }
    #[test]
    fn incoming_warning_interrupts_landing_and_queen_physically_evades() {
        let mut g = world(1);
        let target = Pos {
            lat: 0.,
            lon: 0.,
            alt: 500.,
        };
        let start = target.offset(5000., 0., 1500.);
        g.units[0].pos = target;
        g.units[0].grounded = false;
        g.units[0].mode = "landing".into();
        g.units[0].energy = QUEEN_E;
        let (velocity, duration, path) = plan(start, target).unwrap();
        g.missiles.push(Missile {
            precision: false,
            id: 900,
            hive: 1,
            pos: start,
            start,
            target,
            age: 0.,
            duration,
            path,
            p: start.ecef(),
            velocity,
        });
        enable(&mut g, 0);
        run(&mut g, 70.);
        assert!(
            g.units[0].alive,
            "queen failed to respond to global warning"
        );
        assert!(g.units[0].pos.distance(target) > BLAST_R + QUEEN_HULL_RADIUS);
        assert!(
            g.units[0].energy < QUEEN_E,
            "evasion must pay normal flight cost"
        );
        assert!(g.missiles.is_empty(), "fixture strike must actually finish");
    }
    #[test]
    fn private_concentrations_decay_and_breeding_ignores_pile_count() {
        let mut g = world(1);
        let mut b = Brain::new(0);
        b.memory(sector(g.units[1].pos)).trail = 1000;
        g.ai.brains.insert(0, b);
        let mut other = Brain::new(1);
        other.memory(sector(g.units[1].pos)).danger = 1000;
        g.ai.brains.insert(1, other);
        assert_eq!(g.view(None).pheromones.len(), 2);
        assert!(g
            .view(Some(0))
            .pheromones
            .iter()
            .all(|p| p.hive == 0 && p.trail == 1.));
        g.ai.tick = 4800;
        assert!(g.view(None).pheromones.is_empty());
        let mut outcomes = Vec::new();
        for pieces in [1, 10] {
            let mut g = world(3);
            g.units[0].energy = START_QUEEN_E;
            let mut b = Brain::new(0);
            b.observe(&g, 0, 10);
            for id in 0..pieces {
                b.deposits.insert(
                    id,
                    Deposit {
                        pos: g.units[0].pos.offset(1000., 0., 0.),
                        energy: 2e12 / pieces as f64,
                        minerals: 1000. / pieces as f64,
                        seen: 10,
                    },
                );
            }
            let mut w = Swarm::default();
            w.record_land(g.units[0].pos, true);
            b.queen(&mut g, &mut w, 0, 10);
            outcomes.push((
                g.units[0].brood_remaining,
                g.units[0].energy,
                g.units[0].minerals,
            ));
        }
        assert_eq!(outcomes[0], outcomes[1]);
        assert!(outcomes[0].0 > 0.);
    }
}

fn saved_distance<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::INFINITY))
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedSwarm {
    tick: u64,
    brains: BTreeMap<u32, SavedBrain>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedBrain {
    drives: BTreeMap<usize, Drive>,
    blocked: BTreeMap<u64, u64>,
    tasks: BTreeMap<usize, Task>,
    contacts: BTreeMap<u64, Contact>,
    deposits: BTreeMap<u64, Deposit>,
    memory: BTreeMap<u64, Memory>,
    trace: VecDeque<Trace>,
    deliveries: u64,
    delivered: f64,
    births: u64,
    shots: u64,
    personality: u32,
}
impl Swarm {
    pub(super) fn saved(&self) -> SavedSwarm {
        SavedSwarm {
            tick: self.tick,
            brains: self
                .brains
                .iter()
                .map(|(&h, b)| {
                    (
                        h,
                        SavedBrain {
                            drives: b.drives.clone(),
                            blocked: b.blocked.clone(),
                            tasks: b.tasks.clone(),
                            contacts: b.contacts.clone(),
                            deposits: b.deposits.clone(),
                            memory: b.memory.clone(),
                            trace: b.trace.clone(),
                            deliveries: b.deliveries,
                            delivered: b.delivered,
                            births: b.births,
                            shots: b.shots,
                            personality: b.personality,
                        },
                    )
                })
                .collect(),
        }
    }
    pub(super) fn from_saved(s: SavedSwarm) -> Self {
        let mut swarm = Self {
            tick: s.tick,
            ..Self::default()
        };
        for (h, b) in s.brains {
            let mut brain = Brain::new(h);
            brain.drives = b.drives;
            brain.blocked = b.blocked;
            brain.tasks = b.tasks;
            brain.contacts = b.contacts;
            brain.deposits = b.deposits;
            brain.memory = b.memory;
            brain.trace = b.trace;
            brain.deliveries = b.deliveries;
            brain.delivered = b.delivered;
            brain.births = b.births;
            brain.shots = b.shots;
            brain.personality = b.personality;
            swarm.brains.insert(h, brain);
        }
        swarm
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn remembered_routes_pheromones_and_economy_counters_survive_save() {
        let mut s = Swarm::default();
        s.tick = 1000;
        let mut b = Brain::new(7);
        b.deliveries = 4;
        b.delivered = 123.;
        b.births = 2;
        b.shots = 1;
        b.memory.insert(
            42,
            Memory {
                trail: 400,
                trail_at: 900,
                danger: 200,
                danger_at: 800,
                visited: 600,
            },
        );
        b.tasks
            .insert(0, Task::new(Job::Haul, 9, Pos::default(), 1000, 30));
        b.drives.insert(0, Drive::default());
        s.brains.insert(7, b);
        let bytes = serde_json::to_vec(&s.saved()).unwrap();
        let restored = Swarm::from_saved(serde_json::from_slice(&bytes).unwrap());
        let b = &restored.brains[&7];
        assert_eq!(restored.tick, 1000);
        assert_eq!(b.memory[&42].trail, 400);
        assert_eq!(b.tasks[&0].job, Job::Haul);
        assert_eq!(b.tasks[&0].distance, f64::INFINITY);
        assert_eq!(b.deliveries, 4);
        assert_eq!(b.births, 2);
        assert_eq!(b.shots, 1);
        assert_eq!(b.nodes.capacity(), RETAINED_NODES);
        assert!(restored.manual.is_none());
    }
}
