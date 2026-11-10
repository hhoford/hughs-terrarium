//! Established engine primitives. Game forces and spherical coordinates stay in lab.
use crate::{
    lab::{Pos, Vec3},
    types::Terrain,
};
use rapier3d_f64::{
    parry::{
        query::{Ray, RayCast},
        shape::{Ball, SharedShape},
    },
    prelude::*,
};
use std::cell::RefCell;

thread_local! {
    // Flying actors never bounce or form joints: the game resolves contact as
    // landing/destruction. Reuse Rapier's free-body solver without per-tick worlds.
    static MOTION: RefCell<(PhysicsWorld, RigidBodyHandle)> = RefCell::new({
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::ZERO;
        world.integration_parameters.normalized_max_linear_velocity = f64::MAX;
        let body = world.insert_body(RigidBodyBuilder::dynamic().additional_mass(1.).can_sleep(false).lock_rotations());
        (world, body)
    });
}

pub fn advance(position: Vec3, velocity: Vec3, acceleration: Vec3, dt: f64) -> (Vec3, Vec3) {
    MOTION.with(|motion| {
        let mut motion = motion.borrow_mut();
        let (world, handle) = &mut *motion;
        world.integration_parameters.dt = dt;
        let body = &mut world.bodies[*handle];
        body.set_translation(position, true);
        body.set_linvel(velocity, true);
        body.reset_forces(false);
        body.add_force(acceleration, true); // Unit solver mass; game already computes F/m.
        world.step();
        (
            world.bodies[*handle].translation(),
            world.bodies[*handle].linvel(),
        )
    })
}

pub fn sphere_hit(a: Vec3, b: Vec3, centre: Vec3, radius: f64) -> Option<f64> {
    Ball::new(radius).cast_local_ray(&Ray::new(a - centre, b - a), 1., true)
}

pub fn terrain_hit(patches: &[Terrain], a: Vec3, b: Vec3) -> Option<Pos> {
    terrain_hit_candidates(patches, 0..patches.len(), a, b)
}
fn terrain_hit_candidates(
    patches: &[Terrain],
    candidates: impl IntoIterator<Item = usize>,
    a: Vec3,
    b: Vec3,
) -> Option<Pos> {
    let delta = b - a;
    if delta.length() < 1e-10 {
        return None;
    }
    let skip = (0.001 / delta.length()).min(0.5);
    let mut closest = None::<f64>;
    for index in candidates {
        let terrain = &patches[index];
        let origin = Pos {
            lat: terrain.lat,
            lon: terrain.lon,
            alt: 0.,
        }
        .ecef();
        let ray = Ray::new(a - origin + delta * skip, delta);
        if Ball::new(25_000.).cast_local_ray(&ray, 1., true).is_none() {
            continue;
        }
        let shape = terrain_shape(terrain);
        if let Some(t) = shape.cast_local_ray(&ray, closest.unwrap_or(1.) - skip, true) {
            closest = Some(t + skip);
        }
    }
    closest.map(|t| Pos::from_ecef(a + (b - a) * t))
}

pub fn prepare_terrain(terrain: Terrain) -> Terrain {
    terrain_shape(&terrain);
    terrain
}
fn terrain_shape(terrain: &Terrain) -> &SharedShape {
    let origin = Pos {
        lat: terrain.lat,
        lon: terrain.lon,
        alt: 0.,
    }
    .ecef();
    terrain.shape_cache.get_or_init(|| {
        let mut vertices = Vec::with_capacity(terrain.size * terrain.size);
        let mut triangles = Vec::with_capacity((terrain.size - 1).pow(2) * 2);
        let half = (terrain.size - 1) as f64 / 2.;
        for row in 0..terrain.size {
            for col in 0..terrain.size {
                vertices.push(
                    Pos {
                        lat: terrain.lat + (row as f64 - half) * terrain.step / 111320.,
                        lon: terrain.lon
                            + (col as f64 - half) * terrain.step
                                / (111320. * terrain.lat.to_radians().cos()),
                        alt: terrain.heights[row * terrain.size + col].max(0.),
                    }
                    .ecef()
                        - origin,
                );
                if row + 1 < terrain.size && col + 1 < terrain.size {
                    let i = (row * terrain.size + col) as u32;
                    let n = terrain.size as u32;
                    triangles.extend([[i, i + 1, i + n], [i + 1, i + n + 1, i + n]]);
                }
            }
        }
        SharedShape::trimesh(vertices, triangles).expect("validated DEM mesh")
    })
}

// Broad phase only. Parry still performs every exact swept intersection.
#[derive(Default)]
pub struct TerrainIndex {
    regions: std::collections::HashMap<u64, Vec<usize>>,
    count: usize,
    candidates: RefCell<Vec<usize>>,
}
impl TerrainIndex {
    pub fn rebuild(&mut self, patches: &[Terrain]) {
        self.regions.values_mut().for_each(Vec::clear);
        for (i, t) in patches.iter().enumerate() {
            // A 25 km patch bounding ball is smaller than this deliberately
            // generous four-ring r4 halo, including pentagon neighbourhoods.
            let cell = h3o::CellIndex::try_from(crate::bake::owner(t.lat, t.lon)).unwrap();
            for cell in cell.grid_disk::<Vec<_>>(4) {
                self.regions.entry(u64::from(cell)).or_default().push(i);
            }
        }
        self.regions.retain(|_, indices| !indices.is_empty());
        self.count = patches.len();
    }
    pub fn ensure(&mut self, patches: &[Terrain]) {
        if self.count != patches.len() {
            self.rebuild(patches);
        }
    }
    pub fn elevation(&self, patches: &[Terrain], p: Pos) -> Option<f64> {
        if self.count != patches.len() {
            return crate::lab::elevation(patches, p);
        }
        self.regions
            .get(&crate::bake::owner(p.lat, p.lon))?
            .iter()
            .find_map(|&i| {
                let t = &patches[i];
                let v = crate::lab::local(t, p);
                (v.x.abs() < 7900. && v.y.abs() < 7900.).then(|| t.ground(v).max(0.))
            })
    }
    pub fn body_ground(&self, patches: &[Terrain], u: &crate::lab::Body) -> Option<f64> {
        let mut ground = self.elevation(patches, u.pos)?;
        if u.queen() {
            for (e, n) in [(60., 60.), (-60., 60.), (60., -60.), (-60., -60.)] {
                ground = ground
                    .max(self.elevation(patches, u.pos.offset(e * u.size(), n * u.size(), 0.))?);
            }
        }
        Some(ground)
    }
    pub fn clearance(&self, patches: &[Terrain], a: Pos, b: Pos, radius: f64) -> bool {
        use rapier3d_f64::parry::{
            math::Pose,
            query::{cast_shapes, ShapeCastOptions},
        };
        if (a.ecef() - b.ecef()).length() > 10000. {
            return false;
        }
        let mut candidates = self.candidates.borrow_mut();
        candidates.clear();
        for p in [a, b] {
            if let Some(ids) = self.regions.get(&crate::bake::owner(p.lat, p.lon)) {
                candidates.extend_from_slice(ids);
            }
        }
        if self.count != patches.len() {
            candidates.extend(0..patches.len());
        }
        candidates.sort_unstable();
        candidates.dedup();
        let start = a.ecef();
        let delta = b.ecef() - start;
        for &i in candidates.iter() {
            let t = &patches[i];
            let origin = Pos {
                lat: t.lat,
                lon: t.lon,
                alt: 0.,
            }
            .ecef();
            if Ball::new(25000. + radius)
                .cast_local_ray(&Ray::new(start - origin, delta), 1., true)
                .is_none()
            {
                continue;
            }
            let offset = start - origin;
            let from = Pose::translation(offset.x, offset.y, offset.z);
            let options = ShapeCastOptions {
                max_time_of_impact: 1.,
                ..Default::default()
            };
            if !matches!(
                cast_shapes(
                    &from,
                    delta,
                    &Ball::new(radius),
                    &Pose::IDENTITY,
                    Vec3::ZERO,
                    terrain_shape(t).as_ref(),
                    options
                ),
                Ok(None)
            ) {
                return false;
            }
        }
        true
    }
    pub fn hit(&self, patches: &[Terrain], a: Vec3, b: Vec3) -> Option<Pos> {
        if self.count != patches.len() {
            return terrain_hit(patches, a, b);
        }
        let distance = (b - a).length();
        // Unusual long queries retain the existing broad phase; ordinary tick
        // sweeps sample every <=8 km and reuse their candidate-ID buffer.
        if distance > 100_000. {
            return terrain_hit(patches, a, b);
        }
        let mut candidates = self.candidates.borrow_mut();
        candidates.clear();
        let steps = (distance / 8000.).ceil().max(1.) as usize;
        for i in 0..=steps {
            let p = Pos::from_ecef(a + (b - a) * (i as f64 / steps as f64));
            if let Some(indices) = self.regions.get(&crate::bake::owner(p.lat, p.lon)) {
                candidates.extend_from_slice(indices);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        terrain_hit_candidates(patches, candidates.iter().copied(), a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_ground_and_sweeps_match_full_scans_at_seams_and_pentagons() {
        let mut sites = vec![(0., 179.999), (79., -179.999), (-79., 0.), (0., 0.)];
        sites.extend(h3o::Resolution::Four.pentagons().map(|c| {
            let p = h3o::LatLng::from(c);
            (p.lat(), p.lng())
        }));
        let patches: Vec<_> = sites
            .iter()
            .map(|&(lat, lon)| Terrain {
                lat,
                lon,
                step: 250.,
                size: 65,
                heights: vec![123.; 65 * 65],
                ..Terrain::default()
            })
            .collect();
        let mut index = TerrainIndex::default();
        index.rebuild(&patches);
        for &(lat, lon) in &sites {
            for east in [-7800., -1000., 0., 1000., 7800.] {
                for north in [-7800., 0., 7800.] {
                    let p = Pos {
                        lat,
                        lon,
                        alt: 1000.,
                    }
                    .offset(east, north, 0.);
                    assert_eq!(
                        index.elevation(&patches, p),
                        crate::lab::elevation(&patches, p)
                    );
                    let b = Pos { alt: 0., ..p }.ecef();
                    let a = p.ecef();
                    let expected = terrain_hit(&patches, a, b);
                    let actual = index.hit(&patches, a, b);
                    assert_eq!(actual.is_some(), expected.is_some());
                    if let (Some(a), Some(b)) = (actual, expected) {
                        assert!((a.ecef() - b.ecef()).length() < 1e-8);
                    }
                }
            }
        }
    }
    #[test]
    fn independent_solver_calls_do_not_share_force_or_velocity() {
        let (p, v) = advance(Vec3::ZERO, Vec3::ZERO, Vec3::Y * -10., 0.1);
        assert!((v.y + 1.).abs() < 1e-10 && p.y < 0.);
        let origin = Vec3::new(6_371_000., 100., 0.);
        let (p, v) = advance(origin, Vec3::ZERO, Vec3::ZERO, 0.1);
        assert_eq!(p, origin);
        assert_eq!(v, Vec3::ZERO);
    }
    #[test]
    fn planetary_missile_speeds_are_not_capped_by_engine_defaults() {
        let origin = Vec3::new(6_371_000., 0., 0.);
        let (p, v) = advance(origin, Vec3::Y * 100_000., Vec3::ZERO, 0.025);
        assert!((p.y - 2500.).abs() < 1e-6 && (v.y - 100_000.).abs() < 1e-6);
    }
    #[test]
    fn terrain_surface_does_not_trap_a_lifting_body() {
        let terrain = Terrain {
            lat: 0.,
            lon: 0.,
            step: 250.,
            size: 3,
            heights: vec![0.; 9],
            ..Terrain::default()
        };
        let p = Pos {
            lat: 0.,
            lon: 0.,
            alt: 0.,
        };
        assert!(terrain_hit(&[terrain], p.ecef(), Pos { alt: 0.001, ..p }.ecef()).is_none());
    }
}
