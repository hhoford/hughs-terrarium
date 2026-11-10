import {useEffect,useRef,useState} from 'react';
import maplibregl from 'maplibre-gl';
import {buildStyle} from './style';
import {Symbol} from './Symbol';
import {HiveEmblem} from './HiveEmblem';
import {colour,radarTag,type RadarTag,type Body,type Lab,type Mark,type Pos} from './lab';

export function GlobalRadar({game,pilot,target,onTarget,teaching=false,onUpdate}:{game:Lab;pilot:Body;target:Pos|null;onTarget:(m:Mark)=>void;teaching?:boolean;onUpdate:(g:Lab)=>void}) {
  const [draft,setDraft]=useState<{label:string;pos:Pos;id?:number}|null>(null),[saving,setSaving]=useState(false),[error,setError]=useState('');
  const paused=useRef(teaching);paused.current=teaching;
  const edit=(t?:RadarTag)=>{if(!teaching){setError('');setDraft(t?{label:t.label,pos:{...t.pos},id:t.id}:{label:'',pos:{...pilot.pos}});}};
  const save=async(remove=false)=>{if(!draft||saving||teaching)return;setSaving(true);try{onUpdate(await radarTag(game.epoch,pilot.id,draft.label,draft.pos,draft.id,remove));setDraft(null);}catch(e){setError(String(e));}finally{setSaving(false);}};
  const host=useRef<HTMLDivElement>(null),map=useRef<maplibregl.Map|null>(null),following=useRef(true),latest=useRef(pilot);
  latest.current=pilot;
  useEffect(()=>{
    const key=(e:KeyboardEvent)=>{if(e.code==='KeyK'&&!e.repeat&&!e.ctrlKey&&!e.metaKey&&!e.altKey&&!paused.current&&!(e.target instanceof HTMLElement&&e.target.matches('input,textarea,[contenteditable=true]'))){e.preventDefault();setDraft({label:'',pos:{...latest.current.pos}});}};
    window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key);
  },[]);
  const [ready,setReady]=useState(false),[view,setView]=useState(0);
  useEffect(()=>{
    const style=buildStyle();delete style.terrain;delete style.sources.terrarium;delete style.sources.water;
    style.layers=style.layers.filter(l=>l.id==='space'||l.id==='relief'||l.id==='hillshade');
    const m=new maplibregl.Map({container:host.current!,style,center:[latest.current.pos.lon,latest.current.pos.lat],zoom:0.05,minZoom:-.5,maxZoom:12,attributionControl:false,pixelRatio:1,keyboard:false,pitchWithRotate:false,dragRotate:false,touchPitch:false});
    map.current=m;m.doubleClickZoom.disable();m.touchZoomRotate.disableRotation();
    m.on('load',()=>setReady(true));m.on('move',()=>setView(v=>v+1));m.on('dragstart',()=>{following.current=false;});
    m.on('contextmenu',e=>{e.originalEvent.preventDefault();if(!paused.current)setDraft({label:'',pos:{lat:e.lngLat.lat,lon:e.lngLat.lng,alt:0}});});
    const resize=new ResizeObserver(()=>{m.resize();setView(v=>v+1);});resize.observe(host.current!);
    return()=>{resize.disconnect();map.current=null;m.remove();};
  },[]);
  useEffect(()=>{
    const m=map.current;if(!m||!ready||!following.current)return;
    const c=m.getCenter();if(Math.abs(c.lat-pilot.pos.lat)>1||Math.abs((((c.lng-pilot.pos.lon)+540)%360)-180)>1)m.easeTo({center:[pilot.pos.lon,pilot.pos.lat],duration:250});
  },[pilot.pos.lat,pilot.pos.lon,ready]);
  const used:[number,number][]=[];
  const point=(p:Pos,declutter=true):{x:number;y:number;exact:[number,number]}|null=>{
    const m=map.current;if(!ready||!m)return null;const c=m.getCenter();const rad=Math.PI/180;
    if(Math.sin(c.lat*rad)*Math.sin(p.lat*rad)+Math.cos(c.lat*rad)*Math.cos(p.lat*rad)*Math.cos((p.lon-c.lng)*rad)<0)return null;
    const q=m.project([p.lon,p.lat]);const w=m.getContainer().clientWidth;let x=q.x/w*240,y=q.y/w*240;const exact:[number,number]=[x,y];
    if(x<0||x>240||y<0||y>240)return null;
    for(let step=0;declutter&&step<80&&used.some(a=>Math.hypot(a[0]-x,a[1]-y)<26);step++){
      const angle=(step+1)*2.4,radius=16+Math.sqrt(step+1)*12;
      x=Math.max(14,Math.min(226,exact[0]+Math.sin(angle)*radius));
      y=Math.max(14,Math.min(226,exact[1]+Math.cos(angle)*radius));
    }
    x=Math.max(12,Math.min(228,x));y=Math.max(12,Math.min(228,y));if(declutter)used.push([x,y]);return{x,y,exact};
  };
  const focus=(p:Pos)=>{following.current=false;map.current?.easeTo({center:[p.lon,p.lat],duration:300});};
  const contacts=(pilot.kind==='god'?[pilot,...game.units]:game.units).map(u=>({u,p:point(u.pos)}));
  const marks=game.marks.map(m=>({m,p:point(m.pos)}));
  // view is advanced by globe movement; positions come from authoritative snapshots.
  void view;
  return <aside className="global-radar globe-radar" aria-label={pilot.kind==='god'?'Omniscient God radar globe':`Global radar from hive ${pilot.hive+1}`} style={{'--hive':colour(pilot.hive)} as React.CSSProperties}>
    <div ref={host} className="radar-map" aria-label="Radar globe map"/>
    <svg className="radar-overlay" viewBox="0 0 240 240" role="img" aria-label="Global radar contacts">
      {(game.pheromones??[]).map(c=>{const p=point(c.pos,false);return p&&<g key={`p${c.hive}-${c.cell}`} aria-label={`Hive ${c.hive+1} trail ${c.trail.toFixed(2)}, danger ${c.danger.toFixed(2)}`}>
        {c.trail>0&&<circle cx={p.exact[0]} cy={p.exact[1]} r={3+9*c.trail} fill="#7de4ce" opacity={.15+.4*c.trail}/>}
        {c.danger>0&&<circle cx={p.exact[0]} cy={p.exact[1]} r={3+9*c.danger} fill="#ff8369" opacity={.15+.4*c.danger}/>}</g>;})}
      {game.resources.map(r=>{const p=point(r.pos,false);return p&&<path key={`r${r.id}`} d={`M${p.exact[0]} ${p.exact[1]-3}l3 3-3 3-3-3z`} fill="#eee18e" aria-label={`Scanned resource ${r.id}`}/>})}
      {game.missiles.map(m=>{const p=point(m.pos);return p&&<path key={`s${m.id}`} d={`M${p.exact[0]} ${p.exact[1]-5}l4 8h-8z`} fill="#ffac78"/>})}
      {contacts.map(({u,p})=>p&&<g key={u.id} style={{color:u.alive?colour(u.hive):'#d17d84'}}>
        <line x1={p.exact[0]} y1={p.exact[1]} x2={p.x} y2={p.y} stroke="currentColor" opacity=".4"/>
        <circle cx={p.exact[0]} cy={p.exact[1]} r="1.5" fill="currentColor"/>
        <foreignObject x={p.x-12} y={p.y-12} width="24" height="24"><button className={`radar-contact ${u.id===pilot.id?'self':''} ${u.charge?'charging':''}`} aria-label={`Radar ${u.alive?'':'dead '}${u.kind} ${u.id} of hive ${u.hive+1}`} onClick={()=>focus(u.pos)}>{u.alive&&(u.kind==='queen'||u.founder)?<HiveEmblem hive={u.hive} lineage={u.lineage} size={21}/>:<Symbol name={u.alive?u.kind:'dead'} size={17}/>}</button></foreignObject>
      </g>)}
      {marks.map(({m,p})=>p&&<g key={`m${m.id}`} style={{color:'#f8d58c',opacity:Math.max(.35,(m.expires-game.time)/60)}}>
        <line x1={p.exact[0]} y1={p.exact[1]} x2={p.x} y2={p.y} stroke="currentColor" opacity=".45"/>
        {target&&Math.hypot(target.lat-m.pos.lat,target.lon-m.pos.lon)<.00001&&<circle cx={p.x} cy={p.y} r="13" stroke="currentColor" fill="none"/>}
        <foreignObject x={p.x-11} y={p.y-11} width="22" height="22"><button className={`radar-contact mark-contact ${m.accuracy_m<=10?'precise':''}`} aria-label={`Select dragonfly mark ${m.id}, ${m.accuracy_m} metre accuracy`} onClick={()=>{onTarget(m);focus(m.pos);}}><Symbol name="mark" size={16}/></button></foreignObject>
      </g>)}
      {(game.tags??[]).map(t=>{const p=point(t.pos,false);return p&&<g key={`tag${t.id}`} className="radar-tag" role="button" tabIndex={0} aria-label={`Edit radar tag ${t.label}`} onClick={()=>edit(t)} onKeyDown={e=>{if(e.key==='Enter'){e.preventDefault();edit(t);}}}>
        <circle cx={p.exact[0]} cy={p.exact[1]} r="3" fill="#f8d58c"/>
        <text x={Math.max(8,Math.min(140,p.exact[0]+5))} y={Math.max(12,Math.min(222,p.exact[1]-5))}>{t.label.length>18?t.label.slice(0,18)+'…':t.label}</text><title>{t.label}</title></g>;})}
    </svg>
    <div className="mark-dock">{game.marks.slice(-8).map(m=><button key={m.id} className={m.accuracy_m<=10?"precise":""} aria-label={`Select dragonfly mark ${m.id}, ${m.accuracy_m} metre accuracy`} onClick={()=>{onTarget(m);focus(m.pos);}}><Symbol name="mark" size={14}/></button>)}</div>
    <button className="radar-add-tag" aria-label="Add labeled radar tag" onClick={()=>edit()}><Symbol name="mark" size={16}/></button>
    {draft&&<form className="radar-tag-editor" onSubmit={e=>{e.preventDefault();void save();}}>
      <input autoFocus aria-label="Radar tag label" maxLength={48} value={draft.label} onChange={e=>setDraft({...draft,label:e.target.value})}/>
      <button type="submit" aria-label="Save radar tag" disabled={saving||!draft.label.trim()}><Symbol name="mark" size={15}/></button>
      {draft.id!==undefined&&<button type="button" aria-label="Delete radar tag" disabled={saving} onClick={()=>void save(true)}><Symbol name="dead" size={15}/></button>}
      <button type="button" aria-label="Cancel radar tag" onClick={()=>setDraft(null)}><Symbol name="close" size={15}/></button>
      {error&&<output>{error}</output>}
    </form>}
    <button className="radar-centre" aria-label="Centre radar on pilot" onClick={()=>{following.current=true;map.current?.easeTo({center:[pilot.pos.lon,pilot.pos.lat],zoom:.05,duration:300});}}><Symbol name={pilot.kind} size={16}/></button>
  </aside>;
}
