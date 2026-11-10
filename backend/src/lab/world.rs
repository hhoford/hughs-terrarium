use super::*;
impl Lab {
    pub(super) fn apply_distribution(
        &mut self,
        distribution: crate::distribute::Distribution,
    ) -> Result<(), String> {
        self.units
            .reserve(distribution.hives.iter().map(|h| h.drones + 1).sum());
        self.resources.reserve(distribution.deposits.len());
        for hive in distribution.hives {
            let at = Pos {
                lat: hive.at.lat,
                lon: hive.at.lon,
                alt: 0.,
            };
            if elevation(&self.patches, at).is_none() {
                return Err("Terrain unavailable".into());
            }
            self.add_hive(at, hive.drones, hive.queen_energy, hive.queen_minerals);
        }
        for deposit in distribution.deposits {
            if deposit.energy == 0. && deposit.minerals == 0. {
                continue;
            }
            let at = Pos {
                lat: deposit.at.lat,
                lon: deposit.at.lon,
                alt: 0.,
            };
            let ground = elevation(&self.patches, at).ok_or("Terrain unavailable")?;
            let id = self.id();
            self.resources.push(Resource {
                id,
                pos: Pos { alt: ground, ..at },
                energy: deposit.energy,
                minerals: deposit.minerals,
                discovered: vec![],
                fall_speed: 0.,
            });
        }
        Ok(())
    }

    pub(super) fn remember_patch(&mut self, patch: Terrain) {
        self.patches.push(patch);
        if self.patches.len() > 256 {
            self.patches.remove(0);
        }
        self.terrain_revision += 1;
        self.terrain_index.rebuild(&self.patches);
    }
    pub(super) fn population_used(&self) -> usize {
        self.units.iter().filter(|u| u.alive).count()
            + self
                .units
                .iter()
                .filter(|u| u.alive && u.brood_remaining > 0.)
                .count()
    }
    pub(super) fn id(&mut self) -> u64 {
        let n = self.next;
        self.next += 1;
        n
    }
    pub(super) fn unit_visible(&self, h: u32, u: &Body) -> bool {
        u.hive == h
            || u.pos.alt > 20000.
            || self
                .envelope
                .height(u.pos.lat, u.pos.lon, &mut u.envelope_cell.clone())
                .is_some_and(|v| u.pos.alt >= v + 1000.)
            || self.units.iter().any(|eye| {
                eye.alive
                    && eye.hive == h
                    && eye.pos.distance(u.pos)
                        < if eye.scan_until > self.time {
                            8000.
                        } else {
                            5000.
                        }
                    && !segment_ground(
                        &self.patches,
                        eye.pos.ecef(),
                        u.pos.offset(0., 0., 30.).ecef(),
                    )
            })
    }
    pub(super) fn view(&self, hive: Option<u32>) -> View {
        View {
            all_hives_active: self.units.iter().any(|u| u.alive)
                && self
                    .units
                    .iter()
                    .filter(|u| u.alive)
                    .all(|u| self.ai.brains.contains_key(&u.hive)),
            tags: self
                .tags
                .iter()
                .filter(|t| hive.is_none_or(|h| t.hive == Some(h)))
                .cloned()
                .collect(),
            pheromones: self.ai.pheromones(hive),
            ai_hives: self
                .ai
                .brains
                .keys()
                .copied()
                .filter(|h| hive.is_none_or(|x| x == *h))
                .collect(),
            balance: Balance {
                blast_radius: BLAST_R,
                precision_radius: PRECISION_R,
                radar_offset: 1000.,
                close_mark_altitude: CLOSE_MARK_ALTITUDE,
                strike_energy: STRIKE_E,
                birth_energy: BIRTH_E,
                birth_minerals: BIRTH_M,
            },
            marks: self
                .marks
                .iter()
                .filter(|m| m.expires > self.time && hive.map_or(true, |h| m.hive == h))
                .cloned()
                .collect(),
            terrain_envelope: envelope::info(),
            epoch: self.epoch,
            time: self.time,
            units: self
                .units
                .iter()
                .filter(|u| hive.is_none_or(|h| self.unit_visible(h, u)))
                .cloned()
                .map(|mut u| {
                    refresh_envelope(&self.envelope, &mut u);
                    u.sample_time = self.time;
                    u
                })
                .collect(),
            bullets: self
                .bullets
                .iter()
                .filter(|b| {
                    hive.map_or(true, |h| {
                        h == b.hive
                            || self
                                .envelope
                                .height(b.pos.lat, b.pos.lon, &mut None)
                                .is_some_and(|h| b.pos.alt >= h + 1000.)
                    })
                })
                .cloned()
                .collect(),
            missiles: self.missiles.clone(),
            bursts: self.bursts.clone(),
            loading: self.loading,
            revision: self.terrain_revision,
            tutorial: self.tutorial.clone(),
            resources: self
                .resources
                .iter()
                .filter(|r| hive.map_or(true, |h| r.discovered.contains(&h)))
                .cloned()
                .collect(),
        }
    }
    pub(super) fn spawn_altitude(&self, at: Pos, ground: f64) -> f64 {
        (ground + 1600.).max(
            self.envelope
                .height(at.lat, at.lon, &mut None)
                .unwrap_or(ground)
                + 1600.,
        )
    }
    pub(super) fn add_hive(&mut self, at: Pos, drones: usize, energy: f64, minerals: f64) {
        let hive = self.units.iter().map(|u| u.hive).max().map_or(0, |x| x + 1);
        let id = self.id();
        let g = elevation(&self.patches, at).unwrap();
        let pos = Pos {
            alt: self.spawn_altitude(at, g),
            ..at
        };
        let mut q = Body::new(format!("q{id}"), hive, true, pos);
        q.energy = energy;
        q.minerals = minerals;
        q.agl = pos.alt - g;
        refresh_envelope(&self.envelope, &mut q);
        self.ai.record_land(q.pos, true);
        self.journal.emit(self.epoch,self.time,"hive_spawned",Some(hive),Some(&q.id),serde_json::json!({"pos":q.pos,"energy_j":q.energy,"minerals_kg":q.minerals,"workers":drones}));
        self.units.push(q);
        for i in 0..drones {
            let p = member_position(pos, i);
            self.add_drone(hive, p);
        }
    }
    pub(super) fn add_drone(&mut self, hive: u32, at: Pos) {
        let id = self.id();
        let ground = elevation(&self.patches, at).unwrap();
        let p = Pos {
            alt: at.alt.max(self.spawn_altitude(at, ground)),
            ..at
        };
        let mut u = Body::new(format!("d{id}"), hive, false, p);
        if let Some(parent) = self.units.iter().find(|q| q.hive == hive && q.queen()) {
            u.lineage.clone_from(&parent.lineage);
        }
        u.agl = p.alt - ground;
        refresh_envelope(&self.envelope, &mut u);
        self.journal.emit(
            self.epoch,
            self.time,
            "worker_spawned",
            Some(hive),
            Some(&u.id),
            serde_json::json!({"pos":u.pos}),
        );
        self.units.push(u);
    }
    pub(super) fn tick(&mut self, dt: f64) {
        if self.loading || self.tutorial.is_some() {
            return;
        }
        self.terrain_index.ensure(&self.patches);
        self.ai_step();
        self.time += dt;
        self.build(dt);
        self.marks.retain(|m| m.expires > self.time);
        let mut scratch = self.scratch.take().expect("tick scratch returned");
        for r in &mut self.resources {
            let Some(ground) = self.terrain_index.elevation(&self.patches, r.pos) else {
                continue;
            };
            if r.pos.alt > ground {
                let (position, velocity) = crate::physics::advance(
                    Vec3::new(0., r.pos.alt, 0.),
                    Vec3::new(0., -r.fall_speed, 0.),
                    Vec3::new(0., -gravity(r.pos), 0.),
                    dt,
                );
                r.fall_speed = -velocity.y;
                r.pos.alt = position.y.max(ground);
            } else {
                r.fall_speed = 0.;
            }
        }

        for u in &mut self.units {
            if !u.alive {
                continue;
            }
            if u.mode != "hold" && self.time - u.heartbeat > 2. {
                u.hold_here();
            }
            if u.brood_remaining > 0. {
                if !u.grounded {
                    u.brood_remaining = 0.;
                } else {
                    u.brood_remaining = (u.brood_remaining - dt).max(0.);
                    if u.brood_remaining == 0. {
                        scratch.hatches.push((u.hive, u.pos));
                    }
                }
            }
            u.cooldown = (u.cooldown - dt).max(0.);
            u.reload = (u.reload - dt).max(0.);
            let Some(g) = (if u.pos.alt > 20000. {
                Some(0.)
            } else {
                self.terrain_index.body_ground(&self.patches, u)
            }) else {
                continue;
            };
            if u.pos.alt <= 20000. && !self.envelope.ready(u.pos.lat, u.pos.lon) {
                continue;
            }
            u.agl = u.pos.alt - g;
            if u.mode == "preparing" {
                u.timer -= dt;
                u.power = 10. * u.dry_mass;
                u.energy = (u.energy - u.power * dt).max(0.);
                if u.timer <= 0. {
                    u.mode = "rising".into();
                    u.hold.alt = g + TAKEOFF_CLEARANCE;
                    // The paid preparation pre-spins the wings against
                    // ground support. Start at weight balance, then build
                    // excess lift; otherwise spool lag immediately re-lands her.
                    u.thrust = Vec3::new(0., 0., gravity(u.pos));
                }
                continue;
            }
            if u.grounded && u.mode != "landing" && u.hold.alt > g + 2. && u.energy > 0. {
                u.grounded = false;
            }
            if !u.grounded {
                let previous = u.pos;
                let previous_motion = (u.v, u.thrust, u.energy, u.power);
                let data_ready = self.terrain_index.body_ground(&self.patches, u).is_some()
                    && self.envelope.ready(u.pos.lat, u.pos.lon);
                if u.pos.alt > 20000. && !data_ready {
                    let brake = if u.queen() { 4. } else { 20. };
                    let stop = u.v.z.min(0.).powi(2) / (2. * brake);
                    if u.pos.alt < 40000. + stop {
                        u.input.thrust_down = false;
                        u.input.thrust_up = true;
                        u.input.alt_hold = true;
                    }
                }
                fly(u, dt, g);
                // Only this body waits at a missing-data boundary. Never invent
                // ground heights, and never stop the other hives' clocks.
                if u.pos.alt <= 20000.
                    && (self.terrain_index.body_ground(&self.patches, u).is_none()
                        || !self.envelope.ready(u.pos.lat, u.pos.lon))
                {
                    u.pos = previous;
                    (u.v, u.thrust, u.energy, u.power) = previous_motion;
                    continue;
                }
                if let Some(hit) =
                    self.terrain_index
                        .hit(&self.patches, previous.ecef(), u.pos.ecef())
                {
                    u.pos = hit;
                }
                if u.mode == "rising" && u.pos.alt - g >= TAKEOFF_CLEARANCE - 2. {
                    u.mode = "manual".into();
                    u.heartbeat = self.time;
                    u.input = Input {
                        alt_hold: true,
                        ..Input::default()
                    };
                }
                if let Some(ng) = self.terrain_index.body_ground(&self.patches, u) {
                    u.agl = u.pos.alt - ng;
                    if u.agl <= 0. {
                        if u.queen() || (u.founder && u.mode == "landing") {
                            u.pos.alt = ng;
                            u.grounded = true;
                            u.v = Vec3::default();
                            u.pitch = 0.;
                            u.roll = 0.;
                            u.thrust = Vec3::default();
                            u.mode = "hold".into();
                            u.hold = u.pos;
                        } else {
                            self.journal.emit(
                                self.epoch,
                                self.time,
                                "terrain_impact",
                                Some(u.hive),
                                Some(&u.id),
                                serde_json::json!({"pos":u.pos}),
                            );
                            u.alive = false;
                            u.energy = 0.;
                        }
                    }
                }
            } else {
                u.power = 0.;
            }
            if u.mode == "manual"
                && u.input.fire
                && !u.queen()
                && u.reload == 0.
                && u.energy >= 2000.
            {
                u.energy -= 2000.;
                u.reload = 0.4;
                let b = u.pos.basis();
                let dir = b[0]
                    .mul(u.heading.sin() * u.pitch.cos())
                    .add(b[1].mul(u.heading.cos() * u.pitch.cos()))
                    .add(b[2].mul(u.pitch.sin()));
                let v = b[0]
                    .mul(u.v.x)
                    .add(b[1].mul(u.v.y))
                    .add(b[2].mul(u.v.z))
                    .add(dir.mul(300.));
                scratch.fire.push((u.id.clone(), u.hive, u.pos, v));
                u.input.fire = false;
            }
            refresh_envelope(&self.envelope, u);
            if let Some(c) = &mut u.charge {
                if !u.radar_visible || u.grounded || u.energy <= 0. {
                    u.energy = (u.energy + STRIKE_E * 0.75).min(u.capacity);
                    u.charge = None;
                } else {
                    c.remaining -= dt;
                    if c.remaining <= 0. {
                        scratch.launch.push((u.hive, u.pos, c.target, c.precision));
                        u.charge = None;
                        u.cooldown = COOLDOWN;
                    }
                }
            }
        }
        for (h, p) in scratch.hatches.drain(..) {
            let p = p.offset(100., 0., 0.);
            let g = self
                .terrain_index
                .elevation(&self.patches, p)
                .unwrap_or(p.alt);
            let p = Pos { alt: g + 20., ..p };
            let id = self.id();
            let mut child = Body::new(format!("d{id}"), h, false, p);
            if let Some(parent) = self.units.iter().find(|q| q.hive == h && q.queen()) {
                child.lineage.clone_from(&parent.lineage);
            }
            child.agl = 20.;
            refresh_envelope(&self.envelope, &mut child);
            self.journal.emit(
                self.epoch,
                self.time,
                "worker_born",
                Some(h),
                Some(&child.id),
                serde_json::json!({"pos":child.pos,"energy_j":child.energy}),
            );
            self.units.push(child);
        }
        for (source, hive, pos, velocity) in scratch.fire.drain(..) {
            let id = self.id();
            self.journal.emit(
                self.epoch,
                self.time,
                "spit_fired",
                Some(hive),
                Some(&source),
                serde_json::json!({"projectile":id,"pos":pos}),
            );
            self.bullets.push(Bullet {
                id,
                source,
                hive,
                pos,
                velocity,
                age: 0.,
                p: pos.ecef(),
            });
        }
        for (hive, start, target, precision) in scratch.launch.drain(..) {
            if let Some((velocity, duration, path)) = plan(start, target) {
                let id = self.id();
                self.journal.emit(self.epoch,self.time,"missile_launched",Some(hive),None,serde_json::json!({"missile":id,"start":start,"target":target,"duration_s":duration,"precision":precision}));
                self.missiles.push(Missile {
                    precision,
                    id,
                    hive,
                    pos: start,
                    start,
                    target,
                    age: 0.,
                    duration,
                    path,
                    p: start.ecef(),
                    velocity,
                });
            }
        }
        self.weapons_reusing(dt, &mut scratch);
        scratch.active.extend(
            self.bursts
                .iter()
                .filter(|b| b.kind == "nuclear" && b.age < 14.)
                .map(|b| (b.pos, b.precision)),
        );
        for (p, precision) in scratch.active.drain(..) {
            self.blast_damage(p, precision);
        }
        for b in &mut self.bursts {
            b.age += dt;
        }
        self.bursts.retain(|b| b.age < 14.);
        self.settle_deaths();
        self.succession();
        self.scratch = Some(scratch);
    }
}
