import * as THREE from "three";
export const QUEEN_HULL_HEIGHT = 40;
const sphere = new THREE.SphereGeometry(1, 14, 10);
const limb = new THREE.CylinderGeometry(0.3, 1, 1, 8);
const chitin = new THREE.MeshStandardMaterial({
  color: 0x54234f,
  roughness: 0.65,
});
const dark = new THREE.MeshStandardMaterial({
  color: 0x30132e,
  roughness: 0.8,
});
const eyes = new THREE.MeshBasicMaterial({ color: 0xff69ec });
const eggs = new THREE.MeshBasicMaterial({ color: 0x58f3c0 });
/** Model fits the server's 100 m weapon hull; its pivot is the hull centre. */
export class QueenLayer extends THREE.Group {
  constructor() {
    super();
    const body = new THREE.Group();
    this.add(body);
    const orb = (
      x: number,
      y: number,
      z: number,
      rx: number,
      ry: number,
      rz: number,
      material: THREE.Material,
    ) => {
      const mesh = new THREE.Mesh(sphere, material);
      mesh.position.set(x, y - 26, z + 7);
      mesh.scale.set(rx, ry, rz);
      body.add(mesh);
    };
    orb(0, 16, -34, 22, 26, 54, chitin);
    orb(0, 24, 30, 16, 17, 18, chitin);
    orb(0, 32, 62, 9, 9, 12, chitin);
    for (const side of [-1, 1])
      for (const [x, y, z, fx, fz, r] of [
        [0, 24, 52, 24, 60, 3],
        [12, 22, 26, 40, 18, 3.2],
        [12, 20, -4, 36, -22, 3],
      ]) {
        const a = new THREE.Vector3(side * x, y - 26, z + 7),
          b = new THREE.Vector3(side * fx, -26, fz + 7),
          d = b.clone().sub(a);
        const leg = new THREE.Mesh(limb, dark);
        leg.scale.set(r, d.length(), r);
        leg.position.copy(a).add(b).multiplyScalar(0.5);
        leg.quaternion.setFromUnitVectors(
          THREE.Object3D.DEFAULT_UP,
          d.normalize(),
        );
        body.add(leg);
      }
    for (const side of [-1, 1]) {
      orb(side * 4, 42, 63, 4.5, 4.5, 4.5, eyes);
      orb(side * 6.5, 40, 68, 3.2, 3.2, 3.2, eyes);
      orb(side * 2.5, 41.5, 56, 2.4, 2.4, 2.4, eyes);
    }
    for (const [x, y, z, r] of [
      [0, 47, -40, 8],
      [-12, 45, -46, 6],
      [12, 45, -34, 6],
      [0, 51, -52, 5],
      [-8, 46, -26, 5],
      [8, 46, -20, 5],
    ])
      orb(x, y, z, r, r, r, eggs);
    body.updateMatrixWorld(true);
    let radius = 0;
    body.traverse((o) => {
      if (o instanceof THREE.Mesh) {
        o.geometry.computeBoundingSphere();
        const s = o.geometry
          .boundingSphere!.clone()
          .applyMatrix4(o.matrixWorld);
        radius = Math.max(radius, s.center.length() + s.radius);
      }
    });
    const bottom = new THREE.Box3().setFromObject(body).min.y;
    body.scale.setScalar(Math.min(100 / radius, QUEEN_HULL_HEIGHT / -bottom));
    body.rotation.y = Math.PI; // Model's +Z head matches flight's -Z forward.
  }
  animate(_time: number, _frozen: boolean) {}
}
