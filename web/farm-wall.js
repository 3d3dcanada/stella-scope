// Farm wall logic (polish board WO-04, 2026-10-18). Pure functions: what a tile means, how the wall is sorted, filtered and
// searched, what a tile may do, and which tiles may hold a live 3D view. farm.js draws; scripts/farm-wall-tests.mjs checks it.

// The one word a tile is known by. `clear` is a finished print whose plate has not been released yet (the queue is on and
// nobody has said "plate cleared"); without a queue a finished print is just `done`.
export function lookOf(p,qi,qon){
 const s=p.state;
 if(s==='error'||s==='offline'||s==='paused'||s==='printing')return s;
 if(qon&&(qi?.awaiting_clear||(s==='finished'&&!qi?.released)))return 'clear';
 if(s==='finished')return 'done';
 return s==='heating'?'heating':'idle';}

export const WORD={error:'Needs attention',paused:'Paused',clear:'Plate needs clearing',done:'Done',heating:'Heating',printing:'Printing',idle:'Ready',offline:'Offline'};
// The short word on a tile's chip and the icon that goes with it, so the state reads by shape and not by colour alone.
export const CHIP={error:'Attention',paused:'Paused',clear:'Clear plate',done:'Done',heating:'Heating',printing:'Printing',idle:'Ready',offline:'Offline'};
// F07 (2026-10-19): the words on a tile's ring and in its big word are one list, so they cannot say two things for one state (a cancelled print said STOPPED
// here and Cancelled beside it; an error said STOPPED). A cancelled print is CANCELLED, an error is ERROR.
export const ringWord=(look,cancelled,state)=>({printing:'PRINTED',heating:'HEATING',paused:'PAUSED',clear:cancelled?'CANCELLED':state==='finished'?'DONE':'CLEAR',done:cancelled?'CANCELLED':'DONE',idle:'READY',error:'ERROR',offline:'NO LINK'}[look]||'');
export const bigWord=(look,cancelled)=>look==='clear'?'CLEAR PLATE':look==='done'?(cancelled?'CANCELLED':'DONE'):({idle:'READY',offline:'OFF',error:'ERROR'}[look]||'');
// What a spool chip says: its brand and material, and what is left only when the weight is known (an unknown weight was shown as "0 g").
export const spoolLine=s=>[[s?.brand,s?.material].filter(Boolean).join(' '),typeof s?.grams_left==='number'&&Number.isFinite(s.grams_left)?Math.round(s.grams_left)+' g':''].filter(Boolean).join(' · ')||'Spool';
export const ICON={error:'i-warn',paused:'i-pause',clear:'i-bed',done:'i-check',heating:'i-heat',printing:'i-print',idle:'i-live',offline:'i-power'};
// Needs attention first (2026-10-18: error, paused, plate needs clearing, done), then what is working, then what is not.
export const RANK={error:0,paused:1,clear:2,done:3,heating:4,printing:5,idle:6,offline:7};
// The lifecycle order used by the "State" sort.
const FLOW={printing:0,heating:1,paused:2,clear:3,done:4,idle:5,error:6,offline:7};
export const ATTENTION=['error','paused','clear','done'];
export const brandOf=p=>String(p.brand||p.driver||'');
// "Voron 2.4 350" already says Voron: the model alone when it starts with the brand, else the brand and the model.
export const modelLine=p=>{const b=String(p.brand||p.driver||''),m=String(p.model||'');return (m&&b&&m.toLowerCase().startsWith(b.toLowerCase())?[m]:[b,m]).filter(Boolean).join(' · ');};

export const SORTS=[['attention','Attention first'],['name','Name'],['state','State'],['time','Time left'],['progress','Progress']];   // soonest first, furthest first: short, so a phone's menu shows it whole

const num=v=>typeof v==='number'&&Number.isFinite(v)?v:null;
// What "time left" and "progress" mean for a printer in the state it is in: a cancelled print, an error, an idle printer have none; a finished
// print has none left and is all the way done. (The service reports a time left from the progress for any printer that has a job.)
const timeLeft=(p,look)=>look==='printing'||look==='heating'?num(p.time_left_s):look==='done'||look==='clear'?(p.job_state==='cancelled'?null:0):null;
const progressOf=(p,look)=>['printing','heating','paused','done','clear'].includes(look)?num(p.progress):null;
const byName=(a,b)=>String(a.name||a.id).localeCompare(String(b.name||b.id),undefined,{numeric:true,sensitivity:'base'});
const nullsLast=(x,y,dir)=>x===null&&y===null?0:x===null?1:y===null?-1:dir*(x-y);

// `looks` is a Map id -> look. Returns a new array; ties always fall back to the name so the order never shimmers.
export function sortPrinters(list,looks,mode='attention'){
 const look=p=>looks.get(p.id)||'idle',left=p=>timeLeft(p,look(p)),prog=p=>progressOf(p,look(p));
 const cmp={
  attention:(a,b)=>RANK[look(a)]-RANK[look(b)]||(look(a)==='printing'?nullsLast(left(a),left(b),1):0)||byName(a,b),
  name:byName,
  state:(a,b)=>FLOW[look(a)]-FLOW[look(b)]||byName(a,b),
  time:(a,b)=>nullsLast(left(a),left(b),1)||byName(a,b),
  progress:(a,b)=>nullsLast(prog(a),prog(b),-1)||byName(a,b),
 }[mode]||byName;
 return [...list].sort(cmp);}

export function inFilter(key,p,look){
 if(!key||key==='all')return true;
 if(key==='printing')return look==='printing'||look==='heating';
 if(key==='attention')return ATTENTION.includes(look);
 if(key==='idle')return look==='idle';
 if(key==='offline')return look==='offline';
 if(key==='demo')return p.driver==='demo';
 if(key.startsWith('brand:'))return brandOf(p)===key.slice(6);
 return true;}

// Every word of the search must appear somewhere in what a person could call the printer.
export function matchesQuery(p,look,q){
 const terms=String(q||'').toLowerCase().split(/\s+/).filter(Boolean);if(!terms.length)return true;
 const hay=[p.name,p.id,brandOf(p),p.driver,p.model,p.file,WORD[look],p.detail,p.driver==='demo'?'demo':''].filter(Boolean).join(' ').toLowerCase();
 return terms.every(t=>hay.includes(t));}

// The entries of the filter menu with how many printers each holds. Brands and Demo appear only when they tell printers apart.
export function filterOptions(list,looks){
 const n=f=>list.filter(p=>inFilter(f,p,looks.get(p.id)||'idle')).length;
 const out=[['all','All printers',list.length],['printing','Printing',n('printing')],['attention','Needs attention',n('attention'),'Attention'],['idle','Ready',n('idle')],['offline','Offline',n('offline')]];
 const brands=[...new Set(list.map(brandOf).filter(Boolean))].sort();
 if(brands.length>1)for(const b of brands)out.push(['brand:'+b,b,n('brand:'+b)]);
 const demos=n('demo');if(demos>0&&demos<list.length)out.push(['demo','Demo printers',demos,'Demo']);
 // `short` is what a narrow menu says (a phone's menu is about 110 px of text)
 return out.map(([key,label,count,short])=>({key,label,short:short||label,count}));}

// What the header says: the same groups the filter uses.
export function counts(list,looks){
 const c={printing:0,attention:0,idle:0,offline:0};
 for(const p of list){const l=looks.get(p.id)||'idle';if(l==='printing'||l==='heating')c.printing++;else if(ATTENTION.includes(l))c.attention++;else if(l==='offline')c.offline++;else c.idle++;}
 return c;}

// What a printer's tile and menu offer right now, from the same facts the service reports (no guessing).
export function actionsFor(p,{qon=false,qi=null,canManage=false}={}){
 const caps=p.capabilities||{},ctrl=p.controls!==false,s=p.state,flowOn=qon&&ctrl&&caps.start_print!==false;
 return {
  pause:!!(ctrl&&caps.pause_resume&&['printing','heating'].includes(s)),
  resume:!!(ctrl&&caps.pause_resume&&s==='paused'),
  stop:!!(ctrl&&caps.cancel&&['printing','heating','paused'].includes(s)),
  clear:!!(flowOn&&((s==='finished'&&!qi?.released)||qi?.awaiting_clear)&&s!=='printing'),
  next:!!(flowOn&&qi?.next_job&&!qi?.awaiting_clear&&(s==='idle'||(s==='finished'&&qi?.released))),
  power:!!(p.plug?.spec&&canManage),
  camera:!!p.camera_url&&s!=='offline',
 };}

// U04 (2026-10-20): there is one tile size (what the medium tile was) and the S/M/L switch is gone. A press grows a tile in place to the room of four
// (two columns, two rows; the whole width on a phone) and a press again settles it. The wall screen (?tv=1) is the one place the tiles are laid out to fill the
// screen: this picks the columns (the tile closest to the shape of a tile, 1.1 wide to 1 high, with as few empty cells as it can) among those that give every tile at least 340 x 250 px, and says whether there is one.
export const tvLayout=(n,width,height)=>{
 if(!(n>0))return {cols:1,rows:1,fill:false};
 const room=Math.max(200,height),gap=14,all=[];
 for(let c=1;c<=n;c++){const r=Math.ceil(n/c),w=(width-32-(c-1)*gap)/c,h=(room-(r-1)*gap)/r;all.push({cols:c,rows:r,w,h,ok:h>=340&&w>=250,off:Math.abs(Math.log(w/h/1.1))+.15*(c*r-n)/n});}
 const pool=all.some(a=>a.ok)?all.filter(a=>a.ok):all,best=pool.reduce((a,b)=>b.off<a.off?b:a);
 return {cols:best.cols,rows:best.rows,fill:best.ok};};
// The heat gauges of a tile read the printer's own words: a heater with no number is not drawn (a printer that cannot report a bed has no bed gauge).
export const heatOf=h=>{const t=typeof h?.temp==='number'&&Number.isFinite(h.temp),g=typeof h?.target==='number'&&Number.isFinite(h.target);return {has:t||g,now:t?h.temp:null,target:g?h.target:0};};

// ---- the WebGL budget -------------------------------------------------------------------------------------------
// Chrome keeps about 16 WebGL contexts per page and silently kills the oldest beyond that, so the wall holds a fixed number
// of live 3D views. A phone gets few, a laptop more, a big screen the most; the rest of the tiles keep a still of the last frame.
export const budget3d=(width,coarse=false)=>width<=600?3:coarse||width<=1100?6:10;

// items: tiles in screen order, {id, want, visible, state}; state is 'none', 'loading' or 'live'. `want` is true while a tile is
// on screen or only just left it. Returns which tiles start and which stop so that running + loading never exceeds the budget.
// Order of preference: on-screen tiles already running (no flicker), on-screen tiles waiting (screen order), tiles that just left
// the screen and are still running (they free their place first when something on screen needs it).
export function plan3d(items,budget){
 const run=new Set(),take=t=>{if(run.size<budget)run.add(t.id);};
 const wanted=items.filter(t=>t.want);
 wanted.filter(t=>t.visible&&t.state!=='none').forEach(take);
 wanted.filter(t=>t.visible&&t.state==='none').forEach(take);
 wanted.filter(t=>!t.visible&&t.state!=='none').forEach(take);
 return {start:items.filter(t=>run.has(t.id)&&t.state==='none').map(t=>t.id),stop:items.filter(t=>!run.has(t.id)&&t.state!=='none').map(t=>t.id)};}

// What the header shows, as one string (finish-2 F04, 2026-10-19): the wall rewrites its header only when this changes (it used to tear it down and rebuild it every
// two seconds). `c` is counts(), `next` the printer that finishes first (or null) with its clock text, `first` the id the mark leads to.
export const headerKey=(c,{filter='all',queued=-1,next=null,clock='',phone=false,first=''}={})=>[c.printing,c.attention,c.idle,c.offline,filter,queued,next?next.name+'@'+clock:'',phone,first].join('|');
