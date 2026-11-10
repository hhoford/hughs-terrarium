import type {
  Map as GLMap,
  CustomLayerInterface,
  CustomRenderMethodInput,
} from "maplibre-gl";
import * as THREE from "three";
import { QueenLayer, QUEEN_HULL_HEIGHT } from "./flight/QueenLayer";
import { DragonflyLayer } from "./flight/DragonflyLayer";
import type { Lab, Pos, Vector } from "./lab";
import { Motion, createPose, copyPose, type Pose } from "./flight/motion";
const sphere = new THREE.SphereGeometry(1, 16, 10);
export class LabWorld {
  private disposed = false;
  private models = new Map<
    string,
    {
      model: QueenLayer | DragonflyLayer;
      track: Motion;
      pose: Pose;
      queen: boolean;
      size: number;
    }
  >();
  private scene = new THREE.Scene();
  private geometryAssets = new Set<THREE.BufferGeometry>();
  private materialAssets = new Set<THREE.Material>();
  private camera = new THREE.Camera();
  private renderer!: THREE.WebGLRenderer;
  private origin: Pos = { lat: 0, lon: 0, alt: 0 };
  private local = new THREE.Matrix4();
  private originMatrix = new THREE.Matrix4();
  private modelMatrix = new THREE.Matrix4();
  private rotationMatrix = new THREE.Matrix4();
  private rotation = new THREE.Euler(0,0,0,"YXZ");
  private scale = new THREE.Vector3();
  private lngLat: [number,number] = [0,0];
  private seen = new Set<string>();
  private warmed = false;
  private placeEffect = (mesh:THREE.Mesh,lon:number,lat:number,alt:number,sx:number,sy:number,sz:number)=>this.place(mesh,lon,lat,alt,sx,sy,sz);
  private effects = new LabEffects();
  private layer: CustomLayerInterface;
  private frozen = false;
  constructor(private map: GLMap) {
    this.scene.add(new THREE.AmbientLight(0xffffff, 1.6));
    const light = new THREE.DirectionalLight(0xffffff, 2);
    light.position.set(-1, 3, 2);
    this.scene.add(light);
    this.layer = {
      id: "lab-world",
      type: "custom",
      renderingMode: "3d",
      onAdd: (_map, gl) => {
        this.renderer = new THREE.WebGLRenderer({
          canvas: map.getCanvas(),
          context: gl as WebGL2RenderingContext,
          antialias: true,
        });
        this.renderer.autoClear = false;
      },
      render: (_gl, options) => this.draw(options),
      onRemove: () => this.dispose(),
    };
    map.addLayer(this.layer);
  }
  update(g: Lab, hidden?: string) {
    this.frozen = !!g.tutorial;
    const seen = this.seen; seen.clear();
    for (const u of g.units) {
      if (!u.alive || u.id === hidden) continue;
      seen.add(u.id);
      let m = this.models.get(u.id);
      if (m && m.queen !== (u.kind === "queen")) {this.scene.remove(m.model);this.models.delete(u.id);m=undefined;}
      if (!m) {
        const model =
          u.kind === "queen" ? new QueenLayer() : new DragonflyLayer();
        model.matrixAutoUpdate = false;
        model.traverse((o) => {
          if (o instanceof THREE.Mesh) {
            this.geometryAssets.add(o.geometry);
            for (const material of Array.isArray(o.material)
              ? o.material
              : [o.material])
              this.materialAssets.add(material);
          }
        });
        m = { model, track: new Motion(), pose: copyPose(u,createPose()), queen: u.kind === "queen", size:1 };
        this.models.set(u.id, m);
        this.scene.add(model);
      }
      m.size=u.kind==="queen" ? Math.max(.12,Math.cbrt((u.dry_mass??1e7)/1e7)) : 1;
      m.track.push(u, performance.now());
    }
    for (const [id, m] of this.models)
      if (!seen.has(id)) {
        this.scene.remove(m.model);
        this.models.delete(id);
      }
    this.effects.update(g,this.scene);
    if(!this.warmed){this.renderer.compile(this.scene,this.camera);this.renderer.resetState();this.warmed=true;}
    this.map.triggerRepaint();
  }
  render(now: number, selected: string, pose: Pose, time: number) {
    this.origin = pose.pos;
    this.effects.displayTime = time;
    for (const [id, m] of this.models)
      if(id===selected)copyPose(pose,m.pose);else m.track.renderInto(now,this.frozen,m.pose);
  }
  private place(object:THREE.Object3D,lon:number,lat:number,alt:number,sx=1,sy=1,sz=1,pose?:Pose){
    this.lngLat[0]=lon;this.lngLat[1]=lat;
    this.modelMatrix.fromArray(this.map.transform.getMatrixForModel(this.lngLat,alt));
    object.matrix.copy(this.local).multiply(this.modelMatrix);
    if(pose){this.rotation.set(pose.pitch,-pose.heading,-pose.roll,"YXZ");object.matrix.multiply(this.rotationMatrix.makeRotationFromEuler(this.rotation));}
    this.scale.set(sx,sy,sz);object.matrix.scale(this.scale);object.matrixWorldNeedsUpdate=true;
  }
  private draw(options:CustomRenderMethodInput){
    this.lngLat[0]=this.origin.lon;this.lngLat[1]=this.origin.lat;
    this.originMatrix.fromArray(this.map.transform.getMatrixForModel(this.lngLat,this.origin.alt));
    this.local.copy(this.originMatrix).invert();
    this.camera.projectionMatrix.fromArray(options.defaultProjectionData.mainMatrix).multiply(this.originMatrix);
    this.camera.projectionMatrixInverse.copy(this.camera.projectionMatrix).invert();
    const now=performance.now();
    for(const m of this.models.values()){
      this.place(m.model,m.pose.pos.lon,m.pose.pos.lat,m.pose.pos.alt+(m.queen?QUEEN_HULL_HEIGHT*m.size:0),m.size,m.size,m.size,m.pose);
      m.model.animate(now,this.frozen);
    }
    this.effects.render(this.placeEffect);
    this.renderer.resetState();this.renderer.setViewport(0,0,this.map.getCanvas().width,this.map.getCanvas().height);
    this.renderer.render(this.scene,this.camera);this.renderer.resetState();
  }
  private dispose() {
    if (this.disposed) return;
    this.disposed = true;
    const geometries = this.geometryAssets,
      materials = this.materialAssets;
    this.scene.traverse((o) => {
      if (o instanceof THREE.Mesh) {
        geometries.add(o.geometry);
        for (const m of Array.isArray(o.material) ? o.material : [o.material])
          materials.add(m);
      }
    });
    geometries.forEach((g) => g.dispose());
    materials.forEach((m) => m.dispose());
    geometries.clear();
    materials.clear();
    this.renderer?.dispose();
    this.scene.clear();
  }
  destroy() {
    if (!this.disposed && this.map.getStyle() && this.map.getLayer(this.layer.id)) this.map.removeLayer(this.layer.id);
    this.models.clear();
  }
}
type EffectTrack={motion:Motion;pose:Pose;previous:Pos;time:number};
type PlaceEffect=(mesh:THREE.Mesh,lon:number,lat:number,alt:number,sx:number,sy:number,sz:number)=>void;
class LabEffects {
  state:Lab|null=null;displayTime=0;
  private time=-1;
  private bullets=new Map<number,EffectTrack>();
  private missiles=new Map<number,EffectTrack>();
  private freeTracks:EffectTrack[]=Array.from({length:20},()=>this.newTrack());
  private seen=new Set<number>();
  private velocity:Vector={x:0,y:0,z:0};
  private pool:THREE.Mesh<THREE.SphereGeometry,THREE.MeshBasicMaterial>[]=[];
  private used=0;
  private newTrack():EffectTrack{return{motion:new Motion(),pose:createPose(),previous:{lat:0,lon:0,alt:0},time:-1};}
  update(g:Lab,scene:THREE.Scene){
    this.state=g;
    const wanted=Math.max(g.resources.length+g.units.length*6+20,g.resources.length+g.units.length*2+g.bullets.length+g.missiles.length+g.bursts.length*6);
    while(this.pool.length<wanted){const mesh=new THREE.Mesh(sphere,new THREE.MeshBasicMaterial({transparent:true,depthWrite:false}));mesh.matrixAutoUpdate=false;mesh.frustumCulled=false;mesh.visible=false;this.pool.push(mesh);scene.add(mesh);}
    if(g.time<=this.time)return;this.time=g.time;
    this.updateTracks(g.bullets,this.bullets,g.time);this.updateTracks(g.missiles,this.missiles,g.time);
  }
  private updateTracks(items:readonly{id:number;pos:Pos}[],tracks:Map<number,EffectTrack>,time:number){
    this.seen.clear();
    for(const item of items){
      this.seen.add(item.id);let track=tracks.get(item.id);
      if(!track){track=this.freeTracks.pop()??this.newTrack();tracks.set(item.id,track);}
      const old=track.previous,p=item.pos;this.velocity.x=0;this.velocity.y=0;this.velocity.z=0;
      if(track.time>=0&&time>track.time){const dt=time-track.time;this.velocity.x=(((p.lon-old.lon+540)%360)-180)*111320*Math.cos(p.lat*Math.PI/180)/dt;this.velocity.y=(p.lat-old.lat)*111320/dt;this.velocity.z=(p.alt-old.alt)/dt;}
      track.motion.push({pos:p,v:this.velocity,sample_time:time,heading:0,pitch:0,roll:0},performance.now());
      old.lat=p.lat;old.lon=p.lon;old.alt=p.alt;track.time=time;
    }
    for(const[id,track]of tracks)if(!this.seen.has(id)){tracks.delete(id);track.motion.reset();track.time=-1;this.freeTracks.push(track);}
  }
  private draw(place:PlaceEffect,lon:number,lat:number,alt:number,sx:number,sy:number,sz:number,r:number,g:number,b:number,a:number){
    const mesh=this.pool[this.used++];if(!mesh)return;
    mesh.visible=true;mesh.material.color.setRGB(r,g,b,THREE.LinearSRGBColorSpace);mesh.material.opacity=a;place(mesh,lon,lat,alt,sx,sy,sz);
  }
  render(place:PlaceEffect){
    const g=this.state;if(!g)return;this.used=0;const now=performance.now(),frozen=!!g.tutorial;
    for(const r of g.resources)this.draw(place,r.pos.lon,r.pos.lat,r.pos.alt+15,18,25,18,r.energy>0?.9:.5,r.energy>0?.95:.8,r.energy>0?.35:1,.85);
    for(const u of g.units)if(u.alive&&u.scan_until>g.time){const radius=(1-(u.scan_until-g.time)/3)*8000;this.draw(place,u.pos.lon,u.pos.lat,u.pos.alt,radius,8,radius,.2,1,.8,.12);}
    for(const b of g.bullets){const t=this.bullets.get(b.id),p=t?t.motion.renderInto(now,frozen,t.pose).pos:b.pos;this.draw(place,p.lon,p.lat,p.alt,2,2,2,.4,1,.25,1);}
    for(const m of g.missiles){const t=this.missiles.get(m.id),p=t?t.motion.renderInto(now,frozen,t.pose).pos:m.pos;this.draw(place,p.lon,p.lat,p.alt,15,50,15,1,.8,.4,1);}
    for(const q of g.units)if(q.charge)this.draw(place,q.pos.lon,q.pos.lat,q.pos.alt+150,100,180,100,.9,.6,1,.2+.2*Math.sin(g.time*8));
    for(const b of g.bursts){
      const age=Math.max(0,b.age+(g.tutorial?0:this.displayTime-g.time)),p=b.pos;
      if(b.kind==="melt"){const r=10+age*5;this.draw(place,p.lon,p.lat,p.alt,r,r*2,r,.3,1,.2,Math.max(0,1-age/3));continue;}
      if(age<14)this.draw(place,p.lon,p.lat,p.alt,b.radius,b.radius,b.radius,1,.65,.25,.055*(1-age/14));
      if(b.precision&&age<8)this.draw(place,p.lon,p.lat,p.alt,g.balance.precision_radius,g.balance.precision_radius,g.balance.precision_radius,1,.95,.7,.3*(1-age/8));
      const scale=b.radius/6000,r=Math.min(b.radius,age*1500*scale+30*scale);
      this.draw(place,p.lon,p.lat,p.alt+Math.min(3000,age*400)*scale,r*.45,Math.min(3000,age*500+100)*scale,r*.45,1,Math.max(.15,1-age/8),.1,Math.max(0,1-age/14));
      this.draw(place,p.lon,p.lat,p.alt+Math.min(4000,age*500)*scale,r*.8,Math.max(100*scale,r*.25),r*.8,.8,.4,.15,Math.max(0,.7-age/20));
      this.draw(place,p.lon,p.lat,p.alt,r,30*scale,r,1,.7,.3,Math.max(0,.8-age/12));
    }
    for(let i=this.used;i<this.pool.length;i++)this.pool[i].visible=false;
  }
}
