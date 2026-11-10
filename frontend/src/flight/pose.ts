// Presentation state and MapLibre camera only. Physics is authoritative in Rust/Rapier.
export const DEG2RAD = Math.PI / 180;
export const RAD2DEG = 180 / Math.PI;

export interface FlightInput {
  thrustUp: boolean; // W — wingbeat thrust (along body-up axis)
  thrustDown: boolean; // S — reverse thrust (along body-down axis)
  forward: boolean; // ArrowUp — pitch nose DOWN (tilt forward)
  back: boolean; // ArrowDown — pitch nose UP (tilt back)
  strafeLeft: boolean; // ArrowLeft — roll left
  strafeRight: boolean; // ArrowRight — roll right
  yawLeft: boolean; // A
  yawRight: boolean; // D
  boost: boolean; // Shift — afterburner
}

export interface FlightState {
  lng: number;
  lat: number;
  alt: number;
  heading: number;
  pitch: number;
  roll: number;
  vEast: number;
  vNorth: number;
  vUp: number;
}
export const FLIGHT_ZOOM = 16.2;
const EYE_HEIGHT = 3,
  DEPRESSION_BASE = 7,
  DEPRESSION_MAX = 60,
  DEPRESSION_MIN = 0,
  PITCH_TO_CAMERA = 0.8,
  ROLL_SIGN = 1;
export const clamp = (v: number, lo: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));
export class FlightPose {
  s: FlightState;
  constructor(lng: number, lat: number, alt: number, headingDeg: number) {
    this.s = {
      lng,
      lat,
      alt,
      heading: headingDeg * DEG2RAD,
      pitch: 0,
      roll: 0,
      vEast: 0,
      vNorth: 0,
      vUp: 0,
    };
  }
}
export type FlightCamera = {center:[number,number];elevation:number;zoom:number;pitch:number;bearing:number;roll:number};
export const createCamera = ():FlightCamera => ({center:[0,0],elevation:0,zoom:0,pitch:0,bearing:0,roll:0});
export function buildCamera(
  map: { transform: { cameraToCenterDistance: number } },
  heli: FlightPose,
  out: FlightCamera = createCamera(),
) {
  const s = heli.s;
  const zoom = FLIGHT_ZOOM;

  const depressionDeg = clamp(
    DEPRESSION_BASE - s.pitch * RAD2DEG * PITCH_TO_CAMERA,
    DEPRESSION_MIN,
    DEPRESSION_MAX,
  );
  const dep = depressionDeg * DEG2RAD;
  const fE = Math.sin(s.heading) * Math.cos(dep);
  const fN = Math.cos(s.heading) * Math.cos(dep);
  const fU = -Math.sin(dep);

  // Camera-to-center distance in metres = focal length (px) * ground scale.
  const latRad = s.lat * DEG2RAD;
  const mpp = (40075016 * Math.cos(latRad)) / (512 * Math.pow(2, zoom));
  const d = map.transform.cameraToCenterDistance * mpp;

  const eyeAlt = s.alt + EYE_HEIGHT;

  const centerLat = s.lat + (d * fN) / 111320;
  const centerLng =
    s.lng + (d * fE) / (111320 * Math.max(0.02, Math.cos(latRad)));
  const centerAlt = eyeAlt + d * fU;

  out.center[0]=centerLng;out.center[1]=centerLat;out.elevation=centerAlt;
  out.zoom=zoom;out.pitch=90-depressionDeg;out.bearing=s.heading*RAD2DEG;out.roll=s.roll*RAD2DEG*ROLL_SIGN;
  return out;
}
