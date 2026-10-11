import {test} from 'node:test';
import assert from 'node:assert/strict';
import {
 fromTick,deriveMode,dockFor,moreMenu,heatColor,DIAL,dialFraction,dialAngle,polar,arcPath,dialModel,
 fmtTemp,fmtPct,fmtSpeed,fmtFlow,fmtPos,fmtDuration,fileTitle,jobModel,positionModel,flowModel,xyTravel
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
test('idle dock has all eight slots in table order',()=>{
 const dock=dockFor('idle');
 assert.deepEqual(dock.map(s=>s.key),['home','level','move','heat','files','macros','console','more']);
 assert.equal(dock.length,8);assert.ok(dock.every(s=>s.visible));
});
test('compact printing dock retains primary and danger slots in original order',()=>{
 const dock=dockFor('printing',{compact:true});
 assert.deepEqual(dock.map(s=>s.key),['pause','speed','flow','fan','exclude','cancel','more']);
 assert.deepEqual(dock.filter(s=>s.visible).map(s=>s.key),['pause','speed','flow','cancel','more']);
});
test('finished without a job uses idle dock',()=>assert.deepEqual(dockFor('finished',{jobName:''}),dockFor('idle')));
test('service and checking docks are empty',()=>{
 assert.deepEqual(dockFor('service'),[]);assert.deepEqual(dockFor('checking'),[]);
});
test('More starts with overflow and omits visible dock actions',()=>{
 const dock=dockFor('printing',{compact:true}),menu=moreMenu('printing',dock);
 assert.deepEqual(menu.slice(0,2).map(i=>i.key),['fan','exclude']);
 assert.ok(menu.length<=8);assert.ok(!menu.some(i=>dock.some(s=>s.visible&&s.key===i.key)));
 assert.deepEqual(menu.find(i=>i.key==='motors_off')?.danger,true);
 assert.deepEqual(menu.find(i=>i.key==='motors_off')?.hold,true);
});
test('heatColor applies nozzle and bed thresholds',()=>{
 assert.equal(heatColor(22,0,'nozzle').tone,'cold');
 assert.deepEqual(heatColor(120,250,'nozzle'),{tone:'warm',ink:'#FF9C55',glow:0.583});
 assert.equal(heatColor(250,250,'nozzle').tone,'hot');
 assert.equal(heatColor(270,250,'nozzle').tone,'danger');
 assert.equal(heatColor(290,0,'nozzle').tone,'danger');
 assert.equal(heatColor(34,0,'bed').tone,'cold');
 assert.equal(heatColor(40,0,'bed').tone,'warm');
 assert.equal(heatColor(60,0,'bed').tone,'hot');
 assert.equal(heatColor(116,0,'bed').tone,'danger');
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
test('fileTitle removes path, extension, and underscores',()=>assert.equal(fileTitle('sub/Cube_0.2mm_PETG.GCODE'),'Cube 0.2mm PETG'));
test('jobModel estimates remaining time only with useful progress',()=>{
 const readings={job:{state:'printing',name:'Cube.gcode',duration:600,progress:0.5}};
 assert.deepEqual(jobModel(readings),{active:true,title:'Cube',progress:0.5,elapsed:600,remaining:600,state:'printing'});
 readings.job.progress=0.005;assert.equal(jobModel(readings).remaining,null);
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
 assert.deepEqual(moreMenu('service',undefined).length,8);
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
