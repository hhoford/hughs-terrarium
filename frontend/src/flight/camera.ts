// Camera rig for the dragonfly. State is kept SEPARATE from the flight physics:
// changing the camera never changes thrust, velocity, pitch, roll or energy. It
// only decides where the camera sits and which way it looks.
//
// Modes
//   follow      chase view behind the craft, with an adjustable orbit offset
//               (azimuth / elevation / distance). This is the default.
//   lock        world-direction lock: the camera keeps its bearing fixed
//               relative to the world while the craft turns beneath it.
//   firstperson the pilot's-eye view (built elsewhere; this rig only reports it).
//
// "Distance" is MapLibre zoom (higher = closer). "Elevation" is the angle ABOVE
// the horizon the camera looks down from, so MapLibre pitch-from-vertical is
// 90 - elevation.
import type { FlightState } from './pose';
import { DEG2RAD, RAD2DEG, clamp, createCamera, type FlightCamera } from './pose';

export type CameraMode = 'follow' | 'lock' | 'firstperson';

export const CHASE_ZOOM_DEFAULT = 19.5;
const ELEVATION_DEFAULT_DEG = 34; // ≈ 56° pitch from vertical
const AZIMUTH_DEFAULT_DEG = 0; // directly behind the craft

const ELEVATION_MIN_DEG = 6;
const ELEVATION_MAX_DEG = 80;
const ZOOM_MIN = 14;
const ZOOM_MAX = 22;

const AZIMUTH_RATE = 72; // deg/s — orbit around the craft
const ELEVATION_RATE = 36; // deg/s
const ZOOM_RATE = 2.4; // zoom units/s

// Camera bearing never slews faster than this. The shortest-angular-path wrap
// below is what stops a 360° spin across north; this cap is what keeps a large
// re-aim (un-locking, a big orbit jump) a fast sweep instead of a snap.
const YAW_SLEW_MAX = 480; // deg/s

// Terrain collision: the camera keeps this much clear air under itself. When a
// mountain rises behind it, the camera pulls IN toward the craft (zoom up) so it
// stays out of the mesh, then eases back out once clear. Pull faster than it
// eases so the camera never visibly clips into terrain.
const CAMERA_CLEARANCE = 8; // visual metres
const COLLISION_PULL_RATE = 5.0; // zoom/s toward the craft while buried
const COLLISION_EASE_RATE = 1.5; // zoom/s returning to the requested distance

export interface CameraHeld {
  orbitLeft: boolean; // Q
  orbitRight: boolean; // E
  elevationUp: boolean; // R
  elevationDown: boolean; // F
  zoomIn: boolean; // Z
  zoomOut: boolean; // X
}

const wrapDeg = (a: number) => ((a % 360) + 360) % 360;
const shortest = (from: number, to: number) => ((to - from + 540) % 360) - 180;

export class CameraRig {
  mode: CameraMode = 'follow';

  // requested pose (follow / lock)
  zoom = CHASE_ZOOM_DEFAULT;
  elevationDeg = ELEVATION_DEFAULT_DEG;
  azimuthDeg = AZIMUTH_DEFAULT_DEG;

  // smoothed camera bearing (deg) — the value actually used
  yawDeg = 0;

  private lockBearingDeg = 0;
  private effectiveZoom = CHASE_ZOOM_DEFAULT;
  private seeded = false;

  constructor(headingDeg = 0) {
    this.reset(headingDeg);
  }

  reset(headingDeg: number) {
    const wasFirstPerson = this.mode === 'firstperson';
    this.mode = wasFirstPerson ? 'firstperson' : 'follow';
    this.zoom = CHASE_ZOOM_DEFAULT;
    this.elevationDeg = ELEVATION_DEFAULT_DEG;
    this.azimuthDeg = AZIMUTH_DEFAULT_DEG;
    this.lockBearingDeg = wrapDeg(headingDeg);
    this.yawDeg = wrapDeg(headingDeg);
    this.effectiveZoom = CHASE_ZOOM_DEFAULT;
    this.seeded = true;
  }

  toggleFirstPerson() {
    this.mode = this.mode === 'firstperson' ? 'follow' : 'firstperson';
  }

  toggleLock() {
    if (this.mode === 'firstperson') return;
    if (this.mode === 'lock') {
      this.mode = 'follow';
    } else {
      this.mode = 'lock';
      this.lockBearingDeg = this.yawDeg;
    }
  }

  get isFirstPerson() {
    return this.mode === 'firstperson';
  }

  /**
   * Advance the rig and build the chase/lock camera. Call only when the mode is
   * not first-person. `terrainAt` samples the visual DEM elevation and
   * `camToCenterDist` is `map.transform.cameraToCenterDistance`.
   */
  update(
    dt: number,
    s: FlightState,
    held: CameraHeld,
    terrainAt: (lng: number, lat: number) => number | null,
    camToCenterDist: number,
    out: FlightCamera = createCamera(),
  ): FlightCamera {
    if (!this.seeded) this.reset(s.heading * RAD2DEG);

    // --- orbit / elevation / distance --------------------------------------
    const az = (held.orbitRight ? 1 : 0) - (held.orbitLeft ? 1 : 0);
    const el = (held.elevationUp ? 1 : 0) - (held.elevationDown ? 1 : 0);
    const zm = (held.zoomIn ? 1 : 0) - (held.zoomOut ? 1 : 0);

    if (this.mode === 'lock') {
      this.lockBearingDeg = wrapDeg(this.lockBearingDeg + az * AZIMUTH_RATE * dt);
    } else {
      this.azimuthDeg = wrapDeg(this.azimuthDeg + az * AZIMUTH_RATE * dt);
    }
    this.elevationDeg = clamp(
      this.elevationDeg + el * ELEVATION_RATE * dt,
      ELEVATION_MIN_DEG,
      ELEVATION_MAX_DEG,
    );
    this.zoom = clamp(this.zoom + zm * ZOOM_RATE * dt, ZOOM_MIN, ZOOM_MAX);

    // --- bearing: follow the craft's heading + orbit offset, smoothed along
    // the shortest angular path so it never spins through 360° at north -------
    const targetBearing =
      this.mode === 'lock'
        ? this.lockBearingDeg
        : wrapDeg(s.heading * RAD2DEG + this.azimuthDeg);
    const delta = shortest(this.yawDeg, targetBearing);
    const maxSlew = YAW_SLEW_MAX * dt;
    this.yawDeg = wrapDeg(this.yawDeg + clamp(delta, -maxSlew, maxSlew));

    // --- terrain collision --------------------------------------------------
    const latRad = s.lat * DEG2RAD;
    const elevRad = this.elevationDeg * DEG2RAD;
    const baseMpp = (40075016 * Math.cos(latRad)) / 512;
    const distance=camToCenterDist*baseMpp*Math.pow(2,-this.effectiveZoom);
    const horiz=distance*Math.cos(elevRad),bearing=this.yawDeg*DEG2RAD;
    const lng=s.lng-Math.sin(bearing)*horiz/(111320*Math.max(0.02,Math.cos(latRad)));
    const lat=s.lat-Math.cos(bearing)*horiz/111320;
    const alt=s.alt+distance*Math.sin(elevRad);
    const t=terrainAt(lng,lat);
    const clear = t == null || !Number.isFinite(t) || alt >= t + CAMERA_CLEARANCE;
    if (!clear) {
      this.effectiveZoom = clamp(
        this.effectiveZoom + COLLISION_PULL_RATE * dt,
        ZOOM_MIN,
        ZOOM_MAX,
      );
    } else if (this.effectiveZoom > this.zoom) {
      this.effectiveZoom = Math.max(
        this.zoom,
        this.effectiveZoom - COLLISION_EASE_RATE * dt,
      );
    } else {
      this.effectiveZoom = this.zoom;
    }

    out.center[0]=s.lng;out.center[1]=s.lat;out.elevation=s.alt;
    out.zoom=this.effectiveZoom;out.pitch=90-this.elevationDeg;out.bearing=this.yawDeg;out.roll=0;
    return out;
  }
}
