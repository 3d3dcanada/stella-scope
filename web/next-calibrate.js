// Calibrate (2026-10-26: "you likely don't even have calibration, like pre-calibration setup stuff in there ... it could be built out far more").
// One page per printer: the setup in the order that matters, each step run here with the printer's own commands, its figures read back from
// what the printer answers, and kept (presets.calibrations) with the time it was done; then "before you print", a check of what a print relies on.
// A Prusa runs its calibrations itself, from its own screen: its page lists them in order and keeps when each was done.
import {el,icon,tip,button,busy,openSheet,section,toast} from './next-ui.js';
import {pageCtx,u06} from './next-pages.js';

const DAY=86400;
const ago=t=>{if(!Number.isFinite(t))return '';const s=Date.now()/1000-t;return s<3600?'just now':s<DAY?Math.round(s/3600)+' h ago':s<60*DAY?Math.round(s/DAY)+' days ago':new Date(t*1000).toLocaleDateString();};
const f3=v=>Number.isFinite(v)?(+v).toFixed(3):'—';

// Every console line the printer says, while a step listens (the same stream the Console page reads).
function listen(on){let es=null;try{es=new EventSource('/api/tick/stream');es.addEventListener('console',ev=>{try{const d=JSON.parse(ev.data);for(const l of (Array.isArray(d)?d:d.lines||[d])){const t=String(l.text||l.message||'');if(t)on(t);}}catch{}});}catch{}return ()=>{try{es?.close();}catch{}};}
// The figures Klipper prints when a calibration ends.
export const READ={
 pid:t=>{const m=/pid_Kp=([\d.]+)\s+pid_Ki=([\d.]+)\s+pid_Kd=([\d.]+)/.exec(t);return m?{kp:+m[1],ki:+m[2],kd:+m[3]}:null;},
 probeZ:t=>{const m=/probe:\s*z_offset:\s*([-\d.]+)/.exec(t);return m?{z_offset:+m[1]}:null;},
 endstopZ:t=>{const m=/stepper_z:\s*position_endstop:\s*([-\d.]+)/.exec(t);return m?{position_endstop:+m[1]}:null;},
 testZ:t=>{const m=/Z position:.*?-->\s*([-\d.]+)\s*<--/.exec(t);return m?+m[1]:null;},
 qgl:t=>{const m=/Probed points range:\s*([\d.]+)/i.exec(t);return m?{range:+m[1]}:null;},
};
// The Klipper steps, in the order a new printer is set up. `needs` is the section of printer.cfg it calls for.
export const STEPS=[
 {id:'pid_nozzle',n:'Nozzle heater',sub:'PID tune. How steadily the nozzle holds its temperature; do it first, and again after a new heater or thermistor.',time:'about 5 min',kind:'pid',heater:'extruder'},
 {id:'pid_bed',n:'Bed heater',sub:'PID tune. How steadily the bed holds its temperature, at the heat you print hottest.',time:'about 10 min',kind:'pid',heater:'heater_bed'},
 {id:'z_offset',n:'Z offset',sub:'Where the nozzle meets the bed: the paper test, in fine steps. Again after a nozzle or probe change.',time:'about 3 min',kind:'z'},
 {id:'gantry',n:'Gantry level',sub:'Levels the gantry to the bed, every time the printer is homed cold.',time:'about 1 min',kind:'qgl',needs:['quad_gantry_level']},
 {id:'mesh',n:'Bed mesh',sub:'A map of the bed\'s surface, so the first layer follows it. Again for a new plate.',time:'about 2 min',kind:'mesh',needs:['bed_mesh']},
 {id:'shaper',n:'Input shaper',sub:'Measures the frame\'s resonances so fast moves print without ringing. Needs the accelerometer.',time:'about 2 min an axis',kind:'page',page:'shaper'},
 {id:'rotation',n:'Extruder steps',sub:'Rotation distance: that 100 mm asked is 100 mm fed. Once per extruder.',time:'about 4 min',kind:'rotation'},
 {id:'pressure_advance',n:'Pressure advance',sub:'A tower whose corners show the best value, per material. Sharp corners, no bulge.',time:'one tower print',kind:'tower'},
 {id:'flow',n:'Flow',sub:'A single-wall cube measured with calipers, per material, so walls come out the width they were sliced at.',time:'one cube print',kind:'flow'},
];
// A Prusa's own calibrations, in its own menu's words (the printer runs them; Stella keeps when each was done).
export const PRUSA=[
 {id:'prusa_selftest',n:'Selftest',sub:'On the printer: Control › Selftest. Fans, heaters, the loadcell and the axes.'},
 {id:'prusa_first_layer',n:'First layer',sub:'On the printer: Control › Calibrations › First layer. Sets Live Adjust Z for the sheet on the bed.'},
 {id:'prusa_input_shaper',n:'Input shaper',sub:'On the printer: Control › Calibrations › Input shaper, where the firmware offers it.'},
 {id:'prusa_phase',n:'Phase stepping',sub:'On the printer, where offered (CORE One, XL): quieter, smoother motors.'},
 {id:'prusa_nozzle_clean',n:'Nozzle and sheet clean',sub:'A wipe of the nozzle and an IPA clean of the sheet before a first layer check.'},
];

export function pageCalibrate(){const ctx=pageCtx();if(!ctx)return;
 // a demo Klipper machine shows the Klipper setup (its steps answer as a demo printer does); a Prusa or FlashForge, its own
 const c0=ctx.caps?.()||{},klipper=ctx.has('klipper')||(c0.driver==='demo'&&!/prusa/i.test(c0.brand||'')),cal=()=>ctx.state.presets?.calibrations||{};
 const save=async(id,values,note='')=>{const p=ctx.state.presets||{};const c={...(p.calibrations||{}),[id]:{at:Date.now()/1000,values,note}};await ctx.savePresets({...p,calibrations:c});};
 // a pressure advance or flow found for a material is kept on that material too (its profile), so loading it brings it back
 const toMaterial=async(name,field,v)=>{const p=ctx.state.presets||{};if(!name||!(p.materials||[]).some(m=>m.name===name))return false;
  await ctx.savePresets({...p,materials:p.materials.map(m=>m.name===name?{...m,[field]:v}:m)});return true;};
 let cfg=null,stops=[];
 openSheet({title:'Calibrate',icon:'i-probe',wide:true,eyebrow:'SET UP THIS PRINTER',onClose:()=>{stops.forEach(f=>f());stops=[];},build:body=>{u06(body);body.classList.add('nx-cal');
  const head=el('div','nx-cal-head'),bar=el('div','nx-cal-bar'),fill=el('i'),count=el('b');bar.append(fill);head.append(count,bar);body.append(head);
  const check=el('div','nx-cal-check'),prep=el('div','nx-prep');body.append(section('Before you print',check,prep));
  const list=el('div','nx-cal-list');body.append(section(klipper?'Setup, in order':'On the printer, in order',list));
  const steps=klipper?STEPS:PRUSA,cards=new Map();
  for(const [i,s] of steps.entries()){const c=el('article','nx-cal-step');c.dataset.id=s.id;const num=el('span','nx-cal-n',String(i+1).padStart(2,'0')),txt=el('div','nx-cal-t'),st=el('div','nx-cal-st'),run=el('div','nx-cal-run');
   txt.append(el('b','',s.n),el('p','',s.sub));if(s.time)txt.append(el('small','',s.time));c.append(num,txt,st,run);list.append(c);cards.set(s.id,{c,st,run,s});}
  const paint=()=>{const rec=cal();let done=0;
   for(const {c,st,s} of cards.values()){const r=rec[s.id],skip=s.needs&&cfg&&!s.needs.some(k=>cfg[k]);c.toggleAttribute('data-done',!!r);c.toggleAttribute('data-skip',!!skip);if(r)done++;
    st.replaceChildren();if(skip){st.append(el('span','nx-cal-tag','Not on this printer'));continue;}
    if(r){st.append(el('span','nx-cal-tag nx-cal-ok',ago(r.at)));const v=r.values||{},bits=Object.entries(v).filter(([,x])=>x!==null&&x!=='').slice(0,4).map(([k,x])=>`${k.replace(/_/g,' ')} ${typeof x==='number'?(Math.abs(x)<10?f3(x):Math.round(x*10)/10):x}`);if(bits.length)st.append(el('small','',bits.join(' · ')));}
    else st.append(el('span','nx-cal-tag','Not done yet'));}
   const live=[...cards.values()].filter(x=>!x.c.hasAttribute('data-skip')).length;count.textContent=`${done} of ${live} done`;fill.style.width=(live?Math.round(done/live*100):0)+'%';paintCheck();paintPrep();};
  // ---- before you print: what a print relies on, from what this page and the others keep ----
  let meshAt=null;
  const paintCheck=()=>{const p=ctx.state.presets||{},rec=cal(),items=[];
   items.push(['Plate on the bed',p.sheet?.name?`${p.sheet.name}`:'',p.sheet?.name?'ok':'warn','Tap Plate? at the top to record it.']);
   items.push(['Filament in',p.loaded?.name||'',p.loaded?.name?'ok':'warn','Tap Filament? at the top to record it.']);
   if(klipper){items.push(['Bed mesh',meshAt?ago(meshAt):'',meshAt&&Date.now()/1000-meshAt<7*DAY?'ok':'warn',meshAt?'Older than a week: run the mesh again.':'No mesh recorded yet.']);
    items.push(['Z offset',rec.z_offset?ago(rec.z_offset.at):'',rec.z_offset?'ok':'warn','Run the Z offset below.']);
    items.push(['Heaters tuned',rec.pid_nozzle&&rec.pid_bed?'both':rec.pid_nozzle||rec.pid_bed?'one of two':'',rec.pid_nozzle&&rec.pid_bed?'ok':'warn','Tune both heaters below.']);
    if(cfg?.save_config_pending)items.push(['Waiting to be saved',(cfg.pending&&Object.keys(cfg.pending).join(', '))||'',"alarm",'Results are waiting in Klipper: press Save to the printer.']);}
   else{items.push(['First layer',rec.prusa_first_layer?ago(rec.prusa_first_layer.at):'',rec.prusa_first_layer?'ok':'warn','Run it on the printer, then mark it done below.']);}
   check.replaceChildren(...items.map(([n,v,tone,help])=>{const r=el('div','nx-cal-ci');r.dataset.tone=tone;r.append(icon(tone==='ok'?'i-check':'i-warn'),el('b','',n),el('span','',tone==='ok'?v:help));return r;}));
   if(klipper&&cfg?.save_config_pending){const sb=button({icon:'i-backup',label:'Save to the printer',kind:'primary',size:'row',cmd:'save_config',title:'SAVE_CONFIG',does:'Writes the waiting results into printer.cfg. Klipper restarts, which takes a few seconds.',onPress:async b=>{busy(b,true);await ctx.send('save_config',{});busy(b,false);setTimeout(loadCfg,6000);}});check.append(sb);}};
  // ---- the first layer, prepared in one go (2026-10-26, "build those five"): heat for the loaded material, home, level, mesh, the plate's offset ----
  const paintPrep=()=>{if(!klipper||prep.dataset.busy)return;const p=ctx.state.presets||{},mat=(p.materials||[]).find(m=>m.name===p.loaded?.name),plate=(p.sheets||[]).find(x=>x.name===p.sheet?.name);
   const steps=[['heat',mat?`Heat for ${mat.name}: nozzle ${mat.nozzle}°, bed ${mat.bed}°`:'Heat: record the filament first (Filament? at the top)',!!mat],['home','Home every axis',true],
    ['qgl','Level the gantry',!cfg||cfg.quad_gantry_level],['mesh',`Probe the bed mesh${plate?` for ${plate.name}`:''}`,!cfg||cfg.bed_mesh],['plate',plate?`Set ${plate.name}'s Z offset (${plate.measured===false?'not set yet':f3(plate.z_offset)+' mm'})`:'Plate offset: record the plate first (Plate? at the top)',!!plate&&plate.measured!==false]];
   const list=el('ol','nx-prep-list');for(const [k,t,on] of steps){const li=el('li','',t);li.dataset.k=k;if(!on)li.dataset.off='';list.append(li);}
   const go=button({icon:'i-play',label:'Prepare the first layer',kind:'primary',size:'row',title:'Prepare the first layer',does:'Runs the steps above in order, each shown before it is sent. Stops at the first one that is refused.',onPress:()=>runPrep(steps,list,go)});
   prep.replaceChildren(el('b','nx-prep-t','First layer, in one go'),list,go);};
  const runPrep=async(steps,list,go)=>{prep.dataset.busy='1';go.disabled=true;const p=ctx.state.presets||{},mat=(p.materials||[]).find(m=>m.name===p.loaded?.name),plate=(p.sheets||[]).find(x=>x.name===p.sheet?.name);
   const mark=(k,st)=>{const li=list.querySelector(`[data-k="${k}"]`);if(li)li.dataset.st=st;};let ok=true;
   for(const [k,,on] of steps){if(!on){mark(k,'skip');continue;}mark(k,'now');let r=null;
    if(k==='heat'){r=await ctx.send('set_heater',{target:'heater_bed',value:mat.bed});if(r)r=await ctx.send('set_heater',{target:'extruder',value:mat.nozzle});if(r)ctx.applyMaterial?.(mat.name);}
    else if(k==='home')r=await ctx.send('home',{});else if(k==='qgl')r=await ctx.send('qgl',{});else if(k==='mesh')r=await ctx.send('mesh',{});
    else if(k==='plate'){const z=Number(ctx.state.snapshot?.point?.z_offset),d=Number.isFinite(z)?Math.round((plate.z_offset-z)*1000)/1000:null;r=d===null||Math.abs(d)<0.0005?true:Math.abs(d)>1?null:await ctx.send('z_offset',{value:d});}
    mark(k,r?'done':'failed');if(!r){ok=false;break;}}
   delete prep.dataset.busy;go.disabled=false;toast(ok?'The first layer is prepared. Start the print.':'Stopped: a step was refused or not sent.',ok?'ok':'alarm');if(ok)await save('first_layer_prep',{plate:plate?.name||'',material:mat?.name||''},'');paint();};
  const loadCfg=async()=>{if(!klipper)return;try{const r=await fetch('/api/config/calibration',{cache:'no-store'});if(r.ok)cfg=await r.json();}catch{}paint();};
  fetch('/api/history/mesh?limit=1',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(h=>{const m=h?.meshes?.[0];meshAt=Number.isFinite(m?.ts)?m.ts:null;paint();}).catch(()=>{});
  // ---- each step's runner ----
  for(const x of cards.values()){const {run,s}=x;const start=button({icon:x.c.hasAttribute('data-done')?'i-rewind':'i-play',label:'Start',size:'row',title:s.n,does:s.sub,onPress:()=>open(x)});
   if(!klipper){start.querySelector('.nx-lbl').textContent='Mark done';const u=start.querySelector('use');if(u)u.setAttribute('href','#i-check');}run.append(start);}
  const open=x=>{const {run,s}=x;if(!klipper){save(s.id,{},'').then(()=>{toast(`${s.n} recorded as done.`,'ok');paint();});return;}
   for(const y of cards.values())if(y!==x)y.c.removeAttribute('data-open');x.c.setAttribute('data-open','');run.replaceChildren();requestAnimationFrame(()=>x.c.scrollIntoView({block:'nearest',behavior:'smooth'}));
   const close=()=>{x.c.removeAttribute('data-open');run.replaceChildren(button({icon:'i-play',label:'Start',size:'row',title:s.n,does:s.sub,onPress:()=>open(x)}));};
   const panel=el('div','nx-cal-panel');run.append(panel);const say=el('p','nx-cal-say');const acts=el('div','nx-cal-acts');panel.append(say,acts);
   const p=ctx.state.presets||{},mat=(p.materials||[]).find(m=>m.name===p.loaded?.name);
   const cancel=button({icon:'i-close',label:'Close',size:'row',kind:'ghost',title:'Close',does:'Closes this step. Anything the printer is doing goes on.',onPress:()=>{stop();close();}});
   let unlisten=null;const stop=()=>{unlisten?.();unlisten=null;};stops.push(stop);
   if(s.kind==='pid'){const nozzle=s.heater==='extruder',def=nozzle?(mat?.nozzle||245):(mat?.bed>0?mat.bed:100);let t=def;
    const val=el('b','nx-cal-big',t+'°'),less=button({icon:'i-minus',size:'icon',title:'Lower',onPress:()=>{t=Math.max(nozzle?150:40,t-5);val.textContent=t+'°';}}),more=button({icon:'i-plus',size:'icon',title:'Higher',onPress:()=>{t=Math.min(nozzle?290:120,t+5);val.textContent=t+'°';}});
    const row=el('div','nx-cal-row');row.append(el('small','',nozzle?'TUNE THE NOZZLE AT':'TUNE THE BED AT'),less,val,more);panel.insertBefore(row,say);
    say.textContent=`Tune at the heat you print at most${mat?` (${mat.name}: ${nozzle?mat.nozzle:mat.bed}°)`:''}. The heater cycles up and down a few times; leave the printer alone until it says done.`;
    const go=button({icon:'i-play',label:'Start tuning',kind:'primary',size:'row',cmd:'pid_calibrate',title:'PID_CALIBRATE',does:'Heats, cycles and measures, then reports the three PID values.',onPress:async b=>{
     stop();unlisten=listen(line=>{const v=READ.pid(line);if(v){stop();say.textContent=`Done: Kp ${v.kp}, Ki ${v.ki}, Kd ${v.kd}. Save them to printer.cfg, or keep them for now: Klipper uses them until it restarts.`;save(s.id,{...v,target:t},'').then(paint);acts.replaceChildren(saveCfg(),cancel);}});
     busy(b,true);say.textContent=`Tuning at ${t}°… the result appears here when the printer reports it.`;const r=await ctx.send('pid_calibrate',{target:s.heater,value:t});busy(b,false);if(r===null){stop();say.textContent='Not sent.';}}});
    acts.append(go,cancel);}
   else if(s.kind==='z'){const probe=cfg?.probe,cmd=probe?'probe_calibrate':'z_endstop_calibrate',zNow=el('b','nx-cal-big','—');
    say.textContent=`${probe?`With the ${probe.kind}`:'With the Z endstop'}: the printer homes, goes to the middle of the bed and waits. Slide a sheet of paper under the nozzle and step it down until the paper drags a little, then Accept.`;
    const steps=el('div','nx-cal-steps');for(const d of [-1,-0.1,-0.05,-0.01,0.01,0.05,0.1,1])steps.append(button({label:(d>0?'+':'')+d,size:'row',cmd:'testz',title:`Move ${d>0?'up':'down'} ${Math.abs(d)} mm`,does:'One step of the paper test.',onPress:()=>ctx.send('testz',{value:d})}));
    const live=el('div','nx-cal-row');live.append(el('small','','NOZZLE Z'),zNow);
    const accept=button({icon:'i-check',label:'Accept',kind:'primary',size:'row',cmd:'manual_accept',title:'ACCEPT',does:'Keeps this height as the Z offset.',onPress:()=>ctx.send('manual_accept',{})}),
     abort=button({icon:'i-close',label:'Abort',size:'row',cmd:'manual_abort',title:'ABORT',does:'Leaves the paper test without a change.',onPress:async()=>{await ctx.send('manual_abort',{});stop();close();}});
    const go=button({icon:'i-play',label:'Start the paper test',kind:'primary',size:'row',cmd,title:probe?'PROBE_CALIBRATE':'Z_ENDSTOP_CALIBRATE',does:'Home first. It lowers to the bed and waits for your steps.',onPress:async b=>{
     stop();unlisten=listen(line=>{const z=READ.testZ(line);if(z!==null)zNow.textContent=f3(z)+' mm';const v=probe?READ.probeZ(line):READ.endstopZ(line);if(v){stop();say.textContent=`Done: ${probe?'z_offset':'position_endstop'} ${f3(Object.values(v)[0])}. Save it to printer.cfg to keep it.`;save(s.id,v,'').then(paint);acts.replaceChildren(saveCfg(),cancel);}});
     busy(b,true);const r=await ctx.send(cmd,{});busy(b,false);if(r!==null){panel.insertBefore(live,acts);panel.insertBefore(steps,acts);acts.replaceChildren(accept,abort);}else stop();}});
    acts.append(go,cancel);}
   else if(s.kind==='qgl'||s.kind==='mesh'){const id=s.kind==='qgl'?'qgl':'mesh';say.textContent=s.kind==='qgl'?'Home first. The probe touches each corner and the gantry is levelled until the corners agree.':'Home (and level) first. The probe maps the bed; the map is kept with the plate on the bed.';
    const go=button({icon:'i-play',label:s.kind==='qgl'?'Level the gantry':'Probe the mesh',kind:'primary',size:'row',cmd:id,title:s.n,does:s.sub,onPress:async b=>{stop();unlisten=listen(line=>{const v=READ.qgl(line);if(v&&s.kind==='qgl')say.textContent=`Corners within ${v.range} mm…`;});
     busy(b,true);const r=await ctx.send(id,{});busy(b,false);stop();if(r!==null){await save(s.id,s.kind==='qgl'?{}:{plate:p.sheet?.name||''},'');paint();say.textContent='Done. The result is on the Gantry & bed page.';}}});
    acts.append(go,button({icon:'i-nav-leveling',label:'Gantry & bed page',size:'row',title:'Gantry & bed',does:'Every run, the map in 3D, and compare two.',onPress:()=>ctx.openPage?.('level')}),cancel);}
   else if(s.kind==='page'){say.textContent='The Input shaper page runs the measurement and shows the graphs and the recommended shaper.';acts.append(button({icon:'i-nav-signals',label:'Open Input shaper',kind:'primary',size:'row',title:'Input shaper',does:'Measure X, Y, and the belts.',onPress:()=>ctx.openPage?.('shaper')}),button({icon:'i-check',label:'Mark done',size:'row',title:'Mark done',does:'Records it as done today.',onPress:async()=>{await save(s.id,cfg?.input_shaper?{x:cfg.input_shaper.x||'',fx:cfg.input_shaper.fx??null,y:cfg.input_shaper.y||'',fy:cfg.input_shaper.fy??null}:{},'');paint();close();}}),cancel);}
   else if(s.kind==='rotation'){const old=cfg?.extruder?.rotation_distance??null,temp=mat?.nozzle||240;let left=null;
    say.textContent=`Heat to ${temp}°. Mark the filament 120 mm above where it enters the extruder. Stella feeds 100 mm slowly (two 50 mm moves), then you measure what is left to the mark.`;
    const inp=el('input','nx-input');inp.type='number';inp.step='0.1';inp.placeholder='mm left to the mark';inp.setAttribute('aria-label','Millimetres left to the mark');
    const out=el('p','nx-cal-say');
    const heat=button({icon:'i-heat',label:`Heat to ${temp}°`,size:'row',cmd:'set_heater',title:'Heat',does:'Heats the nozzle for the feed.',onPress:()=>ctx.send('set_heater',{target:'extruder',value:temp})}),
     feed=button({icon:'i-filament-load',label:'Feed 100 mm',kind:'primary',size:'row',cmd:'extrude',title:'Feed 100 mm',does:'Two 50 mm feeds at 2 mm/s (Klipper refuses one move over 50 mm).',onPress:async b=>{busy(b,true);if(await ctx.send('extrude',{value:50,speed:2}))await ctx.send('extrude',{value:50,speed:2});busy(b,false);}});
    const calc=button({icon:'i-check',label:'Work it out',size:'row',title:'Work it out',does:'The new rotation distance from what was fed.',onPress:async()=>{left=parseFloat(inp.value);if(!Number.isFinite(left)||left<0||left>120){out.textContent='Type the millimetres left to the mark (0 to 120).';return;}
     const fed=120-left;if(!old){out.textContent=`It fed ${fed.toFixed(1)} mm of 100. Stella could not read the rotation distance from printer.cfg; multiply yours by ${(fed/100).toFixed(4)}.`;return;}
     const nw=old*fed/100;out.textContent=`It fed ${fed.toFixed(1)} mm of 100. New rotation distance: ${nw.toFixed(4)} (was ${old}). Write it under [extruder] rotation_distance in printer.cfg and restart.`;await save(s.id,{fed,old,new:Math.round(nw*10000)/10000},'');paint();}});
    const row=el('div','nx-cal-row');row.append(inp,calc);panel.insertBefore(row,acts);panel.insertBefore(out,acts);acts.append(heat,feed,cancel);}
   else if(s.kind==='tower'){let start=0,factor=0.005;const name=p.loaded?.name||'';
    say.textContent=`Slice a square tower (single wall, about 50 mm high, no top) for ${name||'the material loaded'}, and send it to the printer. Stella sets the tower to raise pressure advance with height, then starts it. Afterwards, find the height where the corners look sharpest.`;
    const fsel=el('select','nx-input');fsel.setAttribute('aria-label','Tower file');fsel.append(el('option','','Choose the tower file…'));fetch('/api/files',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(d=>{for(const f of (d?.files||d?.result||[]).slice(0,80)){const n=f.path||f.filename||f.name;if(!n)continue;const o=el('option','',n);o.value=n;fsel.append(o);}}).catch(()=>{});
    const ds=el('select','nx-input');for(const [v,l] of [['dd','Direct drive: 0 to 0.25'],['bowden','Bowden: 0 to 1.0']]){const o=el('option','',l);o.value=v;ds.append(o);}ds.addEventListener('change',()=>{factor=ds.value==='dd'?0.005:0.02;});
    const h=el('input','nx-input');h.type='number';h.step='0.5';h.placeholder='best height, mm';h.setAttribute('aria-label','Best height in millimetres');const out=el('p','nx-cal-say');
    const go=button({icon:'i-play',label:'Set the tower and start',kind:'primary',size:'row',cmd:'tuning_tower',title:'TUNING_TOWER + start',does:'Sends TUNING_TOWER, then starts the file you chose.',onPress:async b=>{if(!fsel.value){out.textContent='Choose the tower file first.';return;}busy(b,true);
     if(await ctx.send('tuning_tower',{params:{start:String(start),factor:String(factor)}}))await ctx.send('print_start',{filename:fsel.value});busy(b,false);out.textContent='Printing the tower. Come back when it is done and type the best height.';}});
    const calc=button({icon:'i-check',label:'Work it out',size:'row',title:'Work it out',does:'Pressure advance = start + factor × height.',onPress:async()=>{const mm=parseFloat(h.value);if(!Number.isFinite(mm)||mm<0||mm>200){out.textContent='Type the height in millimetres.';return;}
     const pa=Math.round((start+factor*mm)*10000)/10000;out.textContent=`Pressure advance ${pa} for ${name||'this material'}. Put it in your slicer's filament settings (or printer.cfg), or use it now.`;
     acts.prepend(button({icon:'i-check',label:`Use ${pa} now`,size:'row',cmd:'pressure_advance',title:'SET_PRESSURE_ADVANCE',does:'Until Klipper restarts.',onPress:()=>ctx.send('pressure_advance',{value:pa})}));
     const prev=cal().pressure_advance?.values||{};await save(s.id,{...prev,[name||'material']:pa},'');if(await toMaterial(name,'pa',pa))out.textContent+=` Kept on ${name}: loading it sets it again.`;paint();}});
    const r1=el('div','nx-cal-row');r1.append(fsel,ds);const r2=el('div','nx-cal-row');r2.append(h,calc);panel.insertBefore(r1,acts);panel.insertBefore(r2,acts);panel.insertBefore(out,acts);acts.append(go,cancel);}
   else if(s.kind==='flow'){const name=p.loaded?.name||'';say.textContent='Print a single-wall cube (vase mode, one perimeter). Measure the wall with calipers at four places, away from the corners, and type them with the line width it was sliced at.';
    const w=el('input','nx-input');w.type='number';w.step='0.01';w.value=String(Math.round((cfg?.extruder?.nozzle_diameter||0.4)*1.125*100)/100);w.setAttribute('aria-label','Sliced line width, mm');
    const ms=[0,1,2,3].map(i=>{const m=el('input','nx-input');m.type='number';m.step='0.01';m.placeholder='wall '+(i+1);m.setAttribute('aria-label','Measured wall '+(i+1));return m;}),out=el('p','nx-cal-say');
    const calc=button({icon:'i-check',label:'Work it out',kind:'primary',size:'row',title:'Work it out',does:'Flow ratio = sliced width ÷ measured width.',onPress:async()=>{const want=parseFloat(w.value),got=ms.map(m=>parseFloat(m.value)).filter(Number.isFinite);if(!(want>0)||got.length<2){out.textContent='Type the line width and at least two measurements.';return;}
     const avg=got.reduce((a,b)=>a+b,0)/got.length,ratio=Math.round(want/avg*1000)/1000;out.textContent=`Walls average ${avg.toFixed(3)} mm. Flow ratio ${ratio} for ${name||'this material'} (multiply your slicer's flow by it).`;
     const prev=cal().flow?.values||{};await save(s.id,{...prev,[name||'material']:ratio},'');if(await toMaterial(name,'flow',ratio))out.textContent+=` Kept on ${name}.`;paint();}});
    const r1=el('div','nx-cal-row');r1.append(el('small','','SLICED WIDTH'),w);const r2=el('div','nx-cal-row');r2.append(...ms);panel.insertBefore(r1,acts);panel.insertBefore(r2,acts);panel.insertBefore(out,acts);acts.append(calc,cancel);}};
  const saveCfg=()=>button({icon:'i-backup',label:'Save to printer.cfg',kind:'primary',size:'row',cmd:'save_config',title:'SAVE_CONFIG',does:'Writes the result into printer.cfg. Klipper restarts, which takes a few seconds.',onPress:async b=>{busy(b,true);await ctx.send('save_config',{});busy(b,false);setTimeout(loadCfg,6000);}});
  paint();loadCfg();
  tip(head,'Setup','The calibrations this printer has had, in the order a new printer is set up. Each keeps its figures and when it was done.');}});}
