// Stella Scope NEXT app shell. Live data, the printer-state rule, the
// safe command path and every sheet. The data plumbing is ported from service.js / hud.js with the
// same constants (comments name the originals), so NEXT sees and sends exactly what the old HUD did.
import {createScene} from './hud-scene.js';
import {insideTravel,printerLink,controlsSummary,linkAllows,preheatMenu,presetDiff} from './hud-core.js';
import {fromTick,deriveMode,dockFor,moreMenu,jobModel,flowModel,heatColor,fmtTemp,fmtDuration,fmtPct,fileTitle,SUB_MENUS,homingFrame,xyTravel} from './next-core.js';
import {$,el,icon,loadSprite,initTips,tip,setWhy,hideTip,button,busy,control,openPopover,commandBar,fanRing,renderDock,openSheet,openModal,closeSheet,markOpener,clearOpener,closePopover,section,openRadial,toast,segmented,stepper} from './next-ui.js';
import {jogDirection} from './next-gesture.js';
import {initPages,paintPages,openPart,openFile,pageLevel,pageThermal,pageSignals,pageTimeline,pageParts,pagePrint,pageAlerts,pageCauses,pageJournal,pageAssembly,pageMotion,activityCard,liveProbes,meshRgb,openPartByKey,pageFilament,filamentTick,pageCamera,pageShaper,pageHistory,pageMaintenance,printHours,dueTasks,cameras,camImg} from './next-pages.js';

const hash=new URLSearchParams(location.hash.slice(1));
const finite=v=>typeof v==='number'&&Number.isFinite(v)?v:null;
const state={live:!hash.has('ts'),cursor:hash.has('ts')?Number(hash.get('ts')):null,snapshot:null,link:null,tick:null,readings:fromTick(null),
 mode:'checking',dockKey:'',hist:{nozzle:[],bed:[]},busy:false,seq:0,controller:null,presets:null,playing:false,speed:1,desktopAccess:false};

// ---------- 5. layout classes ----------
function layoutClass(){const w=innerWidth,h=innerHeight;
 if(w<600)return 'phone';if(h<440&&w<1000)return 'phone-land';if(h<=640&&w<=1100&&w>h)return 'kiosk';
 if(w<1280)return h>w?'tablet-port':'tablet';return 'desktop';}
function applyLayout(){const c=layoutClass();if(document.body.dataset.layout!==c){document.body.dataset.layout=c;state.dockKey='';render();reframe();}}
// The camera is fitted once the model has loaded and again whenever the layout class changes, so
// the machine is framed inside its own region at every screen size.
let reframeTimer=0;function reframe(){clearTimeout(reframeTimer);reframeTimer=setTimeout(()=>{if(scene?.entries?.length>100)scene.reset();},120);}
// No printer model on this computer (a fresh install has none): drop the 3D view for the camera card, as on a phone.
// Runs once per page: with the 3D view already off (phone, ?noscene) the camera card exists, and a
// second card would open a second camera stream (the recorder allows two in total).
function whenLoaded(start=performance.now()){if(document.body.dataset.scene==='off')return;const renderer=$('nx-scene')?.dataset.renderer;
 if(!scene||renderer==='source-unavailable'||performance.now()-start>90000){scene=null;$('nx-scene').hidden=true;document.body.dataset.scene='off';$('nx-loading')?.remove();stageCard();return;}
 if(renderer==='source-cad'){reframe();setTimeout(()=>{document.body.dataset.scene='ready';},400);return;}
 setTimeout(()=>whenLoaded(start),500);}

// ---------- live data (ported from service.js: get, request, SSE, back-off, jitter buffer) ----------
const SERVICE_BACKOFF_S=[1,2,5,10],svc={down:false,since:null,fails:0,next:0,link:null};   // service.js:13
const TICK_DELAY_S=0.45;                                                                   // service.js:320
let scene=null,lastTick=null,lastFull=performance.now(),tickBusy=false,sse=null,sseOk=false,interpAt=0,lastSceneDraw=0;
const tickBuffer=[],homeMap=homingFrame(),probeStore=new Map();
async function get(url,signal){let r;try{r=await fetch(url,{cache:'no-store',signal});}catch(e){if(e.name!=='AbortError')serviceLost();throw e;}
 serviceBack();const v=await r.json();if(!r.ok)throw Error(v.error||`HTTP ${r.status}; check the recorder log.`);return v;}
async function request(ts=state.live?null:state.cursor,step=null){const seq=++state.seq;state.controller?.abort();state.controller=new AbortController();state.busy=true;lastFull=performance.now();
 const q=new URLSearchParams({window:600,points:240});if(finite(ts)!==null)q.set('ts',ts);if(step)q.set('step',step);
 try{const r=await get('/api/workspace?'+q,state.controller.signal);if(seq!==state.seq)return;state.snapshot=r;if(r.observer_now)state.link=r.observer_now;state.cursor=r.cursor;
  if(Array.isArray(r.history)&&(!state.histFilled||!state.live)){state.hist={nozzle:[],bed:[]};for(const p of r.history)histPush(p.ts,p);if(r.point)histPush(r.point.ts,r.point);state.histFilled=true;}
  if(!state.live)state.readings=fromTick({...r,point:r.point,job:r.job||r.point?.job,homed_axes:r.point?.homed});
  if(!state.live&&scene&&r.point)scene.set(r,null);updateUrl();render();}
 catch(e){if(e.name!=='AbortError'&&seq===state.seq){if(!svc.down)toast(e.message,'alarm');render();}}
 finally{if(seq===state.seq)state.busy=false;}}
function takeTick(r){if(!r||r.schema!=='stella-scope.tick.v1')return;lastTick=r;                        // service.js:298
 // Probe landings arrive on some ticks and not others, newest first; they are kept here, oldest
 // first and once each, for the live mesh views and the probe marks on the plate.
 for(const q of r.point?.probe_points||[])if([q.x,q.y,q.z,q.ts].every(v=>finite(v)!==null))probeStore.set(q.ts+':'+q.x+':'+q.y,q);
 if(probeStore.size>600){const cut=(r.point?.ts??Date.now()/1000)-2400;for(const [k,q]of probeStore)if(q.ts<cut)probeStore.delete(k);}
 // G28 reports in Klipper's homing frame; homingFrame (next-core) turns it back into real travel first.
 if(r.point?.position){let start=null;try{start=JSON.parse($('nx-scene').dataset.drawnPose||'null');}catch{}r.point.position=homeMap(r.point.position,{limits:r.point.limits,velocity:r.point.velocity,ts:r.point.ts,start,homed:r.point.homed??r.homed_axes});
  // The mapped pose is real travel for every axis (homed or tracked from where it was), so the scene
  // draws all three; an axis whose frame was just re-set is marked so the scene lands it at once.
  r.point.rebased=r.point.position.rebased||'';if(typeof(r.point.homed??r.homed_axes)==='string')r.point.homed='xyz';}
 if(insideTravel(r.point?.position,r.point?.limits)){tickBuffer.push(r);if(tickBuffer.length>12)tickBuffer.shift();}
 applyTick(r);}
// Temperature history for the thermal cards: one point every 2 s, ten minutes kept, backfilled from
// the recording on the first snapshot (the recorder's own indexed readings), then from every tick.
function histPush(ts,p){if(finite(ts)===null||!p)return;for(const k of ['nozzle','bed']){const h=state.hist[k],v=p[k];if(!v)continue;
 if(h.length&&ts-h[h.length-1][0]<2)continue;h.push([ts,finite(v.temperature),finite(v.target)]);while(h.length&&h[0][0]<ts-620)h.shift();}}
let liveAt=0;
function applyTick(r){state.tick=r;if(r.observer)state.link=r.observer;if(!state.live)return;state.readings=fromTick(r);
 // Heat glow on the nozzle and plate and the fans' tint, as on the classic HUD (it was never wired here).
 if(scene?.live&&!document.body.hasAttribute('data-stale')&&performance.now()-liveAt>500){liveAt=performance.now();scene.live(r.point);}histPush(r.point?.ts??r.ts,r.point);
 if(!interpolatedNow()&&scene&&state.snapshot&&r.point?.position?.some(v=>finite(v)!==null))scene.set(liveSnapshot(r.point.position,r.point.ts),null);
 renderSoon();}
function serviceLost(){svc.next=performance.now()+SERVICE_BACKOFF_S[Math.min(svc.fails,SERVICE_BACKOFF_S.length-1)]*1000;svc.fails++;if(svc.down)return;
 svc.down=true;svc.since=Date.now()/1000;svc.link=state.link;state.link={printer_state:'service',last_success:svc.since};sse?.close();sse=null;sseOk=false;render();}
function serviceBack(){if(!svc.down)return;svc.down=false;svc.fails=0;svc.next=0;state.link=svc.link;lastFull=0;openSse();render();}
function openSse(){try{sse=new EventSource('/api/tick/stream');sse.onopen=()=>{sseOk=true;serviceBack();};sse.onmessage=e=>{try{takeTick(JSON.parse(e.data));}catch{}};
 sse.onerror=()=>{sseOk=false;if(!svc.down&&!tickBusy){tickBusy=true;get('/api/tick').catch(()=>{}).finally(()=>{tickBusy=false;});}};}catch{sseOk=false;}}
setInterval(async()=>{if(document.hidden||tickBusy)return;if(svc.down?performance.now()<svc.next:(!state.live||sseOk))return;tickBusy=true;
 try{const r=await get('/api/tick');if(state.live)takeTick(r);}catch{}finally{tickBusy=false;}},250);
setInterval(()=>{if(document.hidden||state.busy)return;if(svc.down){render();return;}
 if(state.playing&&state.snapshot){const at=Math.min(state.cursor+0.5*state.speed,state.snapshot.last_ts);if(at>=state.snapshot.last_ts)state.playing=false;request(at);}
 else if(state.live&&performance.now()-lastFull>5000){lastFull=performance.now();request(null);}},500);
function interpolatedNow(){if(!lastTick?.point?.position)return null;const t=interpAt=Date.now()/1000-TICK_DELAY_S;let lo=null,hi=null;   // service.js:329
 for(const s of tickBuffer){const ts=finite(s?.point?.ts);if(ts===null||!s.point.position)continue;if(ts<=t){if(!lo||ts>lo.ts)lo={ts,position:s.point.position};}else if(!hi||ts<hi.ts)hi={ts,position:s.point.position};}
 if(!lo)return null;if(!hi)return [0,1,2].map(i=>finite(lo.position[i]));
 const span=Math.max(1e-6,hi.ts-lo.ts),a=Math.min(1,Math.max(0,(t-lo.ts)/span));return [0,1,2].map(i=>{const x=finite(lo.position[i]),y=finite(hi.position[i]);return x===null||y===null?y:x+(y-x)*a;});}
let tickSnap=null,tickSnapSrc=null;
function liveSnapshot(position,ts){if(tickSnapSrc!==state.snapshot||!tickSnap){tickSnap={...state.snapshot,point:{...state.snapshot.point}};tickSnapSrc=state.snapshot;}   // service.js:336
 tickSnap.point.position=position;tickSnap.point.ts=ts===undefined?tickSnap.point.ts:ts;
 for(const k of ['homed','rebased','limits','max_velocity','max_z_velocity','velocity','commanded'])if(lastTick?.point&&k in lastTick.point)tickSnap.point[k]=lastTick.point[k];
 if(lastTick?.point?.probe_points)tickSnap.point.probe_points=lastTick.point.probe_points;else delete tickSnap.point.probe_points;return tickSnap;}
function tickFrame(){if(state.live&&!document.hidden){const pos=interpolatedNow();
  if(pos&&pos.some(x=>finite(x)!==null)){const t=performance.now();if(t-lastSceneDraw>=32){lastSceneDraw=t;if(scene&&state.snapshot)scene.set(liveSnapshot(pos,interpAt),null);paintPos(pos);}}}
 requestAnimationFrame(tickFrame);}
function updateUrl(){const h=new URLSearchParams();if(!state.live&&finite(state.cursor)!==null)h.set('ts',state.cursor);history.replaceState(null,'',location.pathname+(h.toString()?'#'+h:''));}

// ---------- controls and the command path (ported from hud.js: controls, link, can, why, lock, execute, send) ----------
let caps={enabled:false,reason:'Checking printer controls…',commands:[]},capsAt=-1e9;
const replayMode=()=>state.snapshot?.observer_now?.mode==='replay'||state.link?.mode==='replay';
function link(){return printerLink(state.link,{replay:replayMode(),lastSeen:state.link?.last_success??state.snapshot?.last_ts});}   // hud.js:141
async function controls(force=false){if(!force&&performance.now()-capsAt<8000)return caps;
 try{const r=await fetch('/api/controls');if(!r.ok)throw Error('Printer commands are not enabled in this build.');caps=await r.json();
  if(replayMode())caps.reason='Saved recording. Printer commands are unavailable in replay.';else if(caps.reason==='controls_enabled is false')caps.reason='Printer controls are off: set controls_enabled in stella-scope.json.';}
 catch(e){caps={enabled:false,reason:e.message,commands:[]};}capsAt=performance.now();lockControls();return caps;}
function can(id){const l=link();if(!linkAllows(l,id))return false;return caps.enabled&&(id==='emergency_stop'||state.live)&&caps.commands?.some(c=>c.id===id&&c.allowed);}
function why(id){const l=link();if(!linkAllows(l,id))return l.reason;if(id!=='emergency_stop'&&!state.live)return 'Go live first. Commands never run against a rewound view.';
 return (caps.enabled?caps.commands?.find(c=>c.id===id)?.reason:null)||caps.reason||'Not available right now.';}
function lockControls(root=document){for(const b of root.querySelectorAll('[data-cmd]')){const id=b.dataset.cmd,off=!can(id);
 if(off){if(!b.disabled){b.disabled=true;b.dataset.locked='1';}setWhy(b,why(id));}else{if(b.dataset.locked){b.disabled=false;delete b.dataset.locked;}setWhy(b,'');}}}
async function execute(id,extra,confirm,label=''){const c=await controls(true);if(id!=='emergency_stop'&&!state.live){toast(why(id),'alarm');return null;}if(!can(id)){toast(why(id),'alarm');return null;}
 try{const r=await fetch('/api/commands',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token||''},body:JSON.stringify({id,confirm,...extra})});
  const result=await r.json();if(!r.ok)throw Error(result.error||'Command refused without a reported reason.');
  // One plain line; a typed command's own reply from Klipper follows from consoleEcho.
  toast(id==='emergency_stop'?result.message:'Sent '+(label||id.replace(/_/g,' ')),'ok');return result;}
 catch(e){toast(e.message+' No automatic retry.','alarm');return null;}}
// The same flow as hud.js send: nothing is asked of the service for a control that cannot run; the
// exact G-code is previewed; T3 commands stop for one review unless the service says review:false.
// Klipper's answer to a typed command. The recorder pulls Moonraker's console into events; after a
// send we watch for new response lines and show each one. Stops 8 s after the last line, 10 min at
// most, so a long macro's RESPOND lines still arrive.
const recentConsole=async()=>{const t=Date.now()/1000;try{const d=await (await fetch(`/api/events?from=${Math.floor(t-900)}&to=${Math.ceil(t+900)}`)).json();return (d.events||[]).filter(e=>e.kind==='response');}catch{return [];}};
async function consoleMark(){return Math.max(0,...(await recentConsole()).map(e=>e.ts));}
async function consoleEcho(base){const seen=new Set(),start=Date.now();let last=Date.now();
 while(Date.now()-last<8000&&Date.now()-start<600000){await new Promise(r=>setTimeout(r,800));
  for(const e of (await recentConsole()).filter(e=>e.ts>base&&!seen.has(e.ts)).sort((a,b)=>a.ts-b.ts)){seen.add(e.ts);last=Date.now();
   const m=String(e.message||'').replace(/^\/\/\s?/,'').trim();if(m)toast('Klipper: '+m,/^!!|error/i.test(m)?'alarm':'ok');}}}
let reviewAnchor=null;
async function send(id,extra={}){if(!link().lock&&!can(id))await controls(true);if(!can(id)){toast(why(id),'alarm');return null;}
 if(id==='emergency_stop')return execute(id,extra,false);
 const c=await controls(true);let preview;
 try{const r=await fetch('/api/command-preview',{method:'POST',headers:{'Content-Type':'application/json',...(c.token?{'X-Stella-Control-Token':c.token}:{})},body:JSON.stringify({id,...extra})});
  preview=await r.json();if(!r.ok)throw Error(preview.error||'Exact command preview is unavailable.');}catch(e){toast(e.message,'alarm');return null;}
 if(preview.tier!=='T3'||preview.review===false){const script=(preview.script||preview.action||'').toString().trim();return execute(id,extra,preview.tier==='T3',script.split('\n')[0]);}
 return new Promise(resolve=>{let done=false;const finish=v=>{if(!done){done=true;resolve(v);}};
  openModal({title:'Review command',icon:'i-console',anchor:reviewAnchor,onClose:()=>finish(null),build:(body,close)=>{
   body.append(el('p','nx-note','Check the exact G-code. It goes to the printer once, with no automatic retry.'),el('pre','nx-review',preview.script||preview.action||''));
   const go=button({icon:'i-check',label:'Send once',kind:'primary',size:'row',cmd:id,title:'Send once',does:'Sends the G-code shown here one time.',onPress:async b=>{busy(b,true);const r=await execute(id,extra,preview.tier==='T3',String(preview.script||preview.action||'').trim().split('\n')[0]);busy(b,false);if(r){finish(r);close();}}});
   const cancel=button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Nothing is sent.',onPress:()=>close()});
   body.append(section('',el('div','nx-grid2')));body.lastChild.lastChild.append(cancel,go);lockControls(body);}});});}

// ---------- instruments ----------
const ui={};
// Instrument style: A knob by default, B bar as a setting remembered on this device.
let instStyle=(()=>{try{return localStorage.getItem('nx-inst')==='bar'?'bar':'knob';}catch{return 'knob';}})();
function buildControls(){document.body.dataset.inst=instStyle;const st=instStyle;
 for(const k of ['nozzle','bed','speed','flow','fanPart','fanHotend','fanBay'])ui[k]?.el.remove();
 ui.nozzle=control({style:st,label:'NOZZLE',icon:'i-heat',unit:'°',min:0,max:300,setMax:290,step:5,onSet:v=>send('set_heater',{target:'extruder',value:v}),onOpen:()=>sheetHeat('nozzle'),title:'Nozzle',does:'Drag to set the nozzle target; release sends it. The white mark is the live temperature. Tap for presets.'});
 ui.bed=control({style:st,label:'BED',icon:'i-bed',unit:'°',min:0,max:120,step:5,onSet:v=>send('set_heater',{target:'heater_bed',value:v}),onOpen:()=>sheetHeat('bed'),title:'Bed',does:'Drag to set the bed target; release sends it. The white mark is the live temperature. Tap for presets.'});
 ui.speed=control({style:st,label:'SPEED',icon:'i-speed',unit:'%',min:0,max:200,setMin:10,setMax:200,step:5,detents:[50,75,100,125,150],pull:4,onSet:v=>send('speed_factor',{value:v}),onOpen:()=>sheetFactor('speed'),title:'Speed factor',does:'Drag to set the speed factor (M220); it clicks into 50, 75, 100, 125 and 150%. 100% sits at the top. The bright inner bar is the actual speed against the machine limit.'});
 ui.flow=control({style:st,label:'FLOW',icon:'i-flow',unit:'%',min:50,max:150,setMin:50,setMax:150,step:1,detents:[90,95,100,105,110],pull:1.5,onSet:v=>send('extrude_factor',{value:v}),onOpen:()=>sheetFactor('flow'),title:'Flow factor',does:'Drag to set the flow factor (M221); it clicks into 90 to 110%. 100% sits at the top. The bright inner bar is the actual flow against the material limit.'});
 $('nx-left').prepend(ui.nozzle.el,ui.bed.el);$('nx-right').prepend(ui.speed.el,ui.flow.el);
 state.dockKey='';if(ui.fans)render();}
function setInstStyle(v){instStyle=v;try{localStorage.setItem('nx-inst',v);}catch{}buildControls();paintFlips();}
// The arrow says where the gauges go. Compacting pushes each side toward its wall (left arrow on
// the left, right arrow on the right); opening back out points the other way.
function paintFlips(){for(const [i,f]of (ui.flips||[]).entries()){const out=(i===0)===(instStyle==='knob');f.firstChild.replaceWith(icon(out?'i-arrow-left':'i-arrow-right'));
 tip(f,instStyle==='knob'?'Switch to bars':'Switch to knobs',instStyle==='knob'?'Folds the knobs into slim bars. The same controls; the choice is kept on this device.':'Opens the bars back out into knobs. The same controls; the choice is kept on this device.');}}
// Compact: the right column's bottom lines up with the bottom of the rail on the left.
// With no rail on screen it keeps the usual centring.
function alignCompact(){const r=$('nx-right'),rail=$('nx-rail');if(!r)return;const rb=rail?.getBoundingClientRect(),on=instStyle==='bar'&&rb&&rb.height>0;
 const want=on?Math.max(8,Math.round(innerHeight-rb.bottom))+'px':'';if(r.style.bottom!==want){r.style.bottom=want;r.style.top=on?'auto':'';r.style.transform=on?'none':'';}}
function paintReticle(){if(!ui.ret)return;const p=[...probeStore.values()].reduce((a,q)=>!a||q.ts>a.ts?q:a,null),fresh=p&&Date.now()/1000-p.ts<10;
 const t=fresh?`PROBE ${p.z>=0?'+':''}${p.z.toFixed(4)}`:'';if(ui.retProbe.textContent!==t)ui.retProbe.textContent=t;ui.ret.toggleAttribute('data-probe',!!fresh);}
function buildInstruments(){
 buildControls();
 ui.fans={part:fanRing({label:'Part',onOpen:()=>sheetFan(),onSet:v=>send('set_fan',{value:v}),title:'Part cooling fan',does:'Drag round the ring to set the part fans (both 4010 blowers); release sends M106 once. Tap for presets.'}),
  hotend:fanRing({label:'Hotend',onOpen:()=>openPartByKey('hotend_fan')||sheetFan(),title:'Hotend fan',does:'Automatic: Klipper runs it whenever the nozzle is hot. Tap to change the temperature it starts at.'}),
  controller:fanRing({label:'Controller',onOpen:()=>openPartByKey('controller_fan')||sheetFan(),title:'Electronics bay fans',does:'Automatic: Klipper runs them while the steppers or heaters are on. Tap for details.'})};
 const fans=el('div','nx-fans');fans.append(ui.fans.part.el,ui.fans.hotend.el,ui.fans.controller.el);
 $('nx-right').append(fans);
 // Knob or bar straight from the cluster.
 ui.flips=[$('nx-left'),$('nx-right')].map(c=>{const f=el('button','nx-flip');f.type='button';f.append(icon('i-arrow-down'));f.addEventListener('click',()=>setInstStyle(instStyle==='knob'?'bar':'knob'));
  import('./next-gesture.js').then(g=>g.bindPress(f));c.append(f);return f;});paintFlips();
 // Z beside the nozzle, and each probe result as it lands (the old HUD's reticle, the user
 // 2026-09-24).
 const ret=el('div','nx-reticle');ret.hidden=true;ui.retZ=el('b');ui.retProbe=el('span');ret.append(ui.retZ,ui.retProbe);$('nx-ui').append(ret);ui.ret=ret;
 document.addEventListener('stella-track',e=>{const {x,y,on}=e.detail;ret.hidden=!on||document.body.dataset.layout==='phone';if(on)ret.style.transform=`translate(${Math.round(x)}px,${Math.round(y)}px)`;});
 // position capsule
 const pos=$('nx-pos');pos.classList.add('nx-glass');ui.axes={};for(const a of ['X','Y','Z']){const w=el('div','nx-axis');w.append(el('b','',a),el('span','','—'));ui.axes[a]=w;pos.append(w);}
 const sp=el('div','nx-pos-speed');ui.vel=el('span','','—');sp.append(ui.vel,el('small','','mm/s'));pos.append(el('i','nx-pos-sep'),sp);
 // More useful numbers by the position — the Z offset in use, the acceleration limit, the top
 // speed, and whether the gantry is levelled.
 const xi=(k,label,tt)=>{const w=el('div','nx-pos-x');w.dataset.k=k;w.title=tt;const v=el('span','','—');w.append(el('small','',label),v);return [w,v];};
 const [zo,zov]=xi('zoff','Z OFS','Z offset in use (baby steps included)'),[ac,acv]=xi('acc','ACCEL','Acceleration limit, mm/s²'),[mv,mvv]=xi('vmax','MAX','Velocity limit, mm/s'),[ql,qlv]=xi('qgl','QGL','Quad gantry level applied since homing');
 ui.px={zo:zov,ac:acv,mv:mvv,ql,qlv};pos.append(el('i','nx-pos-sep nx-pos-x'),zo,ac,mv,ql);
 tip(pos,'Toolhead position','Klipper\'s requested position in mm, smoothed between recorded samples. A green axis letter means that axis is homed.');
 // phone pager: page 0 temperatures, page 1 motion
 const pager=$('nx-pager');pager.hidden=false;ui.page=0;for(const [i,name]of ['Temperatures','Motion'].entries()){const b=el('button');b.type='button';b.setAttribute('aria-label',name);b.addEventListener('click',()=>{ui.page=i;paintPager();});pager.append(b);}
 let sx=null;for(const c of [$('nx-left'),$('nx-right')]){c.addEventListener('pointerdown',e=>{sx=e.clientX;});c.addEventListener('pointerup',e=>{if(sx!==null&&Math.abs(e.clientX-sx)>50){ui.page=e.clientX<sx?1:0;paintPager();}sx=null;});}
 // rail
 const rail=$('nx-rail');rail.classList.add('nx-glass');
 // Rail: the pages open as NEXT sheets; the four the old service pages still own are kept below
 // the second line, marked classic. Rail: the everyday pages on the rail, everything in Pages (the
 // last icon).
 const pages=[['i-nav-hud','Printer HUD','/','You are here: the printer, its instruments and every control.',1],['i-print','Print','sheet:print','The job on the plate: progress, the layer, Z offset and skip an object.',1],['i-filament-load','Filament','sheet:filament','Load and unload: heats, waits, then asks before it moves filament.',1],['i-view','Camera','sheet:camera','The printer camera, live.',1],['i-nav-leveling','Gantry & bed','sheet:level','Bed mesh as a map and a 3D surface, compare two meshes, build sheets, Z offset and every gantry level run.',1],['i-nav-signals','Input shaper','sheet:shaper','Resonance graphs, the recommended shaper and belt tension.',1],['i-nav-thermal','Heat & fans','sheet:thermal','Nozzle and bed over the last ten minutes, heater output and the fans.',1],
  ['i-rewind','Print history','sheet:history','Every print, how it went, and the totals.',1],['i-nav-maintenance','Maintenance','sheet:maintenance','Reminders by hours printed.',1],['i-warn','Alerts','sheet:alerts','The last seven days: printer offline, driver warnings, Klipper errors, retransmits.',1],['i-link','Phone & WhatsApp','sheet:phone','Pair your phone, and the WhatsApp alerts.',1],
  ['i-nav-workspace','Motion & motors','sheet:motion','Speed, Z and every stepper driver.',0],['i-nav-signals','Signals','sheet:signals','Every recorded signal, live, with a filter.',0],['i-nav-parts','Parts','sheet:parts','Every part on the printer; the live ones open their panel.',0],['i-nav-electronics','Electronics','sheet:electronics','Controller, toolhead board, host, relay and fans.',0],['i-nav-timeline','Timeline','sheet:timeline','Everything the printer said, in order.',0],['i-nav-diagnostics','Suspected causes','sheet:causes','Recorded warnings and the safe checks for each.',0],['i-panels','Service journal','sheet:journal','Work recorded against each part.',0],['i-nav-assembly','Assembly','sheet:assembly','The CAD Stella is drawn from, with every source file.',0],['i-more','All pages','sheet:pages','Every page.',1]];
 ui.pages=pages.filter(p=>p[2]!=='sheet:pages');let railed=0;for(const [ico,name,href,does,onRail]of pages){if(!onRail)continue;if(railed===1||railed===7||href==='sheet:pages')rail.append(el('hr'));railed++;const b=button({icon:ico,size:'icon',title:name,does,onPress:btn=>{if(href.startsWith('sheet:')){if(btn.hasAttribute('data-showing')){closeSheet();return;}Promise.resolve(openNamedSheet(href.slice(6))).then(()=>markOpener(btn));}else if(href!=='/')location.href=href;}});if(href==='/')b.setAttribute('aria-current','page');rail.append(b);}
 // G-code command bar (top bar, left of centre)
 const cmdHost=el('div','nx-cmd-host');document.querySelector('.nx-id').after(cmdHost);
 ui.cmd=commandBar({host:cmdHost,locked:()=>link().lock?link().reason:'',
  source:async()=>{const get=u=>fetch(u).then(r=>r.ok?r.json():{}).catch(()=>({}));
   const [help,mac,hist]=await Promise.all([link().lock?{}:get('/api/console/help'),link().lock?{}:get('/api/macros'),get('/api/console/history')]);
   const macros=new Set((mac.macros||[]).filter(m=>m.macro===true).map(m=>m.name.toUpperCase()));
   const commands=(help.commands||help.help||[]).map(h=>({name:h.name,help:h.help||'',macro:macros.has(String(h.name).toUpperCase())}));
   for(const m of mac.macros||[])if(m.macro===true&&!commands.some(c=>c.name.toUpperCase()===m.name.toUpperCase()))commands.push({name:m.name,help:m.help||'',macro:true});
   const history=[...new Set((hist.entries||hist.history||[]).map(h=>typeof h==='string'?h:h.script||h.line||'').filter(Boolean).reverse())].slice(0,20);
   const act=(label,icon,go,help)=>({label,icon,go,help});
   const actions=[act('Home all axes','i-home',()=>send('home'),'G28'),act('Quad gantry level','i-level',()=>send('qgl'),'QUAD_GANTRY_LEVEL'),act('Bed mesh','i-mesh',()=>send('mesh'),'BED_MESH_CALIBRATE'),
    act('Motors off','i-motors-off',()=>send('motors_off'),'M84'),act('Firmware restart','i-fw-restart',()=>send('firmware_restart'),'FIRMWARE_RESTART'),
    ...preheatMenu(state.presets).map(p=>act(p.fixed?'Cool down':'Preheat '+p.name,'i-heat',async()=>{if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});},p.fixed?'both heaters off':`${p.nozzle}° / ${p.bed}°`)),
    act('Files','i-files',()=>sheetFiles(),'print a file'),act('Console','i-console',()=>sheetConsole(),'console log'),act('Macros','i-macro',()=>sheetMacros(),'your macros')];
   return {commands,history,actions};},
  run:async it=>{if(it.kind==='action'){await it.go();return true;}const base=await consoleMark();reviewAnchor=document.querySelector('.nx-cmd-box');const r=await send('console',{script:it.script}).finally(()=>{reviewAnchor=null;});if(r){jsend('/api/console/history','POST',{line:it.script}).catch(()=>{});consoleEcho(base);}return !!r;}});
 // top bar
 {const cb=button({icon:'i-camera',size:'icon',kind:'ghost',title:'Camera',does:'Shows the live camera over the printer. Drag its handle to move it; press again to hide it.',onPress:()=>setCamPinned(!camPinned)});cb.id='nx-camtoggle';cb.setAttribute('aria-pressed',String(camPinned));$('nx-state').before(cb);}
 // One phone icon on the main screen. "Pair phone" until a phone is paired, then a phone with a
 // check mark. Desktop only (a paired phone cannot pair others); opens the Phone sheet.
 {const pb=button({icon:'i-phone',label:'Pair phone',size:'row',kind:'ghost',title:'Phone',does:'Pair a phone to watch and control this printer from anywhere, or see which phones are paired.',onPress:()=>sheetPhone()});pb.id='nx-phonebtn';pb.hidden=true;$('nx-state').before(pb);}
 const est=$('nx-estop');tip(est,'Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call. No review. Heaters and motors cut out; the printer then needs a firmware restart.');
 est.addEventListener('click',async()=>{if(est.disabled)return;est.disabled=true;await send('emergency_stop');lockControls();});
 import('./next-gesture.js').then(g=>{g.bindPress(est);g.bindPress($('nx-state'));});
 $('nx-state').addEventListener('click',()=>{const l=link();toast(l.reason||`${l.label}. Moonraker answers and Klipper is ready.`,l.lock?'alarm':'ok');});
}
function paintPager(){const ph=document.body.dataset.layout==='phone';$('nx-left').toggleAttribute('data-page-off',ph&&ui.page!==0);$('nx-right').toggleAttribute('data-page-off',ph&&ui.page!==1);
 [...$('nx-pager').children].forEach((b,i)=>b.setAttribute('aria-current',String(i===ui.page)));$('nx-pager').hidden=!ph;}
function paintPos(pos){const h=state.readings.homed;if(ui.retZ){const z=pos?.[2],t=h.z&&finite(z)!==null?'Z '+z.toFixed(2):'Z —';if(ui.retZ.textContent!==t)ui.retZ.textContent=t;}['X','Y','Z'].forEach((a,i)=>{const v=pos?.[i],w=ui.axes[a];const txt=h[a.toLowerCase()]&&finite(v)!==null?v.toFixed(1):'—';if(w.lastChild.textContent!==txt)w.lastChild.textContent=txt;w.toggleAttribute('data-homed',h[a.toLowerCase()]);});}

// ---------- render: mode, dock, dials, job, chip ----------
let renderPending=false,lastUi=0;
function renderSoon(){if(renderPending)return;const wait=Math.max(0,120-(performance.now()-lastUi));renderPending=true;setTimeout(()=>{renderPending=false;render();},wait);}
function render(){lastUi=performance.now();if(!ui.nozzle)return;const r=state.readings,l=link();
 const {mode,heating}=deriveMode({link:l,live:state.live,readings:r});state.mode=mode;document.body.dataset.mode=mode;
 const stale=['off','service','checking','klipper'].includes(mode)||!state.live;document.body.toggleAttribute('data-stale',stale);const lastSeen=stale&&l.since?'last seen '+l.since+' ago':null;
 // state chip
 const chip=$('nx-state'),cs=controlsSummary(l,{live:state.live,enabled:caps.enabled});
 chip.dataset.state=!state.live&&l.state!=='replay'&&l.state!=='service'?'rewound':l.state;
 chip.toggleAttribute('data-heating',heating&&l.state==='ready'&&mode!=='printing');chip.querySelector('.nx-state-label').textContent=!state.live&&l.state!=='replay'?'REWOUND':l.state==='ready'?(mode==='printing'?'PRINTING':mode==='paused'?'PAUSED':mode==='finished'?(r.job.state==='complete'?'COMPLETE':r.job.state.toUpperCase()):heating?'HEATING':mode==='unhomed'?'NOT HOMED':'READY'):l.label;
 if(document.body.dataset.layout==='phone'){const lab=chip.querySelector('.nx-state-label');lab.textContent={'KLIPPER NOT READY':'HALTED','PRINTER OFF':'OFF','SERVICE OFF':'NO SERVICE','NOZZLE OVER TEMP':'NOZZLE HOT','BED OVER TEMP':'BED HOT'}[lab.textContent]||lab.textContent;}
 chip.querySelector('.nx-state-since').textContent=['off','service'].includes(l.state)&&l.since?l.since:'';
 tip(chip,cs.badge,l.reason||cs.line||'Moonraker answers and Klipper is ready.');
 // Over-temperature is a safety event, not a colour: the chip turns to alarm and says which heater,
 // and one toast fires per episode (heatColor 'danger': above the heater's limit or 15° over target).
 const hot=!document.body.hasAttribute('data-stale')&&['nozzle','bed'].find(k=>heatColor(r[k].temp,r[k].target,k).tone==='danger');
 chip.toggleAttribute('data-alarm',!!hot);if(hot){chip.querySelector('.nx-state-label').textContent=document.body.dataset.layout==='phone'?hot.toUpperCase()+' HOT':hot.toUpperCase()+' OVER TEMP';
  tip(chip,hot==='nozzle'?'Nozzle over temperature':'Bed over temperature',`${fmtTemp(r[hot].temp)} against a target of ${fmtTemp(r[hot].target)}. Check the heater; E-STOP cuts it at once.`);}
 // The over-temperature card itself comes from checkAlerts (anything wrong pops up).
 // dials
 // Instruments (style A knob / B bar, one model): value = what is set, cur = heater reading,
 // live = actual use on the same scale (speed: velocity against the 100% = max velocity mark; flow:
 // volumetric against the material limit at the 100% mark), act = the face's ghost glow.
 const hn=heatColor(r.nozzle.temp,r.nozzle.target,'nozzle'),hb=heatColor(r.bed.temp,r.bed.target,'bed'),fl=flowModel(r,state.presets?.loaded?.name,state.presets?.hotend_max_flow);
 const heatNow=(t,tone)=>t===null?'NO READING':lastSeen?lastSeen.toUpperCase():`NOW ${Math.round(t)}°`;
 ui.nozzle.set({value:stale?null:r.nozzle.target,cur:r.nozzle.temp,col:hn.tone==='unknown'?'var(--ink-3)':hn.ink,act:hn.tone==='danger'?1:hn.glow,now:heatNow(r.nozzle.temp)});
 ui.bed.set({value:stale?null:r.bed.target,cur:r.bed.temp,col:hb.tone==='unknown'?'var(--ink-3)':hb.ink,act:hb.tone==='danger'?1:hb.glow,now:heatNow(r.bed.temp)});
 const vmax=r.maxVelocity||300,vel=r.velocity;
 // Speed and flow read 100 % with the printer off. Klipper resets both on a restart, so an
 // old reading is not today's factor: with no live printer they read unknown.
 ui.speed.set({value:r.speedFactor===null||stale?null:r.speedFactor*100,live:vel===null?null:vel/vmax*100,act:vel===null?0:vel/vmax,col:'var(--accent)',now:vel===null||stale?'—':`${Math.round(vel)} MM/S`});
 ui.flow.set({value:r.extrudeFactor===null||stale?null:r.extrudeFactor*100,live:fl.frac===null?null:50+50*fl.frac,act:fl.frac??0,
  col:fl.tone==='over'?'var(--alarm)':fl.tone==='high'?'var(--heat)':'var(--accent)',now:fl.value===null||stale?'—':`${fl.value.toFixed(1)} MM³/S`});
 for(const [k,cmd]of [['nozzle','set_heater'],['bed','set_heater'],['speed','speed_factor'],['flow','extrude_factor']])ui[k].lock(can(cmd)?'':why(cmd));
 // The printer was off and the controller fan still read 100 %. A stale reading is not a fan
 // speed: with no live printer the rings read unknown and the drawn fans stop.
 const fanOf=f=>stale?{speed:null}:f;
 ui.fans.part.set(fanOf(r.fans.part));ui.fans.hotend.set(fanOf(r.fans.hotend));ui.fans.controller.set(fanOf(r.fans.controller));ui.fans.part.lock(can('set_fan')?'':why('set_fan'));
 if(stale&&state.sceneStale!==true&&scene?.live){scene.live({});state.sceneStale=true;}else if(!stale)state.sceneStale=false;
 if(!state.live)paintPos([r.position.x,r.position.y,r.position.z]);else if(!lastTick)paintPos(null);
 ui.vel.textContent=r.velocity===null?'—':String(Math.round(r.velocity));
 {const P=ui.px,st=state.mode==='off';P.zo.textContent=st||r.zOffset===null?'—':(r.zOffset>=0?'+':'')+r.zOffset.toFixed(3);
  P.ac.textContent=st||r.maxAccel===null?'—':r.maxAccel>=1000?(r.maxAccel/1000).toFixed(r.maxAccel%1000?1:0)+'k':String(Math.round(r.maxAccel));
  P.mv.textContent=st||r.maxVelocity===null?'—':String(Math.round(r.maxVelocity));
  P.qlv.textContent=st||r.qgl===null?'—':r.qgl?'✓':'✗';P.ql.dataset.state=st||r.qgl===null?'':r.qgl?'ok':'no';}
 // job capsule
 paintJob();
 // dock
 const lay=document.body.dataset.layout,compact=['phone','phone-land','kiosk'].includes(lay),max=lay==='kiosk'?6:lay==='phone-land'?4:compact?5:8;
 const jobName=r.job.name,key=[mode,compact,max,jobName,l.state].join('|');
 if(key!==state.dockKey){state.dockKey=key;const SHORT={fw_restart:'FW RESTART',klipper_restart:'RESTART',reprint:'AGAIN'};
  const slots=dockFor(mode,{compact,maxSlots:max,jobName}).map(s=>({...s,label:compact&&SHORT[s.key]||s.label,attention:s.primary&&mode==='unhomed',...DOCK_TEXT[s.key]}));
  state.dock=slots;const msg=mode==='service'?{icon:'i-warn',text:'Stella Scope service is not running. Trying again by itself.'}:mode==='checking'?{icon:'i-live',text:'Connecting to the printer…'}:null;
  renderDock($('nx-dock'),slots,{press:dockPress,message:msg});}
 lockControls();paintPager();
 if(scene)scene.stage?.(mode==='printing'?'printing':heating?'heating':l.state==='not_ready'?'error':'idle');
 paintReticle();paintPages();alignCompact();paintActivity();toolpathAuto();toolpathTick();paintProbeMarks();meshOverlayTick();paintProbeLed();checkAlerts();paintCamPin();paintMaintenance();paintEye();filamentTick();}
// Failure watch (stella-eye, Obico's model on the camera, the user 2026-09-25): a chip in the
// top bar while a print is watched. Green OK, amber LOOK, red FAILING; tap for the camera.
let eyeAt=0,eyeData=null;function paintEye(){if(performance.now()-eyeAt>8000){eyeAt=performance.now();fetch('/api/eye').then(r=>r.ok?r.json():null).then(d=>{eyeData=d;paintEye();}).catch(()=>{});}
 let chip=$('nx-eye');const d=eyeData,show=!!(d&&d.enabled&&d.watching&&state.mode==='printing');
 if(!chip){if(!show)return;chip=button({icon:'i-view',label:'WATCH',size:'icon',kind:'ghost',title:'Failure watch',does:'Obico\'s spaghetti detector looks at the camera every 10 s. It warns here and on WhatsApp; it never stops a print by itself. Tap for the camera.',onPress:()=>pageCamera()});chip.id='nx-eye';chip.append(el('span','nx-lbl',''));$('nx-state').before(chip);}
 chip.hidden=!show;if(!show)return;const v=d.verdict||'ok',pct=Math.round((d.normalized||0)*100);chip.dataset.v=d.learning?'learning':v;
 const lbl=chip.querySelector('.nx-lbl');if(lbl)lbl.textContent=d.learning?'WATCH · LEARNING':v==='failing'?`FAILING ${pct}%`:v==='warning'?`LOOK ${pct}%`:`OK ${pct}%`;}
const DOCK_TEXT={home:{title:'Home',does:'Homes X, Y and Z (G28).'},level:{title:'Level the gantry',does:'Quad gantry level: probes the four corners and levels the gantry.'},
 mesh:{title:'Bed mesh',does:'Probes the bed and makes a fresh mesh (BED_MESH_CALIBRATE).'},move:{title:'Move',does:'Opens the jog pad: X, Y and Z by 0.1 to 50 mm.'},
 heat:{title:'Heat',does:'Nozzle and bed targets, your saved presets and the preset editor.'},files:{title:'Files',does:'Your G-code files with their thumbnails. Print now or queue.'},
 macros:{title:'Macros',does:'Your Klipper macros. Every run asks once.'},more:{title:'More',does:'Everything else: restarts, motors off, console, recording, view and pages.'},
 pause:{title:'Pause',does:'Pauses the print.'},resume:{title:'Resume',does:'Resumes the paused print.'},cancel:{title:'Cancel print',does:'Cancels the running print.'},
 speed:{title:'Speed factor',does:'Scales every move (M220).'},flow:{title:'Flow factor',does:'Scales every extrusion (M221).'},fan:{title:'Part fan',does:'Part cooling fan speed.'},
 exclude:{title:'Exclude objects',does:'Skip one object of this print.'},reprint:{title:'Print again',does:'Starts the same file again. Asks once.'},
 fw_restart:{title:'Firmware restart',does:'Restarts Klipper and its MCUs (FIRMWARE_RESTART). The way out of a halted Klipper. Asks once.'},
 klipper_restart:{title:'Klipper restart',does:'Restarts the Klipper host (RESTART). Asks once.'},console:{title:'Console',does:'The printer\'s console and a line to type G-code.'},
 go_live:{title:'Go live',does:'Returns to the live printer.'},rec_back:{title:'Back',does:'One recorded sample back.'},rec_fwd:{title:'Forward',does:'One recorded sample forward.'},
 rec_play:{title:'Play',does:'Plays the recording from here.'},recording:{title:'Recording',does:'The timeline: scrub, play, speed and a link to this moment.'}};
// Idle strip: with no job the top bar reads the machine at a glance — temperatures, how long since
// the last print ended, the filament recorded as loaded and the sheet on the bed.
let lastEnd=null,lastEndAt=0;
function paintIdle(jobShown){const host=$('nx-idle'),r=state.readings,show=!jobShown&&['idle','unhomed','off','finished'].includes(state.mode);host.hidden=!show;if(!show)return;
 if(Date.now()-lastEndAt>300e3){lastEndAt=Date.now();fetch('/api/print/history?limit=5').then(x=>x.ok?x.json():null).then(d=>{const js=d?.jobs||[],j=js.find(q=>finite(q.end_time)!==null)||js[0];lastEnd=finite(j?.end_time)??finite(j?.start_time);}).catch(()=>{});}
 if(!ui.idle){ui.idle={};for(const [k,ico,tt]of [['noz','i-heat','Nozzle now'],['bed','i-bed','Bed now'],['since','i-rewind','Since the last print ended'],['fil','i-filament-load','Filament recorded as loaded'],['sheet','i-level','Build sheet on the bed']]){
   const it=el('span','nx-idle-item');it.dataset.k=k;it.title=tt;const v=el('b');it.append(icon(ico),v);host.append(it);ui.idle[k]=v;}}
 const off=state.mode==='off',t=v=>off||finite(v)===null?'—':Math.round(v)+'°',pr=state.presets||{};
 ui.idle.noz.textContent=t(r.nozzle.temp);ui.idle.bed.textContent=t(r.bed.temp);
 ui.idle.since.textContent=lastEnd?'idle '+fmtDuration(Date.now()/1000-lastEnd):'no prints yet';
 ui.idle.fil.textContent=pr.loaded?.name||'not set';ui.idle.sheet.textContent=pr.sheet?.name||'not set';}
function paintJob(){const j=jobModel(state.readings),host=$('nx-job');const show=!['off','klipper','checking'].includes(state.mode)&&(j.active||(state.mode==='finished'&&j.title));host.hidden=!show;paintIdle(show);if(!show)return;
 if(!host.firstChild){host.className='nx-job nx-glass';const img=el('img','nx-job-thumb');img.alt='';img.addEventListener('error',()=>{img.style.visibility='hidden';});
  const text=el('div','nx-job-text');ui.jobTitle=el('div','nx-job-title');ui.jobMeta=el('div','nx-job-meta');text.append(ui.jobTitle,ui.jobMeta);
  const ring=document.createElementNS('http://www.w3.org/2000/svg','svg');ring.setAttribute('viewBox','0 0 40 40');ring.classList.add('nx-job-ring');ring.innerHTML='<circle class="track" cx="20" cy="20" r="17"/><circle class="fill" cx="20" cy="20" r="17" transform="rotate(-90 20 20)"/>';
  ui.jobPct=el('div','nx-job-pct');host.append(img,text,ui.jobPct,ring);ui.jobImg=img;ui.jobRing=ring.lastChild;const C=2*Math.PI*17;ui.jobRing.setAttribute('stroke-dasharray',C);ui.jobC=C;
  tip(host,'Current print','File, progress and time left. Tap for the print page: the layer, and skip an object.');host.setAttribute('role','button');host.tabIndex=0;host.addEventListener('click',()=>pagePrint());}
 if(ui.jobFile!==state.readings.job.name){ui.jobFile=state.readings.job.name;ui.jobImg.style.visibility='';ui.jobImg.src='/api/print/thumbnail?filename='+encodeURIComponent(ui.jobFile);}
 ui.jobTitle.textContent=j.title;ui.jobMeta.textContent=state.mode==='finished'?state.readings.job.state.toUpperCase():`${fmtDuration(j.elapsed)} in · ${j.remaining===null?'—':fmtDuration(j.remaining)} left`;
 ui.jobPct.textContent=j.progress===null?'—':Math.round(j.progress*100)+'%';ui.jobRing.setAttribute('stroke-dashoffset',ui.jobC*(1-(j.progress||0)));}

// ---------- dock and radial actions ----------
async function dockPress(slot,btn){const a=slot.action||'';
 // Pressing the button that opened a sheet, pop-up or radial again closes it.
 if(/^(sheet|menu|radial):/.test(a)&&btn?.hasAttribute('data-showing')){closeSheet();closePopover();$('nx-radial').hidden=true;$('nx-radial').replaceChildren();clearOpener();return;}
 if(slot.run){busy(btn,true);try{await slot.run();}finally{busy(btn,false);}return;}
 // Every dock button with a handful of choices opens them as a radial round the button: Home,
 // Level, and the quick presets of Heat, Speed, Flow and Fan (each with its full sheet as the last
 // spoke). Files, macros, the console and the jog pad keep their sheets: long lists and pads do not
 // fit on a ring.
 const quick=quickRadial(slot.key);if(quick||a.startsWith('menu:')){const items=quick||SUB_MENUS[a.slice(5)]||[];const r=btn.getBoundingClientRect();closePopover();closeSheet();
  openRadial(items,{x:r.left+r.width/2,y:r.top-190,onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls($('nx-radial'));return;}
 if(a.startsWith('send:')){busy(btn,true);try{await send(a.slice(5));}finally{busy(btn,false);}return;}
 if(a==='reprint'){busy(btn,true);try{await send('print_start',{filename:state.readings.job.name});}finally{busy(btn,false);}return;}
 if(a.startsWith('sheet:')){closePopover();await openNamedSheet(a.slice(6));if(!$('nx-sheet').hidden)markOpener(btn);return;}
 if(a.startsWith('menu:')){const menu=SUB_MENUS[a.slice(5)]||[];const pop=openPopover(btn,menu,{title:slot.title||slot.label,onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls(pop);return;}
 if(a.startsWith('script:')){busy(btn,true);try{await send('console',{script:a.slice(7)});}finally{busy(btn,false);}return;}
 if(a.startsWith('rec:'))return recAction(a.slice(4));
 if(a==='radial:more'){const r=btn.getBoundingClientRect();const items=moreMenu(state.mode,state.dock).filter((it,i,a)=>a.findIndex(x=>x.key===it.key)===i).map(i=>({...i,...DOCK_TEXT[i.key],title:DOCK_TEXT[i.key]?.title||i.label[0]+i.label.slice(1).toLowerCase()}));
  closeSheet();const rad=openRadial(items,{x:r.left+r.width/2,y:r.top-190,onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls($('nx-radial'));return rad;}}
function quickRadial(key){const both=async p=>{if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});};
 if(key==='heat')return [...preheatMenu(state.presets).map(p=>({key:'pre-'+p.name,icon:p.fixed?'i-close':'i-heat',label:p.fixed?'COOL':p.name.toUpperCase(),sub:p.fixed?'all off':`${p.nozzle}° · ${p.bed}°`,tone:p.fixed?'cool':'heat',cmd:'set_heater',title:p.fixed?'Cool down':'Preheat '+p.name,does:p.fixed?'Both heaters off.':`Nozzle ${p.nozzle}°, bed ${p.bed}°.`,run:()=>both(p)})),
  {key:'heat-sheet',icon:'i-panels',label:'MORE',title:'All heat settings',does:'Targets, presets and the preset editor.',action:'sheet:heat'}];
 const f=(k,id,vals,unit,sheet,scale=1)=>[...vals.map(v=>({key:k+v,icon:k==='fan'?'i-fan':k==='speed'?'i-speed':'i-flow',label:v+unit,cmd:id,title:`${k[0].toUpperCase()+k.slice(1)} ${v}${unit}`,does:`Sets the ${k} to ${v}${unit}.`,run:()=>send(id,{value:v*scale})})),
  {key:k+'-sheet',icon:'i-panels',label:'MORE',title:'More',does:'The full '+k+' sheet.',action:'sheet:'+sheet}];
 if(key==='speed')return f('speed','speed_factor',[50,75,100,125,150],'%','speed');
 if(key==='flow')return f('flow','extrude_factor',[90,95,100,105,110],'%','flow');
 if(key==='fan')return f('fan','set_fan',[0,25,50,75,100],'%','fan',1/100);
 return null;}
function openNamedSheet(name){return ({heat:()=>sheetHeat('nozzle'),move:sheetMove,files:sheetFiles,macros:sheetMacros,console:sheetConsole,speed:()=>sheetFactor('speed'),flow:()=>sheetFactor('flow'),
 fan:sheetFan,exclude:sheetExclude,recording:sheetRecording,view:sheetView,pages:sheetPages,level:pageLevel,thermal:pageThermal,signals:pageSignals,timeline:pageTimeline,parts:()=>pageParts(),electronics:()=>pageParts('electronics'),phone:sheetPhone,print:pagePrint,filament:pageFilament,camera:pageCamera,shaper:pageShaper,history:pageHistory,maintenance:pageMaintenance,alerts:pageAlerts,causes:pageCauses,journal:()=>pageJournal(),assembly:pageAssembly,motion:pageMotion}[name]||(()=>toast('Not available yet.')))();}
function recAction(v){const at=state.snapshot?.point?.ts??state.cursor;
 if(v==='live'){state.live=true;state.playing=false;request(null);return;}
 if(v==='back'||v==='fwd'){state.live=false;state.playing=false;request(at,v==='back'?'previous':'next');return;}
 if(v==='play'){state.live=false;state.playing=!state.playing;if(state.playing&&finite(state.cursor)===null)state.cursor=state.snapshot?.first_ts;render();}}

// ---------- sheets ----------
async function loadPresets(){try{const r=await fetch('/api/presets');const d=await r.json();if(r.ok)state.presets=d;}catch{}return state.presets;}
async function jsend(url,method,body){const c=await controls();const r=await fetch(url,{method,headers:{'Content-Type':'application/json',...(c.token?{'X-Stella-Control-Token':c.token}:{})},body:JSON.stringify(body)});
 const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`HTTP ${r.status}`);return d;}
async function sheetHeat(which='nozzle'){await loadPresets();const r=state.readings;
 openSheet({title:'Heat',icon:'i-heat',build:body=>{
  const noz=stepper({value:Math.round(r.nozzle.target||0),unit:'°',step:5,max:290}),bed=stepper({value:Math.round(r.bed.target||0),unit:'°',step:5,max:120});
  const setN=button({icon:'i-heat',label:'Set nozzle',kind:'primary',size:'row',cmd:'set_heater',title:'Set nozzle target',does:'Sends the nozzle target shown above.',onPress:async b=>{busy(b,true);await send('set_heater',{target:'extruder',value:noz.get()});busy(b,false);}});
  const setB=button({icon:'i-bed',label:'Set bed',kind:'primary',size:'row',cmd:'set_heater',title:'Set bed target',does:'Sends the bed target shown above.',onPress:async b=>{busy(b,true);await send('set_heater',{target:'heater_bed',value:bed.get()});busy(b,false);}});
  const n=section('Nozzle  ·  now '+fmtTemp(r.nozzle.temp),noz,setN),b=section('Bed  ·  now '+fmtTemp(r.bed.temp),bed,setB);
  const tiles=el('div','nx-grid2');for(const p of preheatMenu(state.presets)){const t=button({icon:p.fixed?'i-close':'i-heat',label:p.name,size:'tile',cmd:'set_heater',sub:p.fixed?'heaters off':`${p.nozzle}° · ${p.bed}°`,
   title:p.fixed?'Cool down':'Preheat '+p.name,does:p.fixed?'Sets both heaters to 0.':`Sets the nozzle to ${p.nozzle}° and the bed to ${p.bed}°.`,
   onPress:async btn=>{busy(btn,true);if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});busy(btn,false);}});tiles.append(t);}
  const edit=button({icon:'i-panels',label:'Edit presets',size:'row',title:'Edit presets',does:'Add, rename, change or delete your saved presets. Nothing is kept until Save.',onPress:()=>sheetPresetEditor()});
  body.append(section('Presets',tiles,edit),which==='bed'?b:n,which==='bed'?n:b);lockControls(body);}});}
function sheetPresetEditor(){const saved=(state.presets?.materials||[]).map(m=>({...m}));let rows=saved.map((m,i)=>({...m,orig:i}));
 openSheet({title:'Presets',icon:'i-panels',build:body=>{const list=el('div','nx-list'),status=el('p','nx-note');
  const paint=()=>{list.replaceChildren();const diff=presetDiff(saved,rows);
   rows.forEach((row,i)=>{const w=el('div','nx-item');const name=el('input','nx-input');name.value=row.name;name.setAttribute('aria-label','Preset name');
   const nz=el('input','nx-input');nz.type='number';nz.value=row.nozzle??'';nz.setAttribute('aria-label','Nozzle °C');const bd=el('input','nx-input');bd.type='number';bd.value=row.bed??'';bd.setAttribute('aria-label','Bed °C');
   for(const[f,k]of[[name,'name'],[nz,'nozzle'],[bd,'bed']])f.addEventListener('input',()=>{row[k]=k==='name'?f.value:f.value===''?null:Number(f.value);status.textContent=presetDiff(saved,rows).changed?'Unsaved changes.':'';});
    name.style.flex='2';nz.style.flex='1';bd.style.flex='1';
    const del=button({icon:'i-close',size:'icon',kind:'danger',title:'Delete preset',does:'Removes this preset when you press Save.',onPress:()=>{rows.splice(i,1);paint();}});
    w.append(name,nz,bd,del);if(diff.states[i]!=='same')w.style.borderColor='rgba(4,217,196,.35)';list.append(w);});
   status.textContent=diff.changed?`Unsaved changes${diff.removed?`, ${diff.removed} removed`:''}.`:'Saved.';};
  const add=button({icon:'i-arrow-down',label:'Add preset',size:'row',title:'Add preset',does:'Adds an empty row.',onPress:()=>{rows.push({name:'',nozzle:null,bed:null,orig:null});paint();}});
  const save=button({icon:'i-check',label:'Save',kind:'primary',size:'row',title:'Save presets',does:'Stores the presets in Stella Scope.',onPress:async b=>{busy(b,true);try{if(rows.some(r=>!String(r.name).trim()||r.nozzle===null||r.bed===null))throw Error('Enter a name, nozzle temperature and bed temperature for every preset.');await jsend('/api/presets','PUT',{...(state.presets||{}),materials:rows.map(({name,nozzle,bed})=>({name:String(name).trim(),nozzle:Number(nozzle),bed:Number(bed)}))});await loadPresets();toast('Presets saved.','ok');closeSheet();}catch(e){status.textContent=e.message;}busy(b,false);}});
  const cancel=button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Discards the unsaved changes.',onPress:()=>closeSheet()});
  const actions=el('div','nx-grid2');actions.append(cancel,save);paint();body.append(el('p','nx-note','Name, nozzle °C, bed °C. Nothing is kept until Save. Cool down is built in.'),list,add,status,actions);}});}
// Move: a real jog console. Live position with homed axes lit, one step bar, a round XY pad
// labelled with where the head actually goes on a Voron (Y+ is toward the back), a tall Z rocker (Z
// moves the gantry: Z+ lifts the nozzle), and the homing and parking moves beside them. Arrow keys
// jog X/Y, Page Up/Down jog Z while it is open.
function sheetMove(){openSheet({title:'Move',icon:'i-move',wide:true,build:async(body,host)=>{await loadPresets();
 const steps=[...new Set([0.1,1,10,50,...(state.presets?.steps||[])])].filter(v=>v>0&&v<=100).sort((a,b)=>a-b);let step=steps.includes(10)?10:steps[0];
 const jog=(axis,sign)=>send('jog',{axis,value:sign*step});const script=(line,label)=>send('console',{script:line}).then(r=>{if(r)toast('Sent '+label,'ok');});
 const travel=()=>xyTravel((state.live?lastTick?.point:state.snapshot?.point)?.limits);
 // live position strip
 const pos=el('div','nx-mv-pos'),cell={};for(const a of ['X','Y','Z']){const c=el('div','nx-mv-axis');c.dataset.axis=a;const v=el('b','','—');c.append(el('small','',a),v,el('i','nx-mv-homed','not homed'));cell[a]={c,v};pos.append(c);}
 const vel=el('div','nx-mv-axis nx-mv-vel');const vv=el('b','','—');vel.append(el('small','','SPEED'),vv,el('i','nx-mv-homed','mm/s'));pos.append(vel);
 // step bar
 const stepBar=segmented(steps,step,v=>{step=v;paintSteps();},v=>v+' mm');stepBar.classList.add('nx-mv-steps');
 // XY pad
 const pad=el('div','nx-mv-pad');pad.tabIndex=0;pad.setAttribute('role','group');pad.setAttribute('aria-label','Move keyboard pad. Arrow keys move X and Y; Page Up and Page Down move Z.');const dir=(cls,ico,axis,sign,big,small)=>{const b=el('button','nx-mv-dir '+cls);b.type='button';b.dataset.cmd='jog';b.append(icon(ico),el('b','',big),el('small','',small));
  b.addEventListener('click',()=>jog(axis,sign));tip(b,big+' · '+small,`Moves ${axis} ${sign>0?'+':'−'} the step shown.`);pad.append(b);return b;};
 const dirs=[dir('n','i-arrow-up','Y',1,'Y+','back'),dir('s','i-arrow-down','Y',-1,'Y−','front'),dir('w','i-arrow-left','X',-1,'X−','left'),dir('e','i-arrow-right','X',1,'X+','right')];
 const hxy=el('button','nx-mv-hub');hxy.type='button';hxy.dataset.cmd='console';hxy.append(icon('i-home'),el('b','','HOME'),el('small','','X Y'));hxy.addEventListener('click',()=>script('G28 X Y','G28 X Y'));tip(hxy,'Home X and Y','G28 X Y.');pad.append(hxy);
 // Z rocker
 const zc=el('div','nx-mv-z');const zb=(ico,sign,big,small)=>{const b=el('button','nx-mv-zbtn');b.type='button';b.dataset.cmd='jog';b.append(icon(ico),el('b','',big),el('small','',small));b.addEventListener('click',()=>jog('Z',sign));tip(b,big,`Moves Z ${sign>0?'up':'down'} the step shown.`);return b;};
 const zUp=zb('i-z-up',1,'Z+','nozzle up'),zDn=zb('i-z-down',-1,'Z−','nozzle down'),zv=el('div','nx-mv-zval');const zval=el('b','','—');zv.append(el('small','','Z'),zval);
 const hz=button({icon:'i-home',label:'Home Z',size:'row',cmd:'console',title:'Home Z',does:'G28 Z: X and Y must be homed first.',onPress:()=>script('G28 Z','G28 Z')});zc.append(zUp,zv,zDn,hz);
 // where the head is on the bed, from above (front at the bottom, as you stand at the printer)
 const map=document.createElementNS('http://www.w3.org/2000/svg','svg');map.classList.add('nx-mv-map');let mapKey='';
 const mapBox=el('div','nx-mv-mapbox'),mapNote=el('small','','Waiting for printer travel limits');mapBox.append(map,mapNote);
 function mapFrame(g){const key=g?`${g.x0},${g.y0},${g.x1},${g.y1}`:'unknown';if(key===mapKey)return;mapKey=key;
  if(!g){map.replaceChildren();mapNote.textContent='Printer travel is unknown; no bed position is inferred.';return;}
  const {width:w,height:h}=g;map.setAttribute('viewBox',`-12 -12 ${w+24} ${h+36}`);
  map.innerHTML=`<rect x="0" y="0" width="${w}" height="${h}" rx="10" class="bed"/>`+
   Array.from({length:Math.ceil(w/50)-1},(_,i)=>(i+1)*50).map(v=>`<line x1="${v}" y1="0" x2="${v}" y2="${h}"/>`).join('')+
   Array.from({length:Math.ceil(h/50)-1},(_,i)=>(i+1)*50).map(v=>`<line x1="0" y1="${v}" x2="${w}" y2="${v}"/>`).join('')+
   `<line class="cx" x1="0" x2="${w}"/><line class="cy" y1="0" y2="${h}"/><circle class="halo" r="22"/><circle class="head" r="8"/><text x="${w/2}" y="${h+16}" text-anchor="middle">FRONT</text><text x="${w/2}" y="-2" text-anchor="middle">BACK</text>`;
  mapNote.textContent=`X ${g.x0}–${g.x1}, Y ${g.y0}–${g.y1} mm, seen from above`;}
 const stage=el('div','nx-mv-stage');stage.append(mapBox,pad,zc);
 // homing and parking
 const acts=el('div','nx-mv-acts');acts.append(
  button({icon:'i-home',label:'Home all',kind:'primary',size:'row',cmd:'home',title:'Home all axes',does:'G28.',onPress:async b=>{busy(b,true);await send('home');busy(b,false);}}),
  button({icon:'i-level',label:'Level gantry',size:'row',cmd:'qgl',title:'Quad gantry level',does:'QUAD_GANTRY_LEVEL.',onPress:async b=>{busy(b,true);await send('qgl');busy(b,false);}}),
  button({icon:'i-move',label:'Centre',size:'row',cmd:'console',title:'Centre the head',does:'Moves to the midpoint of the printer’s observed X/Y travel. Needs X and Y homed.',onPress:()=>{const g=travel();if(!g){toast('Printer travel limits are unavailable; Centre was not sent.','alarm');return;}if(!state.readings.homed.x||!state.readings.homed.y){toast('Home X and Y before centring.','alarm');return;}script(`G90\nG1 X${g.cx} Y${g.cy} F6000`,'centre');}}),
  button({icon:'i-arrow-up',label:'Park',size:'row',cmd:'console',title:'Park',does:'Runs your PARK macro.',onPress:()=>script('PARK','PARK')}),
  button({icon:'i-motors-off',label:'Motors off',size:'row',cmd:'motors_off',title:'Motors off',does:'M84: the steppers let go and the printer is no longer homed.',onPress:()=>send('motors_off')}));
 const warn=el('p','nx-mv-warn');
 body.append(pos,section('Step',stepBar),stage,acts,warn);
 function paintSteps(){for(const b of [...dirs,zUp,zDn])b.querySelector('small').dataset.step=step+' mm';}paintSteps();
 const keys=e=>{if(e.target!==pad||e.repeat||e.altKey||e.ctrlKey||e.metaKey||e.shiftKey||host.hidden)return;const m={ArrowUp:['Y',1],ArrowDown:['Y',-1],ArrowLeft:['X',-1],ArrowRight:['X',1],PageUp:['Z',1],PageDown:['Z',-1]}[e.key];if(!m)return;e.preventDefault();jog(...m);};
 pad.addEventListener('keydown',keys);
 const paint=()=>{if(!body.isConnected||host.hidden)return;const r=state.readings;
  for(const a of ['X','Y','Z']){const v=r.position[a.toLowerCase()],h=r.homed[a.toLowerCase()];cell[a].v.textContent=h&&finite(v)!==null?v.toFixed(a==='Z'?2:1):'—';cell[a].c.toggleAttribute('data-homed',!!h);cell[a].c.querySelector('.nx-mv-homed').textContent=h?'homed':'not homed';}
  zval.textContent=r.homed.z&&finite(r.position.z)!==null?r.position.z.toFixed(2):'—';
  {const g=travel();mapFrame(g);if(g){const x=finite(r.position.x),y=finite(r.position.y),ok=r.homed.x&&r.homed.y&&x!==null&&y!==null&&x>=g.x0&&x<=g.x1&&y>=g.y0&&y<=g.y1,px=ok?x-g.x0:g.width/2,py=ok?g.y1-y:g.height/2;
   for(const c of map.querySelectorAll('circle')){c.setAttribute('cx',px);c.setAttribute('cy',py);}map.querySelector('.cx').setAttribute('y1',py);map.querySelector('.cx').setAttribute('y2',py);
   map.querySelector('.cy').setAttribute('x1',px);map.querySelector('.cy').setAttribute('x2',px);map.dataset.known=ok?'1':'0';}}vv.textContent=r.velocity===null?'—':String(Math.round(r.velocity));
  const all=r.homed.x&&r.homed.y&&r.homed.z;warn.textContent=all?'':'Home first: jogging needs X, Y and Z homed. Home all, or home X and Y from the middle of the pad.';warn.hidden=all;
  setTimeout(paint,400);};paint();lockControls(body);}});}
function sheetFactor(which){const r=state.readings,speed=which==='speed',cur=Math.round(((speed?r.speedFactor:r.extrudeFactor)??1)*100);
 openSheet({title:speed?'Speed':'Flow',icon:speed?'i-speed':'i-flow',build:body=>{const st=stepper({value:cur,unit:'%',step:speed?10:1,min:speed?10:50,max:speed?300:150});
  const id=speed?'speed_factor':'extrude_factor',apply=button({icon:'i-check',label:'Apply',kind:'primary',size:'row',cmd:id,title:'Apply',does:`Sends ${speed?'M220':'M221'} S${'{value}'}.`,onPress:async b=>{busy(b,true);await send(id,{value:st.get()});busy(b,false);}});
  const pre=el('div','nx-grid3');for(const v of speed?[50,75,100,125,150,200]:[90,95,100,105,110,115])pre.append(button({label:v+'%',size:'row',cmd:id,title:v+'%',does:`Sets the ${which} factor to ${v}%.`,onPress:()=>{st.setValue(v);send(id,{value:v});}}));
  const fl=flowModel(r,state.presets?.loaded?.name,state.presets?.hotend_max_flow);
  body.append(el('p','nx-note',speed?`Live velocity ${r.velocity===null?'—':Math.round(r.velocity)} mm/s. The factor scales every move the G-code asks for.`:`Volumetric flow now ${fl.value===null?'—':fl.value.toFixed(1)} mm³/s. ${fl.max===null?'No measured hotend limit is saved.':`Measured limit ${fl.max} mm³/s.`} The factor scales every extrusion.`),st,apply,section('Presets',pre));lockControls(body);}});}
function sheetFan(){const r=state.readings;openSheet({title:'Fans',icon:'i-fan',build:body=>{const g=el('div','nx-grid3');
 for(const v of [0,25,50,75,100])g.append(button({label:v+'%',size:'row',cmd:'set_fan',title:`Part fan ${v}%`,does:`Sets the part cooling fan (both 4010 blowers) to ${v}%.`,onPress:()=>send('set_fan',{value:v/100})}));
 const auto=el('div','nx-grid2');auto.append(button({icon:'i-fan',label:'Hotend fan',size:'row',title:'Hotend fan',does:'Its panel, with the temperature it starts at.',onPress:()=>openPartByKey('hotend_fan')}),button({icon:'i-fan',label:'Bay fans',size:'row',title:'Electronics bay fans',does:'Their panel.',onPress:()=>openPartByKey('controller_fan')}));
 body.append(section('Part cooling fan',g,el('p','nx-note','Or drag round the Part ring on the main view.')),
  section('Automatic fans',el('p','nx-note',`Hotend ${fmtPct(r.fans.hotend.speed)} · bay ${fmtPct(r.fans.controller.speed)}. Klipper runs these itself (heater_fan and controller_fan): they cannot be set from a command, only their switch-on rules in printer.cfg. The hotend fan's start temperature can be changed from its panel.`),auto),
  el('p','nx-note','Speeds are what Klipper was told to run; no tachometer is fitted.'));lockControls(body);}});}
// Files: a wide grid of cards, each with the slicer's picture, the name, and what Orca put in the
// file name (layer, material, time), newest first.
function fileFacts(path){const base=path.split('/').pop().replace(/\.gcode$/i,''),parts=base.split('_'),f={};
 for(const p of parts){if(/^\d+(\.\d+)?mm$/.test(p))f.layer=p;else if(/^(PLA|PETG|ABS|ASA|TPU|PC|PA|NYLON|HIPS)(\b|-)/i.test(p))f.mat=p;else if(/^(\d+d)?(\d+h)?(\d+m)?(\d+s)?$/.test(p)&&/\d/.test(p))f.time=p.replace(/(\d+)([dhms])/g,'$1$2 ').trim();}
 return f;}
function sheetFiles(){openSheet({title:'Files',icon:'i-files',wide:true,build:async body=>{const top=el('div','nx-files-top'),search=el('input','nx-input');search.type='search';search.placeholder='Find a file';search.setAttribute('aria-label','Find a file');
 const count=el('span','nx-note');top.append(search,count);const grid=el('div','nx-filegrid');body.append(top,grid);
 if(['off','replay','service'].includes(link().state)){grid.append(el('p','nx-note',link().reason));return;}
 try{const r=await fetch('/api/files');const d=await r.json();if(!r.ok)throw Error(d.error||'File list unavailable.');const files=(d.files||d.result||[]).slice().sort((a,b)=>(b.modified||0)-(a.modified||0));
  const draw=()=>{grid.replaceChildren();const q=search.value.toLowerCase();const shown=files.filter(f=>!q||(f.path||f.filename||'').toLowerCase().includes(q)).slice(0,120);count.textContent=`${shown.length} of ${files.length} files`;
   shown.forEach((f,i)=>{const path=f.path||f.filename,facts=fileFacts(path),card=el('div','nx-filecard');if(i===0&&!q)card.dataset.newest='';
    const pic=el('div','nx-filecard-pic'),img=el('img');img.alt='';img.loading='lazy';img.src='/api/print/thumbnail?filename='+encodeURIComponent(path);img.addEventListener('error',()=>{img.replaceWith(icon('i-files'));});pic.append(img);
    if(i===0&&!q)pic.append(el('i','nx-filecard-new','NEWEST'));
    const meta=el('div','nx-filecard-meta');for(const [k,v] of [['time',facts.time],['mat',facts.mat],['layer',facts.layer]])if(v){const c=el('span','',v);c.dataset.k=k;meta.append(c);}
    const name=el('b','',fileTitle(path)),when=el('small','',[f.modified?new Date(f.modified*1000).toLocaleString([], {month:'short',day:'numeric',hour:'numeric',minute:'2-digit'}):'',f.size?(f.size>1048576?(f.size/1048576).toFixed(1)+' MB':Math.round(f.size/1024)+' KB'):''].filter(Boolean).join(' · '));
    const acts=el('div','nx-filecard-acts');acts.append(button({icon:'i-print',label:'Print',size:'row',kind:'primary',cmd:'print_start',title:'Print '+fileTitle(path),does:'Starts this file now. Asks once.',onPress:async b=>{busy(b,true);if(await send('print_start',{filename:path}))closeSheet();busy(b,false);}}),
     button({icon:'i-plus',size:'icon',cmd:'print_queue_add',title:'Queue',does:'Adds this file to the print queue.',onPress:()=>send('print_queue_add',{filename:path})}),
     button({icon:'i-view',size:'icon',title:'Details',does:'The slicer that made it, print time, filament and settings.',onPress:()=>openFile(path)}));
    card.append(pic,name,when,meta,acts);grid.append(card);});
   if(!shown.length)grid.append(el('p','nx-note',files.length?'No file matches.':'No files on the printer.'));lockControls(body);};
  search.addEventListener('input',draw);draw();}
 catch(e){grid.append(el('p','nx-note',e.message));}}});}
function sheetMacros(){openSheet({title:'Macros',icon:'i-macro',build:async body=>{const list=el('div','nx-list');const search=el('input','nx-input');search.type='search';search.placeholder='Find a macro';search.setAttribute('aria-label','Find a macro');body.append(search,list);
 if(link().lock){list.append(el('p','nx-note',link().reason));return;}
 try{const r=await fetch('/api/macros');const d=await r.json();if(!r.ok)throw Error(d.error||'Macros unavailable.');const mine=d.macros.filter(m=>m.macro===true&&!m.name.startsWith('_'));
  const draw=()=>{list.replaceChildren();const q=search.value.toLowerCase();for(const m of mine.filter(m=>!q||m.name.toLowerCase().includes(q))){const params=Array.isArray(m.params)?m.params:[];
   list.append(button({icon:'i-macro',label:m.name,size:'row',cmd:'macro',title:m.name,does:(m.help?m.help+' ':'')+(params.length?'Opens its parameters, then asks once.':'Asks once.'),
    onPress:()=>{if(!params.length)return send('macro',{target:m.name});openSheet({title:m.name,icon:'i-macro',build:mb=>{const fields={};for(const p of params){const i=el('input','nx-input');i.placeholder=p.default!=null?String(p.default):'';fields[p.name]=i;mb.append(section(p.name,i));}
     mb.append(button({icon:'i-play',label:'Run',kind:'primary',size:'row',cmd:'macro',title:'Run '+m.name,does:'Shows the command, then sends it once.',onPress:()=>{const values={};for(const[k,i]of Object.entries(fields))if(i.value.trim())values[k]=i.value.trim();send('macro',{target:m.name,params:values});}}));lockControls(mb);}});}}));}
   if(!list.children.length)list.append(el('p','nx-note','No macro matches.'));lockControls(list);};search.oninput=draw;draw();}
 catch(e){list.append(el('p','nx-note',e.message));}}});}
function sheetConsole(){let stream=null;openSheet({title:'Console',icon:'i-console',onClose:()=>stream?.close(),build:body=>{const log=el('div','nx-console');const add=(t,c='')=>{const d=el('div',c,t);log.append(d);log.scrollTop=log.scrollHeight;};
 for(const e of (state.snapshot?.events||[]).slice(-40))add(`${new Date(e.ts*1000).toLocaleTimeString()}  ${e.text||e.kind}`);
 try{stream=new EventSource('/api/tick/stream');stream.addEventListener('console',ev=>{try{const d=JSON.parse(ev.data);for(const l of (Array.isArray(d)?d:d.lines||[d]))add(`${l.ts?new Date(l.ts*1000).toLocaleTimeString()+'  ':''}${l.text||l.message||''}`);}catch{}});}catch{}
 const input=el('input','nx-input');input.placeholder='G-code, e.g. QUERY_ENDSTOPS';input.setAttribute('aria-label','G-code');input.dataset.cmd='console';
 const go=async()=>{const line=input.value.trim();if(!line)return;const r=await send('console',{script:line});if(r){add('> '+line,'cmd');input.value='';}};
 input.addEventListener('keydown',e=>{if(e.key==='Enter')go();});
 body.append(log,input,button({icon:'i-play',label:'Send',kind:'primary',size:'row',cmd:'console',title:'Send',does:'Shows the line for review, then sends it once.',onPress:go}));lockControls(body);}});}
function sheetExclude(){openSheet({title:'Exclude objects',icon:'i-exclude',build:async body=>{const list=el('div','nx-list');body.append(el('p','nx-note','Skip one object of this print. The rest keeps printing.'),list);
 try{const r=await fetch('/api/print/objects');const d=await r.json();if(!r.ok)throw Error(d.error||'Objects unavailable.');const objs=d.objects||[];
  for(const o of objs)list.append(button({icon:o.excluded?'i-check':'i-exclude',label:o.name+(o.excluded?' · excluded':''),size:'row',kind:o.excluded?'default':'danger',cmd:'exclude_object',hold:!o.excluded,title:'Exclude '+o.name,does:'Stops printing this object.',onPress:()=>send('exclude_object',{name:o.name})}));
  if(!objs.length)list.append(el('p','nx-note','This print defines no objects.'));lockControls(body);}catch(e){list.append(el('p','nx-note',e.message));}}});}
function sheetRecording(){openSheet({title:'Recording',icon:'i-rewind',build:body=>{const s=state.snapshot;const first=s?.first_ts,last=s?.last_ts;
 const range=el('input','nx-input');range.type='range';range.min=first||0;range.max=last||1;range.step=1;range.value=state.cursor||last||0;range.setAttribute('aria-label','Recording position');
 const when=el('p','nx-note',state.live?'Live':new Date((state.cursor||0)*1000).toLocaleString());range.addEventListener('change',()=>{state.live=false;request(Number(range.value));when.textContent=new Date(Number(range.value)*1000).toLocaleString();});
 const ctl=el('div','nx-grid3');ctl.append(button({icon:'i-step-back',label:'Back',size:'row',title:'Back',does:'One sample back.',onPress:()=>recAction('back')}),button({icon:state.playing?'i-pause':'i-play',label:state.playing?'Pause':'Play',size:'row',title:'Play',does:'Plays from here.',onPress:()=>recAction('play')}),button({icon:'i-step-fwd',label:'Forward',size:'row',title:'Forward',does:'One sample forward.',onPress:()=>recAction('fwd')}));
 body.append(section('Position',range,when),ctl,section('Speed',segmented([1,2,5,10],state.speed,v=>{state.speed=v;},v=>v+'×')),
  button({icon:'i-live',label:'Go live',kind:'primary',size:'row',title:'Go live',does:'Back to the live printer.',onPress:()=>{recAction('live');closeSheet();}}),
  button({icon:'i-link',label:'Copy link to this moment',size:'row',title:'Copy link',does:'Copies a link that opens this exact moment.',onPress:async()=>{try{await navigator.clipboard.writeText(location.href);toast('Link copied.','ok');}catch{toast(location.href);}}}));}});}
function sheetView(){openSheet({title:'View',icon:'i-view',build:body=>{const g=el('div','nx-grid2');
 body.append(section('Instrument style',segmented(['knob','bar'],instStyle,v=>setInstStyle(v),v=>v==='knob'?'Knobs':'Bars'),el('p','nx-note','Knobs: drag round the ring. Bars: slide left and right. Both set the same things; the choice is kept on this device.')));
 for(const [v,lab]of[['iso','Iso'],['front','Front'],['top','Top'],['right','Right']])g.append(button({icon:'i-view',label:lab,size:'row',title:lab+' view',does:'Turns the camera to the '+lab.toLowerCase()+' view.',onPress:()=>scene?.view?.(v)}));
 body.append(section('Camera',g,button({icon:'i-back',label:'Reset view',size:'row',title:'Reset view',does:'Frames the whole printer again.',onPress:()=>scene?.reset?.()})),
  section('Screen',button({icon:'i-fullscreen',label:'Full screen',size:'row',title:'Full screen',does:'Fills the screen with Stella Scope.',onPress:()=>{if(document.fullscreenElement)document.exitFullscreen();else document.documentElement.requestFullscreen?.();}}),
   button({icon:'i-panels',label:'Panels see-through',size:'row',title:'Panels',does:'Makes the enclosure panels see-through.',onPress:()=>scene?.panels?.(true)})),
  section('On the printer',button({icon:'i-mesh',label:meshOn?'Hide the mesh on the bed':'Show the mesh on the bed',size:'row',title:'Mesh on the bed',does:'Drapes the newest bed mesh over the plate in the 3D view, heights exaggerated 25 times.',onPress:b=>{setMeshOn(!meshOn);b.querySelector('.nx-lbl').textContent=meshOn?'Hide the mesh on the bed':'Show the mesh on the bed';}})),
  eyeSwitch(),
  section('Phone',button({icon:'i-link',label:'Pair a phone, paired phones, WhatsApp',size:'row',title:'Phone',does:'One scan sets a phone up. Also the paired phones and WhatsApp alerts.',onPress:()=>sheetPhone()})),
  section('3D printer on this device',segmented(['on','off'],show3d()?'on':'off',v=>{try{localStorage.setItem('nx-3d',v);}catch{}toast(v==='on'?'Reload to show the 3D printer.':'Reload to hide it.');},v=>v==='on'?'Show':'Hide'),el('p','nx-note','Phones load the 3D model only when asked (it is over 100 MB); everything else works without it.')),
  section('Printer screen',segmented(['off','on'],kiosk?'on':'off',v=>setKiosk(v==='on'),v=>v==='on'?'Kiosk mode':'Normal'),el('p','nx-note','Kiosk mode is for a touchscreen on the printer: no side rail, bigger buttons, and the screen dims after three minutes without a touch. The first touch only wakes it.')),
  section('Classic HUD',button({icon:'i-nav-hud',label:'Open the classic HUD',size:'row',title:'Classic HUD',does:'The previous interface, unchanged.',onPress:()=>{location.href='/classic-hud';}})));}});}
// The spaghetti watch, on or off from here. The first time on fetches Obico's model (200 MB) in the
// background and shows how far it has got.
function eyeSwitch(){const box=el('div','nx-sheetpick'),line=el('p','nx-note'),bar=el('div','nx-print-bar'),fill=el('i');bar.append(fill);bar.hidden=true;
 let seg=null;
 const paint=d=>{const dl=d.download||{};bar.hidden=!dl.active;if(dl.active)fill.style.width=Math.round((dl.bytes||0)/(dl.total||1)*100)+'%';
  if(d.installed===false&&!d.enabled){line.textContent='The failure watch is an add-on you install yourself. The Stella Scope manual (Failure watch) shows how.';return;}
  line.textContent=dl.active?`Fetching the detector: ${Math.round((dl.bytes||0)/1048576)} of ${Math.round((dl.total||1)/1048576)} MB…`:dl.error?`Could not fetch the detector: ${dl.error}`:d.enabled?'On. It looks at the camera every 10 s while printing and warns here and on WhatsApp. It never stops a print by itself.':'Off. Turning it on fetches the detector (200 MB) the first time.';
  if(dl.active)setTimeout(load,1000);};
 const load=()=>fetch('/api/eye').then(r=>r.json()).then(d=>{if(!seg){seg=segmented(['on','off'],d.enabled?'on':'off',v=>{if(!state.desktopAccess){toast('Switched from the computer.','alarm');return;}fetch('/api/eye/switch',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({on:v==='on'})}).then(r=>r.json()).then(j=>{if(j.error)toast(j.error,'alarm');paint(j);if(j.download?.active)setTimeout(load,1000);});},v=>v==='on'?'On':'Off');box.prepend(seg);}paint(d);}).catch(()=>{line.textContent='Not available.';});
 box.append(line,bar);load();return section('Spaghetti watch',box);}
// ---------- phone access (src/lan.rs) ----------
async function phoneBadge(){const b=$('nx-phonebtn');if(!b)return;let n=0;try{const d=await (await fetch('/api/pair/devices')).json();if(!d.enabled){b.hidden=true;return;}n=(d.devices||[]).length;}catch{return;}
 b.hidden=false;b.dataset.paired=n?'1':'';b.querySelector('use')?.setAttribute('href',n?'#i-phone-check':'#i-phone');
 const t=n?(n>1?n+' phones paired':'Phone paired'):'Pair phone';const lab=b.querySelector('.nx-lbl');if(lab)lab.textContent=t;b.setAttribute('aria-label',t);}
// -- its own page on the rail, not only inside View. Phone. Three tabs. Pair: get the app from the
// HTTPS download page, then one scan in the app both finds this computer, at home or away, and pairs
// the phone; the six digits are there for the app's pairing page. Paired phones: the list, each
// removable. WhatsApp: the alerts.
function sheetPhone(){openSheet({title:'Phone',icon:'i-link',wide:true,build:body=>{
  if(!state.desktopAccess){body.append(el('p','nx-note','This phone is paired. Pairing and alerts are set up from the Stella Scope window on the computer.'));return;}
 const tabs={pair:el('div','nx-tabpane'),phones:el('div','nx-tabpane'),wa:el('div','nx-tabpane')};tabs.phones.hidden=tabs.wa.hidden=true;
 const bar=segmented(['pair','phones','wa'],'pair',k=>{for(const [n,p] of Object.entries(tabs))p.hidden=n!==k;if(k==='phones')drawPhones();},k=>({pair:'Pair a phone',phones:'Paired phones',wa:'WhatsApp alerts'}[k]));bar.classList.add('nx-tabs');
 body.classList.add('nx-sheet-body--tabs');body.append(bar,tabs.pair,tabs.phones,tabs.wa);tabs.wa.append(whatsApp());
 // Pair tab
 const grid=el('div','nx-pairgrid'),s1=el('div','nx-pairstep'),s2=el('div','nx-pairstep');grid.append(s1,s2);const status=el('p','nx-pairstatus');tabs.pair.append(status,grid);
 const appQr=el('div','nx-qr'),pairQr=el('div','nx-qr nx-qr--pair'),ring=el('div','nx-pairring'),digits=el('div','nx-paircode');
 s1.append(el('b','nx-pairnum','1'),el('h4','','Get the Stella app'),appQr,el('p','nx-note','Point the phone camera at this code to open the secure download page. Install the app before pairing.'));
 const go=button({icon:'i-link',label:'Pair a phone',kind:'primary',size:'row',title:'Pair a phone',does:'Shows a code for five minutes. One scan in the Stella app sets the phone up.',onPress:()=>newPair()});
 s2.append(el('b','nx-pairnum','2'),el('h4','','Scan it in the app'),pairQr,ring,digits,go,el('p','nx-note','Open the Stella app, press Scan the code, and point it here. The app connects through the encrypted link and pairs this phone.'));
 let expireAt=0,timer=0;async function newPair(){try{const r=await fetch('/api/pair/new',{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'});const j=await r.json();if(!r.ok)throw Error(j.error);
   pairQr.innerHTML=j.qr_svg||'';pairQr.hidden=!j.qr_svg;digits.textContent=j.code.replace(/(\d{3})(\d{3})/,'$1 $2');expireAt=Date.now()+j.expires_in_s*1000;s2.dataset.live='';clearInterval(timer);timer=setInterval(tickPair,500);tickPair();}catch(e){toast(e.message,'alarm');}}
 function tickPair(){const left=Math.max(0,expireAt-Date.now());ring.style.setProperty('--p',(left/300000).toFixed(3));ring.textContent=left?`${Math.floor(left/60000)}:${String(Math.floor(left/1000)%60).padStart(2,'0')} left`:'';
  if(!left||!body.isConnected){clearInterval(timer);delete s2.dataset.live;pairQr.innerHTML='';digits.textContent='';}}
 (async()=>{let d;try{d=await (await fetch('/api/pair/remote')).json();}catch{d={};}
  appQr.innerHTML=d.app_qr_svg||'';if(!d.app_qr_svg)appQr.replaceWith(el('p','nx-note','The app download page is unavailable.'));
  status.textContent=!d.enabled?'Anywhere access is off. Enable remote_link to pair a phone.':!d.running?'Anywhere access is unavailable: stella-link is not running.':d.online?'Ready: a paired phone reaches this computer at home and away.':'Starting anywhere access…';status.dataset.tone=d.online?'ok':'';})();
 // Paired phones tab
 async function drawPhones(){tabs.phones.replaceChildren();let d;try{d=await (await fetch('/api/pair/devices')).json();}catch{tabs.phones.append(el('p','nx-note','Not available.'));return;}
  if(!(d.devices||[]).length){tabs.phones.append(el('p','nx-note','No phone is paired yet.'));return;}
  const list=el('div','nx-phonelist');for(const dv of d.devices){const row=el('div','nx-phonecard');row.append(icon('i-link'),el('b','',dv.name),el('small','','paired '+new Date(dv.paired_at*1000).toLocaleString()),button({icon:'i-close',label:'Remove',size:'row',kind:'danger',title:'Remove',does:'This phone must pair again to reach Stella Scope.',onPress:async()=>{await fetch('/api/pair/revoke',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:dv.id})});drawPhones();}}));list.append(row);}
  tabs.phones.append(list);}
}});}
// Without the 3D model (phones, by default) the stage shows the camera, or offers the model.
function stageCard(){const c=el('div','nx-stagecard nx-glass');$('nx-ui').append(c);
 const offer=()=>{c.replaceChildren(el('p','nx-note','The 3D printer is not loaded on this device, to save data and battery.'),button({icon:'i-view',label:'Show the 3D printer',size:'row',title:'Show the 3D printer',does:'Loads the 3D model on this device from now on.',onPress:()=>{try{localStorage.setItem('nx-3d','on');}catch{}location.reload();}}));};
 if(['off','service','replay'].includes(link().state)){offer();return;}
 cameras().then(cams=>{if(!cams.length){offer();return;}const img=camImg(cams[0]);img.addEventListener('error',offer);img.addEventListener('click',()=>pageCamera());c.replaceChildren(img);}).catch(offer);}
// ---------- WhatsApp alerts (src/notify.rs, CallMeBot) ----------
function whatsApp(){const box=el('div','nx-sheetpick');const load=async()=>{box.replaceChildren();if(!state.desktopAccess){box.append(el('p','nx-note','Set up from the Stella Scope window on the computer.'));return;}
 let d;try{d=await (await fetch('/api/notify')).json();}catch{box.append(el('p','nx-note','Not available here.'));return;}
 const phone=el('input','nx-input'),key=el('input','nx-input'),msg=el('p','nx-note');phone.placeholder='+1 506 …';phone.value=d.phone||'';phone.setAttribute('aria-label','WhatsApp number with country code');key.placeholder=d.has_key?'key saved (type to replace)':'CallMeBot key';key.setAttribute('aria-label','CallMeBot key');key.autocomplete='off';
 const post=(u,b)=>fetch(u,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(b)}).then(async r=>{const j=await r.json();if(!r.ok)throw Error(j.error);return j;});
 box.append(el('p','nx-note',`Messages when a print finishes, fails or is cancelled, on a BLTouch alarm, a driver warning, Klipper stopping, or the printer dropping off mid-print. One-time setup: add ${d.bot_number} to your phone's contacts and send it "${d.activation}" on WhatsApp. It replies with your key; paste it below.`),phone,key,
  button({icon:'i-check',label:d.enabled?'Save':'Save and turn on',kind:'primary',size:'row',title:'Save',does:'Stores the number and key on this computer only.',onPress:async()=>{try{const j=await post('/api/notify',{phone:phone.value,apikey:key.value,enabled:true});toast(j.enabled?'WhatsApp alerts are on.':'Saved. Paste the key to turn them on.','ok');load();}catch(e){msg.textContent=e.message;}}}),
  button({icon:'i-play',label:'Send a test message',size:'row',title:'Send a test',does:'Sends one WhatsApp message to the number above.',onPress:async()=>{try{const j=await post('/api/notify/test',{});msg.textContent=j.queued?'Sent to CallMeBot; it should arrive within a minute.':'Held back: turn alerts on first, or wait 20 s between messages.';setTimeout(load,6000);}catch(e){msg.textContent=e.message;}}}),
  d.enabled?button({icon:'i-close',label:'Turn off',size:'row',title:'Turn off',does:'Stops WhatsApp alerts; the key stays saved.',onPress:async()=>{await post('/api/notify',{phone:phone.value,enabled:false});load();}}):el('span'),msg);
 if(d.last)msg.textContent=`Last message ${new Date(d.last.at*1000).toLocaleTimeString()}: ${d.last.ok?'accepted':'not accepted'}${d.last.reply?' · '+d.last.reply:''}`;};load();return section('WhatsApp alerts',box);}
function show3d(){try{const v=localStorage.getItem('nx-3d');if(v)return v==='on';}catch{}return !(innerWidth<600||matchMedia('(pointer: coarse)').matches&&innerWidth<1000);}
function sheetPages(){openSheet({title:'Pages',icon:'i-nav-hud',build:body=>{const l=el('div','nx-list');for(const [ico,name,href,does]of ui.pages){const b=el('a','nx-item');b.href=href.startsWith('sheet:')?'#':href;if(href.startsWith('sheet:'))b.addEventListener('click',e=>{e.preventDefault();openNamedSheet(href.slice(6));});b.style.textDecoration='none';b.append(icon(ico),Object.assign(el('div','nx-item-text'),{}));b.lastChild.append(el('div','nx-item-title',name),el('div','nx-item-sub',does));l.append(b);}body.append(l);}});}

// ---------- toolpath on the plate (ported from hud.js showToolpath / toolpathAuto / toolpathTick) ----------
// the user: the old HUD drew the print on the plate while it ran; NEXT had lost it. The file is read once per
// job from the printer, never in a replay or a rewound view, and a failure backs off 5, 15, 60 s.
const TP_BACKOFF_S=[5,15,60];let gcodeMod=null,tp=null,tpAuto=null,tpFail=0,tpNext=0,tpNoted=null;
async function showToolpath(path){gcodeMod??=await import('./hud-gcode.js');const r=await fetch('/api/print/gcode?filename='+encodeURIComponent(path));if(!r.ok)throw Error('G-code unavailable from the printer.');
 const text=await r.text(),parsed=gcodeMod.parseGcode(text,{maxSegments:1500000});tp={path,parsed,bytes:parsed.byteLength||text.length,layer:null,fraction:0,filePos:null};
 const info=scene?.loadToolpath(parsed,path);if(scene&&!info)throw Error(scene.bedCenter?'That file has no extruding moves to draw.':'The printer model is still loading; the toolpath follows.');scene?.associateJob?.(path);}
function toolpathAuto(){const j=state.readings.job,name=j.name||'';if(!name||!['printing','paused'].includes(j.state))return;const file=name.split('/').pop();
 if(tpAuto===file||tp?.path.split('/').pop()===file)return;
 if(link().state==='replay'||!state.live){if(tpNoted!==file){tpNoted=file;tp={path:name,parsed:null,error:'G-code is not stored in the recording, so this print\'s toolpath cannot be drawn here.'};}return;}
 if(['service','off'].includes(link().state)||performance.now()<tpNext||(scene&&!scene.bedCenter))return;tpAuto=file;
 showToolpath(name).then(()=>{tpFail=0;tpNext=0;toast(`${tp.parsed.totalLayers||tp.parsed.layers.length} layers of ${fileTitle(name)} drawn on the plate.`);})
  .catch(e=>{tpAuto=null;tp={path:name,parsed:null,error:e.message};const wait=TP_BACKOFF_S[Math.min(tpFail,TP_BACKOFF_S.length-1)];tpNext=performance.now()+wait*1000;if(++tpFail===1)toast(`${e.message} Trying again in ${wait} s, then less often.`);});}
function toolpathTick(){if(!tp?.parsed||!gcodeMod)return;const j=state.readings.job;if(j.name&&j.name.split('/').pop()!==tp.path.split('/').pop())return;
 const p=lastTick?.point??state.snapshot?.point,pr=finite(j.progress)??0,at=finite(p?.file_position)!==null?Math.round(p.file_position):Math.round(pr*tp.bytes);
 const step=gcodeMod.progressToLayer(tp.parsed,at);tp.layer=step.layer;tp.fraction=step.fraction;tp.filePos=at;scene?.toolpathProgress?.({...step,filePosition:at,live:p?.position});}
// ---------- probe landings and the mesh on the 3D bed ----------
let probeKey='';function paintProbeMarks(){if(!scene?.probeMarks)return;const pts=liveProbes(0),k=pts.length+':'+(pts.at(-1)?.ts??'');if(k===probeKey)return;probeKey=k;scene.probeMarks(pts,{rgb:meshRgb});}
let meshOn=(()=>{try{return localStorage.getItem('nx-mesh3d')==='1';}catch{return false;}})(),meshShown=null,meshCheck=0;
function setMeshOn(v){meshOn=v;try{localStorage.setItem('nx-mesh3d',v?'1':'0');}catch{}meshShown=null;meshCheck=0;if(!v)scene?.meshOverlay?.(null);meshOverlayTick();}
async function meshOverlayTick(){if(!meshOn||!scene?.meshOverlay||!scene.bedCenter||performance.now()-meshCheck<(meshShown?30000:3000))return;meshCheck=performance.now();
 try{const h=await (await fetch('/api/history/mesh?limit=1')).json(),m=h.meshes?.[0];if(!m||m.ts===meshShown)return;const w=await (await fetch('/api/workspace?ts='+m.ts)).json(),g=w?.point?.mesh;
  if(scene.meshOverlay({grid:g?.grid,min:g?.min,max:g?.max},{rgb:meshRgb}))meshShown=m.ts;}catch{}}
// ---------- live activity card (gantry level, mesh) ----------
let actCard=null;function paintActivity(){if(!actCard){actCard=activityCard();$('nx-ui').append(actCard.el);}actCard.paint(state.live?lastTick?.activity:null);}
// ---------- kiosk mode: a touchscreen on the printer ----------
let kiosk=(()=>{try{return hash.has('kiosk')||localStorage.getItem('nx-kiosk')==='1';}catch{return hash.has('kiosk');}})(),dimTimer=0;
function setKiosk(v){kiosk=v;try{localStorage.setItem('nx-kiosk',v?'1':'0');}catch{}document.body.toggleAttribute('data-kiosk',v);state.dockKey='';render();reframe();wake();}
function wake(){clearTimeout(dimTimer);$('nx-dim')?.setAttribute('hidden','');if(kiosk)dimTimer=setTimeout(()=>{const d=$('nx-dim');if(d){d.removeAttribute('hidden');paintDim();}},180000);}
function paintDim(){const d=$('nx-dim');if(!d||d.hidden)return;const r=state.readings,j=jobModel(r);d.firstChild.textContent=j.active?`${j.title} · ${j.progress===null?'—':Math.round(j.progress*100)+'%'}`:'Stella is '+(link().label||'').toLowerCase();
 d.lastChild.textContent=`Nozzle ${fmtTemp(r.nozzle.temp)} · Bed ${fmtTemp(r.bed.temp)} · touch to wake`;}
function initKiosk(){const d=el('div','nx-dim');d.id='nx-dim';d.hidden=true;d.append(el('b'),el('span'));document.body.append(d);
 // The first touch on a dimmed screen only wakes it: it never reaches a button underneath.
 d.addEventListener('pointerdown',e=>{e.preventDefault();e.stopPropagation();wake();},true);
 for(const ev of ['pointerdown','keydown','wheel'])document.addEventListener(ev,()=>{if(kiosk&&$('nx-dim').hidden)wake();},true);
 setInterval(paintDim,2000);document.body.toggleAttribute('data-kiosk',kiosk);wake();}

// ---------- BLTouch light ---------- Klipper does not report the LED, so it is read from what the
// probe did and said: steady with the pin up, a flash each time it touches while probing, fast
// flashing after a reported failure until a good touch, a reset or a restart clears it, and dark
// when the printer has no power.
const BL_FAIL=/BLTouch failed|failed to (verify sensor state|deploy|raise probe)|Probe triggered prior to movement|No trigger on probe after full movement/i;
const BL_CLEAR=/BLTOUCH_DEBUG COMMAND=reset|FIRMWARE_RESTART|^RESTART\b/i;
let ledState='',ledLastPt=0;
function paintProbeLed(){const l=link(),ev=state.snapshot?.events||[],text=e=>e.message||e.text||'';
 const fail=ev.filter(e=>e.kind!=='command'&&BL_FAIL.test(text(e))).reduce((a,e)=>!a||e.ts>a.ts||(e.ts===a.ts&&/^!!/.test(text(e)))?e:a,null),clear=ev.filter(e=>e.kind==='command'&&BL_CLEAR.test(text(e))).reduce((a,e)=>Math.max(a,e.ts),0);
 const lastPt=[...probeStore.values()].reduce((a,q)=>Math.max(a,q.ts),0),now=lastTick?.point?.ts??Date.now()/1000;
 const st=['off','service','checking'].includes(l.state)?'off':fail&&fail.ts>Math.max(clear,lastPt)?'alarm':lastTick?.activity?.id==='probing'||now-lastPt<4?'probing':'on';
 state.bltouch=st;state.bltouchWhy=st==='alarm'?`Klipper said: "${text(fail).replace(/^!!\s*/,'')}".`:'';
 const flash=lastPt>ledLastPt&&ledLastPt>0;ledLastPt=lastPt;if(scene?.probeLed&&(st!==ledState||flash)){if(scene.probeLed(st,{flash:flash&&st==='probing'}))ledState=st;}}

// ---------- anything wrong pops up ---------- Each check names a problem with a key. A new key
// raises a card at the top of the screen that stays until it is dismissed or the problem clears; a
// dismissed key stays quiet until it clears and comes back. Console errors are keyed by their time,
// so each new one is raised once.
const shown=new Map(),quiet=new Set(),seenAt=Date.now()/1000-120,heatWatch={nozzle:[],bed:[]},rtWatch=new Map();
function alertHost(){let h=$('nx-alerts');if(!h){h=el('div','nx-alerts');h.id='nx-alerts';h.setAttribute('role','alert');$('nx-ui').append(h);}return h;}
function checkAlerts(){const l=link(),r=state.readings,p=state.snapshot?.point,ev=state.snapshot?.events||[],found=[];
 if(!state.live){for(const k of [...shown.keys()])dropAlert(k);return;}
 const add=(key,title,detail,act=null)=>found.push({key,title,detail,act});
 for(const k of ['nozzle','bed']){const h=heatColor(r[k].temp,r[k].target,k);if(h.tone==='danger'&&!document.body.hasAttribute('data-stale'))add('hot:'+k,(k==='nozzle'?'Nozzle':'Bed')+' over temperature',`${fmtTemp(r[k].temp)} against a target of ${fmtTemp(r[k].target)}. Check the heater; STOP cuts it at once.`,{label:'Show me',go:()=>openPartByKey(k==='nozzle'?'hotend':'bed')});}
 for(const t of dueTasks(state.printHours??null))add('maint:'+t.name,'Maintenance due: '+t.name,`Every ${t.every_h} print hours; it has been ${Math.round(state.printHours-(t.last_h??0))}.`,{label:'Maintenance',go:()=>openNamedSheet('maintenance')});
 if(l.state==='not_ready')add('klipper','Klipper has stopped',l.reason||'Klipper is not ready.',{label:'Firmware restart',go:()=>send('firmware_restart')});
 if(l.state==='off'&&['printing','paused'].includes(state.lastJobState))add('offline-print','The printer stopped answering during a print',l.reason||'Moonraker does not answer.',null);
 if(state.bltouch==='alarm')add('bltouch:'+(state.bltouchWhy||''),'BLTouch in alarm',state.bltouchWhy+' Its light is flashing.',{label:'Show the probe',go:()=>openPartByKey('probe')});
 for(const [name,m]of Object.entries(p?.motors||{}))if(m?.flags?.length)add(`drv:${name}:${m.flags.join(',')}`,`Stepper driver warning: ${name}`,`The TMC2209 reports ${m.flags.join(', ')}.`,{label:'Show the motor',go:()=>openPartByKey(name==='extruder'?'extruder':name)});
 for(const e of ev)if(e.ts>seenAt&&e.kind==='response'&&/^!!/.test(e.message||e.text||''))add('err:'+e.ts,'Klipper reported an error',(e.message||e.text).replace(/^!!\s*/,''),{label:'Timeline',go:()=>openNamedSheet('timeline')});
 // A heater driven hard that is not warming: 60 s at over 80 % output with under 2 degrees gained.
 const now=Date.now()/1000;for(const k of ['nozzle','bed']){const w=heatWatch[k],x=r[k];if(x.temp!==null&&x.target>0&&x.power!==null){w.push([now,x.temp,x.power]);while(w.length&&w[0][0]<now-65)w.shift();}else w.length=0;
  if(w.length>10&&now-w[0][0]>=55&&w.every(q=>q[2]>.8)&&w.at(-1)[1]-w[0][1]<2&&x.temp<x.target-10)add('stall:'+k,`The ${k} is not heating`,`Full power for a minute and only ${(w.at(-1)[1]-w[0][1]).toFixed(1)}° gained. Check the heater and thermistor wiring.`,{label:'Show me',go:()=>openPartByKey(k==='nozzle'?'hotend':'bed')});}
 if(r.nozzle.temp!==null&&r.nozzle.temp>70&&r.fans.hotend.speed===0)add('hotend-fan','Hotend fan is off with a hot nozzle',`The nozzle is at ${fmtTemp(r.nozzle.temp)} and the hotend fan command is 0 %. Heat can creep up the heatbreak.`,{label:'Show the fan',go:()=>openPartByKey('hotend_fan')});
 for(const s of p?.telemetry?.signals||[]){if(s.field!=='last_stats.bytes_retransmit'||typeof s.value!=='number')continue;const w=rtWatch.get(s.object)||[];w.push([now,s.value]);while(w.length&&w[0][0]<now-65)w.shift();rtWatch.set(s.object,w);
  if(w.length>2&&w.at(-1)[1]-w[0][1]>300)add('rt:'+s.object,s.object==='mcu'?'The controller link is dropping data':'The toolhead board link is dropping data',`${w.at(-1)[1]-w[0][1]} bytes resent in the last minute. Check the ${s.object==='mcu'?'USB':'CAN'} cable and its plugs.`,{label:'Show the board',go:()=>openPartByKey(s.object==='mcu'?'controller':'toolboard')});}
 // Only a print seen live in this session counts: a page opened after power-off stays quiet.
 if(l.state==='ready')state.lastJobState=r.job.state;
 const keys=new Set(found.map(a=>a.key));for(const k of [...shown.keys()])if(!keys.has(k)&&!k.startsWith('err:'))dropAlert(k);for(const k of [...quiet])if(!keys.has(k)&&!k.startsWith('err:'))quiet.delete(k);
 for(const a of found)if(!shown.has(a.key)&&!quiet.has(a.key))raiseAlert(a);}
function raiseAlert(a){const card=el('div','nx-alert nx-glass nx-glass--raised'),t=el('div','nx-alert-t');t.append(el('b','',a.title),el('span','',a.detail));
 const row=el('div','nx-alert-row');if(a.act)row.append(button({label:a.act.label,size:'row',kind:'primary',title:a.act.label,does:'',onPress:()=>a.act.go()}));
 row.append(button({icon:'i-close',size:'icon',kind:'ghost',title:'Dismiss',does:'Hides this until the problem clears and comes back.',onPress:()=>{quiet.add(a.key);dropAlert(a.key);}}));
 card.append(icon('i-warn'),t,row);alertHost().prepend(card);shown.set(a.key,card);while(alertHost().children.length>3)alertHost().lastChild.remove();try{navigator.vibrate?.([60,40,60]);}catch{}lockControls(card);}
function dropAlert(k){shown.get(k)?.remove();shown.delete(k);}

// ---------- camera pinned on the main view ----------
let camPinned=(()=>{try{return localStorage.getItem('nx-campin')==='1';}catch{return false;}})();
function setCamPinned(v){camPinned=v;try{localStorage.setItem('nx-campin',v?'1':'0');}catch{}$('nx-camtoggle')?.setAttribute('aria-pressed',String(v));paintCamPin();}
async function paintCamPin(){let box=$('nx-campin');if(!camPinned||['off','service','replay'].includes(link().state)){if(box){box.querySelector('img')?.setAttribute('src','');box.remove();}return;}
 if(box)return;box=el('div','nx-campin nx-glass');box.id='nx-campin';$('nx-ui').append(box);try{const cams=await cameras();if(!cams.length){box.remove();return;}const img=camImg(cams[0]);img.addEventListener('click',()=>pageCamera());tip(img,'Camera','Live. Tap for the camera page.');
  const x=button({icon:'i-close',size:'icon',kind:'ghost',title:'Unpin',does:'Removes the camera from the main view.',onPress:()=>setCamPinned(false)});
  const grip=el('div','nx-campin-grip');grip.append(el('i'));tip(grip,'Move','Drag to move the camera anywhere; it stays where you leave it.');box.append(grip,img,x);placeCamPin(box);dragCamPin(box,grip);}catch{box.remove();}}
// Turn the camera on from the main screen and drag it out of the way.
function placeCamPin(box){let p=null;try{p=JSON.parse(localStorage.getItem('nx-campin-pos')||'null');}catch{}if(!p)return;
 const w=box.offsetWidth||300,h=box.offsetHeight||200;box.style.left=Math.max(8,Math.min(p.x,innerWidth-w-8))+'px';box.style.top=Math.max(8,Math.min(p.y,innerHeight-h-8))+'px';box.style.bottom='auto';box.style.right='auto';}
function dragCamPin(box,grip){grip.addEventListener('pointerdown',e=>{e.preventDefault();grip.setPointerCapture(e.pointerId);const r=box.getBoundingClientRect(),dx=e.clientX-r.left,dy=e.clientY-r.top;box.dataset.drag='';
 const mv=ev=>{box.style.left=Math.max(8,Math.min(ev.clientX-dx,innerWidth-r.width-8))+'px';box.style.top=Math.max(8,Math.min(ev.clientY-dy,innerHeight-r.height-8))+'px';box.style.bottom='auto';box.style.right='auto';};
 const up=()=>{grip.removeEventListener('pointermove',mv);delete box.dataset.drag;const b=box.getBoundingClientRect();try{localStorage.setItem('nx-campin-pos',JSON.stringify({x:Math.round(b.left),y:Math.round(b.top)}));}catch{}};
 grip.addEventListener('pointermove',mv);grip.addEventListener('pointerup',up,{once:true});grip.addEventListener('pointercancel',up,{once:true});});}
// ---------- maintenance due (checked every ten minutes) ----------
let hoursAt=0;function paintMaintenance(){if(performance.now()-hoursAt<600000&&hoursAt)return;hoursAt=performance.now();printHours().then(r=>{state.printHours=r.h;});}

// ---------- boot ----------
(async()=>{
 await loadSprite();initTips();applyLayout();addEventListener('resize',applyLayout);buildInstruments();
 const noScene=hash.has('noscene')||!show3d();if(noScene){$('nx-scene').hidden=true;document.body.dataset.scene='off';$('nx-loading')?.remove();stageCard();}
 if(!noScene){try{scene=createScene($('nx-scene'),id=>{if(id)openPart(id);},{stage:'next'});}catch(e){toast('3D view unavailable: '+e.message,'alarm');}}
 initPages({state,send,lockControls,can,why,jsend,scene:()=>scene,files:sheetFiles,tick:()=>lastTick,probes:()=>[...probeStore.values()].sort((a,b)=>a.ts-b.ts),activity:()=>state.live?lastTick?.activity:null,toolpath:()=>tp,
  savePresets:async body=>{await jsend('/api/presets','PUT',body);await loadPresets();render();},meshOnPrinter:v=>{if(v!==undefined)setMeshOn(v);return meshOn;},camPinned:v=>{if(v!==undefined)setCamPinned(v);return camPinned;}});initKiosk();
 fetch('/api/meta').then(r=>r.json()).then(m=>{const p=m.printer||{};if(p.name){$('nx-id-name').textContent=p.name.toUpperCase();document.title=p.name+' · Stella Scope';}$('nx-id-model').textContent=(p.model||'').toUpperCase();}).catch(()=>{});
 fetch('/api/capabilities').then(r=>r.json()).then(c=>{state.desktopAccess=c.desktop===true;if(state.desktopAccess){phoneBadge();setInterval(()=>{if(!document.hidden)phoneBadge();},5000);}}).catch(()=>{});
 openSse();requestAnimationFrame(tickFrame);whenLoaded();loadPresets().then(render);controls(true).then(render);request(state.live?null:state.cursor);
 setInterval(()=>{if(!document.hidden)controls().then(()=>lockControls());},8000);
 render();
})();
