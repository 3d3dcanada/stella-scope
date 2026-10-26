import {test} from 'node:test';
import assert from 'node:assert/strict';
import {
 fromTick,deriveMode,dockFor,moreMenu,heatColor,heatTrend,HEAT_HOLD_S,holdWords,HOLD_NOT_SENT,cancelVerdict,filamentSettings,filamentShakeVerdict,FILAMENT_DEFAULTS,DIAL,dialFraction,dialAngle,polar,arcPath,dialModel,
 fmtTemp,fmtPct,fmtSpeed,fmtFlow,fmtPos,fmtDuration,fileTitle,jobModel,positionModel,flowModel,xyTravel,
 alarmBoxes,alarmWords,driverFlags,filamentSteps,filamentCanRecord,filamentProgress,FILAMENT,supportsFrom,noPrinterYet,paletteItems,printerWords,pageActions,plainRefusal,hitTitle,sampleStateWords,showForceModel
} from '../web/next-core.js';

const ready={state:'ready'};
const homed=fromTick({homed_axes:'xyz'});

test('XY travel uses observed rectangular limits and refuses missing geometry',()=>{
 for(const [w,h] of [[250,250],[300,300],[350,350],[250,300]]){
  const g=xyTravel({min:[0,0],max:[w,h]});assert.equal(g.width,w);assert.equal(g.height,h);
  assert.equal(g.cx,w/2);assert.equal(g.cy,h/2);
 }
 assert.deepEqual(xyTravel({min:[-10,5],max:[240,305]}),{x0:-10,y0:5,x1:240,y1:305,width:250,height:300,cx:115,cy:155});
 for(const limits of [null,{min:[0,0],max:[null,350]},{min:[0,0],max:[0,350]},{min:[0,0],max:[3000,350]}])assert.equal(xyTravel(limits),null);
});

test('fromTick missing tick leaves every reading unknown',()=>{
 const r=fromTick(undefined);
 assert.equal(r.homedAxes,'');assert.equal(r.job.state,'');
 for(const value of [r.job.progress,r.job.duration,r.job.filament,...Object.values(r.position),r.velocity,r.maxVelocity,r.maxAccel,r.speedFactor,r.extrudeFactor,r.extruderVelocity,...Object.values(r.nozzle),...Object.values(r.bed),...Object.values(r.fans.part),...Object.values(r.fans.hotend),...Object.values(r.fans.controller)])assert.equal(value,null);
});
test('fromTick preserves unknown nozzle temperature',()=>assert.equal(fromTick({point:{nozzle:{temperature:null}}}).nozzle.temp,null));
test('fromTick clamps progress',()=>assert.equal(fromTick({job:{progress:1.4}}).job.progress,1));
test('deriveMode applies link and live precedence',()=>{
 for(const [link,live,mode] of [['service',false,'service'],['replay',false,'replay'],['ready',false,'rewound'],['off',true,'off'],['not_ready',true,'klipper'],['unknown',true,'checking']]){
  assert.equal(deriveMode({link:{state:link},live,readings:homed}).mode,mode);
 }
});
test('deriveMode selects job and homing modes',()=>{
 const mode=(state,name,axes='xyz')=>deriveMode({link:ready,live:true,readings:fromTick({homed_axes:axes,job:{state,name}})}).mode;
 assert.equal(mode('printing','Cube'),'printing');assert.equal(mode('paused','Cube'),'paused');
 assert.equal(mode('complete','Cube'),'finished');assert.equal(mode('complete',''),'idle');assert.equal(mode('','', 'xy'),'unhomed');
});
test('deriveMode computes heating and at-temperature flags',()=>{
 const readings=fromTick({point:{nozzle:{temperature:180,target:250}}});
 assert.equal(deriveMode({link:ready,readings}).heating,true);
 readings.nozzle.temp=249;
 assert.equal(deriveMode({link:ready,readings}).heating,false);
 assert.equal(deriveMode({link:ready,readings}).atTemp.nozzle,true);
});
test('idle dock has every slot in table order, Load and Unload beside Heat',()=>{
 // Farm board WO-01 (2026-10-12), 2026-09-28: Load and Unload on the bottom bar. The table had
 // eight slots; it has ten now, and a dock allowed ten shows them all.
 const dock=dockFor('idle',{maxSlots:12});
 // 2026-10-24: COOL beside HEAT, and a STOP that ends what the printer is doing without the emergency stop
 assert.deepEqual(dock.map(s=>s.key),['files','heat','cool','load','unload','home','level','move','macros','console','stop','more']); // 2026-10-23: PRINT first and lit, then heat and filament, then motion
 assert.equal(dock.length,12);assert.ok(dock.every(s=>s.visible));
 // At the old eight the two that give way are the ones More already carries.
 const eight=dockFor('idle');assert.deepEqual(eight.filter(s=>s.visible).map(s=>s.key),['files','heat','cool','load','unload','home','level','more']);
 assert.deepEqual(moreMenu('idle',eight).slice(0,3).map(i=>i.key),['move','macros','console']);
});
test('Load and Unload are one press: named commands, no sheet',()=>{
 for(const mode of ['idle','unhomed']){const d=dockFor(mode,{maxSlots:10});
  assert.deepEqual(d.filter(s=>/^(un)?load$/.test(s.key)).map(s=>[s.cmd,s.action]),[['filament_load','filament:load'],['filament_unload','filament:unload']]);}
 assert.ok(!dockFor('printing',{maxSlots:10}).some(s=>/load/.test(s.key)),'never offered mid-print');
});
test('a phone dock keeps Home, Heat, Load, Unload and More (idle: Print, Heat, Load, Home, More)',()=>{
 assert.deepEqual(dockFor('unhomed',{compact:true,maxSlots:5}).filter(s=>s.visible).map(s=>s.key),['home','heat','load','unload','more']);
 // 2026-10-23: a ready printer leads with PRINT; Unload goes to More on a phone
 assert.deepEqual(dockFor('idle',{compact:true,maxSlots:5}).filter(s=>s.visible).map(s=>s.key),['files','heat','load','home','more']);
 // The kiosk has six: the same four, then Level, and Move goes to More.
 assert.deepEqual(dockFor('idle',{compact:true,maxSlots:6}).filter(s=>s.visible).map(s=>s.key),['files','heat','cool','load','home','more']);
});
test('compact printing dock retains primary and danger slots in original order',()=>{
 const dock=dockFor('printing',{compact:true});
 assert.deepEqual(dock.map(s=>s.key),['pause','speed','flow','fan','exclude','job','camera','cancel','more']); // 2026-10-25: the print page and the camera are on the bar while printing
 assert.deepEqual(dock.filter(s=>s.visible).map(s=>s.key),['pause','speed','flow','cancel','more']);
});
test('finished without a job uses idle dock',()=>assert.deepEqual(dockFor('finished',{jobName:''}),dockFor('idle')));
test('service and checking docks are empty',()=>{
 assert.deepEqual(dockFor('service'),[]);assert.deepEqual(dockFor('checking'),[]);
});
test('More starts with overflow and omits visible dock actions',()=>{
 const dock=dockFor('printing',{compact:true}),menu=moreMenu('printing',dock);
 assert.deepEqual(menu.slice(0,2).map(i=>i.key),['fan','exclude']);
 assert.ok(menu.length<=12);assert.ok(!menu.some(i=>dock.some(s=>s.visible&&s.key===i.key)));
 assert.deepEqual(menu.find(i=>i.key==='motors_off')?.danger,true);
 assert.deepEqual(menu.find(i=>i.key==='motors_off')?.hold,true);
});
test('heatColor applies nozzle and bed thresholds',()=>{
 assert.equal(heatColor(22,0,'nozzle').tone,'cold');
 assert.deepEqual(heatColor(120,250,'nozzle'),{tone:'warm',ink:'#FF9C55',glow:0.583});
 assert.equal(heatColor(250,250,'nozzle').tone,'hot');
 assert.equal(heatColor(270,250,'nozzle').tone,'hot');   // 2026-10-22: 15 over target alone is not an alarm any more; it needs a trend (tests below). Before, this asserted 'danger'.
 assert.equal(heatColor(290,0,'nozzle').tone,'danger');
 assert.equal(heatColor(34,0,'bed').tone,'cold');
 assert.equal(heatColor(40,0,'bed').tone,'warm');
 assert.equal(heatColor(60,0,'bed').tone,'hot');
 assert.equal(heatColor(116,0,'bed').tone,'danger');
});
// 2026-10-22: lowering the nozzle (or bed) target fired "Nozzle over temperature, 250° against a target of 45°" while the heater was just cooling.
const feed=(kind,secs,fn)=>{const t=heatTrend(kind);let last;for(let s=0;s<=secs;s++){const [temp,target]=fn(s);t.push(1000+s,temp,target);last=[temp,target];}return {t,last};};
test('a target lowered from 250 to 45 with the temperature falling is cooling, never an alarm',()=>{
 const {t,last}=feed('nozzle',90,s=>[250-s*1.5,45]);   // cooling 1.5° a second toward 45
 const h=heatColor(last[0],last[1],'nozzle',t.read());
 assert.equal(h.tone,'cooling');assert.notEqual(h.tone,'danger');
 for(let s=0;s<=90;s+=3){const {t:u,last:l}=feed('nozzle',s,q=>[250-q*1.5,45]);assert.notEqual(heatColor(l[0],l[1],'nozzle',u.read()).tone,'danger',s+' s into the cool-down');}
});
test('a bed cooling slowly (0.03° a second) toward a lowered target is cooling too',()=>{
 const {t,last}=feed('bed',300,s=>[85-s*.03,45]);
 assert.equal(heatColor(last[0],last[1],'bed',t.read()).tone,'cooling');
});
test('held above target+15 and flat for 30 s is an alarm; before 30 s it is not',()=>{
 const {t:early,last:e}=feed('nozzle',HEAT_HOLD_S-5,()=>[270,250]);
 assert.notEqual(heatColor(e[0],e[1],'nozzle',early.read()).tone,'danger','25 s flat is not yet an alarm');
 const {t,last}=feed('nozzle',HEAT_HOLD_S+1,()=>[270,250]);
 assert.equal(heatColor(last[0],last[1],'nozzle',t.read()).tone,'danger');
});
test('rising while above target+15 is an alarm at once',()=>{
 const {t,last}=feed('nozzle',10,s=>[262+s*1.2,250]);
 assert.equal(heatColor(last[0],last[1],'nozzle',t.read()).tone,'danger');
});
test('over the absolute limit is an alarm instantly, falling or not, with no history at all',()=>{
 assert.equal(heatColor(290,250,'nozzle',{falling:true,rising:false,heldFor:0}).tone,'danger');
 assert.equal(heatColor(290,0,'nozzle').tone,'danger');
 assert.equal(heatColor(116,0,'bed',{falling:true,rising:false,heldFor:0}).tone,'danger');
});
test('a drop restarts the 30 s clock: a heater that stalls after cooling is judged from the stall',()=>{
 const t=heatTrend('nozzle');for(let s=0;s<=20;s++)t.push(2000+s,300-s*2,45);   // falling for 20 s (to 260)
 for(let s=21;s<=45;s++)t.push(2000+s,260,45);                                    // then flat for 25 s
 assert.notEqual(heatColor(260,45,'nozzle',t.read()).tone,'danger');
 for(let s=46;s<=52;s++)t.push(2000+s,260,45);                                    // 32 s flat
 assert.equal(heatColor(260,45,'nozzle',t.read()).tone,'danger');
});
test('heatColor keeps unknown temperature unknown',()=>assert.deepEqual(heatColor(null,250,'nozzle'),{tone:'unknown',ink:'#6E7883',glow:0}));
test('arcPath emits the 270 degree dial sweep',()=>{
 // 78 / sqrt(2) = 55.154328..., so endpoints round to (44.846,155.154) and (155.154,155.154).
 assert.equal(arcPath(100,100,78,135,405),'M 44.846 155.154 A 78 78 0 1 1 155.154 155.154');
});
test('arcPath rejects zero sweep and caps a sweep past one turn',()=>{
 assert.equal(arcPath(100,100,78,135,135),'');
 assert.equal(arcPath(100,100,78,135,535),arcPath(100,100,78,135,494.999));
});
test('dialModel clamps fractions and power',()=>{
 assert.deepEqual(DIAL,{start:135,sweep:270});
 assert.deepEqual(dialModel({value:125,min:0,max:250,target:250,power:1.5}),{frac:0.5,angle:270,targetFrac:1,targetAngle:405,powerFrac:1});
});
test('fmtDuration formats seconds, minutes, and hours',()=>{
 assert.equal(fmtDuration(59),'59 s');assert.equal(fmtDuration(61),'1 min');
 assert.equal(fmtDuration(3599),'60 min');assert.equal(fmtDuration(3840),'1 h 04 min');
 assert.equal(fmtDuration(7199),'2 h 00 min');
});
test('fileTitle removes path, extension, and underscores',()=>{assert.equal(fileTitle('sub/Cube_0.2mm_PETG.GCODE'),'Cube 0.2mm PETG');assert.equal(fileTitle('Benchy_PLA.gcode.3mf'),'Benchy PLA');assert.equal(fileTitle('Spool-cube.bgcode'),'Spool-cube');
 assert.equal(fileTitle('rings-Stella-PETG-r11-6perplate_0.2mm_PETG_Generic Klipper Printer_30m32s.gcode'),'rings-Stella-PETG-r11-6perplate');
 assert.equal(fileTitle('Spare_flapper_0.1mm_TPU_COREONEL_1h48m.bgcode'),'Spare flapper');});
test('jobModel says warming while a print has no print time yet',()=>{
 const r={job:{state:'printing',name:'a.gcode',progress:0.0066,duration:0}};assert.equal(jobModel(r).warming,true);
 r.job.duration=130;assert.equal(jobModel(r).warming,false);r.job.state='paused';r.job.duration=0;assert.equal(jobModel(r).warming,false);});
test('jobModel estimates remaining time only with useful progress',()=>{
 const readings={job:{state:'printing',name:'Cube.gcode',duration:600,progress:0.5}};
 assert.deepEqual(jobModel(readings),{active:true,warming:false,title:'Cube',progress:0.5,elapsed:600,remaining:600,state:'printing'});
 readings.job.progress=0.005;assert.equal(jobModel(readings).remaining,null);
 readings.job.time_left=840;assert.equal(jobModel(readings).remaining,840,'the printer own figure wins');delete readings.job.time_left;
 assert.equal(fromTick({job:{state:'printing',progress:0.1,duration:60,time_left:840}}).job.time_left,840,'the tick carries it');
});
test('flowModel applies only a saved measured ceiling',()=>{
 const area=Math.PI*(1.75/2)**2;
 const high=flowModel({extruderVelocity:17.9/area},'PETG',20),over=flowModel({extruderVelocity:21/area},'PETG',20);
 assert.equal(high.name,'PETG');assert.equal(high.max,20);assert.equal(high.tone,'high');
 assert.ok(Math.abs(high.value-17.9)<1e-10);assert.equal(over.tone,'over');
 assert.deepEqual(flowModel({extruderVelocity:17.9/area},'PETG').tone,'unknown');
});
test('unknown inputs yield unknown output without throwing',()=>{
 assert.deepEqual([fmtTemp(null),fmtPct(undefined),fmtSpeed(NaN),fmtFlow(null),fmtPos(undefined),fmtDuration(null)],Array(6).fill('—'));
 assert.equal(fileTitle(null),'');assert.equal(dialFraction(null,0,1),null);assert.equal(dialAngle(null),null);
 assert.equal(polar(null,0,1,0),null);assert.equal(arcPath(null,0,1,0,90),'');
 assert.deepEqual(dialModel(),{frac:null,angle:null,targetFrac:null,targetAngle:null,powerFrac:null});
 assert.deepEqual(positionModel(undefined).map(p=>p.text),['—','—','—']);
 assert.equal(jobModel(undefined).remaining,null);assert.equal(flowModel(undefined,null).value,null);
 assert.deepEqual(moreMenu('service',undefined).length,10);
});
test('HOME and LEVEL open sub-menus, and LEVEL is on the dock before homing',async()=>{
 const {SUB_MENUS}=await import('../web/next-core.js');
 assert.equal(dockFor('unhomed').find(s=>s.key==='home').action,'menu:home');
 assert.ok(dockFor('unhomed').some(s=>s.key==='level'&&s.action==='menu:level'));
 assert.deepEqual(SUB_MENUS.level.map(i=>i.cmd),['qgl','mesh','probe_accuracy','probe_query']);
 assert.equal(SUB_MENUS.home.find(i=>i.key==='home_z').action,'script:G28 Z');
});

test('homingFrame turns the recorded G28 into continuous travel',async()=>{const {homingFrame}=await import('../web/next-core.js');
 const lim={min:[0,0,-5],max:[350,350,310]},m=homingFrame(),start=[175,175,30];
 // recording 1790275734: X homing, then Y homing, then Z (x, y, z, velocity)
 const seq=[[0,0,0,0],[-168.99,0,10,70],[350.006,0,10,0],[349.04,0,10,35],[350,-161.38,10,70],[350,-126.29,10,70],[350,-91.19,10,70],[350,-56.09,10,70],[350,-20.96,10,70],[350,14.11,10,70],[350,49.24,10,70],[350,84.29,10,70],[350,119.36,10,70],[350,154.47,10,70],[350,344.83,10,35],
  [207.95,185.72,464.02,0],[207.95,185.72,455.56,5],[207.95,185.72,440.53,5],[207.95,185.72,1.9585,0],[207.95,185.72,10,0]];
 const out=seq.map(([x,y,z,v],k)=>m([x,y,z],{limits:lim,velocity:v,ts:k*0.5,start}));
 for(const p of out)for(const [i,v]of p.entries())assert.ok(v>=lim.min[i]-1&&v<=lim.max[i]+1,`inside travel: ${p}`);
 assert.ok(Math.abs(out[1][0]-(175+6.01))<0.1,'X starts from where it was drawn');
 assert.equal(out[2][0],350.006);
 assert.ok(out[4][1]>175&&out[5][1]>out[4][1]&&out[12][1]>=out[11][1],'Y glides forward, never back to 14');
 assert.equal(out[14][1],344.83);
 assert.ok(out[16][2]<out[15][2]&&out[16][2]<=310,'Z comes down from the top, not from 464');
 assert.equal(out[18][2],1.9585);assert.ok(out[2].rebased.includes('x')&&out[18].rebased.includes('z'),'the end of homing is marked for the scene');assert.equal(out[19][2],10);
 assert.deepEqual([...homingFrame()([100,100,20],{limits:lim})],[100,100,20],'ordinary positions pass through');});
test('homingFrame keeps an unhomed axis where it is drawn',async()=>{const {homingFrame}=await import('../web/next-core.js');
 const m=homingFrame(),lim={min:[0,0,-5],max:[350,350,310]};
 assert.deepEqual([...m([0,0,0],{limits:lim,homed:'',start:[100,120,30]})],[100,120,30]);
 assert.deepEqual([...m([0,0,10],{limits:lim,homed:'',start:[100,120,30]})],[100,120,40],'the Z hop lifts from where it was');
 assert.deepEqual([...m([-170,0,10],{limits:lim,homed:'x',start:[100,120,40],ts:1,velocity:70})],[105,120,40]);});

test('driver flags: open load at standstill is not a warning, in motion it is, other flags always are',()=>{
 // The reading from our report 2026-09-28: stepper_x olb=1, stst=1, cs_actual 25, printer parked.
 assert.deepEqual(driverFlags({flags:['olb'],standstill:true},false),[]);
 assert.deepEqual(driverFlags({flags:['olb'],standstill:true},true),[],'stst wins even if the head is moving elsewhere');
 assert.deepEqual(driverFlags({flags:['olb'],standstill:false},false),[],'a parked machine raises nothing');
 assert.deepEqual(driverFlags({flags:['olb','ola'],standstill:false},true),[],'open load alone is never raised, moving or not (F05: Stella Z motors)');
 assert.deepEqual(driverFlags({flags:['otpw','olb'],standstill:true},false),['otpw']);
 assert.deepEqual(driverFlags({flags:['ot']},false),['ot']);
 assert.deepEqual(driverFlags({flags:null},true),[]);assert.deepEqual(driverFlags(undefined),[]);
 assert.deepEqual(driverFlags({flags:['olb'],stst:true},true),[],'either spelling of standstill');
});
// Filament board 2026-10-22: the step lists grew (a load: heat, push it in, feed, cool, shake, done; an unload: heat, pull out, wait, cool, done). The old assertions counted four and three steps.
test('filament steps: markers are data, one now at a time, nothing recorded before the feed ran',()=>{
 const st=(kind,step)=>filamentSteps(kind,step).map(s=>s.state||'-');
 assert.deepEqual(filamentSteps('load','idle').map(s=>s.key),['heating','ready','running','cooling','shaking','done']);
 assert.deepEqual(filamentSteps('unload','idle').map(s=>s.key),['heating','running','waiting','cooling','done']);
 assert.deepEqual(st('load','idle'),['-','-','-','-','-','-']);assert.deepEqual(st('load','checking'),['-','-','-','-','-','-']);
 assert.deepEqual(st('load','heating'),['now','-','-','-','-','-']);
 assert.deepEqual(st('load','ready'),['done','now','-','-','-','-']);
 assert.deepEqual(st('load','running'),['done','done','now','-','-','-']);
 assert.deepEqual(st('load','cooling'),['done','done','done','now','-','-']);
 assert.deepEqual(st('load','shaking'),['done','done','done','done','now','-']);
 assert.deepEqual(st('load','done'),['done','done','done','done','done','now']);
 assert.deepEqual(st('load','confirmed'),['done','done','done','done','done','done']);
 assert.deepEqual(st('load','failed'),['-','-','-','-','-','-'],'a failed job marks nothing as the current step');
 assert.deepEqual(st('unload','heating'),['now','-','-','-','-']);assert.deepEqual(st('unload','running'),['done','now','-','-','-']);
 assert.deepEqual(st('unload','waiting'),['done','done','now','-','-']);assert.deepEqual(st('unload','cooling'),['done','done','done','now','-']);
 assert.deepEqual(filamentSteps('load','heating').map(s=>s.n),[1,2,3,4,5,6],'numbers are positions, fixed');
 assert.equal(filamentCanRecord({step:'done',ran:true}),true);
 assert.equal(filamentCanRecord({step:'done',ran:false}),false,'2026-09-28: pressing again marked it loaded instantly');
 assert.equal(filamentCanRecord({step:'ready',ran:true}),false);assert.equal(filamentCanRecord({step:'running',ran:true}),false);assert.equal(filamentCanRecord(null),false);
 // the board 2026-10-22: from the moment the macro has returned (the wait, the cool-down, the shake, the end), not before
 for(const step of ['waiting','cooling','shaking','done'])assert.equal(filamentCanRecord({step,ran:true}),true,step);
 for(const step of ['waiting','cooling','shaking','done'])assert.equal(filamentCanRecord({step,ran:false}),false,step+' with no feed run');
});
test('filament settings: 2 minutes, 4 seconds, shake on by default; kept inside their limits',()=>{
 assert.deepEqual(filamentSettings(undefined),{wait:120,pause:4,shake:true});assert.deepEqual(FILAMENT_DEFAULTS,{wait:120,pause:4,shake:true});
 assert.deepEqual(filamentSettings({wait:90,pause:8,shake:false}),{wait:90,pause:8,shake:false});
 assert.deepEqual(filamentSettings({wait:5000,pause:0,shake:'yes'}),{wait:600,pause:1,shake:true},'out of range is clamped; only false turns the shake off');
 assert.deepEqual(filamentSettings({wait:'abc',pause:null}),{wait:120,pause:4,shake:true});
});
test('the shake runs only when it is on, the printer is homed, no print is running or paused, and the printer takes it',()=>{
 const home={x:true,y:true,z:true};
 assert.deepEqual(filamentShakeVerdict({enabled:true,homed:home,mode:'idle',allowed:true}),{ok:true,why:''});
 assert.equal(filamentShakeVerdict({enabled:false,homed:home,mode:'idle'}).ok,false,'the setting turns it off');
 for(const mode of ['printing','paused'])assert.match(filamentShakeVerdict({enabled:true,homed:home,mode}).why,/print is running or paused/,mode);
 assert.match(filamentShakeVerdict({enabled:true,homed:{x:true,y:true,z:false},mode:'idle'}).why,/not homed/);
 assert.match(filamentShakeVerdict({enabled:true,homed:undefined,mode:'unhomed'}).why,/not homed/);
 assert.equal(filamentShakeVerdict({enabled:true,homed:home,mode:'idle',allowed:false}).ok,false);
});
test('filament progress: heat against the target, the feed against what the macro moves',()=>{
 assert.equal(filamentProgress({step:'heating',kind:'load',t:250},125),0.5);
 assert.equal(filamentProgress({step:'heating',kind:'load',t:250},null),0);
 assert.equal(filamentProgress({step:'running',kind:'load',moved:25},250),0.5);
 assert.equal(filamentProgress({step:'running',kind:'unload',moved:50},250),0.5);
 assert.equal(filamentProgress({step:'running',kind:'load',moved:500},250),0.99,'full only when the macro has returned');
 assert.equal(filamentProgress({step:'done',kind:'load',moved:0},250),1);
 assert.equal(filamentProgress({step:'waiting',kind:'unload',waitMs:120000,until:1060000},250,1000000),0.5,'the wait is the time gone');
 assert.equal(filamentProgress({step:'waiting',kind:'unload'},250),0);
 assert.equal(filamentProgress({step:'cooling',kind:'load',t:250},155),0.5,'the cool-down is the nozzle between its temperature and 60');
 assert.equal(filamentProgress({step:'shaking',kind:'load',t:250},60),1);
 assert.equal(filamentProgress({step:'idle'},250),0);
 assert.equal(FILAMENT.load.mm,50);assert.equal(FILAMENT.unload.mm,100);
});

test('a printer with no move capability: the dock has no Move, and the rest stays (farm WO-02)',()=>{
 // The shape /api/controls sends for a brand that heats and prints but cannot jog (src/commands.rs test
 // a_printer_that_cannot_move_says_so...): supported:false is hidden, allowed:false is only locked.
 const controls={commands:[{id:'jog',supported:false,allowed:false,reason:'Testbrand cannot move the head by hand'},{id:'home',supported:false,allowed:false},{id:'set_heater',supported:true,allowed:true},{id:'qgl',supported:false,allowed:false},{id:'filament_load',supported:true,allowed:false,reason:'printer state is printing'}]};
 const supports=supportsFrom(controls);
 assert.equal(supports('jog'),false);assert.equal(supports('set_heater'),true);
 assert.equal(supports('filament_load'),true,'cannot right now is not cannot at all');
 assert.equal(supports('never_listed'),true,'before the answer arrives, nothing is hidden');
 assert.equal(supportsFrom(undefined)('jog'),true);
 const keys=m=>dockFor(m,{maxSlots:12,supports}).map(s=>s.key);
 assert.ok(!keys('idle').includes('move'),'no jog pad on a printer that cannot move');
 assert.ok(!keys('idle').includes('home')&&!keys('idle').includes('level'));
 assert.ok(keys('idle').includes('heat')&&keys('idle').includes('load')&&keys('idle').includes('more'));
 // A paused print on that printer still offers what it supports; the full table is untouched without it.
 assert.deepEqual(dockFor('idle',{maxSlots:12}).map(s=>s.key),['files','heat','cool','load','unload','home','level','move','macros','console','stop','more']);
});

test('a printer that has no homing of its own is never "not homed" (farm WO-04)',()=>{
 const readings={homed:{x:false,y:false,z:false},job:{state:'',name:''},nozzle:{temp:22,target:0},bed:{temp:22,target:0}};
 assert.equal(deriveMode({link:ready,live:true,readings}).mode,'unhomed');
 assert.equal(deriveMode({link:ready,live:true,readings,homing:false}).mode,'idle');
 assert.equal(deriveMode({link:ready,live:true,readings:{...readings,job:{state:'printing',name:'a.gcode'}},homing:false}).mode,'printing');
});

test('the demo farm is offered only when nothing real has answered yet (polish WO-02)',()=>{
 assert.equal(noPrinterYet([]),true);
 assert.equal(noPrinterYet(undefined),true);
 assert.equal(noPrinterYet([{driver:'moonraker',state:'offline',tick_age_s:null}]),true,'a placeholder that never answered');
 assert.equal(noPrinterYet([{driver:'moonraker',state:'offline'},{driver:'prusalink',state:'offline',tick_age_s:null}]),true);
 assert.equal(noPrinterYet([{driver:'moonraker',state:'offline',tick_age_s:42}]),false,'it answered, then went off: a printer that is off');
 assert.equal(noPrinterYet([{driver:'moonraker',state:'idle',tick_age_s:0.4}]),false);
 assert.equal(noPrinterYet([{driver:'moonraker',state:'offline',tick_age_s:null},{driver:'demo',state:'printing',tick_age_s:0.2}]),false,'a demo farm is already there');
});

// polish WO-06 item 4 (2026-10-18): "files" + Enter sent a G-code to a demo printer. The command bar never offers
// free text as a line to send.
const BAR={actions:[{label:'Home all axes',help:'G28',go(){}},{label:'Things to print',help:'pick a file and print it',go(){}}],history:['G28','QUERY_ENDSTOPS'],commands:[{name:'QUERY_ENDSTOPS',help:'Report on the status of each endstop',macro:false},{name:'PARK',help:'',macro:true}]};
test('the command bar offers what is known, and never typed words as a line to send',()=>{
 for(const typed of ['files','sudo reboot','hello world','G28 X Y','M104 S200','kill']){
  const items=paletteItems(BAR,typed);
  assert.ok(!items.some(i=>i.kind==='send'),`"${typed}" is not offered as a line to send`);
  for(const i of items)assert.ok(['action','history','macro','gcode','none'].includes(i.kind),`${typed}: ${i.kind}`);
  // whatever carries a script is a command, macro or history line the printer already knows, never the typed words
  for(const i of items.filter(i=>i.script!==undefined))assert.ok(BAR.commands.some(c=>c.name===i.script)||BAR.history.includes(i.script),'only what the printer knows: '+i.script);
 }
 // nothing matches "files" as a command: the way on is the Console, which takes the text over without sending it
 const f=paletteItems(BAR,'files');
 assert.equal(f.length,1);assert.equal(f[0].kind,'action');assert.equal(f[0].consoleText,'files');assert.equal(f[0].script,undefined,'the Console item has no script to send');
 assert.match(f[0].label,/Console/);
 // a printer with no Console says that nothing matches, and offers nothing to send
 const n=paletteItems(BAR,'zzz',{console:false});
 assert.equal(n.length,1);assert.equal(n[0].kind,'none');assert.equal(n[0].script,undefined);assert.equal(n[0].consoleText,undefined);
 // an empty box: the known things, no Console prompt
 assert.ok(paletteItems(BAR,'').every(i=>i.kind!=='none'&&i.consoleText===undefined));
 // a command the printer lists still goes out, with arguments typed after it
 const q=paletteItems(BAR,'query_endstops probe=1');
 assert.ok(q.some(i=>i.kind==='gcode'&&i.script==='QUERY_ENDSTOPS'));
 assert.ok(q.every(i=>i.consoleText===undefined),'a matching first word needs no Console prompt');
 assert.ok(paletteItems(BAR,'park').some(i=>i.kind==='macro'&&i.script==='PARK'));
 assert.ok(paletteItems(BAR,'home').some(i=>i.kind==='action'&&i.label==='Home all axes'));
});

// polish WO-06 item 8 (2026-10-18, README rule 8): the words that name the engine.
test('Klipper words only on a Klipper printer',()=>{
 const plain=printerWords(false);
 assert.ok(!/Klipper|Moonraker|firmware restart/i.test(JSON.stringify(plain)),'plain words: '+JSON.stringify(plain));
 for(const k of [true,undefined,null]){const w=printerWords(k);assert.match(w.estop,/Klipper/);assert.match(w.position,/Klipper/);assert.match(w.fans,/Klipper/);}
});

// F06 (2026-10-19): with the pages in the list the bar had more than nine entries and its tie-break (-1 for any action) put them in no order: Home and Preheat vanished from an empty bar.
test('an empty command bar lists what the printer can do first, then the pages, in the order they were given',()=>{
 const acts=[...['Home all axes','Preheat PLA','Things to print','Console'].map(label=>({label,kind:'action'})),...Array.from({length:12},(_,i)=>({label:'Page '+i,kind:'action'}))];
 const items=paletteItems({actions:acts,history:[],commands:[]},'');
 assert.equal(items.length,9);
 assert.deepEqual(items.slice(0,5).map(i=>i.label),['Home all axes','Preheat PLA','Things to print','Console','Page 0']);
 const rank=paletteItems({actions:[{label:'Console',kind:'action'},{label:'Go home page',kind:'action'},{label:'Home all axes',kind:'action'}],history:['G28'],commands:[]},'home');
 assert.deepEqual(rank.map(i=>i.label),['Home all axes','Go home page'],'a match at the start of a name comes before a match inside it');
});
// F06 (2026-10-19): typing the name of a page found "Nothing matches"; the pages of the printer are actions now, and a page the printer cannot use is not offered.
const PAGES=[['i-nav-hud','Printer HUD','/'],['i-rewind','Print history','sheet:history'],['i-nav-workspace','Farm','/farm'],['i-files','Queue','sheet:queue'],['i-printers','Printers','sheet:printers'],['i-nav-signals','Signals','sheet:signals'],['i-more','All pages','sheet:pages']];
test('the pages of a printer are in the command bar, and a page it cannot use is not',()=>{
 const klipperOnly={signals:'klipper'},prusa=n=>n!=='klipper';
 const acts=pageActions(PAGES,{has:prusa,needs:klipperOnly}).map(a=>({...a,kind:'action'}));
 assert.deepEqual(acts.map(a=>a.label),['Print history','Farm','Queue','Printers','All pages'],'not the page the person is on, not Signals on a printer that is not Klipper');
 for(const [typed,label] of [['history','Print history'],['farm','Farm'],['queue','Queue'],['printers','Printers']]){
  const found=paletteItems({actions:acts,history:[],commands:[]},typed);
  assert.equal(found[0].label,label,typed+' finds its page');assert.notEqual(found[0].kind,'none');
 }
 assert.ok(pageActions(PAGES,{has:()=>true,needs:klipperOnly}).some(a=>a.label==='Signals'),'a Klipper printer has Signals');
 assert.deepEqual(pageActions(undefined),[]);
});

// F06 (2026-10-19): pressing Home while a print ran said "printer state is printing/printing; T3 commands are blocked while printing or paused".
test('a refusal the service words for a log is said to a person in plain words',()=>{
 assert.equal(plainRefusal('printer state is printing/printing; T3 commands are blocked while printing or paused'),'That cannot be done while a print is running or paused. Pause or cancel the print first.');
 assert.match(plainRefusal('all axes must be homed before qgl; current homed_axes is "x"'),/homed first/);
 assert.match(plainRefusal('G28 is not reported by current G-code help; command was not sent'),/does not offer that command/);
 assert.match(plainRefusal('Klippy state is startup, not ready; command was not sent'),/not ready \(it says: startup\)/);
 assert.match(plainRefusal('printer reported unknown print/idle state a/b; command was not sent'),/not clear right now/);
 assert.equal(plainRefusal('Some other reason.'),'Some other reason.','what is already plain is left alone');
 assert.equal(plainRefusal(undefined),'');
 for(const t of ['T3','printing/printing','Klippy','G-code help'])assert.ok(!plainRefusal('printer state is printing/printing; T3 commands are blocked while printing or paused').includes(t),t+' is not said to a person');
});

// F06 (2026-10-19): discovery listed a Prusa as "PrusaLink"; the connection test said "cancelled"; the queue offered "send to a different model" for a file that fits.
test('a found printer is named for what it is, and its state in the farm\'s words',()=>{
 assert.equal(hitTitle({driver:'prusalink',brand:'PrusaLink',host:'192.168.1.139'}),'A Prusa printer');
 assert.equal(hitTitle({driver:'prusalink',brand:'PrusaLink',name:'core-one-l'}),'Core One L','its own name, when it gives one');
 assert.equal(hitTitle({driver:'moonraker',brand:'Klipper',name:'voron-24'}),'Voron 24');
 assert.equal(hitTitle({driver:'flashforge',brand:'FlashForge',model:'FlashForge Adventurer 5M'}),'Adventurer 5M');
 assert.equal(hitTitle({driver:'moonraker',brand:'Klipper'}),'Klipper');
 assert.equal(sampleStateWords('cancelled'),'Ready (the last print was cancelled)');
 assert.equal(sampleStateWords('standby'),'Ready');assert.equal(sampleStateWords('complete'),'Finished');assert.equal(sampleStateWords('halted'),'Halted','an unknown word is at least tidied');
});
test('"send to a different model anyway" shows only when it means something',()=>{
 const fits=(...r)=>({fits:r.map(x=>({ok:!x,reasons:x?[x]:[]}))});
 assert.equal(showForceModel({...fits(null),force_model:false}),false,'a file that fits');
 assert.equal(showForceModel({...fits(null,'Made for COREONEL; Voron 2.4 350 is a different machine.'),force_model:false}),true,'a printer is held back for being another machine');
 assert.equal(showForceModel({...fits(null,'Sliced for a 0.6 mm nozzle; Bench has 0.4 mm.'),force_model:false}),false,'a nozzle that does not fit is never excused by this choice');
 assert.equal(showForceModel({...fits(null),force_model:true}),true,'already ticked: it can be undone');
 assert.equal(showForceModel(undefined),false);
});

// UI board U07 (2026-10-20): the failure alarm's words and its boxes.
test('the alarm draws the strongest boxes, 0 to 1 of the picture, and scales a 416 px grid down',()=>{
 const b=alarmBoxes({boxes:[{conf:.3,box:[.1,.1,.2,.2]},{conf:.87,box:[.5,.4,.8,.6]},{conf:.5,box:[.9,.9,.4,.8]}]});
 assert.deepEqual(b.map(x=>x.pct),[87,50,30]);
 assert.deepEqual([b[0].x1,b[0].y1,b[0].x2,b[0].y2],[.5,.4,.8,.6]);
 assert.deepEqual([b[1].x1,b[1].x2],[.4,.9],'a box given the wrong way round is swapped');
 const g=alarmBoxes({boxes:[{conf:.9,box:[208,104,416,312]}]})[0];
 assert.deepEqual([g.x1,g.y1,g.x2,g.y2],[.5,.25,1,.75]);
 assert.deepEqual(alarmBoxes({boxes:[{conf:.9,box:[.5,.5,.5,.5]}]}),[],'no area, no box');
 assert.deepEqual(alarmBoxes({boxes:[{conf:.9,box:[1,2,3]},{conf:.9}]}),[]);
 assert.deepEqual(alarmBoxes(null),[]);
 assert.equal(alarmBoxes({boxes:[1,2,3,4,5].map(i=>({conf:i/10,box:[.1*i,.1,.1*i+.05,.3]}))}).length,3,'at most three');
});
test('the alarm says what the failure watch saw, and that nothing was stopped',()=>{
 const f=alarmWords({name:'Adventurer 5M',text:'Adventurer 5M: this print looks like it is FAILING (spaghetti detector at 87%). Check the camera now; the print is still running.',detail:{verdict:'failing'}});
 assert.equal(f.title,'Adventurer 5M: this print may have failed');assert.match(f.body,/\(87%\)/);assert.match(f.body,/nothing has been stopped/);
 const w=alarmWords({name:'Stella',text:'Stella: the spaghetti detector sees something odd (31%). Worth a look at the camera.'});
 assert.equal(w.title,'Stella: something looks wrong with this print');assert.match(w.body,/\(31%\)/);
 assert.equal(alarmWords({text:'FAILING'},{name:'Bolt'}).title,'Bolt: this print may have failed','an old event with no detail still reads, by its words');
});

// 2026-10-22: CANCEL in the paused bar did nothing; Klipper got PAUSE and never CANCEL_PRINT.
test('a held key says what holding does, in plain words',()=>{
 assert.equal(holdWords('cancel','Cancel print'),'Hold to cancel the print');
 assert.equal(holdWords('motors_off','MOTORS OFF'),'Hold to switch the motors off');
 assert.equal(holdWords(null,'Exclude Gear'),'Hold to exclude gear');
 assert.match(HOLD_NOT_SENT,/^Not sent\./);
});
test('the HUD says "Print cancelled, heaters off" only when the printer says cancelled and both heater targets read 0',()=>{
 const r=(job,nt,bt)=>({job:{state:job},nozzle:{target:nt},bed:{target:bt}});
 assert.deepEqual(cancelVerdict(r('cancelled',0,0),1),{state:'ok',words:'Print cancelled, heaters off'});
 assert.equal(cancelVerdict(r('paused',250,60),3).state,'wait','sent, the printer has not answered yet');
 assert.equal(cancelVerdict(r('cancelled',250,60),3).state,'wait','cancelled, the heaters have not been turned off yet');
 assert.equal(cancelVerdict(r('cancelled',250,60),9).state,'warn');assert.match(cancelVerdict(r('cancelled',250,60),9).words,/heater is still on/);
 const f=cancelVerdict(r('paused',250,60),21);assert.equal(f.state,'fail');assert.match(f.words,/still says "paused"/);
 assert.equal(cancelVerdict(r('cancelled',null,null),1).state,'ok','a printer that reports no target is not hot');
});
