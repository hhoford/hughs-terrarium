use super::*;
impl Body {
    pub(super) fn update_capacity(&mut self) {
        self.capacity = FOUNDER_E + (QUEEN_E - FOUNDER_E) * (self.dry_mass - 1000.) / (1e7 - 1000.);
    }
    pub(super) fn brood_time(&self) -> f64 {
        BROOD_TIME * (1e7 / self.dry_mass).cbrt()
    }
}
impl Lab {
    pub(super) fn succession(&mut self) {
        let dead: Vec<u32> = self
            .units
            .iter_mut()
            .filter(|u| u.queen() && !u.alive && !u.succession_done)
            .map(|u| {
                u.succession_done = true;
                u.hive
            })
            .collect();
        for old in dead {
            let knowledge: Vec<u64> = self
                .resources
                .iter()
                .filter(|r| r.discovered.contains(&old))
                .map(|r| r.id)
                .collect();
            let mut next = self.units.iter().map(|u| u.hive).max().unwrap_or(0) + 1;
            for u in &mut self.units {
                if u.alive && !u.queen() && u.hive == old {
                    u.lineage.push(old);
                    u.hive = next;
                    self.journal.emit(
                        self.epoch,
                        self.time,
                        "succession",
                        Some(next),
                        Some(&u.id),
                        serde_json::json!({"parent_hive":old,"lineage":u.lineage,"pos":u.pos}),
                    );
                    if let Some(parent) = self.ai.brains.get(&old) {
                        let child = parent.offspring(next);
                        self.ai.brains.insert(next, child);
                    }
                    next += 1;
                    u.founder = true;
                    for r in &mut self.resources {
                        if knowledge.contains(&r.id) {
                            r.discovered.push(u.hive);
                        }
                    }
                }
            }
        }
    }
    pub(super) fn build(&mut self, dt: f64) {
        for u in &mut self.units {
            if !u.alive {
                continue;
            }
            if u.construction_remaining > 0. {
                if !u.grounded {
                    continue;
                }
                u.construction_remaining = (u.construction_remaining - dt).max(0.);
                if u.construction_remaining == 0. {
                    self.journal.emit(
                        self.epoch,
                        self.time,
                        "queen_founded",
                        Some(u.hive),
                        Some(&u.id),
                        serde_json::json!({"pos":u.pos}),
                    );
                    u.kind = "queen".into();
                    u.dry_mass = 1000.;
                    u.founder = false;
                    u.update_capacity();
                    let excess = (u.energy - u.capacity).max(0.);
                    u.energy -= excess;
                    if excess > 0. {
                        self.resources.push(Resource {
                            id: self.next,
                            pos: u.pos,
                            energy: excess,
                            minerals: 0.,
                            discovered: vec![u.hive],
                            fall_speed: 0.,
                        });
                        self.next += 1;
                    }
                    u.minerals = u.cargo_minerals;
                    u.cargo_minerals = 0.;
                }
            }
            if u.queen() && u.grounded && u.growth_target > u.dry_mass {
                let add = (u.dry_mass * 0.005 * dt)
                    .min(u.growth_target - u.dry_mass)
                    .min(u.minerals)
                    .min(u.energy / 5e6);
                u.minerals -= add;
                u.energy -= add * 5e6;
                u.dry_mass += add;
                u.update_capacity();
            }
        }
    }
}
