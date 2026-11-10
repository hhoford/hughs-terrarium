import type { Terrain } from "./api";
export type Pos = { lat: number; lon: number; alt: number };
export type Vector = { x: number; y: number; z: number };
export type Body = {
  dry_mass:number; founder:boolean; construction_remaining:number; growth_target:number;
  id: string;
  sample_time: number;
  hive: number;
  lineage?: number[];
  kind: "queen" | "dragonfly" | "god";
  pos: Pos;
  v: Vector;
  thrust: Vector;
  heading: number;
  pitch: number;
  roll: number;
  energy: number;
  capacity: number;
  power: number;
  alive: boolean;
  grounded: boolean;
  mode: string;
  agl: number;
  terrain_mean: number | null;
  radar_altitude: number | null;
  shelter_altitude: number | null;
  radar_visible: boolean;
  nuclear_sheltered: boolean;
  brood_remaining:number;
  cooldown: number;
  minerals: number;
  cargo_energy: number;
  cargo_minerals: number;
  scan_until: number;
  charge: { target: Pos; remaining: number; precision:boolean } | null;
};
export type Missile = {
  id: number;
  hive: number;
  pos: Pos;
  start: Pos;
  target: Pos;
  age: number;
  duration: number;
  path: Pos[];
};
export type Resource = {
  id: number;
  pos: Pos;
  energy: number;
  minerals: number;
  discovered: number[];
};
export type Mark={id:number;hive:number;source:string;pos:Pos;expires:number;accuracy_m:number};
export type RadarTag={id:number;hive:number|null;source:string;pos:Pos;label:string};
export type Pheromone={hive:number;cell:number;pos:Pos;trail:number;danger:number};
export type Lab = {
  tags:RadarTag[];
  pheromones:Pheromone[];
  ai_hives:number[];
  all_hives_active:boolean;
  marks:Mark[];
  balance:{blast_radius:number;precision_radius:number;radar_offset:number;close_mark_altitude:number;strike_energy:number;birth_energy:number;birth_minerals:number};
  terrain_envelope: {
    point_count: number;
    spacing_m: number;
    sigma_m: number;
    cutoff_m: number;
    version: string;
  };
  resources: Resource[];
  epoch: number;
  time: number;
  units: Body[];
  bullets: { id: number; hive: number; pos: Pos; age: number }[];
  missiles: Missile[];
  bursts: { id: number; pos: Pos; age: number; radius: number; kind: string;precision:boolean }[];
  loading: boolean;
  revision: number;
  tutorial: string | null;
};
export class LabError extends Error {
  constructor(public status: number, message: string) { super(message); }
}
async function request<T>(path: string, body?: unknown): Promise<T> {
  const r = await fetch(
    path,
    body
      ? {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        }
      : undefined,
  );
  if (!r.ok) throw new LabError(r.status, await r.text());
  return r.json() as Promise<T>;
}
export type StartConfig = {seed:number;hives:number;drones_per_hive:number;queen_energy:number;queen_minerals:number;deposits:number;energy_per_deposit:number;minerals_per_deposit:number};
export const resetLab = (start?: Partial<StartConfig>) =>
  request<Lab>("/api/lab/reset", {
    seed: start?.seed ?? crypto.getRandomValues(new Uint32Array(1))[0],
    start,
  });
export const labState = (hive?: number, unit?:string) =>
  request<Lab>(`/api/lab/state${unit ? `?unit=${encodeURIComponent(unit)}` : hive === undefined ? "" : `?hive=${hive}`}`);
export const labCommand = (
  epoch: number,
  action: string,
  unit?: string,
  target?: Pos,
  hive?: number,
  amount?: number,
  mark?: number,
) =>
  request<Lab>("/api/lab/command", {
    epoch,
    id: crypto.randomUUID(),
    action,
    unit,
    target,
    hive,
    amount,
    mark,
  });
export const labInput = (epoch: number, unit: string, input: unknown) =>
  request<Body>("/api/lab/input", { epoch, unit, input });
export const labTerrain = (unit: string) =>
  request<Terrain>(`/api/lab/terrain?unit=${encodeURIComponent(unit)}`);
export type Hollow = { lat: number; lon: number; depth: number; shelter: number };
export type Hollows = {
  lat: number;
  lon: number;
  radius_km: number;
  cells: number;
  threshold: number;
  points: Hollow[];
};
export const labHollows = (lat: number, lon: number, radiusKm = 20, cells = 65) =>
  request<Hollows>(
    `/api/lab/hollows?lat=${lat}&lon=${lon}&radius_km=${radiusKm}&cells=${cells}`,
  );
export const colours = [
  "#7de4ce",
  "#eeaa7a",
  "#b6a0fa",
  "#e7ce7a",
  "#86b7f2",
  "#e991c5",
];
export const colour = (hive: number) => hive<0?"#f8e5b2":colours[hive % colours.length];
export function offset(p: Pos, e: number, n: number, u = 0): Pos {
  return {
    lat: p.lat + n / 111320,
    lon:
      ((p.lon + e / (111320 * Math.cos((p.lat * Math.PI) / 180)) + 540) % 360) -
      180,
    alt: p.alt + u,
  };
}
export function radarPoint(origin: Pos, p: Pos): [number, number] {
  const r = Math.PI / 180,
    a = origin.lat * r,
    b = p.lat * r,
    d = (p.lon - origin.lon) * r;
  const cos = Math.max(
    -1,
    Math.min(
      1,
      Math.sin(a) * Math.sin(b) + Math.cos(a) * Math.cos(b) * Math.cos(d),
    ),
  );
  const angle = Math.acos(cos);
  const bearing = Math.atan2(
    Math.sin(d) * Math.cos(b),
    Math.cos(a) * Math.sin(b) - Math.sin(a) * Math.cos(b) * Math.cos(d),
  );
  const radius = (angle / Math.PI) * 100;
  return [120 + Math.sin(bearing) * radius, 120 - Math.cos(bearing) * radius];
}

export function distance(a: Pos, b: Pos): number {
  const r = Math.PI / 180;
  const dot =
    Math.sin(a.lat * r) * Math.sin(b.lat * r) +
    Math.cos(a.lat * r) * Math.cos(b.lat * r) * Math.cos((a.lon - b.lon) * r);
  const ra = 6371000 + a.alt,
    rb = 6371000 + b.alt;
  return Math.sqrt(
    Math.max(
      0,
      ra * ra + rb * rb - 2 * ra * rb * Math.max(-1, Math.min(1, dot)),
    ),
  );
}

export const feedLab=(epoch:number,count:number,energy:number,minerals:number)=>request<Lab>("/api/lab/feed",{epoch,count,energy,minerals,seed:crypto.getRandomValues(new Uint32Array(1))[0]});

export const radarTag=(epoch:number,unit:string,label:string,pos:Pos,id?:number,remove=false)=>request<Lab>('/api/lab/tags',{epoch,unit,label,pos,id,remove});
