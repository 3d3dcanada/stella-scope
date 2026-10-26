// The farm's floor (2026-10-13: "better than Watchtower, by far ... the pop-ups the same way, clean,
// pro"): in-app pop-ups for every farm event, the alarm feed, every camera on one screen, spools, analytics
// and the farm backup. Our own look (NEXT UI, docs/next-ui/BLUEPRINT.md) and words. Every figure is read
// from the service, which reads it from the printers' own recordings: nothing here is estimated.
import {el,icon,button,busy,toast,openSheet,section,numberField,numberInput,led,emerge,grow,fold} from './next-ui.js';
import {raiseAlarm} from './next-alarm.js';

const $=id=>document.getElementById(id);
const SEV={alarm:['Alarm','i-warn'],warn:['Warning','i-warn'],ok:['Done','i-check'],info:['Note','i-bell']};
const KIND={done:'Print ended',plate_clear:'Plate to clear',plate_failed:'Plate check',offline:'Offline',online:'Back online',failure_watch:'Failure watch',attention:'Needs attention',queue_empty:'Queue empty',spool_low:'Spool low',test:'Test'};
const hhmm=ts=>{const d=new Date(ts*1000),t=d.toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'});return d.toDateString()===new Date().toDateString()?t:d.toLocaleDateString([],{month:'short',day:'numeric'})+' '+t;};
const since=ts=>{const s=Date.now()/1000-ts;return s<45?'now':s<3600?Math.round(s/60)+' min ago':s<86400?Math.round(s/3600)+' h ago':hhmm(ts);};
const kg=g=>g>=1000?(g/1000).toFixed(g>=10000?0:1)+' kg':Math.round(g)+' g';
const money=v=>typeof v==='number'?'$'+v.toFixed(v>=100?0:2):'—';
const hrs=h=>h<1?Math.round(h*60)+' min':h<100?h.toFixed(1)+' h':Math.round(h)+' h';
const pctx=v=>typeof v==='number'?Math.round(v*100)+'%':'—';
async function printers(){try{return await (await fetch('/api/printers',{cache:'no-store'})).json();}catch{return {printers:[]};}}
async function write(method,url,body){const d=await printers();if(!d.token)throw Error('Changes are made from the Stella Scope window on the computer.');
 const r=await fetch(url,{method,headers:{'Content-Type':'application/json','X-Stella-Control-Token':d.token},body:body===undefined?undefined:JSON.stringify(body)});
 const j=await r.json().catch(()=>({}));if(!r.ok)throw Error(j.error||`Stella Scope answered with an error (${r.status}).`);return j;}

// ---- pop-ups -------------------------------------------------------------------------------------
// One stack in the corner (the top on a phone). Each names the printer, says what happened, shows the
// print's picture when there is one, and opens that printer on a tap. Alarms stay until closed; the rest
// leave by themselves. The bell keeps every one of them (the alarm feed).
let seq=0,unread=0,bell=null,known=new Map(),watchHook=null;
// U07 (2026-10-20): a pop-up grows out of the printer's tile (a printer's own page: its state chip, else the bell) and folds back into it
const homeOf=e=>document.querySelector(`.fm-tile[data-id="${CSS.escape(e.printer||'')}"]`)||document.getElementById('nx-state')||document.querySelector('.fl-bell');
const rectOf=n=>n&&n.isConnected&&n.getClientRects().length?n.getBoundingClientRect():null;
function popHost(){let h=$('fl-pops');if(!h){h=el('div','fl-pops');h.id='fl-pops';h.setAttribute('role','status');h.setAttribute('aria-live','polite');document.body.append(h);}return h;}
const plain=e=>{const n=e.name||'';return n&&e.text?.startsWith(n+': ')?e.text.slice(n.length+2):e.text;};
function pop(e){
 const host=popHost(),p=known.get(e.printer),[sevWord,ico]=SEV[e.severity]||SEV.info;
 const card=el('article','fl-pop nx-glass nx-glass--raised');card.dataset.sev=e.severity||'info';card.tabIndex=0;
 const pic=el('div','fl-pop-pic');
 // (polish WO-04: only where the printer can give one; the demo printer cannot, and its address answered 502 for every pop-up)
 if(p?.thumbnail_url&&p.capabilities?.thumbnails!==false){const im=el('img');im.alt='';im.src=p.thumbnail_url;im.addEventListener('error',()=>{im.replaceWith(icon(ico));});pic.append(im);}else pic.append(icon(ico));
 const txt=el('div','fl-pop-text'),top=el('div','fl-pop-top');
 top.append(el('b','',e.name||p?.name||'Stella Scope'),el('span','fl-pop-kind',KIND[e.kind]||sevWord),el('time','',since(e.at)));
 txt.append(top,el('p','',plain(e)));
 const x=button({icon:'i-close',size:'icon',kind:'ghost',title:'Close',does:'Closes this pop-up. It stays in Alerts.',onPress:ev=>{ev?.stopPropagation?.();gone();}});x.classList.add('fl-pop-x');
 x.addEventListener('click',ev=>ev.stopPropagation());
 card.append(pic,txt,x);
 const gone=()=>{if(card.dataset.out!==undefined)return;card.dataset.out='';fold(card,rectOf(homeOf(e)),()=>card.remove());};
 const go=()=>{if(e.printer&&known.has(e.printer))location.href='/p/'+e.printer;else{gone();sheetFeed();}};
 card.addEventListener('click',go);card.addEventListener('keydown',ev=>{if(ev.key==='Enter')go();if(ev.key==='Escape')gone();});
 host.prepend(card);grow(card,rectOf(homeOf(e)));
 // four at most on screen; the oldest calm ones go first
 const cards=[...host.querySelectorAll('.fl-pop:not([data-out])')];
 for(const c of cards.slice(4).reverse()){c.remove();}
 if(e.severity!=='alarm')setTimeout(gone,e.severity==='warn'?12000:8000);}
function badge(){if(!bell)return;bell.dataset.count=unread?String(Math.min(unread,99)):'';bell.toggleAttribute('data-unread',unread>0);}
async function poll(){
 if(document.hidden)return;
 let d;try{d=await (await fetch(`/api/alerts/feed?after=${seq}&limit=20`,{cache:'no-store'})).json();}catch{return;}
 if(!Array.isArray(d.events))return;
 if(seq===0){seq=d.last_seq||0;return;} // the page opened: what happened before is in the feed, not popped
 const fresh=d.events.filter(e=>e.seq>seq);seq=Math.max(seq,d.last_seq||0);
 if(!fresh.length)return;
 const pl=await printers();known=new Map((pl.printers||[]).map(p=>[p.id,p]));
 for(const e of fresh.slice(-4)){if(e.kind==='failure_watch')raiseAlarm(e,{known,onWatch:watchHook});else pop(e);if(e.severity==='alarm'||e.severity==='warn')unread++;}   // U07: the failure watch takes the screen; everything else is a pop-up
 badge();}
/** Start the pop-ups on a page. `bellHost` gets the bell button (the alarm feed) when given. */
export function startPops(bellHost,opts={}){
 if(opts.onWatch)watchHook=opts.onWatch;
 if(bellHost&&!bell){bell=button({icon:'i-bell',label:'Alerts',size:'row',title:'Alerts',does:'Every alert from every printer, newest first. A number on the bell counts the alerts since you last looked.',onPress:()=>{unread=0;badge();sheetFeed();}});bell.classList.add('fl-bell');bell.setAttribute('aria-label','Alerts');bellHost.append(bell);}
 poll();setInterval(poll,3000);document.addEventListener('visibilitychange',()=>{if(!document.hidden)poll();});}

// ---- the alarm feed ------------------------------------------------------------------------------
export function sheetFeed(){
 let sev='all',who='';
 openSheet({title:'Alerts',icon:'i-bell',wide:true,eyebrow:'Every printer',build:body=>{
  const bar=el('div','fl-filter'),list=el('div','fl-feed');
  const seg=el('div','nx-seg fl-seg');
  for(const [k,l] of [['all','All'],['alarm','Alarms'],['warn','Warnings'],['ok','Done'],['info','Notes']]){const b=el('button','',l);b.type='button';b.setAttribute('aria-pressed',String(k===sev));b.addEventListener('click',()=>{sev=k;for(const x of seg.children)x.setAttribute('aria-pressed',String(x===b));draw();});seg.append(b);}
  const pick=el('select','nx-input fl-who');pick.setAttribute('aria-label','Printer');pick.append(new Option('All printers',''));
  pick.addEventListener('change',()=>{who=pick.value;draw();});
  const settings=button({icon:'i-more',label:'Alert settings',size:'row',title:'Alert settings',does:'Where alerts go (WhatsApp, ntfy, Discord, Slack) and what they tell you about.',onPress:()=>import('./next-notify.js').then(m=>m.sheetAlerts())});
  settings.dataset.needsManage='';
  bar.append(seg,pick,settings);body.append(bar,list);
  printers().then(d=>{for(const p of d.printers||[])pick.append(new Option(p.name||p.id,p.id));settings.hidden=!d.token;});
  async function draw(){
   let d;try{d=await (await fetch(`/api/alerts/feed?limit=300${who?'&printer='+encodeURIComponent(who):''}`,{cache:'no-store'})).json();}catch{list.replaceChildren(el('p','nx-note','Stella Scope did not answer.'));return;}
   const rows=(d.events||[]).filter(e=>sev==='all'||e.severity===sev).reverse();
   list.replaceChildren();
   if(!rows.length){list.append(el('p','fl-empty',sev==='all'&&!who?'Nothing has happened yet. Every print ending, plate, spool, alarm and printer going on or off lands here.':'Nothing matches.'));return;}
   let day='';
   for(const e of rows){
    const dd=new Date(e.at*1000).toDateString();if(dd!==day){day=dd;list.append(el('h4','fl-day',dd===new Date().toDateString()?'Today':new Date(e.at*1000).toLocaleDateString([],{weekday:'long',month:'short',day:'numeric'})));}
    const r=el('button','fl-row');r.type='button';r.dataset.sev=e.severity||'info';
    const [,ico]=SEV[e.severity]||SEV.info;
    const t=el('div','fl-row-text');const top=el('div','fl-row-top');top.append(el('b','',e.name||'Stella Scope'),el('span','fl-pop-kind',KIND[e.kind]||e.kind));
    t.append(top,el('span','',plain(e)));
    r.append(el('i','fl-dot'),icon(ico),t,el('time','',hhmm(e.at)));
    if(e.sent?.length)r.title='Also sent to '+e.sent.join(' and ');
    if(e.printer)r.addEventListener('click',()=>{location.href='/p/'+e.printer;});else r.disabled=true;
    list.append(r);}}
  draw();}});}

// ---- every camera on one screen ------------------------------------------------------------------
// Browsers open about six connections to one host: the first four play, the rest show a picture every
// 3 s. A cell shows the latest picture at once and swaps to the stream when its first frame lands.
// Opened on demand only: no stream runs when this view is closed (Kobra X rule, 2026-10-12).
export function camerasView(){
 if($('fl-cams'))return;
 const root=el('div','fl-cams');root.id='fl-cams';root.setAttribute('role','dialog');root.setAttribute('aria-label','Cameras');
 const head=el('header','fl-cams-head nx-glass'),title=el('div','fl-cams-title');title.append(icon('i-grid'),el('b','','CAMERAS'),el('small','','LOADING'));
 const close=button({icon:'i-close',label:'Close',size:'row',title:'Close',does:'Back to the farm. Every stream stops.',onPress:()=>shut()});
 head.append(title,close);const grid=el('div','fl-cams-grid');root.append(head,grid);document.body.append(root);document.body.dataset.cams='';
 const em=emerge(root);   // U05: the view grows out of the Cameras key and folds back into it
 const cells=new Map(),hasCam=new Map();let timer=0,solo=null;
 const shut=()=>{clearInterval(timer);for(const c of cells.values())c.img.removeAttribute('src');delete document.body.dataset.cams;document.removeEventListener('keydown',esc);root.style.pointerEvents='none';em.back(()=>root.remove());};
 const esc=e=>{if(e.key==='Escape'){if(solo)unsolo();else shut();}};document.addEventListener('keydown',esc);
 // U05 (2026-10-20): a camera grows to fill the view from where it was (and the others fold back in) on the kit's spring, the way a tile does on the wall, instead of the layout jumping
 const flipCells=change=>{const before=new Map([...cells.values()].filter(c=>!c.cell.hidden).map(c=>[c,c.cell.getBoundingClientRect()]));change();
  if(matchMedia('(prefers-reduced-motion: reduce)').matches)return;
  for(const c of cells.values()){if(c.cell.hidden)continue;const a=before.get(c),b=c.cell.getBoundingClientRect();
   if(!a){c.cell.animate([{opacity:0,transform:'scale(.9)'},{opacity:1,transform:'none'}],{duration:420,easing:'cubic-bezier(.34,1.56,.64,1)'});continue;}
   c.cell.animate([{transformOrigin:'0 0',transform:`translate(${a.left-b.left}px,${a.top-b.top}px) scale(${a.width/b.width},${a.height/b.height})`},{transformOrigin:'0 0',transform:'none'}],{duration:520,easing:'cubic-bezier(.34,1.56,.64,1)'});}};
 const unsolo=()=>flipCells(()=>{solo=null;delete root.dataset.solo;for(const c of cells.values())c.cell.hidden=false;sizeGrid();allocate();});
 const sizeGrid=()=>{const n=solo?1:cells.size,w=innerWidth,phone=w<=600;const cols=phone?1:n<=1?1:n<=4?2:n<=9?3:4;grid.style.setProperty('--cols',cols);grid.style.setProperty('--rows',Math.ceil(n/cols));grid.dataset.fill=(!phone&&n<=12)?'1':'0';
  // three cameras: one large, two beside it, so no quarter of the screen is empty
  grid.dataset.n=String(n);};
 function live(c,on){if(c.on===on)return;c.on=on;c.cell.toggleAttribute('data-live',on);
  if(on){const s=new Image();s.onload=()=>{if(c.on)c.img.src=s.src;};s.src=`/api/p/${c.p.id}/camera/stream`;c.stream=s;}
  else{c.stream?.removeAttribute('src');c.stream=null;still(c);}}
 function still(c){if(c.on&&c.img.src.includes('/stream'))return;const im=new Image();im.onload=()=>{c.img.src=im.src;c.cell.dataset.ok='';};im.onerror=()=>{c.cell.dataset.err='';};im.src=`/api/p/${c.p.id}/camera/snapshot?t=${Date.now()}`;}
 function allocate(){let n=0;const max=innerWidth<=600?2:4;for(const c of cells.values()){const want=!c.cell.hidden&&c.p.state!=='offline'&&n<max;if(want)n++;live(c,want);}}
 function paint(c,p){c.p=p;const st=p.state;c.cell.dataset.state=st;c.name.textContent=p.name||p.id;
  const pc=typeof p.progress==='number'?Math.round(p.progress*100):null;
  c.state.textContent={printing:'Printing',heating:'Heating',paused:'Paused',finished:'Done',idle:'Ready',error:'Needs attention',offline:'Offline'}[st]||st;
  c.pct.textContent=['printing','paused','heating'].includes(st)&&pc!==null?pc+'%':'';
  c.file.textContent=p.file&&!['idle','offline'].includes(st)?p.file.replace(/\.(bgcode|gcode)(\.3mf)?$/i,'').replace(/\.3mf$/i,''):'';
  const t=p.temps||{},d=v=>typeof v==='number'?Math.round(v)+'°':'—';c.temps.textContent=`NOZZLE ${d(t.nozzle?.temp)}  BED ${d(t.bed?.temp)}`;
  c.bar.style.setProperty('--p',pc!==null&&['printing','paused'].includes(st)?String(pc/100):'0');}
 async function load(){
  const d=await printers();
  // a camera counts when the printer lists one (the same check the farm tiles make)
  const cand=(d.printers||[]).filter(p=>p.camera_url&&p.state!=='offline'||cells.has(p.id));
  const list=(await Promise.all(cand.map(async p=>{if(cells.has(p.id)||hasCam.get(p.id))return p;
   try{const j=await (await fetch(`/api/p/${p.id}/camera`,{cache:'no-store'})).json();const ok=(j.cameras||[]).length>0;hasCam.set(p.id,ok);return ok?p:null;}catch{return null;}}))).filter(Boolean);
  title.querySelector('small').textContent=`${list.length} ${list.length===1?'CAMERA':'CAMERAS'}`;
  if(!list.length&&!cells.size){grid.replaceChildren(el('p','fl-empty','No printer here has a camera. Add one in a printer\'s settings.'));return;}
  for(const p of list){let c=cells.get(p.id);
   if(!c){const cell=el('button','fl-cam');cell.type='button';cell.style.setProperty('--c',p.accent||'var(--accent)');
    const img=el('img');img.alt=`${p.name} camera`;const over=el('div','fl-cam-over'),top=el('div','fl-cam-top'),name=el('b'),state=el('span','fl-cam-state'),pct=el('span','fl-cam-pct');
    top.append(el('i','fl-cam-dot'),name,state);const bottom=el('div','fl-cam-bottom'),file=el('span','fl-cam-file'),temps=el('span','fl-cam-temps');bottom.append(file,pct,temps);
    const bar=el('i','fl-cam-bar');over.append(top,bottom);cell.append(img,over,bar);grid.append(cell);
    c={p,cell,img,name,state,pct,file,temps,bar,on:false};cells.set(p.id,c);
    cell.addEventListener('click',()=>{if(solo===p.id){location.href='/p/'+p.id;return;}flipCells(()=>{solo=p.id;root.dataset.solo='';for(const o of cells.values())o.cell.hidden=o!==c;sizeGrid();allocate();});});
    still(c);}
   paint(c,p);}
  sizeGrid();allocate();}
 load();timer=setInterval(()=>{if(document.hidden)return;load();for(const c of cells.values())if(!c.on)still(c);},3000);
 addEventListener('resize',sizeGrid);}

// ---- spools --------------------------------------------------------------------------------------
const MATERIALS=['PETG','ASA','PLA','ABS','TPU','PC','PA'];
export function sheetSpools(){
 openSheet({title:'Spools',icon:'i-spool',wide:true,eyebrow:'Filament',build:body=>{
  const msg=el('p','nx-note');msg.setAttribute('role','status');
  const host=el('div','fl-spools');body.append(host,msg);
  const draw=async()=>{
   let d,pl;try{[d,pl]=await Promise.all([fetch('/api/spools',{cache:'no-store'}).then(r=>r.json()),printers()]);}catch{host.replaceChildren(el('p','nx-note','Stella Scope did not answer.'));return;}
   const can=!!pl.token,list=d.spools||[];host.replaceChildren();
   const sum=el('div','fl-stats');const left=list.reduce((a,s)=>a+(s.grams_left||0),0),low=list.filter(s=>s.low).length;
   sum.append(stat(String(list.length),list.length===1?'spool':'spools'),stat(kg(left),'on hand'),stat(String(low),'low',low?'warn':''),stat(String(list.filter(s=>s.printer).length),'loaded'));
   host.append(sum);
   if(!list.length)host.append(el('p','fl-empty','No spools yet. Add one: what is left on it drops by what each finished print on its printer used.'));
   const cards=el('div','fl-spool-grid');
   for(const s of list){
    const c=el('article','fl-spool nx-glass');c.style.setProperty('--sc',s.colour||'#9AA4B2');if(s.low)c.dataset.low='';
    const ring=el('div','fl-spool-ring');const frac=Math.max(0,Math.min(1,(s.grams_left||0)/(s.grams_total||1)));ring.style.setProperty('--f',String(frac));ring.append(el('b','',kg(s.grams_left||0)),el('small','',`of ${kg(s.grams_total||0)}`));
    const info=el('div','fl-spool-info');info.append(el('b','',[s.brand,s.material].filter(Boolean).join(' ')||s.material),el('span','',s.name||' '),
     el('small','',s.printer_name?`On ${s.printer_name}`:'Not on a printer'),el('small','',`${kg(s.used_g||0)} used${s.price_per_kg?` · $${s.price_per_kg}/kg`:''}`));
    c.append(ring,info);
    if(s.low)c.append(el('span','fl-spool-low','LOW'));
    if(can){const act=el('div','fl-spool-act');
     const on=el('select','nx-input');on.setAttribute('aria-label','Printer');on.append(new Option('Not on a printer',''));for(const p of pl.printers||[])on.append(new Option(p.name||p.id,p.id));on.value=s.printer||'';
     on.addEventListener('change',async()=>{try{await write('PATCH','/api/spools/'+s.id,{printer:on.value||null});toast(on.value?'Spool loaded.':'Spool taken off.','ok');draw();}catch(x){msg.textContent=x.message;}});
     const weigh=button({icon:'i-filament-load',label:'Weigh',size:'row',title:'Set what is left',does:'Type what the scale says (filament only, spool weight taken off). Usage counts from there.',onPress:()=>{
      // U05: what the scale says is turned in, not typed (the kit's gauge, a gram a click); Save sends it
      const f=el('div','fl-inline fl-weigh');const g=numberInput({label:'Grams left',unit:'g',min:0,max:Math.max(1000,Math.ceil((s.grams_total||1000)/250)*250),step:1,turn:720,value:Math.round(s.grams_left||0),color:s.colour||'#04D9C4',color2:'#FFFFFF',size:96});
      f.append(g,button({icon:'i-check',label:'Save',kind:'primary',size:'row',title:'Save',does:'Saves what is left.',onPress:async()=>{try{await write('PATCH','/api/spools/'+s.id,{grams_left:g.get()});toast('Saved.','ok');draw();}catch(x){msg.textContent=x.message;}}}));
      act.replaceChildren(f);}});
     const rm=button({icon:'i-close',size:'icon',kind:'ghost',title:'Used up',does:'Takes the spool off the list. Its usage stays in the analytics.',onPress:async()=>{try{await write('DELETE','/api/spools/'+s.id);draw();}catch(x){msg.textContent=x.message;}}});
     act.append(on,weigh,rm);c.append(act);}
    cards.append(c);}
   host.append(cards);
   if(can)host.append(section('Add a spool',addForm(pl,draw,msg)),section('Price',priceForm(d,draw,msg)));
   host.append(el('p','nx-note fl-basis',d.basis||''));};
  draw();}});}
// U05: a count or a total is an LED figure (the value lit, its unit small beside it), not a line of text
function stat(v,l,tone=''){const s=el('div','fl-stat');if(tone)s.dataset.tone=tone;const m=String(v).match(/^(\$?[\d.,]+)\s*(.*)$/);s.append(m?led(m[1],m[2]):led(v),el('span','',l));return s;}
function addForm(pl,after,msg){
 const f=el('div','fl-form');
 const mat=el('select','nx-input');mat.setAttribute('aria-label','Material');for(const m of MATERIALS)mat.append(new Option(m,m));
 const brand=el('input','nx-input');brand.placeholder='Brand';brand.setAttribute('aria-label','Brand');
 const name=el('input','nx-input');name.placeholder='Colour name';name.setAttribute('aria-label','Colour name');
 const col=el('input','fl-colour');col.type='color';col.value='#FF6A00';col.setAttribute('aria-label','Colour');
 // U05: grams on a full spool and the price are the kit's gauges; the price is sent only if it was turned (a spool without one takes the farm price)
 const total=numberInput({label:'Grams on a full spool',unit:'g',min:100,max:5000,step:50,turn:300,value:1000,color:'var(--accent)',color2:'var(--accent-2)',size:92});
 let priced=false;const price=numberInput({label:'Price per kg',prefix:'$',min:0,max:150,step:.5,turn:300,value:25,color:'#FF6A00',color2:'#FFB067',size:92,onSet:()=>{priced=true;}});
 const on=el('select','nx-input');on.setAttribute('aria-label','Printer');on.append(new Option('Not on a printer',''));for(const p of pl.printers||[])on.append(new Option(p.name||p.id,p.id));
 const lab=(t,i)=>{const l=el('label','fl-field');l.append(el('span','',t),i);return l;};
 const nums=el('div','nx-numrow fl-numrow');nums.append(total,price);
 f.append(lab('Material',mat),lab('Brand',brand),lab('Colour',name),lab('',col),lab('On',on),nums,
  button({icon:'i-plus',label:'Add spool',kind:'primary',size:'row',title:'Add spool',does:'Adds this spool; if it is on a printer, that printer\'s prints count from now.',onPress:async b=>{busy(b,true);
   try{await write('POST','/api/spools',{material:mat.value,brand:brand.value,name:name.value,colour:col.value,grams_total:total.get(),price_per_kg:priced?price.get():null,printer:on.value||null});toast('Spool added.','ok');after();}catch(x){msg.textContent=x.message;}finally{busy(b,false);}}}));
 return f;}
function priceForm(d,after,msg){
 const f=el('div','fl-inline fl-priceform');let turned=false;const p=numberInput({label:'Farm price per kg',prefix:'$',min:0,max:150,step:.5,turn:300,value:d.price_per_kg??0,color:'#FF6A00',color2:'#FFB067',size:92,onSet:()=>{turned=true;}});
 f.append(el('span','nx-note','Used for cost where a spool has no price of its own.'),p,button({icon:'i-check',label:'Save',size:'row',title:'Save price',does:'Saves the farm price per kilogram.',onPress:async()=>{try{await write('POST','/api/spools/price',{price_per_kg:d.price_per_kg==null&&!turned?null:p.get()});toast('Saved.','ok');after();}catch(x){msg.textContent=x.message;}}}));
 return f;}

// ---- analytics -----------------------------------------------------------------------------------
export function sheetAnalytics(){
 let days=30;
 openSheet({title:'Analytics',icon:'i-chart',wide:true,eyebrow:'The farm',build:body=>{
  const seg=el('div','nx-seg fl-seg');for(const n of [7,30,90,365]){const b=el('button','',n===365?'Year':n+' days');b.type='button';b.setAttribute('aria-pressed',String(n===days));b.addEventListener('click',()=>{days=n;for(const x of seg.children)x.setAttribute('aria-pressed',String(x===b));draw();});seg.append(b);}
  const host=el('div','fl-an');body.append(seg,host);
  const draw=async()=>{let d;try{d=await (await fetch('/api/analytics?days='+days,{cache:'no-store'})).json();}catch{host.replaceChildren(el('p','nx-note','Stella Scope did not answer.'));return;}
   const t=d.total||{};host.replaceChildren();
   const s=el('div','fl-stats');
   s.append(stat(String(t.prints||0),'prints'),stat(pctx(t.success_rate),'succeeded',t.success_rate!==null&&t.success_rate<.85?'warn':'ok'),stat(hrs(t.print_h||0),'printing'),stat(pctx(t.utilisation),'busy'),stat(kg(t.grams||0),'filament'),stat(t.cost_known===false&&!t.cost?'—':money(t.cost),'filament cost'));
   host.append(s);
   host.append(chart(d.daily||[],days));
   const tb=el('div','fl-table');const COLS=['Printer','Prints','Success','Failed','Stopped','Printing','Busy','Filament','Cost'],head=el('div','fl-tr fl-th');for(const h of COLS)head.append(el('span','',h));tb.append(head);
   for(const p of d.printers||[]){const r=el('div','fl-tr');r.style.setProperty('--c',p.accent||'var(--accent)');
    const nm=el('span','fl-td-name');nm.append(el('i','fl-cam-dot'),el('b','',p.name||p.id));
    const busyCell=el('span','fl-td-bar');const bar=el('i');bar.style.setProperty('--p',String(p.utilisation||0));busyCell.append(bar,el('em','',pctx(p.utilisation)));
    r.append(nm,el('span','',String(p.prints)),el('span','',pctx(p.success_rate)),el('span','',String(p.failed)),el('span','',String(p.cancelled)),el('span','',hrs(p.print_h||0)),busyCell,el('span','',kg(p.grams||0)),el('span','',money(p.cost)));
    [...r.children].forEach((c,i)=>{if(i)c.dataset.l=COLS[i];});   // a phone shows each figure under its name (next.css, F04)
    tb.append(r);}
   const wrap=el('div','fl-table-wrap');wrap.append(tb);host.append(wrap);
   host.append(el('p','nx-note fl-basis',d.basis||''));};
  draw();}});}
function chart(daily,days){
 const box=el('figure','fl-chart nx-glass');const cap=el('figcaption','','Filament per day');box.append(cap);
 const max=Math.max(1,...daily.map(d=>d.grams));const w=Math.max(daily.length,1),NS='http://www.w3.org/2000/svg';
 const svg=document.createElementNS(NS,'svg');svg.setAttribute('viewBox',`0 0 ${w*10} 100`);svg.setAttribute('preserveAspectRatio','none');svg.setAttribute('role','img');svg.setAttribute('aria-label',`Grams of filament per day over ${days} days, highest ${Math.round(max)} g`);
 daily.forEach((d,i)=>{const h=d.grams/max*92;const r=document.createElementNS(NS,'rect');r.setAttribute('x',String(i*10+1.5));r.setAttribute('width','7');r.setAttribute('y',String(100-h));r.setAttribute('height',String(Math.max(h,d.prints?1.5:0)));r.setAttribute('rx','1.5');
  const t=document.createElementNS(NS,'title');t.textContent=`${new Date(d.day*1000).toLocaleDateString()}: ${Math.round(d.grams)} g, ${d.prints} ${d.prints===1?'print':'prints'}`;r.append(t);svg.append(r);});
 const scale=el('div','fl-chart-scale');scale.append(el('span','',kg(max)),el('span','','0'));
 const plot=el('div','fl-chart-plot');plot.append(svg,scale);box.append(plot);return box;}

// ---- backup and restore --------------------------------------------------------------------------
export function sheetBackup(){
 openSheet({title:'Backup',icon:'i-backup',eyebrow:'The farm',build:body=>{
  const msg=el('p','nx-note');msg.setAttribute('role','status');
  const save=button({icon:'i-backup',label:'Save a backup',kind:'primary',size:'row',title:'Save a backup',does:'Saves the printer list, spools, prices and alert choices to a file. Logins, keys and recordings are left out.',onPress:async b=>{busy(b,true);
   try{const r=await fetch('/api/farm/backup',{cache:'no-store'});const j=await r.json();if(!r.ok)throw Error(j.error||'No backup.');
    const blob=new Blob([JSON.stringify(j,null,2)],{type:'application/json'}),a=el('a');a.href=URL.createObjectURL(blob);a.download=`stella-scope-farm-${new Date().toISOString().slice(0,10)}.json`;document.body.append(a);a.click();setTimeout(()=>{URL.revokeObjectURL(a.href);a.remove();},1000);
    msg.textContent=`Saved: ${j.printers.length} ${j.printers.length===1?'printer':'printers'}, ${(j.books?.spools||[]).length} spools. Left out: ${j.left_out}.`;}catch(x){msg.textContent=x.message;}finally{busy(b,false);}}});
  const file=el('input');file.type='file';file.accept='application/json,.json';file.hidden=true;
  const load=button({icon:'i-files',label:'Restore from a file',size:'row',title:'Restore',does:'Adds the printers and spools from a backup that this farm does not have. Nothing here is changed or removed.',onPress:()=>file.click()});
  file.addEventListener('change',async()=>{const f=file.files?.[0];if(!f)return;
   try{const j=JSON.parse(await f.text());const r=await write('POST','/api/farm/restore',j);
    msg.textContent=`Added ${r.added.length} ${r.added.length===1?'printer':'printers'}${r.skipped.length?`, kept ${r.skipped.length} already here`:''}, ${r.spools_added} spools.${r.needs_login.length?` Type the login for ${r.needs_login.join(', ')} in Printers, under Settings.`:''}${r.failed.length?` Not added: ${r.failed.map(x=>x.id+' ('+x.error+')').join('; ')}.`:''}`;toast('Restored.','ok');}
   catch(x){msg.textContent=x.message||'That file is not a Stella Scope farm backup.';}finally{file.value='';}});
  body.append(section('Save',el('p','nx-note','One file with the farm: every printer and its settings, the spools and prices, and which alerts are on. Passwords, keys, check codes and the recordings are never in it.'),save),
   section('Restore',el('p','nx-note','On a new computer, or after a reinstall. Printers and spools this farm already has are left as they are.'),load,file),msg);}});}

// ---- slicer upload -------------------------------------------------------------------------------
// OrcaSlicer, PrusaSlicer and Bambu Studio send to an "OctoPrint" printer: this farm answers as one
// and puts the file in the queue. The slicer gets its own key by default; the owner can also let the
// farm key work there, or turn slicer uploads off (2026-10-13).
export function sheetSlicer(){
 openSheet({title:'Slicer upload',icon:'i-key',eyebrow:'The queue',build:body=>{
  const msg=el('p','nx-note');msg.setAttribute('role','status');const host=el('div','fl-slicer');body.append(host,msg);
  const draw=async()=>{let d;try{const r=await fetch('/api/slicer',{cache:'no-store'});d=await r.json();if(!r.ok)throw Error(d.error);}catch(x){host.replaceChildren(el('p','nx-note',x.message||'Set up from the Stella Scope window on the computer.'));return;}
   host.replaceChildren();
   const row=(label,value)=>{const r=el('div','fl-keyrow');const v=el('code','',value);const c=button({icon:'i-files',label:'Copy',size:'row',title:'Copy',does:'Copies it to paste into the slicer.',onPress:async()=>{try{await navigator.clipboard.writeText(value);toast('Copied.','ok');}catch{const rg=document.createRange();rg.selectNodeContents(v);getSelection().removeAllRanges();getSelection().addRange(rg);msg.textContent='Selected: press Ctrl+C to copy.';}}});c.setAttribute('aria-label','Copy '+label.toLowerCase());r.append(el('span','',label),v,c);return r;};
   const on=d.enabled;
   const sw=el('div','nx-seg fl-seg');for(const [k,l] of [[true,'Slicer uploads on'],[false,'Off']]){const b=el('button','',l);b.type='button';b.setAttribute('aria-pressed',String(on===k));b.addEventListener('click',async()=>{try{await write('POST','/api/slicer',{enabled:k});draw();}catch(x){msg.textContent=x.message;}});sw.append(b);}
   host.append(sw);
   if(!on){host.append(el('p','nx-note','A slicer cannot send files here. Files are added from the Queue only.'));return;}
   host.append(el('p','nx-note','In your slicer, add a physical printer with host type OctoPrint, this address and this API key. "Upload" puts the file in the farm queue; it starts on a printer that fits it after the plate check, never by itself.'),
    row('Host type','OctoPrint'),row('Address',d.address),row('API key',d.key));
   const which=el('div','nx-seg fl-seg');for(const [k,l] of [[false,'Upload key only'],[true,'Upload key or farm key']]){const b=el('button','',l);b.type='button';b.setAttribute('aria-pressed',String(d.accept_farm_key===k));b.addEventListener('click',async()=>{try{await write('POST','/api/slicer',{accept_farm_key:k});draw();}catch(x){msg.textContent=x.message;}});which.append(b);}
   host.append(section('Which key a slicer may use',which,el('p','nx-note',d.accept_farm_key?'The farm key also works. A slicer that stores it can control every printer here, so keep it on this computer.':'The upload key can only add files to the queue. It cannot start, stop or change any printer.')),
    section('',button({icon:'i-restart',label:'Make a new key',size:'row',title:'New key',does:'The old key stops working at once. Paste the new one into your slicer.',onPress:async()=>{try{await write('POST','/api/slicer',{new_key:true});toast('New key made. Paste it into your slicer.','ok');draw();}catch(x){msg.textContent=x.message;}}})));};
  draw();}});}
