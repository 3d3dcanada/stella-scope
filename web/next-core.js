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
  job:{state:string(job?.state).toLowerCase(),name:string(job?.name),time_left:number(job?.time_left),message:string(job?.message),progress:progress===null?null:clamp(progress,0,1),duration:number(job?.duration),filament:number(job?.filament_used)},
  position:{x:number(point?.position?.[0]),y:number(point?.position?.[1]),z:number(point?.position?.[2])},
  velocity:number(point?.velocity),maxVelocity:number(point?.max_velocity),maxAccel:number(point?.max_accel),
  zOffset:number(point?.z_offset),qgl:typeof point?.qgl_applied==='boolean'?point.qgl_applied:null,
  speedFactor:number(point?.speed_factor),extrudeFactor:number(point?.extrude_factor),extruderVelocity:number(point?.extruder_velocity),
  nozzle:heater(point?.nozzle),bed:heater(point?.bed),
  fans:{part:fan(point?.fans?.fan),hotend:fan(point?.fans?.hotend_fan),controller:fan(point?.fans?.controller_fan)}
 };
}

// homing:false is a printer with no homing of its own to report (PrusaLink): it is never "not homed".
export function deriveMode({link,live,readings,homing=true}={}){
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
 else if(homing&&!['x','y','z'].every(axis=>readings?.homed?.[axis]))mode='unhomed';
 return {mode,heating,atTemp};
}

// The table is the complete display order; the final slot is always More. Flags: P primary, D danger,
// H hold to confirm, C kept on a compact dock before the others (2026-09-28: Load and Unload belong
// on the bottom bar, so on a phone they displace Level and Move into More rather than Heat and Home).
const ROWS={
 // 2026-10-25: a printer that is off showed four dead keys spread apart. It offers what still works: its camera, the prints it recorded, the alerts and its settings.
 off:[['camera','i-camera','CAMERA',null,'sheet:camera',1],['history','i-rewind','HISTORY',null,'sheet:history',1],['alerts','i-warn','ALERTS',null,'sheet:alerts',1],['printers','i-printers','PRINTER',null,'sheet:printers',1],['more','i-more','MORE',null,'radial:more',1]],
 klipper:[['fw_restart','i-fw-restart','FIRMWARE RESTART','firmware_restart','send:firmware_restart',1,'P'],['klipper_restart','i-restart','KLIPPER RESTART','klipper_restart','send:klipper_restart',1],['console','i-console','CONSOLE','console','sheet:console',2],['more','i-more','MORE',null,'radial:more',3]],
 replay:[['rec_back','i-step-back','BACK',null,'rec:back',1],['rec_play','i-play','PLAY',null,'rec:play',1,'P'],['rec_fwd','i-step-fwd','FORWARD',null,'rec:fwd',1],['recording','i-rewind','RECORDING',null,'sheet:recording',2],['more','i-more','MORE',null,'radial:more',3]],
 rewound:[['go_live','i-live','GO LIVE',null,'rec:live',1,'P'],['rec_back','i-step-back','BACK',null,'rec:back',2],['rec_play','i-play','PLAY',null,'rec:play',2],['rec_fwd','i-step-fwd','FORWARD',null,'rec:fwd',2],['recording','i-rewind','RECORDING',null,'sheet:recording',3],['more','i-more','MORE',null,'radial:more',4]],
 unhomed:[['home','i-home','HOME','home','menu:home',1,'PC'],['level','i-level','LEVEL','qgl','menu:level',1],['move','i-move','MOVE','jog','sheet:move',2],['heat','i-heat','HEAT','set_heater','sheet:heat',2,'C'],['cool','i-cool','COOL','set_heater','cool',2],['load','i-filament-load','LOAD','filament_load','filament:load',3,'C'],['unload','i-filament-unload','UNLOAD','filament_unload','filament:unload',3,'C'],['files','i-print','PRINT',null,'sheet:files',4],['macros','i-macro','MACROS','macro','sheet:macros',4],['stop','i-stop-square','STOP','set_heater','stop',5,'H'],['more','i-more','MORE',null,'radial:more',5]],
 // 2026-10-23: a ready printer leads with what it is for: PRINT first and lit, then heat and filament, then motion.
 idle:[['files','i-print','PRINT',null,'sheet:files',1,'P'],['heat','i-heat','HEAT','set_heater','sheet:heat',2,'C'],['cool','i-cool','COOL','set_heater','cool',2],['load','i-filament-load','LOAD','filament_load','filament:load',3,'C'],['unload','i-filament-unload','UNLOAD','filament_unload','filament:unload',3],['home','i-home','HOME','home','menu:home',2,'C'],['level','i-level','LEVEL','qgl','menu:level',2],['move','i-move','MOVE','jog','sheet:move',2],['macros','i-macro','MACROS','macro','sheet:macros',4],['console','i-console','CONSOLE','console','sheet:console',4],['stop','i-stop-square','STOP','set_heater','stop',5,'H'],['more','i-more','MORE',null,'radial:more',5]],
 printing:[['pause','i-pause','PAUSE','pause','send:pause',1,'P'],['speed','i-speed','SPEED','speed_factor','sheet:speed',2],['flow','i-flow','FLOW','extrude_factor','sheet:flow',2],['fan','i-fan','FAN','set_fan','sheet:fan',2],['exclude','i-exclude','EXCLUDE','exclude_object','sheet:exclude',3],['job','i-print','THE PRINT',null,'sheet:print',3],['camera','i-camera','CAMERA',null,'sheet:camera',3],['cancel','i-cancel','CANCEL','cancel','send:cancel',4,'DH'],['more','i-more','MORE',null,'radial:more',5]],
 paused:[['resume','i-play','RESUME','resume','send:resume',1,'P'],['move','i-move','MOVE','jog','sheet:move',2],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['exclude','i-exclude','EXCLUDE','exclude_object','sheet:exclude',3],['job','i-print','THE PRINT',null,'sheet:print',3],['camera','i-camera','CAMERA',null,'sheet:camera',3],['cancel','i-cancel','CANCEL','cancel','send:cancel',4,'DH'],['more','i-more','MORE',null,'radial:more',5]],
 finished:[['reprint','i-print','PRINT AGAIN','print_start','reprint',1,'P'],['files','i-files','NEW PRINT',null,'sheet:files',1],['heat','i-heat','HEAT','set_heater','sheet:heat',2],['cool','i-cool','COOL','set_heater','cool',2],['home','i-home','HOME','home','menu:home',2],['stop','i-stop-square','STOP','set_heater','stop',3,'H'],['more','i-more','MORE',null,'radial:more',3]]
};

// supports(commandId) is false for what this printer cannot do at all (farm board WO-02): that slot is
// not shown, rather than shown and failing on press. Without it every slot is offered, as before.
export function dockFor(mode,options={}){
 if(mode==='service'||mode==='checking')return [];
 const compact=options.compact??false,maxSlots=options.maxSlots??(compact?5:8),supports=options.supports??(()=>true);
 const rows=(ROWS[mode==='finished'&&!options.jobName?'idle':mode]||[]).filter(r=>!r[3]||supports(r[3]));
// 2026-10-25 audit: a printer that takes few commands (a Prusa through PrusaLink: no heat, no motion) had a bar of PRINT and MORE. A short bar is
 // filled with what every printer has: its camera, its history and the alerts.
 const FILL=[['camera','i-camera','CAMERA',null,'sheet:camera',6],['history','i-rewind','HISTORY',null,'sheet:history',6],['alerts','i-warn','ALERTS',null,'sheet:alerts',6]];
 if(!['replay','rewound','klipper'].includes(mode))for(const r of FILL){if(rows.length>=6)break;if(!rows.some(x=>x[0]===r[0]))rows.splice(rows.findIndex(x=>x[0]==='more')<0?rows.length:rows.findIndex(x=>x[0]==='more'),0,r);}
 const slots=rows.map(([key,icon,label,cmd,action,group,flags=''])=>({key,icon,label,cmd,action,primary:flags.includes('P'),danger:flags.includes('D'),hold:flags.includes('H'),keep:flags.includes('C'),group,visible:true}));
 if(slots.length<=maxSlots)return slots;
 const kept=new Set(slots.filter(s=>s.key==='more'||s.primary||s.danger||(compact&&s.keep)).map(s=>s.key));
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
 ['look','i-colors','LOOK',null,'sheet:look'],
 ['calibrate','i-probe','CALIBRATE',null,'sheet:calibrate'],
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
 // Deduplicate before the cut (NEXT review 2026-09-24): an overflowed slot that is also a base item
 // must not take two of the eight places.
 const seen=new Set();
 // the wheel pages round, so More is no longer cut at eight (2026-10-24, wheels); twelve keeps it to two pages
 return [...overflow,...base].filter(s=>!visible.has(s.key)&&!seen.has(s.key)&&seen.add(s.key)).slice(0,12);
}

// Over temperature is a trend, not a number (2026-10-22: lowering the nozzle target from 250 to 45 raised "Nozzle over temperature" while the heater was simply cooling).
// The absolute limit (nozzle 285, bed 115) is an instant alarm. "Above its target" (more than 15 over) is only an alarm when the temperature is rising, or has not dropped for
// 30 s; a heater that is falling toward a lowered target is cooling, never in danger. The trend is the last 30 s of readings (heatTrend), fed by the page through trackHeat.
export const HEAT_HOLD_S=30;
export function heatTrend(kind){
 const thr=kind==='bed'?.3:1,win=[];let last=-Infinity,since=null,peak=null,dropAt=null;
 return {
  // one reading: its time in seconds, the temperature and the heater's target. A reading no newer than the last one is ignored.
  // Over its target, the clock of "not dropping" starts at the first reading over, and restarts whenever the temperature has fallen a step (thr) below the highest it has been since.
  // Noise inside a step never restarts it; a steady cool-down restarts it every few seconds.
  push(ts,temp,target){const t=number(temp),goal=number(target);if(number(ts)===null||ts<=last)return;last=ts;
   if(t===null){win.length=0;since=peak=dropAt=null;return;}
   win.push([ts,t]);while(win.length&&win[0][0]<ts-HEAT_HOLD_S)win.shift();
   if(!(goal!==null&&goal>0&&t>goal+15)){since=peak=dropAt=null;return;}
   if(since===null){since=ts;peak=t;}else if(t<=peak-thr){since=ts;peak=t;dropAt=ts;}else peak=Math.max(peak,t);},
  read(){if(!win.length)return {rising:false,falling:false,heldFor:0};const [t0,v0]=win[0],[t1,v1]=win[win.length-1];
   return {rising:t1-t0>=5&&v1-v0>=thr,falling:dropAt!==null&&t1-dropAt<=15,heldFor:since===null?0:t1-since};},
  reset(){win.length=0;since=peak=dropAt=null;last=-Infinity;}};
}
const TRENDS={nozzle:heatTrend('nozzle'),bed:heatTrend('bed')};
export const trackHeat=(kind,ts,temp,target)=>TRENDS[kind]?.push(ts,temp,target);
export function heatColor(temp,target,kind,trend=TRENDS[kind]?.read()){
 const t=number(temp),goal=number(target),cold=kind==='bed'?35:50,hot=kind==='bed'?60:170,danger=kind==='bed'?115:285;
 if(t===null)return {tone:'unknown',ink:'#6E7883',glow:0};
 const over=goal!==null&&goal>0&&t>goal+15,stuck=over&&!!trend&&(trend.rising===true||trend.heldFor>=HEAT_HOLD_S);
 const tone=t>danger||stuck?'danger':over&&trend?.falling?'cooling':t<cold?'cold':t>=hot?'hot':'warm';
 const ink={cold:'#A7B0BA',warm:'#FF9C55',hot:'#FF6A00',cooling:'#FF9C55',danger:'#FF2D92'}[tone];
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
// finish-2 F07 (2026-10-19): the name a screen reader gives a button must contain the words on it (the plate check button said "Plate check" and
// showed "Set up the plate check"). A tooltip title that already holds the visible label is the better, longer name; otherwise the label is the name.
export function accessibleName(label,title){
 const l=String(label||'').trim(),t=String(title||'').trim();
 return !l?t:!t?l:t.toLowerCase().includes(l.toLowerCase())?t:l;
}
// finish-2 F07 (2026-10-19): the chip's own word for Stella Scope not answering was SERVICE OFF (a developer word; hud-core.js keeps the old label
// the tests and the classic page use). And "REWOUND 6 days" read as a time travelled: the time since the printer last answered is only shown
// on the live page, and always as "6 days ago".
// hud-core.js keeps the old sentence and label (its tests, the classic page); the NEXT page says Stella Scope, not "service".
const SERVICE_REASON='Stella Scope service is not running: this page cannot reach it, so nothing new arrives and no command can go. The page tries again by itself and picks up at once when it answers.';
export const plainState=text=>String(text??'').replace('SERVICE OFF','STELLA SCOPE OFF').replace(SERVICE_REASON,'Stella Scope is not answering: this page cannot reach it, so nothing new arrives and no control can be used. The page tries again by itself and carries on as soon as it answers.');
export const sinceText=(link,live=true)=>live&&['off','service'].includes(link?.state)&&link?.since?link.since+' ago':'';
export function fileTitle(name){
 // 2026-10-23 (a live ring print): Orca and PrusaSlicer append "_0.2mm_PETG_Generic Klipper Printer_30m32s"; the title is the part, not the slicer's stamp.
 return string(name).split(/[\\/]/).pop().replace(/\.(gcode\.3mf|bgcode|gcode|3mf)$/i,'').replace(/_\d+(?:\.\d+)?mm_.+?_(?=\d)(?:\d+d)?(?:\d+h)?(?:\d+m)?(?:\d+s)?$/i,'').replaceAll('_',' ').trim();
}
export function jobModel(readings){
 const job=readings?.job||{},duration=number(job.duration),progress=number(job.progress);
 // warming: printing by state, but no print time yet. Klipper counts print_duration from the first extrusion, so this is the
 // start macro heating, soaking, homing and levelling (2026-10-23: it read "layer 1 of 25 · 1%" through a ten minute soak).
 return {active:['printing','paused'].includes(job.state),warming:job.state==='printing'&&duration!==null&&duration<1,title:fileTitle(job.name),progress,elapsed:duration,
  // finish-2 F02: the printer's own time left first (PrusaLink states it); worked out from progress it counted heating as printing
  remaining:number(job.time_left)!==null?Math.round(number(job.time_left)):progress!==null&&progress>0.01&&duration!==null?Math.round(duration/progress-duration):null,state:string(job.state)};
}
export function positionModel(readings){
 return ['x','y','z'].map(axis=>({axis:axis.toUpperCase(),text:fmtPos(readings?.position?.[axis]),homed:Boolean(readings?.homed?.[axis])}));
}
export function flowModel(readings,loadedName,measuredMax){
 const value=volumetricFlow(readings?.extruderVelocity),{max,name}=flowCeiling(loadedName,measuredMax),frac=value===null||max===null?null:value/max;
 return {value,max,name,frac,tone:frac===null?'unknown':frac>1?'over':frac>=0.85?'high':'normal'};
}

// Dock sub-menus (NEXT review 2026-09-24: "Is HOME clickable? Does it have more things?"). A slot
// whose action is menu:<key> opens these items above it; each item goes through the same command path.
// `script` items are sent as console lines (reviewed once); `cmd` items use their own command id.
export const SUB_MENUS={
 home:[{key:'home_all',icon:'i-home',label:'Home all axes',cmd:'home',action:'send:home',does:'Homes X, Y and Z.'},
  {key:'home_x',icon:'i-arrow-right',label:'Home X',cmd:'console',action:'script:G28 X',does:'Homes X only.'},
  {key:'home_y',icon:'i-arrow-up',label:'Home Y',cmd:'console',action:'script:G28 Y',does:'Homes Y only.'},
  {key:'home_z',icon:'i-z-down',label:'Home Z',cmd:'console',action:'script:G28 Z',does:'Homes Z only. X and Y must be homed first.'},
  {key:'motors_off',icon:'i-motors-off',label:'Motors off',cmd:'motors_off',action:'send:motors_off',does:'Lets the motors go. The printer is no longer homed.'}],
 level:[{key:'qgl',icon:'i-level',label:'Quad gantry level',cmd:'qgl',action:'send:qgl',does:'Probes the four corners and levels the gantry.'},
  {key:'mesh',icon:'i-mesh',label:'Bed mesh',cmd:'mesh',action:'send:mesh',does:'Probes the bed and saves a fresh mesh.'},
  {key:'probe_accuracy',icon:'i-probe',label:'Probe accuracy',cmd:'probe_accuracy',action:'send:probe_accuracy',does:'Probes the centre ten times and reports the spread.'},
  {key:'probe_query',icon:'i-probe',label:'Check probe',cmd:'probe_query',action:'send:probe_query',does:'Asks whether the BLTouch probe is triggered right now.'}]
};

// Homing frame (2026-09-24: "the printhead janked big time when I pressed Home"). While Klipper
// homes an axis it force-sets that axis to endstop - 1.5 x travel (X and Y: -175) or endstop + 1.5 x
// travel (Z: about 464) and reports positions in that frame until the endstop triggers, so G28 read
// as X -169 then 350, Y -161 ... 14 ... 154 then 344, Z 464 ... 440 then 2 (recording 1790275734).
// The reported change is still the real travel, so a homing axis is drawn from where it was, moving by
// exactly what Klipper reports, clamped to the machine; the frame ends when the reported value jumps
// (the trigger re-sets it), turns back, or reaches the endstop. Returns a stateful mapper.
export function homingFrame(){const st=[null,null,null],un=[null,null,null];let prevTs=null,out=null;
 return function map(pos,{limits,velocity,ts,start,homed=null}={}){
  if(!Array.isArray(pos)||!limits?.min||!limits?.max)return pos;
  const dt=Number.isFinite(ts)&&Number.isFinite(prevTs)?Math.max(0.05,Math.abs(ts-prevTs)):0.25;prevTs=ts;let rebased='';
  const res=pos.map((v,i)=>{const lo=limits.min[i],hi=limits.max[i];if(!Number.isFinite(v)||!Number.isFinite(lo)||!Number.isFinite(hi))return v;
   const from=Number.isFinite(start?.[i])?start[i]:Number.isFinite(out?.[i])?out[i]:null;
   // An unhomed axis has no absolute position (Klipper calls it 0), but its moves are still real:
   // the Z hop before G28 raises the gantry 10 mm from wherever it hangs. So it is drawn from where
   // it was last seen, moving by what Klipper reports (2026-09-24: Z sat at the bottom, then
   // stepped up to the probe height).
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

// Driver flags worth a popup. On the TMC2209 the open-load bits (ola, olb) only mean something while
// the motor moves; at standstill (stst=1) they read 1 spuriously. 2026-09-28: "stepper_x TMC2209
// reports OLB" with the printer ready and parked, and "We can't have things like this popping up".
// So ola/olb count only for a motor that is not at standstill while the machine is moving; every other
// flag is kept as it was. Mirrors model::reportable_flags in the service.
export function driverFlags(motor,moving=false){
 const flags=Array.isArray(motor?.flags)?motor.flags.filter(f=>typeof f==='string'):[];
 // Finish-2 F05 (2026-10-19): Stella's three Z motors raised "reports olb" pop-ups during a normal PETG print (slow Z moves).
 // Open load is never raised by itself, as the collector already decides for alerts: TMC2209s report it falsely at standstill
 // and on slow moves, and Klipper does not treat it as a fault. The motor page still shows every flag.
 return flags.filter(f=>f!=='ola'&&f!=='olb');
}

// Filament job. The macros move 50 mm (load: purge) and 100 mm (unload: retract) on our printer
// (LOAD_MM / UNLOAD_MM in commands.rs, from Stella's own macros), so progress is the filament seen
// moving against that. The step list is data so the page can build it once and only flip states.
// Filament board 2026-10-22 (the owner): the job now says every step it is on. A load: heat, push it in, feed, cool (the heater goes off at once), shake (a quick flick of the head to
// shake the filament tail off), done. An unload: heat, pull out, wait for a reload (the heater stays on for a time you set), cool (it goes off by itself), done.
export const FILAMENT={
 load:{mm:50,steps:[['heating','Heat'],['ready','Push it in'],['running','Feed'],['cooling','Cool'],['shaking','Shake'],['done','Done']]},
 unload:{mm:100,steps:[['heating','Heat'],['running','Pull out'],['waiting','Wait'],['cooling','Cool'],['done','Done']]}
};
export function filamentSteps(kind,step){
 const spec=FILAMENT[kind==='load'?'load':'unload'].steps;
 const at=step==='confirmed'?spec.length:spec.findIndex(s=>s[0]===step);
 return spec.map(([key,label],i)=>({key,label,n:i+1,state:at<0?'':i<at?'done':i===at?'now':''}));
}
// Nothing is recorded as loaded or unloaded until the macro has actually been sent and has returned.
// (Filament board 2026-10-22: from the moment the macro has returned, through the wait, the cool-down and the shake, to the end.)
export function filamentCanRecord(job){return ['waiting','cooling','shaking','done'].includes(job?.step)&&job?.ran===true;}
// The settings (kept on this device): how long an unloaded printer waits for a reload before the heater goes off, how long after a load before the shake, and whether it shakes.
export const FILAMENT_DEFAULTS={wait:120,pause:4,shake:true};
export function filamentSettings(raw){const r=raw&&typeof raw==='object'?raw:{},n=(v,lo,hi,d)=>{if(v===null||v===undefined||v==='')return d;v=Number(v);return Number.isFinite(v)?clamp(Math.round(v),lo,hi):d;};
 return {wait:n(r.wait,10,600,FILAMENT_DEFAULTS.wait),pause:n(r.pause,1,15,FILAMENT_DEFAULTS.pause),shake:r.shake===false?false:FILAMENT_DEFAULTS.shake};}
// Whether the shake may run now, and why not in plain words. Never during a print or a pause, only with every axis homed, and only if the setting is on.
export function filamentShakeVerdict({enabled=true,homed,mode,allowed=true}={}){
 if(!enabled)return {ok:false,why:'The shake is switched off in the settings.'};
 if(mode==='printing'||mode==='paused')return {ok:false,why:'A print is running or paused, so the head is not shaken.'};
 if(!(homed?.x&&homed?.y&&homed?.z))return {ok:false,why:'The printer is not homed, so the head is not shaken.'};
 if(!allowed)return {ok:false,why:'This printer does not take the shake.'};
 return {ok:true,why:''};}
// 0..1 for the ring: heat by temperature, the feed by filament seen moving against what the macro moves, the wait by the time gone, the cool-down by the nozzle.
export function filamentProgress(job,nozzle,now=Date.now()){
 const step=job?.step,kind=job?.kind==='load'?'load':'unload';
 if(step==='heating'){const t=number(nozzle),goal=number(job?.t);return t===null||!goal?0:clamp(t/goal,0,1);}
 if(step==='running')return clamp((number(job?.moved)||0)/FILAMENT[kind].mm,0,0.99);
 if(step==='waiting'){const total=number(job?.waitMs),until=number(job?.until);return !total||until===null?0:clamp(1-(until-now)/total,0,1);}
 if(step==='cooling'||step==='shaking'){const t=number(nozzle),from=number(job?.t);return t===null||!from||from<=60?0:clamp((from-t)/(from-60),0,1);}
 return step==='done'||step==='confirmed'?1:0;
}

// What the service's /api/controls says each command can do on this printer. `supported:false` is a
// printer that cannot do it at all (hide it); `allowed:false` is a printer that cannot do it right now
// (lock it, with the reason). Before the answer arrives everything counts as supported.
export function supportsFrom(controls){
 const cmds=Array.isArray(controls?.commands)?controls.commands:[];
 return id=>{const c=cmds.find(x=>x?.id===id);return !c||c.supported!==false;};
}

// The farm wall offers the demo farm when there is nothing real to look at yet (polish WO-02, 2026-10-18):
// no printers at all, or only ones that have not answered once since this service started, and no demo
// printer already there. A printer that answered and then went off is a printer that is off, not a farm
// that was never set up, so it does not count.
export function noPrinterYet(printers){
 const list=Array.isArray(printers)?printers:[];
 return list.length===0||(!list.some(p=>p?.driver==='demo')&&list.every(p=>p?.state==='offline'&&(p.tick_age_s===null||p.tick_age_s===undefined)));
}

// The command bar's list. Polish WO-06 (2026-10-18): typing "files" and pressing Enter sent that word to a
// demo printer as G-code. The bar now only offers what the printer or the app already knows: actions, your history,
// the printer's own commands and your macros. Text that matches none of them is never offered as a line to send;
// typed G-code belongs in the Console, so the list points there (or says nothing matches, on a printer with no
// Console). A line that starts with a command the printer itself lists still goes out, with its arguments.
// F07 (2026-10-19): the other names of an action count as whole words ("move" is not "remove"): every typed word must begin one of them.
const wordScore=(q,words)=>{const w=String(words||'').toLowerCase().split(/\s+/).filter(Boolean),t=String(q).split(/\s+/).filter(Boolean);return t.length&&t.every(x=>w.some(y=>y.startsWith(x)))?1.2:0;};
const paletteScore=(q,s)=>{s=String(s).toLowerCase();if(!q)return 1;if(s.startsWith(q))return 3;if(s.includes(q))return 2;return 0;};
export function paletteItems(data,text,{console:hasConsole=true}={}){
 const typed=String(text||'').trim(),q=typed.toLowerCase(),first=q.split(/\s+/)[0]||'';
 const pool=[...(data.actions||[]).map(a=>({...a,kind:'action'})),...(data.history||[]).map(h=>({label:h,kind:'history',script:h})),...(data.commands||[]).map(c=>({label:c.name,help:c.help,kind:c.macro?'macro':'gcode',script:c.name}))];
 const seen=new Set();
 const items=pool.map(it=>({it,s:Math.max(paletteScore(first,it.label),paletteScore(q,it.label)*1.1,it.help?paletteScore(q,it.help)*.5:0,it.words&&q?wordScore(q,it.words):0)})).filter(x=>x.s>0&&!seen.has(x.it.kind+x.it.label)&&seen.add(x.it.kind+x.it.label))
  .sort((a,b)=>b.s-a.s||((b.it.kind==='action')-(a.it.kind==='action'))).slice(0,9).map(x=>x.it);
 // F07 (2026-10-19): a name that is a control this printer cannot do says so, instead of "nothing matches"
 const lack=q?(data.lacking||[]).filter(c=>paletteScore(q,c.label)||wordScore(q,c.words)).slice(0,2):[];
 for(const c of lack)if(!items.some(i=>i.label===c.label))items.push({label:c.label+': not on this printer',kind:'none',help:''});
 if(typed&&!items.length)items.push(hasConsole?{label:`Open the Console for “${typed}”`,kind:'action',icon:'i-console',consoleText:typed,help:'typed G-code is sent from the Console, not from here'}:{label:`Nothing matches “${typed}”`,kind:'none',help:'pick a command from the list'});
 return items;}


// finish-2 F07 (2026-10-19): the command bar finds every control a person can press by the plain name it carries on screen (it found nothing
// for failure watch, view, speed, flow, move or motors). `words` are the other names people say; `needs` is the printer capability the control
// needs (a printer that cannot do it is not offered it); `when` limits it to the print states where it means something. `how`: sheet:<name>
// opens that sheet (next.js openNamedSheet), cmd:<id> sends that command (always shown for review first), href:<path> goes there, rec:live goes live.
// The emergency stop is not in the list: it is one press on a red button and never one Enter in a search.
const C=(label,icon,how,words='',help='',needs=null,when=null)=>({label,icon,how,words,help,needs,when});
export const CONTROLS=[
 C('Home all axes','i-home','cmd:home','home homing reset start position','send the head home','home'),
 C('Heat','i-heat','sheet:heat','nozzle bed temperature preheat cool warm hotend','nozzle and bed temperatures','heat'),
 C('Move','i-move','sheet:move','jog head axis nudge x y z position','move the head by hand','jog'),
 C('Things to print','i-print','sheet:files','files print start slice job gcode','pick a file and print it'),
 C('Speed','i-speed','sheet:speed','faster slower percent feed rate factor','faster or slower moves','speed'),
 C('Flow','i-flow','sheet:flow','extrusion plastic percent factor','how much plastic moves push out','flow'),
 C('Fans','i-fan','sheet:fan','part cooling hotend electronics bay fan speed','part cooling and the others','fan'),
 C('Queue','i-files','sheet:queue','jobs next file waiting plate check','sliced files waiting for a printer'),
 C('Camera','i-camera','sheet:camera','webcam picture video live','the live camera'),
 C('Failure watch','i-view','sheet:view','spaghetti detector failed print watch obico camera','switch it on or off under View'),
 C('View','i-view','sheet:view','angle iso front top side full screen kiosk knobs bars instrument style panels see-through mesh','angles, full screen, style'),
 C('Pair phone','i-phone','sheet:phone','qr app whatsapp alerts notifications ntfy phone','phone, WhatsApp and alerts'),
 C('Pages','i-more','sheet:pages','all pages list menu index','every page of this printer'),
 C('Help','i-help','href:/help','guide manual how to user guide documentation instructions','the user guide'),
 C('Recording','i-rewind','sheet:recording','rewind history back play timeline replay past','go back in time'),
 C('Console','i-console','sheet:console','gcode g-code terminal type command log','type G-code, see the replies','console'),
 C('Macros','i-macro','sheet:macros','macro script klipper','your macros','macros'),
 C('Exclude objects','i-exclude','sheet:exclude','skip object part leave out','skip one object of this print','klipper',['printing','paused']),
 C('Load filament','i-filament-load','sheet:filament','load feed insert push in','load a material','filament'),
 C('Unload filament','i-filament-unload','sheet:filament','unload remove pull out retract','unload a material','filament'),
 C('Quad gantry level','i-level','cmd:qgl','level gantry qgl','level the gantry','level'),
 C('Bed mesh','i-mesh','cmd:mesh','probe bed level calibrate','probe the bed','mesh'),
 C('Motors off','i-motors-off','cmd:motors_off','motors steppers release disable unlock','let the motors go','motors_off'),
 C('Firmware restart','i-fw-restart','cmd:firmware_restart','restart reboot klipper halted','restart the firmware','restart'),
 C('Pause print','i-pause','cmd:pause','pause hold','pause the running print','pause_resume',['printing']),
 C('Resume print','i-play','cmd:resume','resume continue carry on','carry on printing','pause_resume',['paused']),
 C('Cancel print','i-cancel','cmd:cancel','stop abort end kill','stop this print','cancel',['printing','paused']),
 C('Print again','i-print','reprint','repeat reprint again same file','start the same file again','start_print',['finished']),
 C('Go live','i-live','rec:live','now current live','back to the live printer',null,['rewound'])
];
// The controls a printer cannot do at all, valid for the print state: typing one of their names says so ("Motors off: not on this printer") instead of finding nothing.
export function controlsLacking({has=()=>true,mode=''}={}){
 return CONTROLS.filter(c=>c.needs&&!has(c.needs)&&(!c.when||c.when.includes(mode)));
}
// The controls this printer offers right now. has(flag) is the page's capability test (a printer that does not say is assumed to).
export function controlItems({has=()=>true,mode=''}={}){
 return CONTROLS.filter(c=>(!c.needs||has(c.needs))&&(!c.when||c.when.includes(mode)));
}
// F06 (2026-10-19): the bar answered "Nothing matches" to history, farm, queue, printers or camera. Each page of the printer the person is on, except the one they are on,
// is an action that opens it; a page that needs something this printer lacks (Klipper's motion and signals on a Prusa) is not offered.
export function pageActions(pages,{has=()=>true,needs={}}={}){
 return (pages||[]).filter(p=>p[2]!=='/'&&(!needs[String(p[2]).slice(6)]||has(needs[String(p[2]).slice(6)]))).map(p=>({label:p[1],icon:p[0],page:p[2],help:'opens this page'}));}

// F06 (2026-10-19): the service's reasons for not sending a command are written for a log ("printer state is printing/printing; T3 commands are blocked while printing
// or paused"). The page says what a person can do about it. The exact words stay in the service (its tests and audit hold them).
export function plainRefusal(text){
 const t=String(text||'');let m;
 if(/T3 commands are blocked while printing or paused/.test(t))return 'That cannot be done while a print is running or paused. Pause or cancel the print first.';
 if(m=/all axes must be homed before ([a-z_ ]+)/i.exec(t))return 'The printer has to be homed first. Press Home, then try again.';
 if(/is not reported by current G-code help/.test(t))return 'This printer does not offer that command, so nothing was sent.';
 if(m=/Klippy state is (\w+), not ready/.exec(t))return `The printer is not ready (it says: ${m[1]}), so nothing was sent.`;
 if(/unknown print\/idle state/.test(t))return 'The printer\'s state is not clear right now, so nothing was sent. Try again in a moment.';
 return t;}

// F06 (2026-10-19): what a found printer is called in the list. A Prusa that has not given its name (its login comes first) was just "PrusaLink", the name of the software it runs,
// so a person with two could not tell which was theirs.
const prettyName=s=>String(s||'').replace(/^flashforge\s+/i,'').replace(/[-_]+/g,' ').replace(/\b[a-z]/g,c=>c.toUpperCase()).trim();
export const hitTitle=h=>prettyName(h.driver==='flashforge'?h.model||h.name:h.name||h.model)||(h.driver==='prusalink'?'A Prusa printer':h.brand);
// What a printer's own state word says, in the words the farm uses (the connection test showed Klipper's raw "cancelled" and "standby").
const SAMPLE_STATE={standby:'Ready',ready:'Ready',idle:'Ready',printing:'Printing',paused:'Paused',complete:'Finished',finished:'Finished',cancelled:'Ready (the last print was cancelled)',error:'Needs attention'};
export const sampleStateWords=s=>SAMPLE_STATE[String(s||'').toLowerCase()]||prettyName(s);
// The queue's "send to a different model anyway" choice is for a file made for another machine. It showed on a file that fits, where it means nothing and reads as if something were
// wrong. It shows when a printer is held back for being another machine, or when it is already ticked (so it can be undone).
export const showForceModel=j=>!!(j&&(j.force_model||(j.fits||[]).some(f=>(f.reasons||[]).some(r=>/different machine/.test(r)))));

// Words that name the engine (polish WO-06 item 8, README rule 8, 2026-10-18): Klipper's own only on a Klipper
// printer. A demo, a Prusa or a FlashForge reads the plain ones.
export function printerWords(klipper){
 return klipper===false
  ?{position:'Where the head is, in mm.',estop:'Stops the printer at once. No review. Heaters and motors cut out; start the printer again afterwards.',fans:'Speeds are what the printer was told to run.'}
  :{position:'Where Klipper was told to put the head, in mm, smoothed between readings. A green axis letter means that axis is homed.',estop:'Stops Klipper at once. No review. Heaters and motors cut out; the printer then needs a firmware restart.',fans:'Speeds are what Klipper was told to run, not a measured speed.'};}

// UI board U07 (2026-10-20): the failure alarm takes the screen. What it says and where it draws its boxes, with nothing from the page in it so it is node-tested.
// The failure watch (stella-eye) sends corners (x1 y1 x2 y2). They are 0 to 1 of the picture; a model that gives the 416 px grid it was fed (any corner past 1.5) is scaled down.
// Not verified against a real detection (no model on the machine this was built on): a box that comes out the wrong way round is swapped, one with no area is dropped.
export function alarmBoxes(detail){
 const list=(Array.isArray(detail?.boxes)?detail.boxes:[]).map(b=>({c:Number(b?.conf),b:Array.isArray(b?.box)?b.box.map(Number):null})).filter(x=>x.b&&x.b.length===4&&x.b.every(Number.isFinite));
 const m=Math.max(0,...list.flatMap(x=>x.b)),s=m>1.5?416:1,cl=v=>Math.min(1,Math.max(0,v/s));
 return list.map(({c,b})=>({pct:Number.isFinite(c)?Math.round(Math.min(1,Math.max(0,c))*100):null,x1:cl(Math.min(b[0],b[2])),y1:cl(Math.min(b[1],b[3])),x2:cl(Math.max(b[0],b[2])),y2:cl(Math.max(b[1],b[3]))}))
  .filter(b=>(b.x2-b.x1)*(b.y2-b.y1)>=.0004).sort((a,b)=>(b.pct??0)-(a.pct??0)).slice(0,3);}
export function alarmWords(e,printer){
 const name=e?.name||printer?.name||'The printer',t=String(e?.text||''),pct=(t.match(/(\d{1,3})\s*%/)||[])[1],verdict=e?.detail?.verdict||(/FAILING/.test(t)?'failing':'warning');
 const sure=pct?` (${pct}%)`:'',still='The print is still running; nothing has been stopped.';
 return verdict==='failing'
  ?{kicker:'FAILURE WATCH',title:`${name}: this print may have failed`,body:`The failure watch sees what looks like spaghetti or loose plastic${sure}. ${still}`}
  :{kicker:'FAILURE WATCH',title:`${name}: something looks wrong with this print`,body:`The failure watch sees something odd${sure}. ${still}`};}

// A key that has to be held (2026-10-22: CANCEL did nothing on a tap and said nothing). From the first touch a line says what holding does; letting go early says it was not sent.
export const HOLD_NOT_SENT='Not sent. Hold the key until the ring is full.';
export function holdWords(key,label=''){const known={cancel:'Hold to cancel the print',motors_off:'Hold to switch the motors off'};return known[key]||`Hold to ${String(label||'confirm').toLowerCase()}`;}
// The HUD says a print is cancelled only when the printer says so: print_stats.state reads cancelled and both heater targets read 0. `waited` is the seconds since the cancel was sent.
export function cancelVerdict(r,waited=0){const st=r?.job?.state,nt=number(r?.nozzle?.target),bt=number(r?.bed?.target),hot=(nt!==null&&nt>0)||(bt!==null&&bt>0);
 if(st==='cancelled'&&!hot)return {state:'ok',words:'Print cancelled, heaters off'};
 if(st==='cancelled')return waited<8?{state:'wait',words:''}:{state:'warn',words:`Print cancelled, but a heater is still on (nozzle ${nt??'?'}°, bed ${bt??'?'}°). Turn it off from Heat.`};
 return waited<20?{state:'wait',words:''}:{state:'fail',words:`The printer has not cancelled the print: it still says ${st?'"'+st+'"':'nothing'}. Nothing more was sent. Look at the printer's console.`};}
