// Small synthesized signatures, unlocked only by a player gesture.
let context: AudioContext | null = null;
let enabled = true;
export function unlockSound(){if(!enabled)return;context??=new AudioContext();void context.resume();}
export function setSound(value:boolean){enabled=value;if(value)unlockSound();}
export function signature(kind:'launch'|'impact'){
  if(!enabled||!context||context.state!=='running')return;
  const c=context,now=c.currentTime,duration=kind==='impact'?5:2;
  const gain=c.createGain();gain.connect(c.destination);gain.gain.setValueAtTime(0.001,now);gain.gain.exponentialRampToValueAtTime(kind==='impact'?0.24:0.10,now+0.05);gain.gain.exponentialRampToValueAtTime(0.001,now+duration);
  const osc=c.createOscillator();osc.type='triangle';osc.frequency.setValueAtTime(kind==='impact'?90:45,now);osc.frequency.exponentialRampToValueAtTime(kind==='impact'?18:260,now+duration);osc.connect(gain);osc.start(now);osc.stop(now+duration);
  const buffer=c.createBuffer(1,c.sampleRate*duration,c.sampleRate),samples=buffer.getChannelData(0);for(let i=0;i<samples.length;i++)samples[i]=(Math.random()*2-1)*0.5;
  const noise=c.createBufferSource();noise.buffer=buffer;const filter=c.createBiquadFilter();filter.type='lowpass';filter.frequency.setValueAtTime(kind==='impact'?2000:700,now);filter.frequency.exponentialRampToValueAtTime(80,now+duration);noise.connect(filter);filter.connect(gain);noise.start(now);noise.stop(now+duration);
}
