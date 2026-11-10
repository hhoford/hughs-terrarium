import {GodNavigation} from "./flight/god";
import type { Map as GLMap } from "maplibre-gl";
import { CameraRig } from "./flight/camera";
import { QUEEN_HULL_HEIGHT } from "./flight/QueenLayer";
import { InputTracker } from "./flight/input";
import { Motion, createPose, type Pose } from "./flight/motion";
import { FlightPose, buildCamera, createCamera, RAD2DEG } from "./flight/pose";
import { labInput, labCommand, labTerrain, LabError, type Body, type Lab } from "./lab";
import { ground, point, type Terrain } from "./api";
// The server owns all motion. This class interpolates/extrapolates its samples
// for display and reuses the accepted independent camera rig.
export class LabFlight {
  private input = new InputTracker();
  private godNavigation = new GodNavigation();
  private worldTime=0;
  private rig: CameraRig;
  private model: FlightPose;
  private raf = 0;
  private last = 0;
  private sent = 0;
  private pending = false;
  private motion = new Motion(0.04);
  private interpolatedPose = createPose();
  private fireRequested = false;
  private queuedKeys: ReturnType<InputTracker["snapshot"]> | null = null;
  private lastInput = "";
  private terrainPending = false;
  private terrainChecked = 0;
  private inputRtt: number[] = [];
  private focused = true;
  private controlSerial = 0;
  private publishedAt = 0;
  private responseProbe: { at: number; axis: "pitch" | "roll" | "heading"; value: number; direction: number } | null = null;
  private responseTimes: number[] = [];
  private teaching = false;
  private active = true;
  private callbacks: (() => void)[] = [];
  private frames: number[] = [];
  private measured = 0;
  private pose = new Float64Array(7).fill(NaN);
  private cameraPose = createCamera();
  private cameraHeld = {orbitLeft:false,orbitRight:false,elevationUp:false,elevationDown:false,zoomIn:false,zoomOut:false};
  private cameraTerrain = (lon:number,lat:number) => {
    const p=point(this.terrain,lat,lon),edge=(this.terrain.size-1)*this.terrain.step/2;
    return Math.max(Math.abs(p.x),Math.abs(p.y))<edge ? ground(this.terrain,p) : this.map.queryTerrainElevation([lon,lat]);
  };
  private showFirst = false;
  constructor(
    private map: GLMap,
    private terrain: Terrain,
    public body: Body,
    private epoch: number,
    private publish: (b: Body) => void,
    private fireQueen: () => void,
    private cameraState: (mode: string) => void,
    private display: (
      id: string,
      pose: Pose,
      now: number,
      time: number,
    ) => void,
    private controlError: (error: unknown) => void,
    teaching = false,
    private observer = false,
  ) {
    this.worldTime=body.sample_time;
    this.rig = new CameraRig(body.heading * RAD2DEG);
    if(observer)this.rig.zoom=12;
    if (body.kind === "queen") this.rig.zoom = 16.2;
    this.model = new FlightPose(
      body.pos.lon,
      body.pos.lat,
      body.pos.alt,
      body.heading * RAD2DEG,
    );
    this.motion.push(body, performance.now());
    this.teaching = teaching;
    if (!teaching) this.input.attach();
    map.stop();
    map.setProjection({ type: observer?"globe":"mercator" });
    map.setMaxZoom(22);
    map.setMaxPitch(89);
    map.setCenterClampedToGround(false);
    map.setTerrain({ source: "terrarium", exaggeration: 1 });
    for (const h of [
      map.dragPan,
      map.scrollZoom,
      map.doubleClickZoom,
      map.touchZoomRotate,
      map.keyboard,
      map.dragRotate,
      map.touchPitch,
    ]) {
      const enabled = h.isEnabled();
      h.disable();
      this.callbacks.push(() => (enabled ? h.enable() : h.disable()));
    }
    window.addEventListener("keydown", this.onKey);
    window.addEventListener("blur", this.onBlur);
    window.addEventListener("focus", this.onFocus);
    this.raf = requestAnimationFrame(this.frame);
  }
  private onBlur = () => {
    this.focused = false;
    this.controlSerial++;
    this.fireRequested = false;
    this.queuedKeys = null;
    this.lastInput = "";
    this.body = { ...this.body, mode: "hold" };
    if (this.active && !this.teaching && !this.observer)
      void labCommand(
        this.epoch,
        "hold",
        this.body.id,
        undefined,
        this.body.hive,
      ).catch((e) => { if (this.active) this.controlError(e); });
  };
  private onFocus = () => { this.focused = true; };
  private onKey = (e: KeyboardEvent) => {
    if(this.observer || (typeof HTMLElement!=="undefined" && e.target instanceof HTMLElement && e.target.matches("input,textarea,[contenteditable=true]")))return;
    const turn = ({ ArrowUp: ["pitch", -1], ArrowDown: ["pitch", 1], ArrowLeft: ["roll", -1], ArrowRight: ["roll", 1], KeyA: ["heading", -1], KeyD: ["heading", 1] } as Record<string, readonly ["pitch" | "roll" | "heading", number]>)[e.code];
    if (!this.teaching && !e.repeat && turn) {
      const [axis, direction] = turn;
      this.responseProbe = { at: performance.now(), axis, direction, value: this.model.s[axis] };
    }
    if (
      !this.teaching &&
      e.code === "Space" &&
      this.body.kind === "queen" &&
      !e.repeat
    ) {
      e.preventDefault();
      this.fireQueen();
    }
    if (!this.teaching && this.focused && !e.repeat &&
      ["KeyW", "KeyS", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "KeyA", "KeyD", "ShiftLeft", "ShiftRight", "Space"].includes(e.code)) {
      if (this.body.kind === "dragonfly" && this.input.isFiring()) this.fireRequested = true;
      this.queuedKeys = this.input.snapshot();
      if (!this.pending && (e.code !== "Space" || this.body.kind === "dragonfly"))
        void this.sendInput(performance.now(), this.queuedKeys);
    }
  };
  private acceptBody(b: Body) {
    if (b.sample_time < this.body.sample_time) return false;
    this.body = b;
    this.motion.push(b, performance.now());
    return true;
  }
  update(g: Lab) {
    if (g.epoch !== this.epoch) return;
    this.worldTime=g.time;
    if(this.observer)return;
    const b = g.units.find((u) => u.id === this.body.id);
    if (b) {
      this.acceptBody(b);
      if (!b.alive) this.setTeaching(true);
    }
  }
  setTeaching(value: boolean) {
    if (value === this.teaching) return;
    this.teaching = value;
    this.controlSerial++;
    this.lastInput = "";
    this.fireRequested = false;
    this.queuedKeys = null;
    this.input.detach();
    this.motion.reset();
    this.motion.push(this.body, performance.now());
    this.last = 0;
    if (!value) this.input.attach();
  }
  fireSpit() {
    if (!this.teaching && this.body.kind === "dragonfly") this.fireRequested = true;
  }
  private async sendInput(now: number, keys: ReturnType<InputTracker["snapshot"]>) {
    this.pending = true;
    this.queuedKeys = null;
    const serial = this.controlSerial;
    const firing = this.fireRequested;
    const controls = { ...keys, altHold: this.holdAssist, fire: firing };
    try {
      if (this.body.mode === "hold") {
        const g = await labCommand(this.epoch, "pilot", this.body.id, undefined, this.body.hive);
        if (!this.active || this.teaching || serial !== this.controlSerial) return;
        const b = g.units.find((u) => u.id === this.body.id);
        if (!b || g.epoch !== this.epoch) return;
        this.acceptBody(b);
      }
      if (!this.active || this.teaching || serial !== this.controlSerial) return;
      this.sent = performance.now();
      this.lastInput = JSON.stringify(controls);
      if (firing) this.fireRequested = false;
      const b = await labInput(this.epoch, this.body.id, controls);
      this.inputRtt.push(performance.now() - this.sent);
      if (this.inputRtt.length > 100) this.inputRtt.shift();
      if (this.active && !this.teaching && serial === this.controlSerial) {
        const modeChanged = b.mode !== this.body.mode;
        if (this.acceptBody(b) && (modeChanged || performance.now() - this.publishedAt >= 100)) {
          this.publishedAt = performance.now();
          this.publish(b);
        }
      }
    } catch (e) {
      if (this.active && serial === this.controlSerial) {
        if (firing) this.fireRequested = true;
        if (e instanceof LabError && e.message.includes("Unit not piloted")) this.body = { ...this.body, mode: "hold" };
        else this.controlError(e);
      }
    } finally {
      this.sent = Math.max(this.sent, now);
      this.pending = false;
    }
  }
  private refreshTerrain(now: number) {
    if (this.terrainPending || now - this.terrainChecked < 500) return;
    const p = point(this.terrain, this.body.pos.lat, this.body.pos.lon);
    if (Math.max(Math.abs(p.x), Math.abs(p.y)) < 5000) return;
    this.terrainChecked = now;
    this.terrainPending = true;
    void labTerrain(this.body.id).then((t) => {
      if (this.active) this.terrain = t;
    }).catch(() => {}).finally(() => { this.terrainPending = false; });
  }
  private frame = (now: number) => {
    if (!this.active) return;
    const realDt = this.last ? (now - this.last) / 1000 : 1 / 60;
    const dt = Math.min(realDt, 0.1);
    this.last = now;
    this.frames.push(realDt * 1000);
    if (this.frames.length > 600) this.frames.shift();
    if (now - this.measured > 1000) {
      const sorted = [...this.frames].sort((a, b) => a - b),
        cv = this.map.getCanvas();
      cv.dataset.frameP50 = String(sorted[Math.floor(sorted.length * 0.5)]);
      cv.dataset.frameP99 = String(
        sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.99))],
      );
      if (this.inputRtt.length) {
        const rtt = [...this.inputRtt].sort((a, b) => a - b);
        cv.dataset.inputRttP50 = String(rtt[Math.floor(rtt.length * 0.5)]);
        cv.dataset.inputRttP95 = String(rtt[Math.min(rtt.length - 1, Math.floor(rtt.length * 0.95))]);
      }
      if (this.responseTimes.length) {
        const response = [...this.responseTimes].sort((a, b) => a - b);
        cv.dataset.inputVisibleP50 = String(response[Math.floor(response.length * 0.5)]);
        cv.dataset.inputVisibleP95 = String(response[Math.min(response.length - 1, Math.floor(response.length * 0.95))]);
      }
      this.measured = now;
    }
    if (!this.teaching && this.focused) {
      if (this.input.consumeCameraToggle()) {
        this.rig.toggleFirstPerson();
        this.showFirst = this.rig.isFirstPerson;
        this.cameraState(this.rig.mode);
      }
      if (this.input.consumeLockToggle()) {
        this.rig.toggleLock();
        this.cameraState(this.rig.mode);
      }
      if (this.input.consumeResetToggle()) {
        this.rig.reset(this.body.heading * RAD2DEG);
        if (this.body.kind === "queen") this.rig.zoom = 16.2;
        if(this.observer)this.rig.zoom=12;
        this.cameraState(this.rig.mode);
      }
      if (this.input.consumeAltHoldToggle() && !this.observer) this.holdAssist = !this.holdAssist;
      const keys = this.queuedKeys ?? this.input.snapshot();
      if (this.body.kind === "dragonfly" && this.input.isFiring()) this.fireRequested = true;
      const intent = Object.values(keys).some(Boolean) || this.fireRequested;
      const signature = JSON.stringify({ ...keys, altHold: this.holdAssist, fire: this.fireRequested });
      if(this.observer){
        const h=this.map.queryTerrainElevation([this.body.pos.lon,this.body.pos.lat]);
        this.godNavigation.step(this.body,keys,dt,h);
        if(now-this.publishedAt>=100){this.publishedAt=now;this.publish({...this.body,pos:{...this.body.pos},v:{...this.body.v}});}
      } else if (
        !this.pending &&
        (now - this.sent >= 25 || signature !== this.lastInput) &&
        (["manual", "preparing", "rising", "landing"].includes(this.body.mode) ||
          (this.body.mode === "hold" && intent))
      ) {
        void this.sendInput(now, keys);
      }
      if(!this.observer)this.refreshTerrain(now);
    }
    const b = this.body,
      s = this.model.s,
      pose = this.observer ? this.interpolatedPose : this.motion.renderInto(now, this.teaching, this.interpolatedPose);
    if(this.observer){pose.pos.lat=b.pos.lat;pose.pos.lon=b.pos.lon;pose.pos.alt=b.pos.alt;pose.heading=b.heading;pose.pitch=0;pose.roll=0;}
    if (this.responseProbe) {
      const p = this.responseProbe;
      const delta = Math.atan2(Math.sin(pose[p.axis] - p.value), Math.cos(pose[p.axis] - p.value));
      if (delta * p.direction > 0.005) {
        this.responseTimes.push(now - p.at);
        if (this.responseTimes.length > 64) this.responseTimes.shift();
        this.responseProbe = null;
      } else if (now - p.at > 1000) this.responseProbe = null;
    }
    s.lng = pose.pos.lon;
    s.lat = pose.pos.lat;
    s.alt = pose.pos.alt + (b.kind === "queen" ? QUEEN_HULL_HEIGHT*Math.max(.12,Math.cbrt((b.dry_mass??1e7)/1e7)) : 0);
    s.heading = pose.heading;
    s.pitch = pose.pitch;
    s.roll = pose.roll;
    s.vEast = b.v.x;
    s.vNorth = b.v.y;
    s.vUp = b.v.z;
    this.display(b.id, pose, now, this.observer?this.worldTime:this.motion.time);
    const camera = this.rig.isFirstPerson
      ? buildCamera(this.map, this.model, this.cameraPose)
      : this.rig.update(
          dt,
          s,
          this.input.cameraHeld(this.cameraHeld),
          this.cameraTerrain,
          this.map.transform.cameraToCenterDistance,
          this.cameraPose,
        );
    const p=this.pose;
    if (!Number.isFinite(p[0]) || Math.abs(camera.center[0]-p[0])>1e-7 || Math.abs(camera.center[1]-p[1])>1e-7 ||
        Math.abs(camera.elevation-p[2])>.02 || Math.abs(camera.zoom-p[3])>.001 || Math.abs(camera.bearing-p[4])>.01 ||
        Math.abs(camera.pitch-p[5])>.01 || Math.abs(camera.roll-p[6])>.01) {
      this.map.jumpTo(camera);
      p[0]=camera.center[0];p[1]=camera.center[1];p[2]=camera.elevation;p[3]=camera.zoom;
      p[4]=camera.bearing;p[5]=camera.pitch;p[6]=camera.roll;
    } else this.map.triggerRepaint();
    this.raf = requestAnimationFrame(this.frame);
  };
  private holdAssist = true;
  get firstPerson() {
    return this.showFirst;
  }
  get assisted() { return this.holdAssist; }
  toggleCamera() {
    if (this.teaching) return;
    this.rig.toggleFirstPerson();
    this.showFirst = this.rig.isFirstPerson;
    this.cameraState(this.rig.mode);
  }
  toggleLock() {
    if (this.teaching) return;
    this.rig.toggleLock();
    this.cameraState(this.rig.mode);
  }
  exit() {
    this.active = false;
    this.controlSerial++;
    cancelAnimationFrame(this.raf);
    this.input.detach();
    window.removeEventListener("keydown", this.onKey);
    window.removeEventListener("blur", this.onBlur);
    window.removeEventListener("focus", this.onFocus);
    this.callbacks.forEach((f) => f());
    if(this.observer)return Promise.resolve();
    return labCommand(
      this.epoch,
      "hold",
      this.body.id,
      undefined,
      this.body.hive,
    ).catch(() => undefined);
  }
}
