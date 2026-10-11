// Stella Scope HUD. The printer is the screen; everything else is an overlay, a drawer or a modal.
// Data and scene come from service.js (state, ticks, recording cursor) and hud-scene.js.
import {preflight,installationAge,instanceRecordId,partWiring,partSignals,formatSignal,volumetricFlow,flowCeiling,printerLink,preheatMenu,presetDiff,linkAllows,controlsSummary} from './hud-core.js';
import {meshSurface,meshColor,lineChart,psdChart} from './hud-graphs.js';
import {safeUrl} from './service-core.js';
const $=id=>document.getElementById(id);
const node=(tag,cls='',text='')=>{const e=document.createElement(tag);if(cls)e.className=cls;if(text)e.textContent=text;return e;};
const button=(text,fn,cls='')=>{const e=node('button',cls,text);e.type='button';e.onclick=fn;return e;};
const svg=(tag)=>document.createElementNS('http://www.w3.org/2000/svg',tag);
const known=v=>typeof v==='number'&&Number.isFinite(v);
const number=(v,suffix='',digits=1)=>known(v)?v.toFixed(digits)+suffix:'—';
const clamp=(v,a,b)=>Math.max(a,Math.min(b,v));
// Numbers roll to a new reading over 650 ms (cubic ease-out, the card dials' timing) instead of
// snapping. Unknown is set at once and is never rolled through zero. One helper for every number on
// the HUD, so they all move alike. Frame budget, measured 2026-09-23: the printer scene takes about 13
// ms of every 16.7 ms frame while it moves, so a number rewritten on every frame dropped frames all
// through a print (playback: 16.8 ms mean without rolling, 18 to 22 with it). A roll now steps about
// nine times a second, which still reads as a count, and a write that would not change the text is
// skipped.
const rolling=new WeakMap(),ROLL_STEP_MS=110;
function roll(el,v,fmt,instant=false){const st=rolling.get(el)||{};rolling.set(el,st);cancelAnimationFrame(st.raf);
 const put=x=>{const s=fmt(x);if(el.textContent!==s)el.textContent=s;};
 if(!known(v)){st.shown=null;put(null);return;}const from=known(st.shown)?st.shown:v;
 if(instant||from===v||matchMedia('(prefers-reduced-motion: reduce)').matches){st.shown=v;put(v);return;}
 const t0=performance.now();let last=0;const step=now=>{const k=Math.min(1,(now-t0)/650);if(k<1&&now-last<ROLL_STEP_MS){st.raf=requestAnimationFrame(step);return;}last=now;st.shown=from+(v-from)*(1-(1-k)**3);put(st.shown);if(k<1)st.raf=requestAnimationFrame(step);};st.raf=requestAnimationFrame(step);}

// What each card dial and readout is, for its tooltip. They are read only; the HUD's own gauges
// and dialogs are where values are set.
const READ_ABOUT={NOZZLE:['Nozzle temperature','Measured at the hotend thermistor; the notch marks the target. Read only here: the NOZZLE gauge sets it.'],BED:['Bed temperature','Measured at the bed thermistor; the notch marks the target. Read only here: the BED gauge sets it.'],
 TARGET:['Heater target','The temperature Klipper is holding. Read only.'],'HEATER OUTPUT':['Heater output','The duty Klipper commands, not measured watts. Read only.'],'RELAY DUTY':['Bed relay duty','How much of the time the bed heater output holds the relay on. Read only.'],
 FAN:['Fan command','The output Klipper commands, in percent. Read only.'],TACHOMETER:['Fan tachometer','Measured RPM where a tach wire exists; otherwise unknown. Read only.'],
 'FILAMENT FEED':['Filament feed','Live extruder velocity, mm/s. Read only.'],'E POSITION':['Extruder position','Filament moved since the last reset, mm. Read only.'],'DRIVER CURRENT':['Driver current','The TMC2209 run current, with the hold current below. Read only.'],DRIVER:['Driver state','Standstill or stepping, from the TMC2209 status. Read only.'],'PRESSURE ADVANCE':['Pressure advance','Klipper\'s pressure advance and smooth time, seconds. Read only here: Tune sets it.'],EXTRUSION:['Cold extrusion guard','Whether Klipper allows extrusion at the nozzle temperature now. Read only.'],
 'LAST Z RESULT':['Last probe Z','Z of the last probe landing, mm. Read only.'],'LAST PROBE AT':['Last probe position','Where the last probe landed. Read only.'],
 'MCU AWAKE':['MCU awake time','Share of time the microcontroller is busy. Read only.'],RETRANSMITS:['Retransmitted bytes','Bytes resent on the link since the last restart; a fast rise means a noisy link. Read only.'],'ROUND TRIP':['Link round trip','Smoothed round-trip time to the MCU, ms. Read only.'],FIRMWARE:['MCU firmware','The Klipper version this MCU runs. Read only.'],
 'RUN CURRENT':['Run current','TMC2209 run current, amps. Read only.'],'HOLD CURRENT':['Hold current','TMC2209 current at standstill, amps. Read only.'],'DRIVER STATUS':['Driver status bits','TMC2209 drv_status flags. Read only.'],
 CPU:['Host CPU temperature','The host CPU temperature, from Moonraker. Read only.'],'HOST SAMPLE':['Host sample time','When Moonraker last reported host statistics. Read only.'],
 RANGE:['Mesh range','Highest minus lowest probed point, mm. Read only.'],'LOW / HIGH':['Mesh low and high','Lowest and highest points from the mesh mean, mm. Read only.'],'WORST CORNER':['Worst corner','The corner furthest from the mesh mean. Read only.'],PROFILE:['Mesh profile','The bed mesh profile name. Read only.']};
const fansAt=p=>p?.telemetry?.fans||Object.entries(p?.fans||{}).map(([object,fan])=>({object,command:fan.speed,rpm:fan.rpm}));
const CALIBRATIONS=[
 ['home','Home XYZ','G28. Establishes the coordinate reference.'],
 ['qgl','Quad gantry level','QUAD_GANTRY_LEVEL. Four-corner probe and correction.'],
 ['mesh','Bed mesh','BED_MESH_CALIBRATE. Probes the configured mesh.'],
 ['probe_accuracy','Probe accuracy','PROBE_ACCURACY. Repeated landings at one point.'],
 ['resonance_x','Resonance X','TEST_RESONANCES AXIS=X with the toolhead accelerometer.'],
 ['resonance_y','Resonance Y','TEST_RESONANCES AXIS=Y.'],
 ['shaper_x','Shaper X','SHAPER_CALIBRATE AXIS=X. Does not save config.'],
 ['shaper_y','Shaper Y','SHAPER_CALIBRATE AXIS=Y.'],
 ['motors_off','Motors off','M84. Lets you move the gantry by hand.'],
 ['position','Query position','GET_POSITION.'],
 ['probe_query','Query probe','QUERY_PROBE.']];

export function mountHUD({getState,getScene,route,diagnose,partsAt,openDetail,editPart}){
 const main=$('page-machine'),cockpit=main.querySelector('.cockpit'),viewer=main.querySelector('.viewer'),canvasWrap=viewer.querySelector('.canvas-wrap');
 document.body.classList.add('hud-app');
 // Overlays live on the viewer, not inside the clipped canvas box, so that on a phone the canvas can
 // keep the top of the screen and the instruments can flow underneath it instead of being cut off.
 const overlay=(cls)=>{const e=node('div','hud '+cls);viewer.append(e);return e;};

 // ---------- modal system (K10) ----------
 // One stack. Click outside closes. Escape closes. Open several and switch between them.
 const modalStack=[];
 function showModal(entry){const open=modalStack.find(m=>m.dialog.open);if(open&&open!==entry)open.dialog.close();if(!entry.dialog.open)entry.dialog.showModal();drawChips();}
 function modal(id,title,build,{wide=false}={}){
  const existing=modalStack.find(m=>m.id===id);if(existing){focusModal(existing);return existing;}
  const dialog=node('dialog','hud-modal'+(wide?' wide':''));dialog.dataset.id=id;
  const inner=node('div','hud-modal-inner');
  const head=node('div','hud-modal-head');const chips=node('div','hud-modal-chips');
  const titleEl=node('h2','',title);titleEl.id='hud-modal-title-'+id;dialog.setAttribute('aria-labelledby',titleEl.id);
  head.append(titleEl,chips,tip(button('✕',()=>closeModal(entry),'hud-modal-close'),'Close','Closes this window. Nothing is sent.'));
  const body=node('div','hud-modal-body');inner.append(head,body);dialog.append(inner);document.body.append(dialog);
  const entry={id,dialog,body,chips,title,refresh:null,onClose:null};
  let drag=null;head.addEventListener('pointerdown',e=>{if(e.button!==0||e.target.closest('button'))return;const r=inner.getBoundingClientRect();inner.style.transform='none';inner.style.left=r.left+'px';inner.style.top=r.top+'px';drag={x:e.clientX-r.left,y:e.clientY-r.top};head.setPointerCapture(e.pointerId);});
  head.addEventListener('pointermove',e=>{if(!drag)return;const r=inner.getBoundingClientRect();inner.style.left=clamp(e.clientX-drag.x,8,Math.max(8,innerWidth-r.width-8))+'px';inner.style.top=clamp(e.clientY-drag.y,8,Math.max(8,innerHeight-r.height-8))+'px';});
  head.addEventListener('pointerup',()=>{drag=null;});head.addEventListener('pointercancel',()=>{drag=null;});
  dialog.addEventListener('click',e=>{if(e.target===dialog)closeModal(entry);});
  dialog.addEventListener('cancel',e=>{e.preventDefault();closeModal(entry);});
  // A dialog's printer controls are locked the moment it is built, and again once an async build
  // (a file list, a macro list) has finished drawing them.
  modalStack.push(entry);entry.refresh=()=>{body.replaceChildren();const built=build(body,entry);lockControls(dialog);built?.then?.(()=>lockControls(dialog));};
  entry.refresh();showModal(entry);return entry;
 }
 function closeModal(entry){if(entry.busy){feedback('A printer command is in flight. Wait for its reported outcome.','pending');return;}if(entry.dialog.open)entry.dialog.close();entry.dialog.remove();const i=modalStack.indexOf(entry);if(i>=0)modalStack.splice(i,1);
  entry.onClose?.();const top=modalStack[modalStack.length-1];if(top)showModal(top);}
 function focusModal(entry){const busy=modalStack.find(m=>m.busy);if(busy&&busy!==entry){feedback('A printer command is in flight. Wait for its reported outcome.','pending');return;}const i=modalStack.indexOf(entry);if(i>=0){modalStack.splice(i,1);modalStack.push(entry);}showModal(entry);}
 function drawChips(){for(const m of modalStack){m.chips.replaceChildren();if(modalStack.length<2)continue;
  for(const other of modalStack){const b=tip(button(other.title,()=>focusModal(other),'hud-chip'),other.title,other===m?'The window in front.':'Brings this open window to the front.');b.classList.toggle('on',other===m);m.chips.append(b);}}}
 function closeOnBackdrop(dialog,close=()=>dialog.close()){dialog.addEventListener('click',e=>{const r=dialog.getBoundingClientRect();if(e.target===dialog&&(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom))close();});}

 // ---------- toast + controls client ----------
 const toastEl=overlay('hud-toast');toastEl.setAttribute('role','status');let toastTimer=0;
 function toast(text){toastEl.textContent=text;toastEl.dataset.on='1';clearTimeout(toastTimer);toastTimer=setTimeout(()=>{toastEl.dataset.on='';},4200);}
 function feedback(text,state=''){
  const host=modalStack.findLast(m=>m.dialog.open)?.body||[...document.querySelectorAll('dialog[open]')].at(-1);
  let out=host?[...host.querySelectorAll('.hud-command-feedback,.hud-command-status')].at(-1):null;
  if(!out&&host){out=node('p','hud-command-feedback');out.setAttribute('role','status');host.append(out);}
  if(out){out.textContent=text;out.dataset.state=state;}else toast(text);
 }
 // ---------- tooltips ---------- One glass tooltip for the whole HUD: what the control is, what
 // clicking it does and, when it cannot be used right now, the service's own reason. The text lives on
 // the control (data-tip, data-tip-do, data-tip-cmd). A plain title is taken over on first hover so
 // the browser's tooltip never doubles it. Inside an open dialog the tooltip moves into that dialog,
 // or the dialog's top layer would hide it.
 const tipEl=node('div','hud-tip');tipEl.id='hud-tip';tipEl.setAttribute('role','tooltip');let tipAt=null,tipTimer=0;
 function tip(el,what,does='',cmd=''){if(!el)return el;el.dataset.tip=what;if(does)el.dataset.tipDo=does;else delete el.dataset.tipDo;if(cmd)el.dataset.tipCmd=cmd;
  // A control whose only name was its title keeps one when the title is taken over.
  if(!el.hasAttribute('aria-label')&&!el.textContent.trim())el.setAttribute('aria-label',what);el.removeAttribute('title');return el;}
 function showTip(el){if(el.hasAttribute('title'))tip(el,el.getAttribute('title'),el.dataset.tipDo||'');if(!el.dataset.tip||!el.isConnected)return;
  const host=el.closest('dialog[open]')||document.body;if(tipEl.parentNode!==host)host.append(tipEl);
  tipEl.replaceChildren(node('strong','',el.dataset.tip));if(el.dataset.tipDo)tipEl.append(node('span','',el.dataset.tipDo));const c=el.dataset.tipCmd;if(c&&!can(c))tipEl.append(node('em','',why(c)));
  tipEl.style.left='0px';tipEl.style.top='0px';tipEl.dataset.on='1';el.setAttribute('aria-describedby','hud-tip');
  const o=tipEl.getBoundingClientRect(),r=el.getBoundingClientRect();let x=r.left+r.width/2-o.width/2,y=r.top-o.height-8;if(y<6)y=r.bottom+8;
  tipEl.style.left=(clamp(x,6,innerWidth-o.width-6)-o.left)+'px';tipEl.style.top=(clamp(y,6,innerHeight-o.height-6)-o.top)+'px';}
 function hideTip(){clearTimeout(tipTimer);tipEl.dataset.on='';tipAt?.removeAttribute('aria-describedby');tipAt=null;}
 const tipTarget=e=>e.target?.closest?.('[data-tip],[title]');
 // What a printer button does, said once for every button that goes through send().
 const SENDS='Sends it to the printer. The exact G-code is shown first unless it is on your no-review list.';
 document.addEventListener('pointerover',e=>{let el=tipTarget(e);if(el?.dataset.tipFocusOnly)el=null;if(el===tipAt)return;hideTip();if(!el)return;tipAt=el;tipTimer=setTimeout(()=>showTip(el),350);});
 document.addEventListener('focusin',e=>{const el=tipTarget(e);hideTip();if(el?.matches(':focus-visible')){tipAt=el;showTip(el);}});
 document.addEventListener('pointerout',e=>{if(!e.relatedTarget)hideTip();});
 for(const ev of ['pointerdown','wheel','keydown'])document.addEventListener(ev,hideTip,true);
 document.body.append(tipEl);
 let caps={enabled:false,reason:'Checking printer controls…',commands:[]},capsAt=-1e9;
 async function controls(force=false){
  if(!force&&performance.now()-capsAt<8000)return caps;
  try{const r=await fetch('/api/controls');if(!r.ok)throw Error('Printer commands are not enabled in this build.');caps=await r.json();
   if(getState().snapshot?.observer_now?.mode==='replay')caps.reason='Saved recording. Printer commands are unavailable in replay.';
   else if(caps.reason==='controls_enabled is false')caps.reason='Printer controls are off: set controls_enabled in stella-scope.json.';}
  catch(e){caps={enabled:false,reason:e.message,commands:[]};}
  capsAt=performance.now();document.body.dataset.controls=caps.enabled?'on':'off';
  // Both emergency stops, the header one and the toolbar's, carry the same tooltip. Whether they
  // can be pressed is lockControls' call, the same as for every other printer control.
  for(const b of document.querySelectorAll('.hud-emergency,.hud-estop'))tip(b,'Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call, with no review. Heaters and motors cut out, and the printer then needs a firmware restart.','emergency_stop');
  paintControls();lockControls();return caps;
 }
 // The printer's state comes from the collector's own link, the newest observer service.js holds
 // (state.link), so a printer that is off locks every control at once and nothing here polls the
 // printer to find out.
 function link(){const s=getState();return printerLink(s.link,{replay:s.snapshot?.observer_now?.mode==='replay'||s.link?.mode==='replay',lastSeen:s.link?.last_success??s.snapshot?.last_ts});}
 // The file list, the queue and the macros are the printer's: not asked for while it cannot answer.
 function noPrinter(){return ['off','replay'].includes(link().state);}
 function can(id){const l=link();if(!linkAllows(l,id))return false;return caps.enabled&&(id==='emergency_stop'||getState().live)&&caps.commands?.some(c=>c.id===id&&c.allowed);}
 function why(id){const l=link();if(!linkAllows(l,id))return l.reason;if(id!=='emergency_stop'&&!getState().live)return 'Go to Live first. Commands never run against a recording cursor.';return (caps.enabled?caps.commands?.find(c=>c.id===id)?.reason:null)||caps.reason||'Unavailable';}
 // Every control that sends to the printer names its command in data-tip-cmd. While that command
 // cannot run it is disabled (a bar that is not a button says so with aria-disabled), and its
 // tooltip's third line gives the reason. A control disabled for a reason of its own (an empty field,
 // an object already excluded) is left as it is: only what this function locked it unlocks. the one
 // writer of the header badge and the line under the name. It reads the same state as the pill and
 // the locks (controlsSummary over printerLink), so the badge says what is true: live controls,
 // printer off, Klipper not ready, rewound, replay or service not running.
 function paintControls(l=link()){const c=controlsSummary(l,{live:getState().live,enabled:caps.enabled}),badge=document.querySelector('.header-actions .badge');if(badge){badge.textContent=c.badge;badge.dataset.controls=c.key;}readiness.textContent=c.line;}
 function lockControls(root=document){for(const el of root.querySelectorAll('[data-tip-cmd]')){const off=!can(el.dataset.tipCmd);
  if('disabled' in el){if(off){if(!el.disabled){el.disabled=true;el.dataset.locked='1';}}else if(el.dataset.locked){el.disabled=false;delete el.dataset.locked;}}
  else if(off)el.setAttribute('aria-disabled','true');else el.removeAttribute('aria-disabled');}}
 async function execute(id,extra,confirm,status){
  const c=await controls(true);
  const report=(text,state='')=>status?(status.textContent=text,status.dataset.state=state):feedback(text,state);
  if(id!=='emergency_stop'&&!getState().live){report('Go to Live first. Commands never run against a recording cursor.','refused');return null;}
  if(!can(id)){report(why(id),'refused');return null;}
  try{const r=await fetch('/api/commands',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token||''},body:JSON.stringify({id,confirm,...extra})});
   const result=await r.json();if(!r.ok)throw Error(result.error||'Command refused without a reported reason.');report(result.message,'reported');return result;}
  catch(e){report(e.message+' No automatic retry.','refused');return null;}
 }
 async function send(id,extra={}){
  // Nothing is asked of the service, not even a preview, for a control that cannot run now. found
  // replay controls still POSTing previews while their tooltips said they were off.
  if(!link().lock&&!can(id))await controls(true);
  if(!can(id)){feedback(why(id),'refused');return null;}
  if(id==='emergency_stop')return execute(id,extra,false);
  const c=await controls(true);let preview;
  try{const r=await fetch('/api/command-preview',{method:'POST',headers:{'Content-Type':'application/json',...(c.token?{'X-Stella-Control-Token':c.token}:{})},body:JSON.stringify({id,...extra})});
   preview=await r.json();if(!r.ok)throw Error(preview.error||'Exact command preview is unavailable.');}
  catch(e){feedback(e.message,'refused');return null;}
  const previous=modalStack.find(m=>m.id==='command-review');if(previous?.busy){focusModal(previous);feedback('A printer command is already in flight. No second command was started.','pending');return null;}if(previous)closeModal(previous);
  // A gauge is a control. Heaters, fans and factors (T2) go straight out and the exact G-code is
  // reported; only motion and restarts (T3) stop for a review. HOME and QGL come back with
  // review:false. They keep every T3 safety gate on the service side, they just do not stop for a
  // dialog first, so the button homes the printer.
  if(preview.tier!=='T3'||preview.review===false){const script=(preview.script||preview.action||'').toString().trim();feedback(script?`Sending ${script}`:'Sending…','pending');const result=await execute(id,extra,preview.tier==='T3');if(result)feedback(`${script||id} · ${result.message}`,'reported');return result;}
  return new Promise(resolve=>{const entry=modal('command-review','Review printer command',(body)=>{
   const status=node('p','hud-command-feedback');status.setAttribute('role','status');
   const cancel=tip(button('Cancel',()=>closeModal(entry)),'Cancel','Closes this review. Nothing is sent.');const reason=!getState().live?'Go to Live first. Commands never run against a recording cursor.':can(id)?'Review the exact G-code, then confirm once.':why(id);
   const emergency=button('EMERGENCY STOP',async()=>{emergency.disabled=true;await execute('emergency_stop',{},false);emergency.dataset.locked='1';lockControls();},'hud-danger');tip(emergency,'Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call, with no review.','emergency_stop');
   const run=button('Confirm and send once',async()=>{run.disabled=cancel.disabled=true;entry.busy=true;status.textContent='Sending once…';status.dataset.state='pending';
    const result=await execute(id,extra,preview.tier==='T3',status);entry.busy=false;cancel.disabled=false;if(result){entry.onClose=null;closeModal(entry);feedback(`${result.message}${result.audit_id?' Audit '+result.audit_id:''}`,'reported');resolve(result);}else run.disabled=!getState().live||!can(id);},'hud-primary');
   run.disabled=!getState().live||!can(id);tip(run,'Confirm and send once','Sends the G-code shown here to the printer one time. There is no automatic retry.',id);status.textContent=reason;status.dataset.state=run.disabled?'refused':'';
   body.append(node('p','hud-modal-note',id.replaceAll('_',' ')),commandPreview(preview.script||preview.action),status,row('',run,cancel,emergency));
  });entry.onClose=()=>resolve(null);});
 }
 const stop=button('STOP',async()=>{stop.disabled=true;await send('emergency_stop');stop.dataset.locked='1';lockControls();},'hud-emergency');tip(stop,'Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call, with no review.','emergency_stop');
 document.querySelector('.header-actions').append(stop);

 // ---------- chiclet gauge: click where you want it ----------
 function gauge({label,unit='',min=0,max,segments=24,digits=0,kind='thermal',vertical=false,settable=null,presets=null,open=null,fan=null,heat=null,about=''}){
  const el=node('div','hud-gauge'+(vertical?' vertical':''));el.dataset.kind=kind;if(fan)el.dataset.fan=fan;if(heat)el.dataset.heat=heat;
  const head=node('div','hud-gauge-head'),name=node('span','hud-gauge-label',label),value=node('button','hud-gauge-value','—');value.type='button';value.setAttribute('aria-label',label+' control');
  head.append(name,value);const track=node('div','hud-gauge-track');track.setAttribute('role','button');track.setAttribute('aria-label',label+' control');track.tabIndex=0;
  const cells=[];for(let i=0;i<segments;i++){const c=node('i');cells.push(c);track.append(c);}
  const note=node('small','hud-gauge-note');el.append(head,track,note);
  let current=null,target=null,pending=null,holdUntil=0,hold=null;
  const level=v=>clamp((v-min)/(max-min),0,1);
  // After a set, show the commanded value until the printer reports it (or 4 s pass), so the bar
  // does not snap back to the stale reading between ticks. That was the lag the user saw on the part
  // fan.
  function paint(){const held=hold!==null&&performance.now()<holdUntil&&(!known(current)||Math.abs(current-hold)>(max-min)/segments)?hold:null;const shown=pending??held??current;const lv=known(shown)?level(shown):0,tg=known(target)?level(target):null;
   el.style.setProperty('--level',lv);cells.forEach((c,i)=>{const at=(i+.5)/segments;c.classList.toggle('on',known(shown)&&at<=lv);c.classList.toggle('target',tg!==null&&Math.abs(at-tg)<=.5/segments+1e-6);});
   // The number rolls to each new reading; a value being dragged in is shown as it is.
   if(pending!==null)roll(value,pending,v=>v.toFixed(digits)+unit,true);else roll(value,current,v=>known(v)?v.toFixed(digits)+unit:'—');value.classList.toggle('pending',pending!==null);}
  function at(e){const r=track.getBoundingClientRect();const f=vertical?1-(e.clientY-r.top)/r.height:(e.clientX-r.left)/r.width;return min+clamp(f,0,1)*(max-min);}
  const snap=v=>Number((Math.round(v/((max-min)/segments))*((max-min)/segments)).toFixed(digits));
  if(settable){
   track.addEventListener('pointerdown',e=>{if(e.button!==0)return;if(!can(settable.id)){toast(why(settable.id));return;}track.setPointerCapture(e.pointerId);pending=snap(at(e));paint();});
   track.addEventListener('pointermove',e=>{if(pending===null||!track.hasPointerCapture(e.pointerId))return;pending=snap(at(e));paint();});
   const commit=async()=>{if(pending===null)return;const v=pending;pending=null;target=v;hold=v;holdUntil=performance.now()+4000;paint();await settable.set(v);};
   track.addEventListener('pointerup',commit);track.addEventListener('pointercancel',()=>{pending=null;paint();});
   track.addEventListener('keydown',e=>{if((e.key==='Enter'||e.key===' ')&&settable.open){e.preventDefault();settable.open();}});
   value.onclick=()=>settable.open?settable.open():null;
   name.onclick=()=>settable.open?settable.open():null;name.style.cursor='pointer';
   if(presets){const row=node('div','hud-presets');for(const p of presets)row.append(button(p+unit,()=>{if(!can(settable.id)){toast(why(settable.id));return;}target=p;hold=p;holdUntil=performance.now()+4000;paint();settable.set(p);}));el.append(row);}
  }else{track.classList.add('readonly');if(open){value.onclick=open;name.onclick=open;name.style.cursor='pointer';track.addEventListener('click',open);}else value.disabled=true;}
  // Every gauge says what it is and what a click does.
  const what=about||label;
  // The number and the name open the dialog, which is looking and stays open to everyone; only the
  // bar sets, so only the bar is a printer control.
  if(settable){tip(value,what,'Click for the exact value and the presets.');tip(track,what,'Click or drag along the bar to set it. It goes out through the same send path as every printer command.',settable.id);tip(name,what,'Click for the exact value and the presets.');}
  else if(open){tip(value,what,'Click for details.');tip(track,what,'Click for details. Read only: nothing here changes the printer.');tip(name,what,'Click for details.');}else tip(value,what,'Read only.');
  return {el,set(v,t,text=''){current=known(v)?v:null;if(known(t))target=t;else if(hold===null)target=null;if(hold!==null&&known(current)&&Math.abs(current-hold)<=(max-min)/segments)hold=null;note.textContent=text;paint();}};
 }
 // ---------- fan glyph ----------
 function fanGlyph(){const s=svg('svg');s.setAttribute('viewBox','0 0 40 40');s.classList.add('hud-fan');const g=svg('g');g.setAttribute('transform-origin','20 20');
  for(let i=0;i<5;i++){const b=svg('path');b.setAttribute('d','M20 20 C 24 12, 30 8, 34 12 C 30 18, 26 20, 20 20 Z');b.setAttribute('transform',`rotate(${i*72} 20 20)`);g.append(b);}
  const hub=svg('circle');hub.setAttribute('cx',20);hub.setAttribute('cy',20);hub.setAttribute('r',4);s.append(g,hub);
  return {el:s,set(f){const on=known(f)&&f>0;s.dataset.on=on?'1':'';s.style.setProperty('--period',on?(2.4/Math.max(.15,f)).toFixed(2)+'s':'0s');s.style.setProperty('--fan',on?f:0);}};}

 // ---------- heading (top-left) ----------
 const heading=overlay('hud-heading');
 // One block, two lines: identity, then state. It must clear the 108 px header band.
 const ident=node('div','hud-ident'),machineName=node('span','hud-machine-name','PRINTER'),machineSpec=node('span','hud-machine-spec','BED SIZE UNVERIFIED');ident.append(machineName,machineSpec);heading.append(ident);
 const stateRow=node('div','hud-state-row');const statePill=node('span','hud-state-pill','—'),fresh=node('span','hud-fresh',''),readiness=node('span','hud-readiness');const loadedChip=node('span','hud-loaded','');stateRow.append(statePill,fresh,readiness,loadedChip);heading.append(stateRow);

 // ---------- offline banner ----------
 const offline=overlay('hud-offline');
 const offlineText=node('span','','');offline.append(node('strong','','PRINTER OFFLINE'),offlineText);

 // ---------- job strip (top-centre) ----------
 const jobStrip=overlay('hud-jobstrip');const jobName=node('div','hud-job-name','No job reported'),jobMeta=node('div','hud-job-meta','');
 const jobBar=node('div','hud-job-bar');for(let i=0;i<40;i++)jobBar.append(node('i'));jobStrip.append(jobName,jobBar,jobMeta);
 jobStrip.onclick=()=>route('timeline');jobStrip.setAttribute('role','button');jobStrip.tabIndex=0;jobStrip.onkeydown=e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();route('timeline');}};
 tip(jobStrip,'Current print','The file, its progress and the time left. Click for the Timeline: everything the printer said, in order.');

 // ---------- view controls (top-right) ----------
 const viewBox=overlay('hud-viewbox');
 const cube=svg('svg');cube.setAttribute('viewBox','0 0 128 120');cube.classList.add('hud-view-cube');cube.setAttribute('aria-label','View cube. Click a face or drag to turn.');
 const faces=new Map();for(const name of ['top','bottom','front','rear','left','right']){const group=svg('g'),face=svg('polygon'),label=svg('text');group.dataset.view=name;group.setAttribute('role','button');group.setAttribute('tabindex','0');label.textContent=name==='rear'?'BACK':name.toUpperCase();tip(group,label.textContent.charAt(0)+label.textContent.slice(1).toLowerCase()+' view','Click to look at the printer from the '+(name==='rear'?'back':name)+'. Drag the cube to turn the view freely.');group.append(face,label);cube.append(group);faces.set(name,{group,face,label});group.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();getScene()?.view(name);}});}
 // Each label is drawn in its own face's plane (orientation hands over the matrix) and fades out
 // as that face turns edge-on, so FRONT can no longer sit on LEFT or RIGHT.
 function drawCube(){for(const f of getScene()?.orientation()||[]){const {group,face,label}=faces.get(f.name);group.style.display=f.visible?'':'none';face.setAttribute('points',f.points.map(p=>p.join(',')).join(' '));label.setAttribute('transform',`matrix(${f.label.map(v=>v.toFixed(3)).join(' ')})`);label.style.opacity=String(Math.max(0,Math.min(1,(f.facing-.15)/.25)).toFixed(2));}}
 let cubeDrag=null;cube.addEventListener('pointerdown',e=>{if(e.button!==0)return;cubeDrag={x:e.clientX,y:e.clientY,total:0,face:e.target.closest('[data-view]')?.dataset.view};cube.setPointerCapture(e.pointerId);e.preventDefault();});
 cube.addEventListener('pointermove',e=>{if(!cubeDrag)return;const dx=e.clientX-cubeDrag.x,dy=e.clientY-cubeDrag.y;cubeDrag.total+=Math.abs(dx)+Math.abs(dy);cubeDrag.x=e.clientX;cubeDrag.y=e.clientY;getScene()?.orbitBy(dx,dy);});
 cube.addEventListener('pointerup',()=>{if(cubeDrag?.total<5&&cubeDrag.face)getScene()?.view(cubeDrag.face);cubeDrag=null;});cube.addEventListener('pointercancel',()=>{cubeDrag=null;});
 document.addEventListener('stella-camera-change',drawCube);
 const zoomRow=node('div','hud-zoom');for(const [t,label,fn,does]of [['+','Zoom in',()=>getScene()?.zoom(.75),'Moves the camera closer.'],['−','Zoom out',()=>getScene()?.zoom(1.33),'Moves the camera away.'],['⌂','Whole printer',()=>{getScene()?.reset();$('explode').value='0';},'Puts the camera back on the whole printer.']]){const b=button(t,fn);tip(b,label,does);b.setAttribute('aria-label',label);zoomRow.append(b);}
 let panelsOn=false;
 // Light and dark, remembered per browser. Dark is neon-lit rather than grey.
 const app=document.body;
 const savedTheme=(()=>{try{return localStorage.getItem('stella-theme');}catch{return null;}})();
 if(savedTheme)app.dataset.theme=savedTheme;
 const themeButton=button(app.dataset.theme==='light'?'Dark':'Light',()=>{
  const next=app.dataset.theme==='light'?'':'light';
  if(next)app.dataset.theme=next;else delete app.dataset.theme;
  themeButton.textContent=next==='light'?'Dark':'Light';
  try{localStorage.setItem('stella-theme',next);}catch{}
  getScene()?.theme(next==='light'?'light':'dark');});
 const headerTools=node('div','hud-header-tools');
 // The old "Tools" button opened Signals without saying so; it is now Pages, first in the row, and
 // opens the list of every page.
 const pagesButton=button('Pages',()=>openPages(),'hud-pages-button'),fullscreenButton=$('fullscreen');
 headerTools.append(pagesButton,themeButton,button('Colors',()=>openColors()),button('Panels',()=>{panelsOn=!panelsOn;getScene()?.panels(panelsOn);}),fullscreenButton);
 for(const [el,what,does]of [[pagesButton,'Pages','Every page in one list: Signals (every signal on one table), Electronics, Parts, Timeline, the workspace and more.'],[themeButton,'Light or dark','Switches the HUD between the light and the dark theme. This browser remembers it.'],
  [headerTools.children[2],'Colours','Opens the paint panel: colour the whole printer, a group, or any part you click.'],[headerTools.children[3],'Enclosure panels','Shows or hides the enclosure panels on the model.'],[fullscreenButton,'Full screen','Fills the screen with the HUD. Escape leaves full screen.']])tip(el,what,does);
 document.querySelector('.header-actions').prepend(headerTools);
 // Hotend and part cooling stay together on the right rail, and the controller fan "can go somewhere
 // else". It is an automatic output with nothing to set, so it is a readout beside the recorder's
 // link dot rather than a gauge: purple dot, percentage, no click.
 const ctrlFan=node('span','hud-ctrlfan');ctrlFan.append(node('i'),node('b','','—'));
 tip(ctrlFan,'Controller fan','Automatic: Klipper runs it with the steppers and heaters. The percent is its commanded output.');
 $('led-transport')?.after(ctrlFan);
 viewBox.append(cube,zoomRow);
 const selection=node('div','hud-selection');const selectionName=node('span','','Click any part');
 const zoomPart=button('Zoom',()=>{const s=getScene()?.selected;if(s)getScene().focus(s.id);}),inspectPart=button('Details',()=>{const s=getScene()?.selected;if(s)openDetail(s.id,true);});
 selection.append(selectionName,zoomPart,inspectPart);tip(zoomPart,'Zoom to the part','Frames the selected part.');tip(inspectPart,'Part details','Opens the selected part\'s card: its specifications and source, and its live signals when it has any.');
 const backChip=button('← Whole printer',()=>{getScene()?.reset();$('explode').value='0';},'hud-back');
 canvasWrap.append(backChip);backChip.classList.add('hud');backChip.hidden=true;tip(backChip,'Whole printer','Leaves the focused part and shows the whole printer again.');
 document.addEventListener('stella-inspection',e=>{backChip.hidden=e.detail.mode!=='focus';
  backChip.textContent=e.detail.mode==='focus'?'← Whole printer':'';});
 function selectionChanged(){const s=getScene()?.selected;selection.dataset.on=s?'1':'';selectionName.textContent=s?s.name:'Click any part';zoomPart.disabled=inspectPart.disabled=!s||Boolean(s.sourceOnly);}
 document.addEventListener('stella-selection-change',selectionChanged);selectionChanged();

 // ---------- toolhead reticle: the HUD follows the nozzle on screen ---------- Gone. The Z readout
 // he does want keeps the same anchor, so it still rides with the head.
 const reticle=node('div','hud hud-reticle');canvasWrap.append(reticle);
 const reticleText=node('div','hud-reticle-text');reticle.append(reticleText);
 document.addEventListener('stella-track',e=>{const {x,y,on}=e.detail;reticle.dataset.on=on?'1':'';reticle.style.transform=`translate(${Math.round(x)}px,${Math.round(y)}px)`;});

 // ---------- left rail: thermal, two upright bars side by side ----------
 const left=overlay('hud-rail left');left.append(node('div','hud-rail-title','THERMAL'));
 // Nozzle and bed carry their own colour, like the fans, so a glance at the bars says which
 // heater it is without reading the label.
 const nozzle=gauge({about:'Nozzle temperature in °C; the white segment is the target',label:'NOZZLE',unit:'°',max:290,segments:29,kind:'heat',heat:'nozzle',vertical:true,settable:{id:'set_heater',set:v=>send('set_heater',{target:'extruder',value:v}),open:()=>openHeater('nozzle')}});
 const bed=gauge({about:'Bed temperature in °C; the white segment is the target',label:'BED',unit:'°',max:120,segments:24,kind:'heat',heat:'bed',vertical:true,settable:{id:'set_heater',set:v=>send('set_heater',{target:'heater_bed',value:v}),open:()=>openHeater('bed')}});
 const leftRow=node('div','hud-rail-row');leftRow.append(nozzle.el,bed.el);left.append(leftRow);
 // ---------- right rail: cooling, part over hotend ---------- stacked, not three across, and
 // the controller fan is off the rail entirely — it reads out in the header now, beside the
 // link dot.
 const right=overlay('hud-rail right');right.append(node('div','hud-rail-title','COOLING'));
 const partFan=gauge({about:'Part cooling fan, commanded percent (the two 4010 blowers)',label:'PART',unit:'%',max:100,segments:20,kind:'fan',fan:'part',vertical:true,settable:{id:'set_fan',set:v=>send('set_fan',{value:v/100}),open:()=>openFan('fan')}});
 const partGlyph=fanGlyph();partFan.el.prepend(partGlyph.el);partFan.el.querySelector('.hud-gauge-note')?.remove();
 const fanRows=new Map();const rightRow=node('div','hud-rail-row');rightRow.append(partFan.el);
 for(const [key,label,tone]of [['hotend_fan','HOTEND','hotend']]){const g=gauge({about:'Hotend fan, commanded percent. Automatic: Klipper runs it above its trigger temperature',label,unit:'%',max:100,segments:20,kind:'fan',fan:tone,vertical:true,open:()=>openFan(key)});const glyph=fanGlyph();g.el.prepend(glyph.el);g.el.querySelector('.hud-gauge-note')?.remove();rightRow.append(g.el);fanRows.set(key,{glyph,gauge:g});}
 right.append(rightRow);

 // ---------- bottom band: motion + speed ----------
 const bottom=overlay('hud-bottom');
 const motion=node('div','hud-motion');const axes={};for(const a of ['X','Y','Z']){const cell=button('',()=>openMotion(),'hud-axis');cell.append(node('span','',a),node('strong','','—'));motion.append(cell);axes[a]=cell.querySelector('strong');tip(cell,a+' position, mm','Where the toolhead is, from Klipper\'s motion report. Click for the motion panel: jog, home and Z offset.');}
 // The velocity line that sat under X, Y and Z is now the VELOCITY readout beside them.
 const homed=node('div','hud-homed','');motion.append(homed);
 // Fill the empty space beside the X, Y, Z readouts with live motion. Readout windows: a label, a
 // rolling number with its unit, a note. Unknown is a dash, never zero.
 function readoutWindow(label,what,does=''){const el=node('div','hud-readout'),v=node('b','','—'),u=node('i'),n=node('small');el.append(node('span','',label),v,u,n);tip(el,what,does);
  return {el,set(x,{digits=0,unit='',note='',state=''}={}){n.textContent=note;u.textContent=known(x)?unit:'';el.dataset.state=state;el.dataset.unknown=known(x)?'':'1';roll(v,x,y=>known(y)?y.toFixed(digits):'—');}};}
 const velocityRead=readoutWindow('VELOCITY','Toolhead speed, mm/s','The speed the toolhead is moving at this instant, from Klipper\'s motion report. The note is the configured maximum.');
 const accelRead=readoutWindow('ACCEL LIMIT','Acceleration limit, mm/s²','The toolhead\'s current max_accel. Slicer SET_VELOCITY_LIMIT changes it during a print.');
 const volumeRead=readoutWindow('VOLUMETRIC','Volumetric flow, mm³/s','Filament speed times the 1.75 mm filament area, against the hotend ceiling you measured for the loaded material.');
 const flowBar=node('div','hud-flowbar');flowBar.append(node('u'));volumeRead.el.append(flowBar);
 const speed=gauge({about:'Speed factor, M220: scales every move the G-code asks for',label:'SPEED',unit:'%',min:50,max:200,segments:30,kind:'speed',settable:{id:'speed_factor',set:async v=>{if(await send('speed_factor',{value:v}))offerFlow(v,known(point()?.extrude_factor)?point().extrude_factor*100:null);},open:()=>openFactor('speed')},presets:[100,110,120,135]});
 const flow=gauge({about:'Flow factor, M221: scales how much filament every extrusion pushes',label:'FLOW',unit:'%',min:50,max:200,segments:30,kind:'speed',settable:{id:'extrude_factor',set:v=>send('extrude_factor',{value:v}),open:()=>openFactor('flow')},presets:[95,100,105]});
 speed.el.querySelector('.hud-gauge-head').append(velocityRead.el,accelRead.el);flow.el.querySelector('.hud-gauge-head').append(volumeRead.el);
 // Permanent motion row: never more than one click from moving the machine.
 let step=10;
 const jogRow=node('div','hud-jogrow');
 const stepChips=node('div','hud-step-chips');
 for(const st of [0.1,1,10,50]){const b=button(st+'',()=>{step=st;[...stepChips.children].forEach(x=>x.classList.toggle('on',Number(x.dataset.s)===step));});b.dataset.s=st;b.classList.toggle('on',st===step);tip(b,st+' mm jog step','Each jog arrow moves the toolhead this far.');stepChips.append(b);}
 // Arrow glyphs: on a Voron, Y+ is away from you (rear), Z+ raises the head. The letter stays for the label.
 const arrowIcon=(axis,sign)=>{const sv=svg('svg');sv.setAttribute('viewBox','0 0 24 24');sv.classList.add('hud-arrow');const p=svg('path');const d={X:sign>0?'M4 12h13m-5-5 5 5-5 5':'M20 12H7m5-5-5 5 5 5',Y:sign>0?'M12 20V7m-5 5 5-5 5 5':'M12 4v13m-5-5 5 5 5-5',Z:sign>0?'M6 16l6-6 6 6M6 10l6-6 6 6':'M6 8l6 6 6-6M6 14l6 6 6-6'}[axis];p.setAttribute('d',d);sv.append(p);return sv;};
 const axisBtn=(text,axis,sign)=>{const b=button('',async()=>{const p=point();const homed=/x/.test(p?.homed||'')&&/y/.test(p?.homed||'')&&/z/.test(p?.homed||'');
  if(!homed){openMotion();feedback('The printer is not homed. Home XYZ first.','refused');return;}await send('jog',{axis,value:sign*step});},'hud-jog axis-'+axis.toLowerCase());b.append(node('span','hud-jog-axis',axis),arrowIcon(axis,sign));b.setAttribute('aria-label',text);tip(b,axis==='Y'?(sign>0?'Y+ · toward the back':'Y− · toward you'):axis==='Z'?(sign>0?'Z+ · head up':'Z− · head down'):(sign>0?'X+ · right':'X− · left'),'Moves the toolhead by the chosen step, at once. Needs X, Y and Z homed.','jog');return b;};
 jogRow.append(selection,node('span','hud-row-label','MOVE'),stepChips,
  axisBtn('X−','X',-1),axisBtn('X+','X',1),axisBtn('Y−','Y',-1),axisBtn('Y+','Y',1),axisBtn('Z−','Z',-1),axisBtn('Z+','Z',1));
 // They are whole-machine actions, not jog keys, so they lead the actions strip, and that strip
 // now fills the black band that sat under the MOVE row.
 const actions=node('div','hud-actions');
 // Same size as the rest of the strip. hud-machine keeps the teal accent only. The controls a Voron
 // operator reaches for, in groups (machine, heat, move, work, firmware), and an emergency stop that
 // is always on the row. Every one goes through the same send as the rest of the HUD, so the service
 // decides what asks for a review: BED MESH and MOTORS OFF are on the 2026-09-21 no-review list with
 // HOME and QGL, FW RESTART and every macro ask once, heaters are T2 and go straight out, E-STOP is
 // immediate.
 const act=(label,fn,cls,what,does,cmd='')=>tip(button(label,fn,cls),what,does,cmd),sep=()=>node('i','hud-sep');
 const estop=act('E-STOP',async()=>{estop.disabled=true;await send('emergency_stop');estop.dataset.locked='1';lockControls();},'hud-estop','Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call, with no review. Heaters and motors cut out, and the printer then needs a firmware restart.','emergency_stop');
 actions.append(act('HOME',()=>send('home'),'hud-machine','Home X, Y and Z','Sends G28 at once. The toolhead moves to its endstops and the bed is probed for Z.','home'),
  act('QGL',()=>send('qgl'),'hud-machine','Quad gantry level','Sends QUAD_GANTRY_LEVEL at once. Probes the four corners and levels the gantry; needs X, Y and Z homed.','qgl'),
  act('BED MESH',()=>send('mesh'),'hud-machine','Bed mesh','Sends BED_MESH_CALIBRATE at once. The probe maps the bed; the result shows on the bed card.','mesh'),
  act('MOTORS OFF',()=>send('motors_off'),'hud-machine','Motors off','Sends M84 at once. The steppers let go so the gantry moves by hand, and the machine is no longer homed.','motors_off'),sep(),
  act('PREHEAT',()=>openPreheat(),'hud-machine hud-menu','Preheat','Opens your saved preheat presets and Cool down. Nothing heats until you pick one.','set_heater'),
  act('JOG+',()=>openMotion(),'hud-machine','Motion panel','Opens the jog pad, home, QGL, motors off and the live Z offset.'),sep(),
  act('Prepare',()=>openPrepare(),'hud-primary','Prepare','The pre-print checklist with its evidence, and every calibration.'),act('Files',()=>openFiles(),'','Print files','The G-code on the printer, the queue and the print history. Click a file to draw it on the plate.'),
  act('Console',()=>openConsole(),'','Console','The printer\'s console as recorded, and a line to type G-code with Klipper\'s own help; every line is reviewed once before it goes.'),act('Macros',()=>openMacros(),'','Macros','Your Klipper macros, read from the printer\'s gcode_macro sections. Every run asks once.','macro'),
  act('Testing',()=>openTesting(),'','Testing','Probe accuracy, bed mesh, resonance and shaper runs, and their recorded results.'),sep(),
  act('FW RESTART',()=>send('firmware_restart'),'hud-caution','Firmware restart','Sends FIRMWARE_RESTART after one review. Klipper and both MCUs restart and the heaters switch off.','firmware_restart'),
  act('More',()=>openMore(),'','More','Filament, tune, presets, histories and the rest.'),estop);
 // The bottom strip: three fans (RPM where a tachometer reports, otherwise the commanded percent
 // labelled "cmd"), the print speed, the flow, the layer and the time left. It sits in the empty
 // right of the MOVE row, so the canvas and the rails keep their size. Anything not reported is a
 // dash, never zero.
 const strip=node('div','hud-strip');strip.setAttribute('aria-label','Print and cooling readouts');
 const stripItem=(label,what,does)=>{const r=readoutWindow(label,what,does);r.el.classList.add('hud-strip-item');strip.append(r.el);return r;};
 const stripCtrl=stripItem('CTRL FAN','Controller fan','Cools the electronics bay. Automatic: Klipper runs it with the steppers and heaters. RPM when a tachometer reports, otherwise the commanded percent.');
 const stripHotend=stripItem('HOTEND FAN','Hotend fan','Cools the heatsink above the heat break. Automatic above its trigger temperature. RPM when a tachometer reports, otherwise the commanded percent.');
 const stripPart=stripItem('PART FAN','Part cooling fan','The part cooling blowers. RPM when a tachometer reports, otherwise the commanded percent.');
 const stripSpeed=stripItem('PRINT SPEED','Print speed, mm/s','The speed the G-code asks for, times the M220 speed factor. The note is the toolhead\'s live speed.');
 const stripFlow=stripItem('FLOW','Volumetric flow, mm³/s','Filament going through the hotend, against the ceiling you measured for the material.');
 const stripLayer=stripItem('LAYER','Layer','The layer being printed, of the total, from the slicer or the drawn toolpath.');
 const stripEta=stripItem('ETA','Time left','Print time left from the progress so far, and the clock time it should finish.');
 jogRow.append(strip);
 bottom.append(motion,speed.el,flow.el,jogRow,actions);
 // The recording transport and the header's static controls (service.html) get their tooltips here.
 for(const [id,what,does]of [['live','Latest sample','Follows the newest sample. In live mode that is the printer now, and the controls arm.'],['prev','Previous sample','Steps the recording back one sample.'],['play','Play','Plays the recording from the cursor at the chosen speed. The controls stay locked while it plays.'],['next','Next sample','Steps the recording forward one sample.'],
  ['speed','Playback speed','How fast Play runs through the recording.'],['window','History window','How much recorded history the charts and trends show.'],['copy-link','Copy moment link','Copies a link to this exact moment of the recording.'],['cursor','Recording cursor','Drag to rewind to any recorded moment. The controls stay locked until you return to the latest sample.']])tip($(id),what,does);
 tip(document.querySelector('header .brand'),'Stella Scope','Reloads the HUD.');
 tip($('detail-focus'),'Focus on printer','Closes the card and frames this part on the printer.');tip($('detail-work'),'Record service work','Opens the service journal form for this part.');tip(document.querySelector('#detail-dialog [data-close="detail-dialog"]'),'Close','Closes the card.');
 // The 3D view names the part under the pointer itself (#scene-tip), so its own tooltip is for
 // keyboard focus only; a second box following the pointer over the model would fight it.
 const view=$('machine-view');if(view){tip(view,'The printer','Drag to orbit, wheel to zoom, click a part for its card. Hovering a part names it.');view.dataset.tipFocusOnly='1';}
 const status=overlay('hud-status');status.onclick=()=>openReports();

 // ---------- drawer for the specialist pages ----------
 const drawer=node('dialog','hud-drawer');drawer.id='hud-drawer';const drawerHead=node('div','hud-drawer-head'),drawerNav=node('div','hud-drawer-nav'),drawerBody=node('div','hud-drawer-body');
 drawer.setAttribute('aria-label','Pages');
 const nav=$('pages');nav.hidden=true;const pageLinks=[...nav.querySelectorAll('a')].filter(a=>a.dataset.page!=='machine');
 // What each page is, for the Pages panel and every tooltip that leads to one.
 const PAGE_NOTES={signals:'Every signal on one table: sensors, commands and cached queries, each with its kind and its history.',assembly:'The CAD assembly: every part, its source file and where it sits.',electronics:'The reporting chain: host, controllers, configured pins and firmware.',parts:'The parts book: what is fitted, its revisions and the catalogue.',diagnostics:'Suspected causes, each with the recorded evidence for and against it.',maintenance:'The service journal: work recorded against each part.',timeline:'Everything the printer said, in order, and searchable.',thermal:'Heaters and fans over time, and the cooling circuits.',leveling:'The recorded bed mesh, the probe points and the gantry levelling runs.'};
 for(const a of pageLinks){const b=button(a.textContent,()=>route(a.dataset.page));b.dataset.page=a.dataset.page;tip(b,a.textContent,PAGE_NOTES[a.dataset.page]||'');drawerNav.append(b);}
 drawerHead.append(drawerNav,tip(button('← HUD',()=>route('machine'),'hud-drawer-close'),'Back to the HUD','Closes this page and returns to the printer.'));
 drawerBody.append(document.querySelector('.page-heading'),...document.querySelectorAll('section.page:not(#page-machine)'));
 drawer.append(drawerHead,drawerBody);document.body.append(drawer);
 drawer.addEventListener('cancel',e=>{e.preventDefault();route('machine');});
 closeOnBackdrop(drawer,()=>route('machine'));
 function openDrawer(page){route(page);}
 // ---------- pages ---------- Every page in one glass panel from the left edge. Signals leads and
 // is larger ("the page where everything is laid out on a table"). Service pages open in the
 // drawer over the HUD and "← HUD" brings you back; the workspace and Investigation are their own
 // pages, and both link back to the HUD.
 const pagesPanel=node('dialog','hud-pages');pagesPanel.id='hud-pages';pagesPanel.setAttribute('aria-label','Pages');document.body.append(pagesPanel);
 closeOnBackdrop(pagesPanel);pagesPanel.addEventListener('cancel',e=>{e.preventDefault();pagesPanel.close();});
 const WORKSPACE_PAGES=[['printer','Printer','The machine drawn from its recorded geometry, with the inspector.'],['motion','Motion','Recorded position, speed and the motion history.'],['motors','Motors','Every stepper driver, its current and its temperature.'],['thermal','Thermal','Heater and fan history on one chart.'],['leveling','Levelling','Mesh and gantry levelling runs.'],['events','Events','The console, filterable.'],['recording','Recording','What the recorder holds and how it was captured.']];
 function openPages(){pagesPanel.replaceChildren();
  const head=node('div','hud-pages-head');head.append(node('span','','PAGES'),tip(button('Close',()=>pagesPanel.close()),'Close','Back to the HUD.'));
  const here=node('div','hud-page hud-page-here');here.append(node('strong','','Printer HUD'),node('small','','You are here: the printer, its instruments and every control.'));
  const entry=(label,note,go,cls='')=>{const b=button('',()=>{pagesPanel.close();go();},'hud-page'+(cls?' '+cls:''));b.replaceChildren(node('strong','',label),node('small','',note));tip(b,label,note+' It opens over the HUD; "← HUD" brings you back.');return b;};
  const link=(label,note,href)=>{const a=node('a','hud-page');a.href=href;a.append(node('strong','',label),node('small','',note));tip(a,label,note+' A separate page; its "Printer HUD" link brings you back.');return a;};
  const service=node('div','hud-pages-list');const sig=pageLinks.find(a=>a.dataset.page==='signals');
  if(sig)service.append(entry('Signals',PAGE_NOTES.signals,()=>route('signals'),'hud-page-lead'));
  for(const a of pageLinks)if(a!==sig)service.append(entry(a.textContent,PAGE_NOTES[a.dataset.page]||'',()=>route(a.dataset.page)));
  const work=node('div','hud-pages-list');for(const [id,label,note]of WORKSPACE_PAGES)work.append(link(label,note,'/workspace/'+id));
  const more=node('div','hud-pages-list');more.append(link('Investigation','Pick a window of the recording and export the evidence for a fault.','/investigate'));
  pagesPanel.append(head,here,node('h3','','Service pages'),service,node('h3','','Machine workspace'),work,node('h3','','Investigation'),more);
  pagesPanel.showModal();}

 // ---------- dialogs ----------
 const workflow=node('dialog','hud-dialog');workflow.id='hud-prepare';workflow.setAttribute('aria-label','Prepare and calibration');document.body.append(workflow);
 const close=()=>workflow.close();
 closeOnBackdrop(workflow,close);
 async function openPrepare(tab='prepare'){
  workflow.replaceChildren();const head=node('div','hud-dialog-head');head.append(node('span','',tab==='calibrate'?'CALIBRATE':'BEFORE THE NEXT PRINT'),tip(button('Close',close),'Close','Closes this window. Nothing is sent.'));workflow.append(head);
  const s=getState();
  if(tab==='prepare'){workflow.append(node('h2','','Pre-print routine'),node('p','hud-muted','Checks read the newest printer state. Nothing runs by itself.'));
   const list=node('ol','hud-checklist');for(const check of preflight(s.snapshot,s.live?s.tick?.observer:{mode:'replay'})){const li=node('li');li.dataset.state=check.state;const c=node('div');c.append(node('strong','',check.name),node('span','',check.detail));li.append(c,node('span','hud-check-state',check.state));if(check.command)li.append(tip(button('Run',()=>commandDialog(check.command)),'Run: '+check.name,'Opens this step\'s command page. Nothing is sent from here.',check.command));list.append(li);}workflow.append(list);}
  workflow.append(node('h3','','Calibration & diagnostics'));const grid=node('div','hud-calibrations');
  for(const [id,title,detail]of CALIBRATIONS){const b=tip(button(title,()=>commandDialog(id)),title,detail+' Opens its command page; nothing is sent from here.',id);b.disabled=!can(id);grid.append(b);}
  workflow.append(grid,tip(button('Recorded results',()=>{close();route('leveling');}),'Recorded results','Closes this and opens the Gantry & bed page: meshes, probe points and levelling runs.'),node('p','hud-command-status',caps.enabled?'Each command confirms once and checks fresh printer state before it is sent.':caps.reason));
  if(!workflow.open)workflow.showModal();await controls(true);for(const b of grid.querySelectorAll('button'))b.disabled=!can(CALIBRATIONS[[...grid.children].indexOf(b)][0]);
 }
 async function commandDialog(id){const def=CALIBRATIONS.find(c=>c[0]===id);if(!def)return;await controls(true);
  workflow.replaceChildren();const head=node('div','hud-dialog-head');head.append(tip(button('← Back',()=>openPrepare('calibrate')),'Back','Returns to the calibration list.'),tip(button('Close',close),'Close','Closes this window. Nothing is sent.'));
  const st=node('p','hud-command-status');const run=button('Review exact G-code',async()=>{run.disabled=true;const r=await send(id);st.textContent=r?`${r.message}${r.audit_id?' Audit '+r.audit_id:''}`:'Not sent.';run.disabled=!can(id);},'hud-primary');
  run.disabled=!can(id);tip(run,'Review exact G-code',SENDS,id);st.textContent=run.disabled?why(id):'This is sent to the printer. Make sure the machine is clear.';
  workflow.append(head,node('h2','',def[1]),node('p','hud-muted',def[2]),st,run);if(!workflow.open)workflow.showModal();}
 // ---------- colours ----------
 const palette=node('dialog','hud-dialog hud-palette');palette.id='hud-colors';palette.setAttribute('aria-label','Appearance');document.body.append(palette);
 closeOnBackdrop(palette);
 function openColors(){palette.replaceChildren();const head=node('div','hud-dialog-head');head.append(node('span','','APPEARANCE'),tip(button('Close',()=>palette.close()),'Close','Closes the appearance panel.'));
  const st=node('p','hud-color-status');st.setAttribute('role','status');const rows=node('div','hud-color-groups');
  for(const g of [{id:'all',name:'Whole printer',color:'#4d6874'},...(getScene()?.colorGroups||[]).filter(g=>g.count)]){const label=node('label','hud-color-row'),input=node('input');input.type='color';input.value=g.color;input.setAttribute('aria-label',g.name+' color');tip(input,g.name+' colour',g.id==='all'?'Opens a colour picker for every part. Saved in this browser only.':'Opens a colour picker for every part in this group. Saved in this browser only.');input.oninput=()=>{try{getScene().setColor(g.id==='all'?'all':'group:'+g.id,input.value);st.textContent='Saved in this browser';}catch(e){st.textContent=e.message;}};label.append(node('span','',g.name),node('small','',g.count?g.count+' parts':''),input);rows.append(label);}
  palette.append(head,node('h2','','Paint the printer'),node('p','hud-muted','Click any part for its own colour. Groups paint everything in them.'),rows,tip(button('Restore source colours',()=>{try{getScene().resetColors();openColors();}catch(e){st.textContent=e.message;}}),'Restore source colours','Puts every part back to the colour of its CAD source.'),st);if(!palette.open)palette.showModal();}

 // ---------- control popups ---------- One store, the service's (/api/presets, beside the command
 // audit), read by every preheat list: the PREHEAT menu, the heater dialogs and Filament. Until it
 // is read the lists are empty and say so; there is no second table here to fall back on.
 let MATERIALS=[],PRESETS=null,presetsError='Reading the saved presets…';
 async function loadPresets(){try{const r=await fetch('/api/presets');const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||'The saved presets could not be read.');PRESETS=d;presetsError='';MATERIALS=(Array.isArray(d.materials)?d.materials:[]).map(m=>[m.name,m.nozzle,m.bed]);document.querySelector('.hud-loaded')?.replaceChildren(PRESETS.loaded?.name?PRESETS.loaded.name+' loaded':'');
  // The PREHEAT menu lists the saved presets live; an open editor keeps its own unsaved edits.
  modalStack.find(m=>m.id==='preheat')?.refresh();}catch(e){presetsError=e.message;}}
 loadPresets();
 function commandPreview(text){const e=node('code','hud-gcode',text);return e;}
 function row(label,...kids){const r=node('div','hud-row');r.append(node('span','hud-row-label',label),...kids);return r;}
 function numberField(value,min,max,step=1){const i=node('input','hud-num');i.type='number';i.min=min;i.max=max;i.step=step;if(known(value))i.value=value;else i.placeholder='not reported';return i;}
 function fieldValue(input){if(input.value===''||!input.reportValidity())return null;return Number(input.value);}
 function reasonLine(id){const p=node('p','hud-command-status');p.textContent=can(id)?'':why(id);return p;}

 function openHeater(which){const heater=which==='nozzle'?'extruder':'heater_bed',max=which==='nozzle'?290:120;
  modal('heat-'+which,which==='nozzle'?'Nozzle':'Bed',(body)=>{
   const p=point();const now=which==='nozzle'?p?.nozzle:p?.bed;
   body.append(node('p','hud-modal-note',`now ${number(now?.temperature,' °C')} · target ${number(now?.target,' °C')} · output ${number(known(now?.power)?now.power*100:null,'%',0)}`));
   body.append(node('h3','','Preheat'));
   const grid=node('div','hud-material-grid');
   for(const [name,nozzle,bed]of MATERIALS){const b=button('','',null);b.replaceChildren(node('strong','',name),node('small','',`${nozzle}° / ${bed}°`));b.className='hud-material';tip(b,name+' preheat',`Sets the nozzle to ${nozzle} °C and the bed to ${bed} °C.`,'set_heater');
    b.onclick=async()=>{if(await send('set_heater',{target:'extruder',value:nozzle}))await send('set_heater',{target:'heater_bed',value:bed});};
    grid.append(b);}
   body.append(grid,node('p','hud-modal-note','Sets both heaters from your saved presets. Change them with Edit presets under PREHEAT, or Presets under More.'));
   body.append(node('h3','','Exact target'));
   const field=tip(numberField(known(now?.target)?Math.round(now.target):null,0,max),(which==='nozzle'?'Nozzle':'Bed')+' target, °C','Type a target from 0 to '+max+' °C; Set sends it.');
   const set=button('Set',async()=>{const value=fieldValue(field);if(value!==null)await send('set_heater',{target:heater,value:clamp(value,0,max)});},'hud-primary');tip(set,'Set target','Sends the typed target to the '+(which==='nozzle'?'nozzle':'bed')+' heater.','set_heater');
   set.disabled=!field.value;field.oninput=()=>{set.disabled=!field.value;};
   body.append(row(which==='nozzle'?'Nozzle °C':'Bed °C',field,set,tip(button('Cool down',()=>send('set_heater',{target:heater,value:0})),'Cool down','Sets the '+(which==='nozzle'?'nozzle':'bed')+' target to 0 °C.','set_heater')));
   body.append(reasonLine('set_heater'));
  });}
 // The toolbar's PREHEAT. the own profile temperatures; nothing heats until one is picked, and each
 // heater goes out through send like the gauges (T2, no review).
 function openPreheat(){modal('preheat','Preheat',(body)=>{
   const p=point();body.append(node('p','hud-modal-note',`nozzle ${number(p?.nozzle?.temperature,' °C')} · bed ${number(p?.bed?.temperature,' °C')} · your saved presets${presetsError?' · '+presetsError:''}`));
   const grid=node('div','hud-material-grid'),heat=(noz,hot)=>async()=>{if(await send('set_heater',{target:'extruder',value:noz}))await send('set_heater',{target:'heater_bed',value:hot});};
   const menu=preheatMenu(PRESETS);let cool=null;
   for(const m of menu){if(m.fixed){cool=tip(button(m.name,heat(0,0)),'Cool down','Sets both heater targets to 0 °C. Built in, always here.','set_heater');continue;}
    const b=button('',heat(m.nozzle,m.bed),'hud-material');b.replaceChildren(node('strong','',m.name),node('small','',`${m.nozzle}° / ${m.bed}°`));tip(b,m.name+' preheat',`Sets the nozzle to ${m.nozzle} °C and the bed to ${m.bed} °C.`,'set_heater');grid.append(b);}
   if(menu.length===1)grid.append(node('p','hud-modal-note','No saved preheat presets yet. Add them in Edit presets.'));
   const edit=tip(button('Edit presets',()=>openPresets()),'Edit presets','Opens the preset editor: add, rename, change or delete presets, then Save.');
   body.append(grid,row('',cool,edit),reasonLine('set_heater'));
  });}

 function openFactor(which){const id=which==='speed'?'speed_factor':'extrude_factor';
  modal('factor-'+which,which==='speed'?'Print speed':'Flow',(body)=>{
   const p=point(),current=which==='speed'?p?.speed_factor:p?.extrude_factor;
   body.append(node('p','hud-modal-note',`now ${number(known(current)?current*100:null,'%',0)} · 50 to 200 percent`));
   const field=tip(numberField(known(current)?Math.round(current*100):null,50,200),'Percent','Type 50 to 200 percent; Set sends it.');
   const apply=async v=>{const pct=clamp(Number(v),50,200);if(await send(id,{value:pct})&&which==='speed')offerFlow(pct,known(p?.extrude_factor)?p.extrude_factor*100:null);};
   const set=tip(button('Set',()=>{const value=fieldValue(field);if(value!==null)apply(value);},'hud-primary'),'Set '+(which==='speed'?'print speed':'flow'),'Sends the typed percent ('+(which==='speed'?'M220':'M221')+').',id);set.disabled=!field.value;field.oninput=()=>{set.disabled=!field.value;};
   body.append(row('Percent',field,set));
   const presets=node('div','hud-presets');
   for(const v of (which==='speed'?[80,100,110,120,135,150]:[90,95,100,105,110]))presets.append(tip(button(v+'%',()=>apply(v)),v+'%','Sets '+(which==='speed'?'print speed':'flow')+' to '+v+' percent.',id));
   body.append(presets,reasonLine(id));
  });}
 // A speed increase may need a separately measured flow adjustment; never invent one. K14/L9: the
 // arithmetic behind a speed change, shown, never applied for the operator. Volumetric flow = layer
 // height x line width x print speed x factor. The hotend limit is whatever the user measured (the
 // test printer/docs/10, method A); until he enters one it stays blank and nothing is judged.
 function offerFlow(speedPct,flowPct){
  if(speedPct<=100)return;
  const store=(k,d)=>{try{const v=localStorage.getItem('stella-flow:'+k);return v===null?d:Number(v);}catch{return d;}};
  const keep=(k,v)=>{try{localStorage.setItem('stella-flow:'+k,String(v));}catch{}};
  modal('flow-offer','Speed is up. Here is the flow arithmetic',(body)=>{
   body.append(node('p','hud-modal-note',`Speed factor is now ${speedPct}%. Flow factor is ${known(flowPct)?flowPct+'%':'not reported'} and was not changed.`));
   const layer=numberField(store('layer',0.2),0.05,1,0.01),width=numberField(store('width',0.45),0.2,1.5,0.01),speed=numberField(store('speed',150),5,600,5),limit=numberField(known(PRESETS?.hotend_max_flow)?PRESETS.hotend_max_flow:null,1,80,0.5);limit.placeholder='not measured';
   // The measured limit lives on the recorder with the presets, not in this browser.
   let limitTimer=0;limit.addEventListener('change',()=>{clearTimeout(limitTimer);limitTimer=setTimeout(()=>{const v=limit.value?Number(limit.value):null;jsend('/api/presets','PUT',{...(PRESETS||{}),hotend_max_flow:v}).then(loadPresets).catch(e=>feedback(e.message,'refused'));},300);});
   const out=node('p','hud-flow-math');
   const calc=()=>{const l=Number(layer.value),w=Number(width.value),v=Number(speed.value),m=limit.value?Number(limit.value):null;keep('layer',l);keep('width',w);keep('speed',v);
    if(!(l>0&&w>0&&v>0)){out.textContent='Enter layer height, line width and the profile print speed.';out.dataset.state='';return;}
    const base=l*w*v,now=base*speedPct/100;
    out.replaceChildren(node('span','',`${l} mm x ${w} mm x ${v} mm/s = ${base.toFixed(1)} mm³/s at 100%`),node('br'),node('strong','',`x ${speedPct}% = ${now.toFixed(1)} mm³/s now`),node('br'),
     node('span','',m?(now>m?`Over the ${m} mm³/s you measured by ${(now-m).toFixed(1)} mm³/s. Drop speed to ${Math.floor(m/base*100)}% or accept thin extrusion.`:`Inside the ${m} mm³/s you measured, with ${(m-now).toFixed(1)} mm³/s to spare.`):'No measured hotend limit entered, so nothing is judged. Measure it with Stella/docs/10 method A, then type it here.'));
    out.dataset.state=m?(now>m?'refused':'reported'):'';};
   for(const f of [layer,width,speed,limit])f.oninput=calc;tip(layer,'Layer height, mm','Type your profile\'s layer height; the sum below updates.');tip(width,'Line width, mm','Type your profile\'s line width.');tip(speed,'Profile print speed, mm/s','Type your profile\'s print speed.');tip(limit,'Measured hotend limit, mm³/s','Type the most flow you measured. It is saved on the recorder.');
   body.append(row('Layer height mm',layer),row('Line width mm',width),row('Profile print speed mm/s',speed),row('Measured hotend max mm³/s',limit),out);calc();
   body.append(row('',tip(button('Change flow instead',()=>{const m=modalStack.find(m=>m.id==='flow-offer');if(m)closeModal(m);openFactor('flow');},'hud-primary'),'Change flow instead','Closes this and opens the flow factor.'),
    tip(button('Keep flow as it is',()=>{const m=modalStack.find(m=>m.id==='flow-offer');if(m)closeModal(m);}),'Keep flow as it is','Closes this. Nothing is changed.')));
  });}

 function openFan(which){const isPart=which==='fan';
  modal('fan-'+which,isPart?'Part cooling':which==='hotend_fan'?'Hotend fan':'Controller fan',(body)=>{
   const p=point(),fans=fansAt(p),f=fans.find(f=>f.object===which||f.object.endsWith(' '+which));
   body.append(node('p','hud-modal-note',`commanded ${number(known(f?.command)?f.command*100:null,'%',0)} · ${f?.rpm==null?'no tachometer on this fan':number(f.rpm,' RPM',0)}`));
   if(isPart){
    const field=tip(numberField(known(f?.command)?Math.round(f.command*100):null,0,100),'Part fan, percent','Type 0 to 100 percent; Set sends it.');
    const set=tip(button('Set',()=>{const value=fieldValue(field);if(value!==null)send('set_fan',{value:clamp(value,0,100)/100});},'hud-primary'),'Set part fan','Sends the typed percent (M106).','set_fan');set.disabled=!field.value;field.oninput=()=>{set.disabled=!field.value;};
    body.append(row('Percent',field,set));
    const presets=node('div','hud-presets');for(const v of [0,25,50,75,100])presets.append(tip(button(v+'%',()=>send('set_fan',{value:v/100})),v+'%','Sets the part fan to '+v+' percent.','set_fan'));
    body.append(presets,reasonLine('set_fan'));
   }else if(which==='hotend_fan'){
    body.append(node('p','hud-modal-note','Klipper drives this fan automatically above its trigger temperature. It cannot be switched off from here, and it should not be: losing it while the nozzle is hot melts the heatbreak.'));
    body.append(node('h3','','Trigger temperature'));
    body.append(heaterFanEditor());
   }else{
    body.append(node('p','hud-modal-note','Klipper reports this automatic controller-fan output. Stella Scope does not expose a manual controller-fan command.'),
     node('p','hud-command-status','Read only. Change its configured controller activity rules outside this HUD.'));
   }
  });}

 function openMotion(){modal('motion','Motion',(body)=>{
   const p=point();const isHomed=/x/.test(p?.homed||'')&&/y/.test(p?.homed||'')&&/z/.test(p?.homed||'');
   body.append(node('p','hud-modal-note',`X ${number(p?.position?.[0])} · Y ${number(p?.position?.[1])} · Z ${number(p?.position?.[2])} mm · homed ${p?.homed?.toUpperCase()||'none'}`));
   const steps=node('div','hud-presets');for(const st of [0.1,1,10,50]){const b=tip(button(st+' mm',()=>{step=st;[...steps.children].forEach(x=>x.classList.toggle('on',Number(x.dataset.s)===step));}),st+' mm step','Sets how far each jog button moves.');b.dataset.s=st;b.classList.toggle('on',st===step);steps.append(b);}
   const pad=node('div','hud-jog-pad');const cell=(t,fn,cls='')=>{const b=button(t,fn,cls);if(t==='⌂')tip(b,'Home XYZ','Homes all three axes (G28).','home');else if(t)tip(b,'Jog '+t,'Moves '+t+' by the step chosen above. Needs a homed printer.','jog');pad.append(b);};
   const move=(axis,sign)=>async()=>{if(!isHomed){feedback('The printer is not homed. Home XYZ first.','refused');return;}await send('jog',{axis,value:sign*step});};
   cell('');cell('Y+',move('Y',1));cell('');cell('Z+',move('Z',1),'z');
   cell('X−',move('X',-1));cell('⌂',()=>send('home'),'home');cell('X+',move('X',1));cell('');
   cell('');cell('Y−',move('Y',-1));cell('');cell('Z−',move('Z',-1),'z');
   const acts=node('div','hud-calibrations');
   acts.append(tip(button('Home XYZ',()=>send('home')),'Home XYZ','Homes all three axes (G28).','home'),tip(button('Quad gantry level',()=>send('qgl')),'Quad gantry level','Probes the four corners and levels the gantry (QUAD_GANTRY_LEVEL). Needs a homed printer.','qgl'),tip(button('Motors off',()=>send('motors_off')),'Motors off','Turns the steppers off (M84) so the gantry moves by hand.','motors_off'));
   body.append(steps,pad,acts,node('h3','','Z offset · live'),zOffsetRow(p),reasonLine('jog'));
  });}
 function zOffsetRow(p){const wrap=node('div','hud-zoffset');
  wrap.append(node('strong','',number(p?.z_offset,' mm',3)));
  const bar=node('div','hud-presets');
  for(const d of [-0.05,-0.01,0.01,0.05])bar.append(tip(button((d>0?'+':'')+d.toFixed(2),()=>send('z_offset',{value:d})),'Z offset '+(d>0?'+':'')+d.toFixed(2)+' mm','Babysteps the nozzle '+Math.abs(d).toFixed(2)+' mm '+(d>0?'away from':'toward')+' the bed, at once.','z_offset'));
  bar.append(tip(button('Save to config',()=>send('z_offset_save'),'hud-secondary'),'Save to config','Writes the probe offset to printer.cfg and restarts Klipper. Asks once.','z_offset_save'));
  wrap.append(bar,node('small','hud-modal-note','Babystepping applies immediately and moves the toolhead. Saving writes the probe offset and restarts Klipper.'));
  return wrap;}

 function openFilament(){modal('filament','Filament',(body)=>{
   const p=point();
   body.append(node('p','hud-modal-note',`nozzle ${number(p?.nozzle?.temperature,' °C')} · Klipper refuses to extrude below its minimum temperature`));
   const preheat=node('div','hud-presets');
   for(const [name,nozzle]of MATERIALS)preheat.append(tip(button(name,()=>send('set_heater',{target:'extruder',value:nozzle})),name+' nozzle heat','Sets the nozzle to '+nozzle+' °C.','set_heater'));
   body.append(node('h3','','Heat first'),preheat);
   // What is loaded decides the temperature. The choice is stored on the recorder with its time.
   const loaded=PRESETS?.loaded?.name||null;const materialSel=node('select','hud-select');
   for(const [name]of MATERIALS){const o=node('option','',name);o.value=name;if(name===loaded)o.selected=true;materialSel.append(o);}
   if(!loaded){const o=node('option','','choose material');o.value='';o.selected=true;materialSel.prepend(o);}tip(materialSel,'Loaded material','Pick what is in the extruder. LOAD and UNLOAD heat to its nozzle preset.');
   const tempFor=()=>MATERIALS.find(([n])=>n===materialSel.value)?.[1]||null;
   const loadedNote=node('p','hud-modal-note',loaded?`Loaded now: ${loaded}${known(PRESETS?.loaded?.at)&&PRESETS.loaded.at>0?', since '+new Date(PRESETS.loaded.at*1000).toLocaleString():''}. LOAD and UNLOAD heat to its nozzle preset.`:'Nothing recorded as loaded yet. Pick the material and LOAD records it.');
   const remember=async name=>{try{await jsend('/api/presets','PUT',{...(PRESETS||{}),loaded:name?{name,at:Date.now()/1000}:null});await loadPresets();}catch(e){feedback(e.message,'refused');}};
   const loadBtn=button('LOAD_FILAMENT',async()=>{const t=tempFor();if(!t){feedback('Choose the material first.','refused');return;}if(await send('macro',{target:'LOAD_FILAMENT',params:{TEMP:String(Math.round(t))}}))await remember(materialSel.value);},'hud-primary');
   const unloadBtn=button('UNLOAD_FILAMENT',async()=>{const t=tempFor();if(!t){feedback('Choose the material that is in there first.','refused');return;}if(await send('macro',{target:'UNLOAD_FILAMENT',params:{TEMP:String(Math.round(t))}}))await remember(null);});tip(loadBtn,'LOAD_FILAMENT','Runs the printer\'s LOAD_FILAMENT macro at the material\'s nozzle preset and records it as loaded. Asks once.','macro');tip(unloadBtn,'UNLOAD_FILAMENT','Runs the printer\'s UNLOAD_FILAMENT macro at the material\'s nozzle preset. Asks once.','macro');
   body.append(node('h3','','Loaded filament'),loadedNote,row('Material',materialSel,loadBtn,unloadBtn),node('p','hud-modal-note','Your printer\'s own LOAD_FILAMENT and UNLOAD_FILAMENT macros run, with TEMP set from the preset.'));
   body.append(node('h3','','Move filament'));
   const amount=tip(numberField(50,1,200),'Millimetres','Type how much filament the two buttons beside it move.'),speedField=tip(numberField(5,1,50),'Speed, mm/s','Type the filament speed for these moves.');
   const load=button('Load / extrude',()=>{const value=fieldValue(amount);if(value!==null)send('extrude',{value:Math.abs(value),speed:Number(speedField.value)||5});},'hud-primary');
   const unload=button('Unload / retract',()=>{const value=fieldValue(amount);if(value!==null)send('extrude',{value:-Math.abs(value),speed:Number(speedField.value)||5});});
   tip(load,'Load / extrude','Pushes the typed length through the nozzle.','extrude');tip(unload,'Unload / retract','Pulls the typed length back.','extrude');
   body.append(row('Millimetres',amount,load,unload),row('Speed mm/s',speedField,tip(button('Feed 50 mm',()=>send('load_filament',{speed:Number(speedField.value)||5})),'Feed 50 mm','Feeds 50 mm at the typed speed.','load_filament'),tip(button('Retract 100 mm',()=>send('unload_filament',{speed:Number(speedField.value)||5})),'Retract 100 mm','Pulls 100 mm back at the typed speed.','unload_filament')));
   const sensors=Array.isArray(p?.filament_sensors)?p.filament_sensors.map(st=>[st.name,st]):Object.entries(p?.filament_sensors||{});if(sensors.length){const sec=node('div','hud-facts');for(const [name,st]of sensors){const e=node('div','hud-fact');e.append(node('span','',String(name).replace('filament_switch_sensor ','').toUpperCase()),node('strong','',st.filament_detected===true?'Filament present':st.filament_detected===false?'No filament':'Unknown'),node('small','',st.enabled===false?'sensor disabled':'runout sensor'));sec.append(e);}body.append(node('h3','','Runout sensor'),sec);}
   const quick=node('div','hud-presets');
   for(const v of [5,10,25,100])quick.append(tip(button('+'+v,()=>send('extrude',{value:v})),'Extrude '+v+' mm','Pushes '+v+' mm of filament through the nozzle.','extrude'));
   for(const v of [5,25,100])quick.append(tip(button('−'+v,()=>send('extrude',{value:-v})),'Retract '+v+' mm','Pulls '+v+' mm of filament back.','extrude'));
   body.append(quick,reasonLine('extrude'));
  });}

 function openTune(){modal('tune','Tuning',(body)=>{
   const p=detailPoint(),reported=(direct,object,field)=>{if(known(direct))return direct;const value=p?.telemetry?.signals?.find(s=>s.object===object&&s.field===field)?.value;return known(value)?value:null;};
   const values={velocity:reported(p?.max_velocity,'toolhead','max_velocity'),accel:reported(p?.max_accel,'toolhead','max_accel'),scv:reported(p?.square_corner_velocity,'toolhead','square_corner_velocity'),pa:reported(p?.pressure_advance,'extruder','pressure_advance')};
   body.append(node('h3','','Limits'),node('p','hud-modal-note','Current values come from the selected recorded sample. Unreported settings stay blank. Configured defaults are not shown here.'));
   for(const [key,label,min,max,val]of [['velocity','Velocity mm/s',1,1000,values.velocity],['accel','Acceleration mm/s²',100,25000,values.accel],['scv','Square corner velocity',0.5,30,values.scv]]){
    const f=tip(numberField(val,min,max,key==='scv'?0.1:1),label,'Type a new value; Set sends it.');
    const set=tip(button('Set',()=>{const value=fieldValue(f);if(value!==null)send('limit',{target:key,value});}),'Set '+label.replace(/ mm.*$/,'').toLowerCase(),'Sends it with SET_VELOCITY_LIMIT. It lasts until the next restart.','limit');set.disabled=!f.value;f.oninput=()=>{set.disabled=!f.value;};body.append(row(label,f,set));}
   body.append(node('h3','','Pressure advance'));
   const pa=tip(numberField(values.pa,0,1,0.0005),'Pressure advance, s','Type a new value; Set sends it.'),setPa=tip(button('Set',()=>{const value=fieldValue(pa);if(value!==null)send('pressure_advance',{value});}),'Set pressure advance','Sends it with SET_PRESSURE_ADVANCE. It lasts until the next restart.','pressure_advance');setPa.disabled=!pa.value;pa.oninput=()=>{setPa.disabled=!pa.value;};
   body.append(row('Seconds',pa,setPa),reasonLine('limit'),reasonLine('pressure_advance'));
  });}

 function openConsole(){modal('console','Console',(body)=>{
   const log=node('div','hud-console-log');
   const input=node('input','hud-console-input');input.type='text';input.placeholder='G-code, e.g. QUERY_ENDSTOPS';input.setAttribute('aria-label','G-code console');tip(input,'G-code console','Type a command. Enter or Send shows it for review, then sends it once. The arrow keys recall history.','console');
   const history=[];let at=-1;
   const append=(text,cls='')=>{const line=node('div','hud-console-line '+cls,text);log.append(line);log.scrollTop=log.scrollHeight;};
   for(const e of (getState().snapshot?.events||[]).slice(-40))append(`${new Date(e.ts*1000).toLocaleTimeString()}  ${e.text||e.kind}`,'past');
   const datalist=node('datalist');datalist.id='hud-gcode-help';input.setAttribute('list',datalist.id);body.append(datalist);
   if(!link().lock)jget('/api/console/help').then(d=>{for(const h of d.commands||d.help||[]){const o=node('option');o.value=h.name;o.label=h.help||'';datalist.append(o);}}).catch(()=>{});
   jget('/api/console/history').then(d=>{for(const h of d.entries||d.history||[])history.push(typeof h==='string'?h:h.script||h.line||'');at=history.length;}).catch(()=>{});
   let stream=null;try{stream=new EventSource('/api/tick/stream');stream.addEventListener('console',ev=>{try{const d=JSON.parse(ev.data);const lines=Array.isArray(d)?d:Array.isArray(d.lines)?d.lines:[d];for(const l of lines)append(`${l.ts?new Date(l.ts*1000).toLocaleTimeString()+'  ':''}${l.text||l.message||''}`,'past');}catch{}});}catch{}
   const entry=modalStack.find(m=>m.id==='console');if(entry)entry.onClose=()=>{stream?.close();};
   const submit=async()=>{const line=input.value.trim();if(!line)return;const r=await send('console',{script:line});
    if(r){history.push(line);at=history.length;input.value='';append('> '+line,'sent');append(r.message,'ok');jsend('/api/console/history','POST',{line}).catch(()=>{});}else append('Not sent.','bad');};
   input.onkeydown=e=>{if(e.key==='Enter')submit();
    else if(e.key==='ArrowUp'&&history.length){at=Math.max(0,at-1);input.value=history[at];}
    else if(e.key==='ArrowDown'&&history.length){at=Math.min(history.length,at+1);input.value=history[at]||'';}};
   body.append(log,row('',input,tip(button('Send',submit,'hud-primary'),'Send','Shows the typed command for review, then sends it once.','console')),reasonLine('console'));
   setTimeout(()=>input.focus(),50);
  },{wide:true});}

 async function openMacros(){modal('macros','Macros',async(body)=>{
   // The list is your gcode_macro sections (the service flags them, read from the printer's config);
   // every other command Klipper registers is one tick away.
   const intro=node('p','hud-modal-note','Your macros: the gcode_macro sections in the printer\'s config. Every run asks once.');body.append(intro);
   const list=node('div','hud-macro-list');body.append(list);
   try{if(link().lock)throw Error(link().reason);const r=await fetch('/api/macros');const data=await r.json();
    if(!r.ok)throw Error(data.error||'macros unavailable');
    const mine=data.macros.filter(m=>m.macro===true),every=node('input');every.type='checkbox';every.checked=!mine.length;const everyLabel=node('label','hud-paint-scope');everyLabel.append(every,document.createTextNode(' every Klipper command'));for(const el of [everyLabel,every])tip(el,'Every Klipper command','Also lists the commands Klipper and its modules register, not only your macros.');
    if(!mine.length)intro.textContent='The printer\'s config could not be read, so this is every command Klipper registers.';
    const search=node('input','hud-console-input');search.type='search';search.placeholder='Find a macro…';tip(search,'Find a macro','Type part of a name to filter the list.');body.insertBefore(search,list);body.insertBefore(everyLabel,list);every.onchange=()=>draw();
    const draw=()=>{const q=search.value.toLowerCase();list.replaceChildren();
     for(const m of (every.checked?data.macros:mine).filter(m=>!q||m.name.toLowerCase().includes(q)).slice(0,120)){
      const params=Array.isArray(m.params)?m.params:[];
      const b=button(m.name,()=>{if(!params.length)return send('macro',{target:m.name});
       modal('macro-'+m.name,m.name,(mb)=>{const fields={};for(const pr of params){const i=node('input','hud-num');i.type='text';i.placeholder=pr.default!=null?String(pr.default):'';fields[pr.name]=i;tip(i,pr.name,'Type a value, or leave it blank for the macro\'s default'+(pr.default!=null?' ('+pr.default+')':'')+'.');mb.append(row(pr.name,i));}
        mb.append(row('',tip(button('Run',()=>{const values={};for(const [k,i]of Object.entries(fields))if(i.value.trim()!=='')values[k]=i.value.trim();send('macro',{target:m.name,params:values});},'hud-primary'),'Run '+m.name,'Shows the command for review, then sends it once.','macro')),reasonLine('macro'));});});
      tip(b,m.name,(typeof m.help==='string'&&m.help?m.help.replace(/\.?$/,'. '):'')+(params.length?'Opens a form for '+params.map(p=>p.name).join(', ')+', then asks once.':'Shows it for review, then sends it once.'),'macro');list.append(b);}
     if(!list.childElementCount)list.append(node('p','hud-modal-note','No macro matches.'));};
    search.oninput=draw;draw();
   }catch(e){list.append(node('p','hud-modal-note',e.message));}
  },{wide:true});}

 async function openFiles(){modal('files','Print files',async(body)=>{
   const current=job();
   body.append(node('p','hud-modal-note',current?.name?`Current job: ${current.name.split('/').pop()}`:'No job reported.'));
   const controls=node('div','hud-calibrations');
   controls.append(tip(button('Pause',()=>send('pause')),'Pause','Pauses the running print.','pause'),tip(button('Resume',()=>send('resume')),'Resume','Resumes a paused print.','resume'),tip(button('Cancel print',()=>send('cancel'),'hud-danger'),'Cancel print','Stops the running print for good. Asks once.','cancel'),tip(button('Exclude objects',()=>openObjects()),'Exclude objects','Opens the objects in the running file, to skip one.','exclude_object'));
   body.append(controls);
    const queue=node('div','hud-queue');body.append(node('h3','','Queue'),queue);
    (noPrinter()?Promise.reject(Error(link().reason)):jget('/api/print/queue')).then(d=>{const jobs=d.jobs||d.queued_jobs||[];if(!jobs.length)return queue.append(node('p','hud-modal-note','Nothing queued.'));for(const j of jobs){const e=node('div','hud-fact');e.append(node('span','',(j.filename||'').split('/').pop()),node('strong','',j.state||''),tip(button('Remove',()=>send('print_queue_remove',{job_id:j.job_id})),'Remove from queue','Takes this job out of the print queue.','print_queue_remove'));queue.append(e);}}).catch(e=>queue.append(node('p','hud-modal-note',e.message)));
    // Print history, straight from the recorder's own session rows. Click a row to rewind the
    // recording to the moment that print ended.
    const history=node('div','hud-history-list');body.append(node('h3','','Print history'),history);
    jget('/api/history/prints').then(d=>{const prints=d.prints||[];if(!prints.length)return history.append(node('p','hud-modal-note','No finished print recorded yet.'));
     for(const p of prints.slice(0,10)){const e=node('div','hud-fact hud-print');e.dataset.result=(p.result||'').toLowerCase();
      e.append(node('span','',new Date(p.end_ts*1000).toLocaleString()),node('strong','',(p.filename||'').split('/').pop()),
       node('small','',`${(p.result||'')} · ${number(known(p.duration_s)?p.duration_s/60:null,' min',0)}${known(p.filament_used)&&p.filament_used>0?' · '+number(p.filament_used/1000,' m',2):''}`));
      tip(e,'Rewind to this print','Moves the recording cursor to the moment this print ended.'+(p.evidence_note?' '+p.evidence_note:''));e.onclick=()=>{if(known(p.end_ts))document.dispatchEvent(new CustomEvent('stella-seek',{detail:{ts:p.end_ts}}));};
      history.append(e);}}).catch(e=>history.append(node('p','hud-modal-note',e.message)));
    body.append(node('h3','','G-code on the printer'),node('p','hud-modal-note','Click a file to draw its toolpaths on the plate. Print starts it after one confirmation.'));
    const list=node('div','hud-file-list');body.append(list);
    try{if(noPrinter())throw Error(link().reason);const r=await fetch('/api/files');const data=await r.json();if(!r.ok)throw Error(data.error||'file list unavailable');
     for(const f of data.files.slice(0,80)){const b=button('','',null);b.className='hud-file';
      // The slicer thumbnail for this file, proxied read-only from Moonraker; files sliced
      // without thumbnails just lose the image, nothing else.
      const thumb=node('img','hud-file-thumb');thumb.loading='lazy';thumb.alt='';thumb.decoding='async';
      thumb.src='/api/print/thumbnail?filename='+encodeURIComponent(f.path);thumb.onerror=()=>thumb.remove();
      b.replaceChildren(thumb,node('strong','',f.path),node('small','',`${(f.size/1048576).toFixed(1)} MB · ${f.modified?new Date(f.modified*1000).toLocaleDateString():''}`));
     const start=tip(button('Print',()=>send('print_start',{filename:f.path}),'hud-primary'),'Print','Starts this file on the printer. Asks once.','print_start'),queue=tip(button('Queue',()=>send('print_queue_add',{filename:f.path})),'Queue','Adds this file to the print queue.','print_queue_add');
     const rowEl=node('div','hud-file-row');rowEl.append(b,start,queue);
     tip(b,f.path.split('/').pop(),'Draws this file\'s toolpath on the plate. Nothing is sent to the printer.');b.onclick=()=>showToolpath(f.path).catch(e=>feedback(e.message,'refused'));
     list.append(rowEl);}
    if(!data.files.length)list.append(node('p','hud-modal-note','No G-code files on the printer.'));
   }catch(e){list.append(node('p','hud-modal-note',e.message));}
  },{wide:true});}

 // ---------- bed mesh, drawn here so the numbers can be explained ---------- the same fix as the
 // bed card. When the sample at the cursor carries no mesh, this shows the newest one in the
 // recording, dated, and says so; "none" only when there is none.
 function openMesh(){modal('mesh','Bed mesh',(body)=>{
   const p=detailPoint(),sampled=p?.mesh?.grid?.length?p.mesh:null,s=getState(),to=s.live?Date.now()/1000:(s.cursor??Date.now()/1000);
   if(sampled){drawMesh(body,sampled);return;}const wait=node('p','hud-modal-note','Reading the newest mesh in the recording…');body.append(wait);
   recordedMesh(to).then(m=>{wait.remove();if(!m?.grid){body.append(node('p','hud-modal-note','No probed mesh in the recording yet. Run a bed mesh from Prepare, then come back.'),tip(button('Run BED_MESH_CALIBRATE',()=>send('mesh'),'hud-primary'),'Run bed mesh','Probes the bed now (BED_MESH_CALIBRATE). Needs a homed printer.','mesh'));return;}
    body.append(node('p','hud-modal-note',`Recorded ${new Date(m.ts*1000).toLocaleString()} · ${since(to-m.ts)} before ${s.live?'now':'the cursor'}. The sample at the cursor carries no mesh.`));drawMesh(body,m);}).catch(e=>{wait.textContent='Mesh history unavailable: '+e.message;});
  },{wide:true});}
 function drawMesh(body,mesh){
   const wrap=node('div','hud-mesh-canvas');const cv=node('canvas');cv.setAttribute('aria-label','Probed bed mesh surface');wrap.append(cv);body.append(wrap);
   const facts=node('div','hud-facts');body.append(facts);
   try{const info=meshSurface(cv,{grid:mesh.grid,min:mesh.min,max:mesh.max,profile:mesh.profile||''});
    const fact=(k,v,note)=>{const e=node('div','hud-fact');e.append(node('span','',k),node('strong','',v),node('small','',note||''));facts.append(e);};
    fact('RANGE',info.range.toFixed(3)+' mm',`${info.points} probed points, ${info.rows}×${info.cols}`);
    fact('LOW / HIGH',info.low.toFixed(3)+' / '+info.high.toFixed(3),'millimetres from the mesh plane');
    fact('WORST',info.worst.delta.toFixed(3)+' mm',`${info.worst.corner}, near X${info.worst.x.toFixed(0)} Y${info.worst.y.toFixed(0)}`);
    fact('PROFILE',mesh.profile||'unnamed','height is exaggerated ×'+info.exaggeration.toFixed(0)+' so a bow is visible');
    body.append(node('p','hud-modal-note',info.range<0.1?'Under 0.1 mm across the plate. That is a flat bed; mesh compensation has little left to do.'
      :info.range<0.25?'Between 0.1 and 0.25 mm across the measured mesh. Check the first layer before relying on compensation.'
      :'Over 0.25 mm. Worth checking gantry level and whether the sheet is seated before blaming the bed.'));
   }catch(e){facts.append(node('p','hud-modal-note',e.message));}}

 // ---------- temperature history ----------
 async function openHistory(){modal('history','Temperature history',async(body)=>{
   const wrap=node('div','hud-chart');const cv=node('canvas');wrap.append(cv);body.append(wrap);
   const note=node('p','hud-modal-note','Loading captured samples…');body.append(note);
   try{const s=getState(),to=s.live?point()?.ts:s.cursor;if(!known(to))throw Error('Selected recording timestamp is unavailable; history was not requested.');const from=to-3600;
    const r=await fetch(`/api/series?from=${from}&to=${to}&points=400`);const data=await r.json();
    if(!r.ok)throw Error(data.error||'series unavailable');
    const pick=(key,color)=>({color,points:(data.points||[]).filter(p=>typeof p[key]==='number').map(p=>[p.ts,p[key]])});
    const drawn=lineChart(cv,[{...pick('nozzle','#ff9a3c'),width:2},{...pick('nozzle_target','#ff9a3c'),dashed:true,width:1},{...pick('bed','#ffd27a'),width:2},{...pick('bed_target','#ffd27a'),dashed:true,width:1}],{unit:'°'});
    note.textContent=drawn?`Nozzle in orange, bed in amber, targets dashed. ${new Date(drawn.from*1000).toLocaleTimeString()} to ${new Date(drawn.to*1000).toLocaleTimeString()}, ${drawn.low.toFixed(0)} to ${drawn.high.toFixed(0)} °C.`
     :'No temperature history captured in the selected recorded hour.';
    // Part fan and travel speed under the temperatures: the two things that change a heat curve.
    const wrap2=node('div','hud-chart');const cv2=node('canvas');wrap2.append(cv2);body.append(wrap2);
    const fanPts=pick('fan','#2ee6cf');fanPts.points=fanPts.points.map(([t,v])=>[t,v*100]);
    const speedPts=pick('velocity','#5ef29a');
    const drawn2=lineChart(cv2,[{...fanPts,width:2},{...speedPts,width:1}],{unit:''});
    body.append(node('p','hud-modal-note',drawn2?'Part fan in teal (percent), head speed in green (mm/s), same hour.':'No fan or speed samples in this hour.'));
    const extra=Object.keys(data.points?.at(-1)||{}).filter(k=>!['ts','bed','bed_target','nozzle','nozzle_target','fan','velocity','x','y','z','motors'].includes(k)&&typeof data.points.at(-1)[k]==='number');
    if(extra.length){const wrap3=node('div','hud-chart');const cv3=node('canvas');wrap3.append(cv3);body.append(wrap3);const palette=['#6db8ff','#c48bff','#ff6b6b','#ffd27a'];lineChart(cv3,extra.map((k,i)=>({...pick(k,palette[i%palette.length]),width:1.5})),{unit:''});body.append(node('p','hud-modal-note','Extra sensors: '+extra.join(', ')));}
   }catch(e){note.textContent=e.message;}
  },{wide:true});}

 // ---------- has anything changed ----------
 async function openReports(){modal('reports','Has anything changed?',async(body)=>{
   const s=getState(),p=detailPoint();
   const section=(title)=>{body.append(node('h3','',title));const d=node('div','hud-facts');body.append(d);return d;};
   const fact=(into,k,v,note)=>{const e=node('div','hud-fact');e.append(node('span','',k),node('strong','',v),node('small','',note||''));into.append(e);};
   const findings=diagnose(s.snapshot);
   const f=section('Findings in captured evidence');
   if(findings.length)for(const d of findings)fact(f,d.title.toUpperCase(),d.evidence||'see timeline','');
   else fact(f,'NONE','no matching fault','This does not certify mechanical condition.');
   const mesh=p?.mesh?.grid?.flat().filter(known)||[];
   const g=section('Geometry');
   fact(g,'MESH RANGE',mesh.length?(Math.max(...mesh)-Math.min(...mesh)).toFixed(3)+' mm':'—',mesh.length?p.mesh.profile||'recorded mesh':'no probed mesh in this sample');
   const rounds=(s.snapshot?.rounds||[]).slice().sort((a,b)=>b.ts-a.ts);
   fact(g,'QGL',p?.qgl_applied===true?'applied':p?.qgl_applied===false?'review':'—',rounds[0]?`last round ${number(rounds[0].range_mm,' mm',4)} of ${rounds.length} in window`:'no round in window');
   if(rounds.length>1)fact(g,'QGL DRIFT',(rounds[0].range_mm-rounds[rounds.length-1].range_mm).toFixed(4)+' mm','newest minus oldest in this window');
   fact(g,'Z OFFSET',number(p?.z_offset,' mm',3),'live gcode offset');
   const shaper=p?.telemetry?.config_sections?.find(c=>c.name==='input_shaper')?.fields;
   const r=section('Resonance');
   fact(r,'SHAPER',shaper?`${shaper.shaper_type_x||'?'} / ${shaper.shaper_type_y||'?'}`:'—',shaper?`X ${number(Number(shaper.shaper_freq_x),' Hz')} · Y ${number(Number(shaper.shaper_freq_y),' Hz')}`:'no captured settings');
   const shaperEvents=(s.snapshot?.events||[]).filter(e=>/resonance|shaper/i.test(JSON.stringify(e)));
   fact(r,'TEST EVENTS',String(shaperEvents.length),'recorded console evidence in this window');
   body.append(node('p','hud-modal-note','Klipper writes the raw accelerometer CSV to /tmp on the Pi, which Moonraker will not serve. Drawing the power spectrum here needs our own accelerometer stream; that is the next piece of work, and until it lands these are recorded settings, not a measured test.'));
   const m=section('Motors');
   for(const [key,motor]of Object.entries(p?.motors||{}))fact(m,key.replace('stepper_','').toUpperCase(),number(motor.current,' A',2),Array.isArray(motor.flags)&&motor.flags.length?'FLAGS '+motor.flags.join(', '):(motor.energised===false?'disabled':'no driver flags'));
  },{wide:true});}


 // ---------- pass 4 UI (round four) ----------
 const jget=async(url)=>{const r=await fetch(url);const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||url+' unavailable');return d;};
 const jsend=async(url,method,body)=>{const c=await controls(true);const r=await fetch(url,{method,headers:{'Content-Type':'application/json',...(c.token?{'X-Stella-Control-Token':c.token}:{})},body:body===undefined?undefined:JSON.stringify(body)});const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||'refused');return d;};

 // L2: hotend fan trigger temperature. Klipper reads heater_temp at start only, so this edits
 // printer.cfg through Moonraker, shows the diff, and restarts the firmware after one confirmation.
 function heaterFanEditor(){const wrap=node('div','hud-config-edit');
  (async()=>{try{const cfg=await jget('/api/config/heater-fan');
   wrap.append(node('p','hud-modal-note',`printer.cfg line ${cfg.line??'?'}: heater_temp ${cfg.heater_temp} °C. Bounded ${cfg.min??30} to ${cfg.max??60}; never off, or a hot heatbreak melts its own filament.`));
   const field=tip(numberField(cfg.heater_temp,cfg.min??30,cfg.max??60,1),'Trigger temperature, °C','Type the nozzle temperature that turns this fan on; Preview shows the change.');const diff=node('pre','hud-diff');diff.hidden=true;const status=node('p','hud-command-status');let token=null;
   const preview=button('Preview change',async()=>{try{const v=fieldValue(field);if(v===null)return;const d=await jsend('/api/config/heater-fan','POST',{heater_temp:v});token=d.token;diff.textContent=d.diff||'(no change)';diff.hidden=false;apply.disabled=!token;status.textContent='Review the diff. Apply writes a dated backup, uploads printer.cfg and restarts the firmware.';status.dataset.state='';}catch(e){status.textContent=e.message;status.dataset.state='refused';}},'hud-primary');
   const apply=button('Apply and restart firmware',async()=>{if(!token)return;apply.disabled=true;try{const d=await jsend('/api/config/heater-fan/apply','POST',{token});status.textContent=d.message||'Written. Firmware restarting.';status.dataset.state='reported';token=null;}catch(e){status.textContent=e.message;status.dataset.state='refused';apply.disabled=false;}},'hud-danger');apply.disabled=true;
   tip(preview,'Preview change','Shows the printer.cfg diff. Nothing is written yet.');tip(apply,'Apply and restart firmware','Writes a dated backup, uploads printer.cfg and restarts the firmware.');
   wrap.append(row('Trigger °C',field,preview),diff,apply,status,node('p','hud-modal-note','Klipper quirk: with default options any non-zero nozzle target turns this fan on regardless of the trigger.'));
  }catch(e){wrap.append(node('p','hud-command-status',e.message));}})();return wrap;}

 // L5: presets you can edit, stored on the recorder, not in a browser. Add, rename, change and
 // delete freely; nothing is stored until Save, every unsaved row is marked and counted, and Cancel
 // puts the table back to what is saved. The service checks every row (name, nozzle 0 to 290 °C, bed
 // 0 to 120 °C) and a refusal is shown in its own words with nothing stored. Cool down is built in
 // and locked.
 function openPresets(){modal('presets','Presets',(body)=>{
  body.append(node('p','hud-modal-note','Your preheat presets, saved in Stella Scope on this computer and used by every preheat list. Nozzle 0 to 290 °C, bed 0 to 120 °C. Nothing is saved until you press Save.'));
  const table=node('div','hud-preset-table'),rows=[],saved=Array.isArray(PRESETS?.materials)?PRESETS.materials:[];
  const status=node('p','hud-command-status'),pending=node('span','hud-preset-pending');pending.setAttribute('role','status');
  const save=tip(button('Save',async()=>{const materials=rows.map(({n,t,b})=>({name:n.value.trim(),nozzle:t.value===''?null:Number(t.value),bed:b.value===''?null:Number(b.value)}));save.disabled=true;
   try{const keepLoaded=PRESETS?.loaded?.name&&materials.some(m=>m.name===PRESETS.loaded.name)?PRESETS.loaded:null;await jsend('/api/presets','PUT',{...(PRESETS||{}),materials,loaded:keepLoaded});await loadPresets();modalStack.find(m=>m.id==='presets')?.refresh();feedback('Saved. Every preheat list uses these now.','reported');}
   catch(e){status.textContent='Not saved: '+e.message;status.dataset.state='refused';mark();}},'hud-primary'),'Save','Stores the table in Stella Scope. Every preheat list uses it from then on.');
  const cancel=tip(button('Cancel',()=>modalStack.find(m=>m.id==='presets')?.refresh()),'Cancel','Throws away the unsaved edits and shows the saved presets again.');
  const mark=()=>{const d=presetDiff(saved,rows.map(r=>({orig:r.orig,name:r.n.value,nozzle:r.t.value,bed:r.b.value})));rows.forEach((r,i)=>{r.row.dataset.change=d.states[i];});const n=d.states.filter(x=>x!=='same').length+d.removed;
   body.dataset.dirty=d.changed?'1':'';pending.textContent=d.changed?`${n} unsaved change${n===1?'':'s'}${d.removed?` (${d.removed} deleted)`:''}`:'Saved';save.disabled=cancel.disabled=!d.changed;if(d.changed&&status.dataset.state==='reported')status.textContent='';};
  const addRow=(name='',nozzle='',bed='',orig=null)=>{const n=node('input','hud-num');n.type='text';n.value=name;n.placeholder='name';n.maxLength=24;const t=numberField(nozzle===''?null:nozzle,0,290),b=numberField(bed===''?null:bed,0,120);const r=node('div','hud-row hud-preset-row');const del=button('✕',()=>{r.remove();rows.splice(rows.indexOf(entry),1);mark();},'hud-modal-close');tip(n,'Preset name','Type the name the preheat lists show. Change it here to rename the preset.');tip(t,'Nozzle °C','Type the nozzle preheat, 0 to 290.');tip(b,'Bed °C','Type the bed preheat, 0 to 120.');tip(del,'Delete preset','Takes this preset out when you press Save.');const entry={n,t,b,row:r,orig};rows.push(entry);for(const f of [n,t,b])f.addEventListener('input',mark);r.append(n,t,b,del);table.append(r);return entry;};
  saved.forEach((m,i)=>addRow(m.name,m.nozzle,m.bed,i));
  const head=node('div','hud-row hud-preset-head');head.append(node('span','','Preset'),node('span','','Nozzle °C'),node('span','','Bed °C'),node('span','',''));
  const coolRow=node('div','hud-row hud-preset-row hud-preset-fixed');coolRow.append(node('span','','Cool down'),node('span','','0'),node('span','','0'),node('span','','built in'));tip(coolRow,'Cool down','Built in: sets both heaters to 0 °C. It is always in the PREHEAT menu and cannot be deleted.');
  const add=tip(button('Add preset',()=>{addRow().n.focus();mark();}),'Add preset','Adds an empty row to fill in. It is saved when you press Save.');
  body.append(head,table,coolRow,row('',add,save,cancel,pending),status);if(presetsError)status.textContent=presetsError;mark();
  });}

 // L1: resonance measured here, not on the Pi. Capture streams the accelerometer, the recorder
 // computes the spectrum and the shaper table, and the graph is drawn from those numbers.
 function openResonance(){modal('resonance','Resonance',async(body)=>{
  body.append(node('p','hud-modal-note','Runs TEST_RESONANCES on the printer while streaming the toolhead accelerometer. The gantry must be homed. Each run is kept with its spectrum.'));
  const acts=node('div','hud-calibrations');
  for(const [label,axis]of [['Measure X','X'],['Measure Y','Y'],['Compare belts','belts']])acts.append(tip(button(label,async()=>{if(await send('resonance_capture',{axis})){const m=modalStack.find(m=>m.id==='resonance');setTimeout(()=>m?.refresh(),1500);}}),label,axis==='belts'?'Runs TEST_RESONANCES on both belt diagonals while streaming the accelerometer. Needs a homed printer.':'Runs TEST_RESONANCES on '+axis+' while streaming the accelerometer. Needs a homed printer.','resonance_capture'));
  body.append(acts,reasonLine('resonance_capture'));
  const list=node('div','hud-run-list'),view=node('div','hud-run-view');body.append(node('h3','','Runs'),list,view);
  const show=async(id)=>{view.replaceChildren(node('p','hud-modal-note','Loading spectrum…'));try{const run=await jget('/api/resonance/'+encodeURIComponent(id));view.replaceChildren();
   const canvas=node('canvas','hud-chart');view.append(canvas);
   if(run.status==='running'){view.append(node('p','hud-modal-note','Sweep running on the printer. This page refreshes on its own.'));setTimeout(()=>{if(document.querySelector('.hud-modal[open][data-id=resonance]'))show(id);},5000);return;}
   if(run.status==='failed'){view.append(node('p','hud-command-status',run.error||'The capture failed.'));return;}
   if(run.belts){psdChart(canvas,{freq:run.psd.freq,traces:[{label:'belt A',values:run.belts.a,color:'#ff9a3c'},{label:'belt B',values:run.belts.b,color:'#6db8ff'}],peaks:run.peaks||[],title:`Belts · similarity ${known(run.belts.similarity)?(run.belts.similarity*100).toFixed(1)+'%':'—'}`});
    view.append(node('p','hud-modal-note',known(run.belts.similarity)&&run.belts.similarity>=.9?'Matched. The same peaks at the same height means the belts share one tension.':'The curves differ. Re-tension the lower belt toward 110 Hz over 15 cm and run again.'));}
   else{psdChart(canvas,{freq:run.psd.freq,traces:[{label:'X',values:run.psd.x,color:'#ff6b6b'},{label:'Y',values:run.psd.y,color:'#5ef29a'},{label:'Z',values:run.psd.z,color:'#6db8ff'},{label:'sum',values:run.psd.sum,color:'#e8f1ef',width:2}],peaks:run.peaks||[],title:`Axis ${run.axis} · ${new Date(run.ts*1000).toLocaleString()}`});
    const t=node('table','hud-shaper-table');t.append(node('caption','','Fitted shapers. The recommended one is the lowest vibration at acceptable smoothing, the way Klipper picks it.'));const th=node('tr');for(const h of ['Shaper','Hz','Vibration','Smoothing','Max accel',''])th.append(node('th','',h));t.append(th);
    for(const sh of run.shapers||[]){const tr=node('tr');tr.classList.toggle('best',Boolean(sh.recommended)||(run.recommended&&sh.type===run.recommended.type));for(const v of [sh.type,number(sh.freq,'',1),number(sh.vibrations_pct??sh.vibrations,'%',1),number(sh.smoothing,'',3),number(sh.max_accel,'',0)])tr.append(node('td','',v));
     const td=node('td');td.append(tip(button('Use',()=>send('console',{script:`SET_INPUT_SHAPER SHAPER_TYPE_${run.axis}=${sh.type} SHAPER_FREQ_${run.axis}=${Number(sh.freq).toFixed(1)}`})),'Use '+sh.type,'Sets this shaper live with SET_INPUT_SHAPER after one review. SAVE_CONFIG makes it permanent.','console'));tr.append(td);t.append(tr);}
    view.append(t,node('p','hud-modal-note',`${run.psd?.window?'Window '+run.psd.window+', ':''}${known(run.psd?.sample_rate_hz)?run.psd.sample_rate_hz.toFixed(0)+' Hz sampling, ':''}${known(run.psd?.welch_windows)?run.psd.welch_windows+' Welch windows. ':''}Use sets the shaper live. SAVE_CONFIG in the console writes it to printer.cfg and restarts Klipper.`));}
  }catch(e){view.replaceChildren(node('p','hud-command-status',e.message));}};
  try{const d=await jget('/api/resonance');if(!d.runs?.length)list.append(node('p','hud-modal-note','No run captured yet.'));
   for(const r of d.runs){const b=button('','',null);b.className='hud-file';tip(b,r.axis+' run','Shows this run\'s spectrum and fitted shapers.');b.replaceChildren(node('strong','',`${r.axis} · ${r.status||''}`),node('small','',`${new Date(r.ts*1000).toLocaleString()}${known(r.peak_hz)?' · peak '+r.peak_hz.toFixed(1)+' Hz':''}${r.recommended?' · '+r.recommended.type+' '+Number(r.recommended.freq).toFixed(1)+' Hz':''}`));b.onclick=()=>show(r.id);list.append(b);}
   if(d.runs?.[0])show(d.runs[0].id);
  }catch(e){list.append(node('p','hud-command-status',e.message));}
  },{wide:true});}

 // L6: real toolpaths on the plate. Parsed in the browser from the file Moonraker holds.
 let gcodeModule=null,toolpathFile=null,toolpathParsed=null;
 // `say` is where progress is written: the open dialog's status line when a file is clicked in Files,
 // the toast when the HUD loads the printing file by itself.
 async function showToolpath(path,say=feedback){gcodeModule??=await import('./hud-gcode.js');const scene=getScene();if(!scene)return;
  say('Reading '+path.split('/').pop()+'…','pending');
  const r=await fetch('/api/print/gcode?filename='+encodeURIComponent(path));if(!r.ok)throw Error('G-code unavailable from the printer.');const text=await r.text();
   toolpathParsed=gcodeModule.parseGcode(text,{maxSegments:1500000});toolpathFile={path,bytes:toolpathParsed.byteLength||new TextEncoder().encode(text).length};lastLayerText='';
  // Travels are no longer drawn, so a file with no extruding move has nothing to show. The scene
  // refuses a toolpath until it knows where the bed is, which is three seconds of CAD after a
  // cold load — say which of the two it was instead of blaming the file.
  const info=scene.loadToolpath(toolpathParsed,path);if(!info)throw Error(scene.bedCenter?'That file has no extruding moves to draw.':'The printer model is still loading; the toolpath follows.');scene.associateJob(path);
  say(`${info.layers} layers, ${info.segments.toLocaleString()} moves${toolpathParsed.truncated?' (truncated)':''}.`,'reported');}
 // He wants to watch the print, not go and find the file first. When a job is running and its
 // toolpath is not the one on the plate, fetch and draw it once, on its own. Guarded by
 // autoToolpath so a failure (or a file Moonraker will not hand over) is not retried every
 // tick, and so switching files re-arms it exactly once.
  let autoToolpath=null,notStored=null;
  // The wait after each failed automatic load, in seconds; the last one repeats.
  const TP_BACKOFF_S=[5,15,60];let tpFail=0,tpNext=0,lastLink=null;
  // The layer readout in the job strip. progressToLayer already runs for the toolpath, so the
  // layer is captured here and the strip renders it; no second parse, no new state.
  let lastLayerText='',lastLayer=null;
 function toolpathAuto(current,state){
  const name=current?.name||'';if(!name||state!=='printing'&&state!=='paused')return;
  const file=name.split('/').pop();
  if(autoToolpath===file||toolpathFile?.path.split('/').pop()===file)return;
  // Found this retrying on every tick and writing over open dialogs. Now: no fetch at all while the
  // printer is off; a failure waits 5 s, then 15 s, then 60 s between tries, is reported once in the
  // toast and never over a dialog, and the next try comes by itself (at once when the printer is back,
  // see update). a replay or a rewound view draws only what the recording holds and never asks the
  // printer for a file. The recording keeps each print's slicer metadata and ten Orca settings
  // (print_context), not its G-code, so it says so once per file instead of fetching. Nothing is
  // fetched either while the page cannot reach its own service.
  if(link().state==='replay'||!getState().live){if(notStored!==file){notStored=file;toast(`G-code not stored in the recording: the toolpath of ${file} cannot be drawn here.`);}return;}
  if(link().state==='service')return;
  if(link().state==='off'||performance.now()<tpNext)return;
  autoToolpath=file;
  // A failed attempt used to leave toolpathFile pointing at the file it had just failed on, and the
  // guard on the line above then refused every retry for the rest of the print: open the HUD while
  // a job is running and the plate stayed empty for good. Clear both, so the next tick tries again.
  showToolpath(name,(text,state)=>{if(state==='reported')toast(text);}).then(()=>{tpFail=0;tpNext=0;}).catch(e=>{autoToolpath=null;toolpathFile=null;toolpathParsed=null;
   const wait=TP_BACKOFF_S[Math.min(tpFail,TP_BACKOFF_S.length-1)];tpNext=performance.now()+wait*1000;if(++tpFail===1)toast(`${e.message} Trying again in ${wait} s, then less often.`);});
 }
  function toolpathTick(current){if(!toolpathParsed||!toolpathFile||!gcodeModule)return;const name=current?.name||'';if(name&&name.split('/').pop()!==toolpathFile.path.split('/').pop())return;
   // Parts of the cube read as finished before the head got there. The byte the printer is
   // reading drives it now; the progress ratio is only the fallback for a sample that carries no
   // file position (older recordings).
   const p=point(),progress=known(current?.progress)?clamp(current.progress,0,1):0;
   const at=known(p?.file_position)?Math.round(p.file_position):Math.round(progress*toolpathFile.bytes);
   const step=gcodeModule.progressToLayer(toolpathParsed,at);
   lastLayerText=current?.state==='printing'&&toolpathParsed.totalLayers&&step.layer>=0?`layer ${Math.min(step.layer+1,toolpathParsed.totalLayers)}/${toolpathParsed.totalLayers}`:'';lastLayer=lastLayerText?[Math.min(step.layer+1,toolpathParsed.totalLayers),toolpathParsed.totalLayers]:null;
   getScene()?.toolpathProgress({...step,filePosition:at,live:p?.position});}

  // L3: exclude objects while a print runs.
  async function openObjects(){const entry=modal('objects','Exclude objects',async(body)=>{
    body.append(node('p','hud-modal-note','Objects Klipper knows in the running file. Excluding one skips it for the rest of the print. It cannot be undone.'));
    // Closing the modal drops the plate highlight, so an object never stays marked after the
    // modal that marked it is gone.
    entry.onClose=()=>getScene()?.highlightObject(null);
    try{const d=await jget('/api/print/objects');const list=node('div','hud-macro-list');
     for(const o of d.objects||[]){const row=node('div','hud-object-row');
      const b=button(o.name+(o.excluded?' · excluded':''),()=>send('exclude_object',{name:o.name}),o.excluded?'':'hud-danger');b.disabled=Boolean(o.excluded);tip(b,o.name,o.excluded?'Already excluded from this print.':'Skips this object for the rest of the print. It cannot be undone.','exclude_object');
      row.append(b);
      // When the drawn toolpath carries this object (slicer labeled it and the file was parsed),
      // "Show on plate" marks its moves in magenta on the bed — the scene answers which one it
      // is, not the name. Names match case-insensitively: the parser reports the sliced name,
      // Klipper the defined one, and they disagree on case for the same object.
      if(toolpathParsed?.objects?.some(p=>String(p.name).toLowerCase()===String(o.name).toLowerCase()))row.append(tip(button('Show on plate',()=>getScene()?.highlightObject(o.name)),'Show on plate','Marks this object\'s moves in magenta on the bed.'));
      list.append(row);}
     body.append(list.childElementCount?list:node('p','hud-modal-note','No objects reported. The slicer must label objects (Orca: Label objects on).'));}catch(e){body.append(node('p','hud-command-status',e.message));}
   });}

 // L7: histories with evidence links.
 async function historySection(body,title,url,draw){body.append(node('h3','',title));const host=node('div','hud-history');body.append(host);
  try{const d=await jget(url);await draw(host,d);}catch(e){host.append(node('p','hud-modal-note',e.message));}}
 const evidenceLinks=(into,ev)=>{if(!Array.isArray(ev)||!ev.length)return;const r=node('div','hud-evidence');for(const e of ev.slice(0,6))r.append(tip(button(`${e.kind||'sample'} ${e.ts?new Date(e.ts*1000).toLocaleTimeString():''}`,()=>{if(known(e.ts))document.dispatchEvent(new CustomEvent('stella-seek',{detail:{ts:e.ts}}));}),'Evidence: '+(e.kind||'sample'),'Rewinds the recording to this moment.'));into.append(r);};
 async function openHistories(){modal('histories','Histories',async(body)=>{
  body.append(node('p','hud-modal-note','Every recorded run, newest first. Click an evidence chip to rewind the recording to that moment.'));
  const chart=(host,rows,pick,unit,color)=>{const pts=rows.map(r=>[r.ts,pick(r)]).filter(p=>known(p[0])&&known(p[1]));if(pts.length<2)return;const c=node('canvas','hud-chart');host.append(c);lineChart(c,[{points:pts.slice().sort((a,b)=>a[0]-b[0]),color}],{unit,height:150});};
  const when=ts=>known(ts)?new Date(ts*1000).toLocaleString():'—';
  await historySection(body,'Bed mesh range','/api/history/mesh',(host,d)=>{const rows=d.meshes||[];if(!rows.length)return host.append(node('p','hud-modal-note','No saved mesh in the recording.'));chart(host,rows,r=>r.range_mm,' mm','#ff9a3c');
   for(const r of rows.slice(0,8)){const e=node('div','hud-fact');e.append(node('span','',when(r.ts)),node('strong','',number(r.range_mm,' mm',3)),node('small','',`${r.profile||'mesh'}${r.adaptive?' (adaptive)':''} · ${r.rows}×${r.cols}${r.worst_corner?.corner?' · worst '+r.worst_corner.corner+' '+number(r.worst_corner.from_mean_mm,' mm',3):''}`));evidenceLinks(e,r.evidence);host.append(e);}});
  await historySection(body,'Z offset','/api/history/z-offset',(host,d)=>{const rows=d.changes||[];if(!rows.length)return host.append(node('p','hud-modal-note','No Z offset change recorded.'));chart(host,rows,r=>r.applied_z_mm,' mm','#6db8ff');
   for(const r of rows.slice(0,8)){const e=node('div','hud-fact');e.append(node('span','',when(r.ts)),node('strong','',number(r.applied_z_mm,' mm',3)),node('small','',`${r.kind||''} ${r.message||''}`.trim()));evidenceLinks(e,r.evidence);host.append(e);}
   for(const z of (d.zero_references||[]).slice(0,3)){const e=node('div','hud-fact');e.append(node('span','',when(z.ts)),node('strong','',number(z.z_mm,' mm',3)),node('small','','zero reference · '+(z.context||'')));evidenceLinks(e,z.evidence);host.append(e);}});
  await historySection(body,'Gantry level','/api/history/qgl',(host,d)=>{const rows=d.runs||[];if(!rows.length)return host.append(node('p','hud-modal-note','No QGL run recorded.'));chart(host,rows.map(r=>({ts:r.last_ts,v:r.final_range_mm})),r=>r.v,' mm','#5ef29a');
   for(const r of rows.slice(0,8)){const e=node('div','hud-fact');e.append(node('span','',when(r.last_ts)),node('strong','',number(r.final_range_mm,' mm',4)),node('small','',`run ${r.run_id} · ${r.round_count} round${r.round_count===1?'':'s'} · ${r.pass?'pass':r.aborted?'aborted':'not within'} ${number(r.tolerance_mm,' mm',3)}`));evidenceLinks(e,r.evidence);host.append(e);}});
  await historySection(body,'Motor drift','/api/history/motor-drift',(host,d)=>{const rows=d.motors||[];if(!rows.length)return host.append(node('p','hud-modal-note','No driver telemetry recorded.'));
   for(const r of rows){const e=node('div','hud-fact');e.append(node('span','',(r.label||r.motor||'').toUpperCase()),node('strong','',known(r.drift)?(r.drift>=0?'+':'')+number(r.drift,'',3):'—'),node('small','',`${r.samples||0} samples · ${r.flag_events||0} driver flags${Array.isArray(r.flags_seen)&&r.flags_seen.length?' · '+r.flags_seen.join(', '):''}`));evidenceLinks(e,r.evidence);host.append(e);}});
  await historySection(body,'Thermal','/api/history/thermal',(host,d)=>{const rows=d.sessions||[];if(!rows.length)return host.append(node('p','hud-modal-note','No heat-up recorded.'));
   for(const [heater,dr]of Object.entries(d.drift||{})){if(!dr||!known(dr.mean_c_per_s))continue;const e=node('div','hud-fact');e.append(node('span','',heater.replace('heater_','').toUpperCase()+' TREND'),node('strong','',number(dr.latest_c_per_s,' °C/s',3)),node('small','',`mean ${number(dr.mean_c_per_s,' °C/s',3)} over ${dr.sessions} heat-ups${known(dr.latest_vs_mean_pct)?' · latest '+(dr.latest_vs_mean_pct>=0?'+':'')+dr.latest_vs_mean_pct.toFixed(0)+'%':''}`));host.append(e);}
   for(const r of rows.slice(0,10)){const e=node('div','hud-fact');e.append(node('span','',`${(r.heater||'').replace('heater_','').toUpperCase()} ${when(r.ts)}`),node('strong','',number(r.heat_up_c_per_s,' °C/s',3)),node('small','',`${number(r.start_c,'',0)} → ${number(r.target_c,' °C',0)} in ${number(r.seconds_to_target,' s',0)}${known(r.overshoot_c)?' · overshoot '+number(r.overshoot_c,' °C',2):''}`));evidenceLinks(e,r.evidence);host.append(e);}});
  },{wide:true});}
 // One door for every test the machine runs on itself, plus the recorded results. Every run goes
 // through the same send as everywhere else, so the no-review list and the service-side T3 gates
 // apply here unchanged; marker: hud-testing-modal.
 function openTesting(){modal('testing','Testing',(body)=>{
   body.append(node('p','hud-modal-note','Each of these moves the printer. Nothing starts by itself.'));
   const acts=node('div','hud-calibrations');
   for(const id of ['probe_accuracy','mesh','resonance_x','resonance_y','shaper_x','shaper_y']){
    const def=CALIBRATIONS.find(c=>c[0]===id);const b=tip(button(def[1],()=>send(id)),def[1],def[2]+' '+SENDS,id);b.disabled=!can(id);acts.append(b);}
   body.append(acts,reasonLine('mesh'));
   const grid=node('div','hud-more-grid');
   for(const [label,note,fn]of [
    ['Resonance results','spectra, fitted shapers, belt compare',openResonance],
    ['Bed mesh','the probed surface in 3D',openMesh],
    ['Histories','mesh, Z, QGL, motors, heat over time',openHistories],
    ['Reports','has anything changed',openReports]]){
    const b=button('','',null);b.className='hud-more';b.replaceChildren(node('strong','',label),node('small','',note));
    tip(b,label,'Closes Testing and opens '+label+': '+note+'.');b.onclick=()=>{const m=modalStack.find(m=>m.id==='testing');if(m)closeModal(m);fn();};grid.append(b);}
   body.append(node('h3','','Recorded results'),grid);
  });}
 function openMore(){modal('more','More',(body)=>{
   const grid=node('div','hud-more-grid');
   for(const [label,note,fn]of [
    ['Filament','load, unload, extrude',openFilament],
    ['Macros','your own Klipper macros',openMacros],
    ['Tune','limits and pressure advance',openTune],
    ['Bed mesh','the probed surface in 3D',openMesh],
    ['Resonance','measure and fit input shaping',openResonance],
    ['Histories','mesh, Z, QGL, motors, heat over time',openHistories],
    ['History','temperatures over time',openHistory],
    ['Reports','has anything changed',openReports],
    ['Presets','edit the preheat table',openPresets],
    ['Motion','jog, home, Z offset',openMotion],
    ['Restart','firmware restart, motors off',openRestart]]){
    const b=button('','',null);b.className='hud-more';b.replaceChildren(node('strong','',label),node('small','',note));
    tip(b,label,'Opens '+label+': '+note+'.');b.onclick=()=>{const m=modalStack.find(m=>m.id==='more');if(m)closeModal(m);fn();};grid.append(b);}
   body.append(grid);
  });}

 function openRestart(){modal('restart','Restart',(body)=>{
   body.append(node('p','hud-modal-note','Each of these interrupts whatever the printer is doing.'));
   const acts=node('div','hud-calibrations');
   acts.append(tip(button('Klipper restart',()=>send('klipper_restart')),'Klipper restart','Restarts the Klipper host (RESTART). Asks once.','klipper_restart'),tip(button('Firmware restart',()=>send('firmware_restart')),'Firmware restart','Restarts Klipper and its MCUs (FIRMWARE_RESTART). Asks once.','firmware_restart'),tip(button('Motors off',()=>send('motors_off')),'Motors off','Turns the steppers off (M84) so the gantry moves by hand.','motors_off'),tip(button('Reconnect recorder',()=>send('reconnect')),'Reconnect recorder','Reconnects Stella Scope to Moonraker. Nothing is sent to the printer.','reconnect'));
   body.append(acts,tip(button('EMERGENCY STOP',()=>send('emergency_stop'),'hud-danger hud-primary'),'Emergency stop','Stops Klipper at once through Moonraker\'s emergency-stop call, with no review.','emergency_stop'));
  });}

 // ---------- per-second update ----------
 let lastSnapshot=null;
 const point=()=>{const s=getState();return (s.live?s.tick?.point:null)||s.snapshot?.point;};
 // Ticks are the fast path and carry no mesh or signal detail; the snapshot does.
 const detailPoint=()=>{const s=getState();const p=point(),d=s.snapshot?.point;return d?.mesh||d?.telemetry?{...d,...(p||{}),mesh:d.mesh,telemetry:d.telemetry}:p;};
 const job=()=>{const s=getState();return (s.live?s.tick?.job:null)||s.snapshot?.point?.job;};
 function update(){const s=getState(),p=(s.live?s.tick?.point:null)||s.snapshot?.point,tick=s.live?s.tick:null,current=tick?.job||p?.job||s.snapshot?.point?.job;
  const geo=s.snapshot?.geometry;machineName.textContent=geo?.machine||'PRINTER';machineSpec.textContent=geo?.bed_size_confirmed===true?`${geo.bed_size_mm?.join(' × ')||'—'} MM BED`:'BED SIZE UNVERIFIED';
  main.hidden=false;let etaLeft=null;
  const mode=s.snapshot?.observer_now?.mode==='replay'?'replay':s.live?'live':'rewind';
  const activity=tick?.activity?.label||$('activity-label')?.textContent||'';
  const jobState=(current?.state||'unknown').toLowerCase();
  const heating=(known(p?.nozzle?.target)&&p.nozzle.target>0&&p.nozzle.temperature<p.nozzle.target-3)||(known(p?.bed?.target)&&p.bed.target>0&&p.bed.temperature<p.bed.target-3);
  const sceneState=jobState==='printing'?'printing':jobState==='paused'?'paused':jobState==='error'?'error':heating?'heating':'idle';
   cockpit.dataset.state=sceneState;cockpit.dataset.mode=mode;
   // The scene's key light leans with what the machine is doing (heating warms, printing
   // brightens, error washes magenta). Purely a lighting cue; the state pill stays the source of
   // truth in text.
   getScene()?.stage(sceneState);
  statePill.textContent=(mode==='replay'?'RECORDING · ':mode==='rewind'?'REWIND · ':'')+(sceneState==='idle'?activity||'Idle':sceneState).toUpperCase();
  // The printer's state drives the header, the banner and every control. When it comes back, the
  // control list is fetched fresh and the printing file is tried at once. The service's command
  // list is read again whenever the printer can take something new: back to ready, or halted with
  // the restarts open (reviewer's decision on ).
  const l=link(),linkKey=l.state+(l.recover?'+recover':'');if(linkKey!==lastLink){const was=lastLink;lastLink=linkKey;if(l.state==='ready'){tpFail=0;tpNext=0;}if(was&&(!l.lock||l.recover))controls(true);}
  lockControls();
  // Live, the age is the newest sample's own time against the clock: the last tick's age froze at
  // "fresh" when the ticks stopped, which read wrong beside PRINTER OFF.
  const age=mode==='live'&&known(p?.ts)?Math.max(0,Date.now()/1000-p.ts):tick?.age_seconds??s.snapshot?.age_seconds;fresh.textContent=known(age)?(age<5?'fresh':age<120?number(age,' s old',0):number(age/3600,' h old',1)):'';
  paintControls(l);
  loadedChip.textContent=PRESETS?.loaded?.name?PRESETS.loaded.name+' loaded':'';
  const blocked=!caps.enabled||!s.live||l.lock;
  offline.dataset.on=blocked?'1':'';offline.dataset.link=l.state;
  offline.querySelector('strong').textContent=l.state==='service'?'SERVICE NOT RUNNING':mode==='replay'?'SAVED RECORDING':l.state==='off'?'PRINTER OFF':l.state==='not_ready'?'KLIPPER NOT READY':!s.live?'REWOUND':'CONTROLS OFF';
  offlineText.textContent=l.state==='service'?`Stella Scope service is not running${l.since?' (no answer for '+l.since+')':''}. This view stays as it was; nothing new arrives and no command can go. The page tries again by itself, 1 s, 2 s, 5 s, then every 10 s, and picks up at once when the service answers. The printer itself may be fine.`
   :mode==='replay'?'Saved telemetry. This replay instance cannot send printer commands.'
   :l.state==='off'?`The printer is not answering${l.since?'; last data '+l.since+' ago':''}. You can look at everything; the controls unlock by themselves once it is on and Klipper is ready.`
   :l.state==='not_ready'?l.reason+(l.recover?' You can look at everything; the emergency stop, FW RESTART and Klipper restart (More > Restart) still go.':' You can look at everything; only the emergency stop still goes.')
   :!s.live?'Rewound to an earlier moment. Press Live to arm the controls again.'
   :(caps.reason||'Controls are unavailable.');
  // job strip
  jobName.textContent=current?.name?.split('/').pop()||'No job reported';const progress=known(current?.progress)?clamp(current.progress,0,1):null;
  [...jobBar.children].forEach((c,i)=>c.classList.toggle('on',progress!==null&&(i+.5)/40<=progress));jobStrip.dataset.state=jobState;
   const eta=progress&&progress>0.02&&known(current?.duration)&&jobState==='printing'?current.duration/progress-current.duration:null;etaLeft=eta;jobMeta.textContent=(progress===null?'':(progress*100).toFixed(1)+'% · ')+(known(current?.duration)?number(current.duration/60,' min',0):'')+(known(eta)?' · '+(eta>=3600?number(eta/3600,' h left',1):number(eta/60,' min left',0)):'')+(known(current?.filament_used)&&current.filament_used>0?' · '+number(current.filament_used/1000,' m filament',2):'')+(lastLayerText?' · '+lastLayerText:'');
  getScene()?.jobUpdate(current);getScene()?.live(p);toolpathAuto(current,jobState);toolpathTick(current);
  // gauges
  nozzle.set(p?.nozzle?.temperature,p?.nozzle?.target,'target '+number(p?.nozzle?.target,'°',0)+' · power '+number(known(p?.nozzle?.power)?p.nozzle.power*100:null,'%',0));
  bed.set(p?.bed?.temperature,p?.bed?.target,'target '+number(p?.bed?.target,'°',0)+' · power '+number(known(p?.bed?.power)?p.bed.power*100:null,'%',0));
  // Glow only on live values, and a heater drawing power breathes. Rewound or replayed: still.
  const liveNow=mode==='live'&&known(age)&&age<5;document.body.dataset.fresh=liveNow?'1':'';
  nozzle.el.dataset.drawing=liveNow&&known(p?.nozzle?.power)&&p.nozzle.power>0?'1':'';bed.el.dataset.drawing=liveNow&&known(p?.bed?.power)&&p.bed.power>0?'1':'';
  const fans=fansAt(p);const part=fans.find(f=>f.object==='fan');partFan.set(known(part?.command)?part.command*100:null,null,part?.rpm==null?'commanded output':number(part.rpm,' RPM',0));partGlyph.set(part?.command);
  for(const [key,row]of fanRows){const f=fans.find(f=>f.object===key||f.object.endsWith(' '+key));row.glyph.set(f?.command);row.gauge.set(known(f?.command)?f.command*100:null,null,'');}
  const ctrl=fans.find(f=>/controller/.test(f.object));
  ctrlFan.querySelector('b').textContent=known(ctrl?.command)?Math.round(ctrl.command*100)+'%':'—';
  ctrlFan.dataset.on=known(ctrl?.command)&&ctrl.command>0?'1':'';
  for(const [i,a]of ['X','Y','Z'].entries())roll(axes[a],p?.position?.[i],v=>number(v,'',1));
  reticleText.textContent=known(p?.position?.[2])?'Z '+number(p.position[2],'',2):'';
  const h=p?.homed||'';homed.textContent=h.length>=3?'HOMED':h?'HOMED '+h.toUpperCase():'NOT HOMED';homed.dataset.ok=h.length>=3?'1':'';
  // Live motion beside X, Y and Z, and the bottom strip. Unknown is a dash, never zero.
  velocityRead.set(p?.velocity,{unit:'mm/s',note:known(p?.max_velocity)?'max '+number(p.max_velocity,'',0):''});
  accelRead.set(p?.max_accel,{unit:'mm/s²'});
  const ceiling=flowCeiling(PRESETS?.loaded?.name,PRESETS?.hotend_max_flow),vol=volumetricFlow(p?.extruder_velocity),share=known(vol)&&known(ceiling.max)?vol/ceiling.max:null,flowState=share===null?'':share>1?'over':share>=.8?'near':'ok';
  volumeRead.set(vol,{digits:1,unit:'mm³/s',note:ceiling.max===null?'measured limit not set':'of '+ceiling.max+' mm³/s saved limit',state:flowState});flowBar.firstChild.style.setProperty('--f',known(share)?Math.min(1,share).toFixed(3):'0');
  const fanRead=(r,f)=>{if(known(f?.rpm))r.set(f.rpm,{unit:'rpm',note:known(f?.command)?'cmd '+Math.round(f.command*100)+'%':''});else r.set(known(f?.command)?f.command*100:null,{unit:'% cmd'});};
  fanRead(stripCtrl,ctrl);fanRead(stripHotend,fans.find(f=>f.object==='hotend_fan'||f.object.endsWith(' hotend_fan')));fanRead(stripPart,part);
  const asked=known(p?.requested_speed)&&known(p?.speed_factor)?p.requested_speed*p.speed_factor:null,moving=jobState==='printing'||jobState==='paused';
  stripSpeed.set(moving?asked:null,{unit:'mm/s',note:moving&&known(p?.velocity)?'live '+number(p.velocity,'',0):''});
  stripFlow.set(moving?vol:null,{digits:1,unit:'mm³/s',note:known(share)&&moving?Math.round(share*100)+'%':'',state:moving?flowState:''});
  const layer=known(current?.layer)?[current.layer,current.layers]:lastLayer;stripLayer.set(moving&&layer?layer[0]:null,{unit:layer&&known(layer[1])?'/ '+layer[1]:'',note:''});
  stripEta.set(known(etaLeft)?etaLeft/60:null,{unit:'min',note:known(etaLeft)?new Date((known(p?.ts)?p.ts*1000:Date.now())+etaLeft*1000).toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'}):''});
  speed.set(known(p?.speed_factor)?p.speed_factor*100:null,null,'M220');flow.set(known(p?.extrude_factor)?p.extrude_factor*100:null,null,'M221');
  // status strip
  const mesh=p?.mesh?.grid?.flat().filter(known)||[];const rounds=(s.snapshot?.rounds||[]).slice().sort((a,b)=>b.ts-a.ts)[0];const shaper=p?.telemetry?.config_sections?.find(c=>c.name==='input_shaper')?.fields;
  const findings=diagnose(s.snapshot);
  status.replaceChildren(node('b','',findings.length?findings.length+' finding'+(findings.length>1?'s':''):'no faults in evidence'),node('span','','MESH '+(mesh.length?number(Math.max(...mesh)-Math.min(...mesh),' mm',3):'—')),node('span','','QGL '+(p?.qgl_applied===true?'applied':p?.qgl_applied===false?'review':'—')+(rounds?' '+number(rounds.range_mm,' mm',3):'')),node('span','','SHAPER '+(shaper?`${shaper.shaper_type_x||'?'} ${number(Number(shaper.shaper_freq_x),'',1)} / ${shaper.shaper_type_y||'?'} ${number(Number(shaper.shaper_freq_y),'',1)} Hz`:'—')),node('span','','Z OFFSET '+number(p?.z_offset,' mm',3)));
  status.dataset.alert=findings.length?'1':'';
  if($('detail-dialog').open)componentLive(s.detail);
  if(lastSnapshot!==s.snapshot){lastSnapshot=s.snapshot;}
  if(s.page!=='machine'){for(const b of drawerNav.children)b.classList.toggle('on',b.dataset.page===s.page);if(!drawer.open)drawer.showModal();}else if(drawer.open)drawer.close();
  document.body.dataset.hudPage=s.page;
 }

 // ---------- part cards ---------- the user, clicking parts in their own window. Every card reads
 // hud-core's PART_WIRING, so a part lists only what is wired to or measured on it. A part with
 // nothing wired gets no card when clicked: the part chip (paint, hide, isolate, zoom) opens
 // instead, and its specifications and source stay one click away under Details. The bed shows the
 // newest mesh in the recording, dated, instead of whatever the sample at the cursor carries. Tabs
 // only appear with something in them. The look is the style decisions in -STOP-REPORT.md.
 const card={id:null,wiring:null,widgets:null,tab:'signals',signalsBody:null};
 // Where the part was pressed. pointerdown, because the scene selects on pointerup before any
 // listener here would hear it.
 const pick={x:null,y:null};$('machine-view').addEventListener('pointerdown',e=>{pick.x=e.clientX;pick.y=e.clientY;});
 const reduced=()=>matchMedia('(prefers-reduced-motion: reduce)').matches;
 const val=(sigs,object,field)=>sigs.find(s=>s.object===object&&s.field===field)?.value;
 function wiringFor(entry){const scene=getScene(),b=scene.geometryBounds(entry.id);return partWiring(entry,b?[(b.min[0]+b.max[0])/2,(b.min[1]+b.max[1])/2]:null,scene.bedCenter);}
 // The CAD node a part sits in, less its hardware. The LCD panel is nine printed and electronic
 // meshes (plus eleven screws and inserts) over three colour groups, and its card painted only the
 // one mesh clicked, usually the screen: that is why the user could not repaint the panel. The
 // whole assembly is now one tick away. The node is the first place the part's name appears in its
 // path.
 function assemblyOf(entry){const f=(entry.full||'').split(' / '),i=f.indexOf(entry.name);if(i<1)return null;const path=f.slice(0,i).join(' / ')+' / ',members=getScene().entries.filter(e=>(e.full||'').startsWith(path)&&!/shcs|bhcs|fhcs|screw|bolt|nut|washer|insert|magnet/i.test(e.name||''));return members.length>1&&members.length<=60?{name:f[i-1].replace(/:\d+$/,'').replace(/(\s*\(\d+\))+$/,''),members}:null;}
 function paintControl(entry){const scene=getScene(),wrap=node('div','hud-paint'),picker=node('input'),status=node('small'),asm=assemblyOf(entry),whole=asm?node('input'):null;
  picker.type='color';picker.value=scene.color(entry.id)||'#82949c';picker.setAttribute('aria-label','Colour for '+entry.name);tip(picker,'Part colour','Opens a colour picker. The colour is saved in this browser only; nothing goes to the printer.');wrap.append(node('span','','PAINT'),picker);
  if(whole){whole.type='checkbox';whole.checked=/assembly/i.test(asm.name);const scope=node('label','hud-paint-scope');for(const el of [scope,whole])tip(el,'Paint the whole '+asm.name,'When ticked, the colour goes to all '+asm.members.length+' parts in it.');scope.append(whole,document.createTextNode(`whole ${asm.name} (${asm.members.length})`));wrap.append(scope);}
  picker.oninput=()=>{try{for(const m of whole?.checked?asm.members:[entry])scene.setColor(m.id,picker.value);status.textContent=whole?.checked?asm.members.length+' parts painted':'saved';}catch(e){status.textContent=e.message;}};
  wrap.append(status);return wrap;}
 // An instrument dial: 250° sweep, ticks, needle and target mark. The arc, needle and number ease
 // between readings over 650 ms instead of snapping. Unknown is drawn as unknown, never as zero.
 let dialCount=0;const A0=145,SWEEP=250,R=58,C=74,polar=(a,r)=>[C+r*Math.cos(a*Math.PI/180),C+r*Math.sin(a*Math.PI/180)];
 const arcPath=(a,b,r)=>{const [x0,y0]=polar(a,r),[x1,y1]=polar(b,r);return `M${x0.toFixed(2)} ${y0.toFixed(2)}A${r} ${r} 0 ${b-a>180?1:0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;};
 function dial(label,unit,min,max,digits=0){const el=node('div','hud-dial'),s=svg('svg');s.setAttribute('viewBox','0 0 148 148');s.setAttribute('aria-hidden','true');
  const add=(tag,cls,attrs)=>{const e=svg(tag);e.setAttribute('class',cls);for(const [k,v]of Object.entries(attrs))e.setAttribute(k,v);s.append(e);return e;};
  // A glass dial face: a radial gradient under the bezel, lighter at the rim, like a gauge lens.
  const face='hud-dial-face-'+(++dialCount),defs=svg('defs'),grad=svg('radialGradient');grad.id=face;grad.setAttribute('cx','50%');grad.setAttribute('cy','42%');grad.setAttribute('r','62%');
  for(const [o,c]of [['0%','hud-dial-face-in'],['100%','hud-dial-face-out']]){const st=svg('stop');st.setAttribute('offset',o);st.setAttribute('class',c);grad.append(st);}defs.append(grad);s.append(defs);
  add('circle','hud-dial-face',{cx:C,cy:C,r:70,fill:`url(#${face})`});add('circle','hud-dial-bezel',{cx:C,cy:C,r:71});add('path','hud-dial-track',{d:arcPath(A0,A0+SWEEP,R)});
  for(const [a,v,anchor]of [[A0,min,'start'],[A0+SWEEP,max,'end']]){const [x,y]=polar(a,R-24),l=add('text','hud-dial-end',{x:x.toFixed(1),y:(y+4).toFixed(1),'text-anchor':anchor});l.textContent=v;}
  for(let i=0;i<=20;i++){const a=A0+SWEEP*i/20,major=i%5===0,[x0,y0]=polar(a,R-8),[x1,y1]=polar(a,R-(major?17:12));add('line',major?'hud-dial-tick major':'hud-dial-tick',{x1:x0.toFixed(2),y1:y0.toFixed(2),x2:x1.toFixed(2),y2:y1.toFixed(2)});}
  const arc=add('path','hud-dial-value',{d:''}),mark=add('path','hud-dial-target',{d:''}),needle=add('g','hud-dial-needle',{});
  for(const [tag,attrs]of [['line',{x1:C,y1:C,x2:C+R-24,y2:C}],['circle',{cx:C,cy:C,r:5}]]){const e=svg(tag);for(const [k,v]of Object.entries(attrs))e.setAttribute(k,v);needle.append(e);}
  const read=node('div','hud-dial-read'),num=node('b','','—'),note=node('small');read.append(num,node('span','',unit));el.append(s,read,node('span','hud-dial-label',label),note);el.dataset.unknown='1';if(READ_ABOUT[label])tip(el,...READ_ABOUT[label]);
  let shown=null,raf=0;const angle=v=>A0+SWEEP*clamp((v-min)/(max-min),0,1);
  const draw=v=>{const a=angle(v);arc.setAttribute('d',a>A0+.05?arcPath(A0,a,R):'');needle.setAttribute('transform',`rotate(${a.toFixed(2)} ${C} ${C})`);num.textContent=v.toFixed(digits);};
  return {el,set(v,{target=null,note:text='',state=''}={}){el.dataset.state=state;note.textContent=text;
   if(known(target)){const a=angle(target),[x,y]=polar(a,R+6),[x1,y1]=polar(a-4.5,R+13),[x2,y2]=polar(a+4.5,R+13);mark.setAttribute('d',`M${x.toFixed(2)} ${y.toFixed(2)}L${x1.toFixed(2)} ${y1.toFixed(2)}L${x2.toFixed(2)} ${y2.toFixed(2)}Z`);}else mark.setAttribute('d','');
   cancelAnimationFrame(raf);if(!known(v)){shown=null;el.dataset.unknown='1';arc.setAttribute('d','');num.textContent='—';return;}delete el.dataset.unknown;
   const from=shown??v,t0=performance.now();shown=v;if(from===v||reduced()){draw(v);return;}
   const step=now=>{const k=Math.min(1,(now-t0)/650);draw(from+(v-from)*(1-(1-k)**3));if(k<1)raf=requestAnimationFrame(step);};raf=requestAnimationFrame(step);}};}
 // A readout in a recessed well. Numbers ease like the dials; words and unknowns are set as they are.
 function readout(label){const el=node('div','hud-read'),v=node('b','','—'),n=node('small');el.append(node('span','',label),v,n);if(READ_ABOUT[label])tip(el,...READ_ABOUT[label]);let shown=null,raf=0;
  return {el,set(x,{digits=1,unit='',note=''}={}){n.textContent=note;cancelAnimationFrame(raf);if(!known(x)){shown=null;v.textContent=typeof x==='string'&&x?x:'—';return;}
   const from=shown??x,t0=performance.now(),draw=y=>{v.textContent=y.toFixed(digits)+unit;};shown=x;if(from===x||reduced()){draw(x);return;}
   const step=now=>{const k=Math.min(1,(now-t0)/650);draw(from+(x-from)*(1-(1-k)**3));if(k<1)raf=requestAnimationFrame(step);};raf=requestAnimationFrame(step);}};}
 // The instruments a wired part shows. Every value is one of the signals the map gives that part.
 function instruments(key){const box=node('div','hud-cluster'),dials=node('div','hud-dials'),reads=node('div','hud-reads');box.append(dials,reads);
  const D=(...a)=>{const d=dial(...a);dials.append(d.el);return d;},RD=label=>{const r=readout(label);reads.append(r.el);return r;};let update=()=>{};
  if(key==='hotend'||key==='bed'){const o=key==='hotend'?'extruder':'heater_bed',d=D(key==='hotend'?'NOZZLE':'BED','°C',0,key==='hotend'?300:120),t=RD('TARGET'),w=RD('HEATER OUTPUT');
   update=s=>{const pw=val(s,o,'power'),tg=val(s,o,'target');d.set(val(s,o,'temperature'),{target:known(tg)&&tg>0?tg:null,state:known(pw)&&pw>0?'hot':'',note:known(tg)&&tg>0?'target '+tg.toFixed(0)+' °C':'heater off'});t.set(tg,{digits:0,unit:' °C'});w.set(known(pw)?pw*100:null,{digits:0,unit:' %',note:'commanded output, not measured watts'});};}
  else if(key==='bed_relay'){const d=D('RELAY DUTY','%',0,100);update=s=>{const pw=val(s,'heater_bed','power');d.set(known(pw)?pw*100:null,{state:known(pw)&&pw>0?'hot':'',note:'switched by the bed heater output'});};}
  else if(/fan$/.test(key)){const o=key==='part_fan'?'fan':key==='hotend_fan'?'heater_fan hotend_fan':'controller_fan controller_fan',d=D('FAN','%',0,100),r=RD('TACHOMETER');
   update=s=>{const sp=val(s,o,'speed'),rpm=val(s,o,'rpm');d.set(known(sp)?sp*100:null,{state:known(sp)&&sp>0?'on':'',note:'output command'});r.set(rpm,{digits:0,unit:' RPM',note:known(rpm)?'measured':'no tach wire, so the % is the command'});};}
  else if(key==='extruder'){const d=D('FILAMENT FEED','mm/s',0,12,1),e=RD('E POSITION'),c=RD('DRIVER CURRENT'),st=RD('DRIVER'),pa=RD('PRESSURE ADVANCE'),cold=RD('EXTRUSION');
   update=s=>{const v=val(s,'motion_report','live_extruder_velocity'),hold=val(s,'tmc2209 extruder','hold_current'),stst=val(s,'tmc2209 extruder','drv_status.stst'),cs=val(s,'tmc2209 extruder','drv_status.cs_actual'),smooth=val(s,'extruder','smooth_time'),ce=val(s,'extruder','can_extrude');
    d.set(known(v)?Math.abs(v):null,{state:known(v)&&v!==0?'on':'',note:known(v)&&v<0?'retracting':''});e.set(val(s,'motion_report','live_position.3'),{digits:1,unit:' mm'});c.set(val(s,'tmc2209 extruder','run_current'),{digits:2,unit:' A',note:known(hold)?'hold '+hold.toFixed(2)+' A':''});
    st.set(stst===1?'Standstill':stst===0?'Stepping':'Unknown',{note:known(cs)?'current scale '+cs+' of 31':''});pa.set(val(s,'extruder','pressure_advance'),{digits:3,unit:' s',note:known(smooth)?'smooth time '+smooth+' s':''});cold.set(ce===true?'Allowed':ce===false?'Blocked, nozzle too cold':'Unknown',{note:'Klipper min_extrude_temp guard'});};}
  else if(key==='probe'){const lamp=node('div','hud-lamp','—');dials.append(lamp);const z=RD('LAST Z RESULT'),at=RD('LAST PROBE AT');
   update=s=>{const q=val(s,'probe','last_query'),px=val(s,'probe','last_probe_position.0'),py=val(s,'probe','last_probe_position.1');lamp.dataset.state=q===true?'on':q===false?'open':'unknown';lamp.textContent=q===true?'TRIGGERED':q===false?'OPEN':'UNKNOWN';tip(lamp,'Probe state','Cached from the last QUERY_PROBE, not a live pin reading. Read only.');z.set(val(s,'probe','last_z_result'),{digits:3,unit:' mm'});at.set(known(px)&&known(py)?`X ${px.toFixed(1)} · Y ${py.toFixed(1)}`:'—');};}
  else if(key==='toolboard'||key==='controller'){const o=key==='toolboard'?'mcu EBBCan':'mcu',d=D('MCU AWAKE','%',0,10,1),rt=RD('RETRANSMITS'),rtt=RD('ROUND TRIP'),fw=RD('FIRMWARE');
   update=s=>{const a=val(s,o,'last_stats.mcu_awake'),srtt=val(s,o,'last_stats.srtt');d.set(known(a)?a*100:null,{note:'time the MCU spends awake'});rt.set(val(s,o,'last_stats.bytes_retransmit'),{digits:0,unit:' bytes',note:'cumulative since the last restart'});rtt.set(known(srtt)?srtt*1000:null,{digits:1,unit:' ms'});fw.set(val(s,o,'mcu_version'));};}
  else if(/^stepper_/.test(key)){const o='tmc2209 '+key,d=D('RUN CURRENT','A',0,2,2),h=RD('HOLD CURRENT'),st=RD('DRIVER STATUS');
   update=s=>{d.set(val(s,o,'run_current'));h.set(val(s,o,'hold_current'),{digits:2,unit:' A'});const bits=s.filter(x=>x.object===o&&x.field.startsWith('drv_status.')&&x.value!=null);st.set(bits.length?bits.map(x=>x.field.slice(11)+' '+x.value).join(' · '):'Not reported',{note:'TMC2209 status bits'});};}
  else if(key==='host'){const d=D('CPU','°C',0,90,1),r=RD('HOST SAMPLE');update=(s,p)=>{const at=p?.telemetry?.host?.captured_at;d.set(val(s,'host','cpu_temp'));r.set(known(at)?new Date(at*1000).toLocaleTimeString():'Unknown',{note:'Moonraker host statistics'});};}
  return {el:box,update};}
 // The bed shows the newest mesh in the recording at or before the cursor, dated, with its probed
 // points as a map. The sample at the cursor alone often carries none (a print clears it), which
 // is why the card said "no probed mesh" after the user had meshed several times.
 async function recordedMesh(to){const h=await fetch(`/api/history/mesh?to=${to}&limit=1`).then(r=>r.ok?r.json():Promise.reject(Error('HTTP '+r.status)));const m=h?.meshes?.[0];if(!m)return null;
  const w=await fetch(`/api/workspace?ts=${m.ts}`).then(r=>r.ok?r.json():null).catch(()=>null),g=w?.point?.mesh;return {...m,grid:Array.isArray(g?.grid)&&g.grid.length?g.grid:null,min:g?.min,max:g?.max};}
 const since=s=>s>=86400?`${Math.floor(s/86400)} d ${Math.floor(s%86400/3600)} h`:s>=3600?`${Math.floor(s/3600)} h ${Math.floor(s%3600/60)} min`:`${Math.max(0,Math.round(s/60))} min`;
 function heatmap(cv,grid){const rows=grid.length,cols=grid[0].length,dpr=Math.min(devicePixelRatio,2),w=cv.clientWidth||300,h=Math.max(60,Math.round(w*rows/cols*.55));cv.width=w*dpr;cv.height=h*dpr;cv.style.height=h+'px';
  const g=cv.getContext('2d'),flat=grid.flat().filter(known),lo=Math.min(...flat),hi=Math.max(...flat),mid=(lo+hi)/2,span=Math.max(.02,(hi-lo)/2),cw=w*dpr/cols,ch=h*dpr/rows;
  const label=cw>=44*dpr&&ch>=22*dpr;g.font=`${11*dpr}px ui-monospace,monospace`;g.textAlign='center';g.textBaseline='middle';
  for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){const v=grid[r][c],x=c*cw,y=(rows-1-r)*ch;g.fillStyle=known(v)?'#'+meshColor(v-mid,span).getHexString():'#1c2224';g.fillRect(x+dpr,y+dpr,cw-2*dpr,ch-2*dpr);if(label&&known(v)){g.fillStyle='rgba(8,12,13,.82)';g.fillText((v>=0?'+':'')+v.toFixed(3),x+cw/2,y+ch/2);}}}
 function meshPanel(){const box=node('div','hud-mesh-well'),head=node('div','hud-mesh-head'),when=node('p','hud-mesh-when','Reading the recording…'),cv=node('canvas','hud-mesh-map'),facts=node('div','hud-reads');
  cv.setAttribute('aria-label','Newest recorded bed mesh, front edge at the bottom');head.append(node('span','','BED MESH · NEWEST RECORDED'),when);box.append(head,cv,facts);const s=getState(),to=s.live?Date.now()/1000:(s.cursor??Date.now()/1000),id=card.id;
  recordedMesh(to).then(m=>{if(card.id!==id)return;if(!m){when.textContent='No mesh in the recording up to '+(s.live?'now':'the cursor');cv.hidden=true;return;}
   when.textContent=`Recorded ${new Date(m.ts*1000).toLocaleString()} · ${since(to-m.ts)} before ${s.live?'now':'the cursor'}`;box.dataset.ts=m.ts;
   const f=(k,v,note)=>{const r=readout(k);r.set(v,{note});facts.append(r.el);};
   f('RANGE',m.range_mm.toFixed(3)+' mm',`${m.rows} × ${m.cols} points${m.adaptive?', adaptive':''}`);f('LOW / HIGH',m.min_mm.toFixed(3)+' / '+m.max_mm.toFixed(3)+' mm','from the mesh mean');f('WORST CORNER',(m.worst_corner.from_mean_mm>=0?'+':'')+m.worst_corner.from_mean_mm.toFixed(3)+' mm',m.worst_corner.corner);f('PROFILE',m.profile||'unnamed');
   if(m.grid)heatmap(cv,m.grid);else cv.hidden=true;}).catch(e=>{if(card.id===id)when.textContent='Mesh history unavailable: '+e.message;});
  return box;}
 function signalRows(box,sigs){box.replaceChildren();if(!sigs.length){box.append(node('p','hud-muted','None of this part\'s signals are in the sample at the cursor.'));return;}
  for(const s of sigs){const r=node('div','hud-raw-signal');r.append(node('span','',s.object+' · '+s.field),node('b','',formatSignal(s.object,s.field,s.value)));box.append(r);}}
 // The Voron catalogue file for a reference part, when the CAD name and the STL name agree.
 const slug=t=>t.toLowerCase().replace(/:\d+$/,'').replace(/\s*\(\d+\)/g,'').replace(/^\[[a-z]\]_/,'').replace(/_x\d+$/,'').replace(/[^a-z0-9]+/g,'_').replace(/^_|_$/g,'');
 const catalogueFile=entry=>{const want=slug(entry.name||'');return want?(getState().catalogue||[]).find(f=>slug(f.path.split('/').pop().replace(/\.stl$/i,''))===want)||null:null;};
 function specsTab(entry,record,wiring){const a=entry.asset||{},dl=node('dl','hud-part-specs'),f=(k,v)=>{if(v!=null&&v!=='')dl.append(node('dt','',k),node('dd','',String(v)));},size=getScene().geometryBounds(entry.id)?.size,cat=catalogueFile(entry),s=getState();
  f('Dimensions',size?size.map(v=>v.toFixed(1)).join(' × ')+' mm':null);f('Wired to',wiring?wiring.signals.map(([o])=>o==='host'?'Moonraker host stats':o).join(', '):'Nothing; this part has no signals of its own');
  // A reference-CAD part's asset record is the whole reference printer, so it is labelled as that.
  const own=record||/^Stella \//.test(entry.full||'');f(own?'Model':'Source assembly',record?.model||a.model);f('Maker',a.manufacturer);f('Revision',record?.revision||a.revision);f(own?'Status':'Source status',entry.representative?'representative geometry, not a measured part':a.status);f('Placement',entry.placement?.status);f('Voron catalogue',cat?`${cat.path} · ${(cat.bytes/1024).toFixed(0)} KB`:null);
  if(record){const age=installationAge(record,s.live?Date.now()/1000:s.cursor);f('Recorded as',[record.model||record.name,record.revision,record.state==='installed'?'installed '+new Date(record.effective_at*1000).toLocaleDateString():record.state].filter(Boolean).join(' · '));f('Time installed',age===null?null:`${age} days (calendar, not operating hours)`);f('Material',record.material);f('Notes',record.reason);}
  const box=node('div');box.append(dl,tip(button(record?'Update record':'Record identity / installation',()=>editPart({id:instanceRecordId(entry.id),name:entry.name,location:(entry.full||'').slice(0,512),model:entry.name,source_url:a.source_url,source_commit:a.revision,license:a.license,evidence:'Source CAD instance '+entry.id+'. Fitted identity requires operator confirmation.'})),record?'Update record':'Record identity and installation','Opens the parts-book form for this part: what is fitted, when, and the evidence. Stored in this browser; the printer is not changed.'));return box;}
 function sourceTab(entry){const a=entry.asset||{},dl=node('dl','hud-part-specs'),f=(k,v)=>{if(v!=null&&v!=='')dl.append(node('dt','',k),node('dd','',String(v)));};
  f('CAD file',a.source_name);f('Licence',a.license);f('Revision',a.revision);f('Maker',a.manufacturer);f('Notes',a.notes);f('Converted by',a.conversion?[a.conversion.engine,a.conversion.version].filter(Boolean).join(' '):null);f('Imported',a.imported_at?new Date(a.imported_at).toLocaleDateString():null);if(!/^Stella \//.test(entry.full||''))f('Instance in the assembly',entry.full);
  const box=node('div');box.append(dl);const url=safeUrl(a.source_url);if(url){const l=node('a','hud-source-link','Open the source repository');l.href=url;l.target='_blank';l.rel='noopener noreferrer';box.append(l);}
  if(a.original_url){const d=node('a','hud-source-link',a.format==='stl'?'Download STL':'Download source');d.href=a.original_url;d.download=a.source_name||'';box.append(d);}return box;}
 function tabs(sets){const box=node('div','hud-tabs'),bar=node('div','hud-tabbar'),body=node('div','hud-tabbody');bar.setAttribute('role','tablist');box.append(bar,body);
  const show=id=>{card.tab=id;for(const b of bar.children){const on=b.dataset.tab===id;b.setAttribute('aria-selected',String(on));b.classList.toggle('on',on);}const t=sets.find(t=>t.id===id);body.replaceChildren(t.build());};
  for(const t of sets){const b=button(t.label,()=>show(t.id));b.dataset.tab=t.id;b.setAttribute('role','tab');tip(b,t.label,'Shows '+t.tip[0].toLowerCase()+t.tip.slice(1)+'.');bar.append(b);}show(sets.some(t=>t.id===card.tab)?card.tab:sets[0].id);return box;}
 // The part chip: a part with nothing wired is selected, highlighted and named, with its actions
 // beside the click, and no card.
 const chip=node('div','hud-partchip');chip.hidden=true;canvasWrap.append(chip);
 function showChip(entry){const scene=getScene();chip.replaceChildren();chip.dataset.id=entry.id;const head=node('div','hud-partchip-head'),close=button('×',()=>{chip.hidden=true;},'hud-partchip-close');tip(close,'Close','Hides this part chip.');close.setAttribute('aria-label','Close');head.append(node('b','',entry.name),close);
  const acts=node('div','hud-partchip-actions'),act=(label,what,does,fn)=>{const b=tip(button(label,fn),what,does);acts.append(b);return b;};
  act('Hide','Hide this part','Hides it on the printer view; ⌂ brings every part back.',()=>{scene.select(entry.id);scene.hideSelected();chip.hidden=true;});act('Isolate','Isolate this part','Shows only this part; ← Whole printer brings the rest back.',()=>{scene.select(entry.id);scene.isolate();});act('Zoom','Zoom to this part','Frames the camera on it.',()=>scene.focus(entry.id));act('Details','Part details','Opens the card: specifications and source.',()=>{chip.hidden=true;openDetail(entry.id,true);});
  chip.append(head,node('span','hud-partchip-sub',(entry.slot_id||entry.layer||'part').toUpperCase()+' · NO SIGNALS WIRED'),paintControl(entry),acts);chip.hidden=false;
  // Beside the press, kept inside the visible canvas rather than the wrapper, which runs on under
  // the readouts at 1568 x 751.
  const r=canvasWrap.getBoundingClientRect(),v=$('machine-view').getBoundingClientRect(),x=pick.x??v.left+24,y=pick.y??v.bottom-chip.offsetHeight-24;
  chip.style.left=clamp(x+14,v.left+8,Math.max(v.left+8,v.right-chip.offsetWidth-8))-r.left+'px';chip.style.top=clamp(y+14,v.top+8,Math.max(v.top+8,v.bottom-chip.offsetHeight-8))-r.top+'px';}
 document.addEventListener('stella-selection-change',()=>{if(!chip.hidden&&getScene()?.selected?.id!==chip.dataset.id)chip.hidden=true;});
 document.addEventListener('keydown',e=>{if(e.key==='Escape')chip.hidden=true;});
 function componentLive(id){const entry=getScene()?.entries.find(e=>e.id===id);if(!entry)return false;if(card.id!==id||!card.wiring)return true;
  // Live means what the header calls fresh: live mode and a sample under 5 s old. A stale live
  // link and a rewound or replayed moment are both still: no glow, and the tag says which.
  const s=getState(),p=detailPoint(),sigs=partSignals(card.wiring,p),age=(s.live?s.tick?.age_seconds:null)??s.snapshot?.age_seconds,mode=s.snapshot?.observer_now?.mode==='replay'?'replay':s.live?'live':'rewind',live=mode==='live'&&known(age)&&age<5,dialog=$('detail-dialog'),tag=dialog.querySelector('.hud-live-tag');
  card.widgets?.update(sigs,p);dialog.dataset.live=live?'1':'';if(tag)tag.textContent=live?'LIVE':mode==='live'?'LAST SAMPLE · '+(known(age)?(age<120?number(age,' s',0):age<7200?number(age/60,' min',0):number(age/3600,' h',1))+' old':'age unknown'):'RECORDED · '+(known(p?.ts)?new Date(p.ts*1000).toLocaleString():'time unknown');
  if(card.tab==='signals'&&card.signalsBody?.isConnected)signalRows(card.signalsBody,sigs);return true;}
 function detail(id,{explicit=false}={}){const scene=getScene(),entry=scene?.entries.find(e=>e.id===id);if(!entry)return false;
  const wiring=wiringFor(entry),wired=Boolean(wiring&&partSignals(wiring,detailPoint()).length);if(!wired&&!explicit){showChip(entry);return true;}chip.hidden=true;
  const s=getState(),slot=entry.slot_id,records=s.book?partsAt({...s.book,revisions:s.book.revisions.filter(r=>r.state!=='candidate')},s.live?Infinity:s.cursor):null,record=records?.get(instanceRecordId(id))||(entry.asset?.status==='installed'&&slot?records?.get(slot):null);
  const dialog=$('detail-dialog');card.id=id;card.wiring=wired?wiring:null;card.widgets=wired?instruments(wiring.key):null;
  $('detail-title').textContent=entry.name;$('detail-focus').textContent='Focus on printer';$('detail-focus').disabled=Boolean(entry.sourceOnly);$('detail-eyebrow').textContent=wired?wiring.label.toUpperCase():'NO SIGNALS WIRED';$('detail-basis').textContent=entry.full;
  const left=dialog.querySelector('.detail-grid>.card'),live=$('detail-live')||node('div'),side=$('detail-side'),model=node('div','hud-part-model'),cam=node('div','hud-part-camera'),size=scene.geometryBounds(id)?.size;
  live.id='detail-live';live.className='hud-live';model.dataset.size=size?size.map(v=>v.toFixed(1)).join(' × ')+' mm':'';
  cam.append(tip(button('+',()=>scene.previewZoom(.75)),'Zoom in','Moves the preview camera closer to the part.'),tip(button('−',()=>scene.previewZoom(1.33)),'Zoom out','Moves the preview camera away.'),tip(button('Reset view',()=>scene.previewReset()),'Reset view','Puts the preview back to its first angle.'),tip(button('Isolate on printer',()=>{dialog.close();scene.select(id);scene.isolate();}),'Isolate on printer','Closes the card and shows only this part on the printer. Show all brings the rest back.'));cam.children[3].disabled=Boolean(entry.sourceOnly);
  left.replaceChildren(model,cam,paintControl(entry));side.replaceChildren(live);live.replaceChildren();
  if(wired){const head=node('div','hud-live-head');head.append(node('span','hud-live-tag','LIVE'),node('small','hud-muted',wiring.signals.map(([o])=>o==='host'?'host stats':o).join(' · ')));live.append(head,card.widgets.el);if(wiring.key==='bed')left.append(meshPanel());}
  const sets=[];if(wired)sets.push({id:'signals',label:'Raw signals',tip:'Every recorded signal wired to this part, at the cursor',build:()=>{const b=node('div','hud-raw');card.signalsBody=b;signalRows(b,partSignals(card.wiring,detailPoint()));return b;}});
  sets.push({id:'specs',label:'Specifications',tip:'Size, model, placement and the recorded identity of this part',build:()=>specsTab(entry,record,wired?wiring:null)},{id:'source',label:'Source',tip:'Where this part\'s CAD came from: file, licence, revision and notes',build:()=>sourceTab(entry)});
  side.append(tabs(sets));componentLive(id);if(!dialog.open)dialog.showModal();scene.preview(id,model);return true;}
 $('detail-dialog').addEventListener('close',()=>{getScene()?.closePreview();card.id=null;card.widgets=null;});
 $('detail-dialog').setAttribute('aria-labelledby','detail-title');
 closeOnBackdrop($('detail-dialog'));

 // hide legacy chrome that the HUD replaces
 for(const sel of ['#camera-buttons','#focus-part','.viewer-bottom #layers','.viewer>p.hint','.viewer-toolbar .live-status','.header-actions>a'])document.querySelectorAll(sel).forEach(e=>{e.hidden=true;});
 const bottomBar=viewer.querySelector('.viewer-bottom');bottomBar.classList.add('hud-legacy-bottom');
 const timer=setInterval(update,1000);const presetTimer=setInterval(loadPresets,60000);update();controls(true);
 return {update,detail,componentLive,openPrepare,dispose(){clearInterval(timer);clearInterval(presetTimer);}};
}
