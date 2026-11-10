use super::*;
pub(super) fn spit_acceleration(p: Vec3, velocity: Vec3) -> Vec3 {
    let rho = 1.225 * (-Pos::from_ecef(p).alt.max(0.) / 8500.).exp();
    acceleration(p) + velocity * (-0.5 * rho * 0.00001 / 0.02 * velocity.length())
}

impl Lab {
    #[cfg(test)]
    pub(super) fn weapons(&mut self, dt: f64) {
        let mut scratch = self.scratch.take().unwrap();
        self.weapons_reusing(dt, &mut scratch);
        self.scratch = Some(scratch);
    }

    pub(super) fn weapons_reusing(&mut self, dt: f64, scratch: &mut model::TickScratch) {
        for b in &mut self.bullets {
            let old = b.p;
            let a = spit_acceleration(b.p, b.velocity);
            (b.p, b.velocity) = crate::physics::advance(b.p, b.velocity, a, dt);
            b.pos = Pos::from_ecef(b.p);
            b.age += dt;
            let delta = b.p.sub(old);
            let terrain_t = self
                .terrain_index
                .hit(&self.patches, old, b.p)
                .map(|p| p.ecef().sub(old).dot(delta) / delta.dot(delta).max(1e-12));
            let mut nearest = terrain_t.unwrap_or(2.);
            let mut hit = None;
            for (i, u) in self.units.iter().enumerate() {
                if !u.alive || u.id == b.source {
                    continue;
                }
                // Coarse central hull: a queen absorbs spit but cannot be
                // injured by it. Do not make her transparent to kinetic fire.
                let (centre, radius) = u.hull();
                if let Some(t) = segment_sphere(old, b.p, centre.ecef(), radius) {
                    if t < nearest {
                        nearest = t;
                        hit = Some(i);
                    }
                }
            }
            if let Some(i) = hit {
                b.age = 99.;
                let u = &mut self.units[i];
                self.journal.emit(self.epoch,self.time,"spit_hit",Some(u.hive),Some(&u.id),serde_json::json!({"projectile":b.id,"attacker":b.source,"attacker_hive":b.hive,"fatal":!u.queen(),"pos":u.pos}));
                if !u.queen() {
                    u.alive = false;
                    u.mode = "dead".into();
                    u.energy = 0.;
                    scratch.melt.push(u.pos);
                }
            } else if terrain_t.is_some() {
                b.age = 99.;
            }
        }
        self.bullets.retain(|b| b.age < 3.);

        for m in &mut self.missiles {
            let old = m.p;
            let before = (m.p, m.velocity, m.age, m.pos);
            step_strike(m, dt.min((m.duration - m.age).max(0.)));
            if m.pos.alt.min(before.3.alt) < 15000.
                && [old, old.add(m.p).mul(0.5), m.p]
                    .iter()
                    .any(|p| elevation(&self.patches, Pos::from_ecef(*p)).is_none())
            {
                (m.p, m.velocity, m.age, m.pos) = before;
                continue;
            }
            if let Some(hit) = self.terrain_index.hit(&self.patches, old, m.p) {
                m.pos = hit;
                scratch.blasts.push((hit, m.precision));
                m.age = m.duration + 10.;
            } else if m.age >= m.duration {
                // Time fuse detonates at the integrated position, including airbursts.
                scratch.blasts.push((m.pos, m.precision));
                m.age = m.duration + 10.;
            }
        }
        self.missiles.retain(|m| m.age <= m.duration + 5.);
        for p in scratch.melt.drain(..) {
            let id = self.id();
            self.bursts.push(Burst {
                id,
                pos: p,
                age: 0.,
                radius: 25.,
                kind: "melt".into(),
                precision: false,
            });
        }
        for (p, precision) in scratch.blasts.drain(..) {
            self.detonate_precision(p, precision);
        }

        self.settle_deaths();
    }
    pub(super) fn blast_damage(&mut self, p: Pos, precision: bool) {
        for u in &mut self.units {
            let (centre, radius) = u.hull();
            let distance = centre.distance(p);
            if !u.alive || distance > BLAST_R + radius {
                continue;
            }
            refresh_envelope(&self.envelope, u);
            if !u.nuclear_sheltered || (precision && distance <= PRECISION_R + radius) {
                self.journal.emit(self.epoch,self.time,"blast_hit",Some(u.hive),Some(&u.id),serde_json::json!({"blast":p,"pos":u.pos,"precision":precision,"distance_m":distance,"sheltered":u.nuclear_sheltered}));
                u.alive = false;
            }
        }
    }
    pub(super) fn settle_deaths(&mut self) {
        for u in &mut self.units {
            if u.alive || u.salvaged {
                continue;
            }
            self.journal.emit(self.epoch,self.time,"creature_died",Some(u.hive),Some(&u.id),serde_json::json!({"kind":u.kind,"pos":u.pos,"salvage_resource":self.next,"energy_j":u.cargo_energy+if u.queen(){u.energy}else{0.},"minerals_kg":u.cargo_minerals+if u.queen(){u.minerals}else{DRONE_MASS}}));
            self.resources.push(Resource {
                id: self.next,
                pos: u.pos,
                energy: u.cargo_energy + if u.queen() { u.energy } else { 0. },
                minerals: u.cargo_minerals + if u.queen() { u.minerals } else { DRONE_MASS },
                discovered: vec![u.hive],
                fall_speed: 0.,
            });
            self.next += 1;
            u.salvaged = true;
            u.energy = 0.;
            u.minerals = 0.;
            u.cargo_energy = 0.;
            u.cargo_minerals = 0.;
            u.charge = None;
            u.power = 0.;
            u.v = Vec3::ZERO;
            u.mode = "dead".into();
        }
    }
    #[cfg(test)]
    pub(super) fn detonate(&mut self, p: Pos) {
        self.detonate_precision(p, false);
    }
    pub(super) fn detonate_precision(&mut self, p: Pos, precision: bool) {
        self.journal.emit(
            self.epoch,
            self.time,
            "nuclear_detonation",
            None,
            None,
            serde_json::json!({"pos":p,"precision":precision,"radius_m":BLAST_R}),
        );
        self.blast_damage(p, precision);
        self.settle_deaths();
        let id = self.id();
        self.bursts.push(Burst {
            id,
            pos: p,
            age: 0.,
            radius: BLAST_R,
            kind: "nuclear".into(),
            precision,
        });
    }
}
pub(super) fn segment_ground(patches: &[Terrain], a: Vec3, b: Vec3) -> bool {
    terrain_hit(patches, a, b).is_some()
}
// Powered strike follows a fixed great-circle loft, never a moving target.
// Gravity plus guidance thrust are integrated; poses are never snapped to the arc.
pub(super) fn strike_state(
    start: Pos,
    target: Pos,
    time: f64,
    duration: f64,
) -> (Vec3, Vec3, Vec3) {
    let a = start.ecef().normalize_or_zero();
    let b = target.ecef().normalize_or_zero();
    let cosine = a.dot(b).clamp(-1., 1.);
    let angle = cosine.acos();
    let projection = b.sub(a.mul(cosine));
    let tangent = if projection.length() > 1e-7 {
        projection.normalize_or_zero()
    } else {
        start.basis()[1]
    };
    let u = (time / duration).clamp(0., 1.);
    let f = u * u * (3. - 2. * u);
    let omega = angle * 6. * u * (1. - u) / duration;
    let alpha = angle * (6. - 12. * u) / (duration * duration);
    let direction = a.mul((angle * f).cos()).add(tangent.mul((angle * f).sin()));
    let along = a
        .mul(-(angle * f).sin())
        .add(tangent.mul((angle * f).cos()));
    let loft = 4000. + 196000. * angle / std::f64::consts::PI;
    let k = std::f64::consts::PI / duration;
    let radius =
        R + start.alt + (target.alt - start.alt) * u + loft * (std::f64::consts::PI * u).sin();
    let radial_v =
        (target.alt - start.alt) / duration + loft * k * (std::f64::consts::PI * u).cos();
    let radial_a = -loft * k * k * (std::f64::consts::PI * u).sin();
    (
        direction.mul(radius),
        direction.mul(radial_v).add(along.mul(radius * omega)),
        direction
            .mul(radial_a - radius * omega * omega)
            .add(along.mul(2. * radial_v * omega + radius * alpha)),
    )
}
pub(super) fn strike_acceleration(
    start: Pos,
    target: Pos,
    time: f64,
    duration: f64,
    p: Vec3,
    v: Vec3,
) -> Vec3 {
    let (goal, velocity, a) = strike_state(start, target, time, duration);
    let gravity = acceleration(p);
    let thrust = a
        .sub(gravity)
        .add(goal.sub(p).mul(0.5))
        .add(velocity.sub(v));
    gravity.add(thrust)
}
pub(super) fn step_strike(m: &mut Missile, dt: f64) {
    // Re-evaluate the guidance force at 160 Hz; Rapier integrates each substep.
    for _ in 0..4 {
        let step = dt / 4.;
        let a = strike_acceleration(m.start, m.target, m.age, m.duration, m.p, m.velocity);
        (m.p, m.velocity) = crate::physics::advance(m.p, m.velocity, a, step);
        m.age += step;
    }
    m.pos = Pos::from_ecef(m.p);
}

pub(super) fn plan(start: Pos, target: Pos) -> Option<(Vec3, f64, Vec<Pos>)> {
    if !start.valid() || !target.valid() {
        return None;
    }
    let angle = start
        .ecef()
        .normalize_or_zero()
        .dot(target.ecef().normalize_or_zero())
        .clamp(-1., 1.)
        .acos();
    let duration = (60. + 240. * angle / std::f64::consts::PI).min(300.);
    let (_, velocity, _) = strike_state(start, target, 0., duration);
    let path = (0..=64)
        .map(|i| Pos::from_ecef(strike_state(start, target, duration * i as f64 / 64., duration).0))
        .collect();
    Some((velocity, duration, path))
}
