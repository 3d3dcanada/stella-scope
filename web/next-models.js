// Printer models: the catalogue of real CAD, installing one, binding one to a printer, and the live 3D view
// that follows the printer's reported position (farm board WO-10, 2026-10-12).
// our rule: if the real model does not exist, there is no model. Nothing here draws a stand-in.
// The 3D engine (model-view.js, three.js) is loaded only when a model is opened.
import {el,icon,tip,button,busy,openSheet,closeSheet,openModal,section,segmented,toast} from './next-ui.js';
import {modelsButton,showNotice} from './next-getmodels.js';
import {plainLabel,matches,needs,selection,dockTools,drawable} from './model-pick.js';

const call=(method,url,body,token)=>fetch(url,{method,headers:{...(body!==undefined?{'Content-Type':'application/json'}:{}),...(token?{'X-Stella-Control-Token':token}:{})},body:body===undefined?undefined:JSON.stringify(body)})
 .then(async r=>{const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);return d;});
const WORDS={gpl:'Open · GPL','cc-by-sa':'Open · CC BY-SA','cc-by':'Open · CC BY',cc0:'Open · CC0',mit:'Open · MIT',nc:'Non-commercial',ocl:'Open · OCL (Prusa)',mixed:'Mixed terms',unlicensed:'No licence stated'};
const KIND={toolchanger:'Toolchangers',toolhead:'Toolheads',addon:'Add-ons'};
const FAMILY={v24:'Voron 2.4',trident:'Voron Trident',v0:'Voron 0',micron:'Voron Micron',vcore4:'Rat Rig V-Core 4',vcore3:'Rat Rig V-Core 3',vminion:'Rat Rig V-Minion',
 annex_k1:'Annex K1',annex_k2:'Annex K2',annex_k3:'Annex K3',snakeoil_xy:'SnakeOil XY',snakeoil_3s:'SnakeOil 3S',proosaxy:'ProosaXY',mercury:'Mercury One.1',sovol_sv08:'Sovol SV08',
 lh_stinger:'LH Stinger',vzbot:'VzBot',ender3:'Creality Ender-3',sboom:'SIBOOR S-Boom',doomcube:'DoomCube',aether:'Aether',satsuma:'Satsuma',tictac:'TicTac',the100:'The 100',rook:'Rook',
 positron:'Positron',remorph:'ReMorph',crossant:'Crossant',fysetc:'FYSETC','247zero':'247Zero',e3ng:'E3NG hotend changer','prusa-core-one':'Prusa CORE One','prusa-i3':'Prusa i3','prusa-mini':'Prusa MINI',prusa:'Prusa'};
const family=f=>FAMILY[f]||String(f||'').replace(/[_-]+/g,' ').replace(/\b[a-z]/g,c=>c.toUpperCase());
const mb=n=>n>=1048576?(n/1048576).toFixed(n>=10485760?0:1)+' MB':Math.max(1,Math.round(n/1024))+' KB';
const tri=n=>n?(n>=1e6?(n/1e6).toFixed(1)+' million':Math.round(n/1000)+' thousand')+' triangles':'';
const note=(t,cls='')=>el('p','nx-note'+(cls?' '+cls:''),t);
const licenceBadge=m=>{const b=el('span','nx-md-badge',WORDS[m.licence.class]||m.licence.class);b.dataset.class=m.licence.class;return b;};
const libraryFetch=()=>fetch('/api/printer-models',{cache:'no-store'}).then(r=>r.json());
const optionsFetch=()=>fetch('/api/printer-models/options',{cache:'no-store'}).then(r=>r.json());
const plural=(n,w)=>`${n} ${w}${n===1?'':'s'}`;
const optionsOf=id=>fetch(`/api/printer-models/${id}/options`,{cache:'no-store'}).then(r=>r.json());
const machineOf=id=>fetch(`/api/printer-models/${id}`,{cache:'no-store'}).then(r=>r.json());
let uid=0;

/** A choice that always reads in full (polish WO-05, 2026-10-18): the chosen text wraps instead of being cut off and the list is
 *  inline under it. A native select clips a long label on a phone ("Xol / Sherpa Mini · r16 / Rap…") and cannot be made to wrap.
 *  `options` is [{value,label}]. Returns the element; `.get()` and `.set(v)` read and write the value. */
export function choice(options,value,onChange,{label=''}={}){
  const w=el('div','nx-choice'),btn=el('button','nx-choice-btn'),list=el('div','nx-choice-list'),text=el('span','nx-choice-text');
  btn.type='button';btn.setAttribute('aria-haspopup','listbox');btn.setAttribute('aria-expanded','false');if(label)btn.setAttribute('aria-label',label);
  list.setAttribute('role','listbox');list.hidden=true;btn.append(text,icon('i-arrow-down','nx-ico nx-choice-caret'));
  let cur=value;
  const paint=()=>{const o=options.find(x=>x.value===cur)||options[0];text.textContent=o?o.label:'';for(const b of list.children)b.setAttribute('aria-selected',String(b.dataset.v===String(cur)));};
  const shut=()=>{list.hidden=true;btn.setAttribute('aria-expanded','false');document.removeEventListener('pointerdown',away,true);};
  for(const o of options){const b=el('button','nx-choice-opt',o.label);b.type='button';b.dataset.v=String(o.value);b.setAttribute('role','option');b.addEventListener('click',()=>{cur=o.value;paint();shut();btn.focus();onChange?.(cur);});list.append(b);}
  const away=e=>{if(!w.contains(e.target)){shut();}};   // a press anywhere else closes the list
  btn.addEventListener('click',()=>{const open=list.hidden;list.hidden=!open;btn.setAttribute('aria-expanded',String(open));if(open){document.addEventListener('pointerdown',away,true);list.querySelector('[aria-selected=true]')?.scrollIntoView({block:'nearest'});}else document.removeEventListener('pointerdown',away,true);});
  w.addEventListener('keydown',e=>{if(e.key==='Escape'&&!list.hidden){e.stopPropagation();shut();btn.focus();}});
  w.append(btn,list);w.get=()=>cur;w.set=v=>{cur=v;paint();};paint();return w;}

/** Getting the files a choice needs: what will be fetched with each licence, the tick for anything not open, then two plain
 *  buttons, From the author (the default) and From a folder on this computer... (2026-10-18: nobody types the word "published").
 *  `items` are needs().todo. `done()` runs once every file is here (an empty list goes straight to it). `extra` are more buttons. */
function getFiles(box,{items,token,done,doneLabel='Use this',later=null,extra=[],got=new Set()}){
  box.replaceChildren();
  const msg=note('');msg.setAttribute('role','status');
  let ok=null;   // the licence tick, when one file is not under an open licence
  box.say=t=>{msg.textContent=t;};box.accepted=()=>!ok||ok.checked;   // for an extra button of the caller (installing a file the person downloaded)
  const run=async(source,btns)=>{
    const todo=items.filter(i=>!got.has(i.kind+i.id));
    if(ok&&!ok.checked&&todo.some(i=>!i.bundled)){msg.textContent='Tick the box to say you have read the licence.';return;}
    btns.forEach(b=>busy(b,true));
    try{let n=0;for(const i of todo){n++;msg.textContent=`Getting ${n} of ${todo.length}: ${i.label}${i.bytes?` (${mb(i.bytes)})`:''}. A large file takes a minute.`;
        await call('POST',i.kind==='machine'?`/api/printer-models/${i.id}/install`:`/api/printer-models/options/${i.id}/install`,{source,accept_licence:i.bundled||!!ok?.checked},token);got.add(i.kind+i.id);}
      msg.textContent='';await done();}
    catch(e){msg.textContent=(got.size?`${plural(got.size,'file')} kept. `:'')+e.message;}finally{btns.forEach(b=>busy(b,false));}};
  if(!token){box.append(note('Models are installed from the Stella Scope window on the computer.'));return;}
  if(!items.length){const go=button({icon:'i-check',label:doneLabel,kind:'primary',size:'row',title:doneLabel,does:'Saves this choice.',onPress:()=>run('published',[go])});box.append(go,msg);return;}
  const bytes=items.reduce((a,i)=>a+i.bytes,0),list=el('div','nx-md-files');
  for(const i of items){const r=el('div','nx-md-file');r.append(el('span','nx-md-fname',i.label),licenceBadge({licence:{class:i.cls}}),el('span','nx-md-meta',[i.text,i.bytes?mb(i.bytes):''].filter(Boolean).join(' · ')));list.append(r);}
  box.append(note(`${plural(items.length,'file')} to get${bytes?` (${mb(bytes)})`:''}. Each is checked against the checksum its author published before it is kept. The licence of each is shown here first.`),list);
  if(items.some(i=>!i.bundled)){ok=el('input');ok.type='checkbox';ok.id='nx-gf-ok'+(++uid);const l=el('label','nx-md-ok');l.htmlFor=ok.id;l.append(ok,el('span','',`I have read the licences above. These files stay under their authors' terms${(t=>t?` (${t})`:'')([...new Set(items.filter(i=>!i.bundled&&i.text).map(i=>i.text))].join(', '))}.`));box.append(l);}
  const folder=el('input','nx-input');folder.placeholder='/home/you/viewer-models';folder.setAttribute('aria-label','Folder on this computer');folder.autocomplete='off';folder.spellcheck=false;
  const frow=el('div','nx-md-folder');frow.hidden=true;
  const fetchFolder=button({icon:'i-files',label:'Get from this folder',kind:'primary',size:'row',title:'Get from this folder',does:'Takes the files from the folder you named, checks each against its published checksum and keeps it.',onPress:()=>{if(!folder.value.trim()){msg.textContent='Type the full path of the folder, starting with /.';return;}run(folder.value.trim(),all);}});
  frow.append(note('The full path of a folder that holds a copy of 3D Print Rig\'s model files (an unpacked viewer-models.zip, or a checkout of the project).'),folder,fetchFolder);
  const author=button({icon:'i-link',label:'From the author',kind:'primary',size:'row',title:'From the author',does:'Downloads each file from the address its author publishes it at, checks it against the published checksum and keeps it.',onPress:()=>run('published',all)});
  const here=button({icon:'i-files',label:'From a folder on this computer...',size:'row',title:'From a folder on this computer',does:'Takes the files from a folder you name instead of the internet.',onPress:()=>{frow.hidden=!frow.hidden;if(!frow.hidden)folder.focus();}});
  const all=[author,here,fetchFolder];
  const row=el('div','nx-md-btns');row.append(author,here,...extra);
  if(later)row.append(button({icon:'i-arrow-right',label:later.label,kind:'ghost',size:'row',title:later.label,does:later.does,onPress:async b=>{busy(b,true);try{await later.run();}catch(e){msg.textContent=e.message;}finally{busy(b,false);}}}));
  box.append(row,frow,msg);}

// ---- the catalogue --------------------------------------------------------------------------------
/** The machines of 3D Print Rig as rows with a thumbnail, licence, search, family and licence filters. With `onUse` each row has a
 *  Use this button (choosing a printer's model); without it a row only opens its details (browsing the library). */
function machinePane(host,{token=null,onUse=null,hint='',onChange=()=>{}}={}){
  const q=el('input','nx-input');q.type='search';q.placeholder='Search the machines';q.setAttribute('aria-label','Search the models');q.autocomplete='off';q.value=hint;
  const fam=el('select','nx-input nx-md-select');fam.setAttribute('aria-label','Family');
  const st={q:hint,fam:'',kind:'all',data:null,banks:new Set()};
  const kind=segmented(['all','open','author','mine','changers'],'all',v=>{st.kind=v;paint();},v=>({all:'All',open:'Open',author:'Other',mine:'Installed',changers:'Changers'}[v]));
  const list=el('div','nx-md-list');
  const filters=el('div','nx-md-filters');filters.append(q,fam);
  host.append(filters,kind,list);
  const paint=()=>{
    const d=st.data;if(!d)return;
    const rows=d.models.filter(m=>(!st.fam||m.family===st.fam)&&matches(st.q,m.label+' '+family(m.family)+' '+m.licence.text)
      &&(st.kind==='all'||(st.kind==='open'&&m.bundled)||(st.kind==='author'&&!m.bundled)||(st.kind==='mine'&&m.installed)||(st.kind==='changers'&&st.banks.has(m.id)))).sort((x,y)=>(!!y.installed-!!x.installed)||x.label.localeCompare(y.label));   // what is on this computer first, then A to Z
    list.replaceChildren();
    // finish-2 F09: the Prusa machines no 3D model draws, each with the exact reason (shown with the whole list and the Prusa family, not under Open or Installed)
    const none=(st.kind==='all'&&(!st.fam||st.fam==='prusa')?d.no_model||[]:[]).filter(m=>matches(st.q,m.label+' '+m.short+' prusa no 3d model'));
    if(!rows.length&&!none.length){list.append(note('No machine matches. Clear the search or the filter.'));return;}
    for(const m of rows)list.append(row(m));
    for(const m of none)list.append(noModelRow(m));
  };
  const noModelRow=m=>{
    const r=el('div','nx-md-row nx-md-row--none'),th=el('div','nx-md-thumb');th.append(icon('i-printers','nx-ico nx-md-ph'));
    const mid=el('div','nx-md-mid');mid.append(el('div','nx-md-name',m.label),el('div','nx-md-meta','Prusa · no 3D model yet'));
    const tags=el('div','nx-md-tags'),t=el('span','nx-md-badge','No 3D model yet');t.dataset.class='none';tags.append(t);mid.append(tags,note(m.why));
    r.append(th,mid);return r;};
  const row=m=>{
    const r=el('div','nx-md-row');
    const th=el('div','nx-md-thumb');
    if(m.thumb){const im=new Image();im.alt='';im.src=m.thumb;im.loading='lazy';th.append(im);}else th.append(icon('i-printers','nx-ico nx-md-ph'));
    const mid=el('div','nx-md-mid');
    mid.append(el('div','nx-md-name',m.label),el('div','nx-md-meta',[family(m.family),m.size_mm?m.size_mm+' mm':'',m.motion?'moves with the printer':'shown as built',m.fits?.length?plural(m.fits.length,'option'):''].filter(Boolean).join(' · ')));
    const tags=el('div','nx-md-tags');tags.append(licenceBadge(m));
    if(st.banks.has(m.id)){const t=el('span','nx-md-badge','Takes a toolchanger');t.dataset.class='installed';tags.append(t);}
    // finish-2 F09: a model the build carries is in the library from the first start and says so
    if(m.in_build){const t=el('span','nx-md-badge','Inside Stella Scope');t.dataset.class='installed';tags.append(t);}
    else if(m.installed){const t=el('span','nx-md-badge','Installed');t.dataset.class='installed';tags.append(t);}
    else if(!m.has_file){const t=el('span','nx-md-badge','No file published');t.dataset.class='none';tags.append(t);}
    mid.append(tags);
    const act=el('div','nx-md-act');
    if(onUse&&m.has_file)act.append(button({icon:'i-check',label:'Use this',kind:'primary',size:'row',title:'Use this machine',does:`Chooses ${m.label}, then what is fitted to it.`,onPress:()=>onUse(m)}));
    act.append(button({icon:'i-view',label:m.installed?'Open':'Details',size:'row',kind:onUse&&m.has_file?'default':'primary',title:m.label,does:m.installed?'Shows the model.':'Shows the licence, where it comes from and how to install it.',onPress:()=>modelDetail(m,{token,onChange:load})}));
    r.append(th,mid,act);return r;};
  const load=async()=>{try{const [d,od]=await Promise.all([libraryFetch(),optionsFetch().catch(()=>null)]);st.data=d;
      st.banks=new Set((od?.options||[]).filter(o=>o.drawable&&o.kind==='toolchanger').flatMap(o=>o.fits));
      q.placeholder=`Search ${d.count} machines`;
      fam.replaceChildren();const o=el('option','','All families');o.value='';fam.append(o);
      for(const f of [...new Set([...st.data.models.map(m=>m.family),...(st.data.no_model?.length?['prusa']:[])])].sort((a,b)=>family(a).localeCompare(family(b)))){const op=el('option','',family(f));op.value=f;fam.append(op);}
      fam.value=st.fam;
      if(st.q&&!d.models.some(m=>matches(st.q,m.label+' '+family(m.family)+' '+m.licence.text))){st.q='';q.value='';}   // a hint that finds nothing is not left in the box
      paint();onChange();}catch(e){list.replaceChildren(note('The model list could not be read: '+e.message));}};
  q.addEventListener('input',()=>{st.q=q.value;paint();});fam.addEventListener('change',()=>{st.fam=fam.value;paint();});
  load();return {reload:load};
}

/** The Printer models sheet. With `pick` (a printer summary) it is the choosing flow for that printer; without, the library: the
 *  machines and the options (toolchangers, toolheads, add-ons) of 3D Print Rig, each with its licence (polish WO-01, 2026-10-18). */
export function modelsSheet({token=null,pick=null,onPicked=()=>{}}={}){
  if(pick&&token)return openSheet({title:`Model for ${pick.name||pick.id}`,icon:'i-printers',wide:true,build:body=>modelFlow(body,{token,printer:pick,start:{cad_model:pick.cad_model,cad_options:pick.cad_options},onDone:sel=>{closeSheet();onPicked(sel);}})});
  openSheet({title:pick?`Model for ${pick.name||pick.id}`:'Printer models',icon:'i-printers',wide:true,build:body=>{
    const machines=el('div','nx-md-pane'),opts=el('div','nx-md-pane');opts.hidden=true;
    if(!pick)body.append(segmented(['machines','options'],'machines',v=>{machines.hidden=v!=='machines';opts.hidden=v!=='options';},v=>v==='machines'?'Machines':'Options'));
    body.append(machines,opts);
    machines.append(note(pick
      ?`Choose the real CAD for ${pick.name||pick.id}. If it is not on the list, this printer has no model: Stella Scope never draws a stand-in.`
      :'Real CAD from 3D Print Rig and from Prusa and its community, each file checked against the published checksum. Inside Stella Scope: nothing to download. Open: an open licence, may be shared. Other: non-commercial or no licence stated, so it stays with its author and is downloaded when you ask. Changers: machines that take a toolchanger. A Prusa with no model is listed with the reason.'));
    const pane=machinePane(machines,{token});
    if(!pick)optionsPane(opts,{token,onChange:pane.reload});
  }});
}

// ---- choosing a model: the machine, what is fitted, the files (polish WO-05, 2026-10-18) -----------------------
/** One flow for both places a model is chosen. With `printer` (a printer summary) the choice is saved on it; without (Add a printer)
 *  `onDone({cad_model,cad_options})` gets it, or `onDone(null)` for no model. Step 1 the machine (thumbnails, search), step 2 what is
 *  fitted to it (what fits and can be drawn) and the files it needs with each licence shown first. `onBack` is the step before, if any. */
export function modelFlow(body,{token,printer=null,start={},hint='',step='',intro='',onBack=null,onDone=()=>{}}){
  const kept=new Set();
  const head=(title,sub,back)=>{const h=el('div','nx-fm-head');if(back)h.append(button({icon:'i-back',size:'icon',kind:'ghost',title:'Back',does:'Goes back one step. Nothing has been saved.',onPress:back}));const t=el('div','nx-fm-headtext');t.append(el('b','',title));if(sub)t.append(el('small','',sub));h.append(t);return h;};
  const name=printer?.name||printer?.id||'this printer';
  const machines=()=>{
    body.replaceChildren(head('Choose the machine',step?`${step} · the machine`:'Step 1 of 2',onBack));
    const cur=start.cad_model,top=el('div','nx-md-btns');
    if(cur){const now=el('p','nx-md-cur','');body.append(now);machineOf(cur).then(m=>{now.textContent=`Now: ${plainLabel(m.label)}`;}).catch(()=>{now.textContent='Now: '+cur;});
      top.append(button({icon:'i-check',label:'Keep this machine, change what is fitted',kind:'primary',size:'row',title:'Keep this machine, change what is fitted',does:'Goes to what is fitted to the machine this printer already has.',onPress:()=>machineOf(cur).then(m=>fit(m,start.cad_options||[])).catch(e=>toast(e.message,'alarm'))}));}
    if(cur&&printer)top.append(button({icon:'i-close',label:'No model',kind:'ghost',size:'row',title:'No model',does:'Takes the model off this printer. The downloaded file stays in the library.',onPress:async()=>{try{await call('PATCH','/api/printers/'+printer.id,{cad_model:null},token);toast('Model removed from this printer.','ok');modelChanged(printer.id);onDone(null);}catch(e){toast(e.message,'alarm');}}}));
    if(!printer)top.append(button({icon:'i-arrow-right',label:'No 3D model for now',kind:'ghost',size:'row',title:'No 3D model for now',does:'Adds the printer without a model. One can be chosen later from its page.',onPress:()=>onDone(null)}));
    if(top.children.length)body.append(top);
    if(intro)body.append(note(intro));
    body.append(note(`Choose the real CAD for ${name}. If it is not on the list, this printer has no model: Stella Scope never draws a stand-in.`));
    machinePane(body,{token,hint,onUse:m=>fit(m,m.id===cur?start.cad_options||[]:[])});
  };
  const fit=async(m0,cur=[])=>{
    body.replaceChildren(head(plainLabel(m0.label),step?`${step} · what is fitted`:'Step 2 of 2',()=>machines()));
    let m=m0,d={options:[]};
    try{[m,d]=await Promise.all([machineOf(m0.id),optionsOf(m0.id)]);}catch(e){body.append(note('This machine could not be read: '+e.message));return;}
    const sum=el('div','nx-md-row nx-md-row--fit'),th=el('div','nx-md-thumb');
    if(m.thumb){const im=new Image();im.alt='';im.src=m.thumb;th.append(im);}else th.append(icon('i-printers','nx-ico nx-md-ph'));
    const mid=el('div','nx-md-mid'),tags=el('div','nx-md-tags');tags.append(licenceBadge(m));
    const have=el('span','nx-md-badge',m.installed?'On this computer':m.bytes?`Gets ${mb(m.bytes)}`:'Not on this computer yet');have.dataset.class=m.installed?'installed':'none';tags.append(have);
    mid.append(el('div','nx-md-name',plainLabel(m.label)),el('div','nx-md-meta',[family(m.family),m.size_mm?m.size_mm+' mm':'',m.motion?'moves with the printer':'shown as built'].filter(Boolean).join(' · ')),tags);
    sum.append(th,mid);body.append(sum,note(`${m.licence.words||''}${m.licence.text&&m.licence.class!=='unlicensed'&&!(m.licence.words||'').toLowerCase().includes(m.licence.text.toLowerCase().split(' (')[0])?` (${m.licence.text})`:''}${m.licence.author?` By ${m.licence.author}.`:''}`));
    if(m.note)body.append(note(m.note));
    if(!m.has_file){body.append(note('The author has not published a model file for this machine, so there is nothing to install. Stella Scope does not draw one in its place.'));return;}
    const choose=el('div','nx-md-optbox'),bar=el('div','nx-md-install');
    body.append(section('What is fitted (optional)',choose),section(printer?`Use it on ${name}`:'Use it',bar));
    let ch=null;
    const save=async()=>{const sel=ch.value();
      if(printer){const b={cad_options:sel};if(m.id!==printer.cad_model)b.cad_model=m.id;await call('PATCH','/api/printers/'+printer.id,b,token);toast(sel.length?'Model and options set.':'Model set.','ok');modelChanged(printer.id);}
      return onDone({cad_model:m.id,cad_options:sel});};
    const paintBar=()=>{const n=needs(m,d.options,ch.value());n.todo=n.todo.filter(i=>!kept.has(i.kind+i.id));
      getFiles(bar,{items:n.todo,token,got:kept,done:save,doneLabel:'Use this model',later:n.todo.length?{label:'Choose it now, get the files later',does:'Saves the choice without the files. The printer\'s page then offers to get them.',run:save}:null});};
    ch=optionChooser(choose,{options:drawable(d.options),heads:d.options.filter(o=>o.kind==='toolhead').length,current:cur,onChange:paintBar});paintBar();
  };
  machines();
}

/** What fits this machine and Stella Scope can draw: one toolchanger at most (how many docks, the tool in each, which is on the
 *  gantry) and any add-ons. Every choice reads in full (`choice`). `value()` is the cad_options array. */
function optionChooser(box,{options,heads=0,current=[],onChange=()=>{}}){
  if(!options.length){box.append(note('This machine has no toolchanger or add-on to draw, so there is nothing to choose here: press Use this model.'+(heads?' Toolheads are in the library, but they are only drawn as the tools of a toolchanger.':'')));return {value:()=>[]};}
  const chosen=new Map(current.map(c=>[c.option,c])),rows=[],banks=[];
  const fire=()=>onChange(selection(rows));
  const build=o=>{
    const cur=chosen.get(o.id),r=el('div','nx-md-opt nx-md-opt--pick');
    const use=el('button','nx-md-chip',plainLabel(o.label));use.type='button';use.setAttribute('aria-pressed',String(!!cur));
    const tags=el('div','nx-md-tags');tags.append(licenceBadge(o));
    const have=el('span','nx-md-badge',o.installed?'On this computer':o.bytes?`Gets ${mb(o.bytes)}`:'Not installed yet');have.dataset.class=o.installed?'installed':'none';tags.append(have);
    const top=el('div','nx-md-optname');top.append(use,tags);r.append(top);
    const row={on:!!cur,value:()=>({option:o.id}),show:()=>{}};
    row.set=v=>{row.on=v;use.setAttribute('aria-pressed',String(v));row.show();};
    if(o.kind==='toolchanger'&&o.capacity&&o.tools?.length){
      const ctl=el('div','nx-md-bank'),tools=o.tools.map(t=>({value:t.id,label:plainLabel(t.label||t.id)}));
      let n=cur?.tools?.length||Math.min(2,o.capacity),picked=dockTools(o.tools,n,cur?.tools||[]),active=Math.min(cur?.active||0,n-1);
      const docks=el('div','nx-md-docks'),gantry=el('div','nx-md-dock');
      const draw=()=>{docks.replaceChildren();picked.forEach((id,i)=>{const l=el('div','nx-md-dock');l.append(el('span','',`Dock ${i+1}`),choice(tools,id,v=>{picked[i]=v;fire();},{label:`Tool in dock ${i+1}`}));docks.append(l);});
        gantry.replaceChildren(el('span','','On the gantry'),choice(Array.from({length:n},(_,i)=>({value:i,label:`Dock ${i+1}`})),active,v=>{active=v;fire();},{label:'The tool on the gantry'}));};
      const count=el('div','nx-md-dock');count.append(el('span','','Docks'),choice(Array.from({length:o.capacity},(_,i)=>({value:i+1,label:plural(i+1,'dock')})),n,v=>{n=v;picked=dockTools(o.tools,n,picked);active=Math.min(active,n-1);draw();fire();},{label:'How many docks'}));
      draw();ctl.append(count,docks,gantry);r.append(ctl);ctl.hidden=!cur;row.show=()=>{ctl.hidden=!row.on;};
      row.value=()=>({option:o.id,tools:[...picked],active});banks.push(row);
    }
    use.addEventListener('click',()=>{const on=!row.on;if(on)for(const b of banks)if(b!==row&&b.on)b.set(false);row.set(on);fire();});
    rows.push(row);return r;};
  const part=(title,kind)=>{const list=options.filter(o=>o.kind===kind);if(!list.length)return;const w=el('div','nx-md-opts');for(const o of list)w.append(build(o));box.append(section(title,w));};
  part('Toolchanger (one)','toolchanger');part('Add-ons','addon');
  if(heads)box.append(note('Toolheads are drawn as the tools in a toolchanger\'s docks. On their own they are in the library but not drawn.'));
  return {value:()=>selection(rows)};
}

// ---- the options: toolchangers, toolheads, add-ons -------------------------------------------------
function optionsPane(box,{token=null,onChange=()=>{}}={}){
  const q=el('input','nx-input');q.type='search';q.placeholder='Search the options';q.setAttribute('aria-label','Search the options');q.autocomplete='off';
  const st={q:'',kind:'all',data:null};
  const kinds=segmented(['all','toolchanger','toolhead','addon'],'all',v=>{st.kind=v;paint();},v=>({all:'All',toolchanger:'Changers',toolhead:'Heads',addon:'Add-ons'}[v]));
  const list=el('div','nx-md-list');
  box.append(note('The toolchangers, toolheads and add-ons of 3D Print Rig. Each is a set of files with its own licence, checked against the published checksum. Those marked Drawn on the printer can be fitted to a machine\'s model.'),q,kinds,list);
  const paint=()=>{
    if(!st.data)return;
    const text=st.q.trim().toLowerCase();
    const rows=st.data.options.filter(o=>(st.kind==='all'||o.kind===st.kind)&&(!text||(o.label+' '+o.category+' '+o.licence.text+' '+(o.system||'')).toLowerCase().includes(text)));
    list.replaceChildren();
    if(!rows.length){list.append(note('No option matches. Clear the search or the filter.'));return;}
    for(const o of rows)list.append(optionRow(o,{token,onChange:()=>{load();onChange();}}));
  };
  const load=async()=>{try{st.data=await optionsFetch();q.placeholder=`Search ${st.data.count} options`;paint();}catch(e){list.replaceChildren(note('The option list could not be read: '+e.message));}};
  q.addEventListener('input',()=>{st.q=q.value;paint();});
  load();
}
const optionRow=(o,{token,onChange})=>{
  const r=el('div','nx-md-row');
  const th=el('div','nx-md-thumb');th.append(icon('i-printers','nx-ico nx-md-ph'));
  const mid=el('div','nx-md-mid');
  mid.append(el('div','nx-md-name',o.label),el('div','nx-md-meta',[o.category,plural(o.parts,'file'),mb(o.bytes||0),o.fits.length?'fits '+plural(o.fits.length,'machine'):'shown on its own'].join(' · ')));
  const tags=el('div','nx-md-tags');tags.append(licenceBadge(o));
  if(o.installed){const t=el('span','nx-md-badge','Installed');t.dataset.class='installed';tags.append(t);}
  if(o.drawable){const t=el('span','nx-md-badge','Drawn on the printer');t.dataset.class='installed';tags.append(t);}
  mid.append(tags);
  const act=el('div','nx-md-act');
  act.append(button({icon:'i-view',label:'Details',size:'row',kind:'primary',title:o.label,does:'Shows the licence, the files and which machines take it.',onPress:()=>optionDetail(o,{token,onChange})}));
  r.append(th,mid,act);return r;
};

/** One option: its licence, its files with theirs, the machines that take it, and how to install it. */
export function optionDetail(o,{token=null,onChange=()=>{}}={}){
  openModal({title:o.label,icon:'i-printers',build:(body,close)=>{
    body.closest('.nx-modal-card')?.classList.add('nx-md-card');
    const box=el('div','nx-md-install');
    const paint=async()=>{
      const d=await fetch('/api/printer-models/options/'+o.id,{cache:'no-store'}).then(r=>r.json());
      Object.assign(o,d);body.replaceChildren();
      const head=el('div','nx-md-tags');head.append(licenceBadge(d),el('span','nx-md-badge',(KIND[d.kind]||d.kind).replace(/s$/,'')));
      if(d.drawable)head.append(el('span','nx-md-badge','Drawn on the printer'));
      body.append(head,section('Licence',note(d.licence.text||'Not stated by the author.'),note(d.licence.words),d.licence.author?note('By '+d.licence.author):el('span'),note('The class is the strictest of the files below.')));
      const files=el('div','nx-md-files');
      for(const f of d.files){const r=el('div','nx-md-file');r.append(el('span','nx-md-fname',f.label||f.id));const b=licenceBadge({licence:{class:f.class}});r.append(b,el('span','nx-md-meta',mb(f.bytes||0)+(f.installed?' · installed':'')));files.append(r);}
      body.append(section(plural(d.files.length,'file'),files));
      body.append(note(d.machines.length?'Fits: '+d.machines.slice(0,10).map(m=>m.label).join(', ')+(d.machines.length>10?` and ${d.machines.length-10} more`:'')+'.':'Shown on its own: 3D Print Rig registers no machine for it.'));
      if(d.note)body.append(note(d.note));
      if(d.scope)body.append(note(d.scope));
      body.append(box);installUi(d);
    };
    const installUi=d=>{
      box.replaceChildren();
      const msg=note('');msg.setAttribute('role','status');
      if(d.installed){
        box.append(section('Installed',note(`All ${plural(d.files.length,'file')} are kept on this computer, each checked against its author's checksum.`)));
        const row=el('div','nx-md-btns');
        if(d.previewable)row.append(button({icon:'i-view',label:'Look at it',kind:'primary',size:'row',title:'Look at it',does:'Shows the files of this option together, at the places the project shows them.',onPress:()=>previewOption(d)}));
        if(token){row.append(button({icon:'i-close',label:'Remove from this computer',kind:'danger',size:'row',title:'Remove',does:'Deletes the downloaded files this option needs. Files another installed option needs stay.',onPress:async()=>{try{await call('DELETE','/api/printer-models/options/'+d.id,undefined,token);toast('Removed.','ok');onChange();paint();}catch(e){msg.textContent=e.message;}}}));}
        box.append(row,msg);return;}
      if(!token){box.append(note('Options are installed from the Stella Scope window on the computer.'));return;}
      // Polish WO-05 (2026-10-18): the licence first, then two plain buttons: From the author, or From a folder on this computer...
      getFiles(box,{items:needs(null,[d],[{option:d.id}]).todo,token,doneLabel:'Done',done:async()=>{toast('Installed.','ok');onChange();paint();}});
    };
    paint().catch(e=>body.append(note('The option could not be read: '+e.message)));
  }});
}

// ---- one model: licence, source, install -----------------------------------------------------------
export function modelDetail(m,{token=null,printer=null,onChange=()=>{}}={}){
  openModal({title:m.label,icon:'i-printers',build:(body,close)=>{
    const card=body.closest('.nx-modal-card');card?.classList.add('nx-md-card');
    const head=el('div','nx-md-tags');head.append(licenceBadge(m));
    if(m.motion)head.append(el('span','nx-md-badge','Moves with the printer'));
    body.append(head);
    body.append(section('Licence',note(m.licence.text||'Not stated by the author.'),note(m.licence.words),
      m.licence.author?note('By '+m.licence.author):el('span'),
      m.licence.source?(()=>{const a=el('a','nx-md-link',m.licence.source);a.href=m.licence.source;a.target='_blank';a.rel='noopener noreferrer';return a;})():el('span')));
    const facts=[m.size_mm?m.size_mm+' mm bed':'',tri(m.triangles),m.parts?m.parts+' parts':''].filter(Boolean).join(' · ');
    if(facts)body.append(note(facts));
    if(m.note)body.append(note(m.note));
    const fits=el('div','nx-md-fits');body.append(fits);
    fetch('/api/printer-models/'+m.id+'/options',{cache:'no-store'}).then(r=>r.json()).then(d=>{
      if(!d.options?.length){fits.append(note('No toolchanger, toolhead or add-on is registered for this machine.'));return;}
      for(const k of ['toolchanger','toolhead','addon']){
        const rows=d.options.filter(o=>o.kind===k);if(!rows.length)continue;
        const list=el('div','nx-md-opts');
        for(const o of rows){const r=el('div','nx-md-opt');const nm=el('div','nx-md-optname');nm.append(el('span','nx-md-name',o.label));const tags=el('div','nx-md-tags');tags.append(licenceBadge(o));
          if(o.drawable){const x=el('span','nx-md-badge','Drawn on the printer');x.dataset.class='installed';tags.append(x);}
          if(o.installed){const x=el('span','nx-md-badge','Installed');x.dataset.class='installed';tags.append(x);}
          nm.append(tags);r.append(nm,button({icon:'i-view',label:'Details',size:'row',title:o.label,does:'Shows the licence and the files of this option.',onPress:()=>optionDetail(o,{token,onChange:()=>onChange()})}));list.append(r);}
        fits.append(section(KIND[k]+' that fit this machine ('+rows.length+')',list));}
    }).catch(()=>{});
    const box=el('div','nx-md-install');body.append(box);
    const refresh=async()=>{
      const cur=await fetch('/api/printer-models/'+m.id,{cache:'no-store'}).then(r=>r.json());
      Object.assign(m,cur);installUi(box,m,{token,printer,onChange:()=>{onChange();refresh();},close});};
    refresh().catch(e=>box.append(note(e.message)));
  }});
}

function installUi(box,m,{token,printer,onChange,close}){
  box.replaceChildren();
  const msg=note('');msg.setAttribute('role','status');
  if(m.record){
    box.append(section(m.in_build?'Inside Stella Scope':'Installed',note(m.in_build
      ?`${mb(m.record.bytes)}. This model ships inside Stella Scope, put in your library when Stella Scope first started and checked against the author's checksum. It cannot be removed here. Who made it, and the terms it is shared under, are in the notice below.`
      :`${mb(m.record.bytes)} kept on this computer, checked against the author's checksum. From ${m.record.origin}.`)));
    const row=el('div','nx-md-btns');
    if(m.in_build)row.append(button({icon:'i-view',label:'Who made it, and the terms',size:'row',title:'The maker, the licence and what was done to the file',does:'Shows the notice that goes with this model: the maker, the licence it is under, what was done to the file, and the licence text.',onPress:()=>showNotice(m)}));
    row.append(button({icon:'i-view',label:'Look at it',size:'row',title:'Look at the model',does:'Shows the model on its own, shown as built.',onPress:()=>previewModel(m)}));
    if(printer)row.append(button({icon:'i-check',label:`Use for ${printer.name||printer.id}`,kind:'primary',size:'row',title:'Use this model',does:'Binds this model to the printer.',onPress:async()=>{try{await call('PATCH','/api/printers/'+printer.id,{cad_model:m.id},token);toast('Model set.','ok');close();}catch(e){msg.textContent=e.message;}}}));
    if(token&&!m.in_build){const rm=button({icon:'i-close',label:'Remove from this computer',kind:'danger',size:'row',title:'Remove',does:'Deletes the downloaded file. The catalogue entry stays.',onPress:()=>{
      row.replaceChildren(note('Remove the downloaded file? You can download it again.'),
        button({icon:'i-close',label:'Yes, remove it',kind:'danger',size:'row',title:'Remove',does:'Deletes the file.',onPress:async()=>{try{await call('DELETE','/api/printer-models/'+m.id,undefined,token);toast('Removed.','ok');onChange();}catch(e){msg.textContent=e.message;}}}),
        button({icon:'i-back',label:'Keep it',size:'row',title:'Keep',does:'Leaves the file.',onPress:()=>installUi(box,m,{token,printer,onChange,close})}));}});row.append(rm);}
    box.append(row,msg);return;}
  if(!m.has_file){box.append(note('The author has not published a model file for this machine, so there is nothing to install. Stella Scope does not draw one in its place.'));return;}
  if(!token){box.append(note('Models are installed from the Stella Scope window on the computer.'));return;}
  // Polish WO-05 (2026-10-18): the licence first, then two plain buttons (From the author, From a folder on this computer...); no typing the word "published".
  // The third way, a file the person downloaded, stays: it is checked against the same pin.
  const file=el('input');file.type='file';file.accept='.gz,.glb,.glb.gz';file.hidden=true;
  const fileBtn=button({icon:'i-files',label:'Install a file I downloaded',size:'row',title:'Install a file',does:'Takes the model file you downloaded from the author and checks it against the published checksum.',onPress:()=>{if(!box.accepted()){box.say('Tick the box to say you have read the licence.');return;}file.click();}});
  file.addEventListener('change',async()=>{
    const f=file.files[0];if(!f)return;busy(fileBtn,true);box.say(`Checking ${f.name}.`);
    try{const r=await fetch(`/api/printer-models/${m.id}/upload?accept=${box.accepted()?1:0}&name=${encodeURIComponent(f.name)}`,{method:'POST',headers:{'Content-Type':'application/octet-stream','X-Stella-Control-Token':token},body:f});
      const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);toast('Model installed.','ok');onChange();}
    catch(e){box.say(e.message);}finally{busy(fileBtn,false);file.value='';}});
  getFiles(box,{items:needs(m,[],[]).todo,token,doneLabel:'Done',done:async()=>{toast('Model installed.','ok');onChange();},extra:[fileBtn,file]});
  if(m.from_author){const a=el('a','nx-md-link',m.from_author);a.href=m.from_author;a.target='_blank';a.rel='noopener noreferrer';box.append(note('Published by 3D Print Rig, from the author\'s files, at:'),a);}
}

// ---- the model on a printer ------------------------------------------------------------------------
const AX=['x','y','z'];
/** The stage, the loading veil, the view buttons: one model drawn into `host`. `view` is null when it
 *  could not be drawn (the reason is on the page). */
async function drawModel(host,{id,fileUrl,basis,motion,triangles,compact=false,setup=null}){
  const stage=el('div','nx-md-stage');const veil=el('div','nx-md-veil');veil.append(note('Loading the model…'));
  stage.append(veil);host.append(stage);
  let view=null,dead=false;
  const dispose=()=>{dead=true;view?.dispose();};
  try{
    const [{modelBuffer,createModelView},profile]=await Promise.all([import('./model-view.js'),motion?call('GET',`/api/printer-models/${id}/motion`):Promise.resolve(null)]);
    const buffer=await modelBuffer(fileUrl,{onProgress:f=>{if(veil.firstChild)veil.firstChild.textContent=`Loading the model… ${Math.round(f*100)} %`;}});
    if(dead)return {view:null,dispose};
    const phone=matchMedia('(max-width: 700px)').matches;
    view=await createModelView(stage,{buffer,basis,profile,simple:(triangles||0)>1500000||phone,onPick:p=>document.dispatchEvent(new CustomEvent('stella-part-pick',{detail:p}))});
    veil.remove();
  }catch(e){veil.replaceChildren(note('The model could not be drawn: '+e.message));return {view:null,dispose};}
  const tools=el('div','nx-md-tools');
  tools.append(viewCube(v=>view.view(v)));
  const simple=el('button','nx-md-chip','Simple view');simple.type='button';simple.setAttribute('aria-pressed',String(view.shown()<view.info.parts));
  simple.addEventListener('click',()=>{const on=simple.getAttribute('aria-pressed')!=='true';simple.setAttribute('aria-pressed',String(on));view.setSimple(on);});
  if(!compact)tools.append(simple);
  if(setup){const mb=button({icon:'i-more',label:'Model...',size:'row',title:'Model...',does:'Change the machine, what is fitted to it, or match the model to the printer.',onPress:setup});mb.setAttribute('aria-label','Model...');tools.append(mb);}
  host.append(tools);
  return {view,dispose};
}

/** The files of an option on their own (a changer system, a library part), at the places the project shows them. */
export function previewOption(o){
  let drawn=null;
  openModal({title:o.label,icon:'i-view',onClose:()=>drawn?.dispose(),build:body=>{
    body.closest('.nx-modal-card')?.classList.add('nx-md-card','nx-md-card--viewer');
    const host=el('div','nx-md-host');body.append(host);
    (async()=>{
      const stage=el('div','nx-md-stage'),veil=el('div','nx-md-veil');veil.append(note('Loading the files…'));stage.append(veil);host.append(stage);
      try{
        const [{createModelView},sc]=await Promise.all([import('./model-view.js'),call('GET',`/api/printer-models/options/${o.id}/scene`)]);
        if(sc.missing?.length){veil.replaceChildren(note(`${plural(sc.missing.length,'file')} of this option ${sc.missing.length===1?'is':'are'} not on this computer yet. Install the option first.`));return;}
        const view=await createModelView(stage,{buffer:null,basis:null,profile:null,simple:false});
        drawn={dispose:()=>view.dispose()};
        await view.setScene(sc,{onProgress:f=>{if(veil.firstChild)veil.firstChild.textContent=`Loading the files… ${Math.round(f*100)} %`;}});veil.remove();
        const tools=el('div','nx-md-tools');tools.append(viewCube(v=>view.view(v)));host.append(tools);
        host.append(note(`${o.licence.text||'No licence stated'}${o.licence.author?', by '+o.licence.author:''}. Shown as the project shows it; it is not fitted to a printer.`));
      }catch(e){veil.replaceChildren(note('The files could not be drawn: '+e.message));}
    })();
  }});
}

/** A model on its own, not following any printer. */
export function previewModel(m){
  let drawn=null;
  openModal({title:m.label,icon:'i-view',onClose:()=>drawn?.dispose(),build:body=>{
    body.closest('.nx-modal-card')?.classList.add('nx-md-card','nx-md-card--viewer');
    const host=el('div','nx-md-host');body.append(host);
    drawModel(host,{id:m.id,fileUrl:`/api/printer-models/${m.id}/file`,basis:m.basis,motion:false,triangles:m.triangles}).then(d=>{drawn=d;
      host.append(note(`${m.licence.text||'No licence stated'}${m.licence.author?', by '+m.licence.author:''}. Shown as built; bind it to a printer to watch it follow that printer.`));});
  }});
}

/** Draws printer `id`'s model into `host`, following its reported position. Returns {dispose}. */
export async function mountModel(host,{id,token=null,compact=false,printer=null,setup=null,leave=null,onBound=()=>{}}){
  const api=id?`/api/p/${id}`:'/api';
  host.replaceChildren();
  let st;
  try{st=await call('GET',api+'/model');}catch(e){host.append(note(e.message));return {dispose(){}};}
  if(!st.model){
    host.append(note(st.reason||'This printer has no model.'));
    if(token&&printer)host.append(button({icon:'i-printers',label:'Choose a model',kind:'primary',size:'row',title:'Choose a model',does:'Lists the real CAD that exists. If yours is not there, the printer stays without one.',onPress:()=>modelsSheet({token,pick:printer,onPicked:onBound})}));
    return {dispose(){}};}
  if(!st.installed){
    host.append(note(`${st.view.label} is chosen for this printer but is not installed yet.`));
    host.append(button({icon:'i-view',label:'Licence and install',kind:'primary',size:'row',title:'Install',does:'Shows the licence and how to get this model.',onPress:()=>modelDetail(st.view,{token,printer,onChange:()=>mountModel(host,{id,token,compact,printer,onBound})})}));
    return {dispose(){}};}
  let timer=0,dead=false,matching=false,run=null,calUpdate=null;
  const readout=el('div','nx-md-read');
  // Polish WO-05: in the model dialog the machine is named, with the way to change it (and what is fitted) right there.
  if(!compact&&token&&printer)host.append(machineBar(st,{token,printer,onBound,leave}));
  const drawn=await drawModel(host,{id:st.model,fileUrl:st.file_url,basis:st.basis,motion:st.motion,triangles:st.view.triangles,compact,setup});
  const dispose=()=>{dead=true;clearInterval(timer);run?.stop();drawn.dispose();};
  if(!drawn.view)return {dispose};
  const view=drawn.view;
  // Polish WO-03: the part grows on the plate as the printer reads its file, and the head glides between reports.
  run=(await import('./model-run.js')).runModel(view,{prefix:api,budget:innerWidth<700?150000:600000});
  if(compact)machineChips(host,view);
  host.append(readout);
  if(st.note)host.append(note(st.note));
  // Polish WO-01: the toolchanger, toolhead and add-on options chosen for this model, drawn at the places 3D Print Rig registers.
  const sceneBox=el('div','nx-md-scene');host.append(sceneBox);
  if(st.scene){
    const left=st.scene.missing||[];
    const placed=o=>{const ps=(st.scene.placements||[]).filter(p=>p.option===o.option);return ps.length?ps.every(p=>p.installed!==false):!!o.installed;};
    if(left.length){sceneBox.append(note(`${plural(left.length,'file')} of the chosen options ${left.length===1?'is':'are'} not installed yet, so ${left.length===1?'it is':'they are'} not drawn.`));
      const first=(st.scene.options||[]).find(o=>!placed(o));
      // 2026-10-18 (polish WO-03): when every missing file is one the farm's own "Get the 3D models" fetches (open licences), that is the button here too, so the demo toolchanger needs no non-commercial licence; any other missing file keeps the option's own install.
      const ask=el('div','nx-gm-ask');sceneBox.append(ask);
      const optionBtn=()=>{if(first)ask.append(button({icon:'i-view',label:'Licence and install',kind:'primary',size:'row',title:'Install the options',does:'Shows the licence and downloads the files of the chosen options.',onPress:async()=>{try{optionDetail(await call('GET','/api/printer-models/options/'+first.option),{token,onChange:()=>onBound()});}catch(e){toast(e.message,'alarm');}}}));};
      fetch('/api/printer-models/wanted',{cache:'no-store'}).then(r=>r.json()).then(w=>{const open=new Set((w.models||[]).filter(m=>m.open&&m.kind==='part').map(m=>m.id));if(left.length&&left.every(p=>open.has(p)))ask.append(modelsButton(token,()=>onBound(),w));else optionBtn();}).catch(optionBtn);}
    view.setScene(st.scene).then(r=>{if(r.placed)sceneBox.append(note(`Drawn: ${(st.scene.options||[]).filter(placed).map(o=>o.label).join(', ')}.${r.replaced?` ${plural(r.replaced,'part')} of the machine ${r.replaced===1?'is':'are'} hidden because the options replace ${r.replaced===1?'it':'them'}.`:''}`));})
      .catch(e=>sceneBox.append(note('The options could not be drawn: '+e.message)));
  }
  if(!compact&&st.view.licence.class!=='gpl'&&st.view.licence.class!=='mit')host.append(note(`${st.view.licence.text||'No licence stated'}${st.view.licence.author?', by '+st.view.licence.author:''}.`));

  const fmt=v=>typeof v==='number'?v.toFixed(1):'—';
  // 2026-10-18 (polish WO-03): plain words under the views, no numbers that could differ from the position readout (the
  // service's own note says the model does not move, once). The two sets of numbers, with their reason, are in the calibration panel.
  const words=s=>{
    if(!s.pose)return '';
    const out=[s.recorded?'Rewound: as it was then.':s.position_source==='live'?'Live from the printer.':'Following the file: the head is where the print file says it is, not measured by the printer.'];
    if(s.job?.file&&!s.print_frame)out.push('The plate of this model has not been measured, so the part is not drawn on it.');
    else if(run.error&&s.job?.file)out.push('The part is not drawn: '+run.error);
    if(s.pose.clamped?.length)out.push(`${s.pose.clamped.join(', ').toUpperCase()} is at the end of the model's travel.`);
    if(s.tick_age_s>10)out.push('The printer is not answering.');
    return out.join(' ');
  };
  const paint=s=>{
    readout.textContent=words(s);readout.hidden=!readout.textContent;
    readout.dataset.stale=s.tick_age_s>10?'true':'false';
    run.update(s);
    calUpdate?.(s);
  };
  paint(st);
  // A model with no registered movement still has its print growing on the plate, so it is asked after as well (polish WO-03).
  // finish-2 F02: a rewound page (its address carries #ts=) shows the model as the recorder had it then, not the live pose
  const at=()=>{const t=Number(new URLSearchParams(location.hash.slice(1)).get('ts'));return Number.isFinite(t)&&t>0?'&ts='+t:'';};
  if(st.motion||st.print_frame){timer=setInterval(async()=>{if(document.hidden||dead)return;try{paint(await call('GET',api+'/model?scene=0'+at()));}catch{}},500);}

  if(!compact&&token&&st.motion){const cal=calibration(st,{id,token,view,setMatching:v=>{matching=v;run.pause(v);host.classList.toggle('nx-md-matching',v);},paint,printer});calUpdate=cal.update;host.append(cal);}
  else if(!compact&&st.motion&&!token)host.append(note('Calibration is set from the Stella Scope window on the computer.'));
  if(!compact&&(st.view.fits?.length||st.options?.length)){const op=el('div','nx-md-optbox');host.append(op);optionsPanel(op,{st,id,token,onBound}).catch(e=>op.append(note('The options could not be listed: '+e.message)));}
  return {dispose,view};
}

// Every way of saving a printer's model tells the page, so the 3D view on the page updates in place (polish WO-05, 2026-10-18; it
// used to reload the whole window).
const modelChanged=id=>{try{window.dispatchEvent(new CustomEvent('stella-model-changed',{detail:{id}}));}catch{}};

/** The machine this model is, and the way to change it or what is fitted (polish WO-05). */
function machineBar(st,{token,printer,onBound,leave=null}){
  const bar=el('div','nx-md-machine'),mid=el('div','nx-md-mid'),tags=el('div','nx-md-tags');tags.append(licenceBadge(st.view));
  mid.append(el('div','nx-md-name',plainLabel(st.view.label)),tags);
  bar.append(mid,button({icon:'i-printers',label:'Change machine or add-ons',size:'row',title:'Change machine or add-ons',does:'Chooses another machine, or what is fitted to this one. The model updates when you are done.',onPress:()=>{leave?.();modelsSheet({token,pick:{...printer,cad_model:st.model,cad_options:st.options},onPicked:onBound});}}));
  return bar;}

/** The toolchanger and add-ons fitted to a printer's model (chosen here after the machine is set). Same chooser and same files step
 *  as when the machine was chosen; saved with the printer and the model is drawn again. */
async function optionsPanel(box,{st,id,token,onBound}){
  const d=await optionsOf(st.model);
  const opts=drawable(d.options);
  const wrap=el('details','nx-md-cal');wrap.open=!!(st.options&&st.options.length);
  wrap.append(el('summary','','Toolchanger and add-ons'));box.append(wrap);
  if(!opts.length){wrap.append(note('This machine has no toolchanger or add-on to draw.'));return;}
  if(!token){wrap.append(note('Options are chosen from the Stella Scope window on the computer.'));return;}
  wrap.append(note('Tick what is fitted to this printer. Each is drawn where 3D Print Rig registers it for this machine.'));
  const kept=new Set(),choose=el('div','nx-md-optbox'),bar=el('div','nx-md-install');wrap.append(choose,bar);
  let ch=null;
  const save=async()=>{const sel=ch.value();await call('PATCH','/api/printers/'+id,{cad_options:sel},token);toast('Options saved.','ok');modelChanged(st.id);onBound();};
  const paintBar=()=>{const n=needs(null,d.options,ch.value());n.todo=n.todo.filter(i=>!kept.has(i.kind+i.id));getFiles(bar,{items:n.todo,token,got:kept,done:save,doneLabel:'Fit these'});};
  ch=optionChooser(choose,{options:opts,heads:d.options.filter(o=>o.kind==='toolhead').length,current:st.options||[],onChange:paintBar});paintBar();
}

/** Match the model to the printer, in two plain steps (polish WO-05, 2026-10-18): move the model's nozzle to where the printer's is,
 *  or, for a printer that reports X and Y and a model whose bed corner is known, one press: use the bed corner. The offsets, the
 *  direction of each axis and which printer axis each follows are under "Set it by hand". */
function calibration(st,{id,token,view,setMatching,paint}){
  const api=`/api/p/${id}`,fmt=v=>typeof v==='number'?v.toFixed(1):'—';
  const cal=structuredClone(st.calibration),reports=st.position?.x!=null&&st.position?.y!=null;
  const wrap=el('details','nx-md-cal');wrap.open=!st.calibrated;
  wrap.append(el('summary','','Calibration'));
  wrap.append(note(reports
    ?'The model has to sit where the printer does. Move the model\'s nozzle to where the printer\'s nozzle is, then save.'+(st.bed_corner?' If X 0, Y 0 on this printer is the corner of its bed, one press does it: Use the bed corner.':'')
    :'The printer is not reporting where its head is right now, so there is nothing to match yet. Switch it on, then come back here.'));
  // The two sets of numbers (polish WO-03): where the printer says the head is, and where that puts the model on its own axes.
  // They are different things (a printer's coordinates and a model's axes), and a moment apart, so they live here, said so.
  const nums=el('p','nx-md-read');nums.setAttribute('aria-live','off');wrap.append(nums);
  wrap.update=s=>{const p=s.position||{},a=s.pose?.axes;if(!a){nums.textContent='';return;}
   nums.textContent=`Printer reports X ${fmt(p.x)}  Y ${fmt(p.y)}  Z ${fmt(p.z)}${s.position_source==='live'?'':' (from the file, not measured)'}. The model is at X ${fmt(a.x)}  Y ${fmt(a.y)}  Z ${fmt(a.z)} on its own axes, a moment apart from that.`;};
  wrap.update(st);
  const msg=note('');msg.setAttribute('role','status');
  const save=async c=>{try{const r=await call('PATCH','/api/printers/'+id,{cad_calibration:c},token);toast('Calibration saved.','ok');msg.textContent='Saved.';return r;}catch(e){msg.textContent=e.message;return null;}};
  const refresh=()=>call('GET',api+'/model?scene=0').then(s=>{paint(s);return s;}).catch(()=>null);
  // the model's nozzle, moved by sliders; the service says which calibration puts it there
  const where=note('');
  const matchBox=el('div','nx-md-match');matchBox.hidden=true;matchBox.append(where);
  let pending=null,tm=0;
  const sliders=AX.map(a=>{
    const [lo,hi]=st.view.axes[a],s=el('input');s.type='range';s.min=String(lo);s.max=String(hi);s.step='0.5';s.value=String(st.pose?.axes?.[a]??lo);s.setAttribute('aria-label',`The model's nozzle on ${a.toUpperCase()}`);
    const lab=el('label','nx-md-slider');const v=el('span','nx-md-val',Number(s.value).toFixed(1)+' mm');lab.append(el('b','',a.toUpperCase()),s,v);
    s.addEventListener('input',()=>{v.textContent=Number(s.value).toFixed(1)+' mm';clearTimeout(tm);tm=setTimeout(async()=>{
      try{const r=await call('GET',`${api}/model/match?x=${sliders[0].value}&y=${sliders[1].value}&z=${sliders[2].value}`);pending=r.calibration;view.setGroups(r.pose.groups);msg.textContent='';}
      catch(e){msg.textContent=e.message;}},90);});
    matchBox.append(lab);return s;});
  const stopMatch=()=>{setMatching(false);matchBox.hidden=true;start.hidden=false;if(corner)corner.hidden=false;pending=null;};
  const start=button({icon:'i-move',label:'Move the model\'s nozzle...',size:'row',title:'Move the model\'s nozzle',does:'Pauses the live movement so you can slide the model\'s nozzle to where the printer\'s nozzle is right now.',onPress:async()=>{
    const s=await refresh();if(!s?.pose||s.position?.x==null){msg.textContent='The printer is not reporting a position right now, so there is nothing to match.';return;}
    setMatching(true);matchBox.hidden=false;start.hidden=true;if(corner)corner.hidden=true;
    where.textContent=`The printer's nozzle is at X ${fmt(s.position.x)}  Y ${fmt(s.position.y)}  Z ${fmt(s.position.z)} now. Slide the model's nozzle to the same place.`;
    AX.forEach((a,i)=>{sliders[i].value=String(s.pose.axes[a]);sliders[i].dispatchEvent(new Event('input'));});}});
  const keep=button({icon:'i-check',label:'This is where it is',kind:'primary',size:'row',title:'Save the match',does:'Saves a calibration that puts the model where you slid it, for the position the printer reports now.',onPress:async b=>{
    if(!pending){msg.textContent='Slide the model first.';return;}busy(b,true);try{const r=await save(pending);if(r)fill(r.printer.cad_calibration);stopMatch();await refresh();}finally{busy(b,false);}}});
  const cancel=button({icon:'i-back',label:'Cancel',size:'row',title:'Cancel',does:'Goes back to the live movement.',onPress:()=>{stopMatch();refresh();}});
  const mrow=el('div','nx-md-btns');mrow.append(keep,cancel);matchBox.append(mrow);
  // one press: the printer's X 0, Y 0 is the corner of its bed, so the model's nozzle goes to the corner of the model's bed
  const corner=st.bed_corner&&reports?button({icon:'i-pin',label:'Use the bed corner',kind:'primary',size:'row',title:'Use the bed corner',does:'Puts the model\'s nozzle at the corner of its bed when the printer says X 0, Y 0. Where that is comes from the machine\'s own profile, not a guess.',onPress:async b=>{
    busy(b,true);try{const r=await save(st.bed_corner.calibration);if(r){fill(r.printer.cad_calibration);msg.textContent='Saved. The model\'s nozzle is at the corner of its bed when the printer says X 0, Y 0.';await refresh();}}finally{busy(b,false);}}}):null;
  const quick=el('div','nx-md-btns');quick.append(...[corner,start].filter(Boolean));
  // by hand
  const hand=el('details','nx-md-hand'),axes=el('div','nx-md-axes'),offs=[],revs=[],srcs=[];
  hand.append(el('summary','','Set it by hand'),note('Each line is one axis of the model: its offset in millimetres, whether it runs the other way, and which of the printer\'s axes it follows.'));
  AX.forEach((a,i)=>{
    const r=el('div','nx-md-axis');r.append(el('b','',a.toUpperCase()));
    const off=el('input','nx-input');off.type='number';off.step='0.5';off.setAttribute('aria-label',`${a.toUpperCase()} offset in millimetres`);
    const rev=el('button','nx-md-chip','Reverse');rev.type='button';rev.addEventListener('click',()=>{rev.setAttribute('aria-pressed',String(rev.getAttribute('aria-pressed')!=='true'));});
    const src=choice(AX.map((b,j)=>({value:j,label:'follows printer '+b.toUpperCase()})),cal.source[i],null,{label:`${a.toUpperCase()} follows the printer's`});
    offs.push(off);revs.push(rev);srcs.push(src);r.append(off,rev,src);axes.append(r);});
  const fill=c=>{AX.forEach((a,i)=>{offs[i].value=String(Math.round(c.offset_mm[i]*100)/100);revs[i].setAttribute('aria-pressed',String(c.sign[i]<0));srcs[i].set(c.source[i]);});};fill(cal);
  const current=()=>({offset_mm:offs.map(o=>Number(o.value)||0),sign:revs.map(r=>r.getAttribute('aria-pressed')==='true'?-1:1),source:srcs.map(s=>Number(s.get()))});
  const apply=button({icon:'i-check',label:'Save these',kind:'primary',size:'row',title:'Save',does:'Keeps these offsets, directions and axes for this printer.',onPress:async b=>{busy(b,true);try{await save(current());await refresh();}finally{busy(b,false);}}});
  const reset=button({icon:'i-restart',label:'Start again',size:'row',title:'Start again',does:'Goes back to the starting calibration for this model.',onPress:async b=>{busy(b,true);try{if(!await save(null))return;const s=await refresh();if(s?.calibration)fill(s.calibration);}finally{busy(b,false);}}});
  const btns=el('div','nx-md-btns');btns.append(apply,reset);
  hand.append(axes,btns);
  if(st.bed_corner?.basis)hand.append(note('Where the bed corner comes from: '+st.bed_corner.basis));
  wrap.append(quick,matchBox,hand,msg);
  return wrap;
}

/** A printer's own model viewer, in a dialog. `onChange` runs when the dialog closes after the machine or what is fitted changed. */
export function modelViewer({printer,token=null,onChange=()=>{}}){
  let mounted=null,changed=false;
  openModal({title:`${printer.name||printer.id}: model`,icon:'i-printers',onClose:()=>{mounted?.dispose();if(changed)onChange();},build:(body,close)=>{
    body.closest('.nx-modal-card')?.classList.add('nx-md-card','nx-md-card--viewer');
    const host=el('div','nx-md-host');body.append(host);
    const onBound=()=>{changed=true;modelChanged(printer.id);if(!host.isConnected)return;   // the dialog was closed to choose another machine: the page's own stage updates through the event
      mounted?.dispose();mountModel(host,{id:printer.id,token,printer,leave:close,onBound}).then(m=>{mounted=m;});};
    mountModel(host,{id:printer.id,token,printer,leave:close,onBound}).then(m=>{mounted=m;});
  }});
}

/** The model section of a printer's settings. */
export function modelSection(p,token,changed,leave=null){
  const box=el('div','nx-md-setting');
  const name=el('div','nx-md-cur',p.cad_model?'':'No model');
  const tags=el('div','nx-md-tags'),fitted=note('');fitted.hidden=true;
  box.append(name,tags,fitted);
  if(p.cad_model){
    fetch('/api/printer-models/'+p.cad_model,{cache:'no-store'}).then(r=>r.json()).then(m=>{
      name.textContent=plainLabel(m.label)||p.cad_model;tags.append(licenceBadge(m));
      if(m.installed){const t=el('span','nx-md-badge','Installed');t.dataset.class='installed';tags.append(t);}
      else{const t=el('span','nx-md-badge','Not installed');t.dataset.class='none';tags.append(t);}
    }).catch(()=>{name.textContent=p.cad_model;});
    if(p.cad_options?.length)optionsFetch().then(d=>{const by=new Map(d.options.map(o=>[o.id,o.label]));fitted.textContent='Fitted: '+p.cad_options.map(c=>plainLabel(by.get(c.option)||c.option)).join(', ')+'.';fitted.hidden=false;}).catch(()=>{});
  }else box.append(note('This machine has no 3D model in the catalogue yet, so none is drawn rather than the wrong one.'));
  const row=el('div','nx-md-btns');
  row.append(button({icon:'i-printers',label:p.cad_model?'Change model':'Choose a model',size:'row',title:p.cad_model?'Change model':'Choose a model',does:'Lists the real CAD that exists, with each licence, then what is fitted to the machine.',onPress:()=>{leave?.();modelsSheet({token,pick:p,onPicked:changed});}}));
  if(p.cad_model){
    row.append(button({icon:'i-view',label:'Open the model',size:'row',title:'Open the model',does:'Shows the model following this printer\'s position.',onPress:()=>modelViewer({printer:p,token})}));
    row.append(button({icon:'i-close',label:'No model',kind:'ghost',size:'row',title:'Remove the model from this printer',does:'Unbinds the model. The downloaded file stays in the library.',onPress:async()=>{try{await call('PATCH','/api/printers/'+p.id,{cad_model:null},token);toast('Model removed from this printer.','ok');modelChanged(p.id);changed();}catch(e){toast(e.message,'alarm');}}}));
  }
  box.append(row);
  return section('Model',box);
}

// 2026-10-25: "instead of Iso, Front, Top and Side ... an actual cube we can choose from". A view cube, as CAD programs have it: each face you can
// see turns the camera to that side, the corner they share turns it back to the three-quarter view. The face being looked from is lit.
export function viewCube(onView){const NS='http://www.w3.org/2000/svg',mk=(t,a={})=>{const e=document.createElementNS(NS,t);for(const[k,v]of Object.entries(a))e.setAttribute(k,v);return e;};
 const s=mk('svg',{viewBox:'14 9 72 82',class:'nx-cube',role:'group','aria-label':'View'}),P=pts=>pts.map(p=>p.join(',')).join(' ');
 const c=[50,50],t=[50,13],l=[18,31],r=[82,31],bl=[18,69],br=[82,69],b=[50,87],nodes={};
 const pick=v=>{for(const [k,n] of Object.entries(nodes))n.toggleAttribute('data-on',k===v);onView(v);};
 for(const [k,word,pts,at] of [['top','TOP',[t,r,c,l],[50,30]],['front','FRONT',[l,c,b,bl],[34,63]],['right','RIGHT',[c,r,br,b],[66,63]]]){
  const g=mk('g',{class:'nx-cube-f',role:'button',tabindex:'0','aria-label':'Look from the '+word.toLowerCase()});g.dataset.k=k;
  // the word lies on its face: the two sides are sheared like the faces, the top is laid flat
  const tx=mk('text',{x:0,y:0,transform:`translate(${at[0]} ${at[1]}) `+(k==='front'?'skewY(30)':k==='right'?'skewY(-30)':'scale(1 .62)')});tx.textContent=word;g.append(mk('polygon',{points:P(pts)}),tx);
  g.addEventListener('click',()=>pick(k));g.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();pick(k);}});s.append(g);nodes[k]=g;}
 const iso=mk('circle',{cx:c[0],cy:c[1],r:4.5,class:'nx-cube-iso',role:'button',tabindex:'0','aria-label':'Three-quarter view'});
 iso.addEventListener('click',()=>pick('iso'));iso.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();pick('iso');}});s.append(iso);nodes.iso=iso;iso.toggleAttribute('data-on',true);
 const wrap=el('div','nx-cube-wrap');wrap.append(s);tip(wrap,'View','Press a face of the cube to look from that side. The dot where the faces meet turns back to the three-quarter view.');return wrap;}

// 2026-10-25 (the research: "live numbers sit on the machine", Bambu's chips on the printer). On the printer page's model, a small chip at the nozzle
// and one at the bed, following them as they move; the big gauges stay the instruments. Off in Look (data-chips=off). A chip is only drawn where the model
// has that part (a bed that never moves has no group of its own, so no chip).
function machineChips(host,view){const layer=el('div','nx-mchips');host.append(layer);const chip=(k,ico)=>{const c=el('div','nx-chip-m');c.dataset.k=k;c.append(icon(ico),el('b','','—'));c.hidden=true;layer.append(c);return c;};
 const noz=chip('nozzle','i-heat'),bed=chip('bed','i-bed');let temps=null,raf=0,last=0;
 const fmt=t=>!t||!Number.isFinite(t.temp)?'—':Math.round(t.temp)+'°'+(Number.isFinite(t.target)&&t.target>0&&Math.abs(t.target-t.temp)>2?' → '+Math.round(t.target)+'°':'');
 const hot=t=>t&&Number.isFinite(t.target)&&t.target>0;
 const place=(c,a)=>{if(!a){c.hidden=true;return;}const r=layer.getBoundingClientRect();c.hidden=false;c.style.transform='translate('+Math.round(a.x-r.left)+'px,'+Math.round(a.y-r.top)+'px)';};
 const tick=t=>{raf=requestAnimationFrame(tick);if(t-last<100||!layer.isConnected)return;last=t;if(!temps){noz.hidden=bed.hidden=true;return;}
  const an=view.anchor('head','bottom'),ab=view.anchor('bed','top');place(noz,an);place(bed,ab);
  // two readings that would sit on each other (a bed raised to the nozzle): the bed's opens to the left of its point
  bed.toggleAttribute('data-left',!!(an&&ab&&Math.hypot(an.x-ab.x,an.y-ab.y)<140));
  for(const [c,k] of [[noz,'nozzle'],[bed,'bed']]){const v=fmt(temps[k]);if(c.lastChild.textContent!==v)c.lastChild.textContent=v;c.toggleAttribute('data-hot',hot(temps[k]));}};
 document.addEventListener('stella-temps',e=>{temps=e.detail;});raf=requestAnimationFrame(tick);
 new MutationObserver(()=>{if(!layer.isConnected){cancelAnimationFrame(raf);}}).observe(host,{childList:true});}

/** On a printer's own page: when it has a model, the stage card gets a Model tab beside the camera; a farm
 *  printer with no camera and no model says so, plainly, in place of the note about the 3D view not loading.
 *  Polish WO-05: choosing or changing the model (here, in Settings, in the model dialog) rebuilds the card in place, no reload. */
export async function stageTabs(card,{id,noClassic=false}){
  const base=id?'/api/p/'+id:'/api';
  let st;try{st=await call('GET',base+'/model');}catch{return;}
  const orig=[...card.childNodes],hasCam=!!card.querySelector('img');
  let mounted=null;
  const info=await fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).catch(()=>({}));
  const me=(info.printers||[]).find(p=>p.id===st.id),can=!!(info.writable&&info.token&&me);
  const rebuild=()=>{clearTimeout(tm);window.removeEventListener('stella-model-changed',onChange);mounted?.dispose();mounted=null;card.replaceChildren(...orig);return stageTabs(card,{id});};
  let tm=0;const onChange=e=>{if(e.detail?.id===st.id){clearTimeout(tm);tm=setTimeout(rebuild,250);}};   // one rebuild for the few events a save sends
  window.addEventListener('stella-model-changed',onChange);
  if(!st.model){
    // the first printer keeps its own 3D view; a camera is a fine card. F06 (2026-10-19): the first printer of a farm that has no classic model to show (a fresh install,
    // whose first printer is a Klipper printer with no model chosen) said "The 3D printer is not loaded on this device (the model is over 100 MB)" and offered a button that
    // reloaded the page to the same words: with nothing classic to show it gets the same empty stage as every other printer, with the one button that chooses a model.
    if((st.default&&!noClassic)||hasCam){window.removeEventListener('stella-model-changed',onChange);return;}
    // an empty stage says what it is for in plain words, with the one button (polish WO-05, 2026-10-18)
    const empty=el('div','nx-md-empty');empty.append(icon('i-printers','nx-ico nx-md-ph'),el('b','','No 3D model yet'),note(can?'Pick your machine from the catalogue and it is drawn here, moving as the printer moves. If yours is not in it, this space stays empty rather than showing the wrong printer.':st.reason||'This printer has no model.'));
    if(can)empty.append(button({icon:'i-printers',label:'Choose a model',kind:'primary',size:'row',title:'Choose a model',does:'Lists the real CAD that exists, with each licence, then what is fitted to the machine.',onPress:()=>modelsSheet({token:info.token,pick:me,onPicked:()=>{}})}));
    card.replaceChildren(empty);return;}
  // The card holds the camera picture, or a note about the 3D printer not being loaded; the note is not a camera.
  const cam=el('div','nx-md-cam');
  if(hasCam)cam.append(...card.childNodes);
  const holder=el('div','nx-md-host nx-md-host--stage');
  // finish-2 F02: the Model tab gets the token and the printer, else its Licence and install said "installed from the window
  // on the computer" in that very window and nothing could be installed from the printer page.
  const show=async v=>{
    cam.hidden=v!=='camera';holder.hidden=v!=='model';
    if(v==='model'&&!mounted){mounted=await mountModel(holder,{id,compact:true,token:can?info.token:null,printer:can?me:null,setup:can?()=>modelViewer({printer:me,token:info.token}):null});}
    if(v!=='model'&&mounted){mounted.dispose();mounted=null;holder.replaceChildren();}};
  card.replaceChildren();
  // 2026-10-25 ("Camera and model, I don't get"): no Camera | Model tabs. The stage is the model; the camera is the top bar's camera key,
  // which fills the whole screen with it. A model that cannot be drawn here gives the stage back to the camera.
  if(hasCam)card.append(cam,holder);else card.append(holder);
  await show('model');
  if(hasCam&&!mounted?.view)await show('camera');
}
