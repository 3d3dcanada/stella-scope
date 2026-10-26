import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {
 angleOf,radialLayout,radialIndex,jogDirection,createHold,bindHold,swallowNextClick,createSwipe,createLongPress
} from '../web/next-gesture.js';

test('angleOf uses clockwise screen angles and has no zero-vector angle',()=>{
 assert.deepEqual([[1,0],[0,1],[-1,0],[0,-1],[0,0]].map(([x,y])=>angleOf(x,y)),[0,90,180,270,null]);
});
test('radialLayout places four items at the cardinal points',()=>{
 assert.deepEqual(radialLayout(4,100).map(({x,y})=>[x,y]),[[0,-100],[100,0],[0,100],[-100,0]]);
});
test('radialIndex selects cardinal positions',()=>{
 assert.deepEqual([[0,-50],[50,0],[0,50],[-50,0]].map(([x,y])=>radialIndex(x,y,8)),[0,2,4,6]);
});
test('radialIndex respects deadzone and empty layouts',()=>{
 assert.equal(radialIndex(3,3,8),-1);assert.equal(radialIndex(0,-50,0),-1);
});
test('radialIndex assigns an exact boundary to the higher item',()=>{
 assert.equal(radialIndex(50,-50,4),1);
 const rad=314.9*Math.PI/180;
 assert.equal(radialIndex(50*Math.cos(rad),50*Math.sin(rad),4),0);
});
test('jogDirection maps screen wedges to printer axes',()=>{
 assert.deepEqual([[40,0],[-40,0],[0,-40],[0,40],[10,10],[40,-39],[39,-40]].map(([x,y])=>jogDirection(x,y)),['x+','x-','y+','y-',null,'x+','y+']);
});
test('hold cancels on an early release',()=>{
 const h=createHold();assert.equal(h.start(0),0);assert.equal(h.progress(450),0.5);
 assert.equal(h.release(450),'cancel');assert.equal(h.progress(900),0);
});
test('hold confirms exactly once after completion',()=>{
 const h=createHold();h.start(0);assert.equal(h.progress(900),1);
 assert.equal(h.release(1000),'confirm');assert.equal(h.release(1001),'none');
});
test('starting an active hold restarts its clock',()=>{
 const h=createHold();h.start(0);h.start(300);assert.equal(h.progress(750),0.5);
});
test('hold release and cancellation without completion are safe',()=>{
 const h=createHold();assert.equal(h.release(100),'none');h.start(0);h.cancel();
 assert.equal(h.progress(1000),0);assert.equal(h.release(1000),'none');
});
test('captured hold cancels when the pointer leaves or capture is lost',()=>{
 const oldFrame=globalThis.requestAnimationFrame,oldCancel=globalThis.cancelAnimationFrame;
 let frame,confirmed=0,cancelled=0;globalThis.requestAnimationFrame=fn=>{frame=fn;return 1;};globalThis.cancelAnimationFrame=()=>{};
 const handlers=new Map(),el={disabled:false,isConnected:true,style:{setProperty(){}},getAttribute(){return null;},
  getBoundingClientRect(){return {left:0,top:0,right:100,bottom:40};},setPointerCapture(){},
  addEventListener(name,fn){handlers.set(name,fn);},removeEventListener(name){handlers.delete(name);}};
 const emit=(name,x=50,y=20)=>handlers.get(name)?.({pointerType:'touch',pointerId:1,clientX:x,clientY:y});
 try{const unbind=bindHold(el,{ms:1,onConfirm:()=>confirmed++,onCancel:()=>cancelled++});
  emit('pointerdown');emit('pointermove',150,20);frame();emit('pointerup',150,20);
  assert.equal(confirmed,0);assert.equal(cancelled,1);
  emit('pointerdown');emit('lostpointercapture');frame();assert.equal(confirmed,0);assert.equal(cancelled,2);
  emit('pointerdown');emit('pointerup',150,20);assert.equal(confirmed,0);assert.equal(cancelled,3);
  unbind();
 }finally{if(oldFrame===undefined)delete globalThis.requestAnimationFrame;else globalThis.requestAnimationFrame=oldFrame;
  if(oldCancel===undefined)delete globalThis.cancelAnimationFrame;else globalThis.cancelAnimationFrame=oldCancel;}
});
// 2026-10-22: a tap on CANCEL did nothing and said nothing. A hold says when it starts, and an early release is reported as a cancel (nothing sent).
test('a hold reports its start at the first touch, and a tap is an early release: started, never confirmed',()=>{
 const oldFrame=globalThis.requestAnimationFrame,oldCancel=globalThis.cancelAnimationFrame;
 let frame,started=0,confirmed=0,cancelled=0;globalThis.requestAnimationFrame=fn=>{frame=fn;return 1;};globalThis.cancelAnimationFrame=()=>{};
 const handlers=new Map(),el={disabled:false,isConnected:true,style:{setProperty(){}},getAttribute(){return null;},
  getBoundingClientRect(){return {left:0,top:0,right:100,bottom:40};},setPointerCapture(){},
  addEventListener(name,fn){handlers.set(name,fn);},removeEventListener(name){handlers.delete(name);}};
 const emit=(name,x=50,y=20)=>handlers.get(name)?.({pointerType:'touch',pointerId:1,clientX:x,clientY:y});
 try{const unbind=bindHold(el,{ms:900,onStart:()=>started++,onConfirm:()=>confirmed++,onCancel:()=>cancelled++});
  emit('pointerdown');assert.equal(started,1,'said so at the first touch');emit('pointerup');   // a tap
  assert.equal(confirmed,0);assert.equal(cancelled,1,'the early release is reported');
  unbind();
 }finally{if(oldFrame===undefined)delete globalThis.requestAnimationFrame;else globalThis.requestAnimationFrame=oldFrame;
  if(oldCancel===undefined)delete globalThis.cancelAnimationFrame;else globalThis.cancelAnimationFrame=oldCancel;}
});
test('swipe dismisses after crossing distance threshold',()=>{
 const s=createSwipe();s.start(0,0);assert.equal(s.move(130,200),130);assert.equal(s.end(130,200),'dismiss');
});
test('swipe dismisses on a fast final move',()=>{
 const s=createSwipe();s.start(0,0);s.move(40,100);s.move(60,110);
 assert.equal(s.end(60,110),'dismiss');
});
test('swipe clamps upward movement and stays open',()=>{
 const s=createSwipe();s.start(0,0);assert.equal(s.move(-50,50),0);assert.equal(s.end(-50,50),'stay');
});
test('long press fires once at its threshold',()=>{
 const p=createLongPress();p.start(0,0,0);assert.equal(p.check(499),false);
 assert.equal(p.check(500),true);assert.equal(p.check(600),false);
});
test('long press cancels when movement exceeds slop',()=>{
 const p=createLongPress();p.start(0,0,0);p.move(11,0);assert.equal(p.check(600),false);
});
test('pure exports import and run without browser globals or imports',()=>{
 const source=readFileSync(new URL('../web/next-gesture.js',import.meta.url),'utf8');
 assert.doesNotMatch(source,/^\s*import\s/m);
 assert.equal(typeof window,'undefined');assert.equal(typeof document,'undefined');
 assert.equal(angleOf(1,0),0);assert.equal(radialLayout(0,100).length,0);
 assert.equal(radialIndex(0,-50,8),0);assert.equal(jogDirection(40,0),'x+');
 assert.equal(createHold().progress(0),0);assert.equal(createSwipe().end(0,0),'stay');
 assert.equal(createLongPress().check(0),false);
});

test('after a completed hold the next click is swallowed once (the lift must not press the key that replaced the held one)',()=>{
 const listeners=[];const oldDoc=globalThis.document;
 globalThis.document={addEventListener(n,f,c){listeners.push([n,f,c]);},removeEventListener(n,f){const i=listeners.findIndex(l=>l[0]===n&&l[1]===f);if(i>=0)listeners.splice(i,1);}};
 try{swallowNextClick(900);assert.equal(listeners.length,1);
  let stopped=0,prevented=0;const ev={preventDefault(){prevented++;},stopPropagation(){stopped++;}};
  listeners[0][1](ev);assert.equal(stopped,1);assert.equal(prevented,1);assert.equal(listeners.length,0,'one click only');
 }finally{if(oldDoc===undefined)delete globalThis.document;else globalThis.document=oldDoc;}
});
