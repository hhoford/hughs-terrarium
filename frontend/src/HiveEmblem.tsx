import {colour} from './lab';

// The body is the new dynasty; its parent survives as a smaller ancestral idol.
// All geometry is generated from public hive identity, without image downloads.
function idol(hive:number):string {
  const seed=Math.imul(hive+1,0x9e3779b1)>>>0;
  const arms=3+(seed%3), crown=9+((seed>>>4)%10), eye=3+((seed>>>8)%4);
  let paths='';
  for(let i=0;i<arms;i++){
    const y=27+i*9, reach=21+((seed>>>(i*3))%10);
    const tip=3+((seed>>>(i*4+1))%9);
    paths+=`<path d="M43 ${y+5}Q${48-reach} ${y+4} ${48-reach-6} ${y-tip}L${48-reach+5} ${y-2}M53 ${y+5}Q${48+reach} ${y+4} ${48+reach+6} ${y-tip}L${48+reach-5} ${y-2}"/>`;
  }
  let beads='';
  for(let i=0;i<7;i++)if((hive+1)&(1<<i))beads+=`<circle cx="${30+i*6}" cy="81" r="1.8" fill="currentColor" stroke="none"/>`;
  return `<g stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none">${paths}<path d="M39 31L${34-(seed%7)} ${crown}L45 21L48 5L51 21L${62+(seed%7)} ${crown}L57 31L61 46L54 64L48 76L42 64L35 46Z"/><path d="M43 56L48 63L53 56M44 66L48 71L52 66"/><ellipse cx="48" cy="40" rx="${eye+5}" ry="${eye+2}"/><circle cx="48" cy="40" r="${eye-1}" fill="currentColor"/>${beads}</g>`;
}
function dynasty(hive:number,lineage:readonly number[]):string {
  const parent=lineage.at(-1);
  const inherited=parent===undefined?'':`<circle cx="73" cy="72" r="20" fill="#09151d" stroke="${colour(parent)}" stroke-width="1.4"/><g transform="translate(52 51) scale(.44)" color="${colour(parent)}">${dynasty(parent,lineage.slice(0,-1))}</g>`;
  return `${idol(hive)}${inherited}`;
}
export function hiveEmblemMarkup(hive:number,lineage:readonly number[]=[]):string {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" width="100%" height="100%" aria-hidden="true">${dynasty(hive,lineage)}</svg>`;
}
export function HiveEmblem({hive,lineage=[],size=40}:{hive:number;lineage?:readonly number[];size?:number}){
  return <span className="hive-emblem" style={{width:size,height:size,color:colour(hive)}} dangerouslySetInnerHTML={{__html:hiveEmblemMarkup(hive,lineage)}}/>;
}
