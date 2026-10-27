// Stella Scope NEXT app shell (docs/next-ui/BLUEPRINT.md). Live data, the printer-state rule, the
// safe command path and every sheet. The data plumbing is ported from service.js / hud.js with the
// same constants (comments name the originals), so NEXT sees and sends exactly what the old HUD did.
import {createScene} from './hud-scene.js';
import {insideTravel,printerLink,controlsSummary,linkAllows,preheatMenu,presetDiff,volumetricFlow} from './hud-core.js';
import {CONTROLS,controlItems,controlsLacking,plainState,sinceText,fromTick,deriveMode,dockFor,moreMenu,jobModel,flowModel,heatColor,trackHeat,holdWords,HOLD_NOT_SENT,cancelVerdict,fmtTemp,fmtDuration,fmtPct,fileTitle,SUB_MENUS,homingFrame,xyTravel,driverFlags,supportsFrom,printerWords,plainRefusal,accessibleName} from './next-core.js';
import {bindHold} from './next-gesture.js';
import {pageCalibrate} from './next-calibrate.js';
import {sheetTimelapses} from './next-report.js';
import {pageHealth} from './next-health.js';
import {sheetFeedback} from './next-feedback.js';
import {$,el,icon,loadSprite,initTips,tip,setWhy,button,busy,openPopover,addFromStandard,notices,onNotice,commandBar,openSheet,openModal,closeSheet,markOpener,clearOpener,closePopover,section,openRadial,toast,segmented,stepper,stat,numberField,holdLine,holdLineOff,lookPanel} from './next-ui.js';
import {initStage,camZoom,isGrown,drop as dropStage} from './next-stage.js';
import {control as nxc,CONTROLS as NXC,key as nxkey,click as nxclick} from './next-control.js';
import {GcodeFollower} from './gcode-follow.js';
import {sheetPrinters,sheetAddPrinter} from './next-farm.js';
import {initSwitcher} from './next-switch.js';
import {sheetQueue} from './next-queue.js';
import {initPages,paintPages,openPart,openFile,pageLevel,pageThermal,pageSignals,pageTimeline,pageParts,pagePrint,pageAlerts,pageCauses,pageJournal,pageAssembly,pageMotion,activityCard,liveProbes,meshRgb,openPartByKey,showPartByKey,pageFilament,filamentTick,filamentQuick,pageCamera,pageShaper,pageHistory,pageMaintenance,printHours,dueTasks,adoptJobs,pagePlates,cameras,camImg,u06,relWheel,filamentWheel,filamentStop,filamentBusy} from './next-pages.js';

const hash=new URLSearchParams(location.hash.slice(1));
// #wheel=<key> presses that dock key once the page is up, to open its wheel (screens and checks). Nothing is sent: a wheel only opens.
let wheelOnce=hash.get('wheel');
// #sheet=<name> opens that page once the page is up (screens and checks), as its rail key would.
let sheetOnce=hash.get('sheet');
const finite=v=>typeof v==='number'&&Number.isFinite(v)?v:null;
const state={live:!hash.has('ts'),cursor:hash.has('ts')?Number(hash.get('ts')):null,snapshot:null,link:null,tick:null,readings:fromTick(null),
 mode:'checking',dockKey:'',hist:{nozzle:[],bed:[]},busy:false,seq:0,controller:null,presets:null,playing:false,speed:1,desktopAccess:false};

// ---------- 5. layout classes ----------
// Back to the farm: the words where there is room, "FARM" where there is not (a phone, a tablet)
const backLabel=()=>['phone','phone-land','tablet','kiosk'].includes(document.body.dataset.layout)?'FARM':'BACK TO THE FARM';
function layoutClass(){const w=innerWidth,h=innerHeight;
 if(w<600)return 'phone';if(h<440&&w<1000)return 'phone-land';if(h<=640&&w<=1100&&w>h)return 'kiosk';
 if(w<1280)return h>w?'tablet-port':'tablet';return 'desktop';}
function applyLayout(){const c=layoutClass();if(document.body.dataset.layout!==c){document.body.dataset.layout=c;state.dockKey='';dropStage();render();reframe();if(ui.back)ui.back.el.querySelector('.nx-lbl').textContent=backLabel();}if(gaugesFolded)placeFold();}
// The camera is fitted once the model has loaded and again whenever the layout class changes, so
// the machine is framed inside its own region at every screen size (BLUEPRINT 9).
let reframeTimer=0;function reframe(){clearTimeout(reframeTimer);reframeTimer=setTimeout(()=>{if(scene?.entries?.length>100){scene.reset();framePhone();}},120);}
// U02: in portrait the machine is framed in the band between the header and the bank of gauges, not in the middle of the whole screen; the lift is kept so a grown machine can be centred again.
let stageShift=0;
function framePhone(){stageShift=0;if(!scene?.cameraState||innerWidth>=innerHeight)return;scene.zoom?.(.78);   // the fit is made for a wide window; upright, the machine can use the width
 const job=$('nx-job'),a=$('nx-top').getBoundingClientRect().bottom+(job.hidden?0:job.getBoundingClientRect().height+16);
 const b=Math.min($('nx-dock').getBoundingClientRect().top,gaugesFolded?1e9:$('nx-gauges').getBoundingClientRect().top||1e9),up=innerHeight/2-(a+b)/2;if(!(up>0))return;
 const s=scene.cameraState,d=Math.hypot(...s.position.map((v,i)=>v-s.target[i])),el=(s.position[2]-s.target[2])/d,mm=2*d*Math.tan(16*Math.PI/180)/innerHeight*up/Math.sqrt(1-el*el);
 stageShift=mm;scene.look(s.position.map((v,i)=>i===2?v-mm:v),s.target.map((v,i)=>i===2?v-mm:v));}
// No printer model on this computer (a fresh install has none): drop the 3D view for the camera card, as on a phone.
// Runs once per page: with the 3D view already off (phone, ?noscene) the camera card exists, and a
// second card would open a second camera stream (the recorder allows two in total).
// U03 a (2026-10-20): the machine was shown by a poll every 500 ms plus a 400 ms wait, so it appeared up to 0.9 s after it was ready (measured: 25 to 800 ms late). The scene says when it is
// (stella-geometry-ready), the poll stays for the failure path, and the wait is the 120 ms the reframe needs.
let sceneShown=false;
function whenLoaded(start=performance.now()){if(document.body.dataset.scene==='off'||sceneShown)return;const renderer=$('nx-scene')?.dataset.renderer;
 if(!scene||renderer==='source-unavailable'||performance.now()-start>90000){window.__stellaNoClassic=renderer==='source-unavailable'||!scene;scene=null;$('nx-scene').hidden=true;document.body.dataset.scene='off';$('nx-loading')?.remove();stageCard();return;}
 if(renderer==='source-cad'){sceneShown=true;reframe();setTimeout(()=>{document.body.dataset.scene='ready';},160);return;}
 setTimeout(()=>whenLoaded(start),250);}

// ---------- live data (ported from service.js: get, request, SSE, back-off, jitter buffer) ----------
const SERVICE_BACKOFF_S=[1,2,5,10],svc={down:false,since:null,fails:0,next:0,link:null};   // service.js:13
const TICK_DELAY_S=0.45;                                                                   // service.js:320
let scene=null,lastTick=null,lastFull=performance.now(),tickBusy=false,sse=null,sseOk=false,interpAt=0,lastSceneDraw=0;
const tickBuffer=[],homeMap=homingFrame(),probeStore=new Map();
async function get(url,signal){let r;try{r=await fetch(url,{cache:'no-store',signal});}catch(e){if(e.name!=='AbortError')serviceLost();throw e;}
 serviceBack();const v=await r.json();if(!r.ok)throw Error(v.error||`Stella Scope answered with an error (${r.status}).`);return v;}
async function request(ts=state.live?null:state.cursor,step=null){const seq=++state.seq;state.controller?.abort();state.controller=new AbortController();state.busy=true;lastFull=performance.now();
 const q=new URLSearchParams({window:600,points:240});if(finite(ts)!==null)q.set('ts',ts);if(step)q.set('step',step);
 try{const r=await get('/api/workspace?'+q,state.controller.signal);if(seq!==state.seq)return;state.snapshot=r;if(r.observer_now)state.link=r.observer_now;state.cursor=r.cursor;
  if(Array.isArray(r.history)&&(!state.histFilled||!state.live)){state.hist={nozzle:[],bed:[]};for(const p of r.history)histPush(p.ts,p);if(r.point)histPush(r.point.ts,r.point);state.histFilled=true;}
  if(!state.live)state.readings=fromTick({...r,point:r.point,job:r.job||r.point?.job,homed_axes:r.point?.homed});
  if(!state.live&&scene&&r.point)scene.set(r,null);updateUrl();render();}
 catch(e){if(e.name!=='AbortError'&&seq===state.seq){// a busy machine can take more than 5 s to read the history: live, the next tick fills it, so it is no alarm (2026-10-25: no jargon in the lane)
  if(/time budget/.test(e.message)){if(!state.live)toast('The recording took too long to read just now. Try that moment again.','warn');}else if(!svc.down)toast(e.message,'alarm');render();}}
 finally{if(seq===state.seq)state.busy=false;}}
function takeTick(r){if(!r||r.schema!=='stella-scope.tick.v1')return;lastTick=r;                        // service.js:298
 // Probe landings arrive on some ticks and not others, newest first; they are kept here, oldest
 // first and once each, for the live mesh views and the probe marks on the plate.
 for(const q of r.point?.probe_points||[])if([q.x,q.y,q.z,q.ts].every(v=>finite(v)!==null))probeStore.set(q.ts+':'+q.x+':'+q.y,q);
 if(probeStore.size>600){const cut=(r.point?.ts??Date.now()/1000)-2400;for(const [k,q]of probeStore)if(q.ts<cut)probeStore.delete(k);}
 // G28 reports in Klipper's homing frame; homingFrame (next-core) turns it back into real travel first.
 // finish-2 F05 (Opus supplement 1): with no axis homed Klipper reports X 0 Y 0 Z 0, which is not where the head is; the scene keeps
 // its last pose and the readout says NOT HOMED, until G28 runs (homing is followed as before)
 if(has('home')&&(r.point?.homed??r.homed_axes)===''&&r.activity?.id!=='homing'&&r.point)r.point.position=null;
 if(r.point?.position){let start=null;try{start=JSON.parse($('nx-scene').dataset.drawnPose||'null');}catch{}r.point.position=homeMap(r.point.position,{limits:r.point.limits,velocity:r.point.velocity,ts:r.point.ts,start,homed:r.point.homed??r.homed_axes});
  // The mapped pose is real travel for every axis (homed or tracked from where it was), so the scene
  // draws all three; an axis whose frame was just re-set is marked so the scene lands it at once.
  r.point.rebased=r.point.position.rebased||'';if(typeof(r.point.homed??r.homed_axes)==='string')r.point.homed='xyz';}
 if(insideTravel(r.point?.position,r.point?.limits)){tickBuffer.push(r);if(tickBuffer.length>12)tickBuffer.shift();}
 applyTick(r);}
// Temperature history for the thermal cards: one point every 2 s, ten minutes kept, backfilled from
// the recording on the first snapshot (the recorder's own indexed readings), then from every tick.
function histPush(ts,p){if(finite(ts)===null||!p)return;for(const k of ['nozzle','bed']){const h=state.hist[k],v=p[k];if(!v)continue;trackHeat(k,ts,finite(v.temperature),finite(v.target));   // the over-temperature rule reads the trend, not one number (next-core heatColor)
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
function link(){const l=printerLink(state.link,{replay:replayMode(),lastSeen:state.link?.last_success??state.snapshot?.last_ts,klipper:caps.capabilities?.klipper!==false,brand:caps.brand||'the printer'});return {...l,label:plainState(l.label),reason:plainState(l.reason)};}   // hud.js:141
async function controls(force=false){if(!force&&performance.now()-capsAt<8000)return caps;
 try{const r=await fetch('/api/controls');if(!r.ok)throw Error('Printer controls are not available right now.');caps=await r.json();
  if(replayMode())caps.reason='This is a saved recording. Printer controls are not available.';else if(caps.reason==='controls_enabled is false')caps.reason='Printer controls are switched off in Stella Scope. The Help guide, under Controls, shows how to switch them on.';}
 catch(e){caps={enabled:false,reason:e.message,commands:[]};}capsAt=performance.now();lockControls();applyCaps();return caps;}
// What this printer can do at all, from /api/controls (farm board WO-02). Unknown until it answers.
function has(flag){return caps.capabilities?.[flag]!==false;}
// Pages whose description speaks of Klipper get a plain one on any other brand (polish WO-05, 2026-10-18; the rule: no Klipper words on a printer that is not).
const NEUTRAL={Print:'The job on the plate: progress, the layer, pause and cancel.',Alerts:'The last seven days: printer offline and Stella Scope restarts.'};
const doesOf=(name,d)=>has('klipper')||!NEUTRAL[name]?d:NEUTRAL[name];
// WO-13 (2026-10-12): never show a control the driver says it lacks. The columns and the fan row
// are flex stacks, so a hidden instrument leaves no empty slot.
function applyCaps(){const hide=(elm,off)=>{if(elm)elm.style.display=off?'none':'';};
 for(const b of document.querySelectorAll('#nx-rail button[data-tip-title]')){const t=b.dataset.tipTitle;if(NEUTRAL[t]){b.dataset.tipDoes0??=b.dataset.tipDoes;b.dataset.tipDoes=has('klipper')?b.dataset.tipDoes0:NEUTRAL[t];}}
 // Commands the driver lacks are never drawn rotating/dashed on the wall (2026-10-12): speed,
 // flow, part fan, the two automatic Klipper fans. Temperature dials stay even when the driver
 // cannot set heat (PrusaLink): the printer still reports its temperatures and the dials show them.
 hide(ui.speed?.el,!has('speed')&&finite(state.readings?.speedFactor)===null);hide(ui.flow?.el,!has('flow')&&finite(state.readings?.extrudeFactor)===null);
 if(!has('speed'))ui.speed?.lock('This printer reports its speed but does not take a change from here.');if(!has('flow'))ui.flow?.lock('This printer reports its flow but does not take a change from here.');
 hide(ui.fans?.part?.el,!has('fan'));hide(ui.fans?.hotend?.el,has('klipper')===false);hide(ui.fans?.controller?.el,has('klipper')===false);
 hide(document.querySelector('.nx-fans'),!(has('fan')||has('klipper')));
 for(const x of document.querySelectorAll('#nx-pos .nx-pos-x'))x.style.display=has('klipper')?'':'none';
 const est=$('nx-estop');if(est)est.hidden=!has('emergency_stop');
 // the two tips that name Klipper read plainly on any other brand (polish WO-06 item 8)
 {const w=printerWords(has('klipper'));if(est)tip(est,'Emergency stop',w.estop);const pe=$('nx-pos');if(pe)tip(pe,'Head position',w.position);}
 // the command search finds commands and macros; typed G-code goes in the Console (polish WO-06)
 const ci=document.querySelector('.nx-cmd-host input');if(ci)ci.placeholder='Find a command…';}
function can(id){const l=link();if(!linkAllows(l,id))return false;return caps.enabled&&(id==='emergency_stop'||state.live)&&caps.commands?.some(c=>c.id===id&&c.allowed);}
function why(id){const l=link();if(!linkAllows(l,id))return l.reason;if(id!=='emergency_stop'&&!state.live)return 'Go back to live first. Controls only reach the live printer, never a past moment.';
 return plainRefusal((caps.enabled?caps.commands?.find(c=>c.id===id)?.reason:null)||caps.reason||'Not available right now.');}
function lockControls(root=document){for(const b of root.querySelectorAll('[data-cmd]')){const id=b.dataset.cmd,off=!can(id);
 if(off){if(!b.disabled){b.disabled=true;b.dataset.locked='1';}setWhy(b,why(id));}else{if(b.dataset.locked){b.disabled=false;delete b.dataset.locked;}setWhy(b,'');}}}
async function execute(id,extra,confirm,label=''){const c=await controls(true);if(id!=='emergency_stop'&&!state.live){toast(why(id),'alarm');return null;}if(!can(id)){toast(why(id),'alarm');return null;}
 try{const r=await fetch('/api/commands',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token||''},body:JSON.stringify({id,confirm,...extra})});
  const result=await r.json();if(!r.ok)throw Error(result.error||'The printer refused it and gave no reason.');
  // One plain line (2026-09-25: two prompts, one saying it was not observed, were confusing); a
  // typed command's own reply from Klipper follows from consoleEcho().
  toast(id==='emergency_stop'?result.message:'Sent: '+String(label||id.replace(/_/g,' ')).replace(/\.$/,'')+'.','ok');return result;}
 catch(e){toast(plainRefusal(e.message)+' No automatic retry.','alarm');return null;}}
// The same flow as hud.js send(): nothing is asked of the service for a control that cannot run;
// the exact G-code is previewed; T3 commands stop for one review unless the service says review:false.
// Klipper's answer to a typed command (2026-09-25: QUERY_PROBE "didn't do anything" — it did;
// the reply sat in the console log). The recorder pulls Moonraker's console into events; after a
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
  preview=await r.json();if(!r.ok)throw Error(preview.error||'Stella Scope could not show what it would send.');}catch(e){toast(plainRefusal(e.message),'alarm');return null;}
 if(preview.tier!=='T3'||preview.review===false){const script=(preview.script||preview.action||'').toString().trim();return execute(id,extra,preview.tier==='T3',script.split('\n')[0]);}
 return new Promise(resolve=>{let done=false;const finish=v=>{if(!done){done=true;resolve(v);}};
  openModal({title:'Review command',icon:'i-console',anchor:reviewAnchor,onClose:()=>finish(null),build:(body,close)=>{
   body.append(el('p','nx-note',preview.script?'Check the exact G-code. It goes to the printer once, with no automatic retry.':'Check what will happen. It goes to the printer once, with no automatic retry.'),el('pre','nx-review',preview.script||preview.action||''));
   const go=button({icon:'i-check',label:'Send once',kind:'primary',size:'row',cmd:id,title:'Send once',does:preview.script?'Sends the G-code shown here one time.':'Sends this to the printer one time.',onPress:async b=>{busy(b,true);const r=await execute(id,extra,preview.tier==='T3',String(preview.script||preview.action||'').trim().split('\n')[0]);busy(b,false);if(r){finish(r);close();}}});
   const cancel=button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Nothing is sent.',onPress:()=>close()});
   body.append(section('',el('div','nx-grid2')));body.lastChild.lastChild.append(cancel,go);lockControls(body);}});});}

// ---------- instruments ----------
const ui={};
// U02 (2026-10-20, UI board): every number on this page is the kit's control (next-control.js): an LED gauge at rest, a black knob when held, and
// the printer is told once, on release, through send() with its review. gauge() gives each one the three things render() has always used: .el, .set(model), .lock(why).
// The old dials, the fan rings, the knob/bar style and the little corner arrows are gone with it (2026-10-20: no pop-ups, no corner arrows).
const heater=k=>k==='nozzle'||k==='bed';
function gauge(k,{wire,title,does,readOnly=false,onTap=null}){
 const spec=NXC.find(c=>c.key===k);let model={},pending=null,waiting=false,at=0;
 // what the hand set is kept on the face until the printer says so (or refuses, or 8 s pass); a review that is still open is not a refusal
 const go=v=>{pending=v;waiting=true;Promise.resolve(wire(v)).then(r=>{waiting=false;at=performance.now();if(!r)pending=null;set(model);},()=>{waiting=false;pending=null;set(model);});};
 // 2026-10-25 ("radial menus everywhere"): a tap on a heater opens a wheel of the material temperatures, OFF, and the typed number; a turn of the knob still sets any value
 const onTapWheel=heater(k)?(kit,{open})=>{const pr=state.presets||{},bed=k==='bed',seen=new Set(),r=kit.el.getBoundingClientRect();
  const items=(pr.materials||[]).filter(m=>{const t=bed?m.bed:m.nozzle;if(!(t>0)||seen.has(t+m.name))return false;seen.add(t+m.name);return true;}).slice(0,9).map(m=>{const t=bed?m.bed:m.nozzle;return {key:'t-'+m.name,icon:'i-heat',label:m.name.toUpperCase(),sub:t+'°',tone:'heat',num:false,title:`${m.name}: ${t}°`,does:`Sets the ${bed?'bed':'nozzle'} to ${t}°, the ${m.name} preset.`,t};});
  items.push({key:'t-off',icon:'i-cool',label:'OFF',tone:'cool',title:'Off',does:`Turns the ${bed?'bed':'nozzle'} heater off.`,t:0},{key:'t-type',icon:'i-console',label:'TYPE',title:'Type a temperature',does:'The number field, for any temperature.'});
  openRadial(items,{x:r.left+r.width/2,y:r.top+r.height/2,from:{x:r.left+r.width/2,y:r.top+r.height/2},title:bed?'Bed':'Nozzle',center:()=>{const m=model||{};return `Now ${finite(m.cur)!==null?Math.round(m.cur)+'°':'—'} · target ${finite(m.value)!==null?Math.round(m.value)+'°':'—'}`;},
   onPick:it=>{if(it.key==='t-type'){open();return;}kit.set(it.t);go(it.t);}});}:null;
 const c=nxc({...spec,sweep:true,value:spec.min,now:heater(k)?0:null,readOnly,onTap,onTapWheel,onSet:go});
 function set(m){model=m;const v=m.value;c.setUnknown(v===null||v===undefined);if(heater(k))c.setNow(finite(m.cur));
  // a heater on its way up breathes (2026-10-25, "alive"): the light in its face swells and settles until it reaches the target
  if(heater(k)){const cur=finite(m.cur),t=finite(v);c.el.toggleAttribute('data-heating',t!==null&&t>0&&cur!==null&&t-cur>3);}
  if(v===null||v===undefined||c.el.classList.contains('live'))return;
  if(pending!==null){if(waiting)return;if(Math.round(v)===pending||performance.now()-at>8000)pending=null;else return;}
  c.set(v);}
 tip(c.el,title,does);
 return {el:c.el,set,kit:c,lock(why){c.setLocked(why);setWhy(c.el,why||'');}};}
function buildControls(){
 ui.nozzle=gauge('nozzle',{wire:v=>send('set_heater',{target:'extruder',value:v}),title:'Nozzle',does:'Press and hold, then turn: the nozzle target moves 5° a click. Letting go sends it once. The dot on the ring is the live temperature.'});
 ui.bed=gauge('bed',{wire:v=>send('set_heater',{target:'heater_bed',value:v}),title:'Bed',does:'Press and hold, then turn: the bed target moves 1° a click. Letting go sends it once. The dot on the ring is the live temperature.'});
 ui.speed=gauge('speed',{wire:v=>send('speed_factor',{value:v}),title:'Speed',does:'Press and hold, then turn: the speed moves 5% a click, 100% is as sliced. Letting go sends it once.'});
 ui.flow=gauge('flow',{wire:v=>send('extrude_factor',{value:v}),title:'Flow',does:'Press and hold, then turn: how much plastic every move pushes out, 1% a click, 100% is as sliced. Letting go sends it once.'});
 $('nx-left').prepend(ui.nozzle.el,ui.bed.el);$('nx-right').prepend(ui.speed.el,ui.flow.el);
 $('nx-ui').append(document.getElementById('nxctl-dim'));   // the dim lives in the page, so the held gauge (z 41 in its own cluster) rises above it and only the page behind is dimmed
 state.dockKey='';if(ui.fans)render();}
function paintReticle(){if(!ui.ret)return;const p=[...probeStore.values()].reduce((a,q)=>!a||q.ts>a.ts?q:a,null),fresh=p&&Date.now()/1000-p.ts<10;
 const t=fresh?`PROBE ${p.z>=0?'+':''}${p.z.toFixed(4)}`:'';if(ui.retProbe.textContent!==t)ui.retProbe.textContent=t;ui.ret.toggleAttribute('data-probe',!!fresh);}
// The one bottom bar (U02): the position on the left, then this state's keys (the kit's square key, same slots dockFor() has always chosen), then Gauges.
function renderBar(slots,{press,message}={}){const host=$('nx-dock'),keys=host.querySelector('.nx-keys');keys.replaceChildren();host.toggleAttribute('data-msg',!!message);
 if(message){const m=el('div','nx-dock-msg');m.append(icon(message.icon||'i-warn'),el('span','',message.text));keys.append(m);return [];}
 const out=[];let group=null,pod=null;
 for(const s of slots.filter(x=>x.visible!==false)){if(pod===null||s.group!==group){pod=el('div','nx-keygroup');keys.append(pod);}group=s.group;
  const title=s.title||s.label.charAt(0)+s.label.slice(1).toLowerCase();let b;const k=nxkey({label:s.label,icon:String(s.icon||'').replace(/^i-/,''),onPress:s.hold?null:()=>press(s,b)});b=k.el;
  b.dataset.kind=s.primary?'primary':s.danger?'danger':'default';if(s.cmd)b.dataset.cmd=s.cmd;if(s.attention)b.dataset.attention='';
  b.setAttribute('aria-label',accessibleName(s.label,title));tip(b,title,s.does||'');b.style.setProperty('animation-delay',(out.length*24)+'ms');b.nxSlot=s;b.dataset.key=s.key;
  // no little corner arrow on a key that opens something (2026-10-20): the key stays pressed while what it opened is open (data-showing)
  if(/^(menu|sheet|radial):/.test(s.action||''))b.dataset.opens=s.action.split(':')[0];
  // a key that cancels something is held, not tapped (hold-to-confirm, as the old dock did)
  if(s.hold){b.dataset.hold='';b.append(el('i','nx-hold'));bindHold(b,{ms:900,onStart:()=>holdLine(b,holdWords(s.cmd||s.key,s.title||s.label)),onConfirm:()=>{holdLineOff();nxclick(true);press(s,b);},onCancel:()=>holdLine(b,HOLD_NOT_SENT,'warn',3500)});b.dataset.tipDoes=(s.does?s.does+' ':'')+'Press and hold to confirm.';}
  pod.append(b);out.push(b);}
 return out;}
// Gauges fold down into the bar and open back out of it (2026-10-20). Each folded piece flies to the middle of the bar: the offset is measured open, then the class does the move.
// 2026-10-23: the GAUGES key is gone (it only folded or grew the rings); the gauges always stand on the machine. A fold kept on a device from before is ignored.
let gaugesFolded=false;
function placeFold(){const b=document.body,was=b.dataset.gauges;b.dataset.gauges='';b.classList.add('nx-still');const bar=$('nx-dock').getBoundingClientRect(),bx=bar.left+bar.width/2,by=bar.top+bar.height/2;
 for(const e of [$('nx-left'),$('nx-right'),$('nx-gauges')]){const r=e.getBoundingClientRect();if(!r.width)continue;e.style.setProperty('--fx',Math.round(bx-(r.left+r.width/2))+'px');e.style.setProperty('--fy',Math.round(by-(r.top+r.height/2))+'px');}
 void b.offsetWidth;b.dataset.gauges=was||'';void b.offsetWidth;b.classList.remove('nx-still');}
function foldGauges(fold,instant=false){gaugesFolded=fold;try{localStorage.setItem('nx-gauges',fold?'folded':'open');}catch{}
 ui.gk?.setPressed(!fold);const b=document.body;if(instant)b.classList.add('nx-still');placeFold();b.dataset.gauges=fold?'folded':'';if(instant){void b.offsetWidth;b.classList.remove('nx-still');}}
function buildInstruments(){
 buildControls();
 ui.fans={part:gauge('part',{wire:v=>send('set_fan',{value:v/100}),title:'Part cooling fan',does:'Press and hold, then turn: the part cooling fan moves 5% a click. Letting go sends it once.'}),
  hotend:gauge('hotend',{readOnly:true,onTap:()=>openPartByKey('hotend_fan')||sheetFan(),title:'Hotend fan',does:'Automatic: Klipper runs it whenever the nozzle is hot, so it does not turn. Tap to change the temperature it starts at.'}),
  controller:gauge('board',{readOnly:true,onTap:()=>openPartByKey('controller_fan')||sheetFan(),title:'Electronics bay fans',does:'Automatic: Klipper runs them while the motors or heaters are on, so they do not turn. Tap for details.'})};
 const fans=el('div','nx-fans');fans.append(ui.fans.part.el,ui.fans.hotend.el,ui.fans.controller.el);
 $('nx-right').append(fans);
 // Z beside the nozzle, and each probe result as it lands (the old HUD's reticle, 2026-09-24).
 const ret=el('div','nx-reticle');ret.hidden=true;ui.retZ=el('b');ui.retProbe=el('span');ret.append(ui.retZ,ui.retProbe);$('nx-ui').append(ret);ui.ret=ret;
 // Polish WO-03 (Opus S4): the model view of a catalogue machine says the part is on the plate once it really is.
 document.addEventListener('stella-part-drawn',e=>toast(`${e.detail.layers} layers of ${fileTitle(e.detail.file)} drawn on the plate.`));
 document.addEventListener('stella-track',e=>{const {x,y,on}=e.detail;ret.hidden=!on||document.body.dataset.layout==='phone';if(on)ret.style.transform=`translate(${Math.round(x)}px,${Math.round(y)}px)`;});
 // position capsule
 const pos=$('nx-pos');ui.axes={};for(const a of ['X','Y','Z']){const w=el('div','nx-axis');w.append(el('b','',a),el('span','','—'));ui.axes[a]=w;pos.append(w);}
 // Polish WO-03 (2026-10-18): a position that is not the printer's own live reading says where it comes from, here beside the
 // numbers, for a demo printer always. Never a position with its source left out.
 const src=el('div','nx-pos-src');src.append(el('small','','POSITION'),el('span','','from the print file'));src.hidden=true;
 src.title='This printer is not reporting where its head is right now. This is where the print file puts it at this point of the print.';ui.posSrc=src;pos.append(src);
 const sp=el('div','nx-pos-speed');ui.vel=el('span','','—');sp.append(ui.vel,el('small','','mm/s'));pos.append(el('i','nx-pos-sep'),sp);
 // 2026-09-25: more useful numbers by the position — the Z offset in use, the acceleration
 // limit, the top speed, and whether the gantry is levelled.
 const xi=(k,label,tt)=>{const w=el('div','nx-pos-x');w.dataset.k=k;w.title=tt;const v=el('span','','—');w.append(el('small','',label),v);return [w,v];};
 const [zo,zov]=xi('zoff','Z OFFSET','Z offset in use, live adjustments included'),[ac,acv]=xi('acc','ACCEL','Acceleration the printer may use, mm/s²'),[mv,mvv]=xi('vmax','TOP SPEED','Top speed the printer may use, mm/s'),[ql,qlv]=xi('qgl','GANTRY','Gantry levelled since homing (quad gantry level)');
 ui.px={zo:zov,ac:acv,mv:mvv,ql,qlv};pos.append(el('i','nx-pos-sep nx-pos-x'),zo,ac,mv,ql);
 tip(pos,'Head position','Where Klipper was told to put the head, in mm, smoothed between readings. A green axis letter means that axis is homed.');
 // U02 (2026-10-20): one bottom bar holds the position, the keys for this state and a Gauges key; the gauges fold down into it and open back out of it. No corner arrows anywhere.
 // 2026-10-24: the bottom bar is the command deck. Its left end is the live lane: what the printer is doing right now (heating with its
 // progress, printing, a filament step with its own stop) and, sliding in over it, each new notification. Nothing of this sits over the machine.
 {const bar=$('nx-dock'),lane=el('button','nx-lane');lane.type='button';lane.id='nx-lane';const base=el('div','nx-lane-base'),ico=el('span','nx-lane-ico'),txt=el('span','nx-lane-txt'),bar_=el('i','nx-lane-bar'),toasts=el('div','nx-lane-toasts');toasts.id='nx-lane-toasts';
  ui.lane={el:lane,ico,t:el('b'),d:el('small'),bar:bar_};txt.append(ui.lane.t,ui.lane.d);base.append(ico,txt);lane.append(base,bar_,toasts);bar.insertBefore(lane,bar.firstChild);
  tip(lane,'What the printer is doing','Heating, printing or a filament step, and each new message as it comes. Tap for the messages.');lane.addEventListener('click',e=>{if(e.target.closest('.nx-filmark'))return;openNotices(lane);});}
 {const bar=$('nx-dock'),row=bar.querySelector('.nx-keyrow'),mini=el('div','nx-bar-mini');ui.mini={noz:el('b','','—'),bed:el('b','','—')};
  for(const [k,l]of [['noz','NOZZLE'],['bed','BED']]){const w=el('span','',l);w.append(ui.mini[k]);mini.append(w);}bar.insertBefore(mini,row);
  // a clear way back to the farm, top left, on every size (2026-10-20)
  ui.back=nxkey({label:backLabel(),icon:'back',onPress:()=>{location.href='/farm';}});ui.back.el.id='nx-back';tip(ui.back.el,'Back to the farm','Leaves this printer and shows every printer in the farm on one wall.');$('nx-top').prepend(ui.back.el);}
 // rail
 const rail=$('nx-rail');rail.classList.add('nx-glass');
 // Rail (2026-09-24: "everything should be the new style"): the pages open as NEXT sheets; the
 // four the old service pages still own are kept below the second line, marked classic.
 // Rail (2026-09-24): the everyday pages on the rail, everything in Pages (the last icon).
 const pages=[['i-nav-hud','Printer','/','The printer page, where you are now: its instruments and every control.',1],['i-print','Print','sheet:print','The job on the plate: progress, the layer, Z offset and skip an object.',1],['i-filament-load','Filament','sheet:filament','Load and unload: heats, waits, then asks before it moves filament.',1],['i-view','Camera','sheet:camera','The printer camera, live.',1],['i-nav-leveling','Gantry & bed','sheet:level','Bed mesh as a map and a 3D surface, compare two meshes, build plates, Z offset and every gantry level run.',1],['i-probe','Calibrate','sheet:calibrate','Set up this printer: heaters, Z offset, gantry, mesh, input shaper, extruder, pressure advance and flow, in order, and a check before you print.',1],['i-nav-signals','Input shaper','sheet:shaper','Resonance graphs, the recommended shaper and belt tension.',0],['i-nav-thermal','Heat & fans','sheet:thermal','Nozzle and bed over the last ten minutes, heater output and the fans.',1],
  ['i-rewind','Print history','sheet:history','Every print, how it went, and the totals.',1],['i-nav-maintenance','Maintenance','sheet:maintenance','Reminders by hours printed.',1],['i-warn','Alerts','sheet:alerts','The last seven days: printer offline, motor driver warnings, Klipper errors and resent data.',1],['i-link','Phone & WhatsApp','sheet:phone','Pair your phone, and the WhatsApp alerts.',0],['i-nav-workspace','Farm','/farm','Every printer on one wall: state, progress, camera and time left.',0],['i-files','Queue','sheet:queue','Sliced files waiting for a printer that fits them, and the plate check before each start.',0],['i-printers','Printers','sheet:printers','Your printers: find and add one, rename it, remove it.',0],
  ['i-nav-workspace','Motion & motors','sheet:motion','Speed, Z height and every motor.',0],['i-nav-signals','Signals','sheet:signals','Every recorded signal, live, with a filter.',0],['i-nav-parts','Parts','sheet:parts','Every part on the printer; the live ones open their panel.',0],['i-nav-electronics','Electronics','sheet:electronics','Controller, toolhead board, host, relay and fans.',0],['i-nav-timeline','Timeline','sheet:timeline','Everything the printer said, in order.',0],['i-nav-diagnostics','Suspected causes','sheet:causes','Recorded warnings and the safe checks for each.',0],['i-panels','Service journal','sheet:journal','Work recorded against each part.',0],['i-nav-assembly','Assembly','sheet:assembly','The CAD this printer is drawn from, with every source file.',0],['i-colors','Look','sheet:look','The colour, the finish and the dial style of this screen.',0],['i-chart','Health','sheet:health','Belts, motors and heaters over time: how alike the belts answer, each axis\'s resonance, each motor driver and each heater\'s heat-up speed.',1],['i-camera','Timelapses','sheet:timelapse','A timelapse of every print with a camera, one picture a layer: play it back.',0],['i-spool','Materials','sheet:materials','Every material: temperatures, part fan, price, density, and the pressure advance and flow Calibrate measured.',0],['i-chat','Feedback','sheet:feedback','Tell 3D3D what to fix or add. Sent only when you press Send.',0],['i-help','Help','/help','The user guide: set up, add a printer, cameras, the queue, alerts, backup and what to do when something is wrong.',0],['i-more','All pages','sheet:pages','Every page of this printer.',1]];
 ui.pages=pages.filter(p=>p[2]!=='sheet:pages');let railed=0;for(const [ico,name,href,does,onRail]of pages){if(!onRail)continue;if(railed===1||railed===7||href==='sheet:pages')rail.append(el('hr'));railed++;const b=button({icon:ico,size:'icon',title:name,does,onPress:btn=>{if(href.startsWith('sheet:')){if(btn.hasAttribute('data-showing')){closeSheet();return;}Promise.resolve(openNamedSheet(href.slice(6))).then(()=>markOpener(btn));}else if(href!=='/')location.href=href;}});if(href==='/')b.setAttribute('aria-current','page');if(PAGE_NEEDS[href.slice(6)])b.dataset.needs=PAGE_NEEDS[href.slice(6)];rail.append(b);}
 // G-code command bar (top bar, left of centre)
 const cmdHost=el('div','nx-cmd-host');document.querySelector('.nx-id').after(cmdHost);
 ui.cmd=commandBar({host:cmdHost,locked:()=>link().lock?link().reason:'',openConsole:q=>sheetConsole(q),hasConsole:()=>has('console'),
  source:async()=>{const get=u=>fetch(u).then(r=>r.ok?r.json():{}).catch(()=>({}));
   const [help,mac,hist]=await Promise.all([link().lock?{}:get('/api/console/help'),link().lock?{}:get('/api/macros'),get('/api/console/history')]);
   const macros=new Set((mac.macros||[]).filter(m=>m.macro===true).map(m=>m.name.toUpperCase()));
   const commands=(help.commands||help.help||[]).map(h=>({name:h.name,help:h.help||'',macro:macros.has(String(h.name).toUpperCase())}));
   for(const m of mac.macros||[])if(m.macro===true&&!commands.some(c=>c.name.toUpperCase()===m.name.toUpperCase()))commands.push({name:m.name,help:m.help||'',macro:true});
   const history=[...new Set((hist.entries||hist.history||[]).map(h=>typeof h==='string'?h:h.script||h.line||'').filter(Boolean).reverse())].slice(0,20);
   const act=(label,icon,go,help,words='')=>({label,icon,go,help,words});
   // polish WO-05 (2026-10-18): the search offers only what this printer can do (a Prusa is not offered a gantry level or a firmware restart).
   // finish-2 F07 (2026-10-19): every control a person can press is found by its plain name (next-core.js CONTROLS), then the preheat presets, then
   // every page on the rail that is not already one of them. It found nothing for failure watch, view, speed, flow, move or motors before.
   const goTo=c=>c.how.startsWith('sheet:')?openNamedSheet(c.how.slice(6)):c.how.startsWith('cmd:')?send(c.how.slice(4)):c.how==='reprint'?send('print_start',{filename:state.readings.job.name}):c.how==='rec:live'?recAction('live'):c.how.startsWith('href:')?(location.href=c.how.slice(5)):null;
   // The five commands whose capability gate farm_tests asserts by name (polish WO-05) stay written out here; the rest of the controls come from CONTROLS, in its order.
   const five=[has('home')&&act('Home all axes','i-home',()=>send('home'),'send the head home','home homing reset start position'),has('level')&&act('Quad gantry level','i-level',()=>send('qgl'),'level the gantry','level gantry qgl'),has('mesh')&&act('Bed mesh','i-mesh',()=>send('mesh'),'probe the bed','probe bed level calibrate'),
    has('motors_off')&&act('Motors off','i-motors-off',()=>send('motors_off'),'let the motors go','motors steppers release disable unlock'),has('restart')&&act('Firmware restart','i-fw-restart',()=>send('firmware_restart'),'restart the firmware','restart reboot klipper halted')].filter(Boolean);
   const own=new Set(five.map(a=>a.label)),order=CONTROLS.map(c=>c.label),ix=l=>{const i=order.indexOf(l);return i<0?999:i;};
   const ctl=[...five,...controlItems({has,mode:state.mode}).filter(c=>!own.has(c.label)).map(c=>act(c.label,c.icon,()=>goTo(c),c.help,c.words))].sort((a,b)=>ix(a.label)-ix(b.label));
   const preheat=(has('heat')?preheatMenu(state.presets):[]).map(p=>act(p.fixed?'Cool down':'Preheat '+p.name,'i-heat',async()=>{if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});},p.fixed?'both heaters off':`${p.nozzle}° / ${p.bed}°`,p.fixed?'cool off heaters':'warm up nozzle bed'));
   const named=new Set(ctl.map(a=>a.label.toLowerCase()));
   const pg=(ui.pages||[]).filter(p=>p[2]!=='/'&&(!PAGE_NEEDS[p[2].slice(6)]||has(PAGE_NEEDS[p[2].slice(6)]))&&!named.has(p[1].toLowerCase())).map(p=>act(p[1],p[0],()=>p[2].startsWith('sheet:')?openNamedSheet(p[2].slice(6)):(location.href=p[2]),doesOf(p[1],p[3]),p[1]));
   const actions=[...ctl,...preheat,...pg,act('Add a printer','i-plus',()=>sheetAddPrinter(),'finds and adds a printer')];   // F06: found from any printer page too
   return {commands,history,actions,lacking:controlsLacking({has,mode:state.mode}).map(c=>({label:c.label,words:c.words}))};},
  run:async it=>{if(it.kind==='action'){await it.go();return true;}const base=await consoleMark();reviewAnchor=document.querySelector('.nx-cmd-box');const r=await send('console',{script:it.script}).finally(()=>{reviewAnchor=null;});if(r){jsend('/api/console/history','POST',{line:it.script}).catch(()=>{});consoleEcho(base);}return !!r;}});
 // top bar
 {const cb=button({icon:'i-camera',size:'icon',kind:'ghost',title:'Camera',does:'Fills the whole background with the live camera, and the glass turns light so the picture stays bright. Press again for the 3D machine.',onPress:async()=>{if(!camPinned&&!(await cameras().catch(()=>[])).length){pageCamera();return;}setCamPinned(!camPinned);}});cb.id='nx-camtoggle';cb.setAttribute('aria-pressed',String(camPinned));$('nx-state').before(cb);}
 // 2026-09-27: one phone icon on the main screen. "Pair phone" until a phone is paired, then a
 // phone with a check mark. Desktop only (a paired phone cannot pair others); opens the Phone sheet.
 // 2026-10-24: messages are counted on a bell, not shown over the machine.
 {const bb=button({icon:'i-bell',size:'icon',kind:'ghost',title:'Messages',does:'Every message since this page opened, newest first, and the alerts of the last seven days.',onPress:()=>openNotices(bb)});bb.id='nx-bell';bb.append(el('i','nx-bell-n'));$('nx-state').before(bb);onNotice(paintBell);}
 {const pb=button({icon:'i-phone',label:'Pair phone',size:'row',kind:'ghost',title:'Phone',does:'Pair a phone to watch and control this printer from anywhere, or see which phones are paired.',onPress:()=>sheetPhone()});pb.id='nx-phonebtn';pb.hidden=true;$('nx-state').before(pb);}
 const est=$('nx-estop');tip(est,'Emergency stop','Stops the printer at once. No review. Heaters and motors cut out; the printer then needs a firmware restart.');
 est.addEventListener('click',async()=>{if(est.disabled)return;est.disabled=true;await send('emergency_stop');lockControls();});
 import('./next-gesture.js').then(g=>{g.bindPress(est);g.bindPress($('nx-state'));});
 $('nx-state').addEventListener('click',()=>{const l=link();toast(l.reason||(l.klipper===false?`${plainState(l.label)}. ${caps.brand||'The printer'} is ready.`:`${plainState(l.label)}. Connected, and Klipper is ready.`),l.lock?'alarm':'ok');});
 applyCaps();
}
function paintPos(pos){const h=has('home')?state.readings.homed:{x:true,y:true,z:true};
 // Polish WO-07 (live pass): a PrusaLink printer reports X and Y whenever its head is still, so only a print in motion is the file followed (the server's position_source says the same).
 if(ui.posSrc){const f=(caps.driver==='demo'||(caps.capabilities?.live_xy===false&&state.readings?.job?.state==='printing'))&&has('xy');if(ui.posSrc.hidden===f)ui.posSrc.hidden=!f;}if(ui.retZ){const z=pos?.[2],t=h.z&&finite(z)!==null?'Z '+z.toFixed(2):'Z —';if(ui.retZ.textContent!==t)ui.retZ.textContent=t;}['X','Y','Z'].forEach((a,i)=>{const v=pos?.[i],w=ui.axes[a];const txt=h[a.toLowerCase()]&&finite(v)!==null&&(a==='Z'||has('xy'))?v.toFixed(1):'—';if(w.lastChild.textContent!==txt)w.lastChild.textContent=txt;w.toggleAttribute('data-homed',h[a.toLowerCase()]);});}

// ---------- render: mode, dock, dials, job, chip ----------
let renderPending=false,lastUi=0;
function renderSoon(){if(renderPending)return;const wait=Math.max(0,120-(performance.now()-lastUi));renderPending=true;setTimeout(()=>{renderPending=false;render();},wait);}
function render(){lastUi=performance.now();if(!ui.nozzle)return;const r=state.readings,l=link();
 const {mode,heating}=deriveMode({link:l,live:state.live,readings:r,homing:has('home')});state.mode=mode;document.body.dataset.mode=mode;
 // finish-2 F02: a printer that is off has no current values; a rewound view has the recorded ones and shows them (it showed dashes)
 const stale=['off','service','checking','klipper'].includes(mode);document.body.toggleAttribute('data-stale',stale||!state.live);const lastSeen=stale&&l.since?'last seen '+l.since+' ago':null;
 // state chip
 const chip=$('nx-state'),cs=controlsSummary(l,{live:state.live,enabled:caps.enabled});
 chip.dataset.state=!state.live&&l.state!=='replay'&&l.state!=='service'?'rewound':l.state;chip.dataset.mode=mode;chip.dataset.job=r.job.state||'';
 chip.toggleAttribute('data-heating',heating&&l.state==='ready'&&mode!=='printing');chip.querySelector('.nx-state-label').textContent=!state.live&&l.state!=='replay'?'REWOUND':l.state==='ready'?(mode==='printing'?'PRINTING':mode==='paused'?'PAUSED':mode==='finished'?({complete:'FINISHED',cancelled:'CANCELLED',error:'FAILED'}[r.job.state]||r.job.state.toUpperCase()):heating?'HEATING':mode==='unhomed'?'NOT HOMED':'READY'):plainState(l.label);
 if(document.body.dataset.layout==='phone'){const lab=chip.querySelector('.nx-state-label');lab.textContent={'KLIPPER NOT READY':'HALTED','PRINTER NOT READY':'NOT READY','PRINTER OFF':'OFF','STELLA SCOPE OFF':'NO LINK','NOZZLE OVER TEMP':'NOZZLE HOT','BED OVER TEMP':'BED HOT'}[lab.textContent]||lab.textContent;}
 chip.querySelector('.nx-state-since').textContent=sinceText(l,state.live);
 tip(chip,plainState(cs.badge),l.reason||cs.line||(l.klipper===false?`${caps.brand||'The printer'} is ready.`:'Connected, and Klipper is ready.'));
 // polish WO-06 (2026-10-18): tip() names the button once and never again, so a screen reader kept hearing the first badge ("READ ONLY") while the chip read PRINTING; the name follows what the chip says.
 chip.setAttribute('aria-label',`${chip.querySelector('.nx-state-label').textContent}. ${plainState(cs.badge)}`);
 // Over-temperature is a safety event, not a colour: the chip turns to alarm and says which heater,
 // and one toast fires per episode (heatColor 'danger': above the heater's limit or 15° over target).
 const hot=!document.body.hasAttribute('data-stale')&&['nozzle','bed'].find(k=>heatColor(r[k].temp,r[k].target,k).tone==='danger');
 chip.toggleAttribute('data-alarm',!!hot);if(hot){chip.querySelector('.nx-state-label').textContent=document.body.dataset.layout==='phone'?hot.toUpperCase()+' HOT':hot.toUpperCase()+' OVER TEMP';
  tip(chip,hot==='nozzle'?'Nozzle over temperature':'Bed over temperature',`${fmtTemp(r[hot].temp)} against a target of ${fmtTemp(r[hot].target)}. Check the heater; E-STOP cuts it at once.`);}
 // The over-temperature card itself comes from checkAlerts (anything wrong pops up).
 // dials: the kit's gauges (U02). value = what is set, cur = the heater's own reading (the dot on the ring). A printer that is off has no values, so they read a dash (2026-09-25:
 // speed and flow read 100 % with the printer off; Klipper resets both on a restart, so an old reading is not today's factor).
 ui.nozzle.set({value:stale?null:r.nozzle.target,cur:stale?null:r.nozzle.temp});
 ui.bed.set({value:stale?null:r.bed.target,cur:stale?null:r.bed.temp});
 document.dispatchEvent(new CustomEvent('stella-temps',{detail:stale?null:{nozzle:{temp:r.nozzle.temp,target:r.nozzle.target},bed:{temp:r.bed.temp,target:r.bed.target}}}));   // the chips on the machine (next-models.js)
 ui.speed.set({value:r.speedFactor===null||stale?null:r.speedFactor*100});
 ui.flow.set({value:r.extrudeFactor===null||stale?null:r.extrudeFactor*100});
 for(const [k,cmd]of [['nozzle','set_heater'],['bed','set_heater'],['speed','speed_factor'],['flow','extrude_factor']])ui[k].lock(can(cmd)?'':why(cmd));
 // 2026-09-24: the printer was off and the controller fan still read 100 %. A stale reading is
 // not a fan speed: with no live printer the fans read a dash.
 const fanOf=f=>stale||!f||f.speed===null||f.speed===undefined?{value:null}:{value:Math.round(f.speed*100)};
 ui.fans.part.set(fanOf(r.fans.part));ui.fans.hotend.set(fanOf(r.fans.hotend));ui.fans.controller.set(fanOf(r.fans.controller));ui.fans.part.lock(can('set_fan')?'':why('set_fan'));
 // folded into the bar the gauges still say the two numbers that matter (U02)
 if(ui.mini){const t=(k,x)=>stale||finite(x.temp)===null?'—':Math.round(x.temp)+'°';ui.mini.noz.textContent=t('nozzle',r.nozzle);ui.mini.bed.textContent=t('bed',r.bed);}
 if(stale&&state.sceneStale!==true&&scene?.live){scene.live({});state.sceneStale=true;}else if(!stale)state.sceneStale=false;
 if(!state.live)paintPos([r.position.x,r.position.y,r.position.z]);else if(!lastTick)paintPos(null);
 ui.vel.textContent=r.velocity===null||!has('live_xy')?'—':String(Math.round(r.velocity));
 {const P=ui.px,st=state.mode==='off';P.zo.textContent=st||r.zOffset===null?'—':(r.zOffset>=0?'+':'')+r.zOffset.toFixed(3);
  P.ac.textContent=st||r.maxAccel===null?'—':r.maxAccel>=1000?(r.maxAccel/1000).toFixed(r.maxAccel%1000?1:0)+'k':String(Math.round(r.maxAccel));
  P.mv.textContent=st||r.maxVelocity===null?'—':String(Math.round(r.maxVelocity));
  P.qlv.textContent=st||r.qgl===null?'—':r.qgl?'level':'not level';P.ql.dataset.state=st||r.qgl===null?'':r.qgl?'ok':'no';P.ql.hidden=!has('level');P.ac.parentElement.hidden=!has('klipper');P.mv.parentElement.hidden=!has('klipper');}
 // job capsule
 paintJob();
 // dock
 const lay=document.body.dataset.layout,compact=['phone','phone-land','kiosk'].includes(lay),max=lay==='kiosk'?6:lay==='phone-land'?4:compact?5:innerWidth>=1500?12:innerWidth>=1200?10:8;
 const jobName=r.job.name,supports=supportsFrom(caps),key=[mode,compact,max,jobName,l.state,caps.driver||'',(caps.commands||[]).filter(c=>c.supported===false).map(c=>c.id).join(',')].join('|');
 if(key!==state.dockKey){state.dockKey=key;const SHORT={fw_restart:'FW RESTART',klipper_restart:'RESTART',reprint:'AGAIN'};
  const slots=dockFor(mode,{compact,maxSlots:max,jobName,supports}).map(s=>({...s,label:compact&&SHORT[s.key]||s.label,attention:s.primary&&mode==='unhomed',...DOCK_TEXT[s.key]}));
  state.dock=slots;const msg=mode==='service'?{icon:'i-warn',text:'Stella Scope is not answering. Trying again by itself.'}:mode==='checking'?{icon:'i-live',text:'Connecting to the printer…'}:null;
  renderBar(slots,{press:dockPress,message:msg});}
 lockControls();for(const b of document.querySelectorAll('#nx-rail [data-needs]'))b.hidden=!has(b.dataset.needs);
 if(scene)scene.stage?.(mode==='printing'?'printing':heating?'heating':l.state==='not_ready'?'error':'idle');
 paintLane();paintReticle();paintPages();paintActivity();toolpathAuto();toolpathTick();paintProbeMarks();meshOverlayTick();paintProbeLed();checkAlerts();paintCamPin();paintMaintenance();paintEye();filamentTick();}
// Failure watch (stella-eye, Obico's model on the camera, 2026-09-25): a chip in the top bar
// while a print is watched. Green OK, amber LOOK, red FAILING; tap for the camera.
let eyeAt=0,eyeData=null;function paintEye(){if(performance.now()-eyeAt>8000){eyeAt=performance.now();fetch('/api/eye').then(r=>r.ok?r.json():null).then(d=>{eyeData=d;paintEye();}).catch(()=>{});}
 let chip=$('nx-eye');const d=eyeData,show=!!(d&&d.enabled&&d.watching&&state.mode==='printing');
 if(!chip){if(!show)return;chip=button({icon:'i-view',label:'WATCH',size:'icon',kind:'ghost',title:'Failure watch',does:'Looks at the camera every 10 seconds for spaghetti and failed prints. It warns you here and on WhatsApp and never stops a print by itself. Tap for the camera.',onPress:()=>pageCamera()});chip.id='nx-eye';chip.append(el('span','nx-lbl',''));$('nx-state').before(chip);}
 chip.hidden=!show;if(!show)return;const v=d.verdict||'ok',pct=Math.round((d.normalized||0)*100);chip.dataset.v=d.learning?'learning':v;
 const lbl=chip.querySelector('.nx-lbl');if(lbl)lbl.textContent=d.learning?'WATCH · LEARNING':v==='failing'?`FAILING ${pct}%`:v==='warning'?`LOOK ${pct}%`:`OK ${pct}%`;}
// Pages that exist only for a printer that can do the named thing (farm board WO-02): they are not
// offered on a printer that cannot, rather than opened onto emptiness.
// polish WO-05 (2026-10-18): the More menu offers only what this printer can do; a Klipper restart or a macro is not shown, greyed out, on a Prusa or a FlashForge.
const MORE_NEEDS={console:'console',macro:'macros',motors_off:'motors_off',firmware_restart:'restart',klipper_restart:'restart'};
const PAGE_NEEDS={level:'level',shaper:'klipper',filament:'filament',motion:'klipper',signals:'klipper',parts:'klipper',electronics:'klipper',causes:'klipper',journal:'klipper',assembly:'klipper'};
const DOCK_TEXT={job:{title:'The print',does:'The job on the plate: the layer, time left, the part drawn as it grows, skip an object.'},
 camera:{title:'Camera',does:'The printer camera, live, when it answers.'},history:{title:'Print history',does:'Every print it recorded, how it went, and the totals.'},alerts:{title:'Alerts',does:'The last seven days: the printer going offline, driver warnings, errors.'},printers:{title:'This printer',does:'Its address, name, colour and model. Change where Stella Scope looks for it.'},
 home:{title:'Home',does:'Homes X, Y and Z: the head finds its end stops.'},level:{title:'Level the gantry',does:'Probes the four corners and levels the gantry (quad gantry level).'},
 mesh:{title:'Bed mesh',does:'Probes the bed and makes a fresh bed mesh.'},move:{title:'Move',does:'Opens the jog pad: X, Y and Z by 0.1 to 50 mm.'},
 heat:{title:'Heat',does:'Nozzle and bed targets, your saved presets and the preset editor.'},files:{title:'Things to print',does:'The files on the printer, with pictures. Pick one and press Print, or add it to the queue.'},
 macros:{title:'Macros',does:'Your Klipper macros. Each one asks you to confirm.'},more:{title:'More',does:'Everything else: restarts, motors off, console, recording, view and pages.'},
 pause:{title:'Pause',does:'Pauses the print.'},resume:{title:'Resume',does:'Resumes the paused print.'},cancel:{title:'Cancel print',does:'Cancels the running print.'},
 speed:{title:'Speed',does:'Speeds up or slows down every move. 100% is as sliced.'},flow:{title:'Flow',does:'Changes how much plastic every move pushes out. 100% is as sliced.'},fan:{title:'Part fan',does:'Part cooling fan speed.'},
 load:{title:'Load filament',does:'Heats to the chosen material\'s nozzle temperature, then feeds by itself. Push the filament in while it heats.'},unload:{title:'Unload filament',does:'Heats to the loaded material\'s nozzle temperature and pulls the filament out.'},
 exclude:{title:'Exclude objects',does:'Skip one object of this print.'},reprint:{title:'Print again',does:'Starts the same file again, after you confirm.'},
 fw_restart:{title:'Firmware restart',does:'Restarts Klipper and the printer\'s boards. The way out of a halted Klipper. Asks you to confirm.'},
 klipper_restart:{title:'Klipper restart',does:'Restarts Klipper\'s software on the printer\'s computer. Asks you to confirm.'},console:{title:'Console',does:'The printer\'s console and a line to type G-code.'},
 go_live:{title:'Go live',does:'Returns to the live printer.'},rec_back:{title:'Back',does:'One step back in the recording.'},rec_fwd:{title:'Forward',does:'One step forward in the recording.'},
 rec_play:{title:'Play',does:'Plays the recording from here.'},recording:{title:'Recording',does:'Move through time, play it back, and copy a link to this moment.'},look:{title:'Look',does:'The colour, the finish and the dial style of this screen. Changes at once.'},calibrate:{title:'Calibrate',does:'Set up this printer in order: heaters, Z offset, gantry, mesh, input shaper, extruder, pressure advance and flow.'}};
// Idle strip (2026-09-25): with no job the top bar reads the machine at a glance — temperatures,
// how long since the last print ended, the filament recorded as loaded and the sheet on the bed.
let lastEnd=null,lastEndAt=0;
function paintIdle(jobShown){const host=$('nx-idle'),r=state.readings,show=!jobShown&&['idle','unhomed','off','finished'].includes(state.mode);host.hidden=!show;if(!show)return;
 if(Date.now()-lastEndAt>300e3){lastEndAt=Date.now();fetch('/api/print/history?limit=5').then(x=>x.ok?x.json():null).then(d=>{const js=d?.jobs||[],j=js.find(q=>finite(q.end_time)!==null)||js[0];lastEnd=finite(j?.end_time)??finite(j?.start_time);}).catch(()=>{});}
 if(!ui.idle){ui.idle={};for(const [k,ico,tt]of [['noz','i-heat','Nozzle now'],['bed','i-bed','Bed now'],['since','i-rewind','Since the last print ended'],['fil','i-filament-load','Filament recorded as loaded'],['sheet','i-level','Build plate on the bed']]){
   // 2026-09-28: "the filament button up top says not set". The two that hold a record open what
   // they record: filament -> the Filament page, sheet -> the build plate's panel.
   // 2026-10-25: "I just loaded filament, it reads no filament set ... we have those stock things". A Prusa never says what is loaded, so the pill is a
   // one-tap pick from the stock lists (nothing moves, it is a record); the full pages are the last choice of each wheel.
   const open=k==='fil'?b=>pickRecord(b,'fil'):k==='sheet'?b=>pickRecord(b,'sheet'):null;
   const it=el(open?'button':'span','nx-idle-item');if(open){it.type='button';it.addEventListener('click',()=>open(it));}it.dataset.k=k;it.title=tt+(open?'. Tap to open.':'');const v=el('b');it.append(icon(ico),v);host.append(it);ui.idle[k]=v;}}
 const off=state.mode==='off',t=v=>off||finite(v)===null?'—':Math.round(v)+'°',pr=state.presets||{};
 ui.idle.noz.textContent=t(r.nozzle.temp);ui.idle.bed.textContent=t(r.bed.temp);
 ui.idle.since.textContent=lastEnd?'Idle '+fmtDuration(Date.now()/1000-lastEnd):'No prints yet';
 ui.idle.fil.textContent=pr.loaded?.name||'Filament?';ui.idle.sheet.textContent=pr.sheet?.name||'Plate?';}
// A material just recorded as loaded brings its profile with it: its pressure advance goes to a Klipper printer (2026-10-26, material profiles).
// Flow stays a slicer setting (the printer's own flow factor would multiply it a second time), so it is only named.
async function applyMaterial(name){const m=(state.presets?.materials||[]).find(x=>x.name===name);if(!m)return;
 if(finite(m.pa)!==null&&has('klipper')&&can('pressure_advance')){if(await send('pressure_advance',{value:m.pa}))toast(`${name}: pressure advance ${m.pa} set${finite(m.flow)!==null?` · flow ${m.flow} is for your slicer`:''}.`,'ok');}}
// What is in the printer and on the bed, recorded in one tap from the stock lists (paintIdle's pills).
function pickRecord(b,k){const pr=state.presets||{},fil=k==='fil',list=fil?(pr.materials||[]):(pr.sheets||[]),cur=fil?pr.loaded?.name:pr.sheet?.name,r=b.getBoundingClientRect();
 const items=list.map(m=>({key:'rec-'+m.name,icon:fil?'i-spool':'i-level',label:m.name.toUpperCase(),sub:fil?m.nozzle+'°':(m.name===cur?'ON':''),name:m.name,title:m.name+(m.name===cur?(fil?' (recorded now)':' (on the bed now)'):''),
  does:fil?`Records ${m.name} as the filament in the printer. Nothing heats or moves.`:`Records ${m.name} as the plate on the bed, with its Z offset.`}));
 items.push(fil?{key:'rec-page',icon:'i-filament-load',label:'LOAD OR UNLOAD',title:'Filament page',does:'Heat and load or unload, step by step.'}:{key:'rec-page',icon:'i-panels',label:'ALL PLATES',title:'Build plates',does:'Every plate, its Z offset, add or remove one.'});
 openRadial(items,{x:r.left+r.width/2,y:r.bottom+200,from:{x:r.left+r.width/2,y:r.top+r.height/2},title:fil?'Filament in the printer':'Plate on the bed',center:()=>cur?(fil?'Now: ':'On the bed: ')+cur:'',onPick:async it=>{
  if(it.key==='rec-page'){fil?openNamedSheet('filament'):pagePlates();return;}
  const at=Date.now()/1000,body=fil?{...pr,loaded:{name:it.name,at}}:{...pr,sheet:{name:it.name,at},sheet_log:[...(pr.sheet_log||[]),{name:it.name,at}].slice(-200)};
  try{await jsend('/api/presets','PUT',body);await loadPresets();render();toast(fil?`${it.name} recorded as the filament in the printer.`:`${it.name} recorded as the plate on the bed.`,'ok');if(fil)applyMaterial(it.name);}catch(e){toast(e.message,'alarm');}}});}
function paintJob(){const j=jobModel(state.readings),host=$('nx-job');const show=!['off','klipper','checking'].includes(state.mode)&&(j.active||(state.mode==='finished'&&j.title));host.hidden=!show;paintIdle(show);if(!show)return;
 // 2026-10-23: the job is one wide block, not a pill with a ring and empty space after it: the picture, the name, one line of
 // what matters (the layer, the time left, the clock time it will be done), the percentage large, and the progress as a lit bar along its foot.
 if(!host.firstChild){host.className='nx-job nx-glass';const img=el('img','nx-job-thumb');img.alt='';img.addEventListener('error',()=>{img.style.visibility='hidden';});
  const text=el('div','nx-job-text');ui.jobTitle=el('div','nx-job-title');ui.jobMeta=el('div','nx-job-meta');text.append(ui.jobTitle,ui.jobMeta);
  const bar=el('div','nx-job-bar');ui.jobBar=el('i');bar.append(ui.jobBar);
  ui.jobPct=el('div','nx-job-pct');host.append(img,text,ui.jobPct,bar);ui.jobImg=img;
  tip(host,'Current print','File, progress and time left. Tap for the print page: the layer, and skip an object.');host.removeAttribute('aria-label');host.setAttribute('role','button');host.tabIndex=0;host.addEventListener('click',()=>pagePrint());}
 if(ui.jobFile!==state.readings.job.name){ui.jobFile=state.readings.job.name;ui.jobImg.style.visibility=has('thumbnails')?'':'hidden';if(has('thumbnails'))ui.jobImg.src='/api/print/thumbnail?filename='+encodeURIComponent(ui.jobFile);}
 ui.jobTitle.textContent=j.title;host.dataset.state=state.mode;
 if(state.mode==='finished'){const w={complete:'Finished',cancelled:'Cancelled',error:'Stopped by an error'}[state.readings.job.state]||'Ended';ui.jobMeta.textContent=w;}
 else if(j.warming){const r=state.readings,h=(n,x)=>finite(x.target)>0?`${n} ${Math.round(x.temp)}° of ${Math.round(x.target)}°`:`${n} off`;
  ui.jobMeta.textContent=['Warming up',h('bed',r.bed),h('nozzle',r.nozzle)].join('  ·  ');ui.jobPct.textContent='';ui.jobBar.style.width='0%';return;}
 else{const parts=[],lay=jobLayer();if(lay)parts.push(lay);
  if(state.mode==='paused')parts.push('paused');
  if(j.remaining===null)parts.push(j.elapsed?fmtDuration(j.elapsed)+' in':'starting');else if(j.remaining<30&&(j.progress??0)<1)parts.push('almost done');
  else{parts.push(fmtDuration(j.remaining)+' left');if(state.mode==='printing'&&document.body.dataset.layout!=='phone')parts.push('done '+new Date(Date.now()+j.remaining*1000).toLocaleTimeString([],{hour:'numeric',minute:'2-digit'}));}
  ui.jobMeta.textContent=parts.join('  ·  ');}
 const pc=j.progress===null?null:Math.round(j.progress*100);ui.jobPct.textContent=pc===null?'':pc+'%';ui.jobBar.style.width=(pc??0)+'%';}
// The layer the head is on, from the toolpath when it is drawn, else from the printer's own count; nothing when neither knows.
function jobLayer(){const n=tp?.parsed?(tp.parsed.totalLayers||tp.parsed.layers?.length):null;if(n&&Number.isFinite(tp.layer))return `layer ${tp.layer+1} of ${n}`;
 const i=(lastTick?.point??state.snapshot?.point)?.print_stats?.info;if(Number.isFinite(i?.current_layer)&&Number.isFinite(i?.total_layer)&&i.total_layer>0)return `layer ${i.current_layer} of ${i.total_layer}`;return '';}

// ---------- dock and radial actions ----------
async function dockPress(slot,btn){const a=slot.action||'';
 // 2026-09-24: pressing the button that opened a sheet, pop-up or radial again closes it.
 if(/^(sheet|menu|radial):/.test(a)&&btn?.hasAttribute('data-showing')){closeSheet();closePopover();$('nx-radial').hidden=true;$('nx-radial').replaceChildren();clearOpener();return;}
 if(slot.run){busy(btn,true);try{await slot.run();}finally{busy(btn,false);}return;}
 // 2026-10-24: "I see heat. I don't see cool down." One press, both heaters off.
 if(a==='cool'){busy(btn,true);try{if(await execute('set_heater',{target:'extruder',value:0},true,'Nozzle heater off'))await execute('set_heater',{target:'heater_bed',value:0},true,'Bed heater off');}finally{busy(btn,false);}return;}
 // 2026-10-24: a stop for the printer itself, not the emergency stop. Held to confirm; ends a filament job, then both heaters and the part fan off. Klipper keeps running.
 if(a==='stop'){busy(btn,true);try{await filamentStop();await execute('set_heater',{target:'extruder',value:0},true,'Nozzle heater off');await execute('set_heater',{target:'heater_bed',value:0},true,'Bed heater off');if(has('fan'))await execute('set_fan',{value:0},true,'Part fan off');toast('Stopped: heaters and part fan off. Klipper is still running.','ok');}finally{busy(btn,false);}return;}
 // 2026-09-25: "a million buttons, and only one radial". Every dock button with a handful of
 // choices opens them as a radial round the button: Home, Level, and the quick presets of Heat,
 // Speed, Flow and Fan (each with its full sheet as the last spoke). Files, macros, the console and
 // the jog pad keep their sheets: long lists and pads do not fit on a ring.
 const quick=(await lazyWheel(slot.key))||quickRadial(slot.key);if(quick||a.startsWith('menu:')){const items=(quick||SUB_MENUS[a.slice(5)]||[]).filter(it=>!it.cmd||supportsFrom(caps)(it.cmd));const r=btn.getBoundingClientRect();closePopover();closeSheet();
  openRadial(items,{x:r.left+r.width/2,y:r.top-190,from:{x:r.left+r.width/2,y:r.top+r.height/2},title:slot.title||DOCK_TEXT[slot.key]?.title||slot.label,center:()=>wheelCenter(slot.key),onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls($('nx-radial'));return;}
 // A completed hold IS the confirmation (2026-10-22): it goes to the printer with confirm:true, without the review dialog that would ask the same question again.
 if(a.startsWith('send:')&&slot.hold){busy(btn,true);try{const id=a.slice(5),r=await execute(id,{},true,slot.title||slot.label);if(r&&id==='cancel')watchCancel();}finally{busy(btn,false);}return;}
 if(a.startsWith('send:')){busy(btn,true);try{await send(a.slice(5));}finally{busy(btn,false);}return;}
 // 2026-09-28: Load and Unload on the bottom bar, one press each (next-pages filamentQuick).
 if(a.startsWith('filament:')){busy(btn,true);try{await filamentQuick(a.slice(9));}finally{busy(btn,false);}return;}
 if(a==='reprint'){busy(btn,true);try{await send('print_start',{filename:state.readings.job.name});}finally{busy(btn,false);}return;}
 if(a.startsWith('sheet:')){closePopover();await openNamedSheet(a.slice(6));if(!$('nx-sheet').hidden)markOpener(btn);return;}
 if(a.startsWith('menu:')){const menu=(SUB_MENUS[a.slice(5)]||[]).filter(it=>!it.cmd||supportsFrom(caps)(it.cmd));const pop=openPopover(btn,menu,{title:slot.title||slot.label,onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls(pop);return;}
 if(a.startsWith('script:')){busy(btn,true);try{await send('console',{script:a.slice(7)});}finally{busy(btn,false);}return;}
 if(a.startsWith('rec:'))return recAction(a.slice(4));
 if(a==='radial:more'){const r=btn.getBoundingClientRect();const items=moreMenu(state.mode,state.dock).filter((it,i,a)=>a.findIndex(x=>x.key===it.key)===i).filter(it=>!MORE_NEEDS[it.cmd]||has(MORE_NEEDS[it.cmd])).map(i=>({...i,...DOCK_TEXT[i.key],title:DOCK_TEXT[i.key]?.title||i.label[0]+i.label.slice(1).toLowerCase()}));
  closeSheet();const rad=openRadial(items,{x:r.left+r.width/2,y:r.top-190,from:{x:r.left+r.width/2,y:r.top+r.height/2},title:'More',center:()=>wheelCenter('more'),onPick:it=>dockPress(it,btn)});markOpener(btn);lockControls($('nx-radial'));return rad;}}
let noticesSeen=0;
function paintBell(){const b=$('nx-bell');if(!b)return;const n=notices.filter(x=>x.at>noticesSeen).length;b.querySelector('.nx-bell-n').textContent=n?String(Math.min(n,99)):'';b.toggleAttribute('data-new',n>0);}
function openNotices(anchor){noticesSeen=Date.now();paintBell();const ago=t=>{const s=Math.round((Date.now()-t)/1000);return s<60?'now':s<3600?Math.floor(s/60)+' min ago':new Date(t).toLocaleTimeString([], {hour:'numeric',minute:'2-digit'});};
 const items=[...notices].reverse().slice(0,12).map(x=>({icon:x.tone==='alarm'?'i-warn':x.tone==='ok'?'i-check':'i-live',label:`${x.text}  ·  ${ago(x.at)}`,does:x.text}));
 if(!items.length)items.push({icon:'i-check',label:'No messages since this page opened',does:''});
 items.push({icon:'i-warn',label:'Alerts of the last seven days',does:'Printer offline, motor driver warnings, Klipper errors.',go:'alerts'});
 openPopover(anchor,items,{title:'Messages',onPick:it=>{if(it.go)openNamedSheet(it.go);}});markOpener(anchor);}
// The lane's own line (the filament step, when one runs, lies over it: next-pages paintFilMarker).
// a printer that reports speed and flow but cannot be told them (PrusaLink) shows them once the first reading is in
function showReported(){const r=state.readings;for(const [g,k,v] of [[ui.speed,'speed',r.speedFactor],[ui.flow,'flow',r.extrudeFactor]])if(g&&!has(k)){const off=finite(v)===null;if((g.el.style.display==='none')!==off)g.el.style.display=off?'none':'';}}
function paintLane(){showReported();if(sheetOnce&&state.mode!=='checking'){const n=sheetOnce;sheetOnce=null;setTimeout(()=>openNamedSheet(n),500);}if(wheelOnce){const b=document.querySelector(`#nx-dock .nxkey[data-key="${CSS.escape(wheelOnce)}"]`);if(b){wheelOnce=null;setTimeout(()=>b.click(),400);}}
 if(!ui.lane)return;const r=state.readings,l=link(),mode=state.mode,{el:lane,ico,t,d,bar}=ui.lane,j=jobModel(r);
 const hot=k=>finite(r[k].target)>0,far=k=>hot(k)&&finite(r[k].temp)!==null&&Math.abs(r[k].temp-r[k].target)>3,deg=v=>finite(v)===null?'—':Math.round(v)+'°';
 let tone='',icn='i-live',a='',b='',p=null;
 if(['off','service','klipper','checking'].includes(mode)||l.state!=='ready'){tone=mode==='checking'?'':'warn';icn=mode==='checking'?'i-live':'i-warn';a=mode==='checking'?'Connecting':(l.label?l.label.charAt(0)+l.label.slice(1).toLowerCase():'Not ready');b=l.reason||'';}
 else if(mode==='printing'||mode==='paused'){icn=mode==='paused'?'i-pause':'i-print';tone=mode==='paused'?'warn':'ok';a=mode==='paused'?'Paused':j.warming?'Warming up':'Printing';
  // the job card above already says the layer and the time left: the lane says what the machine is doing this moment (2026-10-25, nothing twice)
  const z=finite(r.position?.z),v=finite(r.velocity),fl=finite(volumetricFlow(r.extruderVelocity));
  b=j.warming?['nozzle','bed'].filter(hot).map(k=>`${k==='nozzle'?'Nozzle':'Bed'} ${deg(r[k].temp)} → ${deg(r[k].target)}`).join('  ·  '):[z!==null?`Z ${z.toFixed(2)} mm`:'',v!==null&&has('live_xy')?`${Math.round(v)} mm/s`:'',fl!==null&&fl>0?`${fl.toFixed(1)} mm³/s`:''].filter(Boolean).join('  ·  ')||ui.jobMeta?.textContent||'';p=j.warming?null:j.progress;}
 else if(far('nozzle')||far('bed')){icn='i-heat';tone='heat';a='Heating';const parts=['nozzle','bed'].filter(hot).map(k=>`${k==='nozzle'?'Nozzle':'Bed'} ${deg(r[k].temp)} → ${deg(r[k].target)}`);b=parts.join('  ·  ');
  p=Math.min(...['nozzle','bed'].filter(far).map(k=>Math.max(0,Math.min(1,(r[k].temp-20)/Math.max(1,r[k].target-20)))));}
 else{icn='i-check';tone='ok';a=mode==='finished'?({complete:'Finished',cancelled:'Cancelled',error:'Stopped by an error'}[r.job.state]||'Finished'):mode==='unhomed'?'Ready, not homed':'Ready';
  const held=['nozzle','bed'].filter(hot).map(k=>`${k==='nozzle'?'nozzle':'bed'} held at ${deg(r[k].target)}`);b=[held.length?held.join(', '):'heaters off',state.presets?.loaded?.name?state.presets.loaded.name+' loaded':''].filter(Boolean).join('  ·  ');b=b[0].toUpperCase()+b.slice(1);}
 lane.dataset.tone=tone;if(ico.dataset.i!==icn){ico.dataset.i=icn;ico.replaceChildren(icon(icn));}if(t.textContent!==a)t.textContent=a;if(d.textContent!==b)d.textContent=b;bar.style.width=p===null?'0%':(Math.round(p*1000)/10)+'%';lane.toggleAttribute('data-bar',p!==null);lane.toggleAttribute('data-fil',filamentBusy());}
// A part of a machine model was clicked (finish E17): its plain name, the assembly it is in, and the page that shows it live when there is one.
// The CAD's names read 'Pv24_00004__NEMA17_Black_1'; a part named only by a number ('p00001', Prusa's own CAD) says nothing, so nothing opens.
const partWords=n=>{const t=String(n||'').replace(/^P[a-z0-9]*_\d+__/i,'').replace(/^p\d+(__)?/i,'').replace(/__.*$/,'').replace(/_/g,' ').replace(/\s+\d+$/,'').trim();return t&&!/^\d+$/.test(t)?t:'';};
const PART_PAGES=[[/bed|build ?plate|mic6|heater bed/i,'thermal','Heat & fans'],[/hotend|nozzle|heat ?break|heater block|extruder|cht|volcano|dragon|revo/i,'thermal','Heat & fans'],[/fan|blower|4010|5015|axial/i,'thermal','Heat & fans'],[/nema|stepper|motor/i,'motion','Motion & motors'],[/belt|gt2|pulley|idler/i,'shaper','Input shaper and belts'],[/probe|bltouch|klicky|tap|beacon/i,'level','Levelling'],[/octopus|mcu|pi\b|raspberry|psu|power supply|ebb|can/i,'electronics','Electronics']];
document.addEventListener('stella-part-pick',e=>{const p=e.detail||{},name=partWords(p.name);if(!name)return;const grp=String(p.group||'').replace(/^V\d+_/,'').replace(/_/g,' ');
 const page=PART_PAGES.find(([re])=>re.test(name+' '+grp)),a=el('div','nx-pick-anchor');a.style.cssText=`position:fixed;left:${p.x}px;top:${p.y}px;width:1px;height:1px`;document.body.append(a);
 const items=[{icon:'i-nav-parts',label:grp?`${name}  ·  ${grp}`:name,does:'This part of the machine, by the name its maker gave it.'}];if(page)items.push({icon:'i-panels',label:'Open '+page[2],go:page[1],does:'The live figures for this part.'});
 openPopover(a,items,{title:'Part',onPick:it=>{a.remove();if(it.go)openNamedSheet(it.go);}});setTimeout(()=>a.remove(),15000);});
// Wheels whose choices are fetched (2026-10-24, every key a radial): PRINT shows the newest files, LOAD and UNLOAD the materials, MOVE the
// homing and the jog pad, MACROS the printer's own macros. Each ends with its full page. Null for a key whose choices are known already.
async function lazyWheel(key){
 if(key==='load'||key==='unload')return filamentWheel(key);
 if(key==='move')return [...SUB_MENUS.home,{key:'jog',icon:'i-move',label:'JOG PAD',cmd:'jog',action:'sheet:move',title:'Jog pad',does:'X, Y and Z by 0.1 to 50 mm, with the head drawn where it is.'}];
 if(key==='files'){let files=[];try{const r=await fetch('/api/files');const d=await r.json();files=(d.files||d.result||[]).slice().sort((a,b)=>(b.modified||0)-(a.modified||0)).slice(0,6);}catch{}
  return [...files.map(f=>{const p=f.path||f.filename||f.name||'';return {key:'file-'+p,icon:'i-print',label:fileTitle(p).slice(0,40),sub:f.modified?new Date(f.modified*1000).toLocaleDateString([], {month:'short',day:'numeric'}):'',cmd:'print_start',title:'Print '+fileTitle(p),does:'Starts this file. You see the command first.',run:()=>send('print_start',{filename:p})};}),
   {key:'all-files',icon:'i-files',label:'ALL FILES',title:'All files',does:'Every file on the printer, with pictures, a search and the queue.',action:'sheet:files'}];}
 if(key==='macros'){let mac=[];try{const r=await fetch('/api/macros');const d=await r.json();mac=(d.macros||[]).filter(m=>m.macro===true&&!m.name.startsWith('_'));}catch{}
  return [...mac.slice(0,20).map(m=>({key:'mac-'+m.name,icon:'i-macro',label:m.name.replace(/_/g,' ').slice(0,22),cmd:'macro',title:m.name,does:m.help||'Runs this macro. You see the command first.',run:()=>(Array.isArray(m.params)&&m.params.length)?openNamedSheet('macros'):send('macro',{target:m.name})})),
   {key:'all-macros',icon:'i-panels',label:'ALL MACROS',title:'All macros',does:'Every macro, with a search and their settings.',action:'sheet:macros'}];}
 return null;}
// What the hub of a wheel shows when nothing is lit: the key's own figures (they left the bottom bar for here, 2026-10-24).
function wheelCenter(key){const r=state.readings,p=r.position,t=v=>finite(v)===null?'off':Math.round(v)+'°',n=v=>finite(v)===null?'not reported':v.toFixed(1);
 const heat=`Nozzle ${t(r.nozzle.temp)}${r.nozzle.target>0?' → '+t(r.nozzle.target):''}\nBed ${t(r.bed.temp)}${r.bed.target>0?' → '+t(r.bed.target):''}`;
 switch(key){case 'move':case 'home':{const fh=finite(p.x)===null&&window.__stellaHead&&performance.now()-window.__stellaHead.at<5000?window.__stellaHead:null;
   if(fh){const q=fh.position;return `X ${n(q[0])}   Y ${n(q[1])}\nZ ${n(finite(p.z)??q[2])}  ·  from the print file`;}
   return `X ${n(p.x)}   Y ${n(p.y)}\nZ ${n(p.z)}  ·  ${r.homed.x&&r.homed.y&&r.homed.z?'homed':'not homed'}`;}
  case 'level':return `Gantry ${r.qgl===null?'not reported':r.qgl?'level':'not level'}\nZ offset ${finite(r.zOffset)===null?'not reported':(r.zOffset>=0?'+':'')+r.zOffset.toFixed(3)} mm`;
  // 2026-10-25 audit: the fan wheel showed speed, flow and accel; it shows the fans
  case 'fan':{const fp=v=>finite(v)===null?'not reported':Math.round(v*100)+'%';return `Part fan ${fp(r.fans.part.speed)}\nHotend fan ${fp(r.fans.hotend.speed)}`;}
  case 'speed':case 'flow':return `Speed ${finite(r.speedFactor)===null?'not reported':Math.round(r.speedFactor*100)+'%'}  ·  Flow ${finite(r.extrudeFactor)===null?'not reported':Math.round(r.extrudeFactor*100)+'%'}\nAccel ${finite(r.maxAccel)===null?'not reported':Math.round(r.maxAccel)}  ·  top ${finite(r.maxVelocity)===null?'not reported':Math.round(r.maxVelocity)+' mm/s'}`;
  case 'heat':case 'cool':case 'load':case 'unload':return heat;
  case 'files':return state.presets?.loaded?.name?`Loaded: ${state.presets.loaded.name}`:'';
  default:return '';}}
// The HUD says a print is cancelled only when the printer says so (next-core cancelVerdict): print_stats.state reads cancelled and both heater targets read 0. A cancel that does
// not show up in the printer's state says so in plain words after 20 s, and a cancel that left a heater on says that after 8.
let cancelWatch=0;
function watchCancel(){clearInterval(cancelWatch);const t0=performance.now();
 cancelWatch=setInterval(()=>{const v=cancelVerdict(state.readings,(performance.now()-t0)/1000);if(v.state==='wait')return;clearInterval(cancelWatch);toast(v.words,v.state==='ok'?'ok':'alarm');},400);}
function quickRadial(key){const both=async p=>{if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});};
 if(key==='heat')return [...preheatMenu(state.presets).map(p=>({key:'pre-'+p.name,icon:p.fixed?'i-close':'i-heat',label:p.fixed?'COOL':p.name.toUpperCase(),sub:p.fixed?'all off':`${p.nozzle}° · ${p.bed}°`,tone:p.fixed?'cool':'heat',cmd:'set_heater',title:p.fixed?'Cool down':'Preheat '+p.name,does:p.fixed?'Both heaters off.':`Nozzle ${p.nozzle}°, bed ${p.bed}°.`,run:()=>both(p)})),
  {key:'heat-edit',icon:'i-plus',label:'EDIT LIST',title:'Edit the material list',does:'Add, change or take away materials, or add them back from the standard list. Nothing is sent to the printer.',run:()=>{closePopover();$('nx-radial').hidden=true;sheetPresetEditor();}},
  {key:'heat-sheet',icon:'i-panels',label:'HEAT PANEL',title:'All heat settings',does:'Targets, presets and the preset editor.',action:'sheet:heat'}];
 const f=(k,id,vals,unit,sheet,scale=1)=>[...vals.map(v=>({key:k+v,icon:k==='fan'?'i-fan':k==='speed'?'i-speed':'i-flow',label:v+unit,num:true,cmd:id,title:`${k[0].toUpperCase()+k.slice(1)} ${v}${unit}`,does:`Sets the ${k} to ${v}${unit}.`,run:()=>send(id,{value:v*scale})})),
  {key:k+'-sheet',icon:'i-panels',label:'EXACT VALUE',title:'Exact value',does:'The full '+k+' sheet.',action:'sheet:'+sheet}];
 if(key==='speed')return f('speed','speed_factor',[50,75,100,125,150],'%','speed');
 if(key==='flow')return f('flow','extrude_factor',[90,95,100,105,110],'%','flow');
 if(key==='fan')return f('fan','set_fan',[0,25,50,75,100],'%','fan',1/100);
 return null;}
function openNamedSheet(name){return ({heat:()=>sheetHeat('nozzle'),move:sheetMove,files:sheetFiles,macros:sheetMacros,console:sheetConsole,speed:()=>sheetFactor('speed'),flow:()=>sheetFactor('flow'),
 fan:sheetFan,exclude:sheetExclude,recording:sheetRecording,view:sheetView,look:sheetLook,calibrate:pageCalibrate,materials:sheetPresetEditor,timelapse:sheetTimelapses,health:pageHealth,feedback:sheetFeedback,pages:sheetPages,level:pageLevel,thermal:pageThermal,signals:pageSignals,timeline:pageTimeline,parts:()=>pageParts(),electronics:()=>pageParts('electronics'),phone:sheetPhone,plates:pagePlates,printers:sheetPrinters,queue:sheetQueue,print:pagePrint,filament:pageFilament,camera:pageCamera,shaper:pageShaper,history:pageHistory,maintenance:pageMaintenance,alerts:pageAlerts,causes:pageCauses,journal:()=>pageJournal(),assembly:pageAssembly,motion:pageMotion}[name]||(()=>toast('Not available yet.')))();}
function recAction(v){const at=state.snapshot?.point?.ts??state.cursor;
 if(v==='live'){state.live=true;state.playing=false;request(null);return;}
 if(v==='back'||v==='fwd'){state.live=false;state.playing=false;request(at,v==='back'?'previous':'next');return;}
 if(v==='play'){state.live=false;state.playing=!state.playing;if(state.playing&&finite(state.cursor)===null)state.cursor=state.snapshot?.first_ts;render();}}

// ---------- sheets ----------
async function loadPresets(){try{const r=await fetch('/api/presets');const d=await r.json();if(r.ok)state.presets=d;}catch{}return state.presets;}
async function jsend(url,method,body){const c=await controls();const r=await fetch(url,{method,headers:{'Content-Type':'application/json',...(c.token?{'X-Stella-Control-Token':c.token}:{})},body:JSON.stringify(body)});
 const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);return d;}
// 2026-10-25: "either include PLA or what it is ... that should just be a couple buttons. A lot smaller, simpler, easier." The presets are small
// buttons, two to a row; the two targets sit side by side under them; the list is edited from one quiet line.
async function sheetHeat(which='nozzle'){await loadPresets();const r=state.readings;
 openSheet({title:'Heat',icon:'i-heat',build:body=>{
  // audit 2026-10-26: twenty presets in one grid was a wall. Cool down first, then the materials by family, each family its own short row.
  const chips=el('div','nx-heat-groups'),FAM=[['PLA',/^(pla|silk|matte|pla\+)/i],['PETG',/^(petg|pctg|cpe)/i],['ABS · ASA',/^(abs|asa|hips|pc-abs)/i],['FLEXIBLE',/^(tpu|tpe|flex)/i],['NYLON · ENGINEERING',/./]];
  const fam=n=>FAM.find(([,re])=>re.test(String(n).trim()))?.[0]||'OTHER',rows=new Map();
  const rowFor=p=>{const k=p.fixed?'':fam(p.name);if(!rows.has(k)){const g=el('div','nx-heat-group');if(k)g.append(el('small','nx-heat-fam',k));const c=el('div','nx-heat-chips');g.append(c);chips.append(g);rows.set(k,c);}return rows.get(k);};
  for(const p of [...preheatMenu(state.presets)].sort((x,y)=>(y.fixed?1:0)-(x.fixed?1:0))){const b=el('button','nx-heat-chip');b.type='button';b.dataset.cmd='set_heater';if(p.fixed)b.dataset.cool='';
   b.append(el('b','',p.fixed?'Cool down':p.name),el('small','',p.fixed?'both off':`${p.nozzle}° · ${p.bed}°`));
   tip(b,p.fixed?'Cool down':'Preheat '+p.name,p.fixed?'Both heaters off.':`Nozzle ${p.nozzle}°, bed ${p.bed}°.`);
   b.addEventListener('click',async()=>{if(b.disabled)return;b.disabled=true;try{if(await send('set_heater',{target:'extruder',value:p.nozzle}))await send('set_heater',{target:'heater_bed',value:p.bed});}finally{b.disabled=false;lockControls(body);}});rowFor(p).append(b);}
  const tgt=(k,id,label)=>{const nf=numberField({...NXC.find(c=>c.key===k),...(k==='nozzle'?{max:290}:{}),value:Math.round(r[k].target||0),size:76}),w=el('div','nx-heat-tgt');
   const set=button({icon:k==='nozzle'?'i-heat':'i-bed',label:'Set',kind:'primary',size:'row',cmd:'set_heater',title:'Set '+label.toLowerCase()+' target',does:`Sends the ${label.toLowerCase()} target shown.`,onPress:async b=>{busy(b,true);await send('set_heater',{target:id,value:nf.get()});busy(b,false);}});
   w.append(el('small','',`${label.toUpperCase()}  ·  NOW ${fmtTemp(r[k].temp)}`),nf,set);return w;};
  const targets=el('div','nx-heat-tgts');targets.append(tgt('nozzle','extruder','Nozzle'),tgt('bed','heater_bed','Bed'));
  const edit=button({icon:'i-panels',label:'Edit the list',size:'row',kind:'ghost',title:'Edit the material list',does:'Add, change or take away materials, or add them back from the standard list.',onPress:()=>sheetPresetEditor()});
  body.append(section('Presets',chips),section('Targets',targets),edit);lockControls(body);}});}
// Material profiles (2026-10-26, "build those five"): each material is a card with its temperatures, its part fan, what it costs and weighs
// (for the print report's grams and cost), and what Calibrate measured for it (pressure advance, flow). Empty fields stay empty: nothing is guessed.
const MAT_FIELDS=[['nozzle','Nozzle','°',1,0,290],['bed','Bed','°',1,0,120],['fan','Part fan','%',5,0,100],['price_kg','Price','/kg',0.5,0,1000],['density','Density','g/cm³',0.01,0.5,3],['pa','Pressure adv.','',0.001,0,1],['flow','Flow','×',0.01,0.5,1.5]];
function sheetPresetEditor(){const saved=(state.presets?.materials||[]).map(m=>({...m}));let rows=saved.map((m,i)=>({...m,orig:i}));
 openSheet({title:'Materials',icon:'i-spool',wide:true,build:body=>{const list=el('div','nx-matprof'),status=el('p','nx-note');
  const paint=()=>{list.replaceChildren();const diff=presetDiff(saved,rows);
   rows.forEach((row,i)=>{const w=el('article','nx-mp');if(diff.states[i]!=='same')w.dataset.changed='';
    const name=el('input','nx-input nx-mp-name');name.value=row.name||'';name.placeholder='Material name';name.setAttribute('aria-label','Material name');
    name.addEventListener('input',()=>{row.name=name.value;status.textContent='Unsaved changes.';});
    const del=button({icon:'i-close',size:'icon',kind:'ghost',title:'Remove '+(row.name||'this material'),does:'Removes it when you press Save.',onPress:()=>{rows.splice(i,1);paint();}});
    const top=el('div','nx-mp-top');top.append(name,del);const grid=el('div','nx-mp-grid');
    for(const [k,label,unit,step,lo,hi] of MAT_FIELDS){const f=el('label','nx-mp-f'),inp=el('input','nx-input');inp.type='number';inp.step=String(step);inp.min=String(lo);inp.max=String(hi);inp.value=row[k]??'';inp.placeholder='—';
     inp.setAttribute('aria-label',label);inp.addEventListener('input',()=>{row[k]=inp.value===''?null:Number(inp.value);status.textContent='Unsaved changes.';});f.append(el('small','',label+(unit?' '+unit:'')),inp);grid.append(f);}
    w.append(top,grid);list.append(w);});
   status.textContent=diff.changed?`Unsaved changes${diff.removed?`, ${diff.removed} removed`:''}.`:'Saved.';};
  const add=button({icon:'i-plus',label:'Add a material',size:'row',title:'Add a material',does:'Adds an empty card.',onPress:()=>{rows.push({name:'',nozzle:null,bed:null,orig:null});paint();}});
  const std=button({icon:'i-plus',label:'Add from the standard list',size:'row',title:'Add from the standard list',does:'Shows the standard materials you do not have, with their usual temperatures. Pick one or add them all.',
   onPress:b=>addFromStandard(b,'materials',rows.map(r=>r.name),list=>{for(const m of list)rows.push({name:m.name,nozzle:m.nozzle,bed:m.bed,orig:null});paint();},{noun:'material',detail:m=>`${m.nozzle}° / ${m.bed}°`})});
  const cal=button({icon:'i-probe',label:'Measure in Calibrate',size:'row',title:'Calibrate',does:'Pressure advance and flow are measured there and land on the material that is loaded.',onPress:()=>openNamedSheet('calibrate')});
  const adds=el('div','nx-grid3');adds.append(add,std,cal);
  const save=button({icon:'i-check',label:'Save',kind:'primary',size:'row',title:'Save materials',does:'Stores the materials in Stella Scope.',onPress:async b=>{busy(b,true);try{if(rows.some(r=>!String(r.name).trim()||r.nozzle===null||r.bed===null))throw Error('Every material needs a name, a nozzle and a bed temperature.');
    const out=rows.map(r=>{const m={name:String(r.name).trim(),nozzle:Number(r.nozzle),bed:Number(r.bed)};for(const [k] of MAT_FIELDS.slice(2))if(r[k]!==null&&r[k]!==undefined&&r[k]!=='')m[k]=Number(r[k]);return m;});
    await jsend('/api/presets','PUT',{...(state.presets||{}),materials:out});await loadPresets();toast('Materials saved.','ok');closeSheet();}catch(e){status.textContent=e.message;}busy(b,false);}});
  const cancel=button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Discards the unsaved changes.',onPress:()=>closeSheet()});
  const actions=el('div','nx-grid2');actions.append(cancel,save);paint();
  body.append(el('p','nx-note','A material carries its temperatures, its part fan, its price and density (for the grams and cost of each print), and its pressure advance and flow from Calibrate. Loading a material with a pressure advance sets it on a Klipper printer. Nothing is kept until Save.'),list,adds,status,actions);}});}
// Move (2026-09-25: "very limited, and it looks like shit"): a real jog console. Live position
// with homed axes lit, one step bar, a round XY pad labelled with where the head actually goes on a
// Voron (Y+ is toward the back), a tall Z rocker (Z moves the gantry: Z+ lifts the nozzle), and the
// homing and parking moves beside them. Arrow keys jog X/Y, Page Up/Down jog Z while it is open.
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
 const hxy=el('button','nx-mv-hub');hxy.type='button';hxy.dataset.cmd='console';hxy.append(icon('i-home'),el('b','','HOME'),el('small','','X Y'));hxy.addEventListener('click',()=>script('G28 X Y','G28 X Y'));tip(hxy,'Home X and Y','Homes X and Y only.');pad.append(hxy);
 // Z rocker
 const zc=el('div','nx-mv-z');const zb=(ico,sign,big,small)=>{const b=el('button','nx-mv-zbtn');b.type='button';b.dataset.cmd='jog';b.append(icon(ico),el('b','',big),el('small','',small));b.addEventListener('click',()=>jog('Z',sign));tip(b,big,`Moves Z ${sign>0?'up':'down'} the step shown.`);return b;};
 const zUp=zb('i-z-up',1,'Z+','nozzle up'),zDn=zb('i-z-down',-1,'Z−','nozzle down'),zv=el('div','nx-mv-zval');const zval=el('b','','—');zv.append(el('small','','Z'),zval);
 const hz=button({icon:'i-home',label:'Home Z',size:'row',cmd:'console',title:'Home Z',does:'Homes Z only. X and Y must be homed first.',onPress:()=>script('G28 Z','G28 Z')});zc.append(zUp,zv,zDn,hz);
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
  button({icon:'i-home',label:'Home all',kind:'primary',size:'row',cmd:'home',title:'Home all axes',does:'Homes X, Y and Z.',onPress:async b=>{busy(b,true);await send('home');busy(b,false);}}),
  button({icon:'i-level',label:'Level gantry',size:'row',cmd:'qgl',title:'Quad gantry level',does:'Probes the four corners and levels the gantry.',onPress:async b=>{busy(b,true);await send('qgl');busy(b,false);}}),
  button({icon:'i-move',label:'Centre',size:'row',cmd:'console',title:'Centre the head',does:'Moves to the midpoint of the printer’s observed X/Y travel. Needs X and Y homed.',onPress:()=>{const g=travel();if(!g){toast('Printer travel limits are unavailable; Centre was not sent.','alarm');return;}if(!state.readings.homed.x||!state.readings.homed.y){toast('Home X and Y before centring.','alarm');return;}script(`G90\nG1 X${g.cx} Y${g.cy} F6000`,'centre');}}),
  button({icon:'i-arrow-up',label:'Park',size:'row',cmd:'console',title:'Park',does:'Runs your PARK macro.',onPress:()=>script('PARK','PARK')}),
  button({icon:'i-motors-off',label:'Motors off',size:'row',cmd:'motors_off',title:'Motors off',does:'Lets the motors go. The printer is no longer homed.',onPress:()=>send('motors_off')}));
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
 openSheet({title:speed?'Speed':'Flow',icon:speed?'i-speed':'i-flow',build:body=>{const st=numberField({...NXC.find(c=>c.key===(speed?'speed':'flow')),value:cur,size:112});   // U05: the kit's gauge, with its own step (speed 5%, flow 1%)
  const id=speed?'speed_factor':'extrude_factor',apply=button({icon:'i-check',label:'Apply',kind:'primary',size:'row',cmd:id,title:'Apply',does:`Sets the ${speed?'speed':'flow'} to the number above.`,onPress:async b=>{busy(b,true);await send(id,{value:st.get()});busy(b,false);}});
  const pre=el('div','nx-grid3');for(const v of speed?[50,75,100,125,150,200]:[90,95,100,105,110,115])pre.append(button({label:v+'%',size:'row',cmd:id,title:v+'%',does:`Sets the ${which} to ${v}%.`,onPress:()=>{st.setValue(v);send(id,{value:v});}}));
  const fl=flowModel(r,state.presets?.loaded?.name,state.presets?.hotend_max_flow);
  body.append(el('p','nx-note',speed?`Live velocity ${r.velocity===null?'—':Math.round(r.velocity)} mm/s. This scales every move in the print file.`:`Volumetric flow now ${fl.value===null?'—':fl.value.toFixed(1)} mm³/s. ${fl.max===null?'No measured hotend limit is saved.':`Measured limit ${fl.max} mm³/s.`} This scales how much plastic every move pushes out.`),st,apply,section('Presets',pre));lockControls(body);}});}
function sheetFan(){const r=state.readings;openSheet({title:'Fans',icon:'i-fan',build:body=>{const g=el('div','nx-grid3');
 for(const v of [0,25,50,75,100])g.append(button({label:v+'%',size:'row',cmd:'set_fan',title:`Part fan ${v}%`,does:`Sets the part cooling fan to ${v}%.`,onPress:()=>send('set_fan',{value:v/100})}));
 const auto=el('div','nx-grid2');auto.append(button({icon:'i-fan',label:'Hotend fan',size:'row',title:'Hotend fan',does:'Its panel, with the temperature it starts at.',onPress:()=>openPartByKey('hotend_fan')}),button({icon:'i-fan',label:'Bay fans',size:'row',title:'Electronics bay fans',does:'Their panel.',onPress:()=>openPartByKey('controller_fan')}));
 body.append(section('Part cooling fan',g,el('p','nx-note','Or drag round the Part ring on the main view.')),
  has('klipper')?section('Automatic fans',el('p','nx-note',`Hotend ${fmtPct(r.fans.hotend.speed)} · bay ${fmtPct(r.fans.controller.speed)}. Klipper runs these fans by itself, so you cannot set them. The hotend fan's start temperature can be changed from its panel.`),auto):el('span'),
  el('p','nx-note',printerWords(has('klipper')).fans));lockControls(body);}});}
// Files (2026-09-25: wasted real estate): a wide grid of cards, each with the slicer's picture,
// the name, and what Orca put in the file name (layer, material, time), newest first.
function fileFacts(path){const base=path.split('/').pop().replace(/\.gcode$/i,''),parts=base.split('_'),f={};
 for(const p of parts){if(/^\d+(\.\d+)?mm$/.test(p))f.layer=p;else if(/^(PLA|PETG|ABS|ASA|TPU|PC|PA|NYLON|HIPS)(\b|-)/i.test(p))f.mat=p;else if(/^(\d+d)?(\d+h)?(\d+m)?(\d+s)?$/.test(p)&&/\d/.test(p))f.time=p.replace(/(\d+)([dhms])/g,'$1$2 ').trim();}
 return f;}
// 2026-09-28: "there's nothing that says print, or things to print". The page is named for what
// Finish-2 F02 (CORE One L, 2026-10-19): a PrusaLink picture takes about 1.3 s and goes through the printer one at a time, so a
// file sheet that asked for every card at once held all six of the browser's connections for 33 s and a Print pressed meanwhile
// did nothing until they were done. Pictures now load when their card is on screen, two at a time; the rest of the page keeps
// its connections.
const thumbQ={busy:0,wait:[]};
function thumbPump(){while(thumbQ.busy<2&&thumbQ.wait.length){const [img,url]=thumbQ.wait.shift();if(!img.isConnected)continue;thumbQ.busy++;const done=()=>{thumbQ.busy--;thumbPump();};img.addEventListener('load',done,{once:true});img.addEventListener('error',done,{once:true});img.src=url;}}
const thumbSeen=typeof IntersectionObserver==='function'?new IntersectionObserver(es=>{for(const e of es)if(e.isIntersecting){thumbSeen.unobserve(e.target);thumbQ.wait.push([e.target,e.target.dataset.src]);}thumbPump();},{rootMargin:'200px'}):null;
function thumbLater(img,url){if(!thumbSeen){img.src=url;return;}img.dataset.src=url;thumbSeen.observe(img);}
// it is: the G-code on the printer, ready to print.
function sheetFiles(){openSheet({title:'Things to print',eyebrow:'FILES ON THE PRINTER',icon:'i-print',wide:true,build:async body=>{const top=el('div','nx-files-top'),search=el('input','nx-input');search.type='search';search.placeholder='Find something to print';search.setAttribute('aria-label','Find something to print');
 const count=el('span','nx-note');top.append(search,count);const grid=el('div','nx-filegrid');body.append(top,grid);
 if(['off','replay','service'].includes(link().state)){grid.append(el('p','nx-note',link().reason));return;}
 try{const r=await fetch('/api/files');const d=await r.json();if(!r.ok)throw Error(d.error||'The file list is not available.');const files=(d.files||d.result||[]).slice().sort((a,b)=>(b.modified||0)-(a.modified||0));
  const draw=()=>{grid.replaceChildren();const q=search.value.toLowerCase();const shown=files.filter(f=>!q||(f.path||f.filename||'').toLowerCase().includes(q)).slice(0,120);count.textContent=q?`${shown.length} of ${files.length} to print`:`${files.length} to print. Pick one and press Print.`;
   shown.forEach((f,i)=>{const path=f.path||f.filename,facts=fileFacts(path),card=el('div','nx-filecard');if(i===0&&!q)card.dataset.newest='';
    const pic=el('div','nx-filecard-pic'),img=el('img');img.alt='';img.addEventListener('error',()=>{img.replaceWith(icon('i-files'));});if(has('thumbnails')){pic.append(img);thumbLater(img,'/api/print/thumbnail?filename='+encodeURIComponent(path));}else pic.append(icon('i-files'));
    if(i===0&&!q)pic.append(el('i','nx-filecard-new','NEWEST'));
    const meta=el('div','nx-filecard-meta');for(const [k,v] of [['time',facts.time],['mat',facts.mat],['layer',facts.layer]])if(v){const c=el('span','',v);c.dataset.k=k;meta.append(c);}
    const name=el('b','',fileTitle(path)),when=el('small','',[f.modified?new Date(f.modified*1000).toLocaleString([], {month:'short',day:'numeric',hour:'numeric',minute:'2-digit'}):'',f.size?(f.size>1048576?(f.size/1048576).toFixed(1)+' MB':Math.round(f.size/1024)+' KB'):''].filter(Boolean).join(' · '));
    const acts=el('div','nx-filecard-acts');acts.append(button({icon:'i-print',label:'Print',size:'row',kind:'primary',cmd:'print_start',title:'Print '+fileTitle(path),does:'Starts this file now, after you confirm.',onPress:async b=>{busy(b,true);if(await send('print_start',{filename:path}))closeSheet();busy(b,false);}}),
     button({icon:'i-plus',size:'icon',cmd:'print_queue_add',title:'Queue',does:'Adds this file to the print queue.',onPress:()=>send('print_queue_add',{filename:path})}),
     button({icon:'i-view',size:'icon',title:'Details',does:'The slicer that made it, print time, filament and settings.',onPress:()=>openFile(path)}));
    card.append(pic,name,when,meta,acts);grid.append(card);});
   if(!shown.length)grid.append(el('p','nx-note',files.length?'Nothing to print matches.':'Nothing to print yet. Slice a model and send the file to this printer; it shows up here.'));lockControls(body);};
  search.addEventListener('input',draw);draw();}
 catch(e){grid.append(el('p','nx-note',e.message));}}});}
function sheetMacros(){openSheet({title:'Macros',icon:'i-macro',build:async body=>{u06(body);const list=el('div','nx-list');const search=el('input','nx-input');search.type='search';search.placeholder='Find a macro';search.setAttribute('aria-label','Find a macro');body.append(search,list);
 if(link().lock){list.append(el('p','nx-note',link().reason));return;}
 try{const r=await fetch('/api/macros');const d=await r.json();if(!r.ok)throw Error(d.error||'Macros are not available.');const mine=d.macros.filter(m=>m.macro===true&&!m.name.startsWith('_'));
  const draw=()=>{list.replaceChildren();const q=search.value.toLowerCase();for(const m of mine.filter(m=>!q||m.name.toLowerCase().includes(q))){const params=Array.isArray(m.params)?m.params:[];
   list.append(button({icon:'i-macro',label:m.name.replace(/_/g,' '),size:'row',cmd:'macro',title:m.name,does:(m.help?m.help+' ':'')+(params.length?'Opens its settings, then asks you to confirm.':'Asks you to confirm.'),
    onPress:()=>{if(!params.length)return send('macro',{target:m.name});openSheet({title:m.name,icon:'i-macro',build:mb=>{const fields={};for(const p of params){const i=el('input','nx-input');i.placeholder=p.default!=null?String(p.default):'';fields[p.name]=i;mb.append(section(p.name,i));}
     mb.append(button({icon:'i-play',label:'Run',kind:'primary',size:'row',cmd:'macro',title:'Run '+m.name,does:'Shows the command, then sends it once.',onPress:()=>{const values={};for(const[k,i]of Object.entries(fields))if(i.value.trim())values[k]=i.value.trim();send('macro',{target:m.name,params:values});}}));lockControls(mb);}});}}));}
   if(!list.children.length)list.append(el('p','nx-note','No macro matches.'));lockControls(list);};search.oninput=draw;draw();}
 catch(e){list.append(el('p','nx-note',e.message));}}});}
function sheetConsole(prefill=''){let stream=null;openSheet({title:'Console',icon:'i-console',onClose:()=>stream?.close(),build:body=>{u06(body);const log=el('div','nx-console');const add=(t,c='')=>{const d=el('div',c,t);log.append(d);log.scrollTop=log.scrollHeight;};
 for(const e of (state.snapshot?.events||[]).slice(-40))add(`${new Date(e.ts*1000).toLocaleTimeString()}  ${e.text||e.kind}`);
 try{stream=new EventSource('/api/tick/stream');stream.addEventListener('console',ev=>{try{const d=JSON.parse(ev.data);for(const l of (Array.isArray(d)?d:d.lines||[d]))add(`${l.ts?new Date(l.ts*1000).toLocaleTimeString()+'  ':''}${l.text||l.message||''}`);}catch{}});}catch{}
 const input=el('input','nx-input');input.placeholder='G-code, e.g. QUERY_ENDSTOPS';input.setAttribute('aria-label','G-code');input.dataset.cmd='console';input.value=prefill;
 const go=async()=>{const line=input.value.trim();if(!line)return;const r=await send('console',{script:line});if(r){add('> '+line,'cmd');input.value='';}};
 input.addEventListener('keydown',e=>{if(e.key==='Enter')go();});
 body.append(log,input,button({icon:'i-play',label:'Send',kind:'primary',size:'row',cmd:'console',title:'Send',does:'Shows the line for review, then sends it once.',onPress:go}));lockControls(body);}});}
function sheetExclude(){openSheet({title:'Exclude objects',icon:'i-exclude',build:async body=>{const list=el('div','nx-list');body.append(el('p','nx-note','Skip one object of this print. The rest keeps printing.'),list);
 try{const r=await fetch('/api/print/objects');const d=await r.json();if(!r.ok)throw Error(d.error||'The object list is not available.');const objs=d.objects||[];
  for(const o of objs)list.append(button({icon:o.excluded?'i-check':'i-exclude',label:o.name+(o.excluded?' · excluded':''),size:'row',kind:o.excluded?'default':'danger',cmd:'exclude_object',hold:!o.excluded,title:'Exclude '+o.name,does:'Stops printing this object.',onPress:()=>send('exclude_object',{name:o.name})}));
  if(!objs.length)list.append(el('p','nx-note','Nothing to skip in this print. Turn on Label objects in your slicer and each part of the plate can be skipped on its own.'));lockControls(body);}catch(e){list.append(el('p','nx-note',e.message));}}});}
// Recording (U06, 2026-10-20): the moment is an LED figure and a lit position bar; going through time is the kit's shuttle wheel (one click is a minute, ten minutes or an hour, picked on a three-way switch of square keys;
// the recording moves once, when the hand lets go, and the wheel springs back to rest), then the same square keys as before.
function sheetRecording(){openSheet({title:'Recording',icon:'i-rewind',build:body=>{u06(body);const s=state.snapshot;const first=s?.first_ts,last=s?.last_ts;
 const at=()=>finite(state.cursor)!==null&&!state.live?state.cursor:(finite(s?.point?.ts)??last??0),M=stat('MOMENT'),bar=el('div','nx-print-bar'),fill=el('i');bar.append(fill);
 const paint=()=>{const t=at();M.set(state.live?'LIVE':new Date(t*1000).toLocaleString([],{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',second:'2-digit'}),{note:finite(first)!==null&&finite(last)!==null?`recorded from ${new Date(first*1000).toLocaleString([],{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit'})} to ${new Date(last*1000).toLocaleString([],{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit'})}`:'',tone:state.live?'ok':''});
  fill.style.width=(finite(first)!==null&&last>first?Math.max(0,Math.min(1,(t-first)/(last-first)))*100:100).toFixed(1)+'%';};paint();
 const hm=m=>{const a=Math.abs(m);return a>=60?Math.floor(a/60)+' h'+(a%60?' '+(a%60)+' min':''):a+' min';};
 const sh=relWheel({label:'SHUTTLE',unit:'min',steps:[{label:'1 min',step:1,span:30,turn:300},{label:'10 min',step:10,span:120,turn:300},{label:'1 hour',step:60,span:720,turn:300}],fmt:v=>(v>0?'+':v<0?'\u2212':'')+hm(v),
  send:v=>{const to=Math.max(first??0,Math.min(last??Infinity,at()+v*60));state.live=false;state.playing=false;request(to);state.cursor=to;paint();}});
 tip(sh.kit.el,'Shuttle','Press and hold, then turn: one click goes back or forward by the step you chose. Letting go moves the recording once.');
 const deck=el('div','nx-jw'),side=el('div','nx-jw-side');side.append(el('span','nx-jw-l','One click is'),sh.keys,el('p','nx-note','Turn it back to go back in time, forward to come nearer to now.'));deck.append(sh.kit.el,side);
 const ctl=el('div','nx-grid3');ctl.append(button({icon:'i-step-back',label:'Back',size:'row',title:'Back',does:'One step back in the recording.',onPress:()=>{recAction('back');setTimeout(paint,600);}}),button({icon:state.playing?'i-pause':'i-play',label:state.playing?'Pause':'Play',size:'row',title:'Play',does:'Plays from here.',onPress:()=>recAction('play')}),button({icon:'i-step-fwd',label:'Forward',size:'row',title:'Forward',does:'One step forward in the recording.',onPress:()=>{recAction('fwd');setTimeout(paint,600);}}));
 body.append(section('Position',M.el,bar),section('Go through time',deck),ctl,section('Speed',segmented([1,2,5,10],state.speed,v=>{state.speed=v;},v=>v+'×')),
  button({icon:'i-live',label:'Go live',kind:'primary',size:'row',title:'Go live',does:'Back to the live printer.',onPress:()=>{recAction('live');closeSheet();}}),
  button({icon:'i-link',label:'Copy link to this moment',size:'row',title:'Copy link',does:'Copies a link that opens this exact moment.',onPress:async()=>{try{await navigator.clipboard.writeText(location.href);toast('Link copied.','ok');}catch{toast(location.href);}}}));}});}
function sheetLook(){openSheet({title:'Look',icon:'i-colors',build:body=>body.append(lookPanel())});}
function sheetView(){openSheet({title:'View',icon:'i-view',build:body=>{const g=el('div','nx-grid2');
 for(const [v,lab]of[['iso','Iso'],['front','Front'],['top','Top'],['right','Right']])g.append(button({icon:'i-view',label:lab,size:'row',title:lab+' view',does:'Turns the camera to the '+lab.toLowerCase()+' view.',onPress:()=>scene?.view?.(v)}));
 body.append(section('Camera',g,button({icon:'i-back',label:'Reset view',size:'row',title:'Reset view',does:'Frames the whole printer again.',onPress:()=>scene?.reset?.()})),
  section('Screen',button({icon:'i-fullscreen',label:'Full screen',size:'row',title:'Full screen',does:'Fills the screen with Stella Scope.',onPress:()=>{if(document.fullscreenElement)document.exitFullscreen();else document.documentElement.requestFullscreen?.();}}),
   button({icon:'i-panels',label:'Panels see-through',size:'row',title:'Panels',does:'Makes the enclosure panels see-through.',onPress:()=>scene?.panels?.(true)})),
  (has('mesh')?section:()=>el('span'))('On the printer',button({icon:'i-mesh',label:meshOn?'Hide the mesh on the bed':'Show the mesh on the bed',size:'row',title:'Mesh on the bed',does:'Drapes the newest bed mesh over the plate in the 3D view, heights exaggerated 25 times.',onPress:b=>{setMeshOn(!meshOn);b.querySelector('.nx-lbl').textContent=meshOn?'Hide the mesh on the bed':'Show the mesh on the bed';}})),
  eyeSwitch(),
  section('Phone',button({icon:'i-link',label:'Pair a phone, paired phones, WhatsApp',size:'row',title:'Phone',does:'One scan sets a phone up. Also the paired phones and WhatsApp alerts.',onPress:()=>sheetPhone()})),
  section('3D printer on this device',segmented(['on','off'],show3d()?'on':'off',v=>{try{localStorage.setItem('nx-3d',v);}catch{}toast(v==='on'?'Reload to show the 3D printer.':'Reload to hide it.');},v=>v==='on'?'Show':'Hide'),el('p','nx-note','The 3D machine is drawn on every device, a phone included. Hide it here if this device is too slow for it; everything else works without it.')),
  section('Printer screen',segmented(['off','on'],kiosk?'on':'off',v=>setKiosk(v==='on'),v=>v==='on'?'Kiosk mode':'Normal'),el('p','nx-note','Kiosk mode is for a touchscreen on the printer: no side rail, bigger buttons, and the screen dims after three minutes without a touch. The first touch only wakes it.')));}});}   // (2026-10-25, finish E15: the old view's link is retired: everything it did is on these pages; /classic-hud still answers a bookmark)
// The spaghetti watch, on or off from here (2026-09-25). The first time on fetches Obico's model
// (200 MB) in the background and shows how far it has got.
function eyeSwitch(){const box=el('div','nx-sheetpick'),line=el('p','nx-note'),bar=el('div','nx-print-bar'),fill=el('i');bar.append(fill);bar.hidden=true;
 let seg=null;
 const paint=d=>{const dl=d.download||{};bar.hidden=!dl.active;if(dl.active)fill.style.width=Math.round((dl.bytes||0)/(dl.total||1)*100)+'%';
  if(d.installed===false&&!d.enabled){line.textContent='The failure watch is an optional add-on. The Help guide, under Failure watch, shows how to add it.';return;}
  line.textContent=dl.active?`Getting the failure watch: ${Math.round((dl.bytes||0)/1048576)} of ${Math.round((dl.total||1)/1048576)} MB…`:dl.error?`Could not get the failure watch: ${dl.error}`:d.enabled?'On. It looks at the camera every 10 seconds while printing and warns you here and on WhatsApp. It never stops a print by itself.':'Off. Turning it on downloads about 200 MB the first time.';
  if(dl.active)setTimeout(load,1000);};
 const load=()=>fetch('/api/eye').then(r=>r.json()).then(d=>{if(!seg){seg=segmented(['on','off'],d.enabled?'on':'off',v=>{if(!state.desktopAccess){toast('Switch the failure watch on or off from the computer, not from this phone.','alarm');return;}fetch('/api/eye/switch',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({on:v==='on'})}).then(r=>r.json()).then(j=>{if(j.error)toast(j.error,'alarm');paint(j);if(j.download?.active)setTimeout(load,1000);});},v=>v==='on'?'On':'Off');box.prepend(seg);}paint(d);}).catch(()=>{line.textContent='Not available.';});
 box.append(line,bar);load();return section('Failure watch',box);}
// ---------- phone access (src/lan.rs) ----------
async function phoneBadge(){const b=$('nx-phonebtn');if(!b)return;let n=0;try{const d=await (await fetch('/api/pair/devices')).json();if(!d.enabled){b.hidden=true;return;}n=(d.devices||[]).length;}catch{return;}
 b.hidden=false;b.dataset.paired=n?'1':'';b.querySelector('use')?.setAttribute('href',n?'#i-phone-check':'#i-phone');
 const t=n?(n>1?n+' phones paired':'Phone paired'):'Pair phone';const lab=b.querySelector('.nx-lbl');if(lab)lab.textContent=t;b.setAttribute('aria-label',t);}
// 2026-09-25: "where do I find add my phone" -- its own page on the rail, not only inside View.
// Phone (2026-09-25: "it should just be a scan of the QR code and set it up"). Three tabs. Pair:
// get the app from the HTTPS download page, then one
// scan in the app both finds this computer, at home or away, and pairs the phone; the six digits are
// there for the app's pairing page. Paired phones: the list, each removable. WhatsApp: the alerts.
function sheetPhone(){openSheet({title:'Phone',icon:'i-link',wide:true,build:body=>{
  if(!state.desktopAccess){body.append(el('p','nx-note','This phone is paired. Pairing and alerts are set up from the Stella Scope window on the computer.'));return;}
 const tabs={pair:el('div','nx-tabpane'),phones:el('div','nx-tabpane'),wa:el('div','nx-tabpane'),alerts:el('div','nx-tabpane')};tabs.phones.hidden=tabs.wa.hidden=tabs.alerts.hidden=true;
 const bar=segmented(['pair','phones','wa','alerts'],'pair',k=>{for(const [n,p] of Object.entries(tabs))p.hidden=n!==k;if(k==='phones')drawPhones();},k=>(innerWidth<=480?{pair:'Pair',phones:'Phones',wa:'WhatsApp',alerts:'Alerts'}:{pair:'Pair a phone',phones:'Paired phones',wa:'WhatsApp',alerts:'Alerts'})[k]);bar.classList.add('nx-tabs');   // F06: four tabs did not fit a phone ("Alerts" ran off the edge at 390)
 body.classList.add('nx-sheet-body--tabs');body.append(bar,tabs.pair,tabs.phones,tabs.wa,tabs.alerts);tabs.wa.append(whatsApp());import('./next-notify.js').then(m=>tabs.alerts.append(m.alertsPanel()));
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
  // F06 (2026-10-19): "Enable remote_link" named a setting in a file and gave no way to set it. The button writes it; the phone's door opens when Stella Scope starts.
  if(!d.enabled&&d.pending_restart)tabs.pair.prepend(el('p','nx-note','Phone access is on. Close Stella Scope and open it again, then come back here to pair a phone.'));
  else if(!d.enabled){const off=el('div','nx-pairoff');const msg=el('p','nx-note','Turn it on, then close Stella Scope and open it again: the phone link starts with it. Nothing is sent anywhere until you pair a phone.');
   off.append(msg,button({icon:'i-link',label:'Turn on phone access',kind:'primary',size:'row',title:'Turn on phone access',does:'Switches phone access on in this computer\'s settings. It starts the next time Stella Scope starts.',onPress:async b=>{busy(b,true);
    try{const t=await fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).then(j=>j.writable&&j.token?j.token:null);if(!t)throw Error('Phone access is switched on from the Stella Scope window on the computer.');
     const r=await fetch('/api/farm/phone-access',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':t},body:'{}'});const j=await r.json();if(!r.ok)throw Error(j.error);
     off.replaceChildren(el('p','nx-note','Phone access is on. Close Stella Scope and open it again, then come back here to pair a phone.'));status.hidden=true;}
    catch(e){msg.textContent=e.message;}finally{busy(b,false);}}}));tabs.pair.prepend(off);}
  if(!d.enabled)go.hidden=true;
  status.hidden=!d.enabled&&!!d.pending_restart;
  status.textContent=!d.enabled?(d.pending_restart?'Phone access is on and starts the next time Stella Scope starts.':'Phone access is switched off on this computer.'):!d.running?'Anywhere access is not running right now. Restart Stella Scope to start it.':d.online?'Ready: a paired phone reaches this computer at home and away.':'Starting anywhere access…';status.dataset.tone=d.online?'ok':'';})();
 // Paired phones tab
 async function drawPhones(){tabs.phones.replaceChildren();let d;try{d=await (await fetch('/api/pair/devices')).json();}catch{tabs.phones.append(el('p','nx-note','Not available.'));return;}
  if(!(d.devices||[]).length){tabs.phones.append(el('p','nx-note','No phone is paired yet.'));return;}
  const list=el('div','nx-phonelist');for(const dv of d.devices){const row=el('div','nx-phonecard');row.append(icon('i-link'),el('b','',dv.name),el('small','','paired '+new Date(dv.paired_at*1000).toLocaleString()),button({icon:'i-close',label:'Remove',size:'row',kind:'danger',title:'Remove',does:'This phone must pair again to reach Stella Scope.',onPress:async()=>{await fetch('/api/pair/revoke',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:dv.id})});drawPhones();}}));list.append(row);}
  tabs.phones.append(list);}
}});}
// Without the 3D model (phones, by default) the stage shows the camera, or offers the model.
function stageCard(){const c=el('div','nx-stagecard nx-glass');$('nx-ui').append(c);
 const offer=()=>{c.replaceChildren(el('p','nx-note','The 3D printer is not loaded on this device (the model is over 100 MB).'),button({icon:'i-view',label:'Show the 3D printer',size:'row',title:'Show the 3D printer',does:'Loads the 3D model on this device from now on.',onPress:()=>{try{localStorage.setItem('nx-3d','on');}catch{}location.reload();}}));};
 const tabs=()=>import('./next-models.js').then(m=>m.stageTabs(c,{id:(/^\/p\/([a-z0-9-]{1,32})(?:\/|$)/.exec(location.pathname)||[])[1]||null,noClassic:!!window.__stellaNoClassic})).catch(()=>{});
 // polish WO-05 (2026-10-18): a printer just added is not live yet when its page opens; the catalogue model (which follows it once it answers) still gets its stage.
 if(['off','service','replay'].includes(link().state)){offer();if(link().state==='off')tabs();return;}   // not in a replay (it is not the live position) nor with the service down
 cameras().then(cams=>{if(!cams.length){offer();return;}const img=camImg(cams[0]);
  // polish WO-05 (2026-10-18): a camera that does not answer (the CORE One L's, when it is off) must not wipe the 3D model the stage tabs have put in this card since: the picture alone is replaced, by a plain line.
  img.addEventListener('error',()=>{if(img.parentElement===c)offer();else img.replaceWith(el('p','nx-note','The camera is not answering.'));});img.addEventListener('click',()=>pageCamera());c.replaceChildren(img);}).catch(offer).finally(tabs);}
// ---------- WhatsApp alerts (src/notify.rs, CallMeBot) ----------
function whatsApp(){const box=el('div','nx-sheetpick');const load=async()=>{box.replaceChildren();if(!state.desktopAccess){box.append(el('p','nx-note','Set up from the Stella Scope window on the computer.'));return;}
 let d;try{d=await (await fetch('/api/notify')).json();}catch{box.append(el('p','nx-note','Not available here.'));return;}
 const phone=el('input','nx-input'),key=el('input','nx-input'),msg=el('p','nx-note');phone.placeholder='+1 506 …';phone.value=d.phone||'';phone.setAttribute('aria-label','WhatsApp number with country code');key.placeholder=d.has_key?'key saved (type to replace)':'CallMeBot key';key.setAttribute('aria-label','CallMeBot key');key.autocomplete='off';
 const post=(u,b)=>fetch(u,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(b)}).then(async r=>{const j=await r.json();if(!r.ok)throw Error(j.error);return j;});
 box.append(el('p','nx-note',`Messages when a print finishes, fails or is cancelled, ${has('klipper')?'on a BLTouch alarm, a motor driver warning, Klipper stopping, or ':'or '}the printer dropping off mid-print. One-time setup: add ${d.bot_number} to your phone's contacts and send it "${d.activation}" on WhatsApp. It replies with your key; paste it below.`),phone,key,
  button({icon:'i-check',label:d.enabled?'Save':'Save and turn on',kind:'primary',size:'row',title:'Save',does:'Stores the number and key on this computer only.',onPress:async()=>{try{const j=await post('/api/notify',{phone:phone.value,apikey:key.value,enabled:true});toast(j.enabled?'WhatsApp alerts are on.':'Saved. Paste the key to turn them on.','ok');load();}catch(e){msg.textContent=e.message;}}}),
  button({icon:'i-play',label:'Send a test message',size:'row',title:'Send a test',does:'Sends one WhatsApp message to the number above.',onPress:async()=>{try{const j=await post('/api/notify/test',{});msg.textContent=j.queued?'Sent to CallMeBot; it should arrive within a minute.':'Held back: turn alerts on first, or wait 20 s between messages.';setTimeout(load,6000);}catch(e){msg.textContent=e.message;}}}),
  d.enabled?button({icon:'i-close',label:'Turn off',size:'row',title:'Turn off',does:'Stops WhatsApp alerts; the key stays saved.',onPress:async()=>{await post('/api/notify',{phone:phone.value,enabled:false});load();}}):el('span'),msg);
 if(d.last)msg.textContent=`Last message ${new Date(d.last.at*1000).toLocaleTimeString()}: ${d.last.ok?'accepted':'not accepted'}${d.last.reply?' · '+d.last.reply:''}`;};load();return section('WhatsApp alerts',box);}
function show3d(){try{const v=localStorage.getItem('nx-3d');if(v)return v==='on';}catch{}return true;}   // U02 (2026-10-20): the printer page shows the 3D machine on a phone too; 'off' here is the choice of someone whose device cannot carry it
function sheetPages(){openSheet({title:'Pages',icon:'i-nav-hud',build:body=>{const l=el('div','nx-list');for(const [ico,name,href,does]of ui.pages.filter(p=>!PAGE_NEEDS[p[2].slice(6)]||has(PAGE_NEEDS[p[2].slice(6)]))){const b=el('a','nx-item');b.href=href.startsWith('sheet:')?'#':href;if(href.startsWith('sheet:'))b.addEventListener('click',e=>{e.preventDefault();openNamedSheet(href.slice(6));});b.style.textDecoration='none';b.append(icon(ico),Object.assign(el('div','nx-item-text'),{}));b.lastChild.append(el('div','nx-item-title',name),el('div','nx-item-sub',doesOf(name,does)));l.append(b);}body.append(l);}});}

// ---------- toolpath on the plate (ported from hud.js showToolpath / toolpathAuto / toolpathTick) ----------
// the owner: the old HUD drew the print on the plate while it ran; NEXT had lost it. The file is read once
// per job from the printer, never in a replay or a rewound view, and a failure backs off 5, 15, 60 s.
const TP_BACKOFF_S=[5,15,60];let gcodeMod=null,tp=null,tpAuto=null,tpFail=0,tpNext=0,tpNoted=null;
async function showToolpath(path){gcodeMod??=await import('./hud-gcode.js');const r=await fetch('/api/print/gcode?filename='+encodeURIComponent(path));if(!r.ok)throw Error('The layer preview needs the print file, and the printer did not send it.');
 const text=await r.text(),parsed=gcodeMod.parseGcode(text,{maxSegments:1500000});tp={path,parsed,bytes:parsed.byteLength||text.length,layer:null,fraction:0,filePos:null,source:'following the file'};
 tp.follower=new GcodeFollower();tp.follower.loadJob(path,tp.bytes,text);
 const info=scene?.loadToolpath(parsed,path);if(scene&&!info)throw Error(scene.bedCenter?'That file has no printing moves to draw.':'The 3D printer is still loading; the layers follow.');scene?.associateJob?.(path);return !!info;}
function toolpathAuto(){const j=state.readings.job,name=j.name||'';if(!name||!['printing','paused'].includes(j.state))return;const file=name.split('/').pop();
 if(tpAuto===file||tp?.path.split('/').pop()===file)return;
 if(link().state==='replay'||!state.live){if(tpNoted!==file){tpNoted=file;tp={path:name,parsed:null,error:'A recording does not keep the print file, so this print\'s layers cannot be drawn here.'};}return;}
 // polish WO-06 item 3 (2026-10-18): a printer that does not let the service read its print file (a FlashForge) is not asked, and the Print sheet says why, once.
 if(!has('gcode_file')){if(tpNoted!==file){tpNoted=file;tp={path:name,parsed:null,error:'This printer does not share its print file, so the layers cannot be drawn here. Progress and time left still come from the printer.'};}return;}
 if(['service','off'].includes(link().state)||performance.now()<tpNext||(scene&&!scene.bedCenter))return;tpAuto=file;
 showToolpath(name).then(drawn=>{tpFail=0;tpNext=0;if(drawn)toast(`${tp.parsed.totalLayers||tp.parsed.layers.length} layers of ${fileTitle(name)} drawn on the plate.`);})
  .catch(e=>{tpAuto=null;tp={path:name,parsed:null,error:e.message};const wait=TP_BACKOFF_S[Math.min(tpFail,TP_BACKOFF_S.length-1)];tpNext=performance.now()+wait*1000;if(++tpFail===1)toast(`${e.message} Trying again in ${wait} s, then less often.`);});}
function toolpathTick(){if(!tp?.parsed)return;const j=state.readings.job;if(j.name&&j.name.split('/').pop()!==tp.path.split('/').pop())return;
 const p=lastTick?.point??state.snapshot?.point,pr=finite(j.progress)??0;
 const live=p?.position&&p.position.length>=2&&Number.isFinite(p.position[0])&&Number.isFinite(p.position[1]);
 const signal={};
 if(Number.isFinite(p?.file_position))signal.byte=p.file_position;
 else if(Number.isFinite(pr))signal.progress=pr;
 else if(Number.isFinite(p?.print_stats?.info?.current_layer)){signal.layer=p.print_stats.info.current_layer;signal.layers=p.print_stats.info.total_layer;}
 else if(Number.isFinite(j.time_printing)&&Number.isFinite(j.estimated_time)){signal.elapsed=j.time_printing;signal.estimate=j.estimated_time;}
 const z=Number.isFinite(p?.position?.[2])?p.position[2]:null;
 const followed=tp.follower.tick({signal,live,z,now:performance.now()});
 if(followed){tp.layer=followed.layer;tp.fraction=followed.fraction;tp.source=followed.source;tp.z=followed.z;}
 const at=Number.isFinite(p?.file_position)?Math.round(p.file_position):Math.round(pr*tp.bytes);
 scene?.toolpathProgress?.({layer:tp.layer,fraction:tp.fraction,filePosition:at,live:p?.position});}
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
function paintDim(){const d=$('nx-dim');if(!d||d.hidden)return;const r=state.readings,j=jobModel(r);d.firstChild.textContent=j.active?`${j.title} · ${j.progress===null?'—':Math.round(j.progress*100)+'%'}`:(state.printerName||'The printer')+': '+(link().label||'').toLowerCase();
 d.lastChild.textContent=`Nozzle ${fmtTemp(r.nozzle.temp)} · Bed ${fmtTemp(r.bed.temp)} · touch to wake`;}
function initKiosk(){const d=el('div','nx-dim');d.id='nx-dim';d.hidden=true;d.append(el('b'),el('span'));document.body.append(d);
 // The first touch on a dimmed screen only wakes it: it never reaches a button underneath.
 d.addEventListener('pointerdown',e=>{e.preventDefault();e.stopPropagation();wake();},true);
 for(const ev of ['pointerdown','keydown','wheel'])document.addEventListener(ev,()=>{if(kiosk&&$('nx-dim').hidden)wake();},true);
 setInterval(paintDim,2000);document.body.toggleAttribute('data-kiosk',kiosk);wake();}

// ---------- BLTouch light (2026-09-24: "the BL Touch is supposed to blink") ----------
// Klipper does not report the LED, so it is read from what the probe did and said: steady with the
// pin up, a flash each time it touches while probing, fast flashing after a reported failure until
// a good touch, a reset or a restart clears it, and dark when the printer has no power.
const BL_FAIL=/BLTouch failed|failed to (verify sensor state|deploy|raise probe)|Probe triggered prior to movement|No trigger on probe after full movement/i;
const BL_CLEAR=/BLTOUCH_DEBUG COMMAND=reset|FIRMWARE_RESTART|^RESTART\b/i;
let ledState='',ledLastPt=0;
function paintProbeLed(){const l=link(),ev=state.snapshot?.events||[],text=e=>e.message||e.text||'';
 const fail=ev.filter(e=>e.kind!=='command'&&BL_FAIL.test(text(e))).reduce((a,e)=>!a||e.ts>a.ts||(e.ts===a.ts&&/^!!/.test(text(e)))?e:a,null),clear=ev.filter(e=>e.kind==='command'&&BL_CLEAR.test(text(e))).reduce((a,e)=>Math.max(a,e.ts),0);
 const lastPt=[...probeStore.values()].reduce((a,q)=>Math.max(a,q.ts),0),now=lastTick?.point?.ts??Date.now()/1000;
 const st=['off','service','checking'].includes(l.state)?'off':fail&&fail.ts>Math.max(clear,lastPt)?'alarm':lastTick?.activity?.id==='probing'||now-lastPt<4?'probing':'on';
 state.bltouch=st;state.bltouchWhy=st==='alarm'?`Klipper said: "${text(fail).replace(/^!!\s*/,'')}".`:'';
 const flash=lastPt>ledLastPt&&ledLastPt>0;ledLastPt=lastPt;if(scene?.probeLed&&(st!==ledState||flash)){if(scene.probeLed(st,{flash:flash&&st==='probing'}))ledState=st;}}

// ---------- anything wrong pops up (2026-09-24) ----------
// Each check names a problem with a key. A new key raises a card at the top of the screen that stays
// until it is dismissed or the problem clears; a dismissed key stays quiet until it clears and comes
// back. Console errors are keyed by their time, so each new one is raised once.
const shown=new Map(),quiet=new Set(),seenAt=Date.now()/1000-120,heatWatch={nozzle:[],bed:[]},rtWatch=new Map();
function alertHost(){let h=$('nx-alerts');if(!h){h=el('div','nx-alerts');h.id='nx-alerts';h.setAttribute('role','alert');$('nx-ui').append(h);}return h;}
function checkAlerts(){const l=link(),r=state.readings,p=state.snapshot?.point,ev=state.snapshot?.events||[],found=[];
 if(!state.live){for(const k of [...shown.keys()])dropAlert(k);return;}
 const add=(key,title,detail,act=null)=>found.push({key,title,detail,act});
 for(const k of ['nozzle','bed']){const h=heatColor(r[k].temp,r[k].target,k);if(h.tone==='danger'&&!document.body.hasAttribute('data-stale'))add('hot:'+k,(k==='nozzle'?'Nozzle':'Bed')+' over temperature',`${fmtTemp(r[k].temp)} against a target of ${fmtTemp(r[k].target)}. Check the heater; STOP cuts it at once.`,{label:'Show me',go:()=>showPartByKey(k==='nozzle'?'hotend':'bed')});}
 for(const t of dueTasks(state.printHours??null))add('maint:'+t.name,'Maintenance due: '+t.name,`Every ${t.every_h} print hours; it has been ${Math.round(state.printHours-(t.last_h??0))}.`,{label:'Maintenance',go:()=>openNamedSheet('maintenance')});
 if(l.state==='not_ready'){ // WO-13: the words and the way out are the driver's, not Klipper's.
  if(l.klipper===false){const msg=l.reason||'The printer is not ready.',login=/log ?in|password|401|403|unauthor/i.test(msg);
   add('printer',login&&state.printerName?`Can't log in to the ${state.printerName}`:login?'Can\'t log in to the printer':`${caps.brand||'The printer'} is not ready`,msg,login?{label:'Check login',go:()=>openNamedSheet('printers')}:{label:'Printers',go:()=>openNamedSheet('printers')});}
  else add('klipper','Klipper has stopped',l.reason||'Klipper is not ready.',{label:'Firmware restart',go:()=>send('firmware_restart')});}
 // polish WO-06 (live finding, 2026-10-19): a paused print says why when the printer says it (a PrusaLink in ATTENTION: check its screen)
 if(r.job.state==='paused'&&r.job.message)add('attn:'+r.job.name,`${state.printerName||'The printer'} paused with a message`,r.job.message,null);
 if(l.state==='off'&&['printing','paused'].includes(state.lastJobState))add('offline-print','The printer stopped answering during a print',l.reason||'The printer does not answer.',null);
 if(state.bltouch==='alarm')add('bltouch:'+(state.bltouchWhy||''),'BLTouch in alarm',state.bltouchWhy+' Its light is flashing.',{label:'Show the probe',go:()=>showPartByKey('probe')});
 // Open-load bits at standstill are not a warning (next-core driverFlags; 2026-09-28: a false
 // "stepper_x TMC2209 reports OLB" on a parked printer). The head or the extruder must be moving.
 const moving=(r.velocity??0)>1||Math.abs(r.extruderVelocity??0)>.1;
 // The live tick, not the snapshot (refreshed every 5 s): the flags and the motion must be read at the same moment.
 for(const [name,m]of Object.entries((state.live&&lastTick?.point?.motors)||p?.motors||{})){const fl=driverFlags(m,moving);if(fl.length)add(`drv:${name}:${fl.join(',')}`,`Motor driver warning: ${name}`,`The TMC2209 reports ${fl.join(', ')}.`,{label:'Show the motor',go:()=>showPartByKey(name==='extruder'?'extruder':name)});}
 for(const e of ev)if(e.ts>seenAt&&e.kind==='response'&&/^!!/.test(e.message||e.text||''))add('err:'+e.ts,'Klipper reported an error',(e.message||e.text).replace(/^!!\s*/,''),{label:'Timeline',go:()=>openNamedSheet('timeline')});
 // A heater driven hard that is not warming: 60 s at over 80 % output with under 2 degrees gained.
 const now=Date.now()/1000;for(const k of ['nozzle','bed']){const w=heatWatch[k],x=r[k];if(x.temp!==null&&x.target>0&&x.power!==null){w.push([now,x.temp,x.power]);while(w.length&&w[0][0]<now-65)w.shift();}else w.length=0;
  if(w.length>10&&now-w[0][0]>=55&&w.every(q=>q[2]>.8)&&w.at(-1)[1]-w[0][1]<2&&x.temp<x.target-10)add('stall:'+k,`The ${k} is not heating`,`Full power for a minute and only ${(w.at(-1)[1]-w[0][1]).toFixed(1)}° gained. Check the heater and thermistor wiring.`,{label:'Show me',go:()=>showPartByKey(k==='nozzle'?'hotend':'bed')});}
 if(r.nozzle.temp!==null&&r.nozzle.temp>70&&r.fans.hotend.speed===0)add('hotend-fan','Hotend fan is off with a hot nozzle',`The nozzle is at ${fmtTemp(r.nozzle.temp)} and the hotend fan command is 0 %. Heat can creep up the heatbreak.`,{label:'Show the fan',go:()=>showPartByKey('hotend_fan')});
 for(const s of p?.telemetry?.signals||[]){if(s.field!=='last_stats.bytes_retransmit'||typeof s.value!=='number')continue;const w=rtWatch.get(s.object)||[];w.push([now,s.value]);while(w.length&&w[0][0]<now-65)w.shift();rtWatch.set(s.object,w);
  if(w.length>2&&w.at(-1)[1]-w[0][1]>300)add('rt:'+s.object,s.object==='mcu'?'The controller link is dropping data':'The toolhead board link is dropping data',`${w.at(-1)[1]-w[0][1]} bytes resent in the last minute. Check the ${s.object==='mcu'?'USB':'CAN'} cable and its plugs.`,{label:'Show the board',go:()=>showPartByKey(s.object==='mcu'?'controller':'toolboard')});}
 // Only a print seen live in this session counts: a page opened after power-off stays quiet.
 if(l.state==='ready')state.lastJobState=r.job.state;
 const keys=new Set(found.map(a=>a.key));for(const k of [...shown.keys()])if(!keys.has(k)&&!k.startsWith('err:'))dropAlert(k);for(const k of [...quiet])if(!keys.has(k)&&!k.startsWith('err:'))quiet.delete(k);
 for(const a of found)if(!shown.has(a.key)&&!quiet.has(a.key))raiseAlert(a);}
function raiseAlert(a){const card=el('div','nx-alert nx-glass nx-glass--raised'),t=el('div','nx-alert-t');t.append(el('b','',a.title),el('span','',a.detail));
 const row=el('div','nx-alert-row');if(a.act)row.append(button({label:a.act.label,size:'row',kind:'primary',title:a.act.label,does:'',onPress:()=>a.act.go()}));
 row.append(button({icon:'i-close',size:'icon',kind:'ghost',title:'Dismiss',does:'Hides this until the problem clears and comes back.',onPress:()=>{quiet.add(a.key);dropAlert(a.key);}}));
 card.append(icon('i-warn'),t,row);alertHost().prepend(card);shown.set(a.key,card);while(alertHost().children.length>3)alertHost().lastChild.remove();try{navigator.vibrate?.([60,40,60]);}catch{}lockControls(card);}
function dropAlert(k){shown.get(k)?.remove();shown.delete(k);}

// ---------- the camera as the background (U02, 2026-10-20) ----------
// Camera is a choice of what fills the whole background: the 3D machine or the live picture. The glass turns light over the picture (body[data-bg=camera]), and the picture grows,
// zooms and resets exactly as the machine does (next-stage.js).
let camPinned=(()=>{try{return localStorage.getItem('nx-campin')==='1';}catch{return false;}})(),camBusy=false;
function setCamPinned(v){camPinned=v;try{localStorage.setItem('nx-campin',v?'1':'0');}catch{}$('nx-camtoggle')?.setAttribute('aria-pressed',String(v));paintCamPin();}
async function paintCamPin(){let box=$('nx-bgcam');if(!camPinned||['off','service','replay'].includes(link().state)){if(box){box.querySelector('img')?.setAttribute('src','');box.remove();}delete document.body.dataset.bg;return;}
 if(box||camBusy)return;camBusy=true;
 try{const cams=await cameras();if(!camPinned)return;if(!cams.length){toast('No camera for this printer yet.');setCamPinned(false);return;}
  // rc9 WO-03: until the first picture arrives the lit stage stays and says what it is waiting for (it was ten seconds of black on Stella)
  box=el('div');box.id='nx-bgcam';const z=el('div','nx-bgcam-z'),img=camImg(cams[0]),wait=el('div','nx-bgcam-wait nx-glass');wait.append(el('i','nx-dock-spin'),el('span','','Connecting to the camera…'));
  document.body.dataset.camWait='';const got=()=>{delete document.body.dataset.camWait;wait.remove();};img.addEventListener('load',got,{once:true});
  img.addEventListener('error',()=>{wait.lastChild.textContent='The camera is not answering. Still trying.';},{once:true});z.append(img);box.append(z,wait);$('nx-backdrop').after(box);camZoom(box);document.body.dataset.bg='camera';}
 catch{box?.remove();}finally{camBusy=false;}}
// ---------- maintenance due (checked every ten minutes) ----------
let hoursAt=0;function paintMaintenance(){if(performance.now()-hoursAt<600000&&hoursAt)return;hoursAt=performance.now();printHours().then(r=>{state.printHours=r.h;adoptJobs(r.h);});}

// ---------- boot ----------
// F06 (2026-10-19): a page opened for a printer that is not on this farm (a link kept of one that was removed, or its first-printer page after it was taken away) was a dead
// screen: dials with no reading, "Connecting to the printer…" for ever and a message that read "GET /api/printers lists them". It says what happened and the way to the farm.
async function goneCard(){await loadSprite();document.body.replaceChildren();const c=el('main','nx-gone nx-glass');
 c.append(el('h2','','This printer is not on this farm'),el('p','nx-note','It may have been removed, or this is an old link. The farm shows every printer there is.'),
  button({icon:'i-nav-workspace',label:'Open the farm',kind:'primary',size:'row',title:'Open the farm',does:'Shows every printer in the farm.',onPress:()=>{location.href='/farm';}}));
 document.body.append(c);document.title='Stella Scope';}
(async()=>{
 if(window.__stellaPrinter&&(await fetch('/api/meta',{cache:'no-store'}).then(r=>r.status).catch(()=>0))===404){await goneCard();return;}
 await loadSprite();initTips();applyLayout();addEventListener('resize',applyLayout);buildInstruments();if(gaugesFolded)foldGauges(true,true);
 const noScene=hash.has('noscene')||!show3d();if(noScene){$('nx-scene').hidden=true;document.body.dataset.scene='off';$('nx-loading')?.remove();stageCard();}
 if(!noScene){try{scene=createScene($('nx-scene'),id=>openPart(id),{stage:'next'});}catch(e){toast('3D view unavailable: '+e.message,'alarm');}}   // 2026-10-24: a click on a part opens its panel again (it grew the machine since U02, which he never wanted)
 initStage({scene:()=>scene,shift:()=>stageShift});
 initPages({state,send,lockControls,can,why,jsend,has,applyMaterial,openPage:n=>openNamedSheet(n),caps:()=>caps,scene:()=>scene,files:sheetFiles,rewindTo:ts=>{if(finite(ts)===null)return;state.live=false;state.playing=false;closeSheet();request(ts);},tick:()=>lastTick,probes:()=>[...probeStore.values()].sort((a,b)=>a.ts-b.ts),activity:()=>state.live?lastTick?.activity:null,toolpath:()=>tp,
  savePresets:async body=>{await jsend('/api/presets','PUT',body);await loadPresets();render();},meshOnPrinter:v=>{if(v!==undefined)setMeshOn(v);return meshOn;},camPinned:v=>{if(v!==undefined)setCamPinned(v);return camPinned;}});initKiosk();
  fetch('/api/meta').then(r=>r.json()).then(m=>{const p=m.printer||{};if(p.name){state.printerName=p.name;$('nx-id-name').textContent=p.name.toUpperCase();$('nx-id-name').title=p.name;$('nx-id-name').toggleAttribute('data-long',p.name.length>13);document.title=p.name+' · Stella Scope';}$('nx-id-model').textContent=(p.model||'').toUpperCase()+(p.driver==='demo'?'  ·  DEMO':'').toUpperCase();}).catch(()=>{});
 initSwitcher({openPrinters:()=>openNamedSheet('printers')});
 // farm pop-ups on a printer's page too: a print ending elsewhere is never missed (2026-10-13)
 // U07 (2026-10-20): the failure alarm's Watch key shows this printer's live camera as the background (or goes to the printer the alarm is about)
 if(!replayMode())import('./next-fleet.js').then(m=>m.startPops(undefined,{onWatch:e=>{const mine=(/^\/p\/([a-z0-9-]{1,32})(?:\/|$)/.exec(location.pathname)||[])[1],here=mine?mine===e.printer:(!!e.name&&e.name===state.printerName);if(here)setCamPinned(true);else location.href='/p/'+e.printer;}})).catch(()=>{});
 fetch('/api/capabilities').then(r=>r.json()).then(c=>{state.desktopAccess=c.desktop===true;if(state.desktopAccess){phoneBadge();setInterval(()=>{if(!document.hidden)phoneBadge();},5000);}}).catch(()=>{});
 openSse();requestAnimationFrame(tickFrame);document.addEventListener('stella-geometry-ready',()=>whenLoaded(),{once:true});whenLoaded();loadPresets().then(render);controls(true).then(render);request(state.live?null:state.cursor);
 setInterval(()=>{if(!document.hidden)controls().then(()=>lockControls());},8000);
 render();
})();
