//! Deterministic, isolated distribution of hives and mixed resource deposits.
//!
//! This module is self-contained on purpose: it depends only on `serde` and a
//! local copy of splitmix64, so it can be dropped in and unit-tested without
//! touching `lab.rs`. It generates a *plan* — hives and deposits as plain data
//! — rather than mutating the live `Lab`, because `Body` and `Lab` are not
//! externally constructible (their constructors and several fields are private
//! to the `lab` module).
//!
//! Applying the plan to the live world is a small, separate wiring step that
//! lives in `lab::reset` (or a new endpoint), not here:
//!
//! ```ignore
//! let dist = distribute::plan(&cfg);
//! for h in &dist.hives {
//!     g.add_hive(Pos { lat: h.at.lat, lon: h.at.lon, alt: 0.0 }, h.drones);
//!     // then set the queen's energy/minerals from h.queen_energy / h.queen_minerals
//! }
//! for d in &dist.deposits {
//!     g.resources.push(Resource {
//!         id: g.id(),
//!         pos: Pos { lat: d.at.lat, lon: d.at.lon, alt: ground },
//!         energy: d.energy,
//!         minerals: d.minerals,
//!         discovered: vec![],
//!         fall_speed: 0.0,
//!     });
//! }
//! ```
//!
//! Triggering: call `distribute::plan(&config)` directly, or deserialize a
//! `StartConfig` from the startup screen and pass it through.

use serde::Deserialize;

/// Starting configuration for one distributed world, shaped by the startup
/// screen. `#[serde(default)]` keeps every field optional in the wire payload,
/// so `{}` is a valid request and `{ "seed": 1, "hives": 5 }` overrides only
/// those fields.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct StartConfig {
    /// Determinism seed for hive and deposit placement.
    pub seed: u64,
    /// Number of hives to distribute ("distribute N hives").
    pub hives: usize,
    /// Dragonflies spawned per hive alongside its queen.
    pub drones_per_hive: usize,
    /// Starting energy store for each queen, in joules.
    pub queen_energy: f64,
    /// Starting mineral store for each queen, in kilograms.
    pub queen_minerals: f64,
    /// Number of mixed deposits; each carries both energy and minerals.
    pub deposits: usize,
    /// Energy carried by each mixed deposit, in joules.
    pub energy_per_deposit: f64,
    /// Minerals carried by each mixed deposit, in kilograms.
    pub minerals_per_deposit: f64,
}

impl Default for StartConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            hives: 0,
            drones_per_hive: 3,
            queen_energy: 1e12,   // 1 TJ per queen
            queen_minerals: 500., // Five additional births before mineral deliveries
            deposits: 0,
            energy_per_deposit: 250e9, // One worker load
            minerals_per_deposit: 5e3, // 5000 kg per mixed deposit
        }
    }
}

/// A latitude/longitude anchor.
#[derive(Clone, Copy, Debug)]
pub struct Coord {
    pub lat: f64,
    pub lon: f64,
}

/// A hive: one queen plus `drones` dragonflies, with starting stores.
#[derive(Clone, Debug)]
pub struct Hive {
    pub at: Coord,
    pub drones: usize,
    pub queen_energy: f64,
    pub queen_minerals: f64,
}

/// A mixed deposit carrying both energy and minerals.
#[derive(Clone, Debug)]
pub struct Deposit {
    pub at: Coord,
    pub energy: f64,
    pub minerals: f64,
}

/// The full generated distribution.
#[derive(Clone, Debug, Default)]
pub struct Distribution {
    pub hives: Vec<Hive>,
    pub deposits: Vec<Deposit>,
}

/// Local splitmix64 so this module stays self-contained (mirrors the crate-root
/// `splitmix64`; no dependency on it so the file can be moved or tested alone).
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

/// Scatter `n` positions deterministically around the globe: evenly spaced in
/// longitude with per-point jitter, and a uniform latitude distribution.
pub fn positions(seed: u64, n: usize) -> Vec<Coord> {
    if n == 0 {
        return Vec::new();
    }
    let mut r = splitmix64(seed);
    let rotation = (r as f64 / u64::MAX as f64) * 360.;
    (0..n)
        .map(|i| {
            r = splitmix64(r);
            let lat = ((r as f64 / u64::MAX as f64 * 2. - 1.) * 0.9659)
                .asin()
                .to_degrees();
            r = splitmix64(r);
            let jitter = (r as f64 / u64::MAX as f64 * 2. - 1.) * 25.;
            let lon =
                (rotation + i as f64 * (360. / n as f64) + jitter + 540.).rem_euclid(360.) - 180.;
            Coord { lat, lon }
        })
        .collect()
}

/// Generate the complete distribution from a starting config. Pure and
/// deterministic: the same `StartConfig` always yields the same plan.
pub fn plan(cfg: &StartConfig) -> Distribution {
    let mut d = Distribution::default();
    // Distinct seeds keep hive and deposit positions from deterministically
    // colliding while staying reproducible.
    for at in positions(cfg.seed, cfg.hives) {
        d.hives.push(Hive {
            at,
            drones: cfg.drones_per_hive,
            queen_energy: cfg.queen_energy,
            queen_minerals: cfg.queen_minerals,
        });
    }
    for at in positions(cfg.seed ^ 0xd1b54a32d192ed03, cfg.deposits) {
        d.deposits.push(Deposit {
            at,
            energy: cfg.energy_per_deposit,
            minerals: cfg.minerals_per_deposit,
        });
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = plan(&StartConfig::default());
        let b = plan(&StartConfig::default());
        assert_eq!(a.hives.len(), b.hives.len());
        assert_eq!(a.deposits.len(), b.deposits.len());
        for (x, y) in a.hives.iter().zip(&b.hives) {
            assert_eq!(x.at.lat, y.at.lat);
            assert_eq!(x.at.lon, y.at.lon);
        }
        for (x, y) in a.deposits.iter().zip(&b.deposits) {
            assert_eq!(x.at.lat, y.at.lat);
            assert_eq!(x.at.lon, y.at.lon);
        }
    }

    #[test]
    fn counts_and_validity() {
        let cfg = StartConfig {
            hives: 5,
            drones_per_hive: 2,
            deposits: 12,
            ..Default::default()
        };
        let d = plan(&cfg);
        assert_eq!(d.hives.len(), 5);
        assert!(d.hives.iter().all(|h| h.drones == 2));
        assert_eq!(d.deposits.len(), 12);
        for h in &d.hives {
            assert!((-90.0..=90.0).contains(&h.at.lat));
            assert!((-180.0..=180.0).contains(&h.at.lon));
            assert!(h.queen_energy > 0. && h.queen_minerals > 0.);
        }
        for dep in &d.deposits {
            assert!((-90.0..=90.0).contains(&dep.at.lat));
            assert!((-180.0..=180.0).contains(&dep.at.lon));
        }
    }

    #[test]
    fn mixed_deposits_carry_both() {
        let d = plan(&StartConfig {
            deposits: 30,
            ..Default::default()
        });
        assert!(!d.deposits.is_empty());
        for dep in &d.deposits {
            assert!(dep.energy > 0.0);
            assert!(dep.minerals > 0.0);
        }
    }

    #[test]
    fn zero_hives_and_zero_deposits_are_legal() {
        let d = plan(&StartConfig {
            hives: 0,
            deposits: 0,
            ..Default::default()
        });
        assert!(d.hives.is_empty());
        assert!(d.deposits.is_empty());
    }
}
