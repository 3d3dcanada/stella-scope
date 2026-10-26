// U02 (2026-10-20): the stage's click logic. One click grows the machine (or the camera) after a short wait a second click can cancel; two clicks are a reset and
// change nothing else; a drag (to turn the machine) is never a click; Escape shrinks it back. next-stage.js touches the page only through `document`, so a stand-in
// document that keeps its listeners is enough to press it exactly as a browser would.
import assert from 'node:assert/strict';
const listeners={};
const body={dataset:{},append(){}};
globalThis.matchMedia=()=>({matches:false});globalThis.requestAnimationFrame=f=>setTimeout(()=>f(performance.now()),16);globalThis.cancelAnimationFrame=t=>clearTimeout(t);
globalThis.document={body,querySelector:()=>null,createElement:()=>({setAttribute(){},style:{}}),addEventListener:(t,f)=>{(listeners[t]??=[]).push(f);}};
const stage=await import('../web/next-stage.js');
let resets=0;
const scene={cameraState:{position:[0,-1000,500],target:[0,0,0]},looks:[],look(p,t){this.looks.push([p,t]);this.cameraState={position:p,target:t};},reset(){resets++;this.cameraState={position:[0,-1000,500],target:[0,0,0]};}};
stage.initStage({scene:()=>scene,shift:()=>0});
const target={closest:sel=>sel&&/nx-scene/.test(sel)?target:null};
const fire=(type,e)=>{for(const f of listeners[type]||[])f({target,button:0,pointerId:1,clientX:0,clientY:0,...e});};
const click=(dx=0)=>{fire('pointerdown',{clientX:100,clientY:100});fire('pointerup',{clientX:100+dx,clientY:100});};
const wait=ms=>new Promise(r=>setTimeout(r,ms));

// 2026-10-24: "no matter where I click on the screen ... it zooms in on the printer." One click grows nothing now (a part's own pick opens it).
click();await wait(330);assert.equal(stage.isGrown(),false,'one click grows nothing');assert.ok(!('grown' in body.dataset));assert.equal(scene.looks.length,0,'the view does not move');

// two clicks in a row are still a reset of the view
click();await wait(100);click();await wait(330);
assert.equal(stage.isGrown(),false);assert.equal(resets,1,'a double click resets the view');

// grown on purpose (stage.grow, from the View page) it still shrinks back to exactly where it was, and Escape shrinks it
stage.grow();assert.equal(stage.isGrown(),true);assert.ok('grown' in body.dataset,'the page is told, so the chrome can fall away');await wait(600);
const d=p=>Math.hypot(...p),grownLook=scene.looks.at(-1);
assert.ok(Math.abs(d(grownLook[0])/d([0,-1000,500])-.8)<.01,'grown is 0.8 of the framed distance');
for(const f of listeners.keydown||[])f({key:'Escape'});assert.equal(stage.isGrown(),false,'Escape shrinks it');await wait(600);
const back=scene.looks.at(-1)[0];assert.ok(back.every((v,i)=>Math.abs(v-[0,-1000,500][i])<1e-6),'shrunk back to the framing it grew from');

// the layout changing under a grown machine drops it with nothing to animate back to
stage.grow();stage.drop();assert.equal(stage.isGrown(),false);assert.ok(!('grown' in body.dataset));
console.log('next-stage tests: ok');
