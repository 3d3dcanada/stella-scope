import {finite,volumetricFlow,flowCeiling} from './hud-core.js';

const clamp=(v,min,max)=>Math.max(min,Math.min(max,v));
const number=v=>finite(v);
const string=v=>typeof v==='string'?v:'';

export function xyTravel(limits){
 const a=limits?.min,b=limits?.max;if(!Array.isArray(a)||!Array.isArray(b))return null;
 const [x0,y0]=a,[x1,y1]=b;if([x0,y0,x1,y1].some(v=>number(v)===null||Math.abs(v)>2000))return null;
 const width=x1-x0,height=y1-y0;if(width<10||height<10||width>2000||height>2000)return null;
 return {x0,y0,x1,y1,width,height,cx:(x0+x1)/2,cy:(y0+y1)/2};
}

export function fromTick(tick){
 const point=tick?.point,job=tick?.job,axes=string(tick?.homed_axes).toLowerCase();
 const heater=h=>({temp:number(h?.temperature),target:number(h?.target),power:number(h?.power)});
 const fan=f=>({speed:number(f?.speed),rpm:number(f?.rpm)});
 const progress=number(job?.progress);
 return {
  homedAxes:axes,homed:{x:axes.includes('x'),y:axes.includes('y'),z:axes.includes('z')},
  job:{state:string(job?.state).toLowerCase(),name:string(job?.name),progress:progress===null?null:clamp(progress,0,1),duration:number(job?.duration),filament:number(job?.filament_used)},
  position:{x:number(point?.position?.[0]),y:number(point?.position?.[1]),z:number(point?.position?.[2])},
  velocity:number(point?.velocity),maxVelocity:number(point?.max_velocity),maxAccel:number(point?.max_accel),
  zOffset:number(point?.z_offset),qgl:typeof point?.qgl_applied==='boolean'?point.qgl_applied:null,
  speedFactor:number(point?.speed_factor),extrudeFactor:number(point?.extrude_factor),extruderVelocity:number(point?.extruder_velocity),
  nozzle:heater(point?.nozzle),bed:heater(point?.bed),
  fans:{part:fan(point?.fans?.fan),hotend:fan(point?.fans?.hotend_fan),controller:fan(point?.fans?.controller_fan)}
 };
}

export function deriveMode({link,live,readings}={}){
 const atTemp={};let heating=false;
 for(const kind of ['nozzle','bed']){
  const temp=number(readings?.[kind]?.temp),target=number(readings?.[kind]?.target);
  atTemp[kind]=target!==null&&target>0&&temp!==null&&Math.abs(temp-target)<=3;
  heating ||= target!==null&&target>0&&temp!==null&&Math.abs(temp-target)>3;
 }
 const state=readings?.job?.state,name=readings?.job?.name;
 let mode='idle';
 if(link?.state==='service')mode='service';
 else if(link?.state==='replay')mode='replay';
 else if(live===false)mode='rewound';
 else if(link?.state==='off')mode='off';
 else if(link?.state==='not_ready')mode='klipper';
 else if(link?.state==='unknown')mode='checking';
 else if(state==='printing')mode='printing';
 else if(state==='paused')mode='paused';
 else if(['complete','cancelled','error'].includes(state)&&name)mode='finished';
 else if(!['x','y','z'].every(axis=>readings?.homed?.[axis]))mode='unhomed';
 return {mode,heating,atTemp};
}

// The table is the complete display order; the final slot is always More.
const ROWS={
 off:[['home','i-home','HOME','home','menu:home',1],['heat','i-heat','HEAT','set_heater','sheet:heat',1],['files','i-files','FILES',null,'sheet:files',2],['more','i-more','MORE',null,'radial:more',3]],
 klipper:[['fw_restart','i-fw-restart','FIRMWARE RESTART','firmware_restart','send:firmware_restart',1,'P'],['klipper_restart','i-restart','KLIPPER RESTART','klipper_restart','send:klipper_restart',1],['console','i-console','CONSOLE','console','sheet:console',2],['more','i-more','MORE',null,'radial:more',3]],
 replay:[['rec_back','i-step-back','BACK',null,'rec:back',1],['rec_play','i-play','PLAY',null,'rec:play',1,'P'],['rec_fwd','i-step-fwd','FORWARD',null,'rec:fwd',1],['recording','i-rewind','RECORDING',null,'sheet:recording',2],['more','i-more','MORE',null,'radial:more',3]],
 rewound:[['go_live','i-live','GO LIVE',null,'rec:live',1,'P'],['rec_back','i-step-back','BACK',null,'rec:back',2],['rec_play','i-play','PLAY',null,'rec:play',2],['rec_fwd','i-step-fwd','FORWARD',null,'rec:fwd',2],['recording','i-rewind','RECORDING',null,'sheet:recording',3],['more','i-more','MORE',null,'radial:more',4]],
 unhomed:[['home','i-home','HOME','home','menu:home',1,'P'],['level','i-level','LEVEL','qgl','menu:level',1],['move','i-move','MOVE','jog','sheet:move',2],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['files','i-files','FILES',null,'sheet:files',3],['macros','i-macro','MACROS','macro','sheet:macros',3],['more','i-more','MORE',null,'radial:more',4]],
 idle:[['home','i-home','HOME','home','menu:home',1],['level','i-level','LEVEL','qgl','menu:level',1],['move','i-move','MOVE','jog','sheet:move',2],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['files','i-files','FILES',null,'sheet:files',3],['macros','i-macro','MACROS','macro','sheet:macros',3],['console','i-console','CONSOLE','console','sheet:console',3],['more','i-more','MORE',null,'radial:more',4]],
 printing:[['pause','i-pause','PAUSE','pause','send:pause',1,'P'],['speed','i-speed','SPEED','speed_factor','sheet:speed',2],['flow','i-flow','FLOW','extrude_factor','sheet:flow',2],['fan','i-fan','FAN','set_fan','sheet:fan',2],['exclude','i-exclude','EXCLUDE','exclude_object','sheet:exclude',3],['cancel','i-cancel','CANCEL','cancel','send:cancel',4,'DH'],['more','i-more','MORE',null,'radial:more',5]],
 paused:[['resume','i-play','RESUME','resume','send:resume',1,'P'],['move','i-move','MOVE','jog','sheet:move',2],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['exclude','i-exclude','EXCLUDE','exclude_object','sheet:exclude',3],['cancel','i-cancel','CANCEL','cancel','send:cancel',4,'DH'],['more','i-more','MORE',null,'radial:more',5]],
 finished:[['reprint','i-print','PRINT AGAIN','print_start','reprint',1,'P'],['files','i-files','FILES',null,'sheet:files',1],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['home','i-home','HOME','home','menu:home',2],['more','i-more','MORE',null,'radial:more',3]]
};

export function dockFor(mode,options={}){
 if(mode==='service'||mode==='checking')return [];
 const compact=options.compact??false,maxSlots=options.maxSlots??(compact?5:8);
 const rows=ROWS[mode==='finished'&&!options.jobName?'idle':mode]||[];
 const slots=rows.map(([key,icon,label,cmd,action,group,flags=''])=>({key,icon,label,cmd,action,primary:flags.includes('P'),danger:flags.includes('D'),hold:flags.includes('H'),group,visible:true}));
 if(slots.length<=maxSlots)return slots;
 const kept=new Set(slots.filter(s=>s.key==='more'||s.primary||s.danger).map(s=>s.key));
 for(const slot of slots)if(kept.size<maxSlots)kept.add(slot.key);
 for(const slot of slots)slot.visible=kept.has(slot.key);
 return slots;
}

// Order = how often each is wanted on a small screen (a phone has no page rail, so Pages and View
// must survive the 8-item cut); the restarts come last because a halted Klipper puts them in the dock.
const MORE_ROWS=[
 ['console','i-console','CONSOLE','console','sheet:console'],
 ['macros','i-macro','MACROS','macro','sheet:macros'],
 ['recording','i-rewind','RECORDING',null,'sheet:recording'],
 ['view','i-view','VIEW',null,'sheet:view'],
 ['pages','i-nav-hud','PAGES',null,'sheet:pages'],
 ['motors_off','i-motors-off','MOTORS OFF','motors_off','send:motors_off'],
 ['fw_restart','i-fw-restart','FIRMWARE RESTART','firmware_restart','send:firmware_restart'],
 ['klipper_restart','i-restart','KLIPPER RESTART','klipper_restart','send:klipper_restart']
];
export function moreMenu(mode,dock){
 const slots=Array.isArray(dock)?dock:[];
 const visible=new Set(slots.filter(s=>s.visible).map(s=>s.key));
 const item=s=>({key:s.key,icon:s.icon,label:s.label,cmd:s.cmd,action:s.action,danger:Boolean(s.danger),hold:Boolean(s.hold)});
 const overflow=slots.filter(s=>s.visible===false&&s.key!=='more').map(item);
 const base=MORE_ROWS.map(([key,icon,label,cmd,action])=>({key,icon,label,cmd,action,danger:key==='motors_off'&&mode==='printing',hold:key==='motors_off'&&mode==='printing'}));
 // Deduplicate before the cut: an overflowed slot that is also a base item
 // must not take two of the eight places.
 const seen=new Set();
 return [...overflow,...base].filter(s=>!visible.has(s.key)&&!seen.has(s.key)&&seen.add(s.key)).slice(0,8);
}

export function heatColor(temp,target,kind){
 const t=number(temp),goal=number(target),cold=kind==='bed'?35:50,hot=kind==='bed'?60:170,danger=kind==='bed'?115:285;
 if(t===null)return {tone:'unknown',ink:'#6E7883',glow:0};
 const tone=t>danger||goal!==null&&goal>0&&t>goal+15?'danger':t<cold?'cold':t>=hot?'hot':'warm';
 const ink={cold:'#A7B0BA',warm:'#FF9C55',hot:'#FF6A00',danger:'#FF2D92'}[tone];
 return {tone,ink,glow:tone==='danger'?1:Math.round(clamp((t-cold)/(hot-cold),0,1)*1000)/1000};
}

export const DIAL={start:135,sweep:270};
export function dialFraction(value,min,max){
 const v=number(value),lo=number(min),hi=number(max);
 return v===null||lo===null||hi===null||hi<=lo?null:clamp((v-lo)/(hi-lo),0,1);
}
export function dialAngle(fraction){const f=number(fraction);return f===null?null:DIAL.start+f*DIAL.sweep;}
const rounded=v=>Number(v.toFixed(3));
export function polar(cx,cy,r,deg){
 if([cx,cy,r,deg].some(v=>number(v)===null))return null;
 const rad=deg*Math.PI/180;
 return {x:rounded(cx+r*Math.cos(rad)),y:rounded(cy+r*Math.sin(rad))};
}
export function arcPath(cx,cy,r,a0,a1){
 if([cx,cy,r,a0,a1].some(v=>number(v)===null)||a1-a0<=0)return '';
 const end=a0+Math.min(a1-a0,359.999),p0=polar(cx,cy,r,a0),p1=polar(cx,cy,r,end);
 return `M ${p0.x} ${p0.y} A ${rounded(r)} ${rounded(r)} 0 ${end-a0>180?1:0} 1 ${p1.x} ${p1.y}`;
}
export function dialModel({value,min,max,target=null,power=null}={}){
 const frac=dialFraction(value,min,max),targetFrac=dialFraction(target,min,max),p=number(power);
 return {frac,angle:dialAngle(frac),targetFrac,targetAngle:dialAngle(targetFrac),powerFrac:p===null?null:clamp(p,0,1)};
}

const shown=(v,digits=0)=>number(v)===null?'—':v.toFixed(digits);
export const fmtTemp=v=>number(v)===null?'—':`${Math.round(v)}°`;
export const fmtPct=v=>number(v)===null?'—':`${Math.round(v*100)}%`;
export const fmtSpeed=v=>number(v)===null?'—':String(Math.round(v));
export const fmtFlow=v=>shown(v,1);
export const fmtPos=v=>shown(v,1);
export function fmtDuration(seconds){
 const v=number(seconds);if(v===null)return '—';
 if(v<60)return `${Math.round(v)} s`;
 if(v<3600)return `${Math.round(v/60)} min`;
 const minutes=Math.round(v/60);
 return `${Math.floor(minutes/60)} h ${String(minutes%60).padStart(2,'0')} min`;
}
export function fileTitle(name){
 return string(name).split(/[\\/]/).pop().replace(/\.gcode$/i,'').replaceAll('_',' ').trim();
}
export function jobModel(readings){
 const job=readings?.job||{},duration=number(job.duration),progress=number(job.progress);
 return {active:['printing','paused'].includes(job.state),title:fileTitle(job.name),progress,elapsed:duration,
  remaining:progress!==null&&progress>0.01&&duration!==null?Math.round(duration/progress-duration):null,state:string(job.state)};
}
export function positionModel(readings){
 return ['x','y','z'].map(axis=>({axis:axis.toUpperCase(),text:fmtPos(readings?.position?.[axis]),homed:Boolean(readings?.homed?.[axis])}));
}
export function flowModel(readings,loadedName,measuredMax){
 const value=volumetricFlow(readings?.extruderVelocity),{max,name}=flowCeiling(loadedName,measuredMax),frac=value===null||max===null?null:value/max;
 return {value,max,name,frac,tone:frac===null?'unknown':frac>1?'over':frac>=0.85?'high':'normal'};
}

// Dock sub-menus. A slot whose action is menu:<key> opens these items above it; each item goes through
// the same command path. `script` items are sent as console lines (reviewed once); `cmd` items use
// their own command id.
export const SUB_MENUS={
 home:[{key:'home_all',icon:'i-home',label:'Home all axes',cmd:'home',action:'send:home',does:'G28: homes X, Y and Z.'},
  {key:'home_x',icon:'i-arrow-right',label:'Home X',cmd:'console',action:'script:G28 X',does:'G28 X only.'},
  {key:'home_y',icon:'i-arrow-up',label:'Home Y',cmd:'console',action:'script:G28 Y',does:'G28 Y only.'},
  {key:'home_z',icon:'i-z-down',label:'Home Z',cmd:'console',action:'script:G28 Z',does:'G28 Z only: X and Y must be homed first.'},
  {key:'motors_off',icon:'i-motors-off',label:'Motors off',cmd:'motors_off',action:'send:motors_off',does:'M84: steppers let go, the printer is no longer homed.'}],
 level:[{key:'qgl',icon:'i-level',label:'Quad gantry level',cmd:'qgl',action:'send:qgl',does:'QUAD_GANTRY_LEVEL: probes the four corners and levels the gantry.'},
  {key:'mesh',icon:'i-mesh',label:'Bed mesh',cmd:'mesh',action:'send:mesh',does:'BED_MESH_CALIBRATE: probes the bed and saves a fresh mesh.'},
  {key:'probe_accuracy',icon:'i-probe',label:'Probe accuracy',cmd:'probe_accuracy',action:'send:probe_accuracy',does:'PROBE_ACCURACY: ten probes at the centre, reports the spread.'},
  {key:'probe_query',icon:'i-probe',label:'Query probe',cmd:'probe_query',action:'send:probe_query',does:'QUERY_PROBE: is the BLTouch triggered right now?'}]
};

// Homing frame. While Klipper homes an axis it force-sets that axis to endstop - 1.5 x travel (X and
// Y: -175) or endstop + 1.5 x travel (Z: about 464) and reports positions in that frame until the
// endstop triggers, so G28 read as X -169 then 350, Y -161... 14... 154 then 344, Z 464... 440 then 2
// (recording 1790275734). The reported change is still the real travel, so a homing axis is drawn from
// where it was, moving by exactly what Klipper reports, clamped to the machine; the frame ends when
// the reported value jumps (the trigger re-sets it), turns back, or reaches the endstop. Returns a
// stateful mapper.
export function homingFrame(){const st=[null,null,null],un=[null,null,null];let prevTs=null,out=null;
 return function map(pos,{limits,velocity,ts,start,homed=null}={}){
  if(!Array.isArray(pos)||!limits?.min||!limits?.max)return pos;
  const dt=Number.isFinite(ts)&&Number.isFinite(prevTs)?Math.max(0.05,Math.abs(ts-prevTs)):0.25;prevTs=ts;let rebased='';
  const res=pos.map((v,i)=>{const lo=limits.min[i],hi=limits.max[i];if(!Number.isFinite(v)||!Number.isFinite(lo)||!Number.isFinite(hi))return v;
   const from=Number.isFinite(start?.[i])?start[i]:Number.isFinite(out?.[i])?out[i]:null;
   // An unhomed axis has no absolute position (Klipper calls it 0), but its moves are still real:
   // the Z hop before G28 raises the gantry 10 mm from wherever it hangs. So it is drawn from where
   // it was last seen, moving by what Klipper reports.
   if(typeof homed==='string'&&!homed.includes('xyz'[i])){st[i]=null;if(!un[i])un[i]={anchor:from??v,ref:v};return Math.max(lo,Math.min(hi,un[i].anchor+(v-un[i].ref)));}
   if(un[i]){un[i]=null;if(from!==null&&v>=lo-1&&v<=hi+1&&Math.abs(v-from)>1)rebased+='xyz'[i];}
   const clamp=x=>Math.max(st[i]?.dir<0?Math.min(hi,Math.max(lo,0)):lo,Math.min(hi,x));let h=st[i];
   if(h){const step=v-h.last,jump=Math.abs(step)>Math.max(Number.isFinite(velocity)?velocity:0,5)*dt*2+5;
    const back=step*h.dir<-0.5,home=v>=lo&&v<=hi&&(h.dir>0?v>=hi-0.5:v<=Math.max(lo,0)+0.5);
    if(jump||back||home){st[i]=h=null;rebased+='xyz'[i];}else{h.last=v;return clamp(h.s+(v-h.force));}}
   if(v<lo-1||v>hi+1){const dir=v<lo?1:-1,e=dir>0?hi:Math.min(hi,Math.max(lo,0));
    const force=dir>0?e-1.5*(e-lo):e+1.5*(hi-e),s=from??(lo+hi)/2;
    st[i]={dir,force,s,last:v};return clamp(s+(v-force));}
   return v;});
  out=res;res.rebased=rebased;return res;};}
