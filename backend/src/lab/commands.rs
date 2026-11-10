use super::*;
impl Lab {
    pub(super) fn command(&mut self, c: &Cmd) -> Result<(), String> {
        let inventory = |g: &Lab| {
            g.units.iter().find(|u|Some(&u.id)==c.unit.as_ref()).map(|u|serde_json::json!({"energy_j":u.energy,"cargo_energy_j":u.cargo_energy,"minerals_kg":u.minerals,"cargo_minerals_kg":u.cargo_minerals}))
        };
        let before = inventory(self);
        let result = self.execute_command(c);
        let hive = c.hive.or_else(|| {
            self.units
                .iter()
                .find(|u| Some(&u.id) == c.unit.as_ref())
                .map(|u| u.hive)
        });
        self.journal.emit(self.epoch,self.time,if result.is_ok(){"command"}else{"command_rejected"},hive,c.unit.as_deref(),serde_json::json!({"action":c.action,"id":c.id,"actor":if c.id.starts_with("ai-"){"ai"}else{"user"},"target":c.target,"amount":c.amount,"mark":c.mark,"before":before,"after":inventory(self),"error":result.as_ref().err()}));
        result
    }
    fn execute_command(&mut self, c: &Cmd) -> Result<(), String> {
        if c.epoch != self.epoch {
            return Err("World replaced".into());
        }
        if self.tutorial.is_some() && c.action != "tutorial_end" {
            return Err("Tutorial: controls explain only".into());
        }

        if c.id.is_empty() || self.commands.contains(&c.id) {
            return Err("Repeated command".into());
        }
        let target = c.target;
        match c.action.as_str() {
            "ai_disable_all" => {
                self.ai.brains.clear();
                for u in self.units.iter_mut().filter(|u| u.alive) {
                    if self.ai.manual.as_deref() != Some(u.id.as_str()) {
                        u.hold_here();
                    }
                }
            }
            "ai_enable_all" => {
                self.ai
                    .prepare
                    .reserve(64usize.saturating_sub(self.ai.prepare.capacity()));
                self.ai
                    .land_requests
                    .reserve(32usize.saturating_sub(self.ai.land_requests.capacity()));
                for u in self.units.iter().filter(|u| u.alive) {
                    self.ai
                        .brains
                        .entry(u.hive)
                        .or_insert_with(|| ai::Brain::new(u.hive));
                }
            }
            "ai_enable" | "ai_disable" => {
                let hive = c.hive.ok_or("Choose a hive")?;
                if !self.units.iter().any(|u| u.alive && u.hive == hive) {
                    return Err("Hive unavailable".into());
                }
                if c.action == "ai_enable" {
                    self.ai
                        .prepare
                        .reserve(64usize.saturating_sub(self.ai.prepare.capacity()));
                    self.ai
                        .land_requests
                        .reserve(32usize.saturating_sub(self.ai.land_requests.capacity()));
                    self.ai
                        .brains
                        .entry(hive)
                        .or_insert_with(|| ai::Brain::new(hive));
                } else {
                    self.ai.brains.remove(&hive);
                    for u in &mut self.units {
                        if u.hive == hive && self.ai.manual.as_deref() != Some(u.id.as_str()) {
                            u.hold_here();
                        }
                    }
                }
            }
            "tutorial_god" => {
                self.tutorial = Some("god".into());
            }
            "tutorial_end" => {
                self.tutorial = None;
                for u in &mut self.units {
                    u.heartbeat = self.time;
                }
            }
            "tutorial" => {
                let i = self
                    .units
                    .iter()
                    .position(|u| Some(&u.id) == c.unit.as_ref() && u.alive)
                    .ok_or("Unit unavailable")?;
                for u in &mut self.units {
                    if u.mode != "hold" {
                        u.hold_here();
                    }
                }
                let u = &mut self.units[i];
                u.mode = "manual".into();
                u.input = Input {
                    alt_hold: true,
                    ..Input::default()
                };
                u.heartbeat = self.time;
                self.ai.manual = Some(u.id.clone());
                self.tutorial = Some(u.id.clone());
            }
            "drop_energy" | "drop_minerals" => {
                let mut p = target.filter(|p| p.valid()).ok_or("Choose a position")?;
                p.alt = elevation(&self.patches, p).ok_or("Terrain unavailable")?;
                if self.resources.len() >= 1000 {
                    return Err("Resource capacity".into());
                }
                let amount = c.amount.unwrap_or(if c.action == "drop_energy" {
                    DRONE_CARGO_ENERGY
                } else {
                    1000.
                });
                if !amount.is_finite()
                    || amount <= 0.
                    || amount > if c.action == "drop_energy" { 1e16 } else { 1e9 }
                {
                    return Err("Invalid resource amount".into());
                }
                let id = self.id();
                self.resources.push(Resource {
                    id,
                    pos: p,
                    energy: if c.action == "drop_energy" {
                        amount
                    } else {
                        0.
                    },
                    minerals: if c.action == "drop_minerals" {
                        amount
                    } else {
                        0.
                    },
                    discovered: vec![],
                    fall_speed: 0.,
                });
            }
            "add_queen" => {
                let p = target.ok_or("Choose a position")?;
                if self.population_used() >= 100 {
                    return Err("Sandbox capacity".into());
                }
                self.add_hive(p, 0, START_QUEEN_E, START_QUEEN_MINERALS);
            }
            "add_dragonfly" => {
                let p = target.ok_or("Choose a position")?;
                let hive = c.hive.ok_or("Choose a hive")?;
                if !self
                    .units
                    .iter()
                    .any(|u| u.hive == hive && u.queen() && u.alive)
                {
                    return Err("Hive unavailable".into());
                }
                if self.population_used() >= 100 {
                    return Err("Sandbox capacity".into());
                }
                self.add_drone(hive, p);
            }
            _ => {
                let i = self
                    .units
                    .iter()
                    .position(|u| Some(&u.id) == c.unit.as_ref() && u.alive)
                    .ok_or("Unit unavailable")?;
                match c.action.as_str() {
                    "lift_founder" => {
                        let u = &mut self.units[i];
                        if !u.founder
                            || !u.grounded
                            || u.construction_remaining > 0.
                            || u.energy <= 0.
                        {
                            return Err("Founder unavailable".into());
                        }
                        u.grounded = false;
                        u.mode = "rising".into();
                        u.hold.alt = u.pos.alt + 20.;
                        u.thrust = Vec3::new(0., 0., gravity(u.pos));
                        u.heartbeat = self.time;
                    }
                    "found" => {
                        let u = &mut self.units[i];
                        if !u.founder
                            || u.queen()
                            || !u.grounded
                            || u.construction_remaining > 0.
                            || u.cargo_minerals < FOUNDER_MINERALS
                            || u.energy + u.cargo_energy < FOUNDER_E + 4.5e9
                        {
                            return Err("Founding requires land, 1000 kg and 204.5 GJ".into());
                        }
                        u.cargo_minerals -= 900.;
                        u.energy += u.cargo_energy - 4.5e9;
                        u.cargo_energy = 0.;
                        u.construction_remaining = 120.;
                    }
                    "grow" => {
                        let u = &mut self.units[i];
                        let target = c.amount.unwrap_or((u.dry_mass * 2.).min(1e7));
                        if !u.queen()
                            || !u.grounded
                            || !target.is_finite()
                            || target < u.dry_mass
                            || target > 1e7
                        {
                            return Err("Growth requires grounded queen".into());
                        }
                        u.growth_target = target;
                    }
                    "refuel_self" => {
                        let u = &mut self.units[i];
                        if !u.founder {
                            return Err("Queen refuelling required".into());
                        }
                        let e = u.cargo_energy.min((u.capacity - u.energy).max(0.));
                        u.energy += e;
                        u.cargo_energy -= e;
                    }
                    "pilot" => {
                        for u in &mut self.units {
                            if u.mode != "hold"
                                && (!self.ai.brains.contains_key(&u.hive)
                                    || self.ai.manual.as_deref() == Some(u.id.as_str()))
                            {
                                u.hold_here();
                            }
                        }
                        self.ai.manual = Some(self.units[i].id.clone());
                        for b in self.ai.brains.values_mut() {
                            b.tasks.remove(&i);
                        }
                        let time = self.time;
                        let u = &mut self.units[i];
                        u.mode = "manual".into();
                        u.input = Input {
                            alt_hold: true,
                            ..Input::default()
                        };
                        u.heartbeat = time;
                    }
                    "mark" => {
                        let u = &self.units[i];
                        if u.queen() || u.energy < 1000. {
                            return Err("Dragonfly mark unavailable".into());
                        }
                        let (pos, hive, source) = (u.pos, u.hive, u.id.clone());
                        let low = self
                            .envelope
                            .height(u.pos.lat, u.pos.lon, &mut u.envelope_cell.clone())
                            .is_some_and(|h| u.pos.alt <= h + CLOSE_MARK_ALTITUDE);
                        let near_ground = elevation(&self.patches, u.pos)
                            .is_some_and(|ground| u.pos.alt - ground <= 120.);
                        let accuracy_m = if (low || near_ground) && u.v.length() <= 20. {
                            10.
                        } else {
                            150.
                        };
                        self.marks.retain(|m| m.source != source);
                        let id = self.id();
                        self.marks.push(Mark {
                            id,
                            hive,
                            source,
                            pos,
                            expires: self.time + MARK_TTL,
                            accuracy_m,
                        });
                        self.units[i].energy -= 1000.;
                    }
                    "reproduce" => {
                        let alive = self.units.iter().filter(|u| u.alive).count();
                        let pending = self.units.iter().filter(|u| u.brood_remaining > 0.).count();
                        let u = &mut self.units[i];
                        if !u.queen()
                            || !u.grounded
                            || u.brood_remaining > 0.
                            || u.energy < BIRTH_E
                            || u.minerals < BIRTH_M
                            || alive + pending >= 100
                        {
                            return Err("Reproduction unavailable".into());
                        }
                        u.energy -= BIRTH_E;
                        u.minerals -= BIRTH_M;
                        u.brood_remaining = u.brood_time();
                    }
                    "scan" => {
                        let u = &self.units[i];
                        let fraction = ((u.pos.alt - 20000.) / 60000.).clamp(0., 1.);
                        let radius = 8000. + 992000. * fraction;
                        let cost = 50000. + 9950000. * fraction * fraction;
                        if u.queen() || u.energy < cost || u.scan_until > self.time {
                            return Err("Scan unavailable".into());
                        }
                        let (p, h) = (u.pos, u.hive);
                        for r in &mut self.resources {
                            if p.distance(r.pos) <= radius
                                && segment_sphere(
                                    p.ecef(),
                                    r.pos.offset(0., 0., 5.).ecef(),
                                    Vec3::ZERO,
                                    R - 1.,
                                )
                                .is_none()
                                && (fraction > 0.
                                    || !segment_ground(
                                        &self.patches,
                                        p.ecef(),
                                        r.pos.offset(0., 0., 5.).ecef(),
                                    ))
                                && !r.discovered.contains(&h)
                            {
                                r.discovered.push(h);
                            }
                        }
                        self.units[i].energy -= cost;
                        self.units[i].scan_until = self.time + 3.;
                    }
                    "collect" => {
                        if self.units[i].queen() && !self.units[i].grounded {
                            return Err("Land to collect".into());
                        }
                        let u = &mut self.units[i];
                        let r = self
                            .resources
                            .iter_mut()
                            .filter(|r| {
                                r.discovered.contains(&u.hive)
                                    && r.pos.distance(u.pos) < 120.
                                    && ((r.energy > 0.
                                        && (if u.queen() {
                                            u.energy < u.capacity
                                        } else {
                                            u.cargo_energy < DRONE_CARGO_ENERGY
                                        }))
                                        || (r.minerals > 0.
                                            && (if u.queen() {
                                                u.minerals < u.mineral_capacity()
                                            } else {
                                                u.cargo_minerals < DRONE_CARGO_MINERALS
                                            })))
                            })
                            .min_by(|a, b| a.pos.distance(u.pos).total_cmp(&b.pos.distance(u.pos)))
                            .ok_or("No scanned resource within 120 m")?;
                        if u.v.length() > 20. {
                            return Err("Slow down to collect".into());
                        }
                        let e = r.energy.min(
                            (if u.queen() {
                                u.capacity - u.energy
                            } else {
                                DRONE_CARGO_ENERGY - u.cargo_energy
                            })
                            .max(0.),
                        );
                        let m = r.minerals.min(
                            (if u.queen() {
                                u.mineral_capacity() - u.minerals
                            } else {
                                DRONE_CARGO_MINERALS - u.cargo_minerals
                            })
                            .max(0.),
                        );
                        r.energy -= e;
                        r.minerals -= m;
                        if u.queen() {
                            u.energy += e;
                            u.minerals += m;
                        } else {
                            u.cargo_energy += e;
                            u.cargo_minerals += m;
                        }
                        self.resources.retain(|r| r.energy > 0. || r.minerals > 0.);
                    }
                    "deliver" => {
                        if self.units[i].queen() {
                            return Err("Dragonfly only".into());
                        }
                        let h = self.units[i].hive;
                        let qi = self
                            .units
                            .iter()
                            .position(|q| {
                                q.alive
                                    && q.queen()
                                    && q.hive == h
                                    && q.pos.distance(self.units[i].pos) < 300.
                            })
                            .ok_or("Queen must be within 300 m")?;
                        let e = self.units[i]
                            .cargo_energy
                            .min((self.units[qi].capacity - self.units[qi].energy).max(0.));
                        self.units[qi].energy += e;
                        self.units[i].cargo_energy -= e;
                        let minerals = self.units[i].cargo_minerals.min(
                            (self.units[qi].mineral_capacity() - self.units[qi].minerals).max(0.),
                        );
                        self.units[qi].minerals += minerals;
                        self.units[i].cargo_minerals -= minerals;
                        let refill = (self.units[i].capacity - self.units[i].energy)
                            .min(self.units[qi].energy);
                        self.units[qi].energy -= refill;
                        self.units[i].energy += refill;
                    }
                    "hold" => {
                        self.units[i].hold_here();
                        if self.ai.manual.as_deref() == Some(self.units[i].id.as_str()) {
                            self.ai.manual = None;
                        }
                    }
                    "land" => {
                        if !self.units[i].queen() && !self.units[i].founder {
                            return Err("Queen or founder only".into());
                        }
                        self.units[i].hold_here();
                        self.units[i].mode = "landing".into();
                        self.units[i].heartbeat = self.time;
                    }
                    "lift" => {
                        let u = &mut self.units[i];
                        if !u.queen() || !u.grounded || u.energy <= 0. {
                            return Err("Cannot lift".into());
                        }
                        u.brood_remaining = 0.;
                        u.mode = "preparing".into();
                        u.heartbeat = self.time;
                        u.timer = TAKEOFF_PREP;
                        u.hold.alt = u.pos.alt + TAKEOFF_CLEARANCE;
                    }
                    "strike" => {
                        let h = self.units[i].hive;
                        let m = self
                            .marks
                            .iter()
                            .find(|m| Some(m.id) == c.mark && m.hive == h && m.expires > self.time)
                            .cloned()
                            .ok_or("Choose a fresh dragonfly mark")?;
                        let seed = crate::splitmix64(self.epoch ^ m.id ^ self.next);
                        let angle =
                            (seed as u32 as f64 / u32::MAX as f64) * 2. * std::f64::consts::PI;
                        let radius = ((seed >> 32) as f64 / u32::MAX as f64).sqrt() * m.accuracy_m;
                        let mut p = m.pos.offset(radius * angle.cos(), radius * angle.sin(), 0.);
                        if m.accuracy_m > 10. {
                            p.alt = elevation(&self.patches, p).ok_or("Terrain unavailable")?;
                        }
                        let u = &self.units[i];
                        if !u.queen()
                            || u.grounded
                            || !(u.pos.alt > 20000.
                                || self
                                    .envelope
                                    .height(u.pos.lat, u.pos.lon, &mut u.envelope_cell.clone())
                                    .is_some_and(|h| u.pos.alt >= h + 1000.))
                            || u.energy < STRIKE_E
                            || u.cooldown > 0.
                            || u.charge.is_some()
                        {
                            return Err("Strike unavailable".into());
                        }
                        if plan(u.pos, p).is_none() {
                            return Err("No strike solution".into());
                        }
                        let u = &mut self.units[i];
                        u.energy -= STRIKE_E;
                        u.charge = Some(Charging {
                            precision: m.accuracy_m <= 10.,
                            target: p,
                            remaining: CHARGE,
                        });
                    }
                    "cancel" => {
                        let u = &mut self.units[i];
                        if u.charge.take().is_some() {
                            u.energy = (u.energy + STRIKE_E * 0.75).min(u.capacity);
                        }
                    }
                    _ => return Err("Unknown command".into()),
                }
            }
        }
        self.commands.push_back(c.id.clone());
        if self.commands.len() > 10000 {
            self.commands.pop_front();
        }
        Ok(())
    }
}
