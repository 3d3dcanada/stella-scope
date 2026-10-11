// Shared evidence rules for the landing HUD and its focused checks.
export const finite=v=>typeof v==='number'&&Number.isFinite(v)?v:null;
export function jobDisplay(job,associatedName){
 const matched=Boolean(associatedName&&job?.name===associatedName);
 const progress=matched&&finite(job?.progress)!==null?Math.max(0,Math.min(1,job.progress)):null;
 const state=matched?job.state:'preview';
 return {matched,progress,state,color:state==='error'?0xff2d92:state==='paused'?0xffac45:state==='complete'?0x18e36b:state==='printing'?0x04d9c4:0x819ba8};
}
export function preflight(snapshot,observer){
 const recorded=observer?.mode==='replay'||snapshot?.observer_now?.mode==='replay';
 const p=snapshot?.point,age=finite(snapshot?.age_seconds),homed=p?.homed||'';
 const mesh=p?.mesh?.grid,knownMesh=Array.isArray(mesh)&&mesh.length>0&&mesh.every(r=>Array.isArray(r)&&r.length>0&&r.every(v=>finite(v)!==null));
 return [
  {id:'connection',name:'Recorder & printer',state:!p?'unknown':recorded||observer?.connected===false||age!==null&&age>3?'review':'observed',detail:recorded?'Saved recording; current printer readiness is not established':!p?'No recorded sample':age===null?'Sample age unknown':`${age.toFixed(1)} s since sample`},
  {id:'home',name:'Home axes',state:!p?'unknown':['x','y','z'].every(a=>homed.includes(a))?'observed':'review',detail:homed?`${homed.toUpperCase()} reported homed`:'Homing not established',command:'home'},
  {id:'qgl',name:'Level the gantry',state:p?.qgl_applied===true?'observed':p?.qgl_applied===false?'review':'unknown',detail:p?.qgl_applied===true?'Klipper reports QGL applied':'No applied QGL in selected sample',command:'qgl'},
  {id:'mesh',name:'Inspect bed mesh',state:knownMesh?'observed':'unknown',detail:knownMesh?`${mesh.length} × ${mesh[0].length} recorded points`:'No usable recorded mesh',command:'mesh'},
  {id:'probe',name:'Probe repeatability',state:'review',detail:'Review measured accuracy before relying on a new setup',command:'probe_accuracy'},
  {id:'resonance',name:'Resonance & shaping',state:'review',detail:'Inspect results; rerun after relevant mechanical changes',command:'resonance_x'}
 ];
}
export function installationAge(record,at){
 if(record?.state!=='installed'||finite(record.effective_at)===null||finite(at)===null||at<record.effective_at)return null;
 return Math.floor((at-record.effective_at)/86400);
}

// The gantry still flew to the top. Klipper force-sets the kinematic position before a homing move
// (Z 464.0195 against axis_maximum 310 in the recording of their cube print), so a reported pose
// can be a place the machine cannot be. tested that in hud-scene set, but the live path
// interpolates between two recorded samples BEFORE set sees anything, and every blend between Z 10
// and Z 464 is inside the machine, so the guard passed it. The test lives here so the jitter buffer
// in service.js can drop the impossible sample before it can be interpolated through, and set can
// still refuse one that reaches it another way. 1 mm of slack keeps a legitimate move to the very
// end of travel drawable; unknown limits mean no opinion.
export function insideTravel(position,limits){
 if(!Array.isArray(position)||!position.every(v=>typeof v==='number'&&Number.isFinite(v)))return false;
 if(!Array.isArray(limits?.min)||!Array.isArray(limits?.max))return true;
 return !position.some((v,i)=>finite(limits.min[i])!==null&&finite(limits.max[i])!==null&&(v<limits.min[i]-1||v>limits.max[i]+1));
}

// Namespaced, reversible mapping from immutable CAD instance paths into existing parts-book IDs.
export const instanceRecordId=id=>'cad:'+id.replaceAll('/','.');

// The user, clicking parts in their own window: "I clicked Raw Signals on the extruder and got
// controller fan RPM and speed, extruder, fan, heater fan, probe, TMC... there shouldn't be
// signals from something that's not there." This is the one map every part card reads: what each
// part is physically wired to or measured on, and nothing else. A part matches on its scene slot
// and name; fields are exact names, a RegExp, or null for the whole object. A part that matches
// nothing (extrusions, printed brackets, the cowl, the LCD, whose display_status the recorder does
// not keep) gets no signals card at all. The four Z motors share one slot, so the driver comes
// from which corner of the bed the motor sits at, in service-core's CORNERS order: front left
// stepper_z, back left z1, back right z2, front right z3.
const Z_AT=['stepper_z','stepper_z3','stepper_z1','stepper_z2'];
const LINK=/^(last_stats\.|mcu_version$)/;
export const PART_WIRING=[
 {key:'extruder',label:'Extruder drive',slot:/^head\.(extruder|feeder|feeder_motor)$/,signals:[['tmc2209 extruder',null],['extruder',['can_extrude','motion_queue','pressure_advance','smooth_time']],['motion_report',['live_extruder_velocity','live_position.3']]]},
 {key:'hotend',label:'Hotend heater and thermistor',slot:/^head\.(hotend|nozzle|core|block|cartridge|heatbreak|heatsink|sock|thermistor)$/,signals:[['extruder',['temperature','target','power']]]},
 {key:'hotend_fan',label:'Hotend fan',slot:/^head\.(hotend_fan|hotendfan)$/,signals:[['heater_fan hotend_fan',null]]},
 {key:'part_fan',label:'Part cooling fan',slot:/^head\.(part_fan|partfan)$/,signals:[['fan',null]]},
 {key:'probe',label:'Probe',slot:/^head\.(probe|bltouch)$/,signals:[['probe',null]]},
 {key:'toolboard',label:'Toolhead board',slot:/^head\.board$/,signals:[['mcu EBBCan',LINK]]},
 {key:'bed',label:'Bed heater and mesh',slot:/^bed\.(plate|sheet|heater)$/,signals:[['heater_bed',null],['bed_mesh',null]]},
 {key:'stepper_x',label:'B motor, stepper_x',slot:/^drive_b$/,name:/nema|stepper/i,signals:[['tmc2209 stepper_x',null]]},
 {key:'stepper_y',label:'A motor, stepper_y',slot:/^drive_a$/,name:/nema|stepper/i,signals:[['tmc2209 stepper_y',null]]},
 {key:'z_motor',label:'Z motor',slot:/^z_drive$/,name:/nema|stepper/i,signals:[]},
 {key:'controller',label:'Main controller board',slot:/^electronics$/,name:/^(octopus|spider)\b(?!.*bracket)/i,signals:[['mcu',LINK]]},
 {key:'host',label:'Raspberry Pi host',name:/^raspberry[ _]pi/i,signals:[['host',null]]},
 {key:'bed_relay',label:'Bed heater relay',name:/^main relay body/i,signals:[['heater_bed',['power']]]},
 {key:'controller_fan',label:'Electronics bay fan',name:/^6020 (housing|impeller)/i,full:/\/ Skirt/,signals:[['controller_fan controller_fan',null]]}];
export function partWiring(entry,center,bedCenter){
 if(!entry)return null;const slot=entry.slot_id||'',name=entry.name||'',full=entry.full||'';
 const w=PART_WIRING.find(w=>(!w.slot||w.slot.test(slot))&&(!w.name||w.name.test(name))&&(!w.full||w.full.test(full)));
 if(!w||w.key!=='z_motor')return w||null;
 if(!Array.isArray(center)||!Array.isArray(bedCenter)||[center[0],center[1],bedCenter[0],bedCenter[1]].some(v=>finite(v)===null))return null;
 const stepper=Z_AT[(center[0]>bedCenter[0]?1:0)+(center[1]>bedCenter[1]?2:0)];
 return {...w,key:stepper,label:'Z motor, '+stepper,signals:[['tmc2209 '+stepper,null]]};
}
// The signals a wired part shows, in map order. The Pi's host stats arrive beside the Klipper
// signals rather than among them, so its scalar fields are read from there.
export function partSignals(wiring,point){
 if(!wiring)return [];const all=point?.telemetry?.signals||[],host=point?.telemetry?.host?.data,out=[];
 for(const [object,fields]of wiring.signals){
  const rows=object==='host'?Object.entries(host||{}).filter(([,v])=>v===null||typeof v!=='object').map(([field,value])=>({object,field,value})):all.filter(s=>s.object===object);
  for(const s of rows)if(fields===null||(Array.isArray(fields)?fields.includes(s.field):fields.test(s.field)))out.push(s);}
 return out;
}
// Reviewer on the cards: raw values printed at full float precision (hold_current 0.489535463898379). Every raw
// number now prints by its unit, with the unit beside it: current 2 dp, millimetres 3 dp, temperatures 1 dp, ratios
// 2 dp. The unit comes from the field name, and from the object where one name means two things: a fan's speed is a
// 0 to 1 command, gcode_move's speed is the G1 F value in mm/min. Counts stay whole; anything unrecognised keeps 3
// dp, never all 15.
const RATIO_OBJECT=/^(fan|heater_fan|controller_fan|fan_generic|temperature_fan)\b/;
const SIGNAL_UNITS=[[/current$/,'A',2],[/temp(erature)?$|^target$/,'°C',1],[/^(power|speed_factor|extrude_factor|progress|value|duty)$|_ratio$/,'',2],[/rpm$/,'rpm',0],
 [/velocity$|^speed$/,'mm/s',1],[/accel/,'mm/s²',0],[/^pressure_advance$|smooth_time$/,'s',3],[/^freq$/,'Hz',0],[/^(mcu_)?phase_offset$/,null,null],[/position|offset|origin|distance$/,'mm',3]];
export function signalUnit(object,field){
 const key=String(field||'').replace(/\.\d+$/,'').split('.').pop();
 if(key==='speed'&&RATIO_OBJECT.test(object||''))return {unit:'',digits:2};
 // gcode_move.speed is the last G1 F value, in mm/min (the cube print records 12000 at 200 mm/s).
 if(key==='speed'&&object==='gcode_move')return {unit:'mm/min',digits:0};
 const hit=SIGNAL_UNITS.find(([re])=>re.test(key));return hit&&hit[1]!==null?{unit:hit[1],digits:hit[2]}:null;
}
export function formatSignal(object,field,value){
 if(value===null||value===undefined)return 'Unknown';if(typeof value==='boolean')return value?'yes':'no';
 if(typeof value==='string')return value;if(typeof value==='object')return JSON.stringify(value);if(!Number.isFinite(value))return 'Unknown';
 const u=signalUnit(object,field);if(!u)return Number.isInteger(value)?String(value):value.toFixed(3);
 return value.toFixed(u.digits)+(u.unit?' '+u.unit:'');
}
// Volumetric flow from the filament speed Klipper reports (motion_report.live_extruder_velocity,
// mm/s of 1.75 mm filament). A retraction is not flow, so it reads 0; unknown stays unknown. The
// A flow warning needs the user's measured limit. An unknown limit stays unknown.
export function volumetricFlow(extruderVelocity,diameter=1.75){
 const v=finite(extruderVelocity);return v===null?null:Math.max(0,v)*Math.PI*(diameter/2)**2;
}
export function flowCeiling(loaded,measured){const max=finite(measured);return {name:loaded||'material not recorded',max:max!==null&&max>0?max:null};}
// The printer's state as the header shows it and every control obeys, read off the collector's own
// link (observer.printer_state); nothing here polls the printer. lock: printer controls are off.
// estop: the emergency stop may still go (Moonraker answers).
export function printerLink(observer,{replay=false,lastSeen=null,now=Date.now()/1000}={}){
 const ago=finite(lastSeen)===null?null:Math.max(0,now-lastSeen);
 const since=ago===null?'':ago<90?Math.round(ago)+' s':ago<5400?Math.round(ago/60)+' min':ago<172800?(ago/3600).toFixed(1)+' h':Math.round(ago/86400)+' days';
 // The page's own service comes first. While Stella Scope itself does not answer (service.js then
 // hands this rule {printer_state:'service'} with the moment it was lost), nothing about the printer
 // is current and no command can go, so it is a state of its own, not PRINTER OFF.
 if(observer?.printer_state==='service')return {state:'service',label:'SERVICE OFF',since,lock:true,estop:false,reason:'Stella Scope service is not running: this page cannot reach it, so nothing new arrives and no command can go. The page tries again by itself and picks up at once when it answers.'};
 if(replay)return {state:'replay',label:'REPLAY',since,lock:true,estop:false,reason:'Saved recording: printer commands are unavailable in replay.'};
 const st=observer?.printer_state;
 if(st==='unreachable')return {state:'off',label:'PRINTER OFF',since,lock:true,estop:false,reason:'Printer is off: Moonraker is not answering'+(since?`, last data ${since} ago`:'')+'. Viewing still works.'};
 if(st==='not_ready')return {state:'not_ready',label:'KLIPPER NOT READY',since,lock:true,estop:true,recover:['shutdown','error'].includes(observer.klipper_state),reason:'Klipper not ready: '+(observer.printer_message||'Moonraker answers but Klipper does not')+'.'};
 if(st==='ready')return {state:'ready',label:'PRINTER READY',since,lock:false,estop:true,reason:''};
 return {state:'unknown',label:'CHECKING',since,lock:false,estop:true,reason:''};
}
// What the header badge and the line under the name say about the controls, from the same state as
// the pill and the locks, so the badge cannot say LIVE CONTROLS while the view is rewound or the
// printer is off. `enabled`: the service lets controls run at all (its config).
export function controlsSummary(link,{live=true,enabled=false}={}){
 if(link.state==='service')return {key:'service',badge:'SERVICE OFF · LOCKED',line:'controls locked: Stella Scope is not running'};
 if(link.state==='replay')return {key:'replay',badge:'REPLAY · READ ONLY',line:'saved recording, read only'};
 if(link.state==='off')return {key:'printer_off',badge:'PRINTER OFF · LOCKED',line:'controls locked: printer off'};
 if(link.state==='not_ready')return link.recover?{key:'restarts',badge:'RESTARTS ONLY',line:'controls locked: only the restarts and E-STOP go'}:{key:'klipper',badge:'KLIPPER · LOCKED',line:'controls locked: Klipper not ready'};
 if(!enabled)return {key:'read_only',badge:'READ ONLY',line:''};
 if(!live)return {key:'rewound',badge:'REWOUND · LOCKED',line:'controls paused while rewound'};
 return {key:'live',badge:'LIVE CONTROLS',line:'controls armed'};
}
// Reviewer's decision on with Moonraker answering and Klipper halted or in error (`recover`), the two
// restarts are the way out, as in Mainsail and Fluidd, so they stay usable. Each still asks once.
// linkAllows says whether the printer's state lets a command through at all; the service's own rules
// for it come after.
export const RECOVERY=['firmware_restart','klipper_restart'];
export function linkAllows(link,id){return !link.lock||id==='emergency_stop'&&link.estop||link.recover===true&&RECOVERY.includes(id);}
// One preset store. The PREHEAT menu is the saved presets in their saved order, then Cool down, which
// is built in and cannot be deleted. Nothing in the HUD holds a second table.
export const COOL_DOWN=Object.freeze({name:'Cool down',nozzle:0,bed:0,fixed:true});
export function preheatMenu(presets){const list=Array.isArray(presets?.materials)?presets.materials:[];
 return [...list.filter(m=>typeof m?.name==='string'&&finite(m.nozzle)!==null&&finite(m.bed)!==null).map(m=>({name:m.name,nozzle:m.nozzle,bed:m.bed,fixed:false})),COOL_DOWN];}
// What the preset editor has not saved yet. rows carry `orig`, the index of the saved preset a row
// started from (null for an added row); a row is new, edited or the same, and a saved preset with no
// row left is removed.
export function presetDiff(saved,rows){const s=Array.isArray(saved)?saved:[],kept=new Set();
 const states=rows.map(r=>{const o=r.orig==null?null:s[r.orig];if(!o)return 'new';kept.add(r.orig);return String(r.name).trim()===o.name&&Number(r.nozzle)===Number(o.nozzle)&&Number(r.bed)===Number(o.bed)?'same':'edited';});
 const removed=s.length-kept.size;return {states,removed,changed:removed>0||states.some(x=>x!=='same')};}
