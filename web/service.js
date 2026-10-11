import {PAGES,SLOTS,slotById,CORNERS,SOURCES,n,f,safeUrl,emptyBook,validateBook,appendRevision,partsAt,mergeBook,journal,diagnose,signalPart} from './service-core.js';
import {createScene} from './hud-scene.js';
import {instanceRecordId,insideTravel,formatSignal,signalUnit,printerLink} from './hud-core.js';
import {mountHUD} from './hud.js';
import {createDial,createBar,createRing} from './service-dials.js';
const $=id=>document.getElementById(id),h=new URLSearchParams(location.hash.slice(1));
const partNames={head:'Toolhead',hotend:'Hotend',probe:'BLTouch',extruder:'Extruder',fan:'Part-cooling fan',hotend_fan:'Hotend cooling fan',controller_fan:'Electronics cooling fan',controller:'Control board',host:'Raspberry Pi host',psu:'DC power supply',ssr:'Bed switching relay',distribution:'Power distribution',inlet:'Mains inlet',frame:'Frame',deck:'Electronics deck',gantry:'Flying gantry',x_rail:'X crossbeam',y_rails:'Y rails',stepper_x:'Rear B / CoreXY motor',stepper_y:'Rear A / CoreXY motor',heater_bed:'Fixed bed',...Object.fromEntries(CORNERS.map(([,name,id])=>[id,`${name} Z drive`]))};
const initialPage=h.get('page')||location.pathname.split('/').filter(Boolean)[1];
const state={page:PAGES.some(p=>p[0]===initialPage)?initialPage:'machine',part:h.get('part')||'head',cursor:h.has('ts')?Number(h.get('ts')):null,live:!h.has('ts'),snapshot:null,seq:0,controller:null,busy:false,playing:false,inspector:'signals',book:null,bookKey:null,catalogue:[],signal:null,signalSeq:0,bounds:null,requestError:null,tick:null,prevTick:null,detail:null};
// The page's own link to the Stella Scope service (serviceLost / serviceBack, below the tick
// loops). Declared here, before anything can draw. While it is down, state.link holds
// {printer_state:'service'} so the one state rule (printerLink) says so everywhere.
const SERVICE_BACKOFF_S=[1,2,5,10],svc={down:false,since:null,fails:0,next:0,link:null};
let assembly=null,assemblyLoading=false,hud=null;
let scene=null,toastTimer,sliderTimer,editVersion=0,editSlot=null,lastPlay=0;
const el=(tag,cls='',value)=>{const e=document.createElement(tag);if(cls)e.className=cls;if(value!==undefined)e.textContent=value;return e;};
const setText=(id,v)=>{const e=$(id);if(e&&e.textContent!==v)e.textContent=v;};
const stamp=ts=>n(ts)===null?'No recorded time':new Date(ts*1000).toLocaleString(undefined,{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',second:'2-digit'});
const local=ts=>{const d=new Date(ts*1000);return new Date(d.getTime()-d.getTimezoneOffset()*60000).toISOString().slice(0,19);};
const value=v=>v===null||v===undefined?'Unknown':typeof v==='boolean'?(v?'Yes':'No'):typeof v==='number'?f(v,Number.isInteger(v)?0:3):String(v);
function error(e){$('error').hidden=false;$('error').textContent=e.message||String(e);}
function toast(s){$('toast').hidden=false;$('toast').textContent=s;clearTimeout(toastTimer);toastTimer=setTimeout(()=>$('toast').hidden=true,4000);}
function btn(label,action,cls=''){const b=el('button',cls,label);b.type='button';b.addEventListener('click',action);return b;}
function link(label,url){const a=el('a','',label),safe=safeUrl(url);if(safe){a.href=safe;a.target='_blank';a.rel='noopener noreferrer';a.referrerPolicy='no-referrer';}return a;}
// The HUD shows one tooltip for every control, saying what it is (title) and what clicking does
// (data-tip-do). Outside the HUD the title is the browser's own tooltip.
function tipped(e,what,does=''){e.title=what;if(does)e.dataset.tipDo=does;return e;}
function kv(parent,key,val){const d=el('div','kv');d.append(el('span','',key),el('b','',value(val)));parent.append(d);}
function card(label,val,note=''){const d=el('article','card');d.append(el('span','',label),el('strong','',val),el('small','',note));return d;}
function table(headers,rows){const t=el('table'),head=el('thead'),hr=el('tr');headers.forEach(x=>hr.append(el('th','',x)));head.append(hr);t.append(head);const body=el('tbody');for(const row of rows){const tr=el('tr');for(const v of row){const td=el('td');v instanceof Node?td.append(v):td.textContent=value(v);tr.append(td);}body.append(tr);}if(!rows.length){const tr=el('tr'),td=el('td','empty','No matching recorded data.');td.colSpan=headers.length;tr.append(td);body.append(tr);}t.append(body);return t;}
function described(top,bottom){const x=el('div');x.append(el('span','',top),el('small','',bottom));return x;}
function route(page){if(!PAGES.some(x=>x[0]===page))page='machine';state.page=page;updateUrl();render();}
function updateUrl(){const p=new URLSearchParams({page:state.page,part:state.part});if(!state.live&&n(state.cursor)!==null)p.set('ts',state.cursor);history.replaceState(null,'','#'+p);}
function select(id){state.part=id;updateUrl();scene?.set(state.snapshot,id);inspector();if(state.page==='parts')parts();}
function pause(){state.playing=false;$('play').textContent='Play';}
// Every request here also tells the page whether its own Stella Scope service is there. One that cannot
// reach it at all (stopped, restarting) marks it down; any answer marks it back.
async function get(url,signal){let r;try{r=await fetch(url,{cache:'no-store',signal});}catch(e){if(e.name!=='AbortError')serviceLost();throw e;}serviceBack();const v=await r.json();if(!r.ok)throw Error(v.error||`HTTP ${r.status}; check the recorder log.`);return v;}
function loadBook(){const key=(state.snapshot?.recording_origin==='demo'?'stella-demo:':'stella-live:')+'parts:v1:stella-voron-350';if(state.bookKey===key)return;state.bookKey=key;try{const raw=localStorage.getItem(key);state.book=raw?validateBook(JSON.parse(raw)):emptyBook();}catch(e){state.book=null;error(Error('Parts book cannot be loaded. It has not been overwritten: '+e.message));}}
function storeBook(book,expected=state.book?.version){if(!state.book)throw Error('Existing parts storage is unavailable; do not overwrite it.');const current=localStorage.getItem(state.bookKey),version=current?validateBook(JSON.parse(current)).version:0;if(version!==expected)throw Error('Another tab changed the parts book. Reload before saving; your existing revisions were not overwritten.');const next=validateBook(book);localStorage.setItem(state.bookKey,JSON.stringify(next));state.book=next;return true;}
window.addEventListener('storage',e=>{if(e.key===state.bookKey)toast('Parts book changed in another tab. Reload before editing to avoid a conflict.');});
function clearSignalHistory(){state.signalSeq++;$('signal-history').replaceChildren();plot($('signal-chart'),[],0,1);}
async function request(ts=state.live?null:state.cursor,step=null){const seq=++state.seq;state.controller?.abort();state.controller=new AbortController();state.busy=true;lastFull=performance.now();if(state.signal)clearSignalHistory();const q=new URLSearchParams({window:$('window').value,points:60});if(n(ts)!==null)q.set('ts',ts);if(step)q.set('step',step);$('capture-status').textContent='Reading selected snapshot…';
try{const r=await get('/api/workspace?'+q,state.controller.signal);if(seq!==state.seq)return;state.snapshot=r;if(r.observer_now)state.link=r.observer_now;state.cursor=r.cursor;state.bounds={first:r.first_ts,last:r.last_ts};if(state.requestError&&$('error').textContent===state.requestError)$('error').hidden=true;state.requestError=null;if(!state.bookKey&&scene)$('error').hidden=true;loadBook();updateUrl();render();}
catch(e){if(e.name!=='AbortError'&&seq===state.seq){if(svc.down){pause();render();return;}state.snapshot=null;state.requestError=e.message||String(e);pause();scene?.set(null,state.part);error(e);render();}}
finally{if(seq===state.seq)state.busy=false;}}
function seek(ts,step=null){if(!Number.isFinite(ts))return;pause();state.live=false;state.cursor=ts;request(ts,step);}
function telemetry(){return state.snapshot?.point?.telemetry||{signals:[],pins:[],controllers:[],fans:[],sensors:[],bltouch:{},coverage:null,host:null};}
// A failed request used to reset the range to 0..1, preventing a historical retry.
// Instrument wall (2026-09-18 cockpit restyle): the machine page's right rail is a
// dense readout stack — recorder link, print job, heat dials, motion, cooling,
// sensors. Built once; ticks update values and animate needles in place.
const wall={built:false,nozzle:null,bed:null,speed:null,ring:null,fanRows:[],sensorRows:[]};
function wallRow(container,label){const r=el('div','row');r.append(el('span','',label),el('b','','—'));container.append(r);return r;}
function wallPanel(title,cls){const p=el('div','ipanel'+(cls?' '+cls:''));p.append(el('h4','',title));$('instrument-wall').append(p);return p;}
function jobColor(s){return s==='printing'||s==='paused'?'var(--orange)':s==='complete'?'var(--green)':s==='error'?'var(--magenta)':'var(--muted)';}
function dur(s){if(n(s)===null)return '—';const m=Math.floor(s/60),h=Math.floor(m/60);return h>0?`${h}h ${m%60}m`:m>0?`${m}m ${Math.floor(s%60)}s`:`${Math.floor(s)}s`;}
function buildWall(){wall.built=true;wall.fanRows=[];wall.sensorRows=[];const W=$('instrument-wall');W.replaceChildren();
 const link=wallPanel('recorder link','wide');
 wall.transport=wallRow(link,'transport').lastElementChild;wall.keepalive=wallRow(link,'keepalive').lastElementChild;wall.updates=wallRow(link,'stream rate').lastElementChild;wall.activity=wallRow(link,'activity').lastElementChild;
 const job=wallPanel('print','wide');
 const head=el('div','row');head.append(el('span','','state'));wall.jobState=el('b','state-word','—');head.append(wall.jobState);job.append(head);
 wall.jobName=el('div','job-name','—');job.append(wall.jobName);
 const grid=el('div','print-grid'),jcv=el('canvas','mini-ring'),rows=el('div','print-rows');
 grid.append(jcv);wall.elapsed=wallRow(rows,'elapsed').lastElementChild;wall.remaining=wallRow(rows,'remaining').lastElementChild;wall.progress=wallRow(rows,'progress').lastElementChild;grid.append(rows);job.append(grid);
 wall.ring=createRing(jcv,{label:'PROGRESS'});
 const heat=wallPanel('heat');
 const hp=el('div','dial-pair'),ncv=el('canvas','mini-dial'),bcv=el('canvas','mini-dial');hp.append(ncv,bcv);heat.append(hp);
 wall.nozzle=createDial(ncv,{max:300,unit:'°C',label:'NOZZLE',accent:'--orange',targetColor:'--teal',digits:0});
 wall.bed=createDial(bcv,{max:120,unit:'°C',label:'BED',accent:'--orange',targetColor:'--teal',digits:0});
 const motion=wallPanel('motion');
 wall.x=wallRow(motion,'x mm').lastElementChild;wall.y=wallRow(motion,'y mm').lastElementChild;wall.z=wallRow(motion,'z mm').lastElementChild;
 const scv=el('canvas','mini-bar');motion.append(scv);
 wall.speed=createBar(scv,{max:350,unit:'mm/s',label:'TRAJECTORY SPEED',accent:'--teal',digits:0});
 const cool=wallPanel('cooling');wall.cool=cool;
 for(let i=0;i<4;i++)wall.fanRows.push(wallRow(cool,'—'));
 const sens=wallPanel('sensors');wall.sensors=sens;wall.sensorNote=el('p','hud-muted','No additional temperature sensors captured.');sens.append(wall.sensorNote);
 for(let i=0;i<3;i++)wall.sensorRows.push(wallRow(sens,'—'));
}
function updateWall(p,job,obs,act){if(!wall.built)buildWall();const t=telemetry();
 wall.transport.textContent=replay()?'REPLAY':obs?.transport?obs.transport.toUpperCase():'Unknown';wall.transport.className=obs?.transport==='push'?'ok':'';
 wall.keepalive.textContent=n(obs?.keepalive_ms)!==null?f(obs.keepalive_ms,0)+' ms':'—';
 wall.updates.textContent=n(obs?.updates_per_s_1m)!==null?f(obs.updates_per_s_1m,1)+'/s':'—';
 wall.activity.textContent=act?.label||'—';
 const js=job?.state||'unknown';wall.jobState.textContent=js.toUpperCase();wall.jobState.style.color=jobColor(js);
 wall.jobName.textContent=job?.name||'No job reported';
 const prog=n(job?.progress);wall.ring.set(prog);
 wall.elapsed.textContent=dur(job?.duration);
 wall.remaining.textContent=prog!==null&&prog>0&&n(job?.duration)!==null?dur(job.duration*(1-prog)/prog):'—';
 wall.progress.textContent=prog===null?'—':f(prog*100,1)+'%';
 const nt=n(p?.nozzle?.temperature),nT=n(p?.nozzle?.target),bt=n(p?.bed?.temperature),bT=n(p?.bed?.target);
 wall.nozzle.set(nt,nT??null,dashStatus(nt,nT));wall.bed.set(bt,bT??null,dashStatus(bt,bT));
 wall.speed.set(n(p?.velocity));
 const pos=p?.position||[];for(const [i,k] of ['x','y','z'].entries())wall[k].textContent=f(n(pos[i])===null?null:pos[i],1);
 const fans=t.fans||[];while(wall.fanRows.length<fans.length)wall.fanRows.push(wallRow(wall.cool,'—')); for(let i=0;i<wall.fanRows.length;i++){const r=wall.fanRows[i],fan=fans[i];r.hidden=!fan;if(fan){r.firstElementChild.textContent=fan.object;r.lastElementChild.textContent=(n(fan.command)===null?'—':f(fan.command*100,0)+'%')+(n(fan.rpm)===null?'':' · '+f(fan.rpm,0)+' rpm');}}
 const sensors=t.sensors||[];wall.sensorNote.hidden=sensors.length>0;while(wall.sensorRows.length<sensors.length)wall.sensorRows.push(wallRow(wall.sensors,'—')); for(let i=0;i<wall.sensorRows.length;i++){const r=wall.sensorRows[i],sn=sensors[i];r.hidden=!sn;if(sn){r.firstElementChild.textContent=sn.object;r.lastElementChild.textContent=f(sn.temperature,1)+' °C';}}
}
function dashStatus(temp,target){const t=n(target);if(t===null||t<=0)return{text:'OFF',color:'--muted'};const v=n(temp);if(v===null)return{text:'NO DATA',color:'--muted'};return Math.abs(v-t)<1.5?{text:'AT SET',color:'--green'}:v<t?{text:'HEATING',color:'--orange'}:{text:'ACTIVE',color:'--orange'};}
function dialCard(field,label,canvasClass){const c=el('div','card');c.dataset.field=field;const cv=el('canvas',canvasClass);const wrap=el('div','dial-slot');wrap.append(cv);c.append(el('span','',label),wrap,el('small',''));return[c,cv,c.querySelector('small')];}

function replay(){return state.snapshot?.observer_now?.mode==='replay'||state.tick?.observer?.mode==='replay';}
function stats(){const s=state.snapshot,p=s?.point,bounds=s?{first:s.first_ts,last:s.last_ts}:state.bounds;const unknown='Unknown';$('origin').textContent=s?.recording_origin==='demo'?'SYNTHETIC DEMONSTRATION':s?.recording_origin==='live'?'RECORDED PRINTER DATA':'RECORDING ORIGIN UNKNOWN';$('capture-status').textContent=p?`${state.live?'Latest capture':'Rewind'} · ${stamp(p.ts)} · ${f(s.age_seconds,2)} s before cursor`:'No usable recorded sample';$('moment-label').textContent=p?`Selected ${stamp(s.cursor)} · captured ${stamp(p.ts)}`:'No usable recorded sample';$('cursor-time').textContent=stamp(state.cursor);$('first-time').textContent=stamp(bounds?.first);$('last-time').textContent=stamp(bounds?.last);$('cursor').min=bounds?.first??0;$('cursor').max=bounds?.last??1;$('cursor').value=state.cursor??1;
for(const id of ['play','prev','next','copy-link'])$(id).disabled=!p;$('cursor').disabled=n(bounds?.first)===null||n(bounds?.last)===null;$('live').classList.toggle('primary',state.live);
$('xyz').textContent=p?`X ${f(p.position?.[0])} / Y ${f(p.position?.[1])} / Z ${f(p.position?.[2])} mm`:'POSITION UNKNOWN'; $('pose-note').textContent=!p||!p.position||p.position.every(x=>n(x)===null)?'Position not reported':p.position.some(x=>n(x)===null)?'Holding last reported axes · not all axes reported (probing/homing)':s.age_seconds>2?'OLDER SAMPLE · not a moving physical-printer view':'Klipper requested position · not encoder feedback';
 if(replay()){$('origin').textContent='SAVED RECORDING · READ ONLY';$('live').textContent='Latest sample';$('capture-status').textContent=p?'Recorded '+stamp(p.ts):'No recorded sample';}
 updateWall(p,state.live?lastTick?.job||p?.job:p?.job,state.live?lastTick?.observer:null,state.live?lastTick?.activity:null);
}
function inspector(){const slot=slotById(state.part),parent=slot?.parent||state.part,p=state.snapshot?.point,t=telemetry();$('part-name').textContent=slot?`${slot.location} · ${slot.name}`:partNames[parent]||parent;$('part-desc').textContent=slot?slot.description:parent==='probe'?'The reference model shows a BLTouch. Verify the installed probe against Klipper configuration and the actual machine; deployment and continuous trigger edges are not reported by this feed.':'Click a signal to inspect its provenance. Imported CAD is a source revision; fitted components require verified mapping.';
const c=$('inspector-content');c.replaceChildren();document.querySelectorAll('[data-inspector]').forEach(b=>b.classList.toggle('active',b.dataset.inspector===state.inspector));
if(state.inspector==='parts'){const records=state.book?partsAt(state.book):new Map(),slots=SLOTS.filter(x=>x.parent===parent||x.id===state.part);for(const s of slots){const r=records.get(s.id),b=btn(s.name,()=>editPart(s),'location-button');b.append(el('small','',r?`${r.model||r.name} · ${r.revision||'revision unknown'} · ${r.state}`:'Identity not yet recorded'));c.append(b);}if(!slots.length)c.append(el('p','empty','Use Parts to add a custom location for this assembly.'));return;}
if(state.inspector==='inspect'){const symptom=parent==='probe'?'probe':parent==='hotend'||parent==='heater_bed'?'heater':parent.includes('fan')?'fan':parent.startsWith('stepper_z')||parent==='gantry'?'corner':'communication';c.append(el('p','hint','Inspection guidance is a set of hypotheses and safe checks, not an automatic diagnosis.'),btn('Inspect suspected causes',()=>{$('symptom').value=symptom;route('diagnostics');}),btn('Record service work',()=>openWork(state.part)));return;}
if(!p){c.append(el('p','empty','No readings at this time.'));return;}
if(p.motors?.[parent]){const m=p.motors[parent];kv(c,'Run current setting',m.current);kv(c,'Hold current setting',m.hold);kv(c,'Driver current scale',m.scale);kv(c,'Energised',m.energised);kv(c,'Standstill flag',m.standstill);kv(c,'Flags',m.flags===null?'Unknown':m.flags?.join(', ')||'No flags in known status');c.append(el('p','hint','Current scale is not mechanical load or proof of bolt tightness.'));}
if(parent==='hotend'||parent==='heater_bed'){const x=parent==='hotend'?p.nozzle:p.bed;kv(c,'Temperature °C',x?.temperature);kv(c,'Target °C',x?.target);kv(c,'Heater output command',x?.power);}
if(parent==='probe'){kv(c,'Recorded BLTouch config',t.bltouch?.configured);kv(c,'Sensor input',t.bltouch?.sensor_pin);kv(c,'Deployment control',t.bltouch?.control_pin);kv(c,'Cached last query',t.bltouch?.last_query);kv(c,'Pin extension','Not reported');c.append(el('p','hint','Open Prepare & calibrate to review supported tests and their execution gates.'));}
if(parent==='head'||parent==='gantry'){kv(c,'X / Y / Z mm',(p.position||[]).map(x=>f(x)).join(' / '));kv(c,'Requested speed mm/s',p.velocity);kv(c,'Homed axes',p.homed);}
const signals=(t.signals||[]).filter(s=>signalPart(s.object)===parent).slice(0,14);for(const s of signals){const b=btn(`${s.field}: ${value(s.value)}`,()=>{state.signal=s;route('signals');showSignal(s);},'location-button');b.append(el('small','',`${s.object} · ${s.kind}`));c.append(b);}if(!signals.length&&!p.motors?.[parent]&&!['head','gantry','hotend','heater_bed','probe'].includes(parent))c.append(el('p','hint','No mapped telemetry for this physical item. Its identity, version and condition must be recorded in Parts.'));
}
function plot(canvas,series,from,to,unit=''){const r=canvas.getBoundingClientRect();if(!r.width)return;const d=Math.min(devicePixelRatio||1,2),w=r.width,h=r.height;canvas.width=w*d;canvas.height=h*d;const c=canvas.getContext('2d');c.scale(d,d);c.font='12px "Geist Mono",monospace';const vals=series.flatMap(s=>s.points.map(p=>n(p.value))).filter(x=>x!==null);c.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim();if(!vals.length){c.fillText('No numeric readings in this selected history.',15,40);return;}let lo=Math.min(...vals),hi=Math.max(...vals);if(hi===lo){hi+=1;lo-=1;}const pad=(hi-lo)*.1;lo-=pad;hi+=pad;const x=t=>45+(w-60)*(t-from)/Math.max(.01,to-from),y=v=>15+(h-50)*(hi-v)/(hi-lo);c.strokeStyle=getComputedStyle(canvas).getPropertyValue('--line').trim();for(let i=0;i<=4;i++){const v=lo+(hi-lo)*i/4;c.beginPath();c.moveTo(42,y(v));c.lineTo(w-10,y(v));c.stroke();c.fillText(v.toFixed(1),0,y(v)+4);}series.forEach((s,i)=>{c.strokeStyle=['#04D9C4','#FF6A00','#FF2D92'][i%3];c.fillStyle=c.strokeStyle;c.beginPath();let prior=null;for(const p of s.points){if(n(p.value)===null){prior=null;continue;}if(prior&&p.ts-prior.ts<=Math.max(3,(to-from)/Math.max(1,s.points.length-1)*2))c.lineTo(x(p.ts),y(p.value));else c.moveTo(x(p.ts),y(p.value));prior=p;}c.stroke();c.fillText(s.name+(unit?' · '+unit:''),50+i*160,h-8);});}
// Signal workbench (2026-09-18): left control stack (hint, coverage strip, filters, dense
// signal list of button rows with kind chips), right glass hero (name, 32px live readout,
// big chart, mono history). The hero readout is owned here — row clicks and every render
// refresh it from the live sample, so showSignal() keeps its existing title/detail/history flow.
function signals(){const t=telemetry(),q=$('signal-search').value.toLowerCase(),kind=$('signal-kind').value,rows=(t.signals||[]).filter(s=>(!q||`${s.object} ${s.field} ${s.kind}`.toLowerCase().includes(q))&&(!kind||s.kind===kind));
 const CHIP={'sensor reading':'k-sensor','command / setting':'k-command','cached query':'k-cached','requested motion':'k-motion','reported statistics':'k-stats'};
 // Reviewer: values print by unit (hud-core formatSignal), never at full precision.
 const readout=s=>{const v=$('signal-value');if(!v)return;if(!s){v.replaceChildren('—');return;}const text=formatSignal(s.object,s.field,s.value),u=signalUnit(s.object,s.field)?.unit;if(u&&text.endsWith(' '+u)){v.replaceChildren(text.slice(0,-u.length-1));v.append(el('small','',u));}else v.replaceChildren(text);};
 const list=$('signal-table');list.replaceChildren();
 for(const s of rows){const r=btn('',()=>{list.querySelectorAll('.sig-row.active').forEach(x=>x.classList.remove('active'));r.classList.add('active');readout(s);showSignal(s);},'sig-row'+(state.signal&&state.signal.object===s.object&&state.signal.field===s.field?' active':''));tipped(r,`${s.object} / ${s.field} · ${s.kind}`,`${s.meaning?String(s.meaning).replace(/\.?$/,'. '):''}Clicking shows its reading and history.`);r.append(el('span','sig-name',`${s.object} / ${s.field}`),el('i','sig-chip '+(CHIP[s.kind]||'k-cached'),s.kind),el('b','sig-val',formatSignal(s.object,s.field,s.value)));list.append(r);}
 if(!rows.length)list.append(el('p','empty','No matching recorded signals.'));
 readout(state.signal?(t.signals||[]).find(x=>x.object===state.signal.object&&x.field===state.signal.field)||state.signal:null);
 const cv=t.coverage;$('coverage-summary').replaceChildren();for(const [label,count]of [['Signals shown',(t.signals||[]).length],['Captured objects',cv?.captured?.length??'Unknown'],['Discovered objects',cv?.available?.length??'Unknown'],['Omitted objects',cv?.omitted?.length??'Unknown']]){const d=el('div','sig-fact');d.append(el('strong','',String(count)),el('span','',label));$('coverage-summary').append(d);}
 const d=$('coverage-detail');d.replaceChildren();d.append(el('p','hint',cv?.policy||'This recording predates automatic coverage metadata. Absence is not evidence that a sensor does not exist.'));if(t.limit_reached)d.append(el('p','error','Signal display cap reached; not every scalar field is shown.'));if(t.pin_limit_reached)d.append(el('p','error','Configured-pin display cap reached.'));for(const x of cv?.omitted||[])d.append(el('p','hint',`${x.object}: ${x.reason}`));for(const x of t.unmeasured||[])d.append(el('p','hint',x));}
async function showSignal(s){state.signal=s;clearSignalHistory();const seq=state.signalSeq;$('signal-title').textContent=`${s.object} / ${s.field}`;$('signal-detail').textContent=s.kind+' — '+s.meaning;if(typeof s.value!=='number'&&typeof s.value!=='boolean'){$('signal-history').append(el('p','hint','This field is not numeric. The selected value is shown in the signal table.'));return;}
try{const r=await get('/api/signal-history?'+new URLSearchParams({object:s.object,field:s.field,ts:state.cursor,window:$('window').value,points:60}));if(seq!==state.signalSeq)return;plot($('signal-chart'),[{name:'Recorded',points:r.points.map(p=>({...p,value:typeof p.value==='boolean'?Number(p.value):p.value}))}],r.from,r.to);$('signal-history').replaceChildren(table(['Capture time','Value'],r.points.slice().reverse().map(p=>[stamp(p.ts),value(p.value)])));$('signal-detail').textContent=r.meaning+' '+r.method;}catch(e){if(seq===state.signalSeq)error(e);}}
function electronics(){const t=telemetry();
 // Power bay (2026-09-18 restyle): bay-map strip → .ipanel controller cards with mono
 // stat rows → pin paths as dense mono tables (two side-by-side halves past 16 pins)
 // → host cache as a slim capped strip. Ids and click handlers unchanged.
 $('topology').replaceChildren();
 for(const [i,[id,label,sub]]of [['host','Raspberry Pi / Klipper','Scheduling, reporting, host statistics'],['controller','MCU / control board','Firmware, counters, configured pads'],['probe','BLTouch / sensors','Sensor input and deployment control'],['hotend','Heaters / fans / drives','Commands versus actual feedback']].entries()){if(i)$('topology').append(el('span','arrow','→'));const b=tipped(btn(label,()=>{select(id);route('machine');scene?.focus(id);}),label,'Shows it on the printer.');b.append(el('small','',sub));$('topology').append(b);}
 const row=(p,label,v)=>{if(v&&typeof v==='object')v=JSON.stringify(v);const r=el('div','row');r.title=String(v??'');r.append(el('span','',label),el('b','',value(v)));p.append(r);};
 $('controller-cards').replaceChildren();
 for(const m of t.controllers||[]){const c=el('div','ipanel');c.append(el('h4','',m.object||'Control board'));row(c,'chip',m.chip||'Chip not reported');row(c,'board identity',m.board_identity);row(c,'firmware',m.firmware);row(c,'clock hz',m.clock_hz);for(const [k,v]of Object.entries(m.stats||{}).slice(0,12))row(c,k,v);$('controller-cards').append(c);}
 if(!(t.controllers||[]).length)$('controller-cards').append(el('p','empty','Controller metadata is not recorded. Auto-discovery captures mcu and named MCUs when they exist.'));
 for(const s of t.config_sections||[]){const c=el('div','ipanel');c.append(el('h4','',s.name),el('small','elec-note','Configuration section · no live status object exists · read from the recorded configfile when captured'));for(const [k,v]of Object.entries(s.fields||{}).slice(0,14))row(c,k,typeof v==='object'?JSON.stringify(v):v);$('controller-cards').append(c);}
 const pins=t.pins||[],half=Math.ceil(pins.length/2),cols=pins.length>16?[pins.slice(0,half),pins.slice(half)]:[pins];
 $('pin-table').replaceChildren(...cols.map(g=>table(['Component','Signal','Configured pin','Ctrl / pad','Connector / basis'],g.map(p=>[tipped(btn(p.component,()=>{select(signalPart(p.component));route('machine');},'link'),p.component,'Shows this component on the printer.'),p.signal,p.configured_pin,`${p.controller} / ${p.pad}`,described(p.physical_connector||'Connector not verified',p.basis)]))));
 $('host-data').replaceChildren();
 if(t.host?.data)$('host-data').append(el('p','elec-hint',`Cached host values captured ${stamp(t.host.captured_at)} · cached separately from the 4 Hz status · no newer host reading is inserted into rewind`),el('pre','',JSON.stringify(t.host.data,null,2)));
 else $('host-data').append(el('p','elec-hint',t.host?.error||'Host statistics were not collected. Set telemetry.host_stats=true in the recorder JSON for future recordings; discovered Klipper system_stats is in Signals.'));}
function parts(){if(!state.book){$('parts-table').replaceChildren(el('p','error','Parts storage is unavailable; existing data has not been overwritten.'));return;}const at=$('parts-time').value==='cursor'?state.cursor??0:Infinity,records=partsAt(state.book,at),q=$('parts-search').value.toLowerCase(),loc=$('parts-location').value,group=$('parts-group').value;const all=[...SLOTS,...[...records.values()].filter(x=>!slotById(x.id)).map(r=>({id:r.id,name:r.name,location:r.location,group:'Custom',parent:'frame'}))];const list=all.filter(s=>(!loc||s.location===loc)&&(!group||s.group===group)&&(!q||[s.id,s.name,s.location,records.get(s.id)?.model,records.get(s.id)?.revision].join(' ').toLowerCase().includes(q)));
$('book-status').textContent=`${SLOTS.length} template locations · ${records.size} locations with entered revisions · ${state.book.revisions.length} history entries · ${state.bookKey?.startsWith('stella-demo')?'DEMO book':'Local machine book'} · ${at===Infinity?'latest entered':'effective at '+stamp(at)}`;
$('parts-table').replaceChildren(table(['Location / piece','Recorded identity','Revision / state','Material / hardware','Source','Record'],list.map(s=>{const r=records.get(s.id);const src=el('div');if(r?.source_url)src.append(tipped(link('Source',r.source_url),'Source','Opens the design source in a new tab.'));if(r?.stl_url)src.append(el('span','',' · '),tipped(link('STL',r.stl_url),'STL','Opens the exact STL in a new tab.'));return [tipped(btn(`${s.location} · ${s.name}`,()=>{select(s.id);route('machine');scene?.focus(s.id);},'link'),`${s.location} · ${s.name}`,'Shows this piece on the printer.'),r?.model||r?.name||'Not identified',r?described(r.revision||'Revision unknown',r.state):'Unverified location',r?described(r.material||'Unknown',r.hardware||'Hardware not recorded'):'Not recorded',src,tipped(btn(r?'New revision':'Identify',()=>editPart(s)),r?'New revision':'Identify',r?'Opens the form for a new revision of this piece. Earlier records are kept.':'Opens the form to record what this piece is.')];})));
}
function catalogue(){const q=$('catalogue-search').value.toLowerCase().trim(),rows=state.catalogue.filter(x=>!q||x.path.toLowerCase().includes(q)).slice(0,60);$('catalogue-results').replaceChildren();$('catalogue-note').textContent=`${state.catalogue.length} pinned official STL entries · showing up to 60 matches · catalogue references are candidates, not installed-part verification`;for(const row of rows){const c=el('article');c.append(tipped(link(row.path.split('/').pop(),row.url),row.path.split('/').pop(),'Opens the pinned official STL in a new tab.'),el('small','',row.path),el('small','',`Git blob ${row.blob_sha?.slice(0,12)} · not a file SHA-256`),tipped(btn('Use as candidate reference',()=>{if(!editSlot){toast('Choose a part location first using Identify, then use the candidate reference.');return;}editPart(editSlot);$('field-source_url').value=row.url;$('field-stl_url').value=row.url;$('field-source_commit').value=row.commit;$('part-state').value='candidate';}),'Use as candidate reference','Copies this STL into the open part form as a candidate source. Choose a location with Identify first.'));$('catalogue-results').append(c);}if(!rows.length)$('catalogue-results').append(el('p','empty','No matching catalogue entries.'));}
const FIELDS=[['id','Location ID'],['name','Piece name'],['location','Physical location'],['model','Installed model / maker'],['revision','Version / revision'],['material','Printed material'],['colour','Colour'],['print_settings','Print settings / orientation'],['hardware','Fasteners / inserts / connector labels'],['reason','Why installed / why changed'],['source_url','Design / supplier source URL'],['stl_url','Exact STL URL'],['source_commit','Source version / Git commit'],['file_sha256','Actual STL SHA-256 fingerprint'],['license','Design licence'],['evidence','Identification evidence / inspection note']];
for(const [id,label]of FIELDS){const l=el('label','',label),i=el(['reason','evidence','hardware','print_settings'].includes(id)?'textarea':'input');i.id='field-'+id;i.name=id;i.maxLength=['reason','evidence','print_settings'].includes(id)?2000:512;if(['name','reason','id'].includes(id))i.required=true;tipped(i,label,'Saved with the next revision.');l.append(i);$('part-fields').append(l);}
function editPart(slot){if(!state.book)return;editSlot=slot;editVersion=state.book.version;const r=partsAt(state.book).get(slot.id);for(const [id]of FIELDS)$('field-'+id).value=r?.[id]??slot[id]??'';$('field-id').readOnly=!!slotById(slot.id)||slot.id.startsWith('cad:');$('field-reason').value='';$('effective-at').value=local(Date.now()/1000);$('part-state').value='unverified';$('edit-title').textContent=`${slot.location||'Custom'} · ${slot.name}`;$('revision-history').replaceChildren(el('h3','','Earlier revisions'));for(const old of state.book.revisions.filter(x=>x.id===slot.id).slice().reverse()){const e=el('article');e.append(el('b','',`${old.model||old.name} · ${old.revision||'unknown revision'} · ${old.state}`),el('small','',`Effective ${stamp(old.effective_at)} / entered ${stamp(old.recorded_at)}`),el('p','',old.reason));$('revision-history').append(e);}if(!$('part-dialog').open)$('part-dialog').showModal();}
$('part-form').addEventListener('submit',e=>{e.preventDefault();try{const v=Object.fromEntries(FIELDS.map(([key])=>[key,$('field-'+key).value]));v.state=$('part-state').value;v.effective_at=new Date($('effective-at').value).getTime()/1000;storeBook(appendRevision(state.book,v),editVersion);$('part-dialog').close();if($('detail-dialog').open&&state.detail)hud?.detail(state.detail);toast('Revision appended. Earlier records are preserved.');render();}catch(e){alert(e.message);}});
$('stl-fingerprint').addEventListener('change',async e=>{try{const file=e.target.files[0];if(!file)return;if(file.size>20*1024*1024)throw Error('Fingerprint input limit is 20 MiB. No file was uploaded.');const hash=await crypto.subtle.digest('SHA-256',await file.arrayBuffer());$('field-file_sha256').value=[...new Uint8Array(hash)].map(x=>x.toString(16).padStart(2,'0')).join('');$('fingerprint-result').textContent=`${file.name} · ${file.size.toLocaleString()} bytes · hash calculated locally; the STL bytes are not stored or uploaded.`;}catch(e){alert(e.message);}});
function diagnoses(){const list=diagnose(state.snapshot,$('symptom').value);$('diagnoses').replaceChildren();
 // Triage board (2026-09-18): glass triage cards — teal-bar title, orange evidence hairline,
 // mono source chips. Restyled via CSS scoped to #page-diagnostics; every part-select,
 // focus and openWork handler below is unchanged.
 if(!list.length){$('diagnoses').append(el('p','card empty','No matching recorded warning in the selected history. This is not a clean bill of health. Choose an operator-observed symptom to open a targeted inspection guide.'));return;}for(const r of list){const c=el('article','card diagnosis');c.append(el('span','eyebrow',r.origin==='operator'?'OPERATOR SYMPTOM · NOT AUTOMATICALLY DETECTED':'RECORDED EVIDENCE · CAUSE STILL UNCONFIRMED'),el('h2','',r.title),el('p','evidence',r.evidence+' Selected sample: '+stamp(r.ts)),el('p','why',r.why));for(const [where,what,how,why]of r.checks){const d=el('div','check'),left=el('div'),right=el('div');left.append(el('h3','',where));right.append(el('h3','',what),el('p','',how),el('p','',`Why: ${why}`));d.append(left,right);c.append(d);}const locations=el('div','button-row');for(const id of r.locations){const s=slotById(id);locations.append(tipped(btn(s?`${s.location}: ${s.name}`:id,()=>{select(id);route('machine');scene?.focus(id);}),s?`${s.location}: ${s.name}`:id,'Shows this location on the printer.'));}c.append(locations,tipped(btn('Create an open inspection entry',()=>openWork(r.locations[0],r.title+' — '+r.evidence)),'Create an open inspection entry','Opens the service journal form with this finding filled in. Nothing is sent to the printer.'));const src=el('div','sources');for(const key of r.sources)src.append(tipped(link('Reference: '+key,SOURCES[key]),'Reference: '+key,'Opens the reference in a new tab.'));c.append(src);$('diagnoses').append(c);}}
function openWork(asset=state.part,note=''){if(!state.book)return;$('work-asset').value=asset;$('work-note').value=note;$('work-state').value='open';$('work-evidence').value='';editVersion=state.book.version;$('work-dialog').showModal();}
$('work-form').addEventListener('submit',e=>{e.preventDefault();try{if(['checked','replaced'].includes($('work-state').value)&&!$('work-evidence').value.trim())throw Error('Record an observed outcome before marking work checked or replaced.');storeBook(journal(state.book,$('work-asset').value,$('work-state').value,$('work-note').value,$('work-evidence').value),editVersion);$('work-dialog').close();toast('Service entry appended. No printer command was sent.');maintenance();}catch(e){alert(e.message);}});
function maintenance(){$('work-list').replaceChildren();
 // Service ledger (2026-09-18): vertical glass rows — coloured state tag, mono 9px stamp,
 // quiet follow-up text button. Handlers unchanged. 'deferred' is a valid schema state the
 // brief does not colour; it gets the muted tag as the closest honest treatment.
 for(const w of state.book?.work?.slice().reverse()||[]){const c=el('article','card wl-row'),meta=el('div','wl-meta'),body=el('div','wl-body');
  meta.append(el('i','wl-tag wl-'+w.state,w.state.toUpperCase()),el('time','',stamp(w.at)));
  body.append(el('h3','',w.asset),el('p','',w.note),el('p','hint',w.evidence||'Outcome not recorded'));
  c.append(meta,body,tipped(btn('Append follow-up',()=>openWork(w.asset,'Follow-up to '+w.id+': '+w.note),'wl-follow'),'Append follow-up','Opens the journal form to add a follow-up to this entry.'));
  $('work-list').append(c);}if(!state.book?.work.length)$('work-list').append(el('p','card empty','No service work has been logged. Nothing is marked inspected or repaired automatically.'));}
function events(){const s=state.snapshot,q=$('event-search').value.toLowerCase(),all=s?.events||[];let shown=0;$('event-list').replaceChildren();
 // Message deck (2026-09-18): compact strip + two-column wall of mono glass chips. The kind
 // tag colours recorded evidence (resonance/shaper teal · warning kinds orange · hard errors
 // magenta · console/gcode kinds muted · job kinds green); unknown kinds fall back to muted.
 // Click-to-seek is unchanged.
  // Probe_retry (a BLTouch check Klipper is retrying) sits in the orange tier with probe_alarm,
  // never magenta: it is worth seeing, not a fault.
  const tag=k=>/resonance|shaper/.test(k)?'ev-teal':/error|shutdown|offline|lost/.test(k)?'ev-magenta':/warn|fail|degraded|gap|notice|alarm|retry/.test(k)?'ev-orange':/^job/.test(k)?'ev-green':'ev-mute';
 for(const e of all){if(q&&!`${e.kind} ${e.message}`.toLowerCase().includes(q))continue;shown++;
  const b=btn('',()=>seek(e.ts),'event td-chip');tipped(b,`${e.kind} · ${stamp(e.ts)}`,'Moves the shared recording cursor to this moment.');
  const head=el('div','td-head');head.append(el('i','td-kind '+tag(e.kind),e.kind),el('time','',stamp(e.ts)));
  b.append(head,el('span','td-msg',e.message));$('event-list').append(b);}
 if(!$('event-list').childNodes.length)$('event-list').append(el('p','empty','No matching messages at this time.'));
 $('event-coverage').textContent=s?.events_capped?'Event list capped. Reduce history window.':`${shown} of ${all.length} recorded messages at or before the shared cursor.`;}
const therm={built:false,nozzle:null,bed:null};
function thermal(){const s=state.snapshot,p=s?.point,t=telemetry();
 if(!therm.built){therm.built=true;$('thermal-cards').replaceChildren();
  const[nc,ncv,nsm]=dialCard('nozzle','NOZZLE SENSOR · °C','dial-canvas');therm.nozzle=createDial(ncv,{max:300,unit:'°C',label:'NOZZLE',accent:'--orange',targetColor:'--teal',digits:1});$('thermal-cards').append(nc);
  const[bc,bcv,bsm]=dialCard('bed','BED SENSOR · °C','dial-canvas');therm.bed=createDial(bcv,{max:120,unit:'°C',label:'BED',accent:'--orange',targetColor:'--teal',digits:1});$('thermal-cards').append(bc);}
 const nt=n(p?.nozzle?.temperature),nT=n(p?.nozzle?.target),bt=n(p?.bed?.temperature),bT=n(p?.bed?.target);
 therm.nozzle.set(nt,nT??null,dashStatus(nt,nT));const nsm=$('thermal-cards').querySelector('[data-field="nozzle"] small');if(nsm)nsm.textContent=`Target ${f(nT,1)} °C · output command ${f(n(p?.nozzle?.power)===null?null:p.nozzle.power*100,0)}%`;
 therm.bed.set(bt,bT??null,dashStatus(bt,bT));const bsm=$('thermal-cards').querySelector('[data-field="bed"] small');if(bsm)bsm.textContent=`Target ${f(bT,1)} °C · output command ${f(n(p?.bed?.power)===null?null:p.bed.power*100,0)}%`;
 plot($('thermal-chart'),[{name:'Nozzle',points:(s?.history||[]).map(x=>({ts:x.ts,value:x.nozzle?.temperature}))},{name:'Bed',points:(s?.history||[]).map(x=>({ts:x.ts,value:x.bed?.temperature}))}],s?.from??0,s?.cursor??1,'°C');$('fan-table').replaceChildren(table(['Fan object','Output command','Tachometer','Interpretation'],(t.fans||[]).map(x=>[x.object,n(x.command)===null?'Unknown':f(x.command*100,0)+'%',n(x.rpm)===null?'Not reported':f(x.rpm,0)+' rpm','Command is not proof of rotation.'])));$('sensor-table').replaceChildren(table(['Additional sensor','Temperature'],(t.sensors||[]).map(x=>[x.object,f(x.temperature,1)+' °C'])));}
function drawMesh(canvas,m,points){const r=canvas.getBoundingClientRect();if(!r.width){return;}const d=Math.min(devicePixelRatio||1,2);canvas.width=r.width*d;canvas.height=r.height*d;const c=canvas.getContext('2d');c.scale(d,d);c.font='12px "Geist Mono",monospace';c.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim();
 if(!m?.grid?.length){c.fillText('No recorded mesh. No flat mesh has been invented.',14,40);return;}
 const a=m.grid,w=a[0].length,h=a.length,size=Math.min((r.width-96)/w,(r.height-70)/h),vals=a.flat().map(x=>n(x)).filter(x=>x!==null),max=Math.max(.001,...vals.map(x=>Math.abs(x)));
 // Teal is nearest zero, orange is a real deviation, magenta-side red is the worst recorded point; sign stays visible as the printed value.
 const shade=z=>n(z)===null?'#23292b':`hsl(${172-150*Math.min(1,Math.abs(z)/max)} 88% ${22+28*Math.min(1,Math.abs(z)/max)}%)`;
 c.fillText(`${h}×${w} recorded points · front at bottom`,14,12);
 for(let x=0;x<w;x++)c.fillText(String(x),70+x*size+size*.4,26);
 a.forEach((row,y)=>{c.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim();c.fillText(String(h-1-y),52,22+(h-1-y)*size+size*.6);row.forEach((z,x)=>{c.fillStyle=shade(z);c.fillRect(70+x*size,30+(h-1-y)*size,size-1,size-1);if(size>30){c.fillStyle=n(z)===null?'#a6b5b6':'#fff';c.fillText(n(z)===null?'?':z.toFixed(2),70+x*size+2,30+(h-1-y)*size+size*.62);}});});
 for(let i=0;i<=40;i++){c.fillStyle=`hsl(${172-150*i/40} 88% ${22+28*i/40}%)`;c.fillRect(70+i*3.5,r.height-16,3.5,10);}
 c.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim();c.fillText(`near 0 → worst ±${max.toFixed(3)} mm`,70,r.height-22);
 // Live BLTouch landings (board item 3): dots at their bed coordinates, newest glowing mint,
 // each labelled with its recorded z. The redraw cadence is the mesh redraw itself.
 const pts=points||[];
 if(pts.length){
  const minx=m.min?.[0]??0,maxx=m.max?.[0]??350,miny=m.min?.[1]??0,maxy=m.max?.[1]??350;
  const px=x=>70+Math.max(0,Math.min(1,(x-minx)/Math.max(1e-6,maxx-minx)))*w*size;
  const py=y=>30+(h-1-Math.max(0,Math.min(1,(y-miny)/Math.max(1e-6,maxy-miny))))*size;
  pts.forEach((p,i)=>{const fresh=i>=Math.max(0,pts.length-4);const X=px(p.x),Y=py(p.y);
   c.beginPath();c.arc(X,Y,6,0,7);c.fillStyle=fresh?'#04D9C4':'#9fa9ae80';c.fill();
   if(fresh){c.beginPath();c.arc(X,Y,10,0,7);c.strokeStyle='#04D9C4';c.globalAlpha=.5;c.stroke();c.globalAlpha=1;}
   c.fillStyle='#e8fff9';c.fillText(n(p.z)===null?'?':p.z.toFixed(2),X+8,Y-6);});}
}
function buildProbeTable(points){const rows=(points||[]).slice(-12).reverse().map(p=>[stamp(p.ts),f(p.x,1),f(p.y,1),f(p.z,3),p.context]);$('probe-table').replaceChildren(table(['Time','X mm','Y mm','Z mm','Procedure'],rows.length?rows:[['No probe points recorded in this window yet','','','','']]));}
function meshSummary(m){if(!m?.grid?.length)return 'Mesh was not available in this sample.';const vals=m.grid.flat().map(x=>n(x)).filter(x=>x!==null),lo=vals.length?Math.min(...vals):null,hi=vals.length?Math.max(...vals):null,adaptive=/^adaptive/i.test(m.profile||'');return `${m.profile||'Unnamed profile'} · ${m.grid.length}×${m.grid[0].length} recorded points · min ${f(lo,3)} / max ${f(hi,3)} / range ${f(n(hi)===null||n(lo)===null?null:hi-lo,3)} mm${adaptive?' · adaptive mesh over the printed object, not the full bed':''} · recorded matrix values in mm.`;}
// Round-4 item 3c: rounds can sit outside every window — older than the 24 h trend
// span, or a failed trends fetch. The whole-recording /api/qgl fallback still names
// the last recorded round and jumps to it. Cached a minute, guarded against stale
// renders. Sits beside the 24 h drawTrends fallback, which wins when trends exist.
let lastQglAt=0,lastQglRound=null;
async function lastRecordedRound(){if(lastQglRound&&performance.now()-lastQglAt<60000)return lastQglRound;try{const r=await get('/api/qgl');lastQglRound=(r.rounds||[])[0]||null;lastQglAt=performance.now();}catch{lastQglRound=null;}return lastQglRound;}
function leveling(){const s=state.snapshot,m=s?.point?.mesh;drawMesh($('mesh'),m,s?.point?.probe_points);buildProbeTable(s?.point?.probe_points);$('mesh-note').textContent=meshSummary(m);$('round-table').replaceChildren(table(['Time / round','Range mm','FL / BL / BR / FR corrections'],(s?.rounds||[]).map(x=>[btn(`${n(x.ts)===null?'—':local(x.ts).slice(11,19)} · run ${x.run_id}/${x.round+1}`,()=>seek(x.ts),'link'),f(x.range_mm,4),[x.z0,x.z1,x.z2,x.z3].map(v=>f(v,4)).join(' / ')])));if(!(s?.rounds||[]).length)lastRecordedRound().then(r=>{if(r&&state.page==='leveling'&&!(state.snapshot?.rounds||[]).length&&!(state.trends?.qgl_runs||[]).length)$('round-table').replaceChildren(table(['Levelling rounds'],[[btn(`No rounds in this window · last recorded round ${stamp(r.ts)} (${f(r.range_mm,4)} mm range) — jump to it`,()=>seek(r.ts),'link')]]));});drawTrends();}
// 24-hour trend evidence: one point per distinct recorded mesh and per complete levelling run.
// Points are recorded facts only; gaps stay gaps and adaptive meshes are labelled, not blended.
let trendsSeq=0,trendsAt=0;
async function drawTrends(){const seq=++trendsSeq;let r=state.trends;
 if(!r||performance.now()-trendsAt>60000){try{r=await get('/api/trends?window=86400');if(seq!==trendsSeq)return;state.trends=r;trendsAt=performance.now();}catch(e){if(!r){$('mesh-trend-note').textContent='Trend data unavailable: '+e.message;$('qgl-trend-note').textContent='';return;}}}
 if(!r)return;const tol=state.snapshot?.geometry?.qgl_tolerance_mm??0.02;
 plot($('mesh-trend'),[{name:'Full-bed mesh range',points:(r.meshes||[]).filter(m=>!m.adaptive).map(m=>({ts:m.ts,value:m.range}))},{name:'Adaptive mesh range',points:(r.meshes||[]).filter(m=>m.adaptive).map(m=>({ts:m.ts,value:m.range}))}],r.from,r.to,'mm');
 $('mesh-trend-note').textContent=`${(r.meshes||[]).length} distinct recorded meshes in 24 h · repeated captures of an unchanged mesh are collapsed · adaptive meshes cover only the object area and are not comparable to full-bed.`;
 const runs=(r.qgl_runs||[]).filter(x=>x.final_range_mm!==null);
 plot($('qgl-trend'),[{name:'Final probed range per run',points:runs.map(x=>({ts:x.last_ts,value:x.final_range_mm}))},{name:'Tolerance',points:[{ts:r.from,value:tol},{ts:r.to,value:tol}]}],r.from,r.to,'mm');
 const warn=(r.warnings||[]).reduce((s2,w)=>s2+(w.count||0),0);
 $('qgl-trend-note').textContent=`${runs.length} complete levelling runs in 24 h · ${warn} driver warnings recorded in the same window · points are recorded evidence, gaps are not interpolated.`;
 if(!(state.snapshot?.rounds||[]).length&&(r.qgl_runs||[]).length){const last=r.qgl_runs[r.qgl_runs.length-1];$('round-table').replaceChildren(table(['Levelling rounds'],[[tipped(btn(`No rounds in this window · last recorded run ${stamp(last.last_ts)} — jump to it`,()=>seek(last.last_ts),'link'),'Last recorded levelling run','Moves the recording cursor to that run.')]]));}
 // Z health (board item 4): zero-reference series with drift, plus the recorded offset log.
 const zh=r.z_health||{};const zeros=(zh.zero_references||[]).slice(-12).reverse();
 const zRows=zeros.map((z,i)=>{const prev=zeros[i+1]?.z;return [stamp(z.ts),f(z.z,3),prev==null?'—':`${z.z-prev>=0?'+':''}${f(z.z-prev,3)} mm`];});
 $('z-health').replaceChildren(table(['Homed zero reference','Z mm','Δ vs previous'],zRows.length?zRows:[['No zero references in 24 h yet — homing Z with the BLTouch will record them','','']]));
 const off=(zh.offset_log||[]).slice(0,12).reverse();
 $('z-health').append(el('h3','','Z-offset change log'),table(['Time','Recorded console evidence'],off.length?off.map(o=>[stamp(o.ts),o.change]):[['No offset changes recorded in this window','']]));
 // Print context (board item 6): slicer metadata + Orca's tuned settings, plus the resonance note.
 const pcs=r.print_context||[];
 $('print-context').replaceChildren(table(['Print started','File','Slicer · layer','Flow / PA / nozzle'],pcs.length?pcs.map(pc=>{const m2=pc.metadata||{},o=pc.orca||{};const name=(pc.filename||'?').split('/').pop();return [stamp(pc.ts),name,`${m2.slicer??'unknown'} · LH ${f(m2.layer_height,2)} mm`,`${o.filament_flow_ratio??'—'} / ${o.pressure_advance??'—'} / ${o.nozzle_temperature??'—'} °C`];}):[['No prints with captured context in 24 h yet','','','']]));
 const secs=(r.resonance?.hours||[]).reduce((s2,h2)=>s2+(h2.approx_seconds||0),0);
 setText('resonance-note',`${pcs.length} print${pcs.length===1?'':'s'} with context in 24 h · ≈${Math.round(secs)} s recorded in the Y belt-resonance speed band (83–93 mm/s) — the hum in and out. Derived from recorded velocities only.`);}
function render(){const titles={assembly:'Assembly library',machine:'Printer reference',signals:'Signals',electronics:'Electronics',parts:'Parts & revisions',diagnostics:'Diagnostics',maintenance:'Service history',timeline:'Timeline',thermal:'Thermal',leveling:'Leveling'};$('page-title').textContent=titles[state.page];$('eyebrow').textContent=`PRINTER REFERENCE / ${state.page.toUpperCase()}`;for(const [id]of PAGES){$('page-'+id).hidden=id!==state.page;}document.querySelectorAll('#pages a').forEach(a=>a.classList.toggle('active',a.dataset.page===state.page));stats(); if(!state.live){$('activity-label').textContent='Rewound view';$('activity-detail').textContent='Live activity detection runs in Live mode from recorded console evidence.';$('freshness').textContent='rewind';}
 // While Live the tick loop owns the model pose; never reset it to the stale workspace sample.
 if(state.page==='assembly')showAssembly();paintLink();hud?.update();
 if(scene){const tp=state.live?lastTick?.point:null;
  scene.set(tp?.position?.some(v=>n(v)!==null)&&state.snapshot?liveSnapshot(tp.position,tp.ts):state.snapshot,state.part);}if(state.page==='machine')inspector();if(state.page==='signals'){signals();if(state.signal&&state.snapshot)showSignal(state.signal);}if(state.page==='electronics')electronics();if(state.page==='parts'){parts();catalogue();}if(state.page==='diagnostics')diagnoses();if(state.page==='maintenance')maintenance();if(state.page==='timeline')events();if(state.page==='thermal')thermal();if(state.page==='leveling')leveling();}
function showAssembly(){
 if(assembly){assembly.refresh();return;}if(assemblyLoading)return;assemblyLoading=true;
 import('./assembly.js').then(({mountAssembly})=>{assembly=mountAssembly($('page-assembly'),{
  openPart:id=>{select(id);route('machine');openDetail(id,true);},
  timeState:()=>state.snapshot?.point?`${state.live?'Latest recorded sample':'Recording cursor'} · ${stamp(state.snapshot.point.ts)} · ${$('freshness').textContent||'freshness unknown'} · source geometry is static`:'No usable recorded sample · source geometry is static',
  canTransport:id=>Boolean(state.snapshot?.point)&&!$(id).disabled,
  transport:id=>{if(id==='live'||state.snapshot)$(id).click();}
 });}).catch(e=>{assemblyLoading=false;error(e);});
}
for(const [id,name]of PAGES){const a=el('a','',name);a.href='#page='+id;a.dataset.page=id;a.addEventListener('click',e=>{e.preventDefault();route(id);});$('pages').append(a);}
try{scene=createScene($('machine-view'),id=>{select(id);openDetail(id);});}catch(e){error(e);}
// Camera navigation is mounted by the HUD view cube.
for(const [id,name]of [['frame','Frame'],['deck','Deck'],['covers','Panels'],['bed','Bed'],['gantry','Gantry'],['z-drives','Z drives'],['toolhead','Toolhead'],['electronics','Electronics']]){const b=btn(name,()=>{if(!scene)return;scene.hidden.has(id)?scene.hidden.delete(id):scene.hidden.add(id);b.classList.toggle('active',!scene.hidden.has(id));scene.schedule();},id==='covers'?'':'active');b.dataset.layer=id;$('layers').append(b);}
const transportAnchor=document.createComment('recording transport');document.querySelector('.transport').before(transportAnchor);document.addEventListener('fullscreenchange',()=>{const transport=document.querySelector('.transport');if(document.fullscreenElement?.classList.contains('cockpit'))document.fullscreenElement.append(transport);else transportAnchor.before(transport);});
$('explode').addEventListener('input',()=>{if(scene){scene.explode=Number($('explode').value);scene.schedule();}});$('focus-part').addEventListener('click',()=>scene?.focus(state.part));$('fullscreen').addEventListener('click',async()=>{try{if(document.fullscreenElement)await document.exitFullscreen();else await document.querySelector('.cockpit').requestFullscreen();}catch(e){error(e);}});
for(const b of document.querySelectorAll('[data-inspector]'))b.addEventListener('click',()=>{state.inspector=b.dataset.inspector;inspector();});$('open-parts').addEventListener('click',()=>{const slot=slotById(state.part);$('parts-search').value=slot?slot.location:partNames[state.part]||'';route('parts');});
for(const loc of [...new Set(SLOTS.map(s=>s.location))]){const o=el('option','',loc);o.value=loc;$('parts-location').append(o);}for(const g of [...new Set(SLOTS.map(s=>s.group))]){const o=el('option','',g);o.value=g;$('parts-group').append(o);}
for(const id of ['parts-search','parts-location','parts-group','parts-time'])$(id).addEventListener('input',parts);for(const id of ['signal-search','signal-kind'])$(id).addEventListener('input',signals);$('catalogue-search').addEventListener('input',catalogue);$('symptom').addEventListener('change',diagnoses);$('event-search').addEventListener('input',events);
$('custom-part').addEventListener('click',()=>editPart({id:'custom.'+crypto.randomUUID().slice(0,8),name:'New component',location:'Custom location'}));$('new-work').addEventListener('click',()=>openWork());
function download(name,data,type){const url=URL.createObjectURL(new Blob([data],{type})),a=el('a');a.href=url;a.download=name;document.body.append(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(url),500);}
$('export-book').addEventListener('click',()=>{if(state.book)download('Stella_Parts_Book_'+new Date().toISOString().slice(0,10)+'.json',JSON.stringify(state.book,null,2),'application/json');});$('import-book').addEventListener('click',()=>$('book-file').click());$('book-file').addEventListener('change',async e=>{try{const file=e.target.files[0];if(!file)return;if(file.size>2000000)throw Error('Import limit is 2 MB.');const incoming=validateBook(JSON.parse(await file.text()));if(!confirm(`Merge ${incoming.revisions.length} revision entries and ${incoming.work.length} service entries into this browser’s ${state.book?.machine}? Existing revisions are preserved.`))return;storeBook(mergeBook(state.book,incoming));toast('Parts history merged.');parts();}catch(e){error(e);}finally{$('book-file').value='';}});
for(const b of document.querySelectorAll('[data-close]'))b.addEventListener('click',()=>$(b.dataset.close).close());
$('live').addEventListener('click',()=>{pause();state.live=true;request(null);scheduleTickFrame();});$('prev').addEventListener('click',()=>seek(state.snapshot?.point?.ts??state.cursor,'previous'));$('next').addEventListener('click',()=>seek(state.snapshot?.point?.ts??state.cursor,'next'));$('cursor').addEventListener('input',()=>{pause();clearTimeout(sliderTimer);sliderTimer=setTimeout(()=>seek(Number($('cursor').value)),80);});$('window').addEventListener('change',()=>request());
$('play').addEventListener('click',()=>{if(state.playing){pause();return;}state.live=false;state.playing=true;lastPlay=performance.now();$('play').textContent='Pause';if(state.cursor>=state.snapshot.last_ts)request(state.snapshot.first_ts);});
// The full workspace snapshot (~147 KB) now loads on seek, page change, or every 5 s while Live;
// the 250 ms tick loop below keeps motion and readouts moving without rebuilding the page.
let lastFull=performance.now();
// While the service is down nothing is asked here; the pill is repainted so its "for how long" keeps counting.
setInterval(()=>{if(document.hidden||state.busy)return;if(svc.down){paintLink();return;}if(state.playing&&state.snapshot){const now=performance.now(),at=state.cursor+(now-lastPlay)/1000*Number($('speed').value);lastPlay=now;const last=state.snapshot.last_ts;if(at>=last)pause();request(Math.min(at,last));}else if(state.live&&performance.now()-lastFull>5000){lastFull=performance.now();request(null);}},500);
// Live tick: server-sent events push each stored sample the moment it lands (/api/tick/stream);
// the 250 ms /api/tick poll below is the fallback while the stream is down. Same payload either way.
let tickBusy=false,lastTick=null,lastSceneDraw=0;const tickBuffer=[];
// A pose the machine cannot be in never reaches the buffer, because interpolating between it and
// the last real one manufactures a whole ramp of poses that ARE inside the machine — that ramp is
// what drew the gantry up the frame during the G28 in their cube print (recorded Z 464.0195 against
// axis_maximum 310). The readouts still take the tick; only the drawn position is refused.
function takeTick(r){if(!r||r.schema!=='stella-scope.tick.v1')return;lastTick=r;
 if(insideTravel(r.point?.position,r.point?.limits)){tickBuffer.push(r);if(tickBuffer.length>12)tickBuffer.shift();}
 applyTick(r);}
// While the Stella Scope service does not answer, the page asks one probe at a time and waits 1 s,
// 2 s, 5 s, then 10 s between tries (the cap). Before, the 250 ms fallback poll and the stream's
// own reconnects asked about four times a second while it was down. The header says "service not
// running", not PRINTER OFF (the printer may be fine), the last view stays up, and the first answer
// resumes everything at once: the stream reopens and a full refresh follows within half a second.
function serviceLost(){svc.next=performance.now()+SERVICE_BACKOFF_S[Math.min(svc.fails,SERVICE_BACKOFF_S.length-1)]*1000;svc.fails++;if(svc.down)return;svc.down=true;svc.since=Date.now()/1000;svc.link=state.link;state.link={printer_state:'service',last_success:svc.since};sse?.close();sse=null;sseOk=false;render();}
function serviceBack(){if(!svc.down)return;svc.down=false;svc.fails=0;svc.next=0;state.link=svc.link;lastFull=0;openSse();render();}
let sse=null,sseOk=false;
// A dropped stream asks the service once: an answer means a hiccup and the browser reconnects the
// stream itself; no answer means the service is down, and the stream stays closed until it is back.
function openSse(){try{sse=new EventSource('/api/tick/stream');sse.onopen=()=>{sseOk=true;serviceBack();};sse.onmessage=e=>{try{takeTick(JSON.parse(e.data));}catch{}};
 sse.onerror=()=>{sseOk=false;if(!svc.down&&!tickBusy){tickBusy=true;get('/api/tick').catch(()=>{}).finally(()=>{tickBusy=false;});}};}catch{sseOk=false;}}
openSse();
setInterval(async()=>{if(document.hidden||tickBusy)return;if(svc.down?performance.now()<svc.next:(!state.live||sseOk))return;tickBusy=true;try{const r=await get('/api/tick');if(state.live)takeTick(r);}catch{/* keep the last readings; the header says what is wrong */}finally{tickBusy=false;}},250);
// Jitter-buffer playback: the display clock trails wall time by a fixed delay slightly larger than
// one tick interval, and position is interpolated between whichever two recorded samples bracket
// that clock. Monotonic — the head never jumps backward when a newer sample arrives; at most one
// interval behind; built only from recorded positions (labelled, never extrapolated or invented).
const TICK_DELAY_S=0.45;
// The display clock the interpolated pose belongs to. It is what set must be told, not the tick's
// timestamp: an interpolated position moves every frame while the tick's ts only moves four times a
// second, so the guard saw dt = 0 between frames, collapsed its reach to the 2 mm floor and refused
// every frame that advanced faster than 60 mm/s — then let the whole accumulated tick land in one
// frame. 80 refusals and 14 steps over 3 mm (worst 15.06 mm) in 22 s of the cube print, measured on
// the installed build. That release-after-hold is the jank.
let interpAt=0;
function interpolatedNow(){if(!lastTick?.point?.position)return null;const t=interpAt=Date.now()/1000-TICK_DELAY_S;let lo=null,hi=null;
 for(const s of tickBuffer){const ts=n(s?.point?.ts);if(ts===null||!s.point.position)continue;if(ts<=t){if(!lo||ts>lo.ts)lo={ts,position:s.point.position};}else if(!hi||ts<hi.ts)hi={ts,position:s.point.position};}
 if(!lo)return null; // display clock older than the whole buffer: freeze rather than guess or jump
 if(!hi)return [0,1,2].map(i=>n(lo.position[i])); // holding the newest recorded sample at or before the clock
 const span=Math.max(1e-6,hi.ts-lo.ts),alpha=Math.min(1,Math.max(0,(t-lo.ts)/span));
 return [0,1,2].map(i=>{const x=n(lo.position[i]),y=n(hi.position[i]);return x===null||y===null?y:x+(y-x)*alpha;});}
// One reused pose-snapshot object for the tick loop: per-frame spread allocation caused GC stalls.
let tickSnap=null,tickSnapSrc=null;
function liveSnapshot(position,ts){if(tickSnapSrc!==state.snapshot||!tickSnap){tickSnap={...state.snapshot,point:{...state.snapshot.point}};tickSnapSrc=state.snapshot;}
 tickSnap.point.position=position;tickSnap.point.ts=ts===undefined?tickSnap.point.ts:ts;
 // Everything the motion guard tests has to come from the tick, not from the snapshot: the full
 // snapshot refreshes on a 5 s timer (and not at all while the tab is hidden), and a G28 changes
 // homed_axes three times inside one of those windows. 's guard was reading a homed_axes and a
 // travel envelope up to 5 s old, which is why it held nothing during the homing.
 for(const k of ['homed','limits','max_velocity','max_z_velocity','velocity','commanded'])
  if(lastTick?.point&&k in lastTick.point)tickSnap.point[k]=lastTick.point[k];
 if(lastTick?.point?.probe_points)tickSnap.point.probe_points=lastTick.point.probe_points;else delete tickSnap.point.probe_points;
 return tickSnap;}
let tickFramePending=false;
function scheduleTickFrame(){if(!tickFramePending){tickFramePending=true;requestAnimationFrame(tickFrame);}}
function tickFrame(){tickFramePending=false;if(state.live&&!document.hidden){const pos=interpolatedNow();
  if(pos&&pos.some(x=>n(x)!==null)&&scene&&state.snapshot){const t=performance.now();if(t-lastSceneDraw>=32){lastSceneDraw=t;scene.set(liveSnapshot(pos,interpAt),state.part);const hm=lastTick?.homed_axes||'';setText('xyz',`X ${hm.includes('x')?f(pos[0],1):'—'} / Y ${hm.includes('y')?f(pos[1],1):'—'} / Z ${hm.includes('z')?f(pos[2],1):'—'} mm`);setText('pose-note',replay()?'Recorded position · static CAD reference · not encoder feedback':pos.some(x=>n(x)===null)?'Holding last reported axes · motion not reported while probing · Klipper requested position':hm&&hm.length<3?'Not all axes homed · position may be incomplete · Klipper requested position':'Smoothed between recorded samples · Klipper requested position · not encoder feedback');}}
  scheduleTickFrame();}else{lastSceneDraw=0;}}
scheduleTickFrame();
window.addEventListener('visibilitychange',()=>{if(!document.hidden)scheduleTickFrame();});
// The header's printer-state indicator. It reads the newest collector link the page holds
// (state.link: a tick's observer while samples flow, the snapshot's observer_now every 5 s when they
// stop), so it says PRINTER OFF within seconds of the test printer going dark, with how long ago her
// last data came, and never polls the printer itself. hud.js locks the controls from the same link.
function paintLink(){const led=$('led-transport');if(!led)return;const o=state.link,l=printerLink(o,{replay:replay(),lastSeen:o?.last_success??state.snapshot?.last_ts}),t=o?.transport;
 led.dataset.state=l.state==='ready'?(t==='push'?'push':t?'poll':'idle'):l.state;
 led.lastChild.textContent=l.state==='ready'?`READY · ${t?t.toUpperCase():'IDLE'}`:['off','service'].includes(l.state)?`${l.label}${l.since?' · '+l.since:''}`:l.label;
 led.dataset.tip=l.state==='service'?'Stella Scope service is not running':l.state==='off'?'Printer off or unreachable':l.state==='not_ready'?'Klipper not ready':l.state==='replay'?'Saved recording':'Printer link';
 led.dataset.tipDo=l.reason||(l.state==='ready'?`Moonraker answers and Klipper is ready; data arrives by ${t||'…'}.`:'Waiting for the recorder\'s first word about the printer.');led.removeAttribute('title');}
function applyTick(r){state.tick=r;if(r.observer)state.link=r.observer;
 setText('freshness',(r.age_seconds==null?'no sample':`${f(r.age_seconds,1)} s old`)+(r.observer?.transport?` · ${r.observer.transport}`:'')+(n(r.observer?.updates_per_s_1m)!==null&&r.observer.updates_per_s_1m>0?` · ${f(r.observer.updates_per_s_1m,1)}/s`:'')+(n(r.observer?.keepalive_ms)!==null?` · keepalive ${f(r.observer.keepalive_ms,0)} ms`:'')+(n(r.observer?.frame_age_s)!==null?` · frame ${f(r.observer.frame_age_s,1)} s`:''));
 // Header link LED: green pulse = push, orange = poll fallback, magenta = recorder link down.
 paintLink();
 setText('activity-label',replay()?'Recorded '+(r.activity?.label||'activity unknown'):r.activity?.label||'Activity unknown');
 setText('activity-detail',r.activity?`${r.activity.detail} · ${r.activity.basis}`:'');
 if(n(r.ts)!==null){$('cursor').max=Math.max(Number($('cursor').max)||0,r.ts);setText('last-time',stamp(r.ts));}
 if(state.live&&!replay()){setText('capture-status',!r.ts?'No recorded sample yet':r.age_seconds!=null&&r.age_seconds>2?`Live · data ${f(r.age_seconds,1)} s old`:'Live');setText('moment-label',r.ts?`Live${r.observer?.transport?` · ${r.observer.transport}`:''}`:'No recorded sample');}
 const p=r.point;if(p&&state.live){updateWall(p,r.job,r.observer,r.activity);hud?.update();
  if(!interpolatedNow()&&scene&&state.snapshot&&lastTick?.point?.position?.some(v=>n(v)!==null))scene.set(liveSnapshot(lastTick.point.position,lastTick.point.ts),state.part);}
 if($('detail-dialog').open&&state.detail)detailLive(state.detail);
 // Leveling page keeps its probe overlay and table live from the tick stream (board item 3).
 if(state.page==='leveling'&&(r.point?.probe_points||[]).length){drawMesh($('mesh'),state.snapshot?.point?.mesh,r.point.probe_points);buildProbeTable(r.point.probe_points);}}
// Click-to-detail overlay: works in fullscreen (dialog is top-layer), shows live tick readings for
// the picked part, its identity from the parts book, and — for the bed — the recorded mesh.
function detailSlot(id){id=scene?.entries.find(e=>e.id===id)?.slot_id||id;return slotById(id)||SLOTS.find(s=>s.id.split('.').pop()===id)||null;}
function detailLive(id){if(hud?.componentLive(id))return;id=scene?.entries.find(e=>e.id===id)?.slot_id||id;const slot=detailSlot(id),parent=slot?.parent||id,c=$('detail-live');c.replaceChildren();
 const p=(state.live?state.tick?.point:null)||state.snapshot?.point;
 if(!p){c.append(el('p','empty','No readings at this time.'));return;}
 if(p.motors?.[parent]){const m=p.motors[parent];kv(c,'Run current setting',m.current);kv(c,'Hold current setting',m.hold);kv(c,'Driver current scale',m.scale);kv(c,'Energised',m.energised);kv(c,'Standstill flag',m.standstill);kv(c,'Warning flags',m.flags===null?'Unknown':m.flags?.join(', ')||'No flags in known status');}
 if(parent==='hotend'){kv(c,'Temperature °C',p.nozzle?.temperature);kv(c,'Target °C',p.nozzle?.target);kv(c,'Heater output command %',n(p.nozzle?.power)===null?null:p.nozzle.power*100);}
 if(parent==='heater_bed'){kv(c,'Temperature °C',p.bed?.temperature);kv(c,'Target °C',p.bed?.target);kv(c,'Heater output command %',n(p.bed?.power)===null?null:p.bed.power*100);}
 if(['fan','hotend_fan','controller_fan'].includes(parent)){const row=(telemetry().fans||[]).find(x=>x.object===parent||signalPart(x.object)===parent);kv(c,'Output command %',row&&n(row.command)!==null?row.command*100:null);kv(c,'Tachometer',row?.rpm===undefined?'Not reported':row.rpm);}
 if(parent==='head'||parent==='gantry'||parent.startsWith('stepper_z')){const hm=(state.live?lastTick?.homed_axes:null)??state.snapshot?.point?.homed??'';kv(c,'X / Y / Z mm',(p.position||[]).map(x=>f(x,1)).join(' / '));kv(c,'Requested speed mm/s',p.velocity);kv(c,'Homed axes',hm||'none');if(!/[xyz]/.test(hm||''))c.append(el('p','hint','No axes homed; displayed position is not physical until homing completes.'));}
 if(parent==='probe'){kv(c,'Cached last query',telemetry().bltouch?.last_query??p.probe?.triggered);c.append(el('p','hint','Deployment state is not reported by this feed; no probe test is sent.'));}
 if(!c.childNodes.length)c.append(el('p','hint',state.live?'No mapped live telemetry for this item. Its identity and condition belong in Parts.':'Rewound view; live readings appear in Live mode.'));}
// Explicit is a request for the card (Details, the parts list). A click on a part with nothing wired
// to it opens the part chip instead of a card.
function openDetail(id,explicit=false){state.detail=id;if(scene?.entries.some(e=>e.id===id&&!e.sourceOnly))select(id);if(hud?.detail(id,{explicit})){detailLive(id);return;}const slot=detailSlot(id),parent=slot?.parent||id;
 $('detail-eyebrow').textContent=(slot?slot.location:'MODEL PART').toUpperCase();
 $('detail-title').textContent=slot?`${slot.name} — ${slot.location}`:partNames[id]||id;
 $('detail-basis').textContent=slot?slot.description:'Inventory location; source geometry mapping has not been established.';
 detailLive(id);
 const side=$('detail-side');side.replaceChildren();
 if(parent==='heater_bed'){side.append(el('h3','','Recorded bed mesh'));const cv=el('canvas','chart detail-mesh');const note=el('p','hint');side.append(cv,note);requestAnimationFrame(()=>{drawMesh(cv,state.snapshot?.point?.mesh);note.textContent=meshSummary(state.snapshot?.point?.mesh);});}
 else{const record=state.book?partsAt(state.book).get(slot?.id||id):null;side.append(el('h3','','Recorded identity'));
  if(record){const d=el('div');kv(d,'Model / maker',record.model||record.name);kv(d,'Revision',record.revision);kv(d,'State',record.state);kv(d,'Material',record.material);side.append(d);}
  else side.append(el('p','hint','No revision recorded for this location yet. Use Parts & history to identify it.'));}
 if(!$('detail-dialog').open)$('detail-dialog').showModal();}
$('detail-focus').addEventListener('click',()=>{$('detail-dialog').close();if(state.page!=='machine')route('machine');scene?.focus(state.detail);$('machine-view').scrollIntoView({block:'center',behavior:'smooth'});});
$('detail-parts').addEventListener('click',()=>{const slot=detailSlot(state.detail);$('parts-search').value=slot?slot.location:partNames[state.detail]||'';$('detail-dialog').close();route('parts');});
$('detail-work').addEventListener('click',()=>{const entry=scene?.entries.find(e=>e.id===state.detail),slot=detailSlot(state.detail);openWork(entry?instanceRecordId(entry.id):slot?.id||state.detail);});
$('copy-link').addEventListener('click',async()=>{try{updateUrl();await navigator.clipboard.writeText(location.href);toast('Moment link copied. The recording itself is not shared.');}catch{toast('Copy the browser address; this context blocks clipboard access.');}});
window.addEventListener('resize',()=>{scene?.schedule();if(state.page==='thermal')thermal();if(state.page==='leveling')leveling();});
hud=mountHUD({getState:()=>state,getScene:()=>scene,route,diagnose,partsAt,openDetail,editPart});document.addEventListener('stella-seek',e=>seek(e.detail?.ts));
(async()=>{try{const c=await get('/service-catalog.json');state.catalogue=c.files||[];}catch{state.catalogue=[];}await request();try{takeTick(await get('/api/tick'));}catch{}})();
