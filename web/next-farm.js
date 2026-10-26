// Printers: the list, adding one, and a printer's settings (farm board WO-06, 2026-10-12).
// Adding is three steps in plain words: find it (a read-only search of this network, or an address you
// type), connect it (only what that brand needs, tested live, with what Stella can do with it shown
// before anything is saved), name it. A wrong password or check code is said in words and saves nothing.
import {el,icon,button,busy,openSheet,section,segmented,toast,openModal,numberField,numberInput} from './next-ui.js';
import {plateSetup} from './next-queue.js';
import {modelSection,modelsSheet,modelFlow,choice} from './next-models.js';
import {demoLicenceLine} from './next-getmodels.js';
import {hitTitle,sampleStateWords} from './next-core.js';

const BRANDS=[['moonraker','Klipper','Voron, or any printer running Klipper with Moonraker'],['prusalink','Prusa','CORE One, MK4, MINI and XL on Prusa firmware (PrusaLink)'],['flashforge','FlashForge','Adventurer 5M and 5M Pro']];
const BRAND=Object.fromEntries(BRANDS.map(b=>[b[0],b[1]]));
// 2026-10-25 audit: the list printed the collector's own words ("cannot reach Moonraker at ... os error 111 ... set STELLA_MOONRAKER_API_KEY").
export function plainDetail(p){const t=String(p.detail||'');if(!t)return '';
 if(/cannot reach|refused|os error|timed? ?out|unreachable|no route|not answering/i.test(t))return `Not answering at ${p.address||'its address'}. Check that it is on and on the network; it reconnects by itself.`;
 if(/401|403|unauthori|forbidden|api.?key|login/i.test(t))return 'It refused Stella Scope: it needs its login. Press Settings to enter it.';
 return t.replace(/Moonraker/g,'the printer').replace(/\s*\(os error \d+\)/g,'').replace(/STELLA_[A-Z_]+/g,'its login');}
export const ACCENTS=[['#04D9C4','Teal'],['#FF6A00','Orange'],['#FF2D92','Magenta'],['#18E36B','Green'],['#3D8BFF','Blue'],['#B58CFF','Violet']];
const STATE_WORDS={idle:'Ready',printing:'Printing',paused:'Paused',heating:'Heating',finished:'Finished',error:'Needs attention',offline:'Offline'};

const call=(method,url,body,token)=>fetch(url,{method,headers:{...(body!==undefined?{'Content-Type':'application/json'}:{}),...(token?{'X-Stella-Control-Token':token}:{})},body:body===undefined?undefined:JSON.stringify(body)})
 .then(async r=>{const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);return d;});
const listPrinters=()=>fetch('/api/printers').then(r=>r.json());
const pretty=s=>String(s||'').replace(/^flashforge\s+/i,'').replace(/[-_]+/g,' ').replace(/\b[a-z]/g,c=>c.toUpperCase()).trim();
const sentence=t=>{t=String(t||'').trim();return t?t[0].toUpperCase()+t.slice(1)+(/[.!?]$/.test(t)?'':'.'):t;};
// F06 (2026-10-19): a Prusa that has not given its name (its login comes first) was listed as just "PrusaLink", the name of the software it runs, so a person with
// two could not tell which was theirs: it is "A Prusa printer", and its address says which.
const label=hitTitle;
const sampleState=sampleStateWords;
const field=(label,input,note)=>{const w=el('label','nx-fm-field');w.append(el('span','nx-fm-label',label),input);if(note)w.append(el('small','nx-fm-note',note));return w;};
// U05: a number that is set (a nozzle, a plate side) is the kit's gauge; `untouched` says it still holds the value it started with, so a setting nobody turned is not sent
const gaugeField=(label,nodes,note)=>{const w=el('div','nx-fm-field nx-fm-gauges');w.append(el('span','nx-fm-label',label));const r=el('div','nx-numrow');r.append(...nodes);w.append(r);if(note)w.append(el('small','nx-fm-note',note));return w;};
const text=(value='',placeholder='',{type='text',aria}={})=>{const i=el('input','nx-input');i.type=type;i.value=value;i.placeholder=placeholder;i.autocomplete='off';i.spellcheck=false;if(aria)i.setAttribute('aria-label',aria);return i;};
export function swatches(current,onPick){const w=el('div','nx-fm-swatches');w.setAttribute('role','radiogroup');w.setAttribute('aria-label','Colour');
 const paint=c=>{for(const b of w.children)b.setAttribute('aria-pressed',String(b.dataset.c===c));};
 for(const [c,name] of ACCENTS){const b=el('button','nx-fm-sw');b.type='button';b.dataset.c=c;b.style.setProperty('--c',c);b.setAttribute('aria-label',name);b.title=name;b.addEventListener('click',()=>{paint(c);onPick(c);});w.append(b);}
 paint(current);return w;}
function capList(caps){const w=el('div','nx-fm-capwrap'),g=el('div','nx-fm-caps'),off=caps.filter(c=>!c.on);
 for(const c of caps.filter(c=>c.on)){const r=el('div','nx-fm-cap',c.label);r.dataset.on='true';r.prepend(icon('i-check'));g.append(r);}
 if(!g.children.length)g.append(el('p','nx-note','Nothing can be done from here yet: this printer is read only.'));
 w.append(g);if(off.length)w.append(el('p','nx-note nx-fm-off','Not available on this printer from here: '+off.map(c=>c.label[0].toLowerCase()+c.label.slice(1)).join(', ')+'.'));return w;}
function stateChip(p){const c=el('span','nx-fm-pill',STATE_WORDS[p.state]||pretty(p.state));c.dataset.s=p.state;return c;}

// ---- Prusa defaults (finish-2 F09, 2026-10-19) -----------------------------------------------------------
// A PrusaLink printer is asked which Prusa it is, from one table the service keeps of every Prusa machine (src/prusa_machines.rs): its plate, its nozzle, the name its
// sliced files carry, a camera hint, the 3D model that draws it, and where none does, exactly why. The printer's own hostname preselects it when it names one.
const prusaTable=()=>fetch('/api/printer-models/prusa',{cache:'no-store'}).then(r=>r.json()).then(j=>j.machines||[]).catch(()=>[]);
const prusaFacts=m=>[m.bed_mm?`plate ${m.bed_mm[0]} × ${m.bed_mm[1]} mm`:'plate size not published (set it under Printers)',`nozzle ${m.nozzle_mm} mm`,m.slicer_model?`called ${m.slicer_model} in sliced files`:'',m.cad_model?'3D model: drawn, moving with the printer':'no 3D model yet'].filter(Boolean).join(' · ');

// ---- the demo farm (polish WO-02, 2026-10-18) -------------------------------------------------
// One press adds nine simulated printers (Voron 2.4, CORE One L, Trident, V0.2, Micron+, SV08, a toolchanger, Ender-3 and Prusa MK3S+), each at its own
// place in a sample print: one heating, one finished, one paused, the rest printing. None is real, none counts toward the licence, each
// says DEMO. A second press takes them away, and only them.
export const hasDemo=printers=>(printers||[]).some(p=>p.driver==='demo');
// F03 (2026-10-19): the press is the person's yes to the open licences shown beside the button (`accept_licence`), and the service fetches the
// demo's 3D models at once, each tile showing its own progress. Nothing else is asked.
export async function demoFarmAdd(token){
 const r=await call('POST','/api/printers/demo-farm',{accept_licence:true},token);
 toast(`Demo farm added: ${r.added.length} printers running sample prints.${r.fetch?.state==='running'?' Their 3D models are arriving now; each machine draws as its file lands.':r.missing_models?.length?' Some 3D models are not here yet.':''}`,'ok');
 return r;}
export function demoFarmRemove(host,{token,onDone}){
 host.replaceChildren(el('p','nx-note','Remove every demo printer? Real printers are not touched.'),
  button({icon:'i-close',label:'Yes, remove the demo farm',kind:'danger',size:'row',title:'Remove the demo farm',does:'Removes the simulated printers and their recordings. Nothing else.',onPress:async()=>{try{const r=await call('DELETE','/api/printers/demo-farm',undefined,token);toast(r.removed.length?`Demo farm removed: ${r.removed.length} printers.`:'There was no demo farm.','ok');onDone();}catch(e){toast(e.message,'alarm');}}}),
  button({icon:'i-back',label:'Keep it',size:'row',title:'Keep it',does:'Leaves the demo farm as it is.',onPress:onDone}));}

// The open licences of the demo's 3D models in one line, from the service (it builds the line from the catalogue), beside the button that fetches them.
function demoLic(){const p=el('p','nx-note nx-demo-lic','');p.hidden=true;demoLicenceLine().then(t=>{p.textContent=t;p.hidden=!t;});return p;}

// ---- the list ---------------------------------------------------------------------------------
export function sheetPrinters(){return openSheet({title:'Printers',icon:'i-printers',wide:true,build:body=>drawList(body)});}
export function sheetAddPrinter(){return openSheet({title:'Add a printer',icon:'i-printers',wide:true,build:body=>addFlow(body,null)});}

async function drawList(body){
 body.replaceChildren(el('p','nx-note','Loading printers…'));
 let d;try{d=await listPrinters();}catch{body.replaceChildren(el('p','nx-note','Stella Scope did not answer.'));return;}
 const can=!!(d.token&&d.writable);body.replaceChildren();
 const list=el('div','nx-fm-list');
 for(const p of d.printers){
  const row=el('div','nx-fm-row');row.style.setProperty('--c',p.accent||'var(--accent)');
  const dot=el('i','nx-fm-dot'),name=el('b','nx-fm-name',p.name||p.id),sub=el('small','nx-fm-sub',`${p.brand||BRAND[p.driver]||p.driver} · ${p.driver==='demo'?'simulated, marked DEMO':String(p.address||'').replace(/^https?:\/\//,'')}`);
  const act=el('div','nx-fm-act');
  act.append(button({icon:'i-nav-hud',label:'Open',size:'row',title:'Open '+(p.name||p.id),does:'Opens this printer: its instruments, camera and controls.',onPress:()=>{location.href=p.default?'/':'/p/'+p.id;}}));
  if(can)act.append(button({icon:'i-more',label:'Settings',size:'row',kind:'ghost',title:'Settings',does:'Rename it, change its colour or camera, or remove it.',onPress:()=>settings(p,d.token,()=>drawList(body))}));
  const top=el('div','nx-fm-top');top.append(name,stateChip(p));
  const why=p.state==='offline'||p.state==='error'?el('small','nx-fm-why',plainDetail(p)):null;
  row.append(dot,top,sub);if(why&&p.detail)row.append(why);row.append(act);list.append(row);}
 body.append(section(`${d.printers.length} ${d.printers.length===1?'printer':'printers'} in this farm`,list));
 const licBox=el('div','nx-q-stage');licenceState().then(st=>licenceSection(licBox,st));body.append(section('Farm licence',licBox));
 if(can&&hasDemo(d.printers)){const box=el('div','nx-fm-demo');const paint=()=>{box.replaceChildren(el('p','nx-note',`${d.printers.filter(p=>p.driver==='demo').length} simulated printers (marked DEMO). They run sample prints and never count toward the licence. Their 3D models are separate downloads: the button at the top of the farm wall gets the open ones, and the rest are under Printer models in More.`),
   button({icon:'i-view',label:'Printer models',size:'row',title:'Printer models',does:'Opens the list of 3D models, each with its licence, to add the ones the demo printers use.',onPress:()=>modelsSheet({token:d.token})}),
   button({icon:'i-close',label:'Remove the demo farm',kind:'danger',size:'row',title:'Remove the demo farm',does:'Removes every simulated printer. Real printers are not touched.',onPress:()=>demoFarmRemove(box,{token:d.token,onDone:()=>drawList(body)})}));};paint();body.append(section('Demo farm',box));}
 if(can)body.append(button({icon:'i-plus',label:'Add a printer',kind:'primary',size:'row',title:'Add a printer',does:'Finds printers on this network, or takes an address you type.',onPress:()=>addFlow(body,()=>drawList(body))}));
 else body.append(el('p','nx-note','Printers are added, renamed and removed from the Stella Scope window on the computer.'));}


// ---- the farm licence (WO-09) -----------------------------------------------------------------
// Free to five printers. Beyond that a licence is asked for, once, on trust: "Not now" still adds the
// printer and nothing is switched off. The check is offline (the service verifies the text).
const NUM=['zero','first','second','third','fourth','fifth','sixth','seventh','eighth','ninth','tenth','eleventh','twelfth'];
const ordinal=n=>NUM[n]||`${n}th`;
export const licenceState=()=>fetch('/api/licence',{cache:'no-store'}).then(r=>r.json()).catch(()=>null);
export const licenceDismissed=()=>{try{return Date.now()-Number(localStorage.getItem('stella-licence-note')||0)<7*86400e3;}catch{return false;}};
export const dismissLicence=()=>{try{localStorage.setItem('stella-licence-note',String(Date.now()));}catch{}};

/** How to buy a farm licence (finish-2 F07, 2026-10-19: no e-Transfer address is ever shown publicly). The only address on screen is where to write;
 *  the payment details come with the invoice. */
export function licenceBuy({onEnter}={}){
 return openModal({title:'Buy a farm licence',icon:'i-key',build:(body,close)=>{
  body.append(el('p','nx-note','Stella Scope is free for up to five printers. A farm licence for more is perpetual: you buy it once for the number of printers you run, and it covers them for good.'),
   el('p','nx-note','To buy, write to info@3d3d.ca with your farm name and how many printers. 3D3D replies with an invoice. You pay it by Interac e-Transfer; the payment details come with the invoice.'),
   el('p','nx-note','You are then sent one line of text that starts SFL1. Paste it into Enter licence. Nothing is sent anywhere: it is checked on this computer.'));
  const mail=button({icon:'i-link',label:'Write to 3D3D',kind:'primary',size:'row',title:'Write to 3D3D',does:'Opens a new email to info@3d3d.ca in your mail program.',onPress:()=>{window.location.href='mailto:info@3d3d.ca?subject='+encodeURIComponent('Stella Scope farm licence');}});
  const enter=button({icon:'i-check',label:'Enter licence',size:'row',title:'Enter licence',does:'Paste a farm licence. It is checked on this computer.',onPress:()=>{close();onEnter?.();}});
  body.append(mail,enter);}});}

/** A small dialog: paste the licence, it is checked, and it says who it is for. */
export function licenceEntry({onDone}={}){
 return openModal({title:'Enter a farm licence',icon:'i-check',build:(body,close)=>{
  const box=el('textarea','nx-input nx-lic-text');box.rows=4;box.placeholder='SFL1.…';box.spellcheck=false;box.autocomplete='off';box.setAttribute('aria-label','Licence text');
  const msg=el('p','nx-note');
  const save=button({icon:'i-check',label:'Save licence',kind:'primary',size:'row',title:'Save licence',does:'Checks the licence on this computer. Nothing is sent anywhere.',onPress:async b=>{
   if(b.dataset.busy!==undefined)return;if(!box.value.trim()){msg.textContent='Paste the licence text first.';return;}
   busy(save,true);
   try{const t=await farmWriteToken();const r=await call('POST','/api/licence',{text:box.value},t);close();toast(`Licensed to ${r.licence.name}, covering ${r.licence.printers} printers.`,'ok');onDone?.(r);}
   catch(e){msg.textContent=sentence(e.message);}finally{busy(save,false);}}});
  body.append(el('p','nx-note','Paste the licence you were sent. It is one line that starts SFL1.'),box,save,msg);box.focus();}});}

const farmWriteToken=()=>fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).then(d=>{if(!(d.token&&d.writable))throw Error('The licence is entered in the Stella Scope window on the computer.');return d.token;});

/** The calm screen shown when a printer beyond the free five is about to be added. `proceed` adds it. */
function licenceScreen(body,st,proceed,back){
 body.replaceChildren();
 const card=el('div','nx-lic');
 const mark=el('img','nx-lic-mark');mark.src='/logo.svg';mark.alt='';
 card.append(mark,el('h3','nx-lic-title','A farm licence'),
  el('p','nx-lic-text-p',`Stella Scope is free for up to five printers. This would be your ${ordinal(st.printers+1)}.`),
  el('p','nx-lic-text-p','A farm licence for more printers is perpetual: you buy it once and it covers them for good. To buy, write to info@3d3d.ca. You get an invoice, and the payment details for the Interac e-Transfer come with it. A licence is one line of text that you paste in; it is checked on this computer, with no internet, and it is not an account.'));
 const row=el('div','nx-q-actions');
 row.append(button({icon:'i-check',label:'Enter licence',kind:'primary',size:'row',title:'Enter licence',does:'Paste a farm licence. It is checked on this computer.',onPress:()=>licenceEntry({onDone:()=>proceed()})}),
  button({icon:'i-arrow-right',label:'Not now',size:'row',title:'Not now',does:'Adds the printer anyway. Nothing is switched off.',onPress:()=>proceed()}));
 card.append(row,el('p','nx-note','Not now still adds the printer, and nothing is switched off. A small note on the Farm page says a licence is wanted; close it and it stays quiet for a week.'));
 body.append(card);
 if(back)body.append(button({icon:'i-back',label:'Back',size:'row',kind:'ghost',title:'Back',does:'Goes back to naming the printer.',onPress:back}));}

function licenceSection(host,st){
 host.replaceChildren();
 if(!st)return;
 const line=el('p','nx-note');
 if(st.state==='free'){
  line.textContent=`Free up to ${st.free_printers} printers. This farm has ${st.printers}.`+(st.licence?` Licensed to ${st.licence.name}, covering ${st.licence.printers}.`:'');
 }else if(st.state==='licensed'){
  line.textContent=`Licensed to ${st.licence.name}. Covers ${st.licence.printers} printers (this farm has ${st.printers}).`+(st.licence.issued?` Issued ${st.licence.issued}.`:'');
 }else if(st.state==='too_few'){
  line.textContent=`The licence covers ${st.licence.printers} printers; this farm has ${st.printers}. A licence for more printers is asked for.`;
 }else if(st.state==='trial'){
  const days=st.trial_days_left??0;
  line.textContent=`Trial running: ${Math.ceil(days)} day(s) left before a farm licence is required beyond ${st.free_printers} printers.`;
 }else if(st.state==='locked'){
  line.textContent=`The trial has ended. A farm licence turns printers beyond the free five back on.`;
 }else{
  line.textContent=`${st.printers} printers: a farm licence is asked for beyond ${st.free_printers}. Nothing is switched off while the trial runs.`;
 }
 const actions=el('div','nx-q-actions');
 actions.append(button({icon:'i-check',label:st.licence?'Enter a different licence':'Enter licence',size:'row',title:'Enter licence',does:'Paste a farm licence. It is checked on this computer.',onPress:()=>licenceEntry({onDone:()=>licenceState().then(s=>licenceSection(host,s))})}));
 if(st.state==='trial'||st.state==='locked'||st.state==='wanted'||st.state==='too_few'){
  actions.append(button({icon:'i-key',label:'Buy a licence',size:'row',title:'Buy a licence',does:'How to buy a farm licence: write to 3D3D, get an invoice.',onPress:()=>licenceBuy({onEnter:()=>licenceEntry({onDone:()=>licenceState().then(s=>licenceSection(host,s))})})}));
 }
 host.append(line,actions);}

// ---- adding -----------------------------------------------------------------------------------
function addFlow(body,back){
 const draft={driver:'',address:'',hit:null,credentials:{},camera:'',name:'',accent:ACCENTS[0][0],test:null,prusa:null};
 let token=null;
 const head=(title,sub,onBack)=>{const h=el('div','nx-fm-head');if(onBack)h.append(button({icon:'i-back',size:'icon',kind:'ghost',title:'Back',does:'Goes back one step. Nothing has been saved.',onPress:onBack}));const t=el('div','nx-fm-headtext');t.append(el('b','',title));if(sub)t.append(el('small','',sub));h.append(t);return h;};
 const gate=async()=>{if(token)return true;try{const d=await listPrinters();if(d.token&&d.writable){token=d.token;return true;}}catch{}body.replaceChildren(head('Add a printer'),el('p','nx-note','Printers are added from the Stella Scope window on the computer.'));return false;};

// A simulated printer: no network, nothing leaves the machine, nothing is printed.
async function addDemo(make,model,id,name){
  if(!await gate())return;
  try{
   const t=await fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).then(d=>d.token);
   await call('POST','/api/printers',{id,name,driver:'demo',address:'',model,accent:ACCENTS[0][0],demo_start_at:0.25},t);
   toast(`${name} added. Open it to heat, home and run a sample print.`,'ok');
   drawList(body);
  }catch(e){toast(e.message,'alarm');}}
// Step 1: find
async function find(){
  if(!await gate())return;
  body.replaceChildren(head('Find your printer','Step 1 of 4',back));
  const status=el('p','nx-note','Stella Scope asks the devices on your network who they are. It reads only; nothing on any printer is changed.');
  const results=el('div','nx-fm-list');
  const net=text('','This network',{aria:'Network to search'});
  const go=button({icon:'i-search',label:'Search my network',kind:'primary',size:'row',title:'Search my network',does:'Looks at the 254 addresses on this network for Klipper, Prusa and FlashForge printers. Takes a few seconds.',onPress:async b=>{
    if(b.dataset.busy!==undefined)return;busy(go,true);results.replaceChildren();status.textContent='Looking at this network…';
    try{const r=await call('POST','/api/printers/discover',net.value.trim()?{subnet:net.value.trim()}:{},token);
     status.textContent=r.printers.length?`${r.printers.length} found in ${r.seconds.toFixed(1)} s. Pick yours.`:`Nothing answered in ${r.seconds.toFixed(1)} s. Check that the printer is on and on this network, or type its address below.`;
     for(const h of r.printers)results.append(hit(h));
     // a camera that answered with no Prusa beside it is still worth saying: it is typed into the printer's camera box
     const lone=(r.cameras||[]).filter(c=>!r.printers.some(h=>h.camera_hint===c||(h.cameras||[]).includes(c)));
     if(lone.length)results.append(el('p','nx-note',`A camera answered at ${lone.join(', ')}. It is not a printer: pick the printer it belongs to, then type this address as its camera.`));
    }catch(e){status.textContent=e.message;}finally{busy(go,false);}}});
  const hit=h=>{const b=el('button','nx-item');b.type='button';b.disabled=!!h.added;const t=el('div','nx-item-text');
   t.append(el('div','nx-item-title',label(h)),el('div','nx-item-sub',`${h.brand} · ${h.host}${h.serial?' · '+h.serial:''}${h.detail?' · '+h.detail:''}${h.camera_hint?' · its camera was found':''}`));
   b.append(icon('i-printers'),t);if(h.added){const tag=el('span','nx-fm-pill','Added');tag.dataset.s='idle';b.append(tag);}else b.addEventListener('click',()=>connect(h));return b;};
  // typed address
  const brand=segmented(BRANDS.map(b=>b[0]),'moonraker',v=>{draft.driver=v;hint.textContent=BRANDS.find(b=>b[0]===v)[2];},v=>BRAND[v]);brand.classList.add('nx-seg--wrap');
  draft.driver='moonraker';const hint=el('p','nx-note',BRANDS[0][2]);
  const addr=text('','192.168.1.20',{aria:'Printer address'});
  const next=button({icon:'i-arrow-right',label:'Continue',size:'row',title:'Continue',does:'Goes to the next step with this address. Nothing is sent until you press Test.',onPress:()=>{
    const v=addr.value.trim().replace(/^https?:\/\//,'').replace(/\/+$/,'');
    if(!v||/[\s@?#]/.test(v)){toast('Type the printer\'s address, for example 192.168.1.20.','alarm');return;}
    const port={moonraker:'',prusalink:'',flashforge:':8898'}[draft.driver];
    connect({driver:draft.driver,brand:BRAND[draft.driver],address:'http://'+(v.includes(':')?v:v+port),host:v.split(':')[0],needs:[],manual:true});}});
  body.append(go,status,results,field('Network to search',net,'Leave empty for the network this computer is on. Another looks like 192.168.2, or type one address. Private networks only.'),section('Or type the address yourself',brand,hint,field('Address',addr,'The number shown on the printer\'s network screen, or its name on your network.'),next),section('Or try a demo',el('p','nx-note','Nine simulated printers in one press, each at its own place in a sample print: Voron 2.4, CORE One L, Trident, V0.2, Micron+, SV08, a toolchanger, Ender-3 and Prusa MK3S+. Nothing real is touched and none count toward the licence.'),demoLic(),button({icon:'i-play',label:'Try the demo farm',kind:'primary',size:'row',title:'Try the demo farm',does:'Adds nine simulated printers running sample prints, so the whole farm can be tried with no printer.',onPress:async b=>{if(!await gate())return;busy(b,true);try{await demoFarmAdd(token);drawList(body);}catch(e){toast(e.message,'alarm');}finally{busy(b,false);}}}),button({icon:'i-play',label:'Demo: Stella (Voron 2.4 350)',size:'row',title:'Demo Stella',does:'Adds a simulated Stella that runs a sample file, so every control can be tried without a printer.',onPress:()=>addDemo('Stella','Voron 2.4 350','stella-demo','Stella')}),button({icon:'i-play',label:'Demo: CORE One L',size:'row',title:'Demo CORE One L',does:'Adds a simulated Prusa CORE One L that runs a sample file, so every control can be tried without a printer.',onPress:()=>addDemo('Prusa','CORE One L','core-one-l-demo','CORE One L')})));}

 // Step 2: connect and test
 function connect(h){
  draft.hit=h;draft.driver=h.driver;draft.address=h.address;draft.test=null;
  body.replaceChildren(head(`Connect the ${BRAND[h.driver]}`,`${String(h.address).replace(/^https?:\/\//,'')} · Step 2 of 4`,find));
  const cred=draft.credentials={};const inputs=[];
  const add=(k,label,value,note,opts)=>{const i=text(value,opts?.placeholder||'',{type:opts?.type,aria:label});i.addEventListener('input',()=>{cred[k]=i.value.trim();draft.test=null;result.replaceChildren();nextBtn.hidden=true;});if(value)cred[k]=value;inputs.push(field(label,i,note));return i;};
  if(h.driver==='prusalink'){
   add('user','User name','maker','Set in the printer\'s own settings, under PrusaLink. Usually maker.');
   add('password','Password','','Shown next to the user name on the printer. It stays on this computer.',{type:'password'});
  }else if(h.driver==='flashforge'){
   add('check_code','Check code','','On the printer\'s own screen. Capital and small letters matter. Turn LAN mode on there for live data.',{placeholder:'8 letters and numbers'});
   if(h.serial)cred.serial=h.serial;else add('serial','Serial number','','Printed on the printer, starts with SN. Optional.');
  }else{
   add('api_key','API key','','Leave empty unless Moonraker asks for one.');
  }
  let cam=null,camBox=document.createElement('span');if(h.driver==='prusalink'){if(h.camera_hint&&!draft.camera)draft.camera=h.camera_hint;cam=text(draft.camera,'rtsp://<camera address>/live',{aria:'Camera address'});cam.addEventListener('input',()=>{draft.camera=cam.value.trim();draft.test=null;});const camField=field('Camera address (optional)',cam,h.camera_hint?'Found on this network: the Buddy3D camera. Clear it to add the printer without one.':'A Prusa Buddy3D camera shows its stream here once it is switched on at the printer.');camBox=camField.querySelector('.nx-fm-note');inputs.push(camField);}
  const result=el('div','nx-fm-resultbox');
  // Which Prusa is it (F09): the table's machine, preselected from what the printer says; it sets the defaults the queue and the 3D model start from.
  const prusaPick=r=>{
   const box=el('div','nx-fm-prusa'),facts=el('p','nx-note'),why=el('p','nx-note nx-fm-prusa-why');box.append(el('p','nx-note','Pick your model so Stella Scope starts with its plate, nozzle, name in sliced files and 3D model. A Prusa that is not listed can be added without one.'));
   prusaTable().then(ms=>{
    const hint=String(r.model||draft.hit?.model||'').toLowerCase();let cur=draft.prusa&&ms.find(m=>m.id===draft.prusa.id)||ms.find(m=>hint&&m.short.toLowerCase()===hint)||null;
    const show=m=>{draft.prusa=m;facts.textContent=m?prusaFacts(m):'No defaults: set the plate and nozzle under Printers afterwards.';why.hidden=!(m&&!m.cad_model);why.textContent=m&&!m.cad_model?'No 3D model yet. '+m.no_model:'';
     if(m&&!draft.camera&&!h.camera_hint)camBox.textContent=m.camera;else if(m)camBox.textContent=h.camera_hint?'Found on this network: the Buddy3D camera. Clear it to add the printer without one.':m.camera;};
    const pick=choice([{value:'',label:'Another Prusa, or I am not sure'},...ms.map(m=>({value:m.id,label:m.name}))],cur?cur.id:'',v=>show(ms.find(m=>m.id===v)||null),{label:'Which Prusa is it'});
    box.append(pick,facts,why);show(cur);});
   result.append(section('Which Prusa is it?',box));};
  const test=button({icon:'i-play',label:'Test connection',kind:'primary',size:'row',title:'Test connection',does:'Reads from the printer once to check the address and login. Nothing is saved.',onPress:async b=>{
    if(b.dataset.busy!==undefined)return;busy(test,true);result.replaceChildren(el('p','nx-note','Connecting…'));nextBtn.hidden=true;
    try{const r=await call('POST','/api/printers/test',{driver:h.driver,address:h.address,credentials:cred,camera:draft.camera||''},token);
     draft.test=r;result.replaceChildren();
     if(!r.ok){const c=el('div','nx-fm-result');c.dataset.tone='alarm';c.append(icon('i-warn'),el('p','',sentence(r.error)||'The printer did not answer.'));result.append(c);return;}
     const c=el('div','nx-fm-result');c.dataset.tone='ok';c.append(icon('i-check'));const t=el('div','nx-fm-resulttext');
     const n=r.temps?.nozzle,b=r.temps?.bed;
     t.append(el('b','',`Connected to a ${r.brand} printer`),el('small','',[r.state?`State: ${sampleState(r.state)}`:'',typeof n==='number'?`Nozzle ${Math.round(n)}°`:'',typeof b==='number'?`Bed ${Math.round(b)}°`:''].filter(Boolean).join(' · ')));c.append(t);
     result.append(c);
     if(h.driver==='prusalink')prusaPick(r);
     result.append(section('What Stella Scope can do with this printer',capList(r.capabilities||[])));
     for(const w of r.warnings||[])result.append(el('p','nx-note',sentence(w)));
     nextBtn.hidden=false;result.scrollIntoView({block:'start',behavior:'smooth'});
    }catch(e){result.replaceChildren();const c=el('div','nx-fm-result');c.dataset.tone='alarm';c.append(icon('i-warn'),el('p','',sentence(e.message)));result.append(c);}finally{busy(test,false);}}});
  const nextBtn=button({icon:'i-arrow-right',label:'Next: name it',kind:'primary',size:'row',title:'Next',does:'Goes to naming. The printer is added when you press Add.',onPress:()=>nameIt()});nextBtn.hidden=true;
  body.append(...inputs,test,result,nextBtn);
  if(h.driver==='moonraker'&&!(h.needs||[]).includes('api_key'))setTimeout(()=>test.click(),0);}

 // Step 3: name it
 function nameIt(){
  const h=draft.hit;draft.name=draft.name||(h.driver==='prusalink'&&draft.prusa?.short)||label(h)||BRAND[h.driver];
  body.replaceChildren(head('Name your printer',`${draft.test?.brand||BRAND[h.driver]} · Step 3 of 4`,()=>connect(h)));
  const name=text(draft.name,'Printer name',{aria:'Printer name'});name.maxLength=40;name.addEventListener('input',()=>{draft.name=name.value;});
  const next=button({icon:'i-arrow-right',label:'Next: its 3D model',kind:'primary',size:'row',title:'Next',does:'Goes to choosing the 3D model, which can be skipped. The printer is added at the end.',onPress:()=>{
   const n=name.value.trim();if(!n){toast('Give the printer a name.','alarm');return;}draft.name=n;modelStep();}});
  body.append(field('Name',name,'Shown on the printer list and at the top of its page.'),section('Colour',swatches(draft.accent,c=>{draft.accent=c;})),next);}

 // Step 4: its 3D model (polish WO-05, 2026-10-18): the machine, then what is fitted, with each licence shown before any file is fetched.
 // Skippable: a printer with no model shows its camera and live numbers. The printer is added when this step ends.
 function modelStep(){
  draft.cad=null;
  const pm=draft.hit?.driver==='prusalink'?draft.prusa:null;
  modelFlow(body,{token,printer:null,step:'Step 4 of 4',hint:pm?.short||draft.test?.model||draft.hit?.model||'',start:pm?.cad_model?{cad_model:pm.cad_model}:{},intro:pm&&!pm.cad_model?`No 3D model yet for the ${pm.short}. ${pm.no_model}`:'',onBack:()=>nameIt(),onDone:sel=>{draft.cad=sel;return create();}});}

 // The printer is added here. Beyond the free five the sixth printer starts a thirty-day trial; only a locked farm (trial
 // ended, no licence) asks for a licence before adding. "Not now" still adds it.
 async function create(){
  const h=draft.hit;
  try{const entry={name:draft.name,driver:h.driver,address:h.address,accent:draft.accent,credentials:draft.credentials};if(draft.camera)entry.camera=draft.camera;if(draft.cad?.cad_model)entry.cad_model=draft.cad.cad_model;
   if(h.driver==='prusalink'&&draft.prusa){const p=draft.prusa;if(p.slicer_model)entry.slicer_model=p.slicer_model;if(p.nozzle_mm)entry.nozzle_mm=p.nozzle_mm;if(p.bed_mm)entry.bed_size_mm=p.bed_mm;}
   const go=async()=>{try{const r=await call('POST','/api/printers',entry,token);
     if(draft.cad?.cad_options?.length){try{await call('PATCH','/api/printers/'+r.printer.id,{cad_options:draft.cad.cad_options},token);}catch(e){toast('The printer is added, but what is fitted was not saved: '+e.message,'alarm');}}
     done(r.printer);}catch(e){toast(e.message,'alarm');}};
   // 2026-10-18: the licence state is read here (the 2026-10-17 change left `lic` undeclared, so every Add printer ended in "lic is not defined").
   const lic=await licenceState();
   if(lic&&lic.state==='locked'&&!licenceDismissed()){licenceScreen(body,lic,go,()=>modelStep());return;}
   await go();}
  catch(e){toast(e.message,'alarm');}}

 function done(p){
  body.replaceChildren(head('Printer added'));
  const c=el('div','nx-fm-result');c.dataset.tone='ok';c.append(icon('i-check'));const t=el('div','nx-fm-resulttext');t.append(el('b','',p?.name||draft.name),el('small','',draft.cad?.cad_model?'Recording has started. Its 3D model is set.':'Recording has started.'));c.append(t);
  // F06 (2026-10-19): the sixth printer starts the 30-day trial and nothing said so; the farm page shows no note while it runs.
  const trial=el('p','nx-note');trial.hidden=true;licenceState().then(st=>{if(st&&st.state==='trial'){trial.textContent=`This is printer number ${st.printers}. Stella Scope is free for ${st.free_printers}, so a trial has started: ${Math.ceil(st.trial_days_left??0)} days before a farm licence is needed. Nothing is switched off, and the Printers list shows the licence.`;trial.hidden=false;}});
  body.append(c,trial,button({icon:'i-nav-hud',label:'Open '+(p?.name||draft.name),kind:'primary',size:'row',title:'Open it',does:'Opens the new printer.',onPress:()=>{location.href='/p/'+p.id;}}),
   button({icon:'i-printers',label:'Back to printers',size:'row',title:'Printers',does:'Shows every printer in the farm.',onPress:()=>drawList(body)}));}

 find();}

// ---- one printer's settings -------------------------------------------------------------------
function settings(p,token,changed){
 openModal({title:p.name||p.id,icon:'i-printers',build:(body,close)=>{
  const draft={name:p.name||'',accent:p.accent||ACCENTS[0][0],camera:p.camera||''};
  const name=text(draft.name,'Printer name',{aria:'Printer name'});name.maxLength=40;
  body.append(field('Name',name),section('Colour',swatches(draft.accent,c=>{draft.accent=c;})));
  let cam=null;if(p.driver==='prusalink'){cam=text(draft.camera,'rtsp://<camera address>/live',{aria:'Camera address'});body.append(field('Camera address',cam,'Leave empty for none.'));}
  // What the queue needs to know to send a sliced file here.
  let nzTurned=false,bedTurned=false;
  const nz=numberInput({label:'Nozzle',unit:' mm',min:.1,max:1.5,step:.05,turn:300,value:p.nozzle_mm??.4,color:'#3D8BFF',color2:'#9EC5FF',size:96,onSet:()=>{nzTurned=true;}}),sm=text(p.slicer_model||'','COREONEL',{aria:'What the slicer calls this printer'}),mat=text(p.material||'','PETG',{aria:'Material loaded'});
  const bedX=numberInput({label:'Width',unit:' mm',min:50,max:2000,step:1,turn:900,value:p.bed_mm?.[0]??300,color:'var(--accent)',color2:'var(--accent-2)',size:96,onSet:()=>{bedTurned=true;}}),bedY=numberInput({label:'Depth',unit:' mm',min:50,max:2000,step:1,turn:900,value:p.bed_mm?.[1]??300,color:'var(--accent)',color2:'var(--accent-2)',size:96,onSet:()=>{bedTurned=true;}});
  const plateBox=el('div','nx-q-stage');
  // Smart plug (2026-10-13): local plugs only. The address is shelly://, tasmota:// or kasa:// and the host.
  const plug=text(p.plug?.spec||'','shelly://192.168.1.50',{aria:'Smart plug address'});
  const plugFound=el('div','nx-plug-found');
  const plugFind=button({icon:'i-search',label:'Find plugs',size:'row',title:'Find plugs',does:'Looks for Shelly, Tasmota and Kasa plugs on this network. It only reads.',onPress:async b=>{
   if(b.dataset.busy!==undefined)return;busy(b,true);plugFound.replaceChildren(el('p','nx-note','Looking…'));
   try{const r=await call('POST','/api/plugs/discover',{},token);plugFound.replaceChildren();
    if(!r.plugs.length)plugFound.append(el('p','nx-note','No Shelly, Tasmota or Kasa plug answered on this network.'));
    for(const f of r.plugs)plugFound.append(button({icon:'i-power',label:`${f.name} · ${f.host}`,size:'row',title:'Use this plug',does:`${f.kind} plug at ${f.host}.`,onPress:()=>{plug.value=f.spec;plugFound.replaceChildren(el('p','nx-note',`${f.name} chosen. Save to keep it.`));}}));}
   catch(e){plugFound.replaceChildren(el('p','nx-note',e.message));}finally{busy(b,false);}}});
  body.append(modelSection(p,token,()=>{close();changed();},close));
  body.append(section('Queue',gaugeField('Nozzle fitted',[nz],'A file sliced for another nozzle is never sent here.'),
   field('Called in the slicer',sm,'The printer name inside your sliced files, for example COREONEL or Anycubic Kobra X. Files for another machine are held back.'),
   field('Material loaded',mat,'Optional. A file for another material is held back.'),plateBox),
  section('Smart plug',field('Plug address',plug,'Shelly, Tasmota and TP-Link Kasa plugs work here, on this network. Amazon Smart Plugs and most Tuya plugs only talk to their own app, so they cannot be read or switched from here. Leave empty for none.'),plugFind,plugFound),
  section('Build plate',gaugeField('Bed width and depth',[bedX,bedY],'A file sliced for a bigger plate is never sent here. New printers take this from the machine itself when it reports it.')));
  (async()=>{
   const st=await fetch(`/api/p/${p.id}/plate`,{cache:'no-store'}).then(r=>r.ok?r.json():null).catch(()=>null);
   if(!st)return;
   const ctlTok=await fetch(`/api/p/${p.id}/controls`,{cache:'no-store'}).then(r=>r.json()).then(j=>j.enabled&&j.token?j.token:null).catch(()=>null);
   plateBox.append(el('p','nx-note',st.has_reference?'A picture of the empty plate is saved. Before a queued file starts, the camera is compared with it.':'No picture of the empty plate is saved, so nothing is compared before a start; the tap alone releases the next file.'));
   plateBox.append(button({icon:'i-camera',label:st.has_reference?'Change the plate check':'Set up the plate check',size:'row',title:'Plate check',does:'Captures the empty plate and marks where the plate is.',onPress:()=>plateSetup({id:p.id,name:p.name||p.id})}));
   const auto=segmented(['off','on'],st.auto_start?'on':'off',async v=>{if(!ctlTok)return;try{await call('PATCH',`/api/p/${p.id}/plate`,{auto_start:v==='on'},ctlTok);}catch(e){toast(e.message,'alarm');}},v=>v==='on'?'Start next by itself':'Wait for me to press Start');
   plateBox.append(section('After a passing check',auto));})();
  // F06 (2026-10-19): a backup restored on a new computer says "Type the login for X in Printers", and Settings had nowhere to type it ("remove the printer and add it again").
  if(p.driver!=='demo'){const kind={prusalink:[['user','User name','maker','Usually maker.'],['password','Password','','Shown next to the user name on the printer.','password']],flashforge:[['check_code','Check code','','On the printer\'s own screen. Capital and small letters matter.']],moonraker:[['api_key','API key','','Only if Moonraker asks for one.']]}[p.driver]||[];
   if(kind.length){const vals={},boxes=kind.map(([k,l,ph,note,type])=>{const i=text('',ph,{type,aria:l});i.addEventListener('input',()=>{vals[k]=i.value.trim();});return field(l,i,note);});
    const lmsg=el('p','nx-note');lmsg.setAttribute('role','status');
    const lsave=button({icon:'i-check',label:'Save the login',size:'row',title:'Save the login',does:'Keeps the login on this computer only (never in the settings file) and starts this printer again on it. Its recordings stay.',onPress:async b=>{
     if(b.dataset.busy!==undefined)return;if(!Object.values(vals).some(v=>v)){lmsg.textContent='Type the login first.';return;}
     busy(lsave,true);lmsg.textContent='Saving the login and starting the printer again…';
     try{await call('PATCH','/api/printers/'+p.id,{credentials:vals},token);toast('Login saved. The printer is starting again on it.','ok');close();changed();}
     catch(e){lmsg.textContent=sentence(e.message);}finally{busy(lsave,false);}}});
    body.append(section('Login',el('p','nx-note',p.login_saved?'A login is saved on this computer. Type a new one to replace it; what you type is kept here and never shown again.':'No login is saved for this printer yet. Type it here; it is kept on this computer and never shown again.'),...boxes,lsave,lmsg));}}
  body.append(el('p','nx-note',`${p.brand||BRAND[p.driver]} at ${String(p.address||'').replace(/^https?:\/\//,'')}. To change the address, remove the printer and add it again; its recordings are kept.`));
  const msg=el('p','nx-note');
  const save=button({icon:'i-check',label:'Save',kind:'primary',size:'row',title:'Save',does:'Saves the name, colour and camera.',onPress:async b=>{
   const n=name.value.trim();if(!n){msg.textContent='Give the printer a name.';return;}
   const nzv=p.nozzle_mm==null&&!nzTurned?null:nz.get(),setBed=p.bed_mm!=null||bedTurned,bx=bedX.get(),by=bedY.get();
   if(b.dataset.busy!==undefined)return;busy(save,true);
   try{const patch={name:n,accent:draft.accent,slicer_model:sm.value.trim(),material:mat.value.trim(),nozzle_mm:nzv};if(setBed)patch.bed_size_mm=[bx,by];if(cam)patch.camera=cam.value.trim();patch.plug=plug.value.trim()||null;await call('PATCH','/api/printers/'+p.id,patch,token);toast('Saved.','ok');close();changed();}
   catch(e){msg.textContent=e.message;}finally{busy(save,false);}}});
  body.append(save,msg);
  const row=el('div','nx-fm-remove');
  const rm=button({icon:'i-close',label:'Remove this printer',kind:'danger',size:'row',title:'Remove',does:'Takes the printer out of the farm. Its recordings are kept as a dated file.',onPress:()=>{
    row.replaceChildren(el('p','nx-note',`Remove ${p.name||p.id}? Its recordings are kept in a dated file next to the others. Its login is deleted.${p.default?' The next printer becomes the first one; with none left, Stella Scope is as it was on the first day.':''}`),
     button({icon:'i-close',label:'Yes, remove it',kind:'danger',size:'row',title:'Remove',does:'Removes the printer now.',onPress:async()=>{try{await call('DELETE','/api/printers/'+p.id,undefined,token);toast('Removed.','ok');close();changed();}catch(e){msg.textContent=e.message;}}}),
     button({icon:'i-back',label:'Keep it',size:'row',title:'Keep',does:'Leaves the printer as it is.',onPress:()=>{row.replaceChildren(rm);}}));}});
  row.append(rm);body.append(row);}});}
