use super::*;
pub(super) fn member_position(pos: Pos, i: usize) -> Pos {
    pos.offset(
        (i % 7) as f64 * 240. - 240.,
        -400. - (i / 7) as f64 * 240.,
        0.,
    )
}
pub(super) fn refresh_envelope(e: &Envelope, u: &mut Body) {
    if u.pos.alt > 20000. {
        u.terrain_mean = None;
        u.radar_altitude = None;
        u.shelter_altitude = None;
        u.radar_visible = true;
        u.nuclear_sheltered = false;
        return;
    }
    let h = e.height(u.pos.lat, u.pos.lon, &mut u.envelope_cell);
    u.terrain_mean = h;
    u.radar_altitude = h.map(|v| v + 1000.);
    u.shelter_altitude = h.map(|v| v - 500.);
    u.radar_visible = match crate::radar_field::bounds(u.pos.lat, u.pos.lon) {
        Some((lo, _hi)) if u.pos.alt < lo + 1000. => false,
        Some((_lo, hi)) if u.pos.alt >= hi + 1000. => true,
        _ => u.radar_altitude.is_some_and(|v| u.pos.alt >= v),
    };
    u.nuclear_sheltered = u.shelter_altitude.is_some_and(|v| u.pos.alt < v);
}
pub(super) fn gravity(p: Pos) -> f64 {
    MU / (R + p.alt).powi(2)
}
pub(super) fn acceleration(p: Vec3) -> Vec3 {
    p.mul(-MU / p.length().powi(3))
}
pub(super) fn drag_acceleration(queen: bool, mass: f64, alt: f64, v: Vec3) -> Vec3 {
    let rho = (-alt.max(0.) / 8500.).exp();
    v * if queen {
        (0.002 + 0.0002 * v.length()) * rho * QUEEN_DRAG_SCALE
    } else {
        (0.08 + 0.003 * v.length()) * rho * DRONE_MASS / mass
    }
}
pub(super) fn actuator_power(mass: f64, thrust: Vec3, v: Vec3) -> f64 {
    mass + 4. * mass * thrust.length() + (mass * thrust.dot(v)).max(0.) / 0.5
}
pub(super) fn fly(u: &mut Body, dt: f64, ground: f64) {
    let mass = u.mass();
    let queen = u.queen();
    let load = if queen { 1. } else { DRONE_MASS / mass };
    let g = gravity(u.pos);
    let drag = drag_acceleration(queen, mass, u.pos.alt, u.v);
    let axis = |a: bool, b: bool| a as i32 as f64 - b as i32 as f64;
    let mut want = if u.mode == "manual" {
        u.heading +=
            axis(u.input.yaw_right, u.input.yaw_left) * if queen { 0.08 } else { 1.9 } * dt;
        u.pitch = (u.pitch
            + axis(u.input.back, u.input.forward) * if queen { 0.15 } else { 3.2 } * dt)
            .clamp(
                if queen { -0.35 } else { -80_f64.to_radians() },
                if queen { 0.35 } else { 80_f64.to_radians() },
            );
        u.roll = (u.roll
            + axis(u.input.strafe_right, u.input.strafe_left)
                * if queen { 0.15 } else { 3.2 }
                * dt)
            .clamp(
                if queen { -0.35 } else { -80_f64.to_radians() },
                if queen { 0.35 } else { 80_f64.to_radians() },
            );
        let (sh, ch) = (u.heading.sin(), u.heading.cos());
        let (sp, cp) = (u.pitch.sin(), u.pitch.cos());
        let (sr, cr) = (u.roll.sin(), u.roll.cos());
        let mag = if u.input.alt_hold { g } else { 0. }
            + axis(u.input.thrust_up, u.input.thrust_down)
                * if queen {
                    4.
                } else if u.input.boost {
                    if u.input.alt_hold {
                        7. * g * load
                    } else {
                        g + 6. * g * load
                    }
                } else if u.input.alt_hold {
                    4. * g * load
                } else {
                    g + 3. * g * load
                };
        let mut w = Vec3::new(-sh * sp * cr + ch * sr, -ch * sp * cr - sh * sr, cp * cr).mul(mag);
        if u.input.alt_hold {
            w.z += (g - w.z - 2.4 * u.v.z).clamp(-g * 4., g * 4.);
            if u.input.thrust_up || u.input.thrust_down {
                w.z += axis(u.input.thrust_up, u.input.thrust_down)
                    * if queen { 4. } else { 4. * g * load };
            }
        }
        w
    } else {
        let delta = u.hold.ecef().sub(u.pos.ecef());
        let basis = u.pos.basis();
        let d = Vec3::new(
            delta.dot(basis[0]),
            delta.dot(basis[1]),
            if u.mode == "landing" {
                ground - u.pos.alt
            } else {
                u.hold.alt - u.pos.alt
            },
        );
        let desired = Vec3::new(
            (d.x * 0.6).clamp(-55., 55.),
            (d.y * 0.6).clamp(-55., 55.),
            (d.z * 0.8).clamp(-25., 25.),
        );
        desired
            .sub(u.v)
            .mul(1.5)
            .add(drag)
            .add(Vec3::new(0., 0., g))
    };
    if queen {
        let lateral = want.x.hypot(want.y);
        // Vertical manoeuvres are deliberately slow. Horizontal momentum builds
        // under finite thrust/drag; no teleporting velocity or arbitrary speed cap.
        let vertical_goal = if u.mode == "manual" {
            axis(u.input.thrust_up, u.input.thrust_down) * 100.
        } else if u.mode == "landing" {
            -100.
        } else {
            ((u.hold.alt - u.pos.alt) * 0.2).clamp(-100., 100.)
        };
        if u.mode != "manual" || u.input.alt_hold || u.input.thrust_up || u.input.thrust_down {
            want.z = g + drag.z + ((vertical_goal - u.v.z) * 0.5).clamp(-4., 4.);
        }
        if lateral > QUEEN_LATERAL {
            want.x *= QUEEN_LATERAL / lateral;
            want.y *= QUEEN_LATERAL / lateral;
        }
        if want.length() > 35. {
            want = want.normalize_or_zero().mul(35.);
        }
    } else if u.input.boost {
        let horizontal = Vec3::new(want.x, want.y, 0.);
        let horizontal = if horizontal.length() > 0.01 {
            horizontal.normalize() * 20.
        } else {
            horizontal
        };
        want.x = horizontal.x + drag.x;
        want.y = horizontal.y + drag.y;
        if u.input.alt_hold {
            let target = axis(u.input.thrust_up, u.input.thrust_down) * 500.;
            want.z = g + drag.z - (u.v.x * u.v.x + u.v.y * u.v.y) / (R + u.pos.alt)
                + ((target - u.v.z) * 2.).clamp(-20., 20.);
        }
        if want.length() > 100000. / mass {
            want = want.normalize() * (100000. / mass);
        }
    } else if want.length() > (8. * g).min(DRONE_FORCE / mass) {
        // A 32 kN hardware budget supports 2 t cargo; the controller limits
        // unloaded handling to 8 g. The 2 MW power/reserve limits still apply.
        want = want
            .normalize_or_zero()
            .mul((8. * g).min(DRONE_FORCE / mass));
    }
    if queen && want.length() > 40. * u.dry_mass / mass {
        want = want.normalize() * (40. * u.dry_mass / mass);
    }
    let spool = 1. - (-dt / if queen { 0.5 } else { 0.07 }).exp();
    u.thrust = u.thrust.add(want.sub(u.thrust).mul(spool));
    let p = actuator_power(mass, u.thrust, u.v);
    let fraction = (u.energy / (p * dt)).clamp(0., 1.).min(if queen {
        (10000. * u.dry_mass / p).min(1.)
    } else {
        ((if u.input.boost { 500e6 } else { DRONE_POWER }) / p).min(1.)
    });
    u.thrust = u.thrust.mul(fraction);
    u.power = p * fraction;
    u.energy = (u.energy - u.power * dt).max(0.);
    let a = u.thrust.sub(drag).sub(Vec3::new(0., 0., g));
    let basis = u.pos.basis();
    let world = |v: Vec3| basis[0] * v.x + basis[1] * v.y + basis[2] * v.z;
    let (position, velocity) = crate::physics::advance(u.pos.ecef(), world(u.v), world(a), dt);
    u.pos = Pos::from_ecef(position);
    let basis = u.pos.basis();
    u.v = Vec3::new(
        velocity.dot(basis[0]),
        velocity.dot(basis[1]),
        velocity.dot(basis[2]),
    );
    // Swept terrain prevents a fast craft crossing an entire ridge between ticks.
    // tick resolves swept contact at <=20 m before publishing the pose.
}
