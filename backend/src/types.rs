use serde::{Deserialize, Serialize};
pub type V = rapier3d_f64::math::Vector;
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Terrain {
    pub lat: f64,
    pub lon: f64,
    pub step: f64,
    pub size: usize,
    pub heights: Vec<f64>,
    #[serde(skip)]
    pub shape_cache: std::sync::OnceLock<rapier3d_f64::parry::shape::SharedShape>,
}
impl Terrain {
    pub fn ground(&self, p: V) -> f64 {
        let half = (self.size - 1) as f64 / 2.;
        let x = (p.x / self.step + half).clamp(0., self.size as f64 - 1.001);
        let y = (p.y / self.step + half).clamp(0., self.size as f64 - 1.001);
        let (i, j) = (x.floor() as usize, y.floor() as usize);
        let (a, b) = (x - i as f64, y - j as f64);
        let h = |i, j| self.heights[j * self.size + i];
        (h(i, j) * (1. - a) + h(i + 1, j) * a) * (1. - b)
            + (h(i, j + 1) * (1. - a) + h(i + 1, j + 1) * a) * b
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Input {
    pub thrust_up: bool,
    pub thrust_down: bool,
    pub forward: bool,
    pub back: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub yaw_left: bool,
    pub yaw_right: bool,
    pub boost: bool,
    pub alt_hold: bool,
    pub fire: bool,
}
