// Shared, side-effect-free view logic; browser and Node tests use the same code.
export const PAGES = [
  ['printer','Virtual printer','01'],['motion','Motion & position','02'],
  ['motors','Motors & drives','03'],['thermal','Heat & cooling','04'],
  ['leveling','Gantry & bed','05'],['events','Event timeline','06'],
  ['recording','Recording & notes','07']
];
export const MOTOR_IDS=['stepper_x','stepper_y','stepper_z','stepper_z1','stepper_z2','stepper_z3','extruder'];
export const PARTS=[
 {id:'head',name:'Print head',short:'Print head',group:'Toolhead',description:'Travels in X and Y on the flying gantry. Nozzle height follows the recorded Z coordinate.',source:'Klipper motion report; requested motion, not encoder feedback.',fields:[['X position','position.0','mm'],['Y position','position.1','mm'],['Nozzle height','position.2','mm'],['Travel speed','velocity','mm/s'],['Homed axes','homed','text'],['Job state','job.state','text'],['Job file','job.name','text'],['Job progress','job.progress','percent']]},
 {id:'hotend',name:'Hotend & nozzle',short:'Hotend',group:'Toolhead',description:'Heats and melts filament. Temperature, target and heater output come from the recorded extruder heater.',source:'Extruder heater status. The housing is a simplified envelope, not a verified CAD model of your hotend.',fields:[['Temperature','nozzle.temperature','°C'],['Target','nozzle.target','°C'],['Heater output','nozzle.power','percent']]},
 {id:'probe',name:'Bed probe',short:'Probe',group:'Toolhead',description:'Checks the surface under the offset probe point. Its X/Y marker follows the recorded nozzle plus the probe offsets.',source:'Last probe query is not continuous contact sensing. Probe pin extension, docking and mount geometry are not reported.',fields:[['Last query triggered','probe.triggered','bool'],['Last Z result','probe.last_z_result','mm']]},
 {id:'extruder',name:'Filament feeder motor',short:'Feeder',group:'Motors',description:'Feeds or retracts filament above the hotend. Driver readings are separate from heater readings.',source:'TMC2209 extruder status; no shaft encoder or filament-force sensor.',motor:true},
 {id:'fan',name:'Part cooling fan',short:'Part fan',group:'Cooling',description:'Directs air at the printed part. The indicated percentage is the recorded fan command.',source:'fan.speed; a command is not measured rotation. RPM appears only when a tachometer reports it.',fields:[['Fan command','fans.fan.speed','percent'],['Measured speed','fans.fan.rpm','rpm']]},
 {id:'hotend_fan',name:'Hotend cooling fan',short:'Hotend fan',group:'Cooling',description:'Cools the hotend heat break. It follows the toolhead in the model.',source:'heater_fan hotend_fan. Exact fan arrangement on the modified toolhead is unverified.',fields:[['Fan command','fans.hotend_fan.speed','percent'],['Measured speed','fans.hotend_fan.rpm','rpm']]},
 {id:'heater_bed',name:'Stationary heated bed',short:'Heated bed',group:'Structure',description:'The V2.4 bed stays fixed. The gantry—not this plate—moves up and down.',source:'heater_bed status. Bed dimensions use recorded geometry; the mounting envelope is schematic.',fields:[['Temperature','bed.temperature','°C'],['Target','bed.target','°C'],['Heater output','bed.power','percent'],['Mesh profile','mesh.profile','text']]},
 {id:'gantry',name:'Flying XY gantry',short:'Gantry',group:'Structure',description:'The complete XY assembly rises on four belted Z corners. The two rear A/B motors rise with it.',source:'Overall Z follows requested nozzle position. Independent corner heights and physical tilt are NOT available.',fields:[['Nozzle height','position.2','mm'],['Levelling applied','qgl_applied','bool']]},
 {id:'x_rail',name:'X crossbeam & carriage',short:'X rail',group:'Structure',description:'The crossbeam moves front-to-back with Y. The print head moves left-to-right along it with X.',source:'Kinematic layout derived from recorded coordinates; rail strain and bearing drag are not measured.',fields:[['Crossbeam Y','position.1','mm'],['Carriage X','position.0','mm']]},
 {id:'y_rails',name:'Left & right Y rails',short:'Y rails',group:'Structure',description:'Guide the X crossbeam through front-to-back travel and rise with the gantry.',source:'Static guides within the moving gantry. No rail-load or deformation sensors.',fields:[['Gantry nozzle height','position.2','mm']]},
 {id:'stepper_x',name:'B drive · rear left',short:'B · left',group:'Motors',description:'Left rear CoreXY drive, named stepper_x in the standard configuration. A and B together produce both X and Y motion.',source:'Standard Voron mapping. Actual wiring may differ; this is not a pure X-only motor.',motor:true},
 {id:'stepper_y',name:'A drive · rear right',short:'A · right',group:'Motors',description:'Right rear CoreXY drive, named stepper_y in the standard configuration. It moves vertically with the gantry.',source:'Standard Voron mapping. Actual wiring may differ; this is not a pure Y-only motor.',motor:true},
 {id:'stepper_z',name:'Z0 drive · front left',short:'Z0 · FL',group:'Motors',description:'Lower front-left Z reduction drive. Its vertical belt lifts the front-left gantry corner.',source:'Supplied and standard corner mapping. No independently measured corner height.',motor:true},
 {id:'stepper_z1',name:'Z1 drive · rear left',short:'Z1 · RL',group:'Motors',description:'Lower rear-left Z reduction drive. Inspect this corner alongside captured levelling corrections and warnings.',source:'A recorded position-skew warning is evidence to inspect—not a measured physical displacement.',motor:true},
 {id:'stepper_z2',name:'Z2 drive · rear right',short:'Z2 · RR',group:'Motors',description:'Lower rear-right Z reduction drive. The motor stays at the base while its gantry carriage rises.',source:'Supplied and standard corner mapping. Driver current scale is not mechanical torque.',motor:true},
 {id:'stepper_z3',name:'Z3 drive · front right',short:'Z3 · FR',group:'Motors',description:'Lower front-right Z reduction drive. Click a historical correction to inspect its recorded moment.',source:'Supplied and standard corner mapping; not proof of physical corner position.',motor:true},
 {id:'controller_fan',name:'Electronics cooling fan',short:'Controller fan',group:'Cooling',description:'Moves cooling air through the lower electronics area. This position is an illustrative location only.',source:'controller_fan controller_fan. Board, power supply and exact fan placement have not been surveyed.',fields:[['Fan command','fans.controller_fan.speed','percent'],['Measured speed','fans.controller_fan.rpm','rpm']]},
 {id:'frame',name:'Frame & Z guides',short:'Frame',group:'Structure',description:'The rigid frame and four Z guide rails stay stationary. The bed sits inside the lower frame.',source:'Voron V2.4 R2 layout. Outer dimensions, clearances and printed-part envelopes are not measured CAD.',fields:[]}
];
export const partById=id=>PARTS.find(p=>p.id===id)||PARTS[0];
export const finite=n=>typeof n==='number'&&Number.isFinite(n)?n:null;
export const read=(o,path)=>path.split('.').reduce((v,k)=>v?.[k],o)??null;
export function format(value,unit='',digits=2){
 if(unit==='bool')return value===true?'Yes':value===false?'No':'Not reported';
 if(unit==='text')return typeof value==='string'&&value.length?value:'Not reported';
 if(finite(value)===null)return 'Not reported';
 if(unit==='percent')return (value*100).toFixed(0)+'%';
 return value.toFixed(unit==='rpm'?0:digits)+(unit?' '+unit:'');
}
export function flagText(flags){
 if(!Array.isArray(flags))return 'Driver status not reported';
 const words={otpw:'Driver warm',ot:'Driver overtemperature',s2vsa:'Short-to-supply A',s2vsb:'Short-to-supply B',s2ga:'Short-to-ground A',s2gb:'Short-to-ground B',ola:'Open-coil A',olb:'Open-coil B',t120:'Thermal flag 120',t143:'Thermal flag 143',t150:'Thermal flag 150',t157:'Thermal flag 157'};
 return flags.length?flags.map(f=>words[f]||f).join(' · '):'No reported warning flags';
}
export function fieldsFor(part){return part.motor?[
 ['Configured run current',`motors.${part.id}.current`,'A'],['Configured hold current',`motors.${part.id}.hold`,'A'],
 ['Driver current scale',`motors.${part.id}.scale`,'/ 31'],['Driver reports standstill',`motors.${part.id}.standstill`,'bool'],
 ['Motor energised',`motors.${part.id}.energised`,'bool']]:part.fields;}
export function severity(point,id){const m=point?.motors?.[id];if(m)return !Array.isArray(m.flags)?'unknown':m.flags.length?'warning':m.energised===true?'active':'idle';return point?'active':'unknown';}
export function eventCategory(e){if(e.kind==='command')return 'command';const s=(e.message||'').toLowerCase();if(e.kind==='position_warning'||s.includes('position skew'))return 'position';if(e.kind==='levelling_failed'||s.includes('retries aborting'))return 'levelling';if(e.kind.includes('connection')||e.kind==='printer_offline')return 'connection';if(e.kind==='error'||s.startsWith('!!')||s.includes('shutdown'))return 'error';if(e.kind.includes('gap')||e.kind.includes('notice')||e.kind==='degraded')return 'capture';return 'console';}
export function eventsFor(events,id){if(['frame','gantry','head','x_rail','y_rails'].includes(id))return events;const keys={hotend:['extruder','heater'],heater_bed:['heater_bed','bed','mesh'],probe:['probe','qgl'],fan:['fan'],hotend_fan:['hotend_fan'],controller_fan:['controller_fan']};return events.filter(e=>(keys[id]||[id]).some(k=>(e.message||'').toLowerCase().includes(k)));}
export function validBookmark(b){return b&&finite(b.ts)!==null&&b.ts>=0&&typeof b.label==='string'&&b.label.length<=80;}
export function viewState(url){const u=new URL(url,'http://localhost');const page=u.pathname.split('/').filter(Boolean).at(-1);const h=new URLSearchParams(u.hash.slice(1));const t=h.has('t')?Number(h.get('t')):null;return {page:PAGES.some(p=>p[0]===page)?page:'printer',part:PARTS.some(p=>p.id===h.get('part'))?h.get('part'):'head',ts:finite(t)!==null&&t>=0?t:null};}
// Ghost position is a layout anchor, never exposed as a measured coordinate.
export function pose(snapshot){const p=snapshot?.point,geo=snapshot?.geometry||{},bed=geo.bed_size_mm||[350,350];const raw=Array.isArray(p?.position)?p.position.slice(0,3):null;const ok=v=>finite(v)!==null&&Math.abs(v)<=10000;
 // Klipper can report null axes during probing/homing. A null axis now holds the last value
 // recorded for that axis in this snapshot's own history instead of teleporting the whole
 // model to the ghost pose; the ghost pose is reserved for when no axis is reported at all.
 const hold=(axis,ghost)=>{if(raw&&ok(raw[axis]))return {v:raw[axis],held:false};
  for(const h of (snapshot?.history||[]).slice().reverse()){const v=h?.position?.[axis];if(ok(v))return {v,held:true};}
  return {v:ghost,held:true};};
 const x=hold(0,bed[0]/2),y=hold(1,bed[1]/2),z=hold(2,Math.min(100,geo.z_max_mm||310));
 return {x:x.v,y:y.v,z:z.v,known:!!raw&&raw.every(v=>ok(v)),held:[x.held,y.held,z.held],bed,probe:geo.probe_offset_mm||[0,0],zmax:geo.z_max_mm||310};}
export function safeDate(ts){if(finite(ts)===null)return 'No sample';const d=new Date(ts*1000);return Number.isFinite(d.getTime())?d.toLocaleString(undefined,{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',second:'2-digit',fractionalSecondDigits:3}):'Timestamp outside browser date range';}
