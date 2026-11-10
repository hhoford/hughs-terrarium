import {Vector3,Quaternion} from 'three';
import type {Body,Pos} from '../lab';
import type {FlightInput} from './pose';
export const GOD_ID='god';
export function godBody(pos:Pos,time:number):Body {
 return {id:GOD_ID,kind:'god',hive:-1,pos:{...pos},sample_time:time,v:{x:0,y:0,z:0},thrust:{x:0,y:0,z:0},heading:0,pitch:0,roll:0,
 energy:1,capacity:1,power:0,alive:true,grounded:false,mode:'observer',agl:pos.alt,dry_mass:0,founder:false,construction_remaining:0,growth_target:0,
 terrain_mean:null,radar_altitude:null,shelter_altitude:null,radar_visible:true,nuclear_sheltered:true,brood_remaining:0,cooldown:0,minerals:0,cargo_energy:0,cargo_minerals:0,scan_until:0,charge:null};
}
// Spectator map navigation, never a body in the authoritative simulation.
export class GodNavigation {
 private up=new Vector3();private east=new Vector3();private north=new Vector3();private tangent=new Vector3();
 private axis=new Vector3();private facing=new Vector3();private turn=new Quaternion();
 step(b:Body,k:FlightInput,dt:number,height:number|null=0){
  dt=Math.min(.1,Math.max(0,dt));
  b.heading+=((+k.yawRight)-(+k.yawLeft))*1.3*dt;
  const a=b.pos.lat*Math.PI/180,l=b.pos.lon*Math.PI/180;
  this.up.set(Math.cos(a)*Math.cos(l),Math.cos(a)*Math.sin(l),Math.sin(a));
  this.east.set(-Math.sin(l),Math.cos(l),0);this.north.set(-Math.sin(a)*Math.cos(l),-Math.sin(a)*Math.sin(l),Math.cos(a));
  const forward=(+k.forward)-(+k.back),side=(+k.strafeRight)-(+k.strafeLeft),speed=k.boost?100000:2000;
  let e=forward*Math.sin(b.heading)+side*Math.cos(b.heading),n=forward*Math.cos(b.heading)-side*Math.sin(b.heading);
  const norm=Math.max(1,Math.hypot(e,n));e=e/norm*speed;n=n/norm*speed;
  const v=((+k.thrustUp)-(+k.thrustDown))*speed;
  this.tangent.copy(this.east).multiplyScalar(e).addScaledVector(this.north,n);
  this.facing.copy(this.east).multiplyScalar(Math.sin(b.heading)).addScaledVector(this.north,Math.cos(b.heading));
  const distance=this.tangent.length()*dt;
  if(distance>0){
   this.axis.crossVectors(this.up,this.tangent).normalize();this.turn.setFromAxisAngle(this.axis,distance/(6371000+b.pos.alt));
   this.up.applyQuaternion(this.turn);this.facing.applyQuaternion(this.turn);
   b.pos.lat=Math.asin(Math.max(-1,Math.min(1,this.up.z)))*180/Math.PI;b.pos.lon=Math.atan2(this.up.y,this.up.x)*180/Math.PI;
   const lat=b.pos.lat*Math.PI/180,lon=b.pos.lon*Math.PI/180;
   this.east.set(-Math.sin(lon),Math.cos(lon),0);this.north.set(-Math.sin(lat)*Math.cos(lon),-Math.sin(lat)*Math.sin(lon),Math.cos(lat));
   b.heading=Math.atan2(this.facing.dot(this.east),this.facing.dot(this.north));
  }
  b.pos.alt=Math.max((height??0)+30,Math.min(5000000,b.pos.alt+v*dt));
  b.v.x=e;b.v.y=n;b.v.z=v;b.agl=b.pos.alt-(height??0);b.sample_time+=dt;
 }
}
