use super::*;

#[test]
fn starting_config_defaults_legacy_seed_and_bounds_are_consistent() {
    let defaults = serde_json::from_str::<Reset>("{}")
        .unwrap()
        .config()
        .unwrap();
    assert_eq!(
        (defaults.hives, defaults.drones_per_hive, defaults.deposits),
        (0, 3, 0)
    );
    assert_eq!(
        (defaults.queen_energy, defaults.queen_minerals),
        (START_QUEEN_E, START_QUEEN_MINERALS)
    );
    let config = serde_json::from_str::<Reset>(
        r#"{"seed":17,"start":{"seed":9,"hives":2,"drones_per_hive":1,"deposits":3}}"#,
    )
    .unwrap()
    .config()
    .unwrap();
    assert_eq!((config.seed, config.hives, config.deposits), (17, 2, 3));
    for raw in [
        r#"{"start":{"hives":101}}"#,
        r#"{"start":{"queen_energy":1e14}}"#,
        r#"{"start":{"deposits":1001}}"#,
    ] {
        assert!(serde_json::from_str::<Reset>(raw)
            .unwrap()
            .config()
            .is_err());
    }
    let empty = serde_json::from_str::<Reset>(r#"{"start":{"hives":0,"deposits":0}}"#)
        .unwrap()
        .config()
        .unwrap();
    assert!(crate::distribute::plan(&empty).hives.is_empty());
}
#[test]
fn distribution_applies_exact_stores_and_mixed_deposits_to_world_state() {
    let config = crate::distribute::StartConfig {
        hives: 2,
        drones_per_hive: 1,
        deposits: 3,
        queen_energy: 123e9,
        queen_minerals: 4567.,
        energy_per_deposit: 890e9,
        minerals_per_deposit: 5000.,
        ..Default::default()
    };
    let distribution = crate::distribute::plan(&config);
    let mut g = new();
    g.envelope = Envelope::for_test(|_, _| 0.);
    for at in distribution
        .hives
        .iter()
        .map(|h| h.at)
        .chain(distribution.deposits.iter().map(|d| d.at))
    {
        g.patches.push(flat(at.lat, at.lon, 0.));
    }
    g.apply_distribution(distribution).unwrap();
    assert_eq!((g.units.len(), g.resources.len()), (4, 3));
    for q in g.units.iter().filter(|u| u.queen()) {
        assert_eq!((q.energy, q.minerals), (123e9, 4567.));
    }
    for r in &g.resources {
        assert_eq!((r.energy, r.minerals, r.pos.alt), (890e9, 5000., 0.));
        assert!(r.discovered.is_empty());
    }
    assert_eq!(
        serde_json::to_value(g.view(None)).unwrap()["resources"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn full_two_tonne_payload_can_hover_and_climb_with_finite_actuators() {
    let mut u = fixture().units[1].clone();
    u.cargo_minerals = DRONE_CARGO_MINERALS;
    let start = u.pos.alt;
    for _ in 0..80 {
        fly(&mut u, DT, 0.);
        assert!(u.power <= DRONE_POWER);
        assert!(u.thrust.length() * u.mass() <= DRONE_FORCE + 0.1);
    }
    assert!((u.pos.alt - start).abs() < 0.01);
    u.mode = "manual".into();
    u.input = Input {
        thrust_up: true,
        alt_hold: true,
        ..Input::default()
    };
    for _ in 0..160 {
        fly(&mut u, DT, 0.);
    }
    assert!(u.v.z > 0.5 && u.energy > 0.);
    // W must lift full cargo with assistance disabled as well. Releasing W
    // still removes all commanded lift; the hardware/power limits are shared.
    let mut manual = fixture().units[1].clone();
    manual.cargo_minerals = DRONE_CARGO_MINERALS;
    manual.mode = "manual".into();
    manual.input.thrust_up = true;
    for _ in 0..160 {
        fly(&mut manual, DT, 0.);
        assert!(manual.thrust.length() * manual.mass() <= DRONE_FORCE + 0.1);
        assert!(manual.power <= DRONE_POWER);
    }
    assert!(manual.v.z > 0.5);
}
#[test]
fn every_dragonfly_death_salvages_body_and_cargo_once_without_fuel() {
    for cause in ["spit", "crash", "nuke"] {
        let mut g = fixture();
        let drone = g.units.remove(1);
        g.units = vec![drone];
        g.units[0].cargo_energy = 123e9;
        g.units[0].cargo_minerals = 2000.;
        let p = g.units[0].pos;
        match cause {
            "spit" => {
                let a = p.offset(-20., 0., 0.);
                g.bullets.push(Bullet {
                    id: 999,
                    source: "attacker".into(),
                    hive: 0,
                    pos: a,
                    velocity: p.basis()[0] * 1000.,
                    age: 0.,
                    p: a.ecef(),
                });
                g.weapons(DT);
            }
            "crash" => {
                g.units[0].pos.alt = 0.01;
                g.units[0].v.z = -10.;
                g.tick(DT);
            }
            _ => g.detonate(p),
        }
        assert!(!g.units[0].alive, "{cause}");
        assert_eq!(g.resources.len(), 1, "{cause}");
        assert_eq!(
            (g.resources[0].energy, g.resources[0].minerals),
            (123e9, DRONE_MASS + 2000.)
        );
        assert_eq!(
            (
                g.units[0].energy,
                g.units[0].cargo_energy,
                g.units[0].cargo_minerals
            ),
            (0., 0., 0.)
        );
        g.settle_deaths();
        assert_eq!(g.resources.len(), 1);
    }
}
#[test]
fn missing_terrain_stops_only_the_affected_body_and_resumes_without_a_latch() {
    let mut g = fixture();
    g.patches.remove(1);
    let stopped = g.units[4].clone();
    let other_energy = g.units[8].energy;
    run(&mut g, 1.);
    assert!((g.time - 1.).abs() < 1e-6 && !g.loading);
    assert!(g.units[4].pos.distance(stopped.pos) < 1e-6);
    assert_eq!(g.units[4].energy, stopped.energy);
    assert!(g.units[8].energy < other_energy);
    g.remember_patch(flat(30., 120., 1000.));
    run(&mut g, 1.);
    assert!(g.units[4].energy < stopped.energy && !g.loading);

    // A fast body must not advance beyond the last validated patch.
    let u = &mut g.units[1];
    u.pos = Pos {
        lat: 0.,
        lon: 0.,
        alt: 1600.,
    }
    .offset(7890., 0., 0.);
    u.mode = "manual".into();
    u.heartbeat = g.time;
    u.v.x = 1000.;
    let before = u.pos;
    g.tick(DT);
    assert!(g.units[1].pos.distance(before) < 1e-6);
}
#[test]
fn payload_changes_inertia_and_hover_cost_under_finite_force_and_power() {
    let mut empty = fixture().units[1].clone();
    empty.mode = "manual".into();
    empty.pitch = -0.5;
    empty.input = Input {
        thrust_up: true,
        alt_hold: true,
        ..Input::default()
    };
    let mut loaded = empty.clone();
    loaded.cargo_minerals = 100.;
    for _ in 0..80 {
        fly(&mut empty, DT, 0.);
        fly(&mut loaded, DT, 0.);
    }
    assert!(loaded.v.y < empty.v.y * 0.8);
    let mut empty = fixture().units[1].clone();
    let mut loaded = empty.clone();
    loaded.cargo_minerals = 100.;
    fly(&mut empty, DT, 0.);
    fly(&mut loaded, DT, 0.);
    assert!(loaded.power > empty.power * 1.9);
    assert!((loaded.pos.alt - empty.pos.alt).abs() < 0.001);
    loaded.mode = "manual".into();
    loaded.pitch = -80_f64.to_radians();
    loaded.v.y = 2000.;
    loaded.input = Input {
        thrust_up: true,
        boost: true,
        alt_hold: true,
        ..Input::default()
    };
    for _ in 0..80 {
        let energy = loaded.energy;
        fly(&mut loaded, DT, 0.);
        assert!(loaded.power <= 500e6 + 0.001);
        assert!(((energy - loaded.energy) - loaded.power * DT).abs() < 1e-4);
        assert!(loaded.thrust.length() * loaded.mass() <= 100000.1);
    }
}
#[test]
fn dragonfly_tilt_response_and_drag_allow_close_range_dogfights() {
    let mut base = fixture().units[1].clone();
    base.mode = "manual".into();
    base.input = Input {
        forward: true,
        alt_hold: true,
        ..Input::default()
    };
    for _ in 0..20 {
        fly(&mut base, DT, 0.);
    }
    assert!((base.pitch.to_degrees() + 80.).abs() < 0.001);
    let mut speeds = vec![];
    for boost in [false, true] {
        let mut u = base.clone();
        u.input = Input {
            thrust_up: true,
            boost,
            alt_hold: true,
            ..Input::default()
        };
        for _ in 0..2400 {
            fly(&mut u, DT, 0.);
        }
        speeds.push(u.v.x.hypot(u.v.y));
        assert!(u.alive && u.energy > 0.);
    }
    println!(
        "dragonfly 60s speed: normal {:.1}m/s, boost {:.1}m/s",
        speeds[0], speeds[1]
    );
    assert!((80. ..180.).contains(&speeds[0]));
    assert!(speeds[1] > speeds[0] + 10. && speeds[1] < 1200.);
}
#[test]
fn weapon_contact_uses_the_same_queen_hull_and_a_broad_terrain_footprint() {
    let mut g = fixture();
    let q = &mut g.units[0];
    q.pos.alt = 0.;
    q.grounded = true;
    let (centre, radius) = q.hull();
    assert_eq!(radius, 100.);
    assert!((centre.alt - q.pos.alt - 40.).abs() < 0.001);
    let start = centre.offset(-200., 99., 0.).ecef();
    let end = centre.offset(200., 99., 0.).ecef();
    assert!(segment_sphere(start, end, centre.ecef(), radius).is_some());
    assert!(
        centre
            .offset(0., BLAST_R + radius - 0.1, 0.)
            .distance(centre)
            < BLAST_R + radius
    );
    g.patches[0].heights[32 * 65 + 33] = 400.;
    assert!(body_ground(&g.patches, &g.units[0]).unwrap() > 50.);
}
fn flat(lat: f64, lon: f64, height: f64) -> Terrain {
    Terrain {
        lat,
        lon,
        step: 250.,
        size: 65,
        heights: vec![height; 65 * 65],
        shape_cache: Default::default(),
    }
}
fn fixture() -> Lab {
    let mut g = new();
    g.envelope = Envelope::for_test(|lat, lon| {
        if (lat - 30.).abs() < 2. && (lon - 120.).abs() < 2. {
            1000.
        } else if (lat + 35.).abs() < 2. && (lon + 60.).abs() < 2. {
            2000.
        } else {
            0.
        }
    });
    g.epoch = 1;
    g.patches = vec![
        flat(0., 0., 0.),
        flat(30., 120., 1000.),
        flat(-35., -60., 2000.),
    ];
    for (lat, lon) in [(0., 0.), (30., 120.), (-35., -60.)] {
        g.add_hive(Pos { lat, lon, alt: 0. }, 3, QUEEN_E, QUEEN_MINERALS);
    }
    g
}
fn cmd(g: &mut Lab, action: &str, unit: Option<&str>, target: Option<Pos>) -> Result<(), String> {
    let mark = if action == "strike" && g.tutorial.is_none() {
        let id = g.id();
        let hive = unit
            .and_then(|id| g.units.iter().find(|u| u.id == id))
            .map_or(0, |u| u.hive);
        g.marks.push(Mark {
            id,
            hive,
            source: "fixture-scout".into(),
            pos: target.unwrap(),
            expires: g.time + 60.,
            accuracy_m: 10.,
        });
        Some(id)
    } else {
        None
    };
    g.command(&Cmd {
        epoch: g.epoch,
        id: format!("{}-{}", g.time, g.commands.len()),
        action: action.into(),
        unit: unit.map(str::to_owned),
        target,
        hive: Some(0),
        amount: None,
        mark,
    })
}
fn run(g: &mut Lab, secs: f64) {
    for _ in 0..(secs / DT).round() as usize {
        g.tick(DT);
    }
}
#[test]
fn dragonfly_fixes_are_private_fresh_immutable_and_required_for_strikes() {
    let mut g = fixture();
    let drone = g.units[1].id.clone();
    g.units[1].pos = g.units[0].pos.offset(500., 0., -1500.);
    cmd(&mut g, "mark", Some(&drone), None).unwrap();
    let fix = g.marks[0].clone();
    assert_eq!(fix.accuracy_m, 10.);
    assert_eq!(g.view(Some(0)).marks.len(), 1);
    assert!(g.view(Some(1)).marks.is_empty());
    g.units[1].pos = g.units[1].pos.offset(800., 0., 0.);
    assert_eq!(g.marks[0].pos.lon, fix.pos.lon);
    let mut shot = Cmd {
        epoch: g.epoch,
        id: "marked-shot".into(),
        action: "strike".into(),
        unit: Some(g.units[0].id.clone()),
        hive: Some(0),
        amount: None,
        target: Some(Pos {
            lat: 30.,
            lon: 120.,
            alt: 0.,
        }),
        mark: None,
    };
    assert!(g.command(&shot).is_err()); // A clicked/forged target cannot fire.
    shot.mark = Some(fix.id);
    shot.unit = Some(g.units[4].id.clone());
    assert!(g.command(&shot).is_err()); // Another hive cannot use this fix.
    shot.unit = Some(g.units[0].id.clone());
    g.command(&shot).unwrap();
    let charge = g.units[0].charge.as_ref().unwrap();
    assert!(charge.precision && charge.target.distance(fix.pos) <= 10.01);
    assert!((charge.target.alt - fix.pos.alt).abs() < 0.01);
    let queen = g.units[0].id.clone();
    cmd(&mut g, "cancel", Some(&queen), None).unwrap();
    g.time = fix.expires;
    assert!(g
        .command(&Cmd {
            id: "stale-shot".into(),
            ..shot
        })
        .is_err());
    assert!(g.view(Some(0)).marks.is_empty());
    g.units[1].pos.alt = 1600.;
    cmd(&mut g, "mark", Some(&drone), None).unwrap();
    assert_eq!(g.marks[0].accuracy_m, 150.);
    g.units[0].energy = QUEEN_E;
    let mark = g.marks[0].id;
    g.command(&Cmd {
        epoch: g.epoch,
        id: "coarse-shot".into(),
        action: "strike".into(),
        unit: Some(queen),
        hive: Some(0),
        amount: None,
        target: None,
        mark: Some(mark),
    })
    .unwrap();
    let charge = g.units[0].charge.as_ref().unwrap();
    assert!(!charge.precision && charge.target.alt == 0.);
    // A scout near the actual ground can also fix a queen on a high peak.
    g.patches[0].heights.fill(1550.);
    cmd(&mut g, "mark", Some(&drone), None).unwrap();
    assert_eq!(g.marks[0].accuracy_m, 10.);
}
#[test]
fn precision_breaches_hollows_and_altitude_never_overrides_blast_overlap() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    g.envelope = Envelope::for_test(|_, _| 1000.);
    let centre = Pos {
        lat: 0.,
        lon: 0.,
        alt: 100.,
    };
    for (i, u) in g.units.iter_mut().enumerate() {
        u.pos = centre.offset(i as f64 * 40., 0., 0.);
    }
    g.detonate(centre);
    assert!(g.units.iter().all(|u| u.alive));
    g.units[0].pos = centre.offset(PRECISION_R + 99., 0., -QUEEN_HULL_HEIGHT); // Hull centre touches the precision core.
    g.detonate_precision(centre, true);
    assert!(!g.units[0].alive && !g.units[1].alive);
    assert!(g.units[2].alive && g.units[3].alive);
    let safe = Pos {
        alt: 1000. + CLOSE_MARK_ALTITUDE + 1.,
        ..centre
    };
    g.units[2].pos = safe;
    g.detonate_precision(safe, true);
    assert!(!g.units[2].alive && !g.units[2].radar_visible);
    g.units[3].pos = safe.offset(0., 0., BLAST_R + 8. + 1.);
    g.detonate(safe);
    assert!(g.units[3].alive, "beyond the 3D blast and hull survives");
    g.units[0].pos = safe;
    g.units[0].grounded = false;
    g.units[0].alive = true;
    g.detonate(safe);
    assert!(!g.units[0].alive, "an airborne queen inside the blast dies");
    // A high summit is not a free invulnerable resting/reproduction site.
    g.units[0].pos = safe;
    g.units[0].grounded = true;
    g.units[0].alive = true;
    g.detonate_precision(safe, true);
    assert!(!g.units[0].alive);
    assert!(BLAST_R < 1000. && PRECISION_R >= 5. * 10. && PRECISION_R < 150.);
}
#[test]
fn finite_stores_make_flight_costly_and_grounded_broods_need_both_resources() {
    let mut g = fixture();
    let start = g.units[0].energy;
    run(&mut g, 60.);
    let per_minute = start - g.units[0].energy;
    println!(
        "queen hover {:.2} GJ/min; reserve {:.2} min; shot {:.0} GJ; birth {:.0} GJ + {:.0} kg",
        per_minute / 1e9,
        QUEEN_E / per_minute,
        STRIKE_E / 1e9,
        BIRTH_E / 1e9,
        BIRTH_M
    );
    assert!(QUEEN_E / per_minute > 100. && QUEEN_E / per_minute < 130.);
    assert!(DRONE_CARGO_ENERGY / 39. < per_minute); // One sparse full load each five minutes loses energy.
    assert!(DRONE_CARGO_ENERGY / 5. > per_minute + BIRTH_E / 5.); // One full five-minute route funds hover and a five-minute brood cadence.
    let id = g.units[0].id.clone();
    assert!(cmd(&mut g, "reproduce", Some(&id), None).is_err());
    g.units[0].pos.alt = 0.;
    g.units[0].hold = g.units[0].pos;
    g.units[0].grounded = true;
    let count = g.units.len();
    let energy = g.units[0].energy;
    let minerals = g.units[0].minerals;
    cmd(&mut g, "reproduce", Some(&id), None).unwrap();
    assert_eq!(g.units[0].energy, energy - BIRTH_E);
    assert_eq!(g.units[0].minerals, minerals - BIRTH_M);
    assert!(cmd(&mut g, "reproduce", Some(&id), None).is_err());
    run(&mut g, 29.);
    assert_eq!(g.units.len(), count);
    run(&mut g, 1.1);
    assert_eq!(g.units.len(), count + 1);
    let child = g.units.last().unwrap();
    assert!(!child.queen() && child.alive && child.hive == 0 && child.agl < 21.);
    assert!(child.energy <= DRONE_E);
    cmd(&mut g, "reproduce", Some(&id), None).unwrap();
    let spent = g.units[0].energy;
    cmd(&mut g, "lift", Some(&id), None).unwrap();
    assert_eq!(g.units[0].brood_remaining, 0.);
    assert_eq!(g.units[0].energy, spent); // No takeoff refund or free offspring.
    run(&mut g, 31.);
    assert_eq!(g.units.len(), count + 1);
}
#[test]
fn manual_drop_quantities_are_validated_and_preserved() {
    let mut g = fixture();
    let p = g.units[0].pos;
    let mut c = Cmd {
        epoch: g.epoch,
        id: "custom-drop".into(),
        action: "drop_energy".into(),
        unit: None,
        target: Some(p),
        hive: None,
        amount: Some(123e9),
        mark: None,
    };
    g.command(&c).unwrap();
    assert_eq!(g.resources[0].energy, 123e9);
    c.id = "invalid-drop".into();
    c.amount = Some(-1.);
    assert!(g.command(&c).is_err());
    assert_eq!(g.resources.len(), 1);
}
#[test]
fn resource_scan_collect_deliver_is_finite_shared_and_no_random_drops() {
    let mut g = fixture();
    assert!(g.resources.is_empty());
    let p = Pos {
        lat: 0.,
        lon: 0.,
        alt: 0.,
    };
    cmd(&mut g, "drop_energy", None, Some(p)).unwrap();
    cmd(&mut g, "drop_minerals", None, Some(p)).unwrap();
    g.resources[1].minerals = 5000.;
    assert_eq!(g.view(None).resources.len(), 2);
    assert!(g.view(Some(0)).resources.is_empty());
    let id = g.units[1].id.clone();
    g.units[1].pos = p.offset(0., 0., 60.);
    let fuel = g.units[1].energy;
    cmd(&mut g, "scan", Some(&id), None).unwrap();
    assert_eq!(g.units[1].energy, fuel - 50000.);
    cmd(&mut g, "collect", Some(&id), None).unwrap();
    assert_eq!(g.units[1].cargo_energy, DRONE_CARGO_ENERGY);
    assert_eq!(
        g.units[1].energy,
        fuel - 50000.,
        "cargo cannot fuel the carrier"
    );
    cmd(&mut g, "collect", Some(&id), None).unwrap();
    assert_eq!(g.units[1].cargo_minerals, DRONE_CARGO_MINERALS);
    assert_eq!(
        g.resources
            .iter()
            .find(|r| r.minerals > 0.)
            .unwrap()
            .minerals,
        3000.
    );
    g.units[1].pos = g.units[0].pos.offset(0., 100., 0.);
    g.units[0].energy -= 1e11;
    g.units[0].minerals -= 1000.;
    g.units[1].energy -= 1e7;
    let total = g.units[0].energy + g.units[1].energy + g.units[1].cargo_energy;
    cmd(&mut g, "deliver", Some(&id), None).unwrap();
    assert_eq!(g.units[1].cargo_energy, DRONE_CARGO_ENERGY - 1e11);
    assert_eq!(
        g.units[1].cargo_minerals, 1000.,
        "overflow minerals stay aboard"
    );
    assert_eq!(g.units[0].minerals, QUEEN_MINERALS);
    assert_eq!(g.units[1].energy, DRONE_E);
    assert!((g.units[0].energy + g.units[1].energy + g.units[1].cargo_energy - total).abs() < 0.1);
}
#[test]
fn scans_respect_mountains_and_resource_visibility_is_per_hive() {
    let mut g = fixture();
    let p = Pos {
        lat: 0.,
        lon: 0.01,
        alt: 0.,
    };
    cmd(&mut g, "drop_minerals", None, Some(p)).unwrap();
    g.units[1].pos = Pos {
        lat: 0.,
        lon: 0.,
        alt: 100.,
    };
    for j in 0..65 {
        g.patches[0].heights[j * 65 + 34] = 600.;
    }
    let id = g.units[1].id.clone();
    cmd(&mut g, "scan", Some(&id), None).unwrap();
    assert!(g.view(Some(0)).resources.is_empty());
}
#[test]
fn deep_depressions_protect_both_kinds_and_the_boundary_is_strict() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    g.envelope = Envelope::for_test(|_, _| 2000.);
    let p = Pos {
        lat: 0.,
        lon: 0.,
        alt: 1500.,
    };
    g.units[0].pos = Pos { alt: 1000., ..p };
    g.units[0].grounded = false;
    g.units[1].pos = Pos { alt: 1499., ..p };
    g.units[2].pos = Pos { alt: 1500., ..p };
    g.units[3].pos = Pos { alt: 9000., ..p };
    g.detonate(p);
    assert!(g.units[0].alive && g.units[1].alive);
    assert!(!g.units[2].alive);
    assert!(g.units[3].alive);
    assert!(g.units[0].nuclear_sheltered && g.units[1].nuclear_sheltered);
    assert!(!g.units[2].nuclear_sheltered);
    assert_eq!(g.units[0].shelter_altitude, Some(1500.));
    assert_eq!(g.units[0].radar_altitude, Some(3000.));
}
#[test]
fn loading_physics_patches_does_not_redefine_the_global_envelope() {
    let mut g = fixture();
    g.envelope = Envelope::for_test(|_, _| 2300.);
    let before = g.view(None).units[0].terrain_mean;
    g.patches.insert(0, flat(0., 0., 1000.));
    g.patches.push(flat(0.01, 0.01, 4000.));
    let after = g.view(None).units[0].terrain_mean;
    assert_eq!(before, after);
    assert_eq!(after, Some(2300.));
}
#[test]
fn nuclear_touch_kills_airborne_queens_and_drops_their_actual_stores_once() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    g.units[0].pos.alt = 100.;
    let p = g.units[0].pos;
    g.units[0].energy = 7e12;
    g.units[0].minerals = 432.;
    let blast = p.offset(-650., 0., 0.); // hull touches outside the 600 m sphere
    g.detonate(blast);
    assert!(!g.units[0].alive);
    assert!(!g.units[0].grounded);
    assert_eq!(g.resources.len(), 1);
    assert_eq!(g.resources[0].energy, 7e12);
    assert_eq!(g.resources[0].minerals, 432.);
    assert!(g.resources[0].pos.alt > 90.);
    g.blast_damage(blast, false);
    assert_eq!(g.resources.len(), 1);
    assert_eq!(g.view(None).units.len(), 4);
    assert!(!g.view(Some(0)).units[0].alive);
    run(&mut g, 30.);
    assert!(g.resources[0].pos.alt < 1.); // salvage physically falls
}
#[test]
fn queen_vertical_motion_is_slow_but_horizontal_momentum_can_build() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0 && u.queen());
    let id = g.units[0].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    let start = g.units[0].pos.alt;
    g.units[0].input = Input {
        thrust_up: true,
        forward: true,
        alt_hold: true,
        ..Input::default()
    };
    for _ in 0..2400 {
        g.units[0].heartbeat = g.time;
        g.tick(DT);
        assert!(g.units[0].v.z <= 101.);
    }
    assert!(g.units[0].pos.alt - start < 6100.);
    assert!(g.units[0].v.x.hypot(g.units[0].v.y) > 0.4);
    assert!(g.units[0].pitch.abs() <= 0.351);
}
#[test]
fn tutorial_blocks_new_resource_and_ai_commands() {
    let mut g = fixture();
    let id = g.units[1].id.clone();
    cmd(&mut g, "tutorial", Some(&id), None).unwrap();
    let before = serde_json::to_value(g.view(None)).unwrap();
    for action in [
        "drop_energy",
        "drop_minerals",
        "scan",
        "collect",
        "deliver",
        "mark",
        "reproduce",
    ] {
        let p = g.units[0].pos;
        assert!(cmd(&mut g, action, Some(&id), Some(p)).is_err());
    }
    run(&mut g, 30.);
    assert_eq!(before, serde_json::to_value(g.view(None)).unwrap());
}
#[test]
fn tutorial_freezes_physics_and_every_action_until_explicit_exit() {
    let mut g = fixture();
    let id = g.units[0].id.clone();
    cmd(&mut g, "tutorial", Some(&id), None).unwrap();
    assert_eq!(g.tutorial.as_deref(), Some(id.as_str()));
    let before = serde_json::to_value(g.view(None)).unwrap();
    run(&mut g, 120.);
    assert_eq!(serde_json::to_value(g.view(None)).unwrap(), before);
    for action in [
        "pilot",
        "hold",
        "land",
        "lift",
        "strike",
        "cancel",
        "add_queen",
        "add_dragonfly",
    ] {
        let target = Some(Pos {
            lat: 0.,
            lon: 0.,
            alt: 0.,
        });
        assert!(
            cmd(&mut g, action, Some(&id), target).is_err(),
            "tutorial allowed {action}"
        );
    }
    assert_eq!(serde_json::to_value(g.view(None)).unwrap(), before);
    cmd(&mut g, "tutorial_end", Some(&id), None).unwrap();
    run(&mut g, 1.);
    assert!(g.tutorial.is_none() && g.time > 0.);
    assert!(g.units[0].energy < QUEEN_E);
}
#[test]
fn public_actor_samples_use_authoritative_simulation_time() {
    let mut g = fixture();
    run(&mut g, 1.);
    let a = g.view(None);
    assert!(a.units.iter().all(|u| u.sample_time == a.time));
    run(&mut g, 0.1);
    let b = g.view(None);
    assert!(b.time > a.time && b.units.iter().all(|u| u.sample_time == b.time));
}
#[test]
fn queen_can_physically_land_prepare_and_take_off_again() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0 && u.queen());
    let q = g.units[0].id.clone();
    let initial = g.units[0].energy;
    cmd(&mut g, "pilot", Some(&q), None).unwrap();
    cmd(&mut g, "land", Some(&q), None).unwrap();
    for _ in 0..40000 {
        g.units[0].heartbeat = g.time;
        g.tick(DT);
        if g.units[0].grounded {
            break;
        }
    }
    assert!(g.units[0].grounded && g.units[0].agl <= 0.01);
    cmd(&mut g, "lift", Some(&q), None).unwrap();
    for _ in 0..400 {
        g.units[0].heartbeat = g.time;
        g.tick(DT);
    }
    assert!(g.units[0].grounded && g.units[0].mode == "preparing");
    for _ in 0..40000 {
        g.units[0].heartbeat = g.time;
        g.tick(DT);
        if g.units[0].mode == "manual" && g.units[0].agl >= TAKEOFF_CLEARANCE - 2. {
            break;
        }
    }
    assert!(
        g.units[0].alive
            && !g.units[0].grounded
            && g.units[0].mode == "manual"
            && g.units[0].agl >= TAKEOFF_CLEARANCE - 2.,
        "takeoff failed: {} {}",
        g.units[0].mode,
        g.units[0].agl
    );
    assert!(g.units[0].energy < initial - 1e7);
}
#[test]
fn queen_absorbs_spit_and_shields_the_dragonfly_behind_her() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    let p = g.units[1].pos;
    g.units[0].pos = p.offset(0., 150., -50.);
    g.units[0].hold = g.units[0].pos;
    g.units[2].pos = p.offset(0., 300., 0.);
    g.units[2].hold = g.units[2].pos;
    let id = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[1].input = Input {
        alt_hold: true,
        fire: true,
        ..Input::default()
    };
    run(&mut g, 1.);
    assert!(g.units[0].alive && g.units[2].alive);
    assert!(g.bullets.is_empty());
    assert!(!g.bursts.iter().any(|b| b.kind == "melt"));
}
#[test]
fn switching_control_or_disconnect_stops_automatic_landing() {
    let mut g = fixture();
    let q = g.units[0].id.clone();
    cmd(&mut g, "pilot", Some(&q), None).unwrap();
    cmd(&mut g, "land", Some(&q), None).unwrap();
    run(&mut g, 1.);
    assert!(g.units[0].v.z < 0.);
    let d = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&d), None).unwrap();
    assert_eq!(g.units[0].mode, "hold");
    let p = g.units[0].pos;
    run(&mut g, 15.);
    assert!(g.units[0].pos.distance(p) < 1.);
    let mut g = fixture();
    let q = g.units[0].id.clone();
    cmd(&mut g, "land", Some(&q), None).unwrap();
    run(&mut g, 3.);
    assert_eq!(g.units[0].mode, "hold");
}
#[test]
fn forward_tilt_drives_forward_and_spit_follows_the_nose() {
    let mut g = fixture();
    let id = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[1].input = Input {
        forward: true,
        alt_hold: true,
        ..Input::default()
    };
    run(&mut g, 0.3);
    assert!(g.units[1].v.y > 0. && g.units[1].pitch < 0.);
    g.units[1].input.forward = false;
    g.units[1].input.fire = true;
    g.tick(DT);
    let b = &g.bullets[0];
    assert!(b.velocity.dot(g.units[1].pos.basis()[2]) < 0.);
}
#[test]
fn sandbox_physics_is_render_independent_and_step_convergent() {
    let simulate = |fps: usize, step: f64| {
        let mut g = fixture();
        g.units[1].mode = "manual".into();
        g.units[1].pitch = -0.3;
        g.units[1].input.alt_hold = true;
        let mut elapsed = 0.;
        for _ in 0..fps * 30 {
            elapsed += 1. / fps as f64;
            while elapsed + 1e-10 >= step {
                g.units[1].heartbeat = g.time;
                g.tick(step);
                elapsed -= step;
            }
        }
        g.units[1].clone()
    };
    let a = simulate(60, DT);
    for fps in [15, 30] {
        let b = simulate(fps, DT);
        assert!(a.pos.distance(b.pos) < 1e-6);
        assert!((a.energy - b.energy).abs() < 1e-6);
        assert!(a.v.sub(b.v).length() < 1e-9);
    }
    let b = simulate(60, DT / 2.);
    let error = (a.energy - b.energy).abs() / (DRONE_E - a.energy);
    println!("sandbox cadence15/30/60 identical, half-step energy relative error={error:.6}");
    assert!(error < 0.01 && a.pos.distance(b.pos) < 5.);
    assert!(a.pos.distance(fixture().units[1].pos) > 400.);
}
#[test]
fn refresh_seed_changes_three_widely_scattered_hives() {
    let scatter = |seed| {
        crate::distribute::positions(seed, 3)
            .into_iter()
            .map(|p| Pos {
                lat: p.lat,
                lon: p.lon,
                alt: 0.,
            })
            .collect::<Vec<_>>()
    };
    let a = scatter(11);
    let b = scatter(22);
    assert_eq!(a.len(), 3);
    for i in 0..3 {
        assert!(a[i].valid() && a[i].distance(b[i]) > 1000.);
        for j in 0..i {
            assert!(a[i].distance(a[j]) > 1e6);
        }
    }
    assert!(scatter(11).iter().zip(a).all(|(x, y)| x.distance(y) < 1e-6));
}
#[test]
fn swept_flight_and_spit_cannot_tunnel_through_ridges() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    for j in 0..65 {
        g.patches[0].heights[j * 65 + 33] = 2000.;
    }
    let a = Pos {
        lat: 0.,
        lon: 0.,
        alt: 1000.,
    };
    let b = a.offset(500., 0., 0.);
    let hit = terrain_hit(&g.patches, a.ecef(), b.ecef()).unwrap();
    assert!(hit.distance(b) > 100.);
    let p = a.ecef();
    g.bullets.push(Bullet {
        id: 99,
        source: "source".into(),
        hive: 0,
        pos: a,
        velocity: a.basis()[0].mul(20000.),
        age: 0.,
        p,
    });
    g.weapons(DT);
    assert!(g.bullets.is_empty());
}
#[test]
fn visual_world_can_show_near_low_queens_without_global_radar_leak() {
    let mut g = fixture();
    let p = g.units[0].pos.offset(500., 0., -1500.);
    g.units[4].pos = p;
    g.units[4].agl = 100.;
    let id = g.units[4].id.clone();
    assert!(g.view(Some(0)).units.iter().any(|u| u.id == id));
    assert!(!g.view(Some(2)).units.iter().any(|u| u.id == id));
}
#[test]
fn three_hives_hold_globally_without_ai_and_spend_real_energy() {
    let mut g = fixture();
    let before = g.units.clone();
    run(&mut g, 60.);
    assert_eq!(g.units.len(), 12);
    for (u, b) in g.units.iter().zip(before) {
        assert!(u.alive && u.pos.distance(b.pos) < 0.01);
        assert!(u.energy < b.energy && u.energy > 0.);
        assert!(u.power > 0.);
        assert!(u.agl >= 1599.9);
        assert_eq!(u.mode, "hold");
    }
    assert!(g.bullets.is_empty() && g.missiles.is_empty());
}
#[test]
fn radar_is_global_smoothed_altitude_and_hive_perspective() {
    let mut g = fixture();
    g.units[4].pos.alt = 1999.;
    g.units[4].agl = 5000.;
    g.units[5].pos.alt = 2000.;
    g.units[5].agl = 0.;
    g.units[6].pos.alt = 2001.;
    g.units[6].agl = 0.;
    let a = g.view(Some(0));
    assert!(!a.units.iter().any(|u| u.id == g.units[4].id));
    assert!(a.units.iter().any(|u| u.id == g.units[5].id));
    assert!(a.units.iter().any(|u| u.id == g.units[6].id));
    assert!(a.units.iter().any(|u| u.hive == 2));
    assert!(g.view(Some(1)).units.iter().any(|u| u.id == g.units[4].id));
}
#[test]
fn pilot_switch_and_release_hold_current_position_without_refill() {
    let mut g = fixture();
    let id = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[1].input.thrust_up = true;
    g.units[1].input.alt_hold = true;
    for _ in 0..40 {
        g.units[1].heartbeat = g.time;
        g.tick(DT);
    }
    let e = g.units[1].energy;
    let p = g.units[1].pos;
    cmd(&mut g, "hold", Some(&id), None).unwrap();
    run(&mut g, 15.);
    assert!(g.units[1].pos.distance(p) < 1.);
    assert!(g.units[1].energy < e);
    assert!(g.units.iter().filter(|u| u.mode == "manual").count() == 0);
}
#[test]
fn disconnected_pilot_stops_at_location_not_at_queen() {
    let mut g = fixture();
    let id = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[1].input.thrust_up = true;
    run(&mut g, 20.);
    assert_eq!(g.units[1].mode, "hold");
    assert!(g.units[1].pos.distance(g.units[0].pos) > 100.);
}
#[test]
fn spit_is_ballistic_friendly_fire_one_hit_and_never_hurts_queen() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    let source = g.units[1].pos;
    g.units[1].heading = 0.;
    let target = source.offset(0., 100., 0.);
    g.units[2].pos = target;
    g.units[2].hold = target;
    g.units[0].pos = source.offset(400., 50., 0.);
    g.units[0].hold = g.units[0].pos;
    let id = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[1].input = Input {
        alt_hold: true,
        fire: true,
        ..Input::default()
    };
    let e = g.units[1].energy;
    run(&mut g, 0.1);
    assert!(g.units[1].energy < e - 1999.);
    assert_eq!(g.bullets.len(), 1);
    let b = &g.bullets[0];
    assert!(b.pos.alt < source.alt && b.velocity.length() < 300.);
    run(&mut g, 0.6);
    assert!(!g.units[2].alive);
    assert!(g.units[0].alive);
    assert!(g.bursts.iter().any(|b| b.kind == "melt"));
}
#[test]
fn empty_reserve_falls_with_assist_and_gravity_is_not_a_toggle() {
    let mut g = fixture();
    let u = &mut g.units[1];
    u.energy = 0.;
    u.thrust = Vec3::default();
    let z = u.pos.alt;
    run(&mut g, 1.);
    assert!(g.units[1].pos.alt < z - 4.);
    assert_eq!(g.units[1].energy, 0.);
}
#[test]
fn queen_is_massive_expensive_and_force_limited() {
    let mut g = fixture();
    let q = &g.units[0];
    assert!(q.mass() / g.units[1].mass() >= 1e5);
    let qp = q.power;
    assert_eq!(qp, 0.);
    run(&mut g, 1.);
    assert!(g.units[0].power / g.units[1].power > 999.);
    let id = g.units[0].id.clone();
    cmd(&mut g, "pilot", Some(&id), None).unwrap();
    g.units[0].input = Input {
        forward: true,
        thrust_up: true,
        alt_hold: true,
        ..Input::default()
    };
    for _ in 0..400 {
        g.units[0].heartbeat = g.time;
        g.tick(DT);
        assert!(g.units[0].thrust.x.hypot(g.units[0].thrust.y) <= QUEEN_LATERAL + 0.001);
    }
    assert!(g.units[0].v.x.hypot(g.units[0].v.y) < 41.);
}
#[test]
fn powered_strikes_reach_fixed_targets_in_one_to_five_minutes_including_antipodes() {
    for (lon, lat) in [
        (0., 0.),
        (0.01, 0.),
        (40., 20.),
        (120., -30.),
        (179.999, 0.),
        (180., 0.),
        (-180., 0.),
    ] {
        let start = Pos {
            lat: 0.,
            lon: 0.,
            alt: 1600.,
        };
        let target = Pos { lat, lon, alt: 0. };
        let (v, duration, path) = plan(start, target).unwrap();
        assert!((60. ..=300.).contains(&duration));
        if lon.abs() == 180. {
            assert_eq!(duration, 300.);
        }
        let mut m = Missile {
            precision: false,
            id: 1,
            hive: 0,
            pos: start,
            start,
            target,
            age: 0.,
            duration,
            path,
            p: start.ecef(),
            velocity: v,
        };
        let mut low = f64::INFINITY;
        while m.age + 1e-8 < duration {
            let step = DT.min(duration - m.age);
            step_strike(&mut m, step);
            low = low.min(m.pos.alt);
        }
        let error = m.pos.distance(target);
        println!(
            "powered lon={lon} lat={lat} flight={duration:.3}s error={error:.3}m min_alt={low:.3}m"
        );
        assert!(error < 5.);
        assert!(low > -1.);
        assert_eq!(m.target.lat, target.lat);
        assert_eq!(m.target.lon, target.lon);
    }
}
#[test]
fn queen_ground_to_full_blast_escape_is_about_seven_minutes() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0 && u.queen());
    let origin = Pos {
        lat: 0.,
        lon: 0.,
        alt: 0.,
    };
    let q = &mut g.units[0];
    q.pos = origin;
    q.hold = origin;
    q.grounded = true;
    q.v = Vec3::default();
    q.agl = 0.;
    let id = q.id.clone();
    cmd(&mut g, "lift", Some(&id), None).unwrap();
    let mut steering_at = 0.;
    while g.time < 600. {
        let q = &mut g.units[0];
        q.heartbeat = g.time;
        if q.mode == "manual" {
            if steering_at == 0. {
                steering_at = g.time;
            }
            q.input = Input {
                forward: true,
                alt_hold: true,
                ..Input::default()
            };
        }
        g.tick(DT);
        if g.units[0].pos.distance(origin) > BLAST_R + 100. {
            break;
        }
    }
    println!(
        "queen rest-to-clear {:.2}s; steering {:.2}s; speed {:.2}m/s; range {:.2}m",
        g.time,
        steering_at,
        g.units[0].v.length(),
        g.units[0].pos.distance(origin)
    );
    assert!(steering_at >= 389. && steering_at < 410.);
    assert!((415. ..=425.).contains(&g.time));
    assert!(g.units[0].v.length() > 50. && g.units[0].v.length() < 101.);
    assert!(g.units[0].energy < QUEEN_E);
}
#[test]
fn charge_requires_radar_exposure_and_takes_thirty_seconds() {
    let mut g = fixture();
    let id = g.units[0].id.clone();
    let target = g.units[0].pos.offset(1000., 0., -1600.);
    g.units[0].pos.alt = 999.;
    assert!(cmd(&mut g, "strike", Some(&id), Some(target)).is_err());
    g.units[0].pos.alt = 1000.;
    cmd(&mut g, "strike", Some(&id), Some(target)).unwrap();
    assert_eq!(g.units[0].charge.as_ref().unwrap().remaining, 30.);
    run(&mut g, 29.);
    assert!(g.missiles.is_empty());
    run(&mut g, 1.1);
    assert_eq!(g.missiles.len(), 1);
}
#[test]
fn strike_reserves_huge_energy_cancels_and_rejects_grounded_queen() {
    let mut g = fixture();
    let id = g.units[0].id.clone();
    let target = g.units[0].pos.offset(1000., 0., -1600.);
    let e = g.units[0].energy;
    cmd(&mut g, "strike", Some(&id), Some(target)).unwrap();
    assert_eq!(g.units[0].energy, e - STRIKE_E);
    assert!(cmd(&mut g, "strike", Some(&id), Some(target)).is_err());
    cmd(&mut g, "cancel", Some(&id), None).unwrap();
    assert_eq!(g.units[0].energy, e - STRIKE_E * 0.25);
    g.units[0].grounded = true;
    assert!(cmd(&mut g, "strike", Some(&id), Some(target)).is_err());
}
#[test]
fn launched_strike_is_fixed_aim_global_visible_and_uses_actual_impact() {
    let mut g = fixture();
    let id = g.units[0].id.clone();
    let target = Pos {
        alt: 0.,
        ..g.units[0].pos.offset(1000., 0., 0.)
    };
    cmd(&mut g, "strike", Some(&id), Some(target)).unwrap();
    run(&mut g, CHARGE + 0.1);
    assert_eq!(g.missiles.len(), 1);
    assert!(g.units[0].cooldown > 179.);
    assert_eq!(g.view(Some(2)).missiles.len(), 1);
    assert!(g.missiles[0].target.distance(target) <= 10.01);
    run(&mut g, 61.);
    assert!(g.missiles.is_empty());
    assert!(g
        .bursts
        .iter()
        .any(|b| b.kind == "nuclear" && b.pos.distance(target) < 50.));
}
#[test]
fn blast_is_3d_large_friendly_and_a_ridge_alone_is_not_shelter() {
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    let p = Pos {
        lat: 0.,
        lon: 0.,
        alt: 0.,
    };
    g.units[0].pos = p;
    g.units[0].grounded = true;
    g.units[1].pos = p.offset(0., 500., 100.);
    g.units[2].pos = p.offset(0., 0., 7000.);
    g.detonate(p);
    assert!(!g.units[0].alive && !g.units[1].alive && g.units[2].alive);
    let mut g = fixture();
    g.units.retain(|u| u.hive == 0);
    for j in 0..65 {
        g.patches[0].heights[j * 65 + 33] = 500.;
    }
    g.units[0].pos = p.offset(500., 0., 0.);
    g.units[0].grounded = true;
    g.detonate(p);
    assert!(!g.units[0].alive); // A wall alone does not make a deep depression.
}
#[test]
fn editor_additions_and_replay_epoch_are_validated() {
    let mut g = fixture();
    let p = Pos {
        lat: 0.,
        lon: 0.,
        alt: 0.,
    };
    cmd(&mut g, "add_queen", None, Some(p)).unwrap();
    assert_eq!(g.units.len(), 13);
    cmd(&mut g, "add_dragonfly", None, Some(p)).unwrap();
    assert_eq!(g.units.len(), 14);
    let c = Cmd {
        epoch: 0,
        id: "stale".into(),
        action: "add_queen".into(),
        target: Some(p),
        unit: None,
        hive: None,
        amount: None,
        mark: None,
    };
    assert!(g.command(&c).is_err());
    let c = Cmd {
        epoch: g.epoch,
        id: g.commands[0].clone(),
        ..c
    };
    assert!(g.command(&c).is_err());
}

#[test]
fn succession_is_independent_and_only_for_survivors() {
    let mut g = fixture();
    let fuel = g.units[2].energy;
    g.units[0].alive = false;
    g.units[1].alive = false;
    g.settle_deaths();
    g.succession();
    let count = g.units.len();
    assert!(!g.units[1].founder);
    assert!(g.units[2].founder && g.units[3].founder);
    assert_ne!(g.units[2].hive, g.units[3].hive);
    assert_eq!(g.units[2].energy, fuel);
    assert_eq!(g.units[2].lineage, vec![0]);
    assert_eq!(g.units[3].lineage, vec![0]);
    assert!(g.units[1].lineage.is_empty());
    let h = g.units[2].hive;
    g.succession();
    assert_eq!(g.units[2].hive, h);
    assert_eq!(g.units.len(), count);
}
#[test]
fn founding_conserves_body_energy_and_overflow() {
    let mut g = fixture();
    let u = &mut g.units[1];
    u.founder = true;
    u.grounded = true;
    u.pos.alt = 0.;
    u.hold = u.pos;
    u.cargo_minerals = 2000.;
    u.cargo_energy = 200e9;
    let before = u.energy + u.cargo_energy;
    let id = u.id.clone();
    cmd(&mut g, "found", Some(&id), None).unwrap();
    g.build(120.);
    let u = &g.units[1];
    assert!(u.queen());
    assert_eq!(u.dry_mass, 1000.);
    assert_eq!(u.minerals, 1100.);
    assert_eq!(u.energy, FOUNDER_E);
    assert!(
        (u.energy + g.resources.iter().map(|r| r.energy).sum::<f64>() + 4.5e9 - before).abs() < 1.
    );
    assert!(cmd(&mut g, "found", Some(&id), None).is_err());
}
#[test]
fn founding_requires_actual_material_and_ordinary_workers_cannot_self_fuel() {
    let mut g = fixture();
    let id = g.units[1].id.clone();
    g.units[1].cargo_energy = 200e9;
    assert!(cmd(&mut g, "refuel_self", Some(&id), None).is_err());
    g.units[1].founder = true;
    g.units[1].grounded = true;
    assert!(cmd(&mut g, "found", Some(&id), None).is_err());
    g.units[1].energy = 0.;
    cmd(&mut g, "refuel_self", Some(&id), None).unwrap();
    assert_eq!(g.units[1].energy, DRONE_E);
    assert_eq!(g.units[1].cargo_energy, 200e9 - DRONE_E);
}
#[test]
fn growth_pays_for_mass_and_increases_capacity_and_productivity() {
    let mut g = fixture();
    let u = &mut g.units[0];
    u.dry_mass = 1000.;
    u.grounded = true;
    u.minerals = 1000.;
    u.energy = 100e9;
    u.update_capacity();
    let interval = u.brood_time();
    let id = u.id.clone();
    cmd(&mut g, "grow", Some(&id), None).unwrap();
    g.build(1.);
    let u = &g.units[0];
    assert_eq!(u.dry_mass, 1005.);
    assert_eq!(u.minerals, 995.);
    assert_eq!(u.energy, 100e9 - 25e6);
    assert!(u.capacity > 100e9 && u.brood_time() < interval);
}
#[test]
fn high_altitude_flight_does_not_wait_for_gaussian_or_dem() {
    let mut g = fixture();
    g.units.truncate(1);
    g.patches.clear();
    g.units[0].pos.alt = 80000.;
    g.units[0].hold = g.units[0].pos;
    g.units[0].v.x = 1000.;
    let start = g.units[0].pos;
    g.tick(DT);
    assert!(g.units[0].pos.distance(start) > 20.);
    assert!(g.units[0].radar_visible && !g.units[0].nuclear_sheltered);
}
#[test]
fn global_boost_obeys_power_force_and_spends_real_energy() {
    let mut u = fixture().units[1].clone();
    u.pos.alt = 80000.;
    u.mode = "manual".into();
    u.input = Input {
        boost: true,
        forward: true,
        alt_hold: true,
        ..Default::default()
    };
    u.cargo_minerals = 2000.;
    let e = u.energy;
    for _ in 0..1000 {
        fly(&mut u, DT, 0.);
        assert!(u.power <= 500e6 + 1.);
        assert!(u.thrust.length() * u.mass() <= 100000.1);
    }
    assert!(u.v.x.hypot(u.v.y) > 100.);
    assert!(u.energy < e);
}

#[test]
fn engine_global_sortie_with_full_payload_fits_fuel_and_time_budget() {
    fn leg(cargo: f64, distance: f64) -> (f64, f64) {
        let mut u = fixture().units[1].clone();
        u.pos = Pos {
            lat: 0.,
            lon: 0.,
            alt: 1.,
        };
        u.heading = std::f64::consts::FRAC_PI_2;
        u.mode = "manual".into();
        u.cargo_minerals = cargo;
        u.energy = DRONE_E;
        let mut time = 0.;
        for phase in 0..3 {
            let mut complete = false;
            for _ in 0..200000 {
                let x = R * u.pos.lon.to_radians();
                let speed = u.v.x;
                u.input = Input {
                    boost: true,
                    alt_hold: true,
                    ..Default::default()
                };
                u.pitch = 0.;
                match phase {
                    0 => {
                        let left = 80000. - u.pos.alt;
                        if left.abs() < 30. && u.v.z.abs() < 3. {
                            complete = true;
                            break;
                        }
                        u.input.thrust_up =
                            left > u.v.z.max(0.).powi(2) / 40. + u.v.z.max(0.) * 0.2 + 5.;
                    }
                    1 => {
                        let left = distance - x;
                        if left.abs() < 1000. && speed.abs() < 3. {
                            complete = true;
                            break;
                        }
                        let target = if left > 0. {
                            (40. * left).sqrt().min(4000.)
                        } else {
                            0.
                        };
                        if speed < target - 2. {
                            u.pitch = -1.4;
                        } else if speed > target + 2. {
                            u.pitch = 1.4;
                        }
                    }
                    _ => {
                        let left = u.pos.alt - 20.;
                        if left.abs() < 30. && u.v.z.abs() < 3. {
                            complete = true;
                            break;
                        }
                        u.input.thrust_down =
                            left > u.v.z.min(0.).powi(2) / 20. + u.v.z.abs() + 30.;
                    }
                }
                fly(&mut u, DT, 0.);
                time += DT;
                assert!(
                    u.energy > 0.,
                    "fuel exhausted phase={phase} cargo={cargo} altitude={} x={x}",
                    u.pos.alt
                );
                assert!(u.power <= 500e6 + 1.);
            }
            assert!(
                complete,
                "phase {phase} failed: altitude={} velocity={:?}",
                u.pos.alt, u.v
            );
        }
        (time, (DRONE_E - u.energy))
    }
    for distance in [1900000., 10700000.] {
        let a = leg(0., distance);
        let b = leg(2000., distance);
        let minutes = (a.0 + b.0) / 60.;
        let energy = (a.1 + b.1) * 1.2;
        println!(
            "engine {distance}m roundtrip: {minutes:.2}min; with20pct contingency {:.2}GJ",
            energy / 1e9
        );
        let predicted = if distance < 2e6 { 39. } else { 112. };
        assert!((minutes / predicted - 1.).abs() < 0.15);
        assert!(energy < DRONE_E);
    }
}

#[test]
fn grounded_queen_skips_full_energy_and_collects_needed_minerals() {
    let mut g = fixture();
    g.units[0].dry_mass = 1000.;
    g.units[0].update_capacity();
    g.units[0].energy = FOUNDER_E;
    g.units[0].minerals = 0.;
    g.units[0].grounded = true;
    g.units[0].pos.alt = 0.;
    let p = g.units[0].pos;
    for (id, energy, minerals, pos) in [(900, 10e9, 0., p), (901, 0., 500., p.offset(5., 0., 0.))] {
        g.resources.push(Resource {
            id,
            pos,
            energy,
            minerals,
            discovered: vec![0],
            fall_speed: 0.,
        });
    }
    let id = g.units[0].id.clone();
    cmd(&mut g, "collect", Some(&id), None).unwrap();
    assert_eq!(g.units[0].minerals, 500.);
    assert_eq!(g.units[0].energy, FOUNDER_E);
    assert_eq!(g.resources[0].energy, 10e9);
}

pub(super) mod heap {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! {
        static TRACK:Cell<bool>=const {Cell::new(false)};
        static COUNT:Cell<usize>=const {Cell::new(0)};
    }
    struct Counter;
    #[global_allocator]
    static ALLOCATOR: Counter = Counter;
    unsafe impl GlobalAlloc for Counter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            TRACK.with(|on| {
                if on.get() {
                    COUNT.with(|n| n.set(n.get() + 1));
                }
            });
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            TRACK.with(|on| {
                if on.get() {
                    COUNT.with(|n| n.set(n.get() + 1));
                }
            });
            System.realloc(ptr, layout, size)
        }
    }
    pub fn count(f: impl FnOnce()) -> usize {
        COUNT.with(|n| n.set(0));
        TRACK.with(|on| on.set(true));
        f();
        TRACK.with(|on| on.set(false));
        COUNT.with(Cell::get)
    }
}
#[test]
fn warm_twelve_member_ticks_reuse_storage() {
    let mut g = fixture();
    g.bursts.push(Burst {
        id: 999,
        pos: Pos {
            lat: 60.,
            lon: 140.,
            alt: 0.,
        },
        age: 0.,
        radius: BLAST_R,
        kind: "nuclear".into(),
        precision: false,
    });
    for _ in 0..20 {
        g.tick(DT);
    }
    let scratch = g.scratch.as_ref().unwrap();
    let buffers = (
        scratch.hatches.as_ptr(),
        scratch.fire.as_ptr(),
        scratch.launch.as_ptr(),
        scratch.active.as_ptr(),
    );
    let mut timings = [0u128; 200];
    let allocations = heap::count(|| {
        for sample in &mut timings {
            let start = std::time::Instant::now();
            g.tick(DT);
            *sample = start.elapsed().as_nanos();
        }
    });
    let scratch = g.scratch.as_ref().unwrap();
    assert_eq!(
        buffers,
        (
            scratch.hatches.as_ptr(),
            scratch.fire.as_ptr(),
            scratch.launch.as_ptr(),
            scratch.active.as_ptr()
        )
    );
    timings.sort_unstable();
    println!("warm 12-member physics: {} allocations / 200 ticks; p50={}us p99={}us max={}us (debug build, synthetic flat terrain)",allocations,timings[100]/1000,timings[198]/1000,timings[199]/1000);
    let engine_allocations = heap::count(|| {
        for _ in 0..2400 {
            crate::physics::advance(Vec3::new(R + 1600., 0., 0.), Vec3::ZERO, Vec3::ZERO, DT);
        }
    });
    println!(
        "same 2400 bare Rapier advances: {} allocations",
        engine_allocations
    );
    assert_eq!(
        allocations, engine_allocations,
        "application tick must add no allocations beyond the engine"
    );
}

fn swarm_fixture() -> Lab {
    let mut g = fixture();
    g.units.truncate(2);
    g.units[0].pos.alt = 0.;
    g.units[0].grounded = true;
    g.units[0].hold = g.units[0].pos;
    g.units[0].energy = START_QUEEN_E;
    g.units[0].minerals = START_QUEEN_MINERALS;
    g.units[1].pos = Pos {
        lat: 0.,
        lon: 0.002,
        alt: 300.,
    };
    g.units[1].hold = g.units[1].pos;
    g.resources.push(Resource {
        id: 500,
        pos: Pos {
            lat: 0.,
            lon: 0.012,
            alt: 0.,
        },
        energy: DRONE_CARGO_ENERGY,
        minerals: 2000.,
        discovered: vec![],
        fall_speed: 0.,
    });
    g.ai.brains.insert(0, ai::Brain::new(0));
    g
}
#[test]
fn swarm_physically_scans_collects_delivers_and_refuels() {
    let mut g = swarm_fixture();
    let start = g.units[0].energy;
    let total = |g: &Lab| {
        g.units
            .iter()
            .map(|u| u.energy + u.cargo_energy)
            .sum::<f64>()
            + g.resources.iter().map(|r| r.energy).sum::<f64>()
    };
    let initial = total(&g);
    for step in 0..24000 {
        while let Some(p) = g.ai.land_requests.pop_front() {
            g.ai.record_land(p, true);
        }
        g.tick(DT);
        if step % 2400 == 0 {
            eprintln!(
                "swarm t={} p={:?} v={:?} energy={} cargo={} status={}",
                g.time,
                g.units[1].pos,
                g.units[1].v,
                g.units[1].energy,
                g.units[1].cargo_energy,
                g.ai.status(0)
            );
        }
        if !g.units[1].alive {
            break;
        }
        if g.ai.brains[&0].status_deliveries() > 0 {
            break;
        }
    }
    assert!(total(&g) <= initial, "supply must never create energy");
    assert!(
        g.units[0].minerals > START_QUEEN_MINERALS,
        "full mineral load never delivered"
    );
    assert!(g.units[1].alive, "courier died on terrain");
    assert!(
        g.ai.brains[&0].status_deliveries() > 0,
        "supply loop never delivered"
    );
    assert!(
        g.units[0].energy > start,
        "queen must receive useful net energy"
    );
}

#[test]
fn editor_stores_match_startup_and_founders_can_afford_a_first_brood() {
    let mut g = fixture();
    let p = g.units[0].pos;
    cmd(&mut g, "add_queen", None, Some(p)).unwrap();
    let q = g.units.last().unwrap();
    assert_eq!(
        (q.energy, q.minerals),
        (START_QUEEN_E, START_QUEEN_MINERALS)
    );
    let i = 1;
    g.units[i].founder = true;
    g.units[i].grounded = true;
    g.units[i].pos.alt = 0.;
    g.units[i].hold = g.units[i].pos;
    g.units[i].cargo_energy = FOUNDER_E + 4.5e9 - g.units[i].energy;
    g.units[i].cargo_minerals = FOUNDER_MINERALS;
    let id = g.units[i].id.clone();
    cmd(&mut g, "found", Some(&id), None).unwrap();
    g.build(120.);
    assert_eq!(g.units[i].minerals, BIRTH_M);
    assert_eq!(g.units[i].capacity, FOUNDER_E);
    cmd(&mut g, "reproduce", Some(&id), None).unwrap();
    assert_eq!(g.units[i].energy, FOUNDER_E - BIRTH_E);
    assert!(g.units[i].brood_remaining > 0.);
}

#[test]
fn activate_all_enables_existing_hives_without_spawning_or_taking_manual_control() {
    let mut empty = new();
    empty.epoch = 1;
    cmd(&mut empty, "ai_enable_all", None, None).unwrap();
    assert!(empty.ai.brains.is_empty());
    assert!(empty.units.is_empty() && empty.resources.is_empty());
    let mut g = fixture();
    let pilot = g.units[1].id.clone();
    cmd(&mut g, "pilot", Some(&pilot), None).unwrap();
    let bodies = serde_json::to_value(&g.units).unwrap();
    let count = g.units.len();
    cmd(&mut g, "ai_enable_all", None, None).unwrap();
    let hives: std::collections::BTreeSet<_> =
        g.units.iter().filter(|u| u.alive).map(|u| u.hive).collect();
    assert_eq!(
        g.ai.brains
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        hives
    );
    assert_eq!(g.ai.manual.as_deref(), Some(pilot.as_str()));
    assert_eq!(g.units.len(), count);
    assert_eq!(serde_json::to_value(&g.units).unwrap(), bodies);
    assert!(g.resources.is_empty());
    cmd(&mut g, "ai_enable_all", None, None).unwrap();
    assert_eq!(g.ai.brains.len(), hives.len());
    let manual = serde_json::to_value(&g.units[1]).unwrap();
    g.units[2].alive = false;
    g.units[2].mode = "dead".into();
    cmd(&mut g, "ai_disable_all", None, None).unwrap();
    assert!(g.ai.brains.is_empty());
    assert_eq!(serde_json::to_value(&g.units[1]).unwrap(), manual);
    assert_eq!(g.ai.manual.as_deref(), Some(pilot.as_str()));
    assert_eq!(g.units[2].mode, "dead");
    assert!(g
        .units
        .iter()
        .filter(|u| u.alive && u.id != pilot)
        .all(|u| u.mode == "hold"));
    assert!(!g.view(None).all_hives_active);
    cmd(&mut g, "ai_disable_all", None, None).unwrap();
    g.tutorial = Some(pilot);
    assert!(cmd(&mut g, "ai_enable_all", None, None).is_err());
    assert!(cmd(&mut g, "ai_disable_all", None, None).is_err());
}

#[test]
fn heraldry_survives_founding_brood_and_another_succession() {
    let mut g = fixture();
    g.units[0].alive = false;
    g.succession();
    let i = 1;
    let h = g.units[i].hive;
    let id = g.units[i].id.clone();
    g.units[i].grounded = true;
    g.units[i].cargo_energy = FOUNDER_E + 4.5e9 - g.units[i].energy;
    g.units[i].cargo_minerals = FOUNDER_MINERALS;
    cmd(&mut g, "found", Some(&id), None).unwrap();
    g.build(120.);
    assert_eq!(g.units[i].lineage, vec![0]);
    g.add_drone(h, g.units[i].pos);
    let last = g.units.len() - 1;
    assert_eq!(g.units[last].lineage, vec![0]);
    g.units[i].alive = false;
    g.succession();
    assert_eq!(g.units[last].lineage, vec![0, h]);
    assert_ne!(g.units[last].hive, h);
}
