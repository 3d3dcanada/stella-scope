import test from 'node:test';
import assert from 'node:assert/strict';
import {PARTS,MOTOR_IDS,PAGES,pose,read,format,flagText,eventsFor,eventCategory,viewState,validBookmark,severity} from '../web/workspace-core.js';
import {layout,fitScale} from '../web/workspace-scene.js';
const snap=(x=100,y=160,z=30)=>({point:{position:[x,y,z]},geometry:{bed_size_mm:[350,350],probe_offset_mm:[-32.95,-10.718],z_max_mm:310}});
const first=(m,id)=>m.parts.find(p=>p.id===id);
test('seven distinct pages and eighteen selectable components',()=>{assert.equal(PAGES.length,7);assert.equal(PARTS.length,18);assert.equal(new Set(PARTS.map(p=>p.id)).size,18);});
test('every selectable component has actual scene geometry',()=>{const m=layout(snap());for(const p of PARTS)assert.ok(m.parts.some(s=>s.id===p.id),p.id);});
test('stationary bed does not follow Z',()=>assert.deepEqual(first(layout(snap()),'heater_bed').pos,first(layout(snap(100,160,180)),'heater_bed').pos));
test('all four Z motor bases remain stationary',()=>{for(const id of MOTOR_IDS.slice(2,6))assert.deepEqual(first(layout(snap()),id).pos,first(layout(snap(140,220,180)),id).pos);});
test('rear A and B drive rise with gantry',()=>{for(const id of ['stepper_x','stepper_y'])assert.equal(first(layout(snap(100,160,180)),id).pos[2]-first(layout(snap()),id).pos[2],150);});
test('CoreXY B is left of A',()=>assert.ok(first(layout(snap()),'stepper_x').pos[0]<first(layout(snap()),'stepper_y').pos[0]));
test('crossbeam follows Y',()=>assert.equal(first(layout(snap(100,220,30)),'x_rail').pos[1]-first(layout(snap()),'x_rail').pos[1],60));
test('head follows X and Y',()=>{const a=first(layout(snap()),'head').pos,b=first(layout(snap(140,200,30)),'head').pos;assert.equal(b[0]-a[0],40);assert.equal(b[1]-a[1],40);});
test('probe follows exact recorded XY offsets',()=>{const p=first(layout(snap()),'probe');assert.equal(p.pos[0],100-32.95);assert.equal(p.pos[1],160-10.718);});
test('toolhead cooling components travel together',()=>{for(const id of ['extruder','hotend','hotend_fan','fan','probe']){const a=first(layout(snap()),id).pos,b=first(layout(snap(130,180,50)),id).pos;assert.equal(b[0]-a[0],30);assert.equal(b[1]-a[1],20);assert.equal(b[2]-a[2],20);}});
test('missing position is ghost not an origin reading',()=>{assert.equal(pose({}).known,false);assert.equal(pose({point:{position:[null,2,3]}}).known,false);});
test('extreme input is not drawn as a measured coordinate',()=>assert.equal(pose(snap(1e100)).known,false));
test('zero is valid',()=>assert.equal(pose(snap(0,0,0)).known,true));
test('no independent corner height inferred',()=>{const m=layout(snap());const carriages=m.parts.filter(p=>MOTOR_IDS.slice(2,6).includes(p.id)&&p.kind==='box'&&p.size[2]===30);assert.equal(new Set(carriages.map(p=>p.pos[2])).size,1);});
test('unknown number is not zero',()=>{assert.equal(format(null,'A'),'Not reported');assert.equal(format(0,'A'),'0.00 A');});
test('booleans require booleans',()=>{assert.equal(format(null,'bool'),'Not reported');assert.equal(format('false','bool'),'Not reported');assert.equal(format(false,'bool'),'No');});
test('missing driver status not reported healthy',()=>{assert.equal(severity({motors:{stepper_z:{}}},'stepper_z'),'unknown');assert.equal(flagText(null),'Driver status not reported');});
test('nested reading preserves false and zero',()=>{assert.equal(read({x:{a:false}},'x.a'),false);assert.equal(read({x:0},'x'),0);});
test('recorded command is not classified as executed error',()=>assert.equal(eventCategory({kind:'command',message:'SHUTDOWN'}),'command'));
test('component message filter does not leak other motors',()=>assert.equal(eventsFor([{message:'stepper_z1 fault'},{message:'stepper_y fault'}],'stepper_z1').length,1));
test('invalid route and unknown part fall back safely',()=>assert.deepEqual(viewState('/workspace/missing#part=unknown&t=NaN'),{page:'printer',part:'head',ts:null}));
test('timestamp zero survives deep link',()=>assert.equal(viewState('/workspace/thermal#part=hotend&t=0').ts,0));
test('bookmarks have finite timestamp and bounded text',()=>{assert.ok(validBookmark({ts:0,label:'point'}));assert.ok(!validBookmark({ts:NaN,label:'point'}));assert.ok(!validBookmark({ts:1,label:'x'.repeat(81)}));});

test('default camera fits the complete frame envelope at desktop and phone widths',()=>{
 const bed=[350,350],top=496;
 for(const [w,h] of [[970,650],[600,500],[368,420],[1400,650]]){
  for(const [yaw,elevation] of [[-.53,.47],[0,.06],[0,1.55],[Math.PI/2,.12],[1.2,.9]]){
   const scale=fitScale(w,h,bed,top,yaw,elevation);
   for(const x of [-90,440])for(const y of [-90,440])for(const z of [-15,511]){
    const rx=x-175,ry=y-175,rz=z-top/2;
    const sx=w/2+(rx*Math.cos(yaw)-ry*Math.sin(yaw))*scale;
    const sy=h*.51+(-rz*Math.cos(elevation)-(rx*Math.sin(yaw)+ry*Math.cos(yaw))*Math.sin(elevation))*scale;
    assert.ok(sx>=20&&sx<=w-20,`horizontal clipping at ${w}x${h}: ${sx}`);
    assert.ok(sy>=25&&sy<=h-25,`vertical clipping at ${w}x${h}: ${sy}`);
   }
  }
 }
});
