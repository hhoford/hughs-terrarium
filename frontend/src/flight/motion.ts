import type { Body, Pos } from "../lab";
export type Pose = { pos: Pos; heading: number; pitch: number; roll: number };
export type MotionValue = Pick<Body, "sample_time" | "pos" | "v" | "heading" | "pitch" | "roll">;
type Sample = { body: MotionValue; time: number; arrival: number };
export const createPose = (): Pose => ({pos:{lat:0,lon:0,alt:0},heading:0,pitch:0,roll:0});
export function copyPose(source:Pose, out:Pose):Pose {
  out.pos.lat=source.pos.lat;out.pos.lon=source.pos.lon;out.pos.alt=source.pos.alt;
  out.heading=source.heading;out.pitch=source.pitch;out.roll=source.roll;return out;
}
const angle=(a:number,b:number,t:number)=>a+Math.atan2(Math.sin(b-a),Math.cos(b-a))*t;
const hermite=(p:number,q:number,v:number,w:number,t:number,span:number)=>
 (2*t*t*t-3*t*t+1)*p+(t*t*t-2*t*t+t)*span*v+(-2*t*t*t+3*t*t)*q+(t*t*t-t*t)*span*w;
// Fixed storage: arrivals copy only motion fields; renderInto reuses the caller's pose.
export class Motion {
  private samples:Sample[]=Array.from({length:40},()=>({body:{sample_time:0,pos:{lat:0,lon:0,alt:0},v:{x:0,y:0,z:0},heading:0,pitch:0,roll:0},time:0,arrival:0}));
  private head=0;
  private count=0;
  private cursor=0;
  private last=0;
  constructor(private delay=0.12){}
  private at(i:number){return this.samples[(this.head-this.count+i+40)%40];}
  reset(){this.head=0;this.count=0;this.cursor=0;this.last=0;}
  push(body:MotionValue,arrival:number){
    const tail=this.count?this.at(this.count-1):undefined;
    const time=body.sample_time;if(tail&&time<=tail.time+1e-8)return false;
    const lon=tail?tail.body.pos.lon+(((body.pos.lon-tail.body.pos.lon+540)%360)-180):body.pos.lon;
    const slot=this.samples[this.head];copyPose(body,slot.body);slot.body.pos.lon=lon;
    slot.body.v.x=body.v.x;slot.body.v.y=body.v.y;slot.body.v.z=body.v.z;slot.body.sample_time=time;
    slot.time=time;slot.arrival=arrival;this.head=(this.head+1)%40;this.count=Math.min(40,this.count+1);
    if(!tail){this.cursor=time-this.delay;this.last=arrival;}return true;
  }
  get time(){return this.cursor;}
  // Retain the allocating convenience API for callers which keep historical results.
  render(now:number,frozen=false):Pose{return this.renderInto(now,frozen,createPose());}
  renderInto(now:number,frozen:boolean,out:Pose):Pose{
    if(!this.count)return out;
    const end=this.at(this.count-1);if(frozen)return copyPose(end.body,out);
    const gap=Math.max(0,(now-this.last)/1000);this.last=now;
    const target=end.time+Math.min(0.15,Math.max(0,(now-end.arrival)/1000))-this.delay;
    const elapsed=gap>0.5?0:Math.min(0.1,gap);if(gap>0.5)this.cursor=target;
    this.cursor+=elapsed*Math.max(0.85,Math.min(1.15,1+(target-this.cursor)*2));
    this.cursor=Math.min(this.cursor,end.time+0.1);
    let a=this.at(0),b=end;
    for(let i=1;i<this.count;i++){const sample=this.at(i);if(sample.time>=this.cursor){b=sample;break;}a=sample;}
    if(this.cursor<=a.time)return copyPose(a.body,out);
    if(this.cursor>=end.time){
      const d=Math.min(0.1,this.cursor-end.time),u=end.body,c=Math.cos(u.pos.lat*Math.PI/180);
      copyPose(u,out);out.pos.lat+=u.v.y*d/111320;out.pos.lon+=u.v.x*d/(111320*c);out.pos.alt+=u.v.z*d;return out;
    }
    const span=b.time-a.time,t=Math.max(0,Math.min(1,(this.cursor-a.time)/span)),u=a.body,v=b.body,cos=Math.cos(u.pos.lat*Math.PI/180);
    const lon=u.pos.lon+(((v.pos.lon-u.pos.lon+540)%360)-180);
    out.pos.lat=hermite(u.pos.lat,v.pos.lat,u.v.y/111320,v.v.y/111320,t,span);
    out.pos.lon=hermite(u.pos.lon,lon,u.v.x/(111320*cos),v.v.x/(111320*cos),t,span);
    out.pos.alt=hermite(u.pos.alt,v.pos.alt,u.v.z,v.v.z,t,span);
    out.heading=angle(u.heading,v.heading,t);out.pitch=angle(u.pitch,v.pitch,t);out.roll=angle(u.roll,v.roll,t);return out;
  }
}
