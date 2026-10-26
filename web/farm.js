// Farm wall (farm board WO-07, 2026-10-12): every printer on one page, in the NEXT look. The service already summarises each printer
// from its own collector (GET /api/printers); this page only draws it. A tile shows the camera when there is one, the machine's own
// 3D model when real CAD is bound to it, and a ring with the numbers when there is neither, so a printer with no camera and no model
// is still a finished-looking tile. Nothing here sends anything to a printer except the quick actions, and those go through the
// same /api/p/<id>/commands as the HUD.
// Polish WO-04 (2026-10-18: "The wall where your printers are should be something you can scroll. Very friendly, PC or phone."):
// the wall wraps to the window from 1 to 100 printers and scrolls (the header stays on top), (tiles came in three sizes remembered
// on this device until U04 took the sizes away: see below), the wall sorts (needs attention first), filters and searches, and only tiles on screen hold a live 3D view, never
// more than a fixed budget (farm-wall.js). The rest keep a still of their last frame.
// U04 (2026-10-20): one tile size, no S/M/L. A tile is a card to scan (the machine or camera, the nozzle, the bed and the progress as small LED gauges from
// the control kit); a press grows it in place to the room of four tiles (the others flow around it, a spring) with the machine or camera bigger, the job, the
// heat gauges as live controls and the actions as square keys; a press again, or on another tile, settles it back. The header is square keys and LED figures.
import {el,icon,loadSprite,initTips,button,busy,toast,openModal,openSheet,closeSheet,tip,openRadial,segmented,lookPanel} from './next-ui.js';
import {control,key,CONTROLS} from './next-control.js';
import {fileTitle,noPrinterYet} from './next-core.js';
import {sheetPrinters,sheetAddPrinter,licenceState,licenceEntry,licenceDismissed,dismissLicence,demoFarmAdd,swatches,ACCENTS,plainDetail} from './next-farm.js';
import {sheetQueue,plateCleared,startNext} from './next-queue.js';
import {modelsSheet} from './next-models.js';
import {sheetAlerts} from './next-notify.js';
import {startPops,camerasView,sheetSpools,sheetAnalytics,sheetBackup,sheetSlicer} from './next-fleet.js';
import {lookOf,modelLine,WORD,CHIP,ICON,sortPrinters,inFilter,matchesQuery,filterOptions,counts,actionsFor,SORTS,budget3d,plan3d,headerKey,ringWord,bigWord,spoolLine,tvLayout,heatOf} from './farm-wall.js';
import {modelsBanner,modelStatus,tileModelPanel,demoLicenceLine,sheetAbout} from './next-getmodels.js';

const $=id=>document.getElementById(id);
const SVG='http://www.w3.org/2000/svg',C=2*Math.PI*44;
const tiles=new Map();let order=[],canManage=false,farmToken=null,raw=[],looks=new Map();
// What the queue says (GET /api/queue): per printer, whether its plate needs clearing and which job is next.
let qinfo=new Map(),qjobs=new Map(),qwaiting=0,qon=false;
let queueAlive=true;
// In replay and whenever the queue is off, GET /api/queue answers 503. Poll printers every two seconds,
// but stop asking for the queue once it has refused: replay never serves it.
// Browsers allow about six connections to one host, and the page needs a few for itself: the first few
// cameras on screen play, the rest show a still every 4 s. One upstream per camera either way (WO-04 hub).
const MAX_LIVE=()=>innerWidth<=600?2:4;
const phone=()=>innerWidth<=600;
const coarse=()=>matchMedia('(pointer: coarse)').matches;
const store=(k,v)=>{try{if(v===undefined)return localStorage.getItem(k);localStorage.setItem(k,v);}catch{}return null;};
const params=new URLSearchParams(location.search),tv=params.get('tv')==='1';
// The view: what to show and how. The sort is remembered on this device (2026-10-18); a filter or a search is not,
// so coming back never finds half the wall hidden. (U04: the tile size is gone, there is one.)
const view={sort:SORTS.some(s=>s[0]===store('fm-sort'))?store('fm-sort'):'attention',filter:'all',q:''};
let openId=null;   // the tile that is grown (at most one)

// A tile counts as on screen a little before it really is, so the picture is ready when it arrives. A flick of the wheel past a
// tile does not start a 3D view for it (SETTLE), and a tile that has just left keeps its view for a moment (GRACE) in case the
// wheel turns back. The full scene of a machine is over 100 MB, so a tile that holds one keeps it longer.
const SETTLE_MS=220,GRACE_MS=2500,SCENE_GRACE_MS=9000;
// Making a view reads and builds a whole machine on the page's own thread (about a tenth of a second or more), so none is made while
// the wall is moving: they wait until the scrolling has been quiet for QUIET_MS, and then one at a time with a breath between (so a touch or a wheel turn is heard).
// Poses are not redrawn while it moves either. The wheel always wins over the pictures.
const QUIET_MS=180,PARALLEL=1;let lastScrollAt=0,held=0,heldAt=0;
const quietIn=()=>held>0&&Date.now()-heldAt<4000?QUIET_MS:Math.max(0,QUIET_MS-(Date.now()-lastScrollAt));   // (4 s at most: a lost pointerup must not keep the pictures away for good)
// (a finger or a button that is down also keeps the wall quiet: the build would hold up the page long enough to lose a long press)
addEventListener('pointerdown',()=>{held++;heldAt=Date.now();},{passive:true,capture:true});
for(const ev of ['pointerup','pointercancel'])addEventListener(ev,()=>{held=Math.max(0,held-1);lastScrollAt=Date.now();},{passive:true,capture:true});
const watch='IntersectionObserver' in window?new IntersectionObserver(es=>{const now=Date.now();for(const e of es){const t=tiles.get(e.target.dataset.id);if(!t)continue;
 if(e.isIntersecting&&!t.visible){t.visible=true;t.inAt=now;}else if(!e.isIntersecting&&t.visible){t.visible=false;t.outAt=now;}}allocate();manage3d();},{rootMargin:'160px 0px'}):{observe(){},unobserve(){}};

const mins=s=>{const m=Math.round(s/60);if(m<1)return 'under a minute';if(m<60)return m+' min';const h=Math.floor(m/60),r=m%60;return r?`${h} h ${r} min`:`${h} h`;};
const clock=ts=>{const d=new Date(ts*1000),t=d.toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'});return d.toDateString()===new Date().toDateString()?t:d.toLocaleDateString([],{weekday:'short'})+' '+t;};
const deg=v=>typeof v==='number'&&isFinite(v)?Math.round(v)+'°':'—';
const pct=p=>typeof p==='number'&&isFinite(p)?Math.round(Math.max(0,Math.min(1,p))*100):null;
const ago=s=>s<90?'a moment ago':s<5400?Math.round(s/60)+' min ago':Math.round(s/3600)+' h ago';
// How far a heating printer is from where it is going: the mean of its heaters that have a target (a 20 °C room is the start).
const heatFrac=p=>{const f=[p.temps?.nozzle,p.temps?.bed].filter(h=>typeof h?.target==='number'&&h.target>20&&typeof h?.temp==='number').map(h=>Math.max(0,Math.min(1,(h.temp-20)/(h.target-20))));return f.length?f.reduce((a,b)=>a+b)/f.length:0;};

// ---- quick actions ------------------------------------------------------------------------------
const ctl=new Map();
async function controls(id,force=false){const c=ctl.get(id);if(c&&!force&&Date.now()-c.at<6000)return c;
 let v={at:Date.now(),token:'',enabled:false,commands:[]};
 try{const j=await (await fetch(`/api/p/${id}/controls`,{cache:'no-store'})).json();v={at:Date.now(),token:j.token||'',enabled:!!j.enabled,commands:j.commands||[],reason:j.reason};}catch{}
 ctl.set(id,v);return v;}
async function run(p,cmd,confirm=false,extra={},say=''){
 const c=await controls(p.id,true);
 if(!c.enabled||!c.token){toast(c.reason==='controls_enabled is false'?'Printer controls are switched off in Stella Scope. The Help guide, under Controls, shows how to switch them on.':'This device can watch but not control. Use the computer or a paired phone.','alarm',{from:tiles.get(p.id)?.root});return false;}
 const row=c.commands.find(x=>x.id===cmd);
 if(row&&row.allowed===false){toast(row.reason||'Not available right now.','alarm',{from:tiles.get(p.id)?.root});return false;}
 try{const r=await fetch(`/api/p/${p.id}/commands`,{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token},body:JSON.stringify({id:cmd,confirm,...extra})});
  const j=await r.json();if(!r.ok)throw Error(j.error||'The printer refused.');
  toast(`${p.name}: ${say||{pause:'paused',resume:'resumed',cancel:'cancelled'}[cmd]||'done'}.`,'ok',{from:tiles.get(p.id)?.root});refresh();return true;}
 catch(e){toast(e.message,'alarm',{from:tiles.get(p.id)?.root});return false;}}
// U04 (2026-10-20): a heat gauge on a grown tile sends once, when the hand lets go, by the printer page's own two steps: what it would send (the preview),
// then the command. Only a T3 command stops for the review (a heater is T2, so it goes straight out, as on the printer page).
async function sendHeat(p,heater,v){
 const c=await controls(p.id,true),extra={target:heater,value:v};
 if(!c.enabled||!c.token)return run(p,'set_heater',false,extra);   // (says why it cannot)
 let pv;try{const r=await fetch(`/api/p/${p.id}/command-preview`,{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token},body:JSON.stringify({id:'set_heater',...extra})});
  pv=await r.json();if(!r.ok)throw Error(pv.error||'Stella Scope could not show what it would send.');}catch(e){toast(e.message,'alarm',{from:tiles.get(p.id)?.root});return false;}
 if(pv.tier==='T3'&&pv.review!==false&&!await new Promise(res=>{let ok=false;openModal({title:'Review command',icon:'i-console',onClose:()=>res(ok),build:(body,close)=>{
  body.append(el('p','nx-note','Check what will happen. It goes to the printer once, with no automatic retry.'),el('pre','nx-review',pv.script||pv.action||''));
  body.append(button({icon:'i-check',label:'Send once',kind:'primary',size:'row',title:'Send once',does:'Sends this to the printer one time.',onPress:()=>{ok=true;close();}}),button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Nothing is sent.',onPress:()=>close()}));}});}))return false;
 return run(p,'set_heater',pv.tier==='T3',extra,`${heater==='extruder'?'nozzle':'bed'} ${v>0?'set to '+v+'°':'turned off'}`);}
function confirmStop(p){openModal({title:`Cancel the print on ${p.name}?`,icon:'i-stop-square',build:(body,close)=>{
 body.append(el('p','nx-note',`${p.file?fileTitle(p.file):'The print'} will be stopped for good. It cannot be resumed.`));
 const go=button({icon:'i-stop-square',label:'Cancel print',kind:'danger',size:'row',title:'Cancel print',does:'Stops the print on this printer now, for good.',onPress:async b=>{if(b.dataset.busy!==undefined)return;busy(go,true);const ok=await run(p,'cancel',true);busy(go,false);if(ok)close();}});
 body.append(go,button({icon:'i-back',label:'Keep printing',size:'row',title:'Keep printing',does:'Leaves the print running.',onPress:close}));}});}
// Power through the printer's smart plug, behind a confirm; cutting a working printer says what it ends.
async function confirmPower(p){
 const pr=p.plug?.reading,working=['printing','paused','heating'].includes(p.state),turnOn=!pr?.on;
 openModal({title:`${p.name} power`,icon:'i-power',build:(body,close)=>{
  const msg=el('p','nx-note');
  body.append(el('p','fm-power-now',pr?(pr.on?`On${typeof pr.watts==='number'?`, drawing ${Math.round(pr.watts)} W`:''}`:'Off'):'The plug has not answered yet.'));
  if(!turnOn&&working)body.append(el('p','fm-power-warn',`${p.name} is ${p.state}. Turning the power off ends the print and leaves the nozzle where it is.`));
  const go=button({icon:'i-power',label:turnOn?'Turn on':working?'Turn off and end the print':'Turn off',kind:turnOn?'primary':'danger',size:'row',title:turnOn?'Turn on':'Turn off',does:'Switches the smart plug.',onPress:async b=>{
   if(b.dataset.busy!==undefined)return;busy(b,true);
   try{const d=await (await fetch('/api/printers',{cache:'no-store'})).json();
    const r=await fetch(`/api/plugs/${p.id}`,{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':d.token||''},body:JSON.stringify({on:turnOn,force:!turnOn&&working})});
    const j=await r.json().catch(()=>({}));if(!r.ok)throw Error(j.error||`Stella Scope answered with an error (${r.status}).`);
    toast(`${p.name} turned ${turnOn?'on':'off'}.`,'ok',{from:tiles.get(p.id)?.root});close();refresh();}catch(e){msg.textContent=e.message;}finally{busy(b,false);}}});
  body.append(go,button({icon:'i-back',label:'Cancel',size:'row',title:'Cancel',does:'Leaves the power as it is.',onPress:()=>close()}),msg);}});}

// ---- one tile -----------------------------------------------------------------------------------
function svgRing(){const s=document.createElementNS(SVG,'svg');s.setAttribute('viewBox','0 0 104 104');s.setAttribute('aria-hidden','true');
 for(const k of ['t','v']){const c=document.createElementNS(SVG,'circle');c.setAttribute('cx','52');c.setAttribute('cy','52');c.setAttribute('r','44');c.setAttribute('class',k);if(k==='v'){c.setAttribute('stroke-dasharray',C.toFixed(2));c.setAttribute('stroke-dashoffset',C.toFixed(2));}s.append(c);}return s;}
// A square key from the kit with the name a screen reader gives it and the tip the rest of the app shows (the wall has no pill buttons).
function sq(icon,label,title,does,onPress,tone){const k=key({icon,label,onPress});k.el.setAttribute('aria-label',label);tip(k.el,title,does);if(tone)k.el.dataset.tone=tone;return k;}
// One of a tile's three LED gauges with its caption under it. The kit's own label is too small for a tile, so the caption is ours.
function gauge(spec,cap){const g=control(spec),cell=el('div','fm-g');g.el.tabIndex=-1;cell.append(g.el,el('small','',cap));return {g,cell};}
function build(p){
 // v3: where this tile's live 3D view is: none, loading, live, or stopping (a view being let go; its place in the budget is held)
 const t={p,fails:0,live:false,hasCam:false,camAt:0,visible:!('IntersectionObserver' in window),inAt:0,outAt:0,hold:0,v3:'none',gen:0,sig:'',look:'idle',holdN:0,holdB:0};
 const root=el('article','fm-tile');root.dataset.id=p.id;root.dataset.cam='off';
 const head=el('div','fm-head'),dot=el('i','fm-dot'),nm=el('div','fm-name'),link=el('a','fm-link'),sub=el('small','fm-sub'),bank=el('small','fm-bank'),line=el('small','fm-line');
 const chip=el('span','fm-chip'),chipIco=icon('i-live'),chipTxt=el('b');chip.append(chipIco,chipTxt);
 nm.append(link,sub,bank);head.append(dot,nm,chip,line);
 // the head is the tile's one stop for the keyboard: Enter or Space grows it and settles it (a press anywhere else on the tile does the same)
 head.tabIndex=0;head.setAttribute('role','button');head.setAttribute('aria-expanded','false');
 head.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();toggleOpen(t.p.id);}});
 const media=el('div','fm-media'),cam=el('img','fm-cam');cam.alt='';cam.decoding='async';
 const m3d=el('div','fm-3d');m3d.hidden=true;
 const still=el('canvas','fm-still');still.hidden=true;   // a canvas, not an image: the page's CSP allows no data: or blob: pictures
 const nocam=el('div','fm-nocam'),ring=el('div','fm-ring'),num=el('span'),numMain=el('b'),numSub=el('small');num.append(numMain,numSub);ring.append(svgRing(),num);
 const nt=el('div','fm-nocam-text'),ntB=el('b'),ntS=el('small');nt.append(ntB,ntS);nocam.append(ring,nt);
 const demo=el('span','fm-demo','DEMO');demo.title='A simulated printer. Nothing real is printing.';demo.hidden=true;
 const mdl=tileModelPanel({token:()=>farmToken});   // F03: what is happening to this tile's 3D model (getting it, failed, not asked for yet)
 const over=el('div','fm-over'),overL=el('span'),overR=el('span'),overRb=el('b'),overRc=el('i','fm-clk');overR.append(overRb,overRc);over.append(overL,overR);media.append(cam,still,nocam,m3d,over,demo,mdl.el);
 // U04: the nozzle, the bed and the progress are LED gauges from the control kit. At rest they only read (a press on them grows the tile); on the grown tile the
 // nozzle and the bed are live: hold and turn, and the target is sent once when the hand lets go (sendHeat). The progress gauge never takes a hand.
 const heat=el('div','fm-heat');
 const gN=gauge({...CONTROLS[0],value:0,now:0,onSet:v=>{t.holdN=Date.now()+6000;heatDone(t,t.gN,'extruder',v);}},'NOZZLE'),gB=gauge({...CONTROLS[1],value:0,now:0,onSet:v=>{t.holdB=Date.now()+6000;heatDone(t,t.gB,'heater_bed',v);}},'BED'),
  gP=gauge({key:'progress',label:'DONE',unit:'%',min:0,max:100,step:1,turn:300,color:'var(--accent)',color2:'var(--accent-2)',value:0},'PROGRESS');
 gP.g.el.setAttribute('role','meter');gP.cell.dataset.g='progress';gN.cell.dataset.g='nozzle';gB.cell.dataset.g='bed';
 heat.append(gN.cell,gB.cell,gP.cell);
 const job=el('div','fm-job'),thumb=el('img','fm-thumb');thumb.alt='';thumb.hidden=true;thumb.addEventListener('error',()=>{thumb.hidden=true;});
 const jt=el('div','fm-jobtext'),jb=el('b'),js=el('small');jt.append(jb,js);
 const big=el('div','fm-big'),pctEl=el('span','fm-pct'),eta=el('span','fm-eta'),etaT=el('span','fm-et'),etaC=el('i','fm-etc');eta.append(etaT,etaC);big.append(pctEl,eta);
 const bar=el('div','fm-bar'),barFill=el('i');bar.append(barFill);
 const jobTop=el('div','fm-jobtop');jobTop.append(thumb,jt);job.append(jobTop,big,bar);
 const temps=el('div','fm-temps'),pw=el('span','fm-watts'),sp=el('span','fm-spool');sp.hidden=true;pw.hidden=true;temps.append(pw,sp);
 const inst=el('div','fm-inst');const right=el('div','fm-inst-main');right.append(job,temps);inst.append(heat,right);
 // the actions are square keys, on the grown tile only (the tile at rest is a card to scan)
 const act=el('div','fm-act');
 const open=sq('nav-hud','Open','Open','Opens this printer: the machine, its instruments and every control.',()=>{location.href='/p/'+t.p.id;});
 const wrap=(b,cmd)=>()=>{if(b.disabled)return;b.disabled=true;run(t.p,cmd).finally(()=>{b.disabled=false;});};
 const pause=sq('pause','Pause','Pause','Pauses the print on this printer.',()=>wrap(pause.el,'pause')());
 const resume=sq('play','Resume','Resume','Resumes the paused print.',()=>wrap(resume.el,'resume')(),'primary');
 const stop=sq('stop-square','Cancel print','Cancel print','Stops the print for good, after you confirm.',()=>confirmStop(t.p),'danger');
 const power=sq('power','Power','Power','The smart plug this printer is on: its power draw, and switching it on or off after you confirm.',()=>confirmPower(t.p));
 const clear=sq('check','Plate cleared','Plate cleared','Says the plate is empty. The camera checks it first, then the next file can start.',()=>plateCleared({id:t.p.id,name:t.p.name},{after:refresh}),'primary');
 const next=sq('play','Start next','Start the next file','Sends the next queued file that fits this printer and starts it, after the plate check.',()=>startNext({id:t.p.id,name:t.p.name},{after:refresh}),'primary');
 // 2026-10-24: the card has a quick HEAT (your materials on a wheel) and COOL, and its colour is yours to choose.
 const heatK=sq('heat','Heat','Heat','Your materials on a wheel: pick one and both heaters go to its temperatures.',()=>quickHeat(t.p,heatK.el)),coolK=sq('cool','Cool','Cool','Both heaters off on this printer.',async()=>{if(await sendHeat(t.p,'extruder',0))await sendHeat(t.p,'heater_bed',0);});
 for(const [k,a] of [[open,'open'],[pause,'pause'],[resume,'resume'],[stop,'stop'],[power,'power'],[clear,'clear'],[next,'next'],[heatK,'heat'],[coolK,'cool']])k.el.dataset.act=a;
 power.el.hidden=clear.el.hidden=next.el.hidden=true;
 act.append(clear.el,next.el,heatK.el,coolK.el,pause.el,resume.el,stop.el,power.el,open.el);
 // 2026-10-24: "These panels should be rather customizable." Its colour (kept on the printer), what the tile shows and whether it stands first
 // (kept on this device: another screen may want its own wall).
 const look=el('div','fm-look'),pr=prefOf(p.id);
 const pin=el('button','fm-pin');pin.type='button';const paintPin=()=>{const on=!!prefOf(t.p.id).pin;pin.setAttribute('aria-pressed',String(on));pin.replaceChildren(icon('i-pin'),el('span','',on?'First on the wall':'Pin it first'));};paintPin();
 pin.addEventListener('click',()=>{setPref(t.p.id,{pin:!prefOf(t.p.id).pin});paintPin();lastKey='';layout();});tip(pin,'Pin it first','This printer stands first on the wall, before the sort. Kept on this device.');
 const shows=segmented(['auto','machine','camera','job'],pr.show||'auto',v=>{setPref(t.p.id,{show:v});t.root.dataset.show=v;manage3d?.();},v=>({auto:'Best',machine:'3D machine',camera:'Camera',job:'The job'}[v]));
 look.append(el('small','','PANEL COLOUR'),swatches(p.accent||ACCENTS[0][0],c=>setAccent(t,c)),el('small','','THE TILE SHOWS'),shows,pin);
 root.dataset.show=pr.show||'auto';
 // A press anywhere on the tile that is not a key, a link or a gauge grows it, and a press on a grown tile settles it. On the wall screen nothing grows.
 root.addEventListener('click',e=>{if(tv)return;if(e.target.closest('button,a[href],input,select,.nxctl'))return;if(getSelection()?.toString())return;
  // U05 (2026-10-20): growing the tile gives its name a link in the middle of this very click, and the browser then followed it (a press on a name at rest opened the printer instead of growing the tile, found on a phone, where the name fills the head)
  if(e.target.closest('.fm-link'))e.preventDefault();toggleOpen(t.p.id);});
 inst.append(act,look);root.append(head,media,inst);
 root.tabIndex=-1;
 watch.observe(root);
 Object.assign(t,{mdl,root,head,dot,link,sub,bank,line,chip,chipIco,chipTxt,cam,still,m3d,ringV:ring.querySelector('.v'),numMain,numSub,ntB,ntS,overL,overRb,overRc,overR,demo,media,heat,gN:gN.g,gB:gB.g,gP:gP.g,cN:gN.cell,cB:gB.cell,cP:gP.cell,job,thumb,jb,js,temps,sp,pw,pctEl,eta,barFill,act,open:open.el,pause:pause.el,resume:resume.el,stop:stop.el,power:power.el,clear:clear.el,next:next.el,etaT,etaC});
 return t;}
// A heat gauge was let go: send the new target once. A refusal puts the gauge back to what the printer says.
// The kit's dial runs to 300 for the nozzle; the service takes 290 at most (the printer page's own limit), so a turn past it is sent as 290 and the gauge says so.
const SET_MAX={extruder:290,heater_bed:120};
async function heatDone(t,g,heater,v){
 if(v>SET_MAX[heater]){v=SET_MAX[heater];g.set(v);}
 const ok=await sendHeat(t.p,heater,v);
 if(!ok){if(heater==='extruder')t.holdN=0;else t.holdB=0;}
 t.sig='';update(t,t.p);setTimeout(()=>{t.sig='';update(t,t.p);},6100);}

// The progress gauge takes the state's neon (the same four as the rest of the wall): teal working, orange heating or paused or cancelled, green done, magenta error.
const TONE={printing:['var(--accent)','var(--accent-2)'],heating:['#FF6A00','#FFB067'],paused:['#FF6A00','#FFB067'],clear:['#18E36B','#8CFFB5'],done:['#18E36B','#8CFFB5'],error:['#FF2D92','#FF86C0'],idle:['#04D9C4','#7FFFF0'],offline:['#04D9C4','#7FFFF0']};
function update(t,p){
 const qi=qinfo.get(p.id),look=lookOf(p,qi,qon),act=actionsFor(p,{qon,qi,canManage}),isOpen=openId===p.id;
 const cancelled=p.state==='finished'&&p.job_state==='cancelled'&&(look==='done'||look==='clear');
 const word=cancelled?'Cancelled':WORD[look],chipWord=cancelled?'Cancelled':CHIP[look];
 const progress=pct(p.progress),active=['printing','paused','heating'].includes(p.state);
 const left=typeof p.time_left_s==='number'?p.time_left_s:null,eta=typeof p.eta_ts==='number'?p.eta_ts:null;
 const leftTxt=active&&left!==null?`${mins(left)} left`:'',clk=active&&eta?clock(eta):'';
 const nz=p.temps?.nozzle,bd=p.temps?.bed,heatPct=Math.round(heatFrac(p)*100);
 const nextTitle=qi?.next_job?(qjobs.get(qi.next_job)?.title||''):'';
 const sig=[look,p.state,p.job_state,p.name,p.brand,p.driver,p.model,p.file,progress,leftTxt,clk,deg(nz?.temp),deg(nz?.target),deg(bd?.temp),deg(bd?.target),p.detail,p.accent,p.thumbnail_url,
  JSON.stringify(p.plug||null),JSON.stringify(p.spool||null),Object.values(act).join(),nextTitle,canManage,typeof p.tick_age_s==='number'?Math.round(p.tick_age_s/30):'',p.cad_model,p.scene_model,JSON.stringify(p.cad_options||null),p.camera_url,isOpen,phone(),qon,t.bankNote||''].join('|');
 t.p=p;t.look=look;
 // A model or an add-on chosen for this printer changed: let the old view go, the next pass draws the new one.
 const key=(p.cad_model||'')+'|'+JSON.stringify(p.cad_options||[]);if(t.m3dKey!==undefined&&t.m3dKey!==key&&t.v3!=='none')stop3d(t,false);t.m3dKey=key;
 if(sig===t.sig)return;t.sig=sig;
 const r=t.root;
 r.dataset.state=p.state;r.dataset.look=look;
 if(p.state==='offline'&&r.dataset.cam==='on'){r.dataset.cam='off';t.hasCam=false;t.camAt=0;}   // U04: an offline printer's tile stops showing its last camera picture at once (it waited for the next camera pass, up to 4 s), and asks for its camera again when it is back
r.toggleAttribute('data-cancelled',cancelled);r.toggleAttribute('data-demo',p.driver==='demo');r.style.setProperty('--c',p.accent||'var(--accent)');
 t.link.textContent=p.name||p.id;if(isOpen)t.link.href='/p/'+p.id;else t.link.removeAttribute('href');   // (a link only on the grown tile: at rest a press on the name grows the tile like any other press)
 t.head.setAttribute('aria-label',`${p.name||p.id}, ${word}${active&&progress!==null?', '+progress+' percent':''}. ${isOpen?'Press to close':'Press to open'}`);t.head.setAttribute('aria-expanded',String(isOpen));
 // F04 (2026-10-19): the machine line and the note about missing tool files are two lines, not one joined with a dot (it needed three lines beside the chip)
 t.sub.textContent=modelLine(p);t.bank.textContent=t.bankNote?'Tools not installed':'';t.bank.title=t.bankNote?'The toolchanger tools are not installed, so the stock head is drawn. Open this printer to get them: their licence is shown first.':'';
 t.demo.hidden=p.driver!=='demo';
 t.chip.dataset.look=look;t.chip.toggleAttribute('data-cancelled',cancelled);t.chip.title=word;t.chipTxt.textContent=chipWord;
 t.chipIco.firstChild.setAttribute('href','#'+ICON[look]);
 // ring and words for the camera-less picture
 const shown=look==='heating'?heatPct:p.state==='finished'?(cancelled?progress:100):active?progress:null;
 t.ringV.setAttribute('stroke-dashoffset',(C*(1-(shown??0)/100)).toFixed(2));
 t.numMain.textContent=shown!==null?shown+'%':p.state==='offline'?'OFF':p.state==='error'?'!':'';
 t.numSub.textContent=p.needs_login&&(look==='offline'||look==='error')?'LOGIN':ringWord(look,cancelled,p.state);
 const nl=!!p.needs_login&&(look==='offline'||look==='error');   // F06 (2026-10-19): no login saved, or the saved one is refused: say so and where to type it, not "check that it is on"
 const lines={
  printing:leftTxt||'Working out the time left',
  heating:`${deg(nz?.temp)} of ${deg(nz?.target)}`,
  paused:p.detail?`Paused: ${p.detail}`:`Paused at ${progress??0}%`,
  clear:cancelled?'Cancelled · clear the plate':'Plate needs clearing',
  done:cancelled?'Cancelled':qon?'Done · the plate is clear':'Done · take the print off',
  error:nl?'Needs its login':plainDetail(p)||'Needs attention',
  offline:nl?'Needs its login':typeof p.tick_age_s==='number'?`Last heard ${ago(p.tick_age_s)}`:'Not answering',
  idle:'Ready · nothing printing'};
 // on a small tile the chip is only an icon, so the line says the state in words
 t.line.textContent=['printing','heating','error','offline'].includes(look)?`${chipWord} · ${lines[look]}`:lines[look]||'';t.line.title=['offline','error'].includes(look)&&p.detail?plainDetail(p):'';
 const nt={
  printing:[p.file?fileTitle(p.file):'Printing',leftTxt?`${leftTxt}, done ${clk||'soon'}`:'Working out the time left.'],
  heating:['Heating up',`Nozzle ${deg(nz?.temp)} of ${deg(nz?.target)}, bed ${deg(bd?.temp)} of ${deg(bd?.target)}.`],
  paused:['Paused',(p.file?fileTitle(p.file)+'. ':'')+(p.detail?p.detail+' ':'')+'Resume it or cancel it from here.'],
  clear:cancelled?['Cancelled','The print was stopped. Clear the plate before the next one.']:['Plate needs clearing','Take the print off, then press Plate cleared.'],
  done:cancelled?['Cancelled','The print was stopped.']:[ 'Done',qon?'The plate is clear.':'Take the print off and clear the plate.'],
  idle:['Ready','Nothing is printing.'],
  error:nl?['Needs its login','Open Printers, then Settings, and type its login. The printer itself is fine.']:['Needs attention',p.detail||'The printer reported an error.'],
  offline:nl?['Needs its login','Open Printers, then Settings, and type its login. The printer itself is fine.']:['Offline','Not answering. Check that it is on and on the network.'+(typeof p.tick_age_s==='number'?` Last heard ${ago(p.tick_age_s)}.`:'')],
 }[look]||['',''];
 t.ntB.textContent=nt[0];t.ntS.textContent=nt[1];t.ntS.title=['offline','error'].includes(look)&&p.detail?p.detail:'';
 // the two pills over the picture: what it is doing, and when it will be done
 // (the chip already says what it is doing, so the left pill is how far and the right one how long)
 t.overL.textContent=look==='printing'||look==='paused'?(progress!==null?progress+'%':chipWord):look==='heating'?`${heatPct}%`:chipWord;
 t.overRb.textContent=active&&left!==null&&look!=='paused'&&look!=='heating'?leftTxt:look==='offline'&&typeof p.tick_age_s==='number'?`Last heard ${ago(p.tick_age_s)}`:'';
 t.overRc.textContent=active&&left!==null&&look!=='paused'&&clk?` · ${clk}`:'';
 t.overR.hidden=!t.overRb.textContent;
 // the job line
 const hasFile=!!p.file&&p.state!=='offline'&&p.state!=='idle';
 t.job.dataset.look=look;t.job.toggleAttribute('data-cancelled',cancelled);
 t.pctEl.textContent=look==='printing'||look==='paused'?(progress!==null?progress+'%':'—'):look==='heating'?heatPct+'%':p.needs_login&&(look==='offline'||look==='error')?'LOGIN':bigWord(look,cancelled);
 t.etaC.textContent=active&&left!==null&&look!=='paused'&&clk?` · ${clk}`:'';
 t.etaT.textContent=active&&left!==null&&look!=='paused'?leftTxt:look==='heating'?'heating up':look==='paused'?'':look==='offline'&&typeof p.tick_age_s==='number'?`last heard ${ago(p.tick_age_s)}`:look==='idle'?'nothing printing':'';
 t.barFill.style.width=(look==='heating'?heatPct:look==='printing'||look==='paused'?progress??0:look==='clear'||look==='done'?(cancelled?0:100):0)+'%';
 if(!hasFile){t.jb.textContent=nl?'Needs its login':p.state==='offline'?'Not answering':p.state==='error'?(p.detail||'Needs attention'):'';t.js.textContent=nl?'Open Printers, then Settings, and type it.':p.state==='offline'?'Check that it is on and on the network.':'';t.thumb.hidden=true;}
 if(hasFile){t.jb.textContent=fileTitle(p.file);
  t.js.textContent=p.state==='printing'?'':p.state==='paused'?(p.detail||`${progress??0}% · paused`):p.state==='finished'?(cancelled?'Cancelled':'Finished'):p.state==='heating'?'Heating':p.state==='error'?'Needs attention':'';
  const want=p.capabilities?.thumbnails?p.thumbnail_url||'':'';if(want&&t.thumb.dataset.src!==want){t.thumb.dataset.src=want;t.thumb.src=want;t.thumb.hidden=false;}else if(!want){t.thumb.hidden=true;}}
 // temperatures: the nozzle and the bed are LED gauges (the kit): the ring is lit to the target, the dot on it is where the heater is now. A gauge that is being
 // held, or whose command was sent a moment ago, is left alone: the printer has not caught up yet and would pull it back.
 const hn=heatOf(nz),hb=heatOf(bd),off=p.state==='offline',gnow=Date.now();
 for(const [g,cell,h,until] of [[t.gN,t.cN,hn,t.holdN],[t.gB,t.cB,hb,t.holdB]]){cell.hidden=off||!h.has;g.setNow(h.now??0);if(!g.el.classList.contains('live')&&gnow>=until)g.set(h.target);}
 t.cP.hidden=shown===null;if(shown!==null){t.gP.set(shown);t.gP.el.style.setProperty('--c',(cancelled?TONE.paused:TONE[look])?.[0]||'var(--accent)');t.gP.el.style.setProperty('--c2',(cancelled?TONE.paused:TONE[look])?.[1]||'#7FFFF0');}
 t.heat.hidden=t.cN.hidden&&t.cB.hidden&&t.cP.hidden;
 // the smart plug (2026-10-13): what the printer draws, and the power button for the person who manages
 const pr=p.plug?.reading;t.pw.hidden=!p.plug?.spec;
 if(p.plug?.spec){t.pw.textContent=p.plug.ok===false?'PLUG ?':!pr?'PLUG':pr.on?(typeof pr.watts==='number'?`${Math.round(pr.watts)} W`:'POWER ON'):'POWER OFF';t.pw.toggleAttribute('data-off',!!pr&&!pr.on);t.pw.title=p.plug.ok===false?`The plug did not answer: ${p.plug.error||''}`:pr&&typeof pr.kwh==='number'?`${pr.kwh.toFixed(2)} kWh so far`:'';}
 t.power.hidden=!act.power;
 // the loaded spool (2026-10-13): its colour, material and what is left
 const s=p.spool;t.sp.hidden=!s;if(s){t.sp.style.setProperty('--sc',s.colour||'#9AA4B2');t.sp.textContent=spoolLine(s);t.sp.toggleAttribute('data-low',!!s.low);t.sp.title=s.name||'';}
 // actions: only what this printer can do and this moment allows
 t.pause.hidden=!act.pause;t.resume.hidden=!act.resume;t.stop.hidden=!act.stop;t.clear.hidden=!act.clear;t.next.hidden=!act.next;
 if(!t.next.hidden){t.next.dataset.tipDoes=`Sends ${nextTitle||'the next file'} and starts it, after the plate check.`;}
 t.resume.dataset.kind='primary';
 t.media.tabIndex=t.root.dataset.cam==='on'?0:-1;
 t.media.setAttribute('aria-label',t.root.dataset.cam==='on'?`${p.name}: open the camera`:`${p.name}: ${word}`);
}

// A camera tile plays its camera. Whether a printer has a camera at all is asked once (and again now and
// then), not guessed from failed requests. Tiles beyond the stream budget, or off screen, show a still.
function still(t){
 if(t.live)return;
 const im=new Image();
 im.onload=()=>{t.fails=0;if(!t.live)t.cam.src=im.src;if(t.root.dataset.cam!=='on'){t.root.dataset.cam='on';allocate();}};
 im.onerror=()=>{if(++t.fails>=2){t.root.dataset.cam='off';t.hasCam=false;t.camAt=0;}};
 im.src=`${t.p.camera_url}?t=${Date.now()}`;}
function setLive(t,on){
 if(t.live===on)return;t.live=on;t.root.toggleAttribute('data-live',on);
 if(on){t.cam.onerror=()=>{t.live=false;t.hold=Date.now()+30000;still(t);};t.cam.src=`/api/p/${t.p.id}/camera/stream`;}
 else{t.cam.onerror=null;t.cam.removeAttribute('src');still(t);}}
function allocate(){
 let n=0;const max=document.hidden?0:MAX_LIVE(),shown=new Set(order);
 for(const id of order){const t=tiles.get(id);if(!t)continue;
  const want=t.root.dataset.cam==='on'&&t.visible&&t.p.state!=='offline'&&Date.now()>=t.hold&&n<max;
  if(want)n++;setLive(t,want);}
 for(const [id,t] of tiles)if(!shown.has(id))setLive(t,false);}
// Only tiles on screen ask their camera for a picture: a wall of a hundred printers must not ask a hundred cameras every few seconds.
function stills(){
 for(const t of tiles.values()){
  if(!t.p.camera_url||t.p.state==='offline'){t.root.dataset.cam='off';continue;}
  if(document.hidden||!t.visible)continue;
  if(Date.now()-t.camAt>(t.hasCam?600000:30000)){
   t.camAt=Date.now();
   fetch(`/api/p/${t.p.id}/camera`,{cache:'no-store'}).then(r=>r.json()).then(j=>{const had=t.hasCam;t.hasCam=(j.cameras||[]).length>0;if(!t.hasCam)t.root.dataset.cam='off';else if(!had)still(t);}).catch(()=>{});}
  if(t.hasCam&&!t.live)still(t);}
 allocate();}

// ---- the 3D machine on a tile (WO-13, our video opener; budget WO-04 2026-10-18) -----------------
// A printer with real CAD bound to it shows the machine itself, moving with the printer, beside the camera picture. No model means
// no made-up stand-in (the owner: real CAD or none). Each live tile owns one GL context and redraws only on a pose change; poses refresh
// every 800 ms per live tile. Which tiles are live is decided in one place (manage3d, with the pure plan3d): a tile on screen gets a
// view if a place is free in the budget, a tile that has gone off screen lets its view go after a moment and keeps a still of the
// last frame, and the page never holds more than the budget (Chrome kills the oldest context beyond about 16).
const m3dViews=new Map();   // printer id -> {key, view, spec, simple}
// A tile at rest draws its machine without the parts smaller than a fastener (half the drawing); the grown tile, on a window big enough for it, draws every part.
const simpleView=(id,st)=>(st.view?.triangles||0)>400000||innerWidth<1000||openId!==id;
// WO-14 item 1: a printer whose own scene CAD is in its managed library (Stella's Voron, the same
// scene the main page draws) shows it on the wall as its model, with no setting to bind. These tiles
// run one full scene each and take their pose from that printer's own tick stream.
const sceneTiles=new Map(); // printer id -> {scene, sse, canvas, loaded}
const bufs=new Map();       // model file url -> its unpacked bytes, for the last few models: twelve tiles of one machine fetch it once
function modelBuf(mv,url){if(bufs.has(url)){const p=bufs.get(url);bufs.delete(url);bufs.set(url,p);return p;}
 const p=mv.modelBuffer(url);bufs.set(url,p);p.catch(()=>bufs.delete(url));while(bufs.size>3)bufs.delete(bufs.keys().next().value);return p;}
const hasModel=t=>!!(t.p.cad_model||t.p.scene_model)&&!(t.noModelUntil>Date.now())&&!t.mdlPending;   // (F03: not while its model is still being fetched)
const graceOf=t=>t.p.cad_model?GRACE_MS:SCENE_GRACE_MS;
let m3dTimer=0,lastReport='',maxContexts=0,maxLive=0;
function report(){
 const all=[...tiles.values()],live=all.filter(t=>t.v3==='live').length,loading=all.filter(t=>t.v3==='loading'||t.v3==='stopping').length,ctx=m3dViews.size+sceneTiles.size,vis=all.filter(t=>t.visible).length;
 maxContexts=Math.max(maxContexts,ctx);maxLive=Math.max(maxLive,live+loading);
 const key=`live=${live} loading=${loading} contexts=${ctx} budget=${budget3d(innerWidth,coarse())} on-screen=${vis} tiles=${all.length} open=${openId||'-'}`;
 if(key!==lastReport){lastReport=key;console.log(`farm3d ${key} scrollY=${Math.round(scrollY)} max-contexts=${maxContexts}`);}}
window.farm3d=()=>{const all=[...tiles.values()];return {live:all.filter(t=>t.v3==='live').length,loading:all.filter(t=>t.v3==='loading'||t.v3==='stopping').length,contexts:m3dViews.size+sceneTiles.size,budget:budget3d(innerWidth,coarse()),maxContexts,maxLive,onScreen:all.filter(t=>t.visible).length,tiles:all.length,stills:all.filter(t=>!t.still.hidden).length};};
function manage3d(){
 clearTimeout(m3dTimer);const now=Date.now();let again=Infinity;const hush=quietIn();if(hush>0)again=hush;
 const shown=new Set(order),all=[...order.map(id=>tiles.get(id)),...[...tiles.values()].filter(t=>!shown.has(t.p.id))].filter(Boolean);
 const items=all.map(t=>{let want=false;
  if(hasModel(t)&&!t.root.hidden&&!['camera','job'].includes(t.root.dataset.show)){   // a tile set to show its camera or its job draws no machine (D16)
   if(t.visible){const wait=t.inAt+SETTLE_MS-now;if(wait<=0)want=true;else again=Math.min(again,wait);}
   else if(t.v3!=='none'){const left=t.outAt+graceOf(t)-now;if(left>0){want=true;again=Math.min(again,left);}}}
  return {id:t.p.id,want,visible:t.visible&&want,state:t.v3};});
 const plan=plan3d(items,budget3d(innerWidth,coarse()));
 for(const id of plan.stop)stop3d(tiles.get(id));
 let slots=hush>0?0:Math.max(0,PARALLEL-all.filter(t=>t.v3==='loading').length);
 for(const id of plan.start){if(slots<=0)break;start3d(tiles.get(id));slots--;}
 if(Number.isFinite(again))m3dTimer=setTimeout(manage3d,again+20);
 report();}
function start3d(t){
 t.v3='loading';const g=++t.gen;
 tileModel(t,g).then(r=>{if(t.gen!==g){t.v3='none';dropIfIdle(t);manage3d();return;}t.v3=r==='live'?'live':'none';report();setTimeout(manage3d,40);})
  .catch(()=>{if(t.gen===g){t.noModelUntil=Date.now()+60000;dropModel(t.p.id);}t.v3='none';manage3d();});
 report();}
const dropIfIdle=t=>{if(t.gen&&t.v3==='none'&&!m3dViews.has(t.p.id)&&!sceneTiles.has(t.p.id)){t.m3d.replaceChildren();t.m3d.hidden=true;delete t.m3d.dataset.loading;}};
function setStill(t,ok){if(!ok)return;t.still.hidden=false;t.media.classList.add('fm-duo');}
// Let a tile's view go. With `keep` the last frame stays on the tile as a picture.
function stop3d(t,keep=true){
 if(!t||t.v3==='none'||t.v3==='stopping')return;const id=t.p.id;t.gen++;
 if(t.v3==='loading'){t.v3='stopping';return;}             // the loader notices, cleans up and frees the place itself
 const m=m3dViews.get(id),sc=sceneTiles.get(id);
 if(m){if(keep){try{setStill(t,m.view.snapshot(t.still));}catch{}}dropModel(id);t.v3='none';report();}
 else if(sc){t.v3='stopping';(keep&&sc.loaded?snapScene(sc,t.still):Promise.resolve(false)).then(ok=>setStill(t,ok)).catch(()=>{}).finally(()=>{dropModel(id);t.v3='none';report();manage3d();});}
 else{dropModel(id);t.v3='none';}
 if(!keep)t.still.hidden=true;}
// The scene draws in its own frame; ask for one and read the picture straight after it, before the browser takes it away.
function snapScene(e,into){return new Promise(res=>{try{e.scene.schedule();}catch{}requestAnimationFrame(()=>{try{const src=e.canvas;if(!src.width){res(false);return;}const w=Math.min(640,src.width),h=Math.max(1,Math.round(src.height*w/src.width));into.width=w;into.height=h;const g=into.getContext('2d');g.clearRect(0,0,w,h);g.drawImage(src,0,0,w,h);res(true);}catch{res(false);}});});}
// The picture waits under the view until it has drawn its first frame, so a tile never flashes empty.
function revealLive(t){delete t.m3d.dataset.loading;requestAnimationFrame(()=>requestAnimationFrame(()=>{if(t.v3==='live'||t.v3==='loading')t.still.hidden=true;}));}
async function tileScene(t,st,g){
 const id=t.p.id;
 let sc;try{sc=await import('./hud-scene.js');}catch{t.noModelUntil=Date.now()+60000;return 'none';}
 if(t.gen!==g||!tiles.has(id))return 'gone';
 const cv=el('canvas');cv.className='fm-3d-canvas';t.m3d.replaceChildren(cv);t.m3d.dataset.loading='';t.m3d.hidden=false;t.media.classList.add('fm-duo');
 let scene=null;
 try{scene=sc.createScene(cv,null,{stage:'next',tile:true,apiPrefix:`/api/p/${id}`});}catch{cv.remove();t.m3d.hidden=true;t.noModelUntil=Date.now()+60000;return 'none';}
 const entry={scene,sse:null,canvas:cv,dead:false,loaded:false};
 sceneTiles.set(id,entry);
 // the scene is over 100 MB: the ring or the last picture stays until it has finished loading (it marks its canvas then)
 const poll=setInterval(()=>{if(entry.dead){clearInterval(poll);return;}if(cv.dataset.renderer==='source-cad'){entry.loaded=true;clearInterval(poll);try{scene.zoom(1.12);}catch{}revealLive(t);}},300);   // (zoom out a little: room around the machine on a tile)
 if(st?.position)setScenePose(entry,st);
 const open=()=>{if(entry.dead)return;const es=new EventSource(`/api/p/${id}/tick/stream`);entry.sse=es;
  let lastDraw=0;
  es.onmessage=e=>{const now=performance.now();if(now-lastDraw<500)return;lastDraw=now;
   try{const r=JSON.parse(e.data);if(r.point?.position)scene.set({point:r.point},null);scene.live(r.point||{});}catch{}};
  es.onerror=()=>{es.close();entry.sse=null;if(!entry.dead)setTimeout(open,5000);};};
 open();return 'live';}
function setScenePose(entry,st){
 const pos=[st.position?.x,st.position?.y,st.position?.z];
 if(pos.some(v=>typeof v!=='number'||!isFinite(v)))return;
 try{entry.scene.set({point:{position:pos,homed:'xyz',ts:st.tick_age_s!=null?Date.now()/1000-st.tick_age_s:undefined}},null);}catch{entry.dead=true;}}
function dropScene(id){const s=sceneTiles.get(id);if(s){s.dead=true;try{s.sse?.close();}catch{}try{s.scene.dispose();}catch{}try{s.scene.renderer.forceContextLoss();}catch{}sceneTiles.delete(id);}}
// The full machine scene is over 100 MB: a phone loads it only when its owner turned 3D on (the same
// switch and rule as the printer page, next.js show3d).
const sceneOk=()=>{try{const v=localStorage.getItem('nx-3d');if(v)return v==='on';}catch{}return !(innerWidth<600||matchMedia('(pointer: coarse)').matches&&innerWidth<1000);};
async function tileModel(t,g){
 const id=t.p.id,now=Date.now();
 let st;try{st=await fetch(`/api/p/${id}/model`,{cache:'no-store'}).then(r=>r.ok?r.json():null);}catch{return 'none';}
 t.bankNote=st?.scene?.missing?.length?'tools not installed':null; // polish WO-03: the toolchanger's files are a separate, licensed download
 if(t.gen!==g)return 'gone';
 // No catalogue model: a scene model counts as the bound model (WO-14 item 1). An offline printer
 // that has one is drawn at its last known pose, dimmed, with the offline note over it (item 2).
 if(!st||!st.model||!st.installed){
  if(st?.scene===true&&sceneOk()){t.m3d.toggleAttribute('data-offline',t.p.state==='offline');return tileScene(t,st,g);}
  t.noModelUntil=now+60000;return 'none';}
 t.m3d.toggleAttribute('data-offline',t.p.state==='offline');
 // Polish WO-01: a toolchanger or add-on chosen for the model is part of what is drawn.
 const key=st.model+'|'+JSON.stringify(st.options||[])+'|'+(st.scene?.swapping?1:0);
 let mv;try{mv=await import('./model-view.js');}catch{t.noModelUntil=now+60000;return 'none';}
 if(t.gen!==g)return 'gone';
 const host=t.m3d;host.dataset.loading='';host.hidden=false;
 try{
  const buffer=await modelBuf(mv,st.file_url);
  if(t.gen!==g||!tiles.has(id)){return 'gone';}
  // The motion groups come from the registered profile (its part keys), as on the printer page: without it a tile draws the model but moves only the parts whose own record names a group (polish WO-02).
  const profile=st.motion?await fetch(`/api/printer-models/${st.model}/motion`,{cache:'no-store'}).then(r=>r.ok?r.json():null).catch(()=>null):null;
  if(t.gen!==g||!tiles.has(id)){return 'gone';}
  const view=await mv.createModelView(host,{buffer,basis:st.basis,profile,tile:true,simple:simpleView(id,st)});   // parts smaller than a fastener are left out of a tile at rest: nobody can see them there, and it is half the drawing; the grown tile draws them all (U04)
  if(t.gen!==g||!tiles.has(id)){view.dispose();return 'gone';}
  if(st.scene?.placements?.length)view.setScene(st.scene).catch(()=>{});
  t.media.classList.add('fm-duo');
  // Polish WO-03: the part grows on the plate and the head glides between reports (model-run.js); a tile is drawn by the shared loop within its budget, with a lighter print.
  const run=(await import('./model-run.js')).runModel(view,{prefix:`/api/p/${id}`,tile:true});
  if(t.gen!==g||!tiles.has(id)){run.stop();view.dispose();return 'gone';}
  m3dViews.set(id,{key,view,run,spec:st,simple:simpleView(id,st),miss:st.scene?.missing?.length||0});drawPose(id,st);revealLive(t);return 'live';
 }catch{host.hidden=true;delete host.dataset.loading;t.noModelUntil=Date.now()+240000;return 'none';}}
// a pose that has not changed is not drawn again (an idle, paused or finished printer costs nothing)
// a tile moved in the page (a new sort) does not always keep what its canvas showed: draw every live view again
function redrawAll(){for(const m of m3dViews.values())m.pose=null;for(const e of sceneTiles.values()){try{e.scene.schedule();}catch{}}}
function drawPose(id,st){const m=m3dViews.get(id);if(!m)return;if(m.run){m.run.setVisible(tiles.get(id)?.visible!==false);m.run.update(st);return;}if(!st.pose?.groups)return;const k=JSON.stringify(st.pose.groups);if(m.pose===k)return;m.pose=k;m.view.setGroups(st.pose.groups);}
function dropModel(id){const m=m3dViews.get(id);if(m){m.run?.stop();m.view.dispose();m3dViews.delete(id);}dropScene(id);const t=tiles.get(id);if(t){t.m3d.replaceChildren();t.m3d.hidden=true;delete t.m3d.dataset.loading;if(t.still.hidden)t.media.classList.remove('fm-duo');}}
function poseTicker(){if(quietIn()>0)return;for(const id of m3dViews.keys()){const t=tiles.get(id);if(!t){dropModel(id);continue;}
 fetch(`/api/p/${id}/model?scene=0`,{cache:'no-store'}).then(r=>r.ok?r.json():null).then(st=>{if(st)drawPose(id,st);}).catch(()=>{});}}
setInterval(()=>{if(!document.hidden)poseTicker();},800);
// F03: every answer about the models (next-getmodels.js) is read by each tile for its own: what its model is doing now. A tile whose model has just
// arrived builds its view at once; one still waiting builds nothing (it would only find a missing file).
document.addEventListener('stella-model-status',e=>{
 for(const t of tiles.values()){
  const st=modelStatus(e.detail,t.p.id),pending=!!st&&(st.state==='getting'||st.state==='waiting');
  t.mdl.set(st);
  if(t.mdlPending&&!pending&&!st){t.noModelUntil=0;if(t.v3==='live')stop3d(t,false);manage3d();}
  t.mdlPending=pending;}});
// New model files landed (next-getmodels.js): tiles that had none may draw now, and a live tile lets its view go so the wall's budget builds it again with the new parts.
// (F03: a live view is left alone: only a tile whose own files have just arrived is rebuilt, by the status handler above.)
document.addEventListener('stella-models',()=>{for(const t of tiles.values())t.noModelUntil=0;manage3d();});
// Median frame time across every tile view on the wall (the WO-13 budget: under 16 ms).
window.farmFrameMedian=()=>{const all=[...m3dViews.values()].flatMap(m=>m.view.perf.ms);if(!all.length)return null;all.sort((a,b)=>a-b);return all[all.length>>1];};
window.farmViews=()=>[...m3dViews.entries()].map(([id,m])=>({id,parts:m.view.info.parts,shown:m.view.shown(),frames:m.view.perf.n,median:m.view.frameMedian()}));
window._mvm=m3dViews; // the frame-time evidence (WO-13 5)

// ---- the page -----------------------------------------------------------------------------------
let summaryKey='';
function summary(list){
 const c=counts(list,looks),host=$('fm-summary');
 // F04 (2026-10-19): the header is rewritten only when something it shows has changed (it was torn down and rebuilt every two seconds, a style and layout pass of its own in a frame that also draws a tile)
 const nx=list.filter(p=>['printing','heating'].includes(p.state)&&typeof p.eta_ts==='number').sort((a,b)=>a.eta_ts-b.eta_ts)[0],fp=list.find(p=>p.default)||list[0];
 const sk=headerKey(c,{filter:view.filter,queued:qon?qwaiting:-1,next:nx,clock:nx?clock(nx.eta_ts):'',phone:phone(),first:fp?.id});
 if(sk===summaryKey&&host.childElementCount)return;summaryKey=sk;host.replaceChildren();
 // The counts are square keys with an LED figure (U04, the kit's language): press one to see just those printers, press it again to see them all.
 // The figure is lit in the state's neon over a ghost of unlit digits, as on an LED panel.
 const led=(n,cls='')=>{const b=el('b','fm-led'+cls,String(n));b.dataset.ghost='8'.repeat(Math.max(2,String(n).length));return b;};
 const stat=(n,label,tone,filter,short)=>{
  const k=key({label:phone()&&short?short:label,toggle:true,pressed:view.filter===filter,onPress:()=>setFilter(view.filter===filter?'all':filter)}),b=k.el;
  b.classList.add('fm-stat');b.dataset.tone=tone;b.dataset.filter=filter;b.prepend(led(n));
  b.setAttribute('aria-label',`${n} ${label}. ${view.filter===filter?'Show every printer':'Show only these'}`);host.append(b);};
 stat(c.printing,'printing','work','printing');
 if(c.attention)stat(c.attention,'needs attention','attn','attention','attention');
 stat(c.idle,'ready','idle','idle');
 if(c.offline)stat(c.offline,'offline','off','offline');
 if(qon&&qwaiting>0){const k=key({label:'in queue',onPress:()=>sheetQueue()}),b=k.el;b.classList.add('fm-stat','fm-stat--q');b.dataset.tone='work';b.prepend(led(qwaiting));b.setAttribute('aria-label',`${qwaiting} in queue. Open the queue`);host.append(b);}
 const next=list.filter(p=>['printing','heating'].includes(p.state)&&typeof p.eta_ts==='number').sort((a,b)=>a.eta_ts-b.eta_ts)[0];
 // On a phone the next-done line is not part of the sticky header (there is no room for it in 120 px): it sits under it, at the top of the wall.
 const nl=$('fm-next');nl.hidden=!(next&&phone());nl.replaceChildren();
 if(next){const s=el('div','fm-stat fm-stat--next');s.append(el('span','','next done'),el('b','fm-led fm-led--clock',clock(next.eta_ts)),el('span','fm-next-name',next.name));(phone()?nl:host).append(s);}
 // The mark leads to the first printer's own page, which on a phone is the way out of the farm.
 const first=list.find(p=>p.default)||list[0],home=document.querySelector('.fm-id');if(first&&home)home.setAttribute('href','/p/'+first.id);}

// A small, dismissible note once the farm is past the free printers and has no licence. Nothing is
// disabled by it, and "Not now" quiets it for a week.
let licenceSeen='';
async function licenceNote(){
 const host=$('fm-licence'),st=await licenceState();
 const show=!!(st&&st.wanted)&&!licenceDismissed();
 const key=show?`${st.printers}|${st.state}|${st.note}`:'';
 if(key===licenceSeen)return;licenceSeen=key;
 host.hidden=!show;host.replaceChildren();if(!show)return;
 const over=st.printers-st.free_printers;
 host.append(el('p','fm-licence-text',st.state==='too_few'?`${st.note} A farm licence for more printers keeps this work going.`:`${st.printers} printers, ${over} over the ${st.free_printers} that are free. A farm licence keeps this work going.`));
 const row=el('div','fm-licence-act');
 row.append(button({icon:'i-check',label:'Enter licence',size:'row',title:'Enter licence',does:'Paste a farm licence. It is checked on this computer.',onPress:()=>licenceEntry({onDone:()=>{licenceSeen='';refresh();}})}),
  button({icon:'i-close',label:'Not now',size:'row',kind:'ghost',title:'Not now',does:'Hides this note for a week. Nothing is switched off.',onPress:()=>{dismissLicence();licenceSeen='';licenceNote();}}));
 host.append(row);}

// The demo farm's card: nine simulated printers (finish-2 F09: the Prusa MK3S+ joined the eight) in one press, for someone with no printer to look at yet.
function demoCard(grid,show){
 let c=$('fm-democard');
 if(!show){c?.remove();return;}
 grid.dataset.fill='0';   // the card is one more cell: a wall that fills the window would give it the whole first row
 if(!c){c=el('div','fm-democard nx-glass');c.id='fm-democard';
  const go=button({icon:'i-play',label:'Try the demo farm',kind:'primary',size:'row',title:'Try the demo farm',does:'Adds nine simulated printers running sample prints, so the whole farm can be tried with no printer, and downloads their 3D models from their authors (the four Prusa models are already inside Stella Scope). Nothing real is touched and none count toward the licence.',onPress:async()=>{busy(go,true);try{await demoFarmAdd(farmToken);await refresh();}catch(e){toast(e.message,'alarm');}finally{busy(go,false);}}});
  // F03: the open licences in one line, where the button is: pressing it is the person's yes to them, and the models are fetched at once.
  const lic=el('p','fm-democard-lic','');demoLicenceLine().then(t=>{lic.textContent=t;lic.hidden=!t;});
  // F04 (2026-10-19): a fresh install's first need is to add a printer (or to try the demo): the card says both in words, beside the header's plus.
  const add=button({icon:'i-plus',label:'Add a printer',size:'row',title:'Add a printer',does:'Finds printers on this network, or takes an address you type.',onPress:()=>sheetAddPrinter()});add.dataset.act='add-first';
  c.append(el('b','','No printer connected yet'),el('p','','Try the demo farm: nine simulated printers, a toolchanger and a Prusa among them, each at its own place in a sample print. Nothing real is touched, and every one is marked DEMO.'),lic,go,add);}
 grid.append(c);}
// What shows, in what order, at what size. The order is kept while nothing that decides it has changed (a printer changing state,
// the list changing, a new sort, filter or search), and re-worked at most every 15 s otherwise, so tiles do not shuffle under
// someone's finger as a percentage ticks over.
let lastKey='',lastAt=0,lastIds=[],domKey='',filterKey='';
function layout(){
 const byId=new Map(raw.map(p=>[p.id,p])),grid=$('fm-grid');
 const shown=raw.filter(p=>inFilter(view.filter,p,looks.get(p.id))&&matchesQuery(p,looks.get(p.id),view.q));
 const setKey=shown.map(p=>p.id+':'+looks.get(p.id)).join(',')+'|'+view.sort+'|'+view.filter+'|'+view.q;
 let ids;
 if(setKey===lastKey&&Date.now()-lastAt<15000)ids=lastIds.filter(id=>byId.has(id));
 else{const pins=prefs();ids=sortPrinters(shown,looks,view.sort).map(p=>p.id);ids=[...ids.filter(id=>pins[id]?.pin),...ids.filter(id=>!pins[id]?.pin)];lastKey=setKey;lastAt=Date.now();lastIds=ids;}
 order=ids;const inShown=new Set(ids);
 for(const [id,t] of tiles)t.root.hidden=!inShown.has(id);
 // a grown tile that the filter has hidden, or whose printer has gone, settles
 if(openId&&!inShown.has(openId)){const t=tiles.get(openId);openId=null;if(t)markOpen(t,false);}
 // (U04: a tile that changes its place in the order, a printer going off or coming back, glides there on the spring instead of jumping; the first draw does not animate)
 const dk=ids.join(',');if(dk!==domKey){const move=()=>{for(const id of ids)grid.append(tiles.get(id).root);};if(domKey&&!document.hidden)flip(move);else move();domKey=dk;redrawAll();}
 // Only the wall screen is laid out to fill the screen (U04: one tile size everywhere else, wrapping to the window and scrolling).
 const n=ids.length,ph=phone();
 let fill=false;
 if(tv){const room=innerHeight-(grid.getBoundingClientRect().top+scrollY)-12,L=tvLayout(n,innerWidth,room);fill=L.fill;grid.style.setProperty('--cols',L.cols);grid.style.setProperty('--rows',L.rows);}
 grid.dataset.fill=fill?'1':'0';if(!fill){grid.style.removeProperty('--cols');grid.style.removeProperty('--rows');}
 for(const t of tiles.values())update(t,t.p);
 // the toolbar
 const opts=filterOptions(raw,looks),fk=JSON.stringify(opts)+'|'+view.filter+'|'+ph;
 if(fk!==filterKey){filterKey=fk;const sel=$('fm-filter');
  if(!opts.some(o=>o.key===view.filter))view.filter='all';
  sel.replaceChildren(...opts.map(o=>new Option(`${ph?o.short:o.label} (${o.count})`,o.key)));sel.value=view.filter;}
 $('fm-sort').value=view.sort;
 const total=raw.length;
 const empty=$('fm-empty');empty.hidden=!(total>0&&n===0);
 if(!empty.hidden){empty.replaceChildren(el('p','','No printer matches '+(view.q?`"${view.q}"`:'this filter')+'.'),button({icon:'i-close',label:'Show every printer',size:'row',title:'Show every printer',does:'Clears the search and the filter.',onPress:()=>{view.q='';$('fm-q').value='';setFilter('all');}}));}
 summary(raw);
 $('fm-sub').textContent=`${n===total?total:n+' OF '+total} ${total===1?'PRINTER':'PRINTERS'}`;
 allocate();manage3d();}
// ---- a press grows a tile in place (U04, 2026-10-20) ----------------------------------------------------------
// The grown tile takes the room of four (two columns and two rows; the whole width on a phone) and the others flow around it. The change is made in the page at once and
// then played from where every tile was to where it is now (FLIP), on the spring of the control kit, so nothing pops: a tile that moves glides, the one that grows
// grows. A press on the grown tile, or on another tile, settles it back the same way.
const SPRING='cubic-bezier(.34,1.56,.64,1)',still_=()=>matchMedia('(prefers-reduced-motion: reduce)').matches;
function flip(change){
 const vh=innerHeight,list=[...tiles.values()].map(t=>t.root).filter(r=>!r.hidden),first=new Map(list.map(r=>[r,r.getBoundingClientRect()]));
 change();
 if(still_())return;
 for(const r of list){const a=first.get(r),b=r.getBoundingClientRect();
  if((a.bottom<-80&&b.bottom<-80)||(a.top>vh+80&&b.top>vh+80))continue;   // neither place is anywhere near the screen: nothing to see
  const dx=a.left-b.left,dy=a.top-b.top,sx=a.width/b.width,sy=a.height/b.height;
  if(Math.abs(dx)<1&&Math.abs(dy)<1&&Math.abs(sx-1)<.01&&Math.abs(sy-1)<.01)continue;
  r.getAnimations().forEach(x=>x.cancel());
  r.animate([{transformOrigin:'0 0',transform:`translate(${dx}px,${dy}px) scale(${sx},${sy})`},{transformOrigin:'0 0',transform:'none'}],{duration:560,easing:SPRING});}}
// Which cell of the wall a tile sits in (from where it is on screen): the grown tile is put in that cell, or in the column before it when it is in the last one,
// so it grows where it was and the others flow around it, instead of jumping to the next row.
function cellOf(grid,r){
 const cs=getComputedStyle(grid),cols=cs.gridTemplateColumns.split(' ').map(parseFloat),rows=cs.gridTemplateRows.split(' ').map(parseFloat),cg=parseFloat(cs.columnGap)||0,rg=parseFloat(cs.rowGap)||0,g=grid.getBoundingClientRect(),b=r.getBoundingClientRect();
 const at=(sizes,gap,p)=>{let acc=0;for(let i=0;i<sizes.length;i++){if(p<acc+sizes[i]+gap/2)return i;acc+=sizes[i]+gap;}return Math.max(0,sizes.length-1);};
 return {col:at(cols,cg,b.left-g.left+2),row:at(rows,rg,b.top-g.top+2),cols:cols.length};}
// 2026-10-24: "when I click on the printer itself, it expands the tab ... uses a bunch of wasteful space ... it could just be an overlay or a card over top
// of it." The open printer is a card over the wall: an empty frame keeps its place so nothing on the wall moves, and a press on the veil or Escape closes it.
function markOpen(t,on){
 t.root.toggleAttribute('data-open',on);t.head.setAttribute('aria-expanded',String(on));
 if(on){const ph=el('div','fm-tile fm-ghost');ph.style.height=t.root.getBoundingClientRect().height+'px';t.root.before(ph);t.ghost=ph;
  let v=$('fm-veil');if(!v){v=el('div','fm-veil');v.id='fm-veil';v.addEventListener('click',()=>{if(openId)toggleOpen(openId);});}t.root.parentElement.append(v);v.hidden=false;document.body.dataset.card='';}
 else{t.ghost?.remove();t.ghost=null;const v=$('fm-veil');if(v)v.hidden=true;delete document.body.dataset.card;}
 for(const g of [t.gN,t.gB]){g.el.tabIndex=on?0:-1;}
 if(on)t.link.href='/p/'+t.p.id;else t.link.removeAttribute('href');}
addEventListener('keydown',e=>{if(e.key==='Escape'&&openId&&!document.querySelector('.nx-modal,#nx-sheet[data-open],.nx-wheel-host:not([hidden])'))toggleOpen(openId);});
// Quick heat from the card: the printer's own materials on the wheel.
async function quickHeat(p,from){let mats=[];try{const d=await (await fetch(`/api/p/${p.id}/presets`)).json();mats=d.materials||[];}catch{}
 const r=from.getBoundingClientRect();
 openRadial([...mats.map(m=>({key:'m-'+m.name,icon:'i-heat',label:m.name.toUpperCase(),sub:`${m.nozzle}° · ${m.bed}°`,tone:'heat',title:'Heat for '+m.name,does:`Nozzle ${m.nozzle}°, bed ${m.bed}°.`,run:async()=>{if(await sendHeat(p,'extruder',m.nozzle))await sendHeat(p,'heater_bed',m.bed);}})),
  {key:'cool',icon:'i-cool',label:'COOL',tone:'cool',title:'Cool down',does:'Both heaters off.',run:async()=>{if(await sendHeat(p,'extruder',0))await sendHeat(p,'heater_bed',0);}}],
  {x:r.left+r.width/2,y:r.top,title:'Heat '+(p.name||p.id),center:()=>{const n=p.temps?.nozzle,b=p.temps?.bed,d=v=>typeof v==='number'?Math.round(v)+'°':'off';return `Nozzle ${d(n?.temp)}${n?.target>0?' → '+d(n.target):''}\nBed ${d(b?.temp)}${b?.target>0?' → '+d(b.target):''}`;},onPick:it=>it.run?.()});}
// What each tile shows and the pinned ones, kept on this device.
function prefs(){try{return JSON.parse(localStorage.getItem('fm-prefs')||'{}')||{};}catch{return {};}}
function prefOf(id){return prefs()[id]||{};}
function setPref(id,patch){const all=prefs();all[id]={...(all[id]||{}),...patch};try{localStorage.setItem('fm-prefs',JSON.stringify(all));}catch{}}
// The panel colour: saved on the printer (it is the printer page's accent too).
async function setAccent(t,c){t.root.style.setProperty('--c',c);if(!farmToken){toast('Colours are changed from this computer only.','alarm');return;}
 try{const r=await fetch('/api/printers/'+t.p.id,{method:'PATCH',headers:{'Content-Type':'application/json','X-Stella-Control-Token':farmToken},body:JSON.stringify({accent:c})});if(!r.ok)throw Error((await r.json().catch(()=>({}))).error||'not saved');t.p.accent=c;}
 catch(e){toast('The colour was not kept: '+e.message,'alarm');}}
function toggleOpen(id){
 if(tv||!tiles.has(id))return;
 const prev=openId?tiles.get(openId):null,next=openId===id?null:tiles.get(id);
 flip(()=>{if(prev)markOpen(prev,false);openId=next?next.p.id:null;if(next)markOpen(next,true);});
 // the grown tile draws its machine with every part and the settled one without the small ones: the view that was drawn the other way is let go (it keeps its last frame) and made again
 for(const t of [prev,next]){if(!t)continue;const m=m3dViews.get(t.p.id);if(m&&m.simple!==simpleView(t.p.id,m.spec))stop3d(t,true);}
 for(const t of tiles.values())update(t,t.p);
 // (a tile taller than what is left under the header is put with its top under the header, where the machine and the heat gauges are)
 if(next)setTimeout(()=>{if(openId===next.p.id){const r=next.root.getBoundingClientRect(),head=parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--fm-sticky-h'))||150;next.root.scrollIntoView({block:r.height>innerHeight-head?'start':'nearest',behavior:still_()?'auto':'smooth'});}},still_()?0:580);   // (after it has grown: measured while it grows, its box is the old one)
 allocate();manage3d();}
function setFilter(k){view.filter=k;$('fm-filter').value=k;layout();}

// U04: one refresh at a time, and none that waits for ever: a request that was never answered (the browser holds six connections to the service, and cameras
// and live views hold some) used to stack up behind itself and keep the wall from changing when a printer went off or came back. After 8 s it is let go.
let refreshAt=0;
async function refresh(){
 if(refreshAt&&Date.now()-refreshAt<9000)return;refreshAt=Date.now();
 try{await refreshNow();}finally{refreshAt=0;}}
async function refreshNow(){
 let d;
 try{d=await (await fetch('/api/printers',{cache:'no-store',signal:AbortSignal.timeout(8000)})).json();$('fm-banner').hidden=true;}
 catch{const b=$('fm-banner');b.textContent='Stella Scope did not answer. This page keeps trying.';b.hidden=false;return;}
 if(queueAlive){try{const q=await fetch('/api/queue',{cache:'no-store'});if(q.ok){const j=await q.json();qon=true;qinfo=new Map(j.printers.map(x=>[x.id,x]));qjobs=new Map(j.jobs.map(x=>[x.id,x]));qwaiting=j.waiting||0;}else{qon=false;qwaiting=0;queueAlive=false;}}catch{qon=false;queueAlive=false;}}
 raw=d.printers||[];canManage=!!(d.token&&d.writable);farmToken=canManage?d.token:null;
 looks=new Map(raw.map(p=>[p.id,lookOf(p,qinfo.get(p.id),qon)]));
 const grid=$('fm-grid'),ids=new Set(raw.map(p=>p.id));
 for(const [id,t] of tiles)if(!ids.has(id)){watch.unobserve(t.root);stop3d(t,false);dropModel(id);setLive(t,false);t.root.remove();tiles.delete(id);}
 for(const p of raw){let t=tiles.get(p.id);if(!t){t=build(p);tiles.set(p.id,t);grid.append(t.root);domKey='';}else t.p=p;}
 // A farm with no printer that answers and no demo gets one more card in the grid: try the demo farm (polish WO-02).
 // Polish WO-03: the 3D models banner, and no tile is drawn for two frames after the tiles were rewritten (their paint is a frame of its own).
 layout();demoCard(grid,canManage&&noPrinterYet(raw));licenceNote();modelsBanner(farmToken);window.stellaPauseDraws?.(2);document.title=`Farm · ${raw.length} ${raw.length===1?'printer':'printers'} · Stella Scope`;
 for(const b of document.querySelectorAll('[data-needs-manage]'))b.hidden=!canManage;}

// The sheets and dialogs are the HUD's, and they dress for the window by the body's layout class.
const layoutClass=()=>{const w=innerWidth,h=innerHeight;if(w<600)return 'phone';if(h<440&&w<1000)return 'phone-land';if(h<=640&&w<=1100&&w>h)return 'kiosk';if(w<1280)return h>w?'tablet-port':'tablet';return 'desktop';};
const applyLayout=()=>{const c=layoutClass();if(document.body.dataset.layout!==c)document.body.dataset.layout=c;};
applyLayout();
// A window resize can change the size of a tile's picture, the budget and what is on screen: look again, a moment after it settles.
let rz=0;addEventListener('resize',()=>{applyLayout();clearTimeout(rz);rz=setTimeout(()=>{if(raw.length)layout();stickyH();},150);});
// keep the first tile clear of the bar when Tab or find-in-page scrolls it into view
const stickyH=()=>{const s=document.querySelector('.fm-sticky');if(!s)return;const top=getComputedStyle(s).position==='sticky'?s.getBoundingClientRect().height:document.querySelector('.fm-top').getBoundingClientRect().height;document.documentElement.style.setProperty('--fm-sticky-h',Math.round(top+24)+'px');};
(async()=>{
 await loadSprite();initTips();
 const a=$('fm-actions');
 // WO-15 (TV/wall mode, the idea taken from print-farm-manager): /farm?tv=1 hangs the wall on a
 // screen with no controls: the header bar steps back, tiles fill everything, zoom cycles off.
 if(tv){document.body.classList.add('fm-tv');
  // F06 (2026-10-19): the wall screen has no buttons by design, and the desktop window has no address bar or Back, so a person who pressed Wall screen could not leave it.
  // Escape leaves it, and a small button shows for a few seconds whenever the pointer moves or the screen is touched, then goes again.
  const out=sq('close','Leave the wall screen','Leave the wall screen','Back to the farm with its buttons. Escape does the same.',()=>{location.href='/farm';}).el;out.classList.add('fm-tv-exit');document.body.append(out);
  let hide=0;const reveal=()=>{out.dataset.show='';clearTimeout(hide);hide=setTimeout(()=>delete out.dataset.show,3500);};
  addEventListener('pointermove',reveal);addEventListener('pointerdown',reveal);addEventListener('keydown',e=>{if(e.key==='Escape')location.href='/farm';});}
 const hk=(ico,label,does,onPress,fmAct)=>{const k=sq(ico,label,label,does,onPress);k.el.dataset.fmAct=fmAct;return k.el;};
 a.append(hk('grid','Cameras','Every camera on one screen. Tap one for the full picture with its printer\'s state over it.',()=>camerasView(),'cameras'));
 a.append(hk('files','Queue','Sliced files waiting for a printer that fits them.',()=>sheetQueue(),'queue'));
 startPops(a,{onWatch:e=>{if(openId!==e.printer)toggleOpen(e.printer);tiles.get(e.printer)?.root.scrollIntoView({block:'center',behavior:'smooth'});}});   // U07: Watch grows that printer's tile, its camera or machine bigger
 const models=async()=>{const d=await fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).catch(()=>({}));modelsSheet({token:d.writable&&d.token?d.token:null});};
 const more=hk('more','More','Printers, spools, analytics, models, alert settings, backup, the wall screen, Help and About.',()=>openSheet({title:'Farm',icon:'i-printers',build:body=>{
  const list=el('div','fl-more');
  const item=(ico,label,does,go,manage=false)=>{const b=button({icon:ico,label,size:'row',title:label,does,onPress:()=>{closeSheet(true);go();}});if(manage&&!canManage)b.hidden=true;const row=el('div','fl-more-row');row.append(b,el('small','',does));list.append(row);};
  // On a phone the header keeps only the alarms and this menu; the rest of its buttons are here.
  if(phone()){item('i-grid','Cameras','Every camera on one screen.',()=>camerasView());item('i-files','Queue','Sliced files waiting for a printer that fits them.',()=>sheetQueue());item('i-plus','Add a printer','Finds printers on this network, or takes an address you type.',()=>sheetAddPrinter(),true);}
  item('i-colors','Look','The colour, the finish and the dial style of this screen. Changes at once.',()=>openSheet({title:'Look',icon:'i-colors',build:b=>b.append(lookPanel())}));
  item('i-printers','Printers','Rename, recolour, set a camera, remove.',()=>sheetPrinters());
  item('i-spool','Spools','What is on each printer and how much is left.',()=>sheetSpools());
  item('i-chart','Analytics','Prints, success rate, hours, filament and cost.',()=>sheetAnalytics());
  item('i-view','Printer models','The real CAD of machines, each with its licence.',models);
  item('i-bell','Alert settings','Where alerts go (WhatsApp, ntfy, Discord, Slack) and what they tell you about.',()=>sheetAlerts(),true);
  item('i-key','Slicer upload','Send files from OrcaSlicer or PrusaSlicer straight into the queue.',()=>sheetSlicer(),true);
  item('i-backup','Backup','Save the farm to a file, or restore it.',()=>sheetBackup(),true);
  item('i-fullscreen','Wall screen','The wall for a screen across the room: no buttons, big tiles.',()=>{location.href='/farm?tv=1';});
  item('i-help','Help','The user guide: set up, add a printer, cameras, the queue, alerts, backup and what to do when something is wrong.',()=>{location.href='/help';});
  item('i-view','About','The version, the licences, who made what is inside Stella Scope, and how to reach 3D3D.',()=>sheetAbout());
  body.append(list);}}),'more');a.append(more);
 const add=hk('plus','Add a printer','Finds printers on this network, or takes an address you type.',()=>sheetAddPrinter(),'add');add.dataset.tone='primary';add.dataset.needsManage='';add.hidden=true;a.append(add);
 // the toolbar: search, what to show, how to sort, how big the tiles are
 const q=$('fm-q'),srt=$('fm-sort'),sel=$('fm-filter');
 for(const [k,l] of SORTS)srt.append(new Option(l,k));srt.value=view.sort;
 srt.addEventListener('change',()=>{view.sort=srt.value;store('fm-sort',view.sort);layout();});
 sel.addEventListener('change',()=>setFilter(sel.value));
 q.addEventListener('input',()=>{view.q=q.value;layout();});
 q.addEventListener('keydown',e=>{if(e.key==='Escape'&&q.value){q.value='';view.q='';layout();e.stopPropagation();}});
 // (U04: no tile size switch: there is one size)
 // back to the top once a long way down (a hundred printers is a long page)
 const up=sq('arrow-up','Top','Back to the top','Scrolls to the top of the wall, where the search and the sort are.',()=>scrollTo({top:0,behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth'})).el;
 up.classList.add('fm-totop');up.hidden=true;document.body.append(up);
 // The top button shows on the way up (a finger, a wheel or a key moving the page up) once the search and the sort have scrolled out of
 // sight; a big window keeps them on screen and has no use for it. It goes after a moment, or when the page goes down again.
 const stuck=()=>getComputedStyle(document.querySelector('.fm-sticky')).position==='sticky';let sp=0,lastY=0,inputAt=0,upTimer=0;
 for(const ev of ['touchstart','touchmove','wheel','keydown','pointerdown'])addEventListener(ev,()=>{inputAt=Date.now();},{passive:true,capture:true});
 addEventListener('scroll',()=>{lastScrollAt=Date.now();if(sp)return;sp=requestAnimationFrame(()=>{sp=0;
  const dy=scrollY-lastY;lastY=scrollY;const driven=Date.now()-inputAt<500;
  if(driven&&!stuck()&&scrollY>innerHeight*1.2&&dy<-6){up.hidden=false;clearTimeout(upTimer);upTimer=setTimeout(()=>{up.hidden=true;},2800);}else if(scrollY<innerHeight*1.2||dy>6){up.hidden=true;}});},{passive:true});
 await refresh();stills();stickyH();
 setInterval(()=>{if(!document.hidden)refresh();},2000);
 setInterval(stills,4000);setTimeout(stills,1500);
 document.addEventListener('visibilitychange',()=>{if(document.hidden)allocate();else{refresh();stills();}});
 // 1 to 9 open the printers in the order shown; / goes to the search
 document.addEventListener('keydown',e=>{if(e.ctrlKey||e.metaKey||e.altKey||/^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)||e.target.isContentEditable||document.querySelector('.nx-modal,#nx-sheet:not([hidden])'))return;
  if(e.key==='Escape'&&openId){toggleOpen(openId);return;}
  if(e.key==='/'){e.preventDefault();q.focus();return;}
  const n=Number(e.key);if(n>=1&&n<=9&&order[n-1])location.href='/p/'+order[n-1];});
 // #sheet=queue|spools|analytics|backup|look opens that panel once the wall is up (a link to it, and the page checks)
 {const want=new URLSearchParams(location.hash.slice(1)).get('sheet'),open={queue:()=>sheetQueue(),spools:()=>sheetSpools(),analytics:()=>sheetAnalytics(),backup:()=>sheetBackup(),printers:()=>sheetPrinters(),look:()=>openSheet({title:'Look',icon:'i-colors',build:b=>b.append(lookPanel())})}[want];if(open)setTimeout(open,1500);}
})();
