// The queue and the plate flow (farm board WO-08, 2026-10-12): the list of files waiting for a printer,
// "plate cleared" with its camera check, and the set-up of the empty-plate picture. A file only goes to
// a printer it fits; a start only happens after a passing check or an override the person confirmed.
// All of that is decided by the service; this file asks and shows the answer in words.
import {el,icon,button,busy,openSheet,openModal,section,segmented,toast,numberField,numberInput} from './next-ui.js';
import {key} from './next-control.js';
import {fileTitle,showForceModel} from './next-core.js';

const call=(method,url,body,token)=>fetch(url,{method,headers:{...(body!==undefined&&!(body instanceof Blob)?{'Content-Type':'application/json'}:{}),...(token?{'X-Stella-Control-Token':token}:{})},
 body:body===undefined?undefined:body instanceof Blob?body:JSON.stringify(body)}).then(async r=>{const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);return d;});
const farmToken=()=>fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).then(d=>d.token&&d.writable?d.token:null).catch(()=>null);
const printerToken=id=>fetch(`/api/p/${id}/controls`,{cache:'no-store'}).then(r=>r.json()).then(d=>d.enabled&&d.token?d.token:null).catch(()=>null);
const sentence=t=>{t=String(t||'').trim();return t?t[0].toUpperCase()+t.slice(1)+(/[.!?]$/.test(t)?'':'.'):t;};
const WORDS={queued:'Waiting',running:'Printing',done:'Done',stopped:'Stopped',cancelled:'Cancelled'};

// ---- the plate cleared flow ---------------------------------------------------------------------
// Returns when the person is done with it. `after` runs when something changed (a tile refresh).
export async function plateCleared(printer,{after}={}){
 const token=await printerToken(printer.id);
 if(!token){toast('This device can watch but not control. Use the computer or a paired phone.','alarm');return;}
 const send=override=>call('POST',`/api/p/${printer.id}/plate/cleared`,{override},token);
 const modal=openModal({title:`${printer.name}: plate`,icon:'i-check',build:(body,close)=>{
  const draw=r=>{body.replaceChildren();
   if(r.cleared){
    const c=r.check?.outcome;
    body.append(result('ok',c==='clear'?'The plate is clear.':r.overridden?'Released because you said the plate is clear.':'Released.',c==='skipped'?(r.check.reason||''):c==='clear'?'It matches the empty plate.':''));
    if(r.start?.started)body.append(el('p','nx-note',`Started ${r.start.title}.`));
    else if(r.start_error)body.append(el('p','nx-note',sentence(r.start_error)));
    else body.append(el('p','nx-note','The next file starts when you press Start on this printer.'));
    body.append(button({icon:'i-check',label:'Done',kind:'primary',size:'row',title:'Done',does:'Closes this.',onPress:()=>{close();after?.();}}));
    return;}
   const c=r.check||{};
   if(c.outcome==='failed'){body.append(result('alarm',"The camera could not be read.",sentence(c.reason)));}
   // finish-2 F02: the printer has moved since the empty plate was captured, so nothing was compared (not "something is on it")
   else if(c.outcome==='moved'){body.append(result('alarm','The plate could not be checked here.',sentence(c.reason)));}
   else body.append(result('alarm','Something is on the plate.',sentence(c.reason||'It does not match the empty plate.')));
   if(c.check_id){body.append(frameView(printer.id,c));}
   const row=el('div','nx-q-actions');
   row.append(button({icon:'i-play',label:'Check again',size:'row',title:'Check again',does:'Looks at the plate once more.',onPress:async b=>{if(b.dataset.busy!==undefined)return;busy(b,true);try{draw(await send(false));}catch(e){toast(e.message,'alarm');}busy(b,false);}}));
   const any=button({icon:'i-warn',label:'Start anyway',kind:'danger',size:'row',title:'Start anyway',does:'Releases the plate without a passing check. Only when you can see it is clear.',onPress:()=>{
    row.replaceChildren(el('p','nx-note','Release the plate without a passing check? You are saying it is clear.'),
     button({icon:'i-check',label:'Yes, it is clear',kind:'danger',size:'row',title:'Yes',does:'Records that you released it.',onPress:async b=>{if(b.dataset.busy!==undefined)return;busy(b,true);try{draw(await send(true));}catch(e){toast(e.message,'alarm');}busy(b,false);}}),
     button({icon:'i-back',label:'No',size:'row',title:'No',does:'Goes back.',onPress:()=>draw(r)}));}});
   if(r.can_override)row.append(any);
   body.append(row);};
  body.append(el('p','nx-note','Checking the plate…'));
  send(false).then(draw).catch(e=>{body.replaceChildren(result('alarm','Could not check the plate.',sentence(e.message)));});}});
 return modal;}

function result(tone,title,detail){const c=el('div','nx-fm-result');c.dataset.tone=tone;c.append(icon(tone==='ok'?'i-check':'i-warn'));
 const t=el('div','nx-fm-resulttext');t.append(el('b','',title));if(detail)t.append(el('small','',detail));c.append(t);return c;}

// The camera frame with the cells that differ drawn over it.
function frameView(printerId,c){
 const w=el('div','nx-q-frame'),img=el('img');img.alt='The plate as the camera saw it';img.src=`/api/p/${printerId}/plate/frame?id=${c.check_id}`;w.append(img);
 const [cols,rows]=c.grid||[48,27];
 for(const [cx,cy] of c.cells||[]){const d=el('i','nx-q-cell');d.style.left=(cx/cols*100)+'%';d.style.top=(cy/rows*100)+'%';d.style.width=(100/cols)+'%';d.style.height=(100/rows)+'%';w.append(d);}
 if(c.box){const b=el('i','nx-q-box');b.style.left=(c.box[0]*100)+'%';b.style.top=(c.box[1]*100)+'%';b.style.width=((c.box[2]-c.box[0])*100)+'%';b.style.height=((c.box[3]-c.box[1])*100)+'%';w.append(b);}
 return w;}

// ---- start the next file --------------------------------------------------------------------------
export async function startNext(printer,{job=null,after}={}){
 const token=await printerToken(printer.id);
 if(!token){toast('This device can watch but not control. Use the computer or a paired phone.','alarm');return;}
 const go=override=>call('POST',`/api/p/${printer.id}/queue/start`,{job,override},token);
 try{
  const r=await go(false);
  if(r.started){toast(`${printer.name}: started ${r.title}.`,'ok');after?.();return;}
  // The plate did not pass: the same screen as "plate cleared", and a confirmed override can start it.
  openModal({title:`${printer.name}: plate`,icon:'i-warn',build:(body,close)=>{
   const c=r.check||{};
   body.append(result('alarm',c.outcome==='failed'?'The camera could not be read.':'Something is on the plate.',sentence(c.reason)));
   if(c.check_id)body.append(frameView(printer.id,c));
   const row=el('div','nx-q-actions');
   const any=button({icon:'i-warn',label:'Start anyway',kind:'danger',size:'row',title:'Start anyway',does:'Sends the file without a passing check. Only when you can see the plate is clear.',onPress:()=>{
    row.replaceChildren(el('p','nx-note','Start without a passing check? You are saying the plate is clear.'),
     button({icon:'i-check',label:'Yes, start it',kind:'danger',size:'row',title:'Yes',does:'Sends the file now.',onPress:async b=>{if(b.dataset.busy!==undefined)return;busy(b,true);try{const x=await go(true);close();if(x.started){toast(`${printer.name}: started ${x.title}.`,'ok');after?.();}}catch(e){toast(e.message,'alarm');}busy(b,false);}}),
     button({icon:'i-back',label:'No',size:'row',title:'No',does:'Leaves it.',onPress:close}));}});
   row.append(button({icon:'i-back',label:'Close',size:'row',title:'Close',does:'Leaves the plate as it is.',onPress:close}),any);
   body.append(row);}});
 }catch(e){toast(sentence(e.message),'alarm');}}

// ---- setting up the empty-plate picture -----------------------------------------------------------
export async function plateSetup(printer,{after}={}){
 const token=await printerToken(printer.id);
 if(!token){toast('Plate set-up is done from the computer or a paired phone.','alarm');return;}
 const status=await fetch(`/api/p/${printer.id}/plate`,{cache:'no-store'}).then(r=>r.json()).catch(()=>({}));
 openModal({title:`${printer.name}: empty plate`,icon:'i-camera',build:(body,close)=>{
  const stage=el('div','nx-q-stage');body.append(stage);
  let region=[[.06,.08],[.94,.08],[.94,.92],[.06,.92]],sens=Number(status.sensitivity||1);
  if(Array.isArray(status.region)&&status.region.length===4)region=status.region.map(p=>[p[0],p[1]]);
  const live=()=>{
   stage.replaceChildren(el('p','nx-note','Take everything off the plate and look at the picture: nothing should be on it. Then capture it. Stella Scope compares later pictures with this one.'));
   const img=el('img','nx-q-live');img.alt='The camera, live';const tick=()=>{if(!img.isConnected)return;img.src=`/api/p/${printer.id}/camera/snapshot?t=${Date.now()}`;};
   img.addEventListener('load',()=>setTimeout(tick,900));img.addEventListener('error',()=>{img.replaceWith(el('p','nx-note','This printer has no camera answering. Add its camera address in its settings first.'));});
   // F06 (2026-10-19): the picture was asked for before the image was on the page (`tick` stops while the image is not attached), so the first set-up of a plate check showed a black box with
   // the words "The camera, live" and never a picture; the picture is asked for once it is there.
   stage.append(img,button({icon:'i-camera',label:'Capture the empty plate',kind:'primary',size:'row',title:'Capture',does:'Saves this picture as the empty plate.',onPress:async b=>{
    if(b.dataset.busy!==undefined)return;busy(b,true);try{await call('POST',`/api/p/${printer.id}/plate/reference`,{},token);mark();}catch(e){toast(sentence(e.message),'alarm');}busy(b,false);}}));tick();};
  const mark=()=>{
   stage.replaceChildren(el('p','nx-note','Drag the four corners onto the plate so only the plate is compared. The bed clips and cables should be outside.'));
   const box=el('div','nx-q-mark'),img=el('img');img.alt='The empty plate';img.src=`/api/p/${printer.id}/plate/reference.jpg?t=${Date.now()}`;
   const NS='http://www.w3.org/2000/svg',svg=document.createElementNS(NS,'svg');svg.setAttribute('viewBox','0 0 100 100');svg.setAttribute('preserveAspectRatio','none');
   const poly=document.createElementNS(NS,'polygon');svg.append(poly);
   const handles=region.map((_,i)=>{const h=el('button','nx-q-handle');h.type='button';h.setAttribute('aria-label',`Corner ${i+1}`);
    h.addEventListener('pointerdown',e=>{e.preventDefault();h.setPointerCapture(e.pointerId);
     const move=ev=>{const r=box.getBoundingClientRect();region[i]=[Math.min(1,Math.max(0,(ev.clientX-r.left)/r.width)),Math.min(1,Math.max(0,(ev.clientY-r.top)/r.height))];paint();};
     const up=()=>{h.removeEventListener('pointermove',move);};h.addEventListener('pointermove',move);h.addEventListener('pointerup',up,{once:true});h.addEventListener('pointercancel',up,{once:true});});
    return h;});
   const paint=()=>{poly.setAttribute('points',region.map(p=>`${p[0]*100},${p[1]*100}`).join(' '));handles.forEach((h,i)=>{h.style.left=region[i][0]*100+'%';h.style.top=region[i][1]*100+'%';});};
   box.append(img,svg,...handles);paint();
   const seg=segmented(['low','normal','high'],sens<.85?'low':sens>1.15?'high':'normal',v=>{sens={low:.7,normal:1,high:1.4}[v];},v=>({low:'Notices less',normal:'Normal',high:'Notices more'}[v]));
   const save=button({icon:'i-check',label:'Save the plate area',kind:'primary',size:'row',title:'Save',does:'Keeps the four corners and how closely it looks.',onPress:async b=>{
    if(b.dataset.busy!==undefined)return;busy(b,true);
    try{await call('PATCH',`/api/p/${printer.id}/plate`,{region,sensitivity:sens},token);toast('The plate check is set up.','ok');close();after?.();}catch(e){toast(sentence(e.message),'alarm');}busy(b,false);}});
   stage.append(box,section('How closely it looks',seg),save,
    button({icon:'i-camera',label:'Capture again',size:'row',title:'Capture again',does:'Replaces the picture with a new one.',onPress:live}),
    button({icon:'i-close',label:'Remove the plate check',kind:'danger',size:'row',title:'Remove',does:'Deletes the picture; starts then skip the check.',onPress:async()=>{try{await call('PATCH',`/api/p/${printer.id}/plate`,{forget:true},token);toast('Removed.','ok');close();after?.();}catch(e){toast(sentence(e.message),'alarm');}}}));};
  if(status.has_reference)mark();else live();}});}

// ---- the queue sheet ------------------------------------------------------------------------------
export function sheetQueue(){return openSheet({title:'Queue',icon:'i-print',wide:true,build:body=>drawQueue(body)});}

async function drawQueue(body){
 let d;
 try{d=await fetch('/api/queue',{cache:'no-store'}).then(async r=>{const j=await r.json();if(!r.ok)throw Error(j.error);return j;});}
 catch(e){body.replaceChildren(el('p','nx-note',sentence(e.message)));return;}
 const token=await farmToken();
 const keepScroll=body.scrollTop;body.replaceChildren();
 // add
 const bar=el('div','nx-q-add');
 if(token){
  const file=el('input');file.type='file';file.accept='.gcode,.bgcode,.gco,.3mf';file.hidden=true;file.multiple=true;
  const add=button({icon:'i-plus',label:'Add files',kind:'primary',size:'row',title:'Add files',does:'Puts sliced files in the queue. Each file is read for the printer, nozzle and material it was sliced for.',onPress:()=>file.click()});
  file.addEventListener('change',async()=>{
   for(const f of [...file.files]){
    busy(add,true);
    try{await call('POST',`/api/queue/upload?name=${encodeURIComponent(f.name)}&copies=1`,f,token);toast(`${f.name} is in the queue.`,'ok');}
    catch(e){toast(`${f.name}: ${sentence(e.message)}`,'alarm');}
    busy(add,false);}
   drawQueue(body);});
  bar.append(add,file);
  const cur=d.printers.filter(p=>p.state!=='offline');
  if(cur.length)bar.append(button({icon:'i-files',label:'From a printer',size:'row',title:'A file on a printer',does:'Queues a file that is already on a printer. It can only run there.',onPress:()=>fromPrinter(d.printers,token,()=>drawQueue(body))}));
 }else bar.append(el('p','nx-note','Files are added and reordered from the Stella Scope window on the computer.'));
 body.append(bar);
 // printers: what each is waiting for
 const pr=el('div','nx-q-printers');
 for(const p of d.printers){
  const row=el('div','nx-q-printer');row.dataset.state=p.state;
  const nm=el('b','',p.name),st=el('small','',stateLine(p,d.jobs));row.append(nm,st);
  pr.append(row);}
 const list=el('div','nx-q-list');
 const order=()=>[...list.children].map(c=>Number(c.dataset.id));
 const save=async()=>{try{await call('POST','/api/queue/reorder',{order:order()},token);}catch(e){toast(sentence(e.message),'alarm');drawQueue(body);}};
 for(const j of d.jobs)list.append(jobCard(j,d,token,body,save));
 // F06 (2026-10-19): the jobs come first. With a farm of ten printers the list of printers filled the whole sheet and the file a person had just added was below it, out of sight.
 if(d.jobs.length)body.append(section('Jobs',list));
 body.append(section('Printers',pr));
 if(!d.jobs.length)body.append(el('p','nx-note','Nothing is waiting. Add a sliced file and it goes to the first printer that fits it, once the plate is clear.'));
 body.scrollTop=keepScroll;}

function stateLine(p,jobs){
 const next=jobs.find(j=>j.id===p.next_job);
 const parts=[({idle:'Ready',finished:'Done',printing:'Printing',paused:'Paused',heating:'Heating',error:'Needs attention',offline:'Offline'})[p.state]||p.state];
 if(p.awaiting_clear)parts.push('plate needs clearing');
 if(!p.has_reference)parts.push('no plate check set up');
 if(next&&['idle','finished'].includes(p.state))parts.push(`next: ${next.title}`);
 return parts.join(' · ');}

function chips(j){
 const n=j.needs,out=[];
 if(n.slicer)out.push(n.slicer);
 out.push(n.printer_model||'printer not stated');
 if(n.nozzle_mm)out.push(n.nozzle_mm+' mm nozzle');
 if(n.filament?.length)out.push([...new Set(n.filament)].join(' + '));
 if(n.extents_mm)out.push(`${n.extents_mm.map(v=>Math.round(v)).join(' x ')} mm`);
 else if(n.bed_mm)out.push(`bed ${n.bed_mm.map(Math.round).join(' x ')}`);
 return out;}

function jobCard(j,d,token,body,onReorder){
 const card=el('article','nx-q-job nx-glass');card.dataset.id=j.id;card.dataset.state=j.state;
 const head=el('div','nx-q-head'),grip=el('span','nx-q-grip');grip.setAttribute('aria-hidden','true');grip.append(icon('i-more'));
 const title=el('b','nx-q-title',j.title),chip=el('span','fm-chip',WORDS[j.state]||j.state);chip.dataset.tone={running:'heat',done:'ok',stopped:'alarm',cancelled:'off'}[j.state]||'off';
 head.append(grip,title,chip);card.append(head);
 const meta=el('div','nx-q-chips');for(const c of chips(j))meta.append(el('span','nx-q-chip',c));card.append(meta);
 if(j.note)card.append(el('p','nx-note',j.note));
 // where it can go
 const fits=el('div','nx-q-fits');
 for(const f of j.fits){const r=el('div','nx-q-fit');r.dataset.ok=String(f.ok);
  r.append(icon(f.ok?'i-check':'i-minus'),el('span','',f.name));
  const why=[...f.reasons,...(f.ok?f.notes:[])];if(why.length)r.append(el('small','',why.join(' ')));fits.append(r);}
 card.append(fits);
 if(j.fits.some(f=>f.reasons.some(r=>/different machine/.test(r))))card.append(el('p','nx-note','A file is sliced for one printer model. Send it to another only if you know it fits.'));
 if(token&&['queued','stopped'].includes(j.state)){
  const row=el('div','nx-q-controls');
  // U05: the number of copies is the kit's gauge (a copy a click, sent once when the hand lets go), not a minus and a plus
  const copies=el('span','nx-q-copies');
  const nf=numberInput({label:'Copies',min:1,max:99,step:1,turn:400,value:j.copies,color:'#04D9C4',color2:'#7FFFF0',size:72,onSet:v=>patch(j.id,{copies:v},token,body)});
  copies.append(nf,el('span','',`${j.done} of ${j.copies} done`));
  const sel=el('select','nx-input');sel.setAttribute('aria-label','Send to');
  sel.append(Object.assign(el('option','','Any printer that fits'),{value:''}));
  const fixed=j.source==='printer';
  for(const p of d.printers)sel.append(Object.assign(el('option','',p.name),{value:p.id}));
  sel.value=j.target||'';sel.disabled=fixed;sel.addEventListener('change',()=>patch(j.id,{target:sel.value||null},token,body));
  const prio=segmented(['low','normal','high'],j.priority>0?'high':j.priority<0?'low':'normal',v=>patch(j.id,{priority:{low:-1,normal:0,high:1}[v]},token,body),v=>({low:'Later',normal:'Normal',high:'First'}[v]));
  row.append(copies,sel,prio);
  const force=el('div','nx-q-force'),fk=key({icon:'check',toggle:true,pressed:!!j.force_model,onPress:on=>patch(j.id,{force_model:on},token,body)});fk.el.setAttribute('aria-label','Send to a different printer model anyway');   // a yes or a no is a latched key (U05)
  force.append(fk.el,el('span','','Send to a different printer model anyway (the file was sliced for another one)'));force.addEventListener('click',ev=>{if(!fk.el.contains(ev.target))fk.el.click();});
  // F06 (2026-10-19, the F02 open item): the choice was offered on a file that fits (its printer ticked, nothing held back), where it means nothing and
  // reads as if something were wrong. It shows when a printer is held back for being another machine, or when it is already ticked (so it can be undone).
  card.append(row);if(showForceModel(j))card.append(force);
  const acts=el('div','nx-q-actions');
  if(j.state==='stopped')acts.append(button({icon:'i-play',label:'Put back in the queue',size:'row',title:'Put back',does:'Makes this job wait again.',onPress:()=>patch(j.id,{state:'queued'},token,body)}));
  acts.append(
   button({icon:'i-arrow-up',size:'icon',kind:'ghost',title:'Move up',does:'Earlier in the queue.',onPress:()=>{const p=card.previousElementSibling;if(p){p.before(card);onReorder();}}}),
   button({icon:'i-arrow-down',size:'icon',kind:'ghost',title:'Move down',does:'Later in the queue.',onPress:()=>{const n=card.nextElementSibling;if(n){n.after(card);onReorder();}}}),
   button({icon:'i-close',label:'Remove',kind:'danger',size:'row',title:'Remove',does:'Takes it out of the queue and deletes its stored copy.',onPress:async()=>{try{await call('DELETE',`/api/queue/jobs/${j.id}`,undefined,token);drawQueue(body);}catch(e){toast(sentence(e.message),'alarm');}}}));
  card.append(acts);
  // drag by the grip
  grip.addEventListener('pointerdown',e=>{e.preventDefault();grip.setPointerCapture(e.pointerId);card.dataset.drag='';
   const move=ev=>{const t=document.elementFromPoint(ev.clientX,ev.clientY)?.closest('.nx-q-job');if(t&&t!==card&&t.parentElement===card.parentElement){const r=t.getBoundingClientRect();(ev.clientY<r.top+r.height/2?t.before(card):t.after(card));}};
   const up=()=>{grip.removeEventListener('pointermove',move);delete card.dataset.drag;onReorder();};
   grip.addEventListener('pointermove',move);grip.addEventListener('pointerup',up,{once:true});grip.addEventListener('pointercancel',up,{once:true});});
 }
 return card;}

async function patch(id,p,token,body){try{await call('PATCH',`/api/queue/jobs/${id}`,p,token);drawQueue(body);}catch(e){toast(sentence(e.message),'alarm');}}

async function fromPrinter(printers,token,done){
 openModal({title:'A file on a printer',icon:'i-files',build:(body,close)=>{
  const sel=el('select','nx-input');sel.setAttribute('aria-label','Printer');
  for(const p of printers.filter(p=>p.state!=='offline'))sel.append(Object.assign(el('option','',p.name),{value:p.id}));
  const list=el('div','nx-q-list');
  const load=async()=>{list.replaceChildren(el('p','nx-note','Loading files…'));
   try{const f=await fetch(`/api/p/${sel.value}/files`,{cache:'no-store'}).then(r=>r.json());
    const rows=(f.files||f.result||[]).map(x=>x.path||x.name||x.filename).filter(Boolean);list.replaceChildren();
    if(!rows.length){list.append(el('p','nx-note','No files on this printer.'));return;}
    for(const name of rows.slice(0,60)){const b=el('button','nx-item');b.type='button';const t=el('div','nx-item-text');t.append(el('div','nx-item-title',fileTitle(name)),el('div','nx-item-sub',name));b.append(icon('i-files'),t);
     b.addEventListener('click',async()=>{try{await call('POST','/api/queue/jobs',{printer:sel.value,file:name,copies:1},token);toast('Queued.','ok');close();done();}catch(e){toast(sentence(e.message),'alarm');}});list.append(b);}
   }catch(e){list.replaceChildren(el('p','nx-note',sentence(e.message)));}};
  sel.addEventListener('change',load);body.append(sel,list);load();}});}
