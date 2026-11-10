export type Vec = { x: number; y: number; z: number };
export type Terrain = {
  lat: number;
  lon: number;
  step: number;
  size: number;
  heights: number[];
};
export const coords = (t: Terrain, p: Vec): [number, number] => [
  t.lon + p.x / (111320 * Math.cos((t.lat * Math.PI) / 180)),
  t.lat + p.y / 111320,
];
export const point = (t: Terrain, lat: number, lon: number): Vec => ({
  x: (((lon - t.lon + 540) % 360) - 180) * 111320 * Math.cos((t.lat * Math.PI) / 180),
  y: (lat - t.lat) * 111320,
  z: 0,
});
export function ground(t: Terrain, p: Vec): number {
  const half = (t.size - 1) / 2;
  const x = Math.max(0, Math.min(t.size - 1.001, p.x / t.step + half));
  const y = Math.max(0, Math.min(t.size - 1.001, p.y / t.step + half));
  const i = Math.floor(x),
    j = Math.floor(y),
    a = x - i,
    b = y - j;
  const h = (i: number, j: number) => t.heights[j * t.size + i];
  return (
    (h(i, j) * (1 - a) + h(i + 1, j) * a) * (1 - b) +
    (h(i, j + 1) * (1 - a) + h(i + 1, j + 1) * a) * b
  );
}
