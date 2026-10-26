// The print report (2026-10-26, "build those five": timelapse and print reports). One print: how it ended, how long, the filament in metres,
// grams and money (from the material's own density and price in Materials), the plate and the material it was printed on, what the printer said
// while it ran, and its timelapse played back from the pictures the service kept (timelapse.rs). Nothing here is guessed: a figure with no
// source is a dash, with the reason.
import {el,icon,button,openSheet,section} from './next-ui.js';
import {pageCtx,u06} from './next-pages.js';
import {fmtDuration,fileTitle} from './next-core.js';

const FIL_MM=1.75;   // the filament every printer here takes; grams = length × cross-section × density
export function grams(mm,density){if(!(mm>0)||!(density>0))return null;const r=FIL_MM/20;return mm/10*Math.PI*r*r*density;}
export function cost(g,priceKg){return g>0&&priceKg>0?g/1000*priceKg:null;}
// what was recorded at a moment: the last record at or before it (the plate log, a loaded material)
export function at(log,t){let hit=null;for(const x of log||[])if(Number.isFinite(x?.at)&&x.at<=t+60)hit=x;return hit;}
// the timelapse of a print: started within ten minutes of the print's start, or the same file name
export function matchTimelapse(list,job){const t=job.start_time||0,base=String(job.filename||'').split('/').pop().replace(/\.b?gcode$/i,'');
 return (list||[]).find(s=>Math.abs((s.started||0)-t)<600)||(base?(list||[]).find(s=>String(s.file||'').split('/').pop().replace(/\.b?gcode$/i,'')===base&&Math.abs((s.started||0)-t)<6*3600):null)||null;}
const WORD={completed:'Finished',complete:'Finished',stopped:'Stopped',printing:'Printing',cancelled:'Cancelled',error:'Failed',klippy_shutdown:'Klipper stopped',klippy_disconnect:'Disconnected',in_progress:'Printing',interrupted:'Interrupted','recorded start':'No end recorded'};

let tlCache=null,tlAt=0;
async function timelapses(){if(tlCache&&Date.now()-tlAt<20000)return tlCache;try{const r=await fetch('/api/timelapse',{cache:'no-store'});tlCache=r.ok?(await r.json()).prints||[]:[];}catch{tlCache=[];}tlAt=Date.now();return tlCache;}

export function player(s){const box=el('div','nx-tlp'),img=el('img','nx-tlp-img');img.alt='Timelapse of the print';const n=Math.max(0,s.frames|0);
 const range=el('input','nx-tlp-range');range.type='range';range.min='1';range.max=String(Math.max(1,n));range.value='1';range.setAttribute('aria-label','Picture');
 const at_=el('small','nx-tlp-at'),play=button({icon:'i-play',size:'icon',title:'Play',does:'Plays the pictures in order.'}),speeds=[8,15,30];let fps=15,cur=1,timer=0;
 const sp=button({label:'15 fps',size:'row',kind:'ghost',title:'Speed',does:'Pictures a second.',onPress:()=>{fps=speeds[(speeds.indexOf(fps)+1)%speeds.length];sp.querySelector('.nx-lbl').textContent=fps+' fps';if(timer){stop();start();}}});
 const show=i=>{cur=Math.min(Math.max(1,i),n);img.src=`/api/timelapse/${encodeURIComponent(s.id)}/${cur}.jpg`;range.value=String(cur);at_.textContent=`${cur} / ${n}`;};
 const stop=()=>{clearInterval(timer);timer=0;play.querySelector('use')?.setAttribute('href','#i-play');};
 const start=()=>{if(cur>=n)show(1);timer=setInterval(()=>{if(!box.isConnected){stop();return;}if(cur>=n){stop();return;}show(cur+1);},1000/fps);play.querySelector('use')?.setAttribute('href','#i-pause');};
 play.addEventListener('click',()=>timer?stop():start());range.addEventListener('input',()=>{stop();show(+range.value);});
 // the next pictures load ahead, so playback does not stutter on a slow disk
 img.addEventListener('load',()=>{for(let k=1;k<=3&&cur+k<=n;k++){const pre=new Image();pre.src=`/api/timelapse/${encodeURIComponent(s.id)}/${cur+k}.jpg`;}});
 const bar=el('div','nx-tlp-bar');bar.append(play,range,at_,sp);box.append(img,bar);if(n)show(1);return box;}

export async function printReport(job){const ctx=pageCtx();if(!ctx)return;const p=ctx.state.presets||{},list=await timelapses();
 openSheet({title:'Print report',icon:'i-chart',wide:true,eyebrow:'ONE PRINT',build:body=>{u06(body);body.classList.add('nx-report');
  const t0=job.start_time||0,t1=job.end_time||(t0&&job.total_duration?t0+job.total_duration:null);
  const hero=el('div','nx-rep-hero'),img=el('img','nx-rep-thumb');img.alt='';img.addEventListener('error',()=>{img.style.visibility='hidden';});if(ctx.has('thumbnails'))img.src='/api/print/thumbnail?filename='+encodeURIComponent(job.filename);else img.style.visibility='hidden';
  const tx=el('div','nx-rep-t'),word=WORD[job.status]||job.status||'—',pill=el('span','nx-rep-pill',word);pill.dataset.status=job.status||'';
  tx.append(el('b','',fileTitle(job.filename)),el('small','',[t0?new Date(t0*1000).toLocaleString([], {weekday:'short',day:'numeric',month:'short',hour:'2-digit',minute:'2-digit'}):'',t1?'to '+new Date(t1*1000).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'}):''].filter(Boolean).join(' ')),pill);hero.append(img,tx);body.append(hero);
  // the figures
  const matName=job.metadata?.filament_type?String(job.metadata.filament_type).split(';')[0]:(at(p.loaded?[p.loaded]:[],t0)?.name||null),mat=(p.materials||[]).find(m=>m.name===matName)||(p.materials||[]).find(m=>matName&&m.name.toLowerCase()===String(matName).toLowerCase());
  const mm=Number(job.filament_used)>0?Number(job.filament_used):null,g=mm?grams(mm,mat?.density):null,money=g!==null?cost(g,mat?.price_kg):null,plate=at(p.sheet_log,t0);
  const figs=el('div','nx-rep-figs'),fig=(l,v,n)=>{const f=el('div','nx-rep-fig');f.append(el('small','',l),el('b','',v),el('span','',n||''));figs.append(f);};
  fig('PRINT TIME',job.print_duration?fmtDuration(job.print_duration):'—',job.total_duration&&job.print_duration&&job.total_duration-job.print_duration>60?`${fmtDuration(job.total_duration-job.print_duration)} heating and paused`:'');
  fig('FILAMENT',mm?(mm/1000).toFixed(2)+' m':'—',mm?'':'not reported by this printer');
  fig('WEIGHT',g!==null?g.toFixed(0)+' g':'—',mm&&!mat?.density?'set a density in Materials':matName?matName:'');
  fig('COST',money!==null?money.toFixed(2):'—',g!==null&&!mat?.price_kg?'set a price in Materials':money!==null?`at ${mat.price_kg}/kg`:'');
  fig('MATERIAL',matName||'—',matName?(mat?.pa!=null?`pressure advance ${mat.pa}`:''):'not recorded for this print');
  fig('PLATE',plate?.name||'—',plate?'':'not recorded');
  body.append(figs);
  // the timelapse
  const tl=matchTimelapse(list,job);
  body.append(section('Timelapse',tl&&tl.frames>0?player(tl):el('p','nx-note',tl?'The timelapse of this print has no pictures: the camera did not answer.':'No timelapse for this print. From now on every print with a camera is kept: one picture a layer.')));
  // what the printer said while it ran
  const said=el('div','nx-tl');body.append(section('While it printed',said));
  if(t0)fetch('/api/events?limit=500',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(d=>{const end=t1||Date.now()/1000,rows=(d?.events||[]).filter(e=>e.ts>=t0&&e.ts<=end&&!['command','response','connection','recorder'].includes(e.kind)).slice(0,40);
   if(!rows.length){said.append(el('p','nx-note','Nothing went wrong that the printer reported.'));return;}
   for(const e of rows){const r=el('div','nx-tl-row');r.dataset.kind=e.kind;r.append(el('span','nx-tl-time',new Date(e.ts*1000).toLocaleTimeString()),el('span','nx-tl-kind',String(e.kind).replace('_',' ')),el('span','nx-tl-msg',e.message||''));said.append(r);}}).catch(()=>said.append(el('p','nx-note','What the printer said is not available.')));
  const acts=el('div','nx-grid2');acts.append(button({icon:'i-rewind',label:'Watch it in the recording',size:'row',title:'Watch it again',does:'Rewinds the page to the start of this print in the recording.',onPress:()=>ctx.rewindTo(job.start_time)}),
   button({icon:'i-spool',label:'Materials',size:'row',title:'Materials',does:'Density and price give each print its weight and cost.',onPress:()=>ctx.openPage?.('materials')}));body.append(acts);}});}

// every timelapse kept, newest first, each opening its player
export async function sheetTimelapses(){const list=await timelapses();openSheet({title:'Timelapses',icon:'i-camera',wide:true,build:body=>{u06(body);
 if(!list.length){body.append(el('p','nx-note nx-empty-line','No timelapse yet. Every print with a camera is kept from now on: one picture a layer, the newest 40 prints.'));return;}
 const host=el('div','nx-tlp-host'),rows=el('div','nx-list');body.append(host,rows);
 const pick=s=>{host.replaceChildren(el('b','nx-tlp-title',fileTitle(s.file||s.id)),player(s));};
 for(const s of list){const r=el('button','nx-item');r.type='button';const t=el('div','nx-item-text');t.append(el('b','nx-item-title',fileTitle(s.file||s.id)),el('small','nx-item-sub',`${new Date((s.started||0)*1000).toLocaleString()} · ${s.frames||0} pictures · ${((s.bytes||0)/1e6).toFixed(0)} MB${s.outcome&&s.outcome!=='printing'?' · '+(WORD[s.outcome]||s.outcome):''}`));
  r.append(icon('i-camera'),t);r.addEventListener('click',()=>pick(s));rows.append(r);}
 pick(list[0]);}});}
