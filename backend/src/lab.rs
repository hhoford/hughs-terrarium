//! Globe terrarium. Optional hive control is off by default; otherwise unpiloted
//! bodies use a powered position controller. SI metres, seconds, kilograms and joules throughout.
use crate::{
    envelope::{self, Cell, Envelope},
    types::{Input, Terrain, V},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
const R: f64 = 6_371_000.;
const MU: f64 = 3.986004418e14;
const DT: f64 = 0.025;
const DRONE_E: f64 = 75e9;
const DRONE_MASS: f64 = 100.; // kg dry; up to 2,000 kg of mineral payload
const DRONE_POWER: f64 = 2e6; // W actuator budget, also constrained by reserve
const QUEEN_E: f64 = 3e12;
const START_QUEEN_E: f64 = 1e12;
const START_QUEEN_MINERALS: f64 = 500.;
const FOUNDER_E: f64 = 200e9;
const FOUNDER_MINERALS: f64 = 1000.;
const QUEEN_MINERALS: f64 = 1e6;
const DRONE_CARGO_ENERGY: f64 = 250e9;
const DRONE_CARGO_MINERALS: f64 = 2000.;
const DRONE_FORCE: f64 = 32_000.;
const STRIKE_E: f64 = QUEEN_E * 0.6; // Preserve the expensive-shot ratio as storage grows.
const BLAST_R: f64 = 600.;
const PRECISION_R: f64 = 50.;
const CLOSE_MARK_ALTITUDE: f64 = 250.;
const MARK_TTL: f64 = 60.;
const BIRTH_E: f64 = 100e9;
const BIRTH_M: f64 = 100.;
const BROOD_TIME: f64 = 30.;
const CHARGE: f64 = 30.;
const TAKEOFF_PREP: f64 = 389.;
const TAKEOFF_CLEARANCE: f64 = 20.;
const QUEEN_LATERAL: f64 = 4.;
const QUEEN_DRAG_SCALE: f64 = 0.010045 / 0.1525;
const COOLDOWN: f64 = 180.;
const QUEEN_HULL_HEIGHT: f64 = 40.;
const QUEEN_HULL_RADIUS: f64 = 100.;
pub type Store = Arc<Mutex<Lab>>;
mod ai;
mod api;
mod combat;
mod commands;
mod economy;
mod flight;
mod model;
pub mod persistence;
#[cfg(test)]
mod tests;
mod world;

use crate::physics::{sphere_hit as segment_sphere, terrain_hit};
pub use api::*;
use combat::*;
use flight::*;
#[cfg(test)]
use model::body_ground;
pub(crate) use model::{elevation, local};
pub use model::{
    new, Balance, Body, Bullet, Burst, Charging, Lab, Mark, Missile, Pos, Resource, Vec3, View,
};
use std::ops::{Add, Mul, Sub};
