// Stella Scope UI board U02 (2026-10-20): the machine, or the camera when Camera is chosen, is the whole background. One click grows it to fill the screen and
// one click shrinks it back the way it grew; drag turns it (the camera picture is dragged), wheel or pinch zooms, double click resets. While it is grown the rest of
// the page falls away, except the state chip and STOP (the CSS does that from body[data-grown]). No dialog, no menu: the picture itself is the control.
const GROW=.8,TARGETS='#nx-scene,.nx-stagecard canvas,#nx-bgcam';   // the camera moves to .8 of its distance: the machine is 1.25 times bigger
let grown=false,saved=null,anim=0,ctx={scene:()=>null,shift:()=>0,layout:()=>''};
const ease=t=>t<.5?4*t*t*t:1-Math.pow(-2*t+2,3)/2;
const lerp=(a,b,t)=>a.map((v,i)=>v+(b[i]-v)*t);
export const isGrown=()=>grown;
function tween(ms,fn){cancelAnimationFrame(anim);const t0=performance.now();const f=now=>{const t=Math.min(1,(now-t0)/ms);fn(ease(t));if(t<1)anim=requestAnimationFrame(f);};anim=requestAnimationFrame(f);}
function moveCamera(sc,to,ms=480){const from=sc.cameraState;tween(ms,t=>sc.look(lerp(from.position,to.position,t),lerp(from.target,to.target,t)));}
// Grown, the machine is centred in the whole screen: a phone frames it above the bank of gauges, and with those gone that band is the whole screen.
const grownPose=s=>{const t=s.target,z=ctx.shift();return {position:s.position.map((v,i)=>t[i]+(v-t[i])*GROW+(i===2?z:0)),target:t.map((v,i)=>v+(i===2?z:0))};};
const hintText=cam=>matchMedia('(pointer: coarse)').matches?'TAP TO SHRINK · DRAG TO '+(cam?'MOVE':'TURN')+' · PINCH TO ZOOM':'CLICK TO SHRINK · DRAG TO '+(cam?'MOVE':'TURN')+' · WHEEL TO ZOOM · DOUBLE CLICK TO RESET';
export function grow(){if(grown)return;grown=true;document.body.dataset.grown='';const h=document.querySelector('.nx-grown-hint');if(h)h.textContent=hintText(!!document.body.dataset.bg);const sc=ctx.scene();if(sc?.cameraState&&!document.body.dataset.bg){saved=sc.cameraState;moveCamera(sc,grownPose(saved));}}
export function shrink(){if(!grown)return;grown=false;delete document.body.dataset.grown;const sc=ctx.scene();if(saved&&sc?.look){moveCamera(sc,saved);}saved=null;}
// The layout changed under it (a phone turned round): nothing to animate back to.
export function drop(){cancelAnimationFrame(anim);grown=false;saved=null;delete document.body.dataset.grown;}
export const toggle=()=>grown?shrink():grow();
// Double click puts the view back where it starts (and stays grown if it was).
export function reset(){const sc=ctx.scene();camReset?.();if(!sc?.reset||document.body.dataset.bg)return;sc.reset();if(grown){saved=sc.cameraState;moveCamera(sc,grownPose(saved));}}
let camReset=null;
export function initStage(c){ctx={...ctx,...c};
 const hint=document.createElement('div');hint.className='nx-grown-hint';hint.setAttribute('aria-hidden','true');hint.textContent=matchMedia('(pointer: coarse)').matches?'TAP TO SHRINK · DRAG TO TURN · PINCH TO ZOOM':'CLICK TO SHRINK · DRAG TO TURN · WHEEL TO ZOOM · DOUBLE CLICK TO RESET';document.body.append(hint);
 let down=null,pts=new Set(),lastUp=-1e9,timer=0;
 const hit=e=>e.target.closest?.(TARGETS);
 document.addEventListener('pointerdown',e=>{if(!hit(e))return;pts.add(e.pointerId);down=pts.size===1&&e.button===0?{x:e.clientX,y:e.clientY,t:performance.now()}:null;},true);
 document.addEventListener('pointercancel',e=>{pts.delete(e.pointerId);down=null;},true);
 document.addEventListener('pointerup',e=>{if(!pts.delete(e.pointerId))return;const d=down;down=null;if(!d||pts.size)return;
  if(Math.hypot(e.clientX-d.x,e.clientY-d.y)>5||performance.now()-d.t>450)return;
  // 2026-10-24: "no matter where I click on the screen ... it zooms in on the printer." A click no longer grows the machine: a click on a part opens
  // that part (hud-scene's own pick), a click on empty space does nothing. Two clicks still put the view back where it starts.
  const now=performance.now();clearTimeout(timer);if(now-lastUp<320){lastUp=-1e9;reset();return;}lastUp=now;},true);
 document.addEventListener('keydown',e=>{if(e.key==='Escape'&&grown&&!document.querySelector('.nx-modal,#nx-sheet[data-open]'))shrink();});}
// The camera picture as the background: drag moves it, wheel or pinch zooms it about the pointer, double click puts it back. `box` is #nx-bgcam; its one child is what moves.
export function camZoom(box){const z=box.firstElementChild;let s=1,x=0,y=0;const pts=new Map();let pinch=0;
 const lim=()=>{const g=grown?1.3:1,W=box.clientWidth,H=box.clientHeight,mx=Math.max(0,(s*g-1)*W/2),my=Math.max(0,(s*g-1)*H/2);x=Math.max(-mx,Math.min(mx,x));y=Math.max(-my,Math.min(my,y));};
 const apply=()=>{lim();z.style.setProperty('--z',s.toFixed(3));z.style.setProperty('--px',Math.round(x)+'px');z.style.setProperty('--py',Math.round(y)+'px');};
 const at=(px,py,k)=>{const r=box.getBoundingClientRect(),cx=px-(r.left+r.width/2),cy=py-(r.top+r.height/2),n=Math.max(1,Math.min(6,s*k)),f=n/s;x=cx-f*(cx-x);y=cy-f*(cy-y);s=n;apply();};
 box.addEventListener('wheel',e=>{e.preventDefault();at(e.clientX,e.clientY,Math.exp(-e.deltaY*.0015));},{passive:false});
 box.addEventListener('pointerdown',e=>{pts.set(e.pointerId,[e.clientX,e.clientY]);box.setPointerCapture?.(e.pointerId);pinch=0;z.dataset.drag='';});
 box.addEventListener('pointermove',e=>{const p=pts.get(e.pointerId);if(!p)return;const dx=e.clientX-p[0],dy=e.clientY-p[1];p[0]=e.clientX;p[1]=e.clientY;
  if(pts.size===2){const [a,b]=[...pts.values()],d=Math.hypot(a[0]-b[0],a[1]-b[1]);if(pinch)at((a[0]+b[0])/2,(a[1]+b[1])/2,d/pinch);pinch=d;}else if(s>1||grown){x+=dx;y+=dy;apply();}});
 const up=e=>{pts.delete(e.pointerId);pinch=0;if(!pts.size)delete z.dataset.drag;};box.addEventListener('pointerup',up);box.addEventListener('pointercancel',up);
 camReset=()=>{if(!box.isConnected)return;s=1;x=0;y=0;apply();};apply();return {reset:camReset,apply};}
