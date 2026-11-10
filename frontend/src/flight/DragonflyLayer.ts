import * as THREE from "three";
const sphere = new THREE.SphereGeometry(1, 12, 8);
const wing = new THREE.SphereGeometry(1, 12, 6);
const chitin = new THREE.MeshStandardMaterial({
  color: 0xf47740,
  roughness: 0.7,
});
const eyes = new THREE.MeshBasicMaterial({ color: 0x28d9c9 });
const membrane = new THREE.MeshStandardMaterial({
  color: 0x76e8dc,
  transparent: true,
  opacity: 0.65,
  side: THREE.DoubleSide,
  depthWrite: false,
});
/** Creature geometry only; Three.js owns meshes, materials and GPU resources. */
export class DragonflyLayer extends THREE.Group {
  private wings: THREE.Group[] = [];
  constructor() {
    super();
    const body = (
      x: number,
      y: number,
      z: number,
      sx: number,
      sy: number,
      sz: number,
      material: THREE.Material = chitin,
    ) => {
      const m = new THREE.Mesh(sphere, material);
      m.position.set(x, y, z);
      m.scale.set(sx, sy, sz);
      this.add(m);
    };
    body(0, 0, 0, 0.48, 0.42, 1.5);
    body(0, -0.05, 2.8, 0.18, 0.18, 2);
    body(0, 0.12, -1.65, 0.58, 0.46, 0.52);
    for (const side of [-1, 1]) {
      body(side * 0.4, 0.32, -1.9, 0.23, 0.25, 0.26, eyes);
      const pair = new THREE.Group();
      for (const z of [-0.7, 0.9]) {
        const m = new THREE.Mesh(wing, membrane);
        m.position.set(side * 2.7, 0.12, z);
        m.scale.set(2.5, 0.035, 0.65);
        pair.add(m);
      }
      this.add(pair);
      this.wings.push(pair);
    }
  }
  animate(time: number, frozen: boolean) {
    if (!frozen)
      this.wings.forEach(
        (w, i) => (w.rotation.z = (i ? 1 : -1) * Math.sin(time * 0.045) * 0.3),
      );
  }
}
