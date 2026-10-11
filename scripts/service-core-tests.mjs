import test from 'node:test';import assert from 'node:assert/strict';
import {PAGES,SLOTS,emptyBook,validateRevision,appendRevision,partsAt,mergeBook,validateBook,journal,diagnose,safeUrl,n} from '../web/service-core.js';
import {serviceLayout,CAMERA_VIEWS,matrix,project} from '../web/service-scene.js';
import {snapshot} from './service-fixture.mjs';
import {partWiring,partSignals,formatSignal,signalUnit,volumetricFlow,flowCeiling,printerLink,preheatMenu,presetDiff,COOL_DOWN,linkAllows,RECOVERY,controlsSummary} from '../web/hud-core.js';
const revision=(changes={})=>({id:'bl.lower',name:'Z housing',state:'installed',reason:'Replaced cracked housing',effective_at:100,recorded_at:110,revision_id:'r1',...changes});
test('ten implemented pages including source assembly, and 154 distinct location templates',()=>{assert.equal(PAGES.length,10);assert.ok(PAGES.some(([id])=>id==='assembly'));assert.equal(SLOTS.length,154);assert.equal(new Set(SLOTS.map(x=>x.id)).size,154);});
test('each corner has 21 distinct records',()=>{for(const c of ['fl','bl','br','fr'])assert.equal(SLOTS.filter(s=>s.id.startsWith(c+'.')).length,21);});
test('unknown value is not zero',()=>{assert.equal(n(null),null);assert.equal(n(0),0);});
test('reject script URLs and credential-bearing references',()=>{for(const u of ['javascript:alert(1)','file:///tmp/a','https://u:p@example.org'])assert.equal(safeUrl(u),'');});
test('source links allow legitimate https',()=>assert.equal(safeUrl('https://example.org/a.stl'),'https://example.org/a.stl'));
test('effective history excludes future installed revision',()=>{let b=appendRevision(emptyBook(),revision());b=appendRevision(b,revision({revision_id:'r2',model:'R2B',effective_at:200,recorded_at:210}));assert.equal(partsAt(b,150).get('bl.lower').revision_id,'r1');assert.equal(partsAt(b,250).get('bl.lower').revision_id,'r2');assert.equal(partsAt(b,50).size,0);});
test('updates preserve original history',()=>{const old=emptyBook(),b=appendRevision(old,revision());assert.equal(old.revisions.length,0);assert.equal(b.revisions.length,1);});
test('merge is deduplicating and lossless',()=>{const b=appendRevision(emptyBook(),revision());const merged=mergeBook(b,b);assert.equal(merged.revisions.length,1);});
test('revision ID conflict rejected',()=>{const a=appendRevision(emptyBook(),revision()),b=appendRevision(emptyBook(),revision({model:'conflict'}));assert.throws(()=>mergeBook(a,b),/Conflicting/);});
test('different machine cannot merge silently',()=>assert.throws(()=>mergeBook(emptyBook('a'),emptyBook('b')),/different machine/));
test('prototype keys invalid',()=>assert.throws(()=>validateRevision(revision({id:'__proto__'}))));
test('negative effective timestamp invalid',()=>assert.throws(()=>validateRevision(revision({effective_at:-1}))));
test('malformed hash rejected',()=>assert.throws(()=>validateRevision(revision({file_sha256:'abc'}))));
test('unknown input fields rejected',()=>assert.throws(()=>validateRevision({...revision(),evil:'x'})));
test('plain hostile text retained as data',()=>assert.equal(validateRevision(revision({model:'<img src=x>'})).model,'<img src=x>'));
test('journal retains outcome and evidence',()=>{const b=journal(emptyBook(),'bl.lower','checked','Inspected without adjustment','Photo1');assert.equal(b.work.length,1);assert.equal(b.work[0].evidence,'Photo1');});
test('zero journal timestamp valid',()=>assert.equal(validateBook({...emptyBook(),work:[{id:'w',state:'open',at:0,note:'zero'}]}).work[0].at,0));
test('current scale alone never diagnoses a loose part',()=>{const s=snapshot(new URLSearchParams({ts:1789552800}));s.events=[];s.point.motors.stepper_z1.scale=31;assert.equal(diagnose(s).length,0);});
test('recorded symptom produces evidence and hypotheses',()=>{const r=diagnose(snapshot(new URLSearchParams()));assert.ok(r.length>0);assert.ok(r.every(x=>x.checks.length>0&&x.evidence&&x.why));});
test('operator symptom is explicitly labelled',()=>{const r=diagnose(null,'corner');assert.ok(r.length);assert.match(JSON.stringify(r),/operator|Operator|selected/i);});
test('bottom camera has negative pitch',()=>assert.ok(CAMERA_VIEWS.bottom[1]<-1.5));
test('underside geometry includes named electronics',()=>{const m=serviceLayout(snapshot(new URLSearchParams()),0);for(const id of ['controller','host','psu','ssr','distribution','inlet','probe'])assert.ok(m.parts.some(p=>p.id===id));});
test('bed stays fixed while gantry rises',()=>{const a=snapshot(new URLSearchParams()),b=structuredClone(a);b.point.position[2]+=50;const l=serviceLayout(a,0),r=serviceLayout(b,0);assert.equal(l.parts.find(p=>p.id==='heater_bed').pos[2],r.parts.find(p=>p.id==='heater_bed').pos[2]);assert.equal(r.parts.find(p=>p.id==='gantry').pos[2]-l.parts.find(p=>p.id==='gantry').pos[2],50);});
test('exploded electronics separate without changing telemetry',()=>{const s=snapshot(new URLSearchParams()),before=JSON.stringify(s),a=serviceLayout(s,0),b=serviceLayout(s,100);assert.ok(b.parts.find(x=>x.id==='controller').pos[2]<a.parts.find(x=>x.id==='controller').pos[2]);assert.equal(JSON.stringify(s),before);});
test('camera produces finite underside projection',()=>{const m=serviceLayout(snapshot(new URLSearchParams()),0),mat=matrix(800,570,m,...CAMERA_VIEWS.bottom,1);for(const p of m.parts){const q=project(p.pos,mat,800,570);assert.ok(Number.isFinite(q.x)&&Number.isFinite(q.y));}});

test('candidate reference does not replace as-built installed identity',()=>{let b=appendRevision(emptyBook(),revision());b=appendRevision(b,revision({revision_id:'candidate',state:'candidate',effective_at:120,recorded_at:121}));assert.equal(partsAt(b,150).get('bl.lower').revision_id,'r1');});
test('BLTouch retry lines are not a probe failure; the hard failure is',()=>{const s=snapshot(new URLSearchParams({ts:1789552800}));const probe=ev=>{s.events=ev;return diagnose(s).some(x=>x.id==='probe');};assert.equal(probe([{kind:'probe_retry',message:'// Failed to verify BLTouch probe is raised; retrying.'},{kind:'response',message:'BLTouch failed to verify sensor state; retrying.'}]),false);assert.equal(probe([{kind:'probe_alarm',message:'!! BLTouch failed to deploy'}]),true);});
// The part-to-signal map. The signals are the object and field names a real recorder
// projected in the sample of 2026-09-22 03:04:59 (GET /api/workspace), trimmed to a few fields
// per object; mcu keeps two of its fifty board constants so the link filter is tested.
const recorded=[['controller_fan controller_fan','speed',1],['controller_fan controller_fan','rpm',null],['extruder','can_extrude',false],['extruder','motion_queue',null],['extruder','power',0],['extruder','pressure_advance',0.04],['extruder','smooth_time',0.04],['extruder','target',0],['extruder','temperature',23.73],['fan','speed',0],['fan','rpm',null],['gcode_move','speed_factor',1],['heater_bed','power',0],['heater_bed','target',0],['heater_bed','temperature',19.51],['heater_fan hotend_fan','speed',0],['heater_fan hotend_fan','rpm',null],['idle_timeout','state','Ready'],['mcu','last_stats.bytes_retransmit',0],['mcu','last_stats.srtt',0.001],['mcu','mcu_version','v0.13.0-438-g5d24122c'],['mcu','mcu_constants.ADC_MAX',4095],['mcu','mcu_constants.BUS_PINS_i2c1','PB6,PB7'],['mcu EBBCan','last_stats.bytes_retransmit',0],['mcu EBBCan','mcu_constants.ADC_MAX',4095],['motion_report','live_extruder_velocity',0],['motion_report','live_position.0',0],['motion_report','live_position.3',-50],['motion_report','live_velocity',0],['print_stats','state','standby'],['probe','last_query',false],['probe','last_z_result',0],['probe','name','bltouch'],['quad_gantry_level','applied',false],['tmc2209 extruder','run_current',0.489535463898379],['tmc2209 extruder','drv_status.stst',1],['tmc2209 stepper_x','run_current',0.7954951288348658],['tmc2209 stepper_y','run_current',0.7954951288348658],['tmc2209 stepper_z','run_current',0.7954951288348658],['tmc2209 stepper_z1','run_current',0.7954951288348658],['tmc2209 stepper_z2','run_current',0.7954951288348658],['tmc2209 stepper_z3','run_current',0.7954951288348658],['toolhead','homed_axes',''],['virtual_sdcard','is_active',false]].map(([object,field,value])=>({object,field,value}));
const recordedPoint={telemetry:{signals:recorded,host:{captured_at:1790100298.5,data:{cpu_temp:42.842,moonraker_stats:[{cpu_usage:2.47}]}}}};
const objectsOf=entry=>new Set(partSignals(partWiring(entry),recordedPoint).map(s=>s.object));
test('the extruder lists its extruder drive and nothing else',()=>{const part={slot_id:'head.extruder',name:'Clockwork extruder',full:'2.4r2-350 v1 / Toolhead / Clockwork extruder'};const rows=partSignals(partWiring(part),recordedPoint);assert.deepEqual([...new Set(rows.map(s=>s.object))].sort(),['extruder','motion_report','tmc2209 extruder']);for(const other of ['fan','heater_fan hotend_fan','controller_fan controller_fan','probe','mcu','mcu EBBCan','heater_bed'])assert.ok(!rows.some(s=>s.object===other),other+' is not on the extruder');assert.ok(!rows.some(s=>s.object==='extruder'&&['temperature','target','power'].includes(s.field)),'the heater belongs to the hotend, not the extruder drive');assert.deepEqual(rows.filter(s=>s.object==='motion_report').map(s=>s.field),['live_extruder_velocity','live_position.3']);});
test('an extrusion, a printed cover, a printed mount and the LCD have no signals card',()=>{for(const entry of [{slot_id:'frame',name:'HFSB5-2020-350:1',full:'2.4r2-350 v1 / Frame:1 / HFSB5-2020-350:1'},{slot_id:'head.cowl',name:'Printed toolhead cowl'},{slot_id:'head.extruder_mount',name:'Extruder mount · printed'},{slot_id:'head.carriage',name:'X carriage · printed'},{slot_id:'enclosure.skirt',name:'Mini12864 Screen (1):1',full:'2.4r2-350 v1 / Skirt (1):1 / Front (1):1 / Mini12864 Assembly (1):1 / Mini12864 Screen (1):1'}]){assert.equal(partWiring(entry),null,entry.name);assert.equal(partSignals(partWiring(entry),recordedPoint).length,0);}});
test('hotend, fans, probe, bed and boards read only their own objects',()=>{assert.deepEqual([...objectsOf({slot_id:'head.core',name:'Hotend heating core'})],['extruder']);assert.deepEqual(partSignals(partWiring({slot_id:'head.nozzle',name:'Nozzle:1'}),recordedPoint).map(s=>s.field).sort(),['power','target','temperature']);assert.deepEqual([...objectsOf({slot_id:'head.part_fan',name:'Part cooling blower · left 4010'})],['fan']);assert.deepEqual([...objectsOf({slot_id:'head.hotend_fan',name:'Hotend fan · 3010 axial'})],['heater_fan hotend_fan']);assert.deepEqual([...objectsOf({slot_id:'head.probe',name:'BLTouch probe'})],['probe']);assert.deepEqual([...objectsOf({slot_id:'bed.sheet',name:'Flex Sheet:1'})],['heater_bed']);assert.deepEqual(partSignals(partWiring({slot_id:'head.board',name:'Toolhead CAN board'}),recordedPoint).map(s=>s.object+' '+s.field),['mcu EBBCan last_stats.bytes_retransmit']);assert.deepEqual(partSignals(partWiring({slot_id:'electronics',name:'Octopus:1'}),recordedPoint).map(s=>s.field),['last_stats.bytes_retransmit','last_stats.srtt','mcu_version']);assert.equal(partWiring({slot_id:'electronics',name:'Octopus Bracket:1'}),null);assert.deepEqual(partSignals(partWiring({slot_id:'electronics',name:'Raspberry Pi (1):1'}),recordedPoint).map(s=>s.field),['cpu_temp']);assert.deepEqual([...objectsOf({slot_id:'enclosure.skirt',name:'6020 Impeller (2) (1):1',full:'2.4r2-350 v1 / Skirt (1):1 / Sides (1):1 / 6020 Fan (3) (1):1 / 6020 Impeller (2) (1):1'})],['controller_fan controller_fan']);assert.equal(partWiring({slot_id:'',name:'6020 Impeller (1):1',full:'2.4r2-350 v1 / Exhaust Filter Assembly:1 / 6020 Fan (1):1 / 6020 Impeller (1):1'}),null,'the exhaust fan is not in printer.cfg');});
test('each Z motor reads the driver of its own corner, A and B their own',()=>{const bed=[127,127,0],z=(x,y)=>partWiring({slot_id:'z_drive',name:'NEMA17 Stepper (1):1'},[x,y],bed);assert.equal(z(-60,-45).key,'stepper_z');assert.equal(z(-60,380).key,'stepper_z1');assert.equal(z(314,380).key,'stepper_z2');assert.equal(z(314,-45).key,'stepper_z3');assert.deepEqual(partSignals(z(314,-45),recordedPoint).map(s=>s.object),['tmc2209 stepper_z3']);assert.equal(partWiring({slot_id:'z_drive',name:'NEMA17 Stepper (1):1'}),null,'no position, no guess');assert.deepEqual([...objectsOf({slot_id:'drive_b',name:'NEMA17 Black (1):1'})],['tmc2209 stepper_x']);assert.deepEqual([...objectsOf({slot_id:'drive_a',name:'NEMA17 Black:2'})],['tmc2209 stepper_y']);assert.equal(partWiring({slot_id:'z_drive',name:'Z Motor Mount (1):1'}),null);});
// A printed hotend mount's two halves are printed plastic with nothing wired to them, so they get
// the part chip and no signals card, like other printed parts.
test('the hotend mount halves have no signals card',()=>{for(const entry of [{slot_id:'head.hotend_mount',name:'Hotend mount, rear half'},{slot_id:'head.hotend_clamp',name:'Hotend clamp, front half'}]){assert.equal(partWiring(entry),null,entry.name);assert.deepEqual(partSignals(partWiring(entry),recordedPoint),[]);}assert.ok(partWiring({slot_id:'head.heatsink',name:'Heatsink:1'}),'the heatsink in the mount still reads the hotend');});
// Raw values print by unit, never at full float precision (0.489535463898379 A and so on).
test('raw signals print by their unit with the unit beside them',()=>{
 const cases=[['tmc2209 extruder','hold_current',0.489535463898379,'0.49 A'],['tmc2209 extruder','run_current',0.489535463898379,'0.49 A'],['tmc2209 extruder','phase_offset_position',0.0014032625,'0.001 mm'],
  ['motion_report','live_position.3',24659.633934501944,'24659.634 mm'],['extruder','temperature',249.87654,'249.9 °C'],['extruder','target',250,'250.0 °C'],['extruder','power',0.7234,'0.72'],
  ['fan','speed',0.5,'0.50'],['heater_fan hotend_fan','speed',1,'1.00'],['gcode_move','speed',12000,'12000 mm/min'],['motion_report','live_extruder_velocity',6.687237932569584,'6.7 mm/s'],
  ['toolhead','max_accel',5000,'5000 mm/s²'],['extruder','pressure_advance',0.04,'0.040 s'],['heater_fan hotend_fan','rpm',5400.4,'5400 rpm'],['gcode_move','homing_origin.2',-0.035,'-0.035 mm']];
 for(const [o,f,v,want]of cases)assert.equal(formatSignal(o,f,v),want,`${o} ${f}`);
 assert.equal(formatSignal('tmc2209 extruder','drv_status.cs_actual',15),'15','a count stays whole');
 assert.equal(formatSignal('tmc2209 extruder','mcu_phase_offset',1),'1','a phase offset is steps, not millimetres');
 assert.equal(formatSignal('x','unrecognised',1.23456),'1.235','unknown units keep 3 dp');
 assert.equal(formatSignal('extruder','can_extrude',true),'yes');assert.equal(formatSignal('extruder','temperature',null),'Unknown');assert.equal(formatSignal('extruder','temperature',NaN),'Unknown');
 assert.equal(signalUnit('fan','speed').unit,'');assert.equal(signalUnit('gcode_move','speed').unit,'mm/min');assert.equal(signalUnit('motion_report','live_velocity').unit,'mm/s');
});
test('volumetric flow stays separate from a user-measured ceiling',()=>{
 assert.ok(Math.abs(volumetricFlow(6.687237932569584)-16.085)<0.001);assert.equal(volumetricFlow(-3),0,'a retraction is not flow');assert.equal(volumetricFlow(null),null,'unknown stays unknown');
 assert.deepEqual(flowCeiling(null),{name:'material not recorded',max:null});assert.deepEqual(flowCeiling('PLA',24),{name:'PLA',max:24});assert.deepEqual(flowCeiling('ASA',null),{name:'ASA',max:null});
});
test('the PREHEAT menu reads the preset store, Cool down last and built in',()=>{const store={materials:[{name:'PLA',nozzle:218,bed:60},{name:'PETG Sunlu',nozzle:245,bed:85}],steps:[1],fan:[0]};
 const menu=preheatMenu(store);assert.deepEqual(menu.map(m=>[m.name,m.nozzle,m.bed,m.fixed]),[['PLA',218,60,false],['PETG Sunlu',245,85,false],['Cool down',0,0,true]]);
 assert.deepEqual(preheatMenu(null),[COOL_DOWN]);assert.deepEqual(preheatMenu({materials:[]}),[COOL_DOWN]);
 assert.deepEqual(preheatMenu({materials:[{name:'X',nozzle:'hot',bed:60},{nozzle:200,bed:60}]}),[COOL_DOWN]);
});
test('printer state from the collector link; E-STOP only while Moonraker answers',()=>{
 const off=printerLink({connected:false,printer_state:'unreachable',printer_message:'cannot reach Moonraker'},{lastSeen:1000,now:1000+61200});
 assert.equal(off.state,'off');assert.equal(off.lock,true);assert.equal(off.estop,false);assert.match(off.reason,/^Printer is off/);assert.match(off.reason,/17\.0 h ago/);
 const klippy=printerLink({printer_state:'not_ready',printer_message:'Printer is not ready (Klipper state: startup)'});
 assert.equal(klippy.lock,true);assert.equal(klippy.estop,true);assert.equal(klippy.reason,'Klipper not ready: Printer is not ready (Klipper state: startup).');
 assert.deepEqual([printerLink({printer_state:'ready'}).lock,printerLink({printer_state:'ready'}).estop],[false,true]);
 const replay=printerLink({printer_state:''},{replay:true});assert.equal(replay.lock,true);assert.equal(replay.estop,false);
 assert.equal(printerLink(null).state,'unknown');assert.equal(printerLink(null).lock,false);
});
test('the preset editor knows what is not saved',()=>{const saved=[{name:'PLA',nozzle:218,bed:60},{name:'PETG',nozzle:250,bed:80}];
 assert.equal(presetDiff(saved,[{orig:0,name:'PLA',nozzle:'218',bed:'60'},{orig:1,name:'PETG ',nozzle:250,bed:80}]).changed,false);
 const d=presetDiff(saved,[{orig:0,name:'PLA+',nozzle:218,bed:60},{orig:null,name:'ASA',nozzle:260,bed:100}]);
 assert.deepEqual(d.states,['edited','new']);assert.equal(d.removed,1);assert.equal(d.changed,true);
});
test('the two restarts stay usable while Klipper is halted and Moonraker answers',()=>{
 const halted=printerLink({printer_state:'not_ready',klipper_state:'shutdown',printer_message:"Lost communication with MCU 'EBBCan'"});
 assert.equal(halted.recover,true);for(const id of RECOVERY)assert.equal(linkAllows(halted,id),true,id);assert.equal(linkAllows(halted,'emergency_stop'),true);
 for(const id of ['home','set_heater','jog','macro','motors_off','reconnect','console'])assert.equal(linkAllows(halted,id),false,id);
 assert.equal(printerLink({printer_state:'not_ready',klipper_state:'error'}).recover,true);
 const starting=printerLink({printer_state:'not_ready',klipper_state:'startup'});assert.equal(starting.recover,false);assert.equal(linkAllows(starting,'firmware_restart'),false);assert.equal(linkAllows(starting,'emergency_stop'),true);
 const klippyGone=printerLink({printer_state:'not_ready',klipper_state:'',printer_message:'Moonraker returned HTTP 503 Service Unavailable'});assert.equal(linkAllows(klippyGone,'klipper_restart'),false);
 const off=printerLink({printer_state:'unreachable',klipper_state:''});for(const id of [...RECOVERY,'emergency_stop'])assert.equal(linkAllows(off,id),false,id);
 assert.equal(linkAllows(printerLink({printer_state:''},{replay:true}),'firmware_restart'),false);
 assert.equal(linkAllows(printerLink({printer_state:'ready',klipper_state:'ready'}),'home'),true);
});
test('the badge says what is true, from the same state rule; the service down is its own state',()=>{
 const ready=printerLink({printer_state:'ready'}),off=printerLink({printer_state:'unreachable'}),halted=printerLink({printer_state:'not_ready',klipper_state:'error'}),starting=printerLink({printer_state:'not_ready',klipper_state:'startup'});
 const replay=printerLink({printer_state:''},{replay:true}),down=printerLink({printer_state:'service',last_success:1000},{lastSeen:1000,now:1012});
 const b=(l,o)=>controlsSummary(l,o).badge;
 assert.equal(b(ready,{live:true,enabled:true}),'LIVE CONTROLS');
 assert.equal(b(off,{live:true,enabled:true}),'PRINTER OFF · LOCKED');
 assert.equal(b(ready,{live:false,enabled:true}),'REWOUND · LOCKED');assert.match(controlsSummary(ready,{live:false,enabled:true}).line,/paused while rewound/);
 assert.equal(b(replay,{live:true,enabled:false}),'REPLAY · READ ONLY');
 assert.equal(b(off,{live:false,enabled:true}),'PRINTER OFF · LOCKED','a printer that is off outranks rewound');
 assert.equal(b(halted,{live:true,enabled:true}),'RESTARTS ONLY');assert.equal(b(starting,{live:true,enabled:true}),'KLIPPER · LOCKED');
 assert.equal(b(ready,{live:true,enabled:false}),'READ ONLY');
 assert.equal(down.state,'service');assert.equal(down.since,'12 s');assert.equal(down.lock,true);assert.equal(down.estop,false);assert.match(down.reason,/^Stella Scope service is not running/);
 assert.equal(b(down,{live:true,enabled:true}),'SERVICE OFF · LOCKED');
 assert.equal(printerLink({printer_state:'service'},{replay:true}).state,'service','the service outranks replay');
 for(const id of ['emergency_stop','firmware_restart','home'])assert.equal(linkAllows(down,id),false,id);
});
