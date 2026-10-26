// "Get the 3D models" (polish board WO-03, 2026-10-18). A fresh install that tries the demo farm has the printers but not their
// CAD: the models are separate downloads under their authors' open licences. After the press this offers one plain button
// that fetches every open model the farm draws, from the address its author publishes it at, each checked against its
// published SHA-256 by the service; the licences are shown first, and the wall draws each machine as it lands.
// The service does the work (src/printer_models.rs `start_fetch`); this watches it and says what is going on.
// F03 (2026-10-19): the demo farm's own press fetches the models too (no second button), so each tile of the wall says what is
// happening to its model: waiting for its turn, arriving with its own percentage, failed (offline, with Try again) or not asked for
// yet. `modelStatus` is that sentence for one printer, from the service's answer; `tileModelPanel` draws it on a tile.
// F09 (2026-10-19): the models Prusa shares under its own licence ship inside the app (a fresh install has them at the first start), so no tile or banner asks
// for a notice to be read before a download any more; `showNotice` is where the maker's notice and the licence text are read (the model's sheet and About).
import {el,button,busy,toast,openModal,openSheet,section} from './next-ui.js';

const mb=b=>b>=1e9?(b/1e9).toFixed(1)+' GB':Math.max(1,Math.round(b/1e6))+' MB';
let box=null,token=null,watching=0,landed=0,runningBefore=false;
const WANT='/api/printer-models/wanted';

/** Look at what the farm wants and draw the banner for it. Call it whenever the wall refreshes; it asks the service itself. */
/** The same fetch from a printer page that is missing files of the open set (the toolchanger's tools): the licences first, then
 *  the fetch, and `onDone` when it is over so the page draws the model again with the files. `w` is the service's `wanted` answer. */
export function modelsButton(tok,onDone,w){
 const open=w.models.filter(m=>m.open),bytes=open.filter(m=>m.kind==='part').reduce((n,m)=>n+(m.bytes||0),0);
 return button({icon:'i-view',label:`Get the 3D models (${mb(bytes)}, open licence)`,kind:'primary',size:'row',title:'Get the 3D models',does:'Shows the licences first, then downloads the missing files from their authors\' own sites. Each file is checked before it is kept.',onPress:()=>{token=tok;licences(w,onDone);}});
}

export async function modelsBanner(farmToken){
 token=farmToken||null;
 let w;try{w=await (await fetch(WANT,{cache:'no-store'})).json();}catch{return;}
 draw(w);
 // While a fetch runs, look at it closely, and again as soon as it is over.
 if(w.fetch?.state==='running'&&!watching)watching=setInterval(async()=>{try{draw(await (await fetch(WANT,{cache:'no-store'})).json());}catch{}},700);
 if(w.fetch?.state!=='running'&&watching){clearInterval(watching);watching=0;}
}

function host(){
 if(box?.isConnected)return box;
 box=el('div','fm-licence nx-glass fm-models');box.id='fm-models';box.setAttribute('role','note');
 const after=document.getElementById('fm-licence')||document.getElementById('fm-banner');
 after?.after(box);return box;
}

function draw(w){
 // Every tile reads its own model's state from this answer (farm.js), before anything else is done with it.
 document.dispatchEvent(new CustomEvent('stella-model-status',{detail:w}));
 const f=w.fetch||{},running=f.state==='running',open=w.models.filter(m=>m.open),rest=w.models.filter(m=>!m.open);
 // Tell the wall about every model that has arrived since it last looked, so each tile draws its machine at once.
 if((f.finished||[]).length>landed){landed=f.finished.length;document.dispatchEvent(new CustomEvent('stella-models',{detail:{ids:f.finished}}));}
 if(runningBefore&&!running&&!(f.failed||[]).length&&(f.finished||[]).length)toast(`The 3D models are here: ${f.finished.length} machines.`,'ok');
 runningBefore=running;
 const b=host();
 if(!running&&!open.length&&!rest.length){b.hidden=true;b.replaceChildren();b.dataset.key='';return;}
 const key=JSON.stringify([running?Math.floor((f.done_bytes||0)/1e6):0,running?f.current?.id:'',open.map(m=>m.id),rest.map(m=>m.id),(f.failed||[]).map(x=>x.id),!!token]);
 if(b.dataset.key===key)return;b.dataset.key=key;b.hidden=false;b.replaceChildren();
 if(running){
  const p=f.total_bytes?Math.min(1,f.done_bytes/f.total_bytes):0,now=String(f.current?.label||'the next one').split(' · ')[0];
  b.append(el('p','fm-licence-text',`Getting the 3D models: ${now}. ${Math.min(f.count,(f.finished||[]).length+(f.failed||[]).length+1)} of ${f.count}, ${mb(f.done_bytes)} of ${mb(f.total_bytes)}. Each machine draws as it arrives.${typeof f.elapsed_s==='number'?` (${Math.round(f.elapsed_s)} s so far)`:''}`));
  const bar=el('div','nx-gm-bar');bar.style.setProperty('--p',p.toFixed(3));bar.setAttribute('role','progressbar');bar.setAttribute('aria-valuenow',String(Math.round(p*100)));bar.setAttribute('aria-label','Getting the 3D models');b.append(bar);
  return;
 }
 const lines=[],mo=open.filter(m=>m.kind!=='part'),po=open.filter(m=>m.kind==='part');
 if(mo.length)lines.push(`${mo.length===1?'One machine':mo.length+' machines'} here ${mo.length===1?'has':'have'} no 3D model yet.`);
 if(po.length)lines.push(`The files of their toolchangers and add-ons are not here either (${po.length} files).`);
 if(open.length)lines.push(`${open.length===1?'It is':'They are'} free to get from ${open.length===1?'its author':'their authors'}, under open licences.`);
 if((f.failed||[]).length)lines.push(failedLine(f));
 for(const m of rest)lines.push(m.why||`${m.label} is added under Printer models.`);
 b.append(el('p','fm-licence-text',lines.join(' ')));
 if(open.length){
  const row=el('div','fm-licence-act');
  row.append(button({icon:'i-view',label:`Get the 3D models (${mb(w.open_bytes)}, open licence)`,kind:'primary',size:'row',title:'Get the 3D models',does:'Shows the licences first, then downloads the models from their authors\' own sites. Each file is checked before it is kept.',onPress:()=>licences(w)}));
  b.append(row);
 }
}
const short=m=>String(m.label||m.id).split(' / ')[0];
// F03 (2026-10-19): what went wrong, once, in the service's plain words (the same sentence a tile says), never the raw error with its address.
function failedLine(f){
 const bad=(f.items||[]).filter(i=>i.state==='failed'),n=bad.length||f.failed.length,plains=[...new Set(bad.map(i=>i.failure?.plain).filter(Boolean))];
 return `${n===1?'One file could':n+' files could'} not be downloaded. ${(plains.length?plains:['Try again.']).join(' ')}`;}

/** Who made a model the build carries, the terms it is under and what was done to the file, then the licence text, from the service
 *  (`GET /api/printer-models/<id>/notice`). Reading only: nothing is fetched and nothing is changed. `m` is a catalogue row or a credit
 *  ({id,label}); about the same words are in About and in the model's own sheet. */
export function showNotice(m){
 openModal({title:`${short(m)}: who made it, and the terms`,icon:'i-view',build:(body)=>{
  body.append(el('p','nx-note',`This model ships inside Stella Scope. Below are its maker, the licence it is under, what was done to the file and the licence text. Nothing here is downloaded.`));
  const pre=el('pre','nx-gm-notice');pre.tabIndex=0;pre.setAttribute('aria-label',`The notice for ${short(m)}`);pre.textContent='Reading the notice…';body.append(pre);
  fetch(`/api/printer-models/${encodeURIComponent(m.id)}/notice`).then(r=>r.json()).then(n=>{if(!n.text)throw Error(n.error||'no notice');pre.textContent=n.text;}).catch(e=>{pre.textContent='The notice could not be read: '+e.message;});
 }});
}

const BADGE={gpl:'Open · GPL','cc-by':'Open · CC BY','cc-by-sa':'Open · CC BY-SA',cc0:'Open · CC0',mit:'Open · MIT',ocl:'Open · OCL (Prusa)'};
/** About (finish-2 F09): the version, what Stella Scope is under, and who is credited for every 3D model the build carries, each with its notice.
 *  The credits come from the service (`GET /api/printer-models/credits`), built from the catalogue: nothing here is typed. */
// A row of buttons in About: [icon, label, what it does, go].
function aboutLinks(list){const row=el('div','nx-grid2');for(const [ico,label,does,go] of list)row.append(button({icon:ico,label,size:'row',title:label,does,onPress:go}));return row;}
function rigNotice(rig){openModal({title:`${rig.name}: ${rig.licence} notice`,icon:'i-view',build:body=>{const pre=el('pre','nx-gm-notice');pre.tabIndex=0;pre.setAttribute('aria-label',`The ${rig.licence} notice of ${rig.name}`);pre.textContent=rig.notice.replace(/([^\n])\n(?!\n)/g,'$1 ');body.append(el('p','nx-note',`${rig.name} by ${rig.by}, ${rig.source}`),pre);}});}
export function sheetAbout(){
 return openSheet({title:'About Stella Scope',icon:'i-view',build:async body=>{
  body.replaceChildren(el('p','nx-note','Reading…'));
  let d;try{d=await (await fetch('/api/printer-models/credits',{cache:'no-store'})).json();}catch{body.replaceChildren(el('p','nx-note','Stella Scope did not answer.'));return;}
  const list=el('div','nx-gm-list');
  for(const m of d.models||[]){
   const r=el('div','nx-gm-row nx-gm-row--about'),sp=el('span','nx-md-badge',BADGE[m.class]||m.class);sp.dataset.class=m.class;
   const a=el('a','nx-md-link',m.source);a.href=m.source;a.target='_blank';a.rel='noopener noreferrer';
   r.append(el('b','',m.label),sp,el('small','',`By ${m.author} · ${mb(m.bytes||0)}`),el('small','',m.licence),a,button({icon:'i-view',label:'Who made it, and the terms',size:'row',title:`Who made it, and the terms: ${m.label}`,does:'The maker, the licence and what was done to the file, with the licence text.',onPress:()=>showNotice(m)}));
   list.append(r);}
  body.replaceChildren(
   // F07 (2026-10-19): the version, the licence, how a bigger farm buys one (the owner: no e-Transfer address is shown publicly; the payment details come with the invoice), and who to write to
   section('Stella Scope',el('p','nx-note',`Version ${d.version}. Free and open source under the ${d.licence} licence, made by ${d.maker}. Free for up to five printers. A perpetual licence for more is bought by Interac e-Transfer, and the payment details come with the invoice. To buy, or with any question, write to ${d.contact}.`),aboutLinks([['i-help','Help','Opens the user guide.',()=>{location.href='/help';}],['i-link','Write to '+d.maker,'Opens a new email to '+d.contact+' in your mail program.',()=>{location.href='mailto:'+d.contact;}]])),
   section('Licences',el('p','nx-note','What other people made that is inside Stella Scope, and the terms each is shared under.'),aboutLinks([['i-view','Code, fonts and data','Every library, font and file inside Stella Scope, with its licence.',()=>{location.href='/help?doc=licences';}],['i-printers','Printer model licences','Each 3D printer model, who made it and the terms it is shared under.',()=>{location.href='/help?doc=models';}]])),
   d.rig?section(`${d.rig.name} (${d.rig.licence})`,el('p','nx-note',`The Printer models page and its catalogue of machines come from ${d.rig.name} by ${d.rig.by}, shared under the ${d.rig.licence} licence. Its own CAD stays with each author and is downloaded only when you ask, with the licence shown first.`),aboutLinks([['i-view','Show the MIT notice','The licence notice that goes with this credit.',()=>rigNotice(d.rig)]])):el('span'),
   section(`3D models inside Stella Scope (${mb(d.bytes||0)})`,el('p','nx-note','These models ship inside Stella Scope so your printer is drawn the moment you add it, with nothing to download. Each is shared free of charge under its maker\'s licence, with its notice. Prusa Research is not connected with Stella Scope and has not approved it.'),list),
   section('Other 3D models',el('p','nx-note','The models of every other machine are not inside Stella Scope. They are downloaded from their authors when you ask, with their licence shown first, under Printer models.')));
 }});
}

// The licences, before the first byte: who made each model, under what terms, how big it is, and where it comes from.
function licences(w,onDone){
 const open=w.models.filter(m=>m.open),mo=open.filter(m=>m.kind!=='part'),po=open.filter(m=>m.kind==='part');
 if(!token){toast('Models are added from the Stella Scope window on the computer.','alarm');return;}
 openModal({title:'Get the 3D models',icon:'i-view',build:(body,close)=>{
  body.append(el('p','nx-note',`These ${mo.length} models are the real CAD of the machines on this farm${po.length?', and the files of their toolchangers and add-ons,':''} made by others and shared under open licences that let anyone use them. Stella Scope downloads each one from the address its author publishes it at (psych0h3ad.github.io, the 3D Print Rig project), and keeps it only if its checksum is the one the author published. Nothing is installed with the app. ${mb(w.open_bytes)} in all.`));
  const list=el('div','nx-gm-list');
  for(const m of mo){const r=el('div','nx-gm-row');r.append(el('b','',m.label),el('span','nx-md-badge',m.licence.text||m.licence.class),el('small','',`${m.licence.author||'its author'} · ${mb(m.bytes||0)}`));r.children[1].dataset.class=m.licence.class;list.append(r);}
  // The tool files are one row: each is small, and they belong together.
  if(po.length){const r=el('div','nx-gm-row'),cls=[...new Set(po.map(m=>m.licence.class))];
   r.append(el('b','',`Tool and add-on files (${po.length} files)`),el('span','nx-md-badge',[...new Set(po.map(m=>m.licence.text))].join(' + ')),el('small','',`${[...new Set(po.map(m=>m.licence.author).filter(Boolean))].join(', ')||'their authors'} · ${mb(po.reduce((n,m)=>n+(m.bytes||0),0))}`));
   r.children[1].dataset.class=cls.length===1?cls[0]:'gpl';list.append(r);}
  body.append(list);
  const msg=el('p','nx-note');msg.setAttribute('role','status');
  const go=button({icon:'i-view',label:`Download ${open.length} files (${mb(w.open_bytes)})`,kind:'primary',size:'row',title:'Download the models',does:'I have read the licences. Downloads these models now.',onPress:async()=>{
   go.disabled=true;
   try{
    const r=await fetch('/api/printer-models/fetch-open',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':token},body:JSON.stringify({accept_licence:true})});
    const j=await r.json();if(!r.ok)throw Error(j.error||'Stella Scope could not start the download.');
    close();modelsBanner(token);
    if(onDone){const t=setInterval(async()=>{try{const x=await (await fetch(WANT,{cache:'no-store'})).json();if(x.fetch?.state!=='running'){clearInterval(t);onDone();}}catch{}},1000);}
   }catch(e){msg.textContent=e.message;go.disabled=false;}}});
  body.append(go,msg);
 }});
}

// ---- F03: one printer's model, said on its tile ------------------------------------------------------------
const pc=x=>Math.round(Math.max(0,Math.min(1,x))*100);
/** What a tile says about its 3D model, from the answer of /api/printer-models/wanted (`needs` names, per printer, the files its picture waits for;
 *  `fetch.items` is each file's own progress). Null: nothing to say (everything it draws is here). Otherwise {state, ...}:
 *  getting (a file of it is arriving: pct), waiting (queued behind others), failed (plain words and a kind), hand (it is added by hand: why) or
 *  missing (nothing is fetching it: a press gets it). */
export function modelStatus(w,pid){
 const ids=w?.needs?.[pid];if(!ids?.length)return null;
 const models=w.models||[],items=(w.fetch?.items)||[];
 const rows=ids.map(id=>({id,row:models.find(m=>m.id===id),item:items.find(i=>i.id===id)}));
 const bad=rows.find(r=>r.item?.state==='failed');
 if(bad)return {state:'failed',kind:bad.item.failure?.kind||'other',plain:bad.item.failure?.plain||bad.item.error||'The 3D model could not be downloaded.',id:bad.id};
 const mine=rows.filter(r=>r.item);
 if(mine.length){
  const got=mine.reduce((n,r)=>n+(r.item.got||0),0),total=mine.reduce((n,r)=>n+(r.item.bytes||0),0);
  const going=mine.some(r=>r.item.state==='fetching'||r.item.state==='done');   // a file of it has started (or finished: the others are queued)
  // every byte of what is arriving has been read: what is left is unpacking it and checking it against its SHA-256
  const now=mine.filter(r=>r.item.state==='fetching'),checking=now.length>0&&now.every(r=>r.item.bytes>0&&r.item.got>=r.item.bytes);
  return {state:going?'getting':'waiting',pct:total?got/total:0,got,total,checking};}
 const hold=rows.find(r=>r.row&&!r.row.open);
 if(hold)return {state:'hand',plain:hold.row.why||'This model is added by hand under Printer models.'};
 return {state:'missing',bytes:rows.reduce((n,r)=>n+(r.row?.bytes||0),0)};}

/** Press again for every open model still missing (a retry after a failure, or the first fetch of a farm that was restarted). */
export async function getModelsNow(tok){
 const r=await fetch('/api/printer-models/fetch-open',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':tok||''},body:JSON.stringify({accept_licence:true})});
 const j=await r.json().catch(()=>({}));if(!r.ok)throw Error(j.error||'Stella Scope could not start the download.');
 modelsBanner(tok);return j;}

/** The open licences of the demo farm in one line (src/printer_models.rs `licence_line`), shown next to the button that fetches them. */
let lineOnce=null;
export function demoLicenceLine(){
 lineOnce??=fetch('/api/printer-models/demo-farm',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(j=>j?.line||'').catch(()=>'');
 return lineOnce;}
export const demoLicenceBytes=()=>fetch('/api/printer-models/demo-farm',{cache:'no-store'}).then(r=>r.json()).then(j=>j.bytes||0).catch(()=>0);

/** The strip on a tile: what is happening to its model. `ctx.token()` is the farm token (null where the window cannot manage);
 *  `set(status)` draws it (null hides it). */
export function tileModelPanel(ctx){
 const box=el('div','fm-mdl');box.hidden=true;box.setAttribute('role','status');
 const text=el('span','fm-mdl-t'),bar=el('div','nx-gm-bar fm-mdl-bar');bar.setAttribute('role','progressbar');bar.setAttribute('aria-label','Getting the 3D model');
 const again=button({icon:'i-play',label:'Try again',size:'row',title:'Try again',does:'Asks the authors\' site for the 3D models that are still missing, with the same checks.',onPress:async b=>{if(b.dataset.busy!==undefined)return;
  const tok=ctx.token();if(!tok){toast('Models are added from the Stella Scope window on the computer.','alarm');return;}busy(b,true);try{await getModelsNow(tok);}catch(e){toast(e.message,'alarm');}finally{busy(b,false);}}});
 const get=button({icon:'i-view',label:'Get it',size:'row',title:'Get it: the 3D model',does:'Downloads this machine\'s 3D model from its author, checked against the checksum its author published. The licences were shown first.',onPress:async b=>{if(b.dataset.busy!==undefined)return;
  const tok=ctx.token();if(!tok){toast('Models are added from the Stella Scope window on the computer.','alarm');return;}busy(b,true);try{await getModelsNow(tok);}catch(e){toast(e.message,'alarm');}finally{busy(b,false);}}});
 for(const b of [again,get])b.hidden=true;
 box.append(text,bar,again,get);
 let last='';
 return {el:box,set(st){
  if(!st){if(!box.hidden){box.hidden=true;last='';}return;}
  const key=JSON.stringify([st.state,st.pct===undefined?'':pc(st.pct),st.plain||'',st.bytes||0,!!ctx.token()]);if(key===last)return;last=key;
  box.hidden=false;box.dataset.state=st.state;
  box.setAttribute('aria-live',st.state==='getting'||st.state==='waiting'?'off':'polite');   // a percentage that moves every second is not read out; a failure or a question is
  bar.hidden=!(st.state==='getting'||st.state==='waiting');again.hidden=st.state!=='failed'||!ctx.token();
  get.hidden=st.state!=='missing'||!ctx.token();
  if(st.state==='getting'){const p=st.pct;text.textContent=st.checking?'3D model: checking it against its checksum':`3D model: getting it, ${pc(p)}% of ${mb(st.total)}`;bar.style.setProperty('--p',p.toFixed(3));bar.setAttribute('aria-valuenow',String(pc(p)));}
  else if(st.state==='waiting'){text.textContent='3D model: waiting for its turn';bar.style.setProperty('--p','0');bar.setAttribute('aria-valuenow','0');}
  else if(st.state==='failed')text.textContent=st.plain;
  else if(st.state==='hand')text.textContent=st.plain;
  else text.textContent=`3D model: not here yet (${mb(st.bytes)}).`;
  box.title=text.textContent;}};}
