import type { FlightInput } from './pose';
import type { CameraHeld } from './camera';

// Held flight axes. H toggles the optional, initially enabled hover assist.
//
//   W / S              wingbeat thrust / reverse thrust
//   Up / Down          pitch nose down / up (tilt forward / back, ±80°)
//   Left / Right       roll left / right
//   A / D              yaw left / right
//   Shift (hold)       afterburner
//   H                  toggle altitude-hold assist
//
// Camera (separate from flight axes — none of these touch the physics):
//   C                  toggle first / third person
//   L                  toggle world-direction lock (freeze the camera bearing)
//   Q / E              orbit the camera around the craft (azimuth)
//   R / F              raise / lower the camera (elevation angle)
//   Z / X              zoom in / out (chase distance)
//   V                  reset the camera to the default chase pose
//
// Economy (hive loop — actions only fire when near a target):
//   G                  collect from the nearest deposit (fills cargo)
//   B                  deliver cargo to the hive
//   N                  refuel flight reserve from the hive energy stock
//   M                  mark this dragonfly's location for its queen
export class InputTracker {
  private keys = new Set<string>();
  private empty = new Set<string>();
  private fireTap = false;
  private altHoldToggle = false;
  private cameraToggle = false;
  private lockToggle = false;
  private resetToggle = false;
  private collectToggle = false;
  private deliverToggle = false;
  private refuelToggle = false;
  private reproduceToggle = false;

  private onKeyDown = (e: KeyboardEvent) => {
    if(typeof HTMLElement!=="undefined" && e.target instanceof HTMLElement && e.target.matches("input,textarea,[contenteditable=true]"))return;
    if (
      e.code === 'ArrowUp' ||
      e.code === 'ArrowDown' ||
      e.code === 'ArrowLeft' ||
      e.code === 'ArrowRight' ||
      e.code === 'Space'
    ) {
      e.preventDefault();
    }
    if (e.code === 'Space' && !e.repeat) this.fireTap = true;
    if (e.code === 'KeyH' && !e.repeat) this.altHoldToggle = true;
    if (e.code === 'KeyC' && !e.repeat) this.cameraToggle = true;
    if (e.code === 'KeyL' && !e.repeat) this.lockToggle = true;
    if (e.code === 'KeyV' && !e.repeat) this.resetToggle = true;
    if (e.code === 'KeyG' && !e.repeat) this.collectToggle = true;
    if (e.code === 'KeyB' && !e.repeat) this.deliverToggle = true;
    if (e.code === 'KeyN' && !e.repeat) this.refuelToggle = true;
    if (e.code === 'KeyM' && !e.repeat) this.reproduceToggle = true;
    this.keys.add(e.code);
  };
  private onKeyUp = (e: KeyboardEvent) => {
    this.keys.delete(e.code);
  };
  private onBlur = () => { this.keys.clear(); this.fireTap = false; };

  attach() {
    window.addEventListener('keydown', this.onKeyDown);
    window.addEventListener('keyup', this.onKeyUp);
    window.addEventListener('blur', this.onBlur);
  }

  detach() {
    window.removeEventListener('keydown', this.onKeyDown);
    window.removeEventListener('keyup', this.onKeyUp);
    window.removeEventListener('blur', this.onBlur);
    this.keys.clear();
    this.fireTap = false;
    this.altHoldToggle = false;
    this.cameraToggle = false;
    this.lockToggle = false;
    this.resetToggle = false;
    this.collectToggle = false;
    this.deliverToggle = false;
    this.refuelToggle = false;
    this.reproduceToggle = false;
  }

  /** True once per physical H press; clears the flag when read. */
  consumeAltHoldToggle(): boolean {
    const v = this.altHoldToggle;
    this.altHoldToggle = false;
    return v;
  }

  /** True once per physical C press (toggle first/third person). */
  consumeCameraToggle(): boolean {
    const v = this.cameraToggle;
    this.cameraToggle = false;
    return v;
  }

  /** True once per physical L press (toggle world-direction lock). */
  consumeLockToggle(): boolean {
    const v = this.lockToggle;
    this.lockToggle = false;
    return v;
  }

  /** True once per physical V press (reset camera). */
  consumeResetToggle(): boolean {
    const v = this.resetToggle;
    this.resetToggle = false;
    return v;
  }

  consumeCollect(): boolean { const v = this.collectToggle; this.collectToggle = false; return v; }
  consumeDeliver(): boolean { const v = this.deliverToggle; this.deliverToggle = false; return v; }
  consumeRefuel(): boolean { const v = this.refuelToggle; this.refuelToggle = false; return v; }
  consumeReproduce(): boolean { const v = this.reproduceToggle; this.reproduceToggle = false; return v; }

  /** Held camera keys (orbit / elevation / distance). */
  cameraHeld(out:CameraHeld={orbitLeft:false,orbitRight:false,elevationUp:false,elevationDown:false,zoomIn:false,zoomOut:false}): CameraHeld {
    const k = typeof document!=="undefined" && document.activeElement?.matches("input,textarea,[contenteditable=true]")?this.empty:this.keys;
    out.orbitLeft=k.has('KeyQ');out.orbitRight=k.has('KeyE');
    out.elevationUp=k.has('KeyR');out.elevationDown=k.has('KeyF');out.zoomIn=k.has('KeyZ');out.zoomOut=k.has('KeyX');return out;
  }

  isFiring() { const fire=this.fireTap||this.keys.has('Space');this.fireTap=false;return fire; }

  snapshot(): FlightInput {
    const k = typeof document!=="undefined" && document.activeElement?.matches("input,textarea,[contenteditable=true]")?this.empty:this.keys;
    return {
      thrustUp: k.has('KeyW'),
      thrustDown: k.has('KeyS'),
      forward: k.has('ArrowUp'),
      back: k.has('ArrowDown'),
      strafeLeft: k.has('ArrowLeft'),
      strafeRight: k.has('ArrowRight'),
      yawLeft: k.has('KeyA'),
      yawRight: k.has('KeyD'),
      boost: k.has('ShiftLeft') || k.has('ShiftRight'),
    };
  }
}
