'use strict';
const $ = id => document.getElementById(id);
const state = {mode:'live',cursor:null,selected:'head',meta:null,snapshot:null,series:[],events:[],rounds:[],window:900,displayTs:0,version:0,controller:null,busy:false,initialized:false,hits:[]};
const colours = {text:'#dce7ef',muted:'#92a4b5',grid:'#283643',teal:'#64d6c2',amber:'#edb56b',red:'#f0838e',bed:'#101923'};
const num = (v,d=2) => Number.isFinite(v) ? v.toFixed(d) : '—';
const pct = v => Number.isFinite(v) ? `${(v*100).toFixed(0)}%` : '—';
const yes = v => v===true || v===1 ? 'Yes' : v===false || v===0 ? 'No' : 'Unknown';
const stamp = t => Number.isFinite(t) ? new Date(t*1000).toLocaleString(undefined,{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',second:'2-digit'}) : '—';
const clock = t => new Date(t*1000).toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit',second:'2-digit'});
function el(tag,text,cls) {const e=document.createElement(tag);if(text!==undefined)e.textContent=String(text);if(cls)e.className=cls;return e;}
function geometry(){return state.snapshot?.geometry || state.meta?.geometry;}
function select(name){state.selected=name;renderDetails();renderMotors();drawMap();}
function flagged(m){return Array.isArray(m?.flags) && m.flags.length>0;}
function motorState(m){
  if(!m)return ['Unknown',''];
  if(flagged(m))return [m.flags.some(f=>['ot','s2vsa','s2vsb','s2ga','s2gb'].includes(f))?'Driver fault':'Driver warning',m.flags.includes('ot') || m.flags.some(f=>f.startsWith('s2'))?'bad':'warn'];
  if(m.energised===false)return ['Not energised',''];
  if(m.stst===null || m.stst===undefined)return ['Status unknown',''];
  return [m.stst?'Idle':'Moving',''];
}
function warningName(s){return ({otpw:'Driver warm',ot:'Driver too hot',s2vsa:'Coil A short warning',s2vsb:'Coil B short warning',s2ga:'Coil A short to ground',s2gb:'Coil B short to ground',ola:'Coil A open-load indication',olb:'Coil B open-load indication',t120:'Driver heat threshold flag',t143:'Driver heat threshold flag',t150:'Driver heat threshold flag',t157:'Driver heat threshold flag'})[s] || s;}
async function get(url,signal){const r=await fetch(url,{signal,cache:'no-store'});let v;try{v=await r.json();}catch{throw new Error('Stella Scope returned an unreadable response; check the service logs.');}if(!r.ok)throw new Error(v.error || `HTTP ${r.status}`);return v;}
async function refresh(force=false){
  if(state.busy && !force)return;
  state.controller?.abort();const controller=new AbortController();state.controller=controller;
  const version=++state.version;state.busy=true;
  try {
    const meta=await get('/api/meta',controller.signal);
    if(!state.initialized){state.initialized=true;state.window=meta.geometry.chart_window_s;$('window').value=String(state.window);if(!$('window').value){const o=el('option',`${state.window} seconds`);o.value=state.window;$('window').append(o);$('window').value=state.window;}if(meta.collector.mode==='replay'){state.mode='rewind';state.cursor=meta.last_ts;}}
    const rewind=state.mode==='rewind';const requested=rewind ? state.cursor ?? meta.last_ts ?? meta.server_time : null;
    const snapshot=await get(rewind?`/api/at?ts=${requested}`:'/api/live',controller.signal);
    const at=snapshot.requested_ts;const from=Math.max(0,at-state.window);const points=Math.min(600,meta.series_max_points??1200);
    const [series,events,rounds]=await Promise.all([
      get(`/api/series?from=${from}&to=${at}&points=${points}`,controller.signal),
      get(`/api/events?from=${from}&to=${at}`,controller.signal),
      get(`/api/qgl?from=0&to=${at}`,controller.signal)
    ]);
    if(version!==state.version)return;
    Object.assign(state,{meta,snapshot,displayTs:at,series:series.points,events:events.events,rounds:rounds.rounds});
    render();
  } catch(e){if(e.name!=='AbortError' && version===state.version){$('notice').hidden=false;$('notice').textContent=`Recorder unavailable: ${e.message} The last displayed snapshot is retained; it is not live.`;$('connection').textContent='Recorder not answering';}}
  finally{if(version===state.version)state.busy=false;}
}
function jump(ts){if(!Number.isFinite(ts))return;state.mode='rewind';state.cursor=ts;$('notice').hidden=false;$('notice').textContent='Loading selected time…';refresh(true);}
function render(){
  const {meta,snapshot}=state;const g=geometry();const sample=snapshot.sample;
  $('live').classList.toggle('active',state.mode==='live');$('rewind').classList.toggle('active',state.mode==='rewind');$('live').disabled=meta.collector.mode==='replay';
  const slider=$('scrubber');slider.disabled=meta.first_ts===null;slider.min=meta.first_ts??0;slider.max=meta.last_ts??1;slider.value=state.mode==='live'?(meta.last_ts??1):state.displayTs;
  $('start-time').textContent=meta.first_ts===null?'No recording yet':stamp(meta.first_ts);$('end-time').textContent=meta.last_ts===null?'':stamp(meta.last_ts);
  $('view-time').textContent=`${state.mode==='live'?'LIVE':'REWIND'} · ${stamp(state.displayTs)}`;
  const health=state.mode==='rewind'?snapshot.recorded_health:meta.collector;
  const stale=sample && snapshot.age_seconds>Math.max(2,g.poll_interval_ms/250);
  const mode=meta.collector.mode==='demo'?'DEMO · ':meta.collector.mode==='replay'?'OFFLINE FILE · ':'';
  $('connection').textContent=mode+(state.mode==='rewind'?'Viewing a recording':health?.connected?'Recording':'Printer not answering');
  $('connection').className=health?.connected || state.mode==='rewind'?'':'warn';
  const notices=[];
  if(!sample)notices.push('No sample was recorded at or before this time.');
  if(stale)notices.push(`No fresh sample at this time. The displayed sample is ${num(snapshot.age_seconds,1)} seconds older than the cursor.`);
  if(health?.last_error)notices.push(health.last_error);
  if(state.mode==='rewind' && !snapshot.recorded_health)notices.push('Connection health was not recorded for this historical moment.');
  if(meta.collector.mode==='demo')notices.push('Demonstration data only. No connection to the printer is made.');
  $('notice').hidden=notices.length===0;$('notice').textContent=notices.join(' ');
  const warnings=[...(health?.degraded || [])];if(!snapshot.geometry_recorded)warnings.push('No historical geometry was recorded; the supplied configuration is being used for the map.');if(g.bed_size_confirmed!==true)warnings.push('Bed dimensions are unverified; map coordinates are a display estimate.');
  $('warnings').hidden=warnings.length===0;$('warning-count').textContent=`${warnings.length} collection limitation${warnings.length===1?'':'s'}`;$('warning-list').replaceChildren(...warnings.map(s=>el('p',s)));
  $('bed-size').textContent=g.bed_size_confirmed===true?`${num(g.bed_size_mm[0],0)} × ${num(g.bed_size_mm[1],0)} mm · configured travel`:'Bed dimensions unverified · schematic top view';
  $('recording-info').textContent=sample?`Snapshot: ${stamp(sample.ts)} · 4 Hz by default · read only`:'No sample selected · read only';
  renderDetails();renderMotors();drawMap();drawCharts();renderHistory();renderConsole();
}
function renderDetails(){
  const sample=state.snapshot?.sample,d=sample?.data || {},summary=sample?.summary || {},g=geometry();if(!g)return;
  const rows=[];const add=(a,b,wide=false)=>rows.push([a,b,wide]);
  if(state.selected==='head'){
    $('detail-title').textContent='Print head';$('detail-badge').textContent='REPORTED';$('detail-badge').className='pill';
    const p=d.motion_report?.live_position;add('Position · X / Y / Z',`${num(p?.[0])} / ${num(p?.[1])} / ${num(p?.[2])} mm`);add('Speed',`${num(summary.velocity,1)} mm/s`);
    add('Nozzle · current / target',`${num(d.extruder?.temperature,1)} / ${num(d.extruder?.target,1)} °C`);add('Bed · current / target',`${num(d.heater_bed?.temperature,1)} / ${num(d.heater_bed?.target,1)} °C`);
    add('Nozzle / bed heater power',`${pct(d.extruder?.power)} / ${pct(d.heater_bed?.power)}`);add('Live height adjustment',`${num(d.gcode_move?.homing_origin?.[2],3)} mm`);
    add('Part / hotend / controller fans',`${pct(d.fan?.speed)} / ${pct(d['heater_fan hotend_fan']?.speed)} / ${pct(d['controller_fan controller_fan']?.speed)}`);add('Homed axes',typeof d.toolhead?.homed_axes==='string'?(d.toolhead.homed_axes.toUpperCase().split('').join(', ')||'None'):'Unknown');
    add('Probe triggered · last check',yes(d.probe?.last_query));add('Gantry levelling completed',yes(d.quad_gantry_level?.applied));add('Bed mesh',d.bed_mesh?.profile_name || 'None reported');add('Job state / progress',`${d.print_stats?.state || 'Unknown'} / ${pct(d.virtual_sdcard?.progress)}`);add('Print file',d.print_stats?.filename || 'None reported',true);
    $('detail-note').textContent='The probe state is its last check, not a continuously refreshed switch reading. Position is interpolated from commanded motion; it cannot confirm a slipping corner.';
  }else{
    const def=g.motors.find(m=>m.name===state.selected);const m=summary.motors?.[state.selected] || {};const [label,cls]=motorState(m);
    $('detail-title').textContent=def?.label || state.selected;$('detail-badge').textContent=label;$('detail-badge').className=`pill ${cls}`;
    add('Run current · setting',`${num(m.cur,2)} A`);add('Hold current · setting',`${num(m.hold,2)} A`);add('Drive level',Number.isFinite(m.cs)?`${m.cs} / 31`:'Unknown');add('Energised',yes(m.energised));add('Step activity',m.stst===true?'Idle · no recent step pulses':m.stst===false?'Moving · step pulses reported':'Unknown');add('Quiet drive mode',yes(m.stealth));add('Driver warnings',Array.isArray(m.flags)?(m.flags.map(warningName).join('; ')||'No flags reported'):'Unknown; driver status is unavailable',true);add('Motor temperature','Not measured',true);
    $('detail-note').textContent='Drive level is current scaling, not measured mechanical load. Step activity does not verify shaft movement. Open-coil indications can be misleading while stationary or moving quickly. No motor temperature sensor is available in this feed.';
  }
  $('details').replaceChildren(...rows.map(([a,b,wide])=>{const box=el('div',undefined,`detail-row${wide?' wide':''}`);box.append(el('dt',a),el('dd',b));return box;}));
}
function renderMotors(){const g=geometry();if(!g)return;const data=state.snapshot?.sample?.summary?.motors || {};
  $('motors').replaceChildren(...g.motors.map(def=>{const m=data[def.name] || {};const [label,cls]=motorState(m);const b=el('button',undefined,`motor-card ${cls}${state.selected===def.name?' active':''}`);b.type='button';b.setAttribute('aria-pressed',String(state.selected===def.name));b.append(el('strong',def.label),el('span',Number.isFinite(m.cs)?`${m.cs} / 31`:'—','reading'),el('small','Drive level'));const bar=el('progress');bar.max=31;if(Number.isFinite(m.cs))bar.value=m.cs;else{bar.value=0;bar.setAttribute('aria-label','Drive level unknown');}b.append(bar,el('span',label,'badge'),el('small',`Run setting ${num(m.cur)} A`));b.onclick=()=>select(def.name);return b;}));
}
function canvas(id){const c=$(id),r=c.getBoundingClientRect(),scale=devicePixelRatio||1;c.width=Math.round(r.width*scale);c.height=Math.round(r.height*scale);const x=c.getContext('2d');x.scale(scale,scale);x.font='11px system-ui';return [x,r.width,r.height];}
function drawMap(){const g=geometry();if(!g)return;const [x,w,h]=canvas('machine');const size=Math.min(w-104,h-64),left=(w-size)/2,top=(h-size)/2;const xy=(a,b)=>[left+a/g.bed_size_mm[0]*size,top+size-b/g.bed_size_mm[1]*size];
  x.fillStyle=colours.bed;x.fillRect(left,top,size,size);x.strokeStyle=colours.grid;x.lineWidth=1;
  for(let i=0;i<=7;i++){const v=i*size/7;x.beginPath();x.moveTo(left+v,top);x.lineTo(left+v,top+size);x.moveTo(left,top+v);x.lineTo(left+size,top+v);x.stroke();}
  x.strokeRect(left,top,size,size);x.fillStyle=colours.muted;x.textAlign='center';x.fillText('BACK',w/2,top-18);x.fillText('FRONT',w/2,top+size+25);state.hits=[];
  for(const c of g.corners){const cx=left+c.xy[0]*size,cy=top+(1-c.xy[1])*size;const m=state.snapshot?.sample?.summary?.motors?.[c.name];const [,cls]=motorState(m);const color=cls==='bad'?colours.red:cls==='warn'?colours.amber:colours.teal;x.fillStyle=color;x.beginPath();x.arc(cx,cy,state.selected===c.name?10:7,0,Math.PI*2);x.fill();x.fillStyle=colours.text;x.textAlign=c.xy[0]?'right':'left';x.fillText(c.label,cx+(c.xy[0]?-13:13),cy+(c.xy[1]?-12:22));state.hits.push({name:c.name,x:cx,y:cy,r:24});}
  const p=state.snapshot?.sample?.data?.motion_report?.live_position;
  if(Number.isFinite(p?.[0])&&Number.isFinite(p?.[1])){const [cx,cy]=xy(p[0],p[1]);x.strokeStyle=colours.teal;x.globalAlpha=.35;x.beginPath();x.moveTo(left,cy);x.lineTo(left+size,cy);x.moveTo(cx,top);x.lineTo(cx,top+size);x.stroke();x.globalAlpha=1;x.strokeStyle=colours.teal;x.lineWidth=2;x.beginPath();x.arc(cx,cy,8,0,Math.PI*2);x.stroke();x.fillStyle=colours.teal;x.fillRect(cx-2,cy-2,4,4);const [px,py]=xy(p[0]+g.probe_offset_mm[0],p[1]+g.probe_offset_mm[1]);x.strokeStyle=colours.amber;x.setLineDash([3,3]);x.beginPath();x.moveTo(cx,cy);x.lineTo(px,py);x.stroke();x.setLineDash([]);x.fillStyle=colours.amber;x.beginPath();x.arc(px,py,4,0,Math.PI*2);x.fill();state.hits.push({name:'head',x:cx,y:cy,r:24});}
  else{x.fillStyle=colours.muted;x.textAlign='center';x.fillText('Position not recorded',w/2,h/2);}
}
function drawCharts(){
  const end=state.displayTs,from=end-state.window,g=geometry();if(!g)return;
  for(const id of ['temperature','motion']){const [x,w,h]=canvas(id),l=43,r=40,t=14,b=27,pw=w-l-r,ph=h-t-b;const isTemp=id==='temperature';const keys=isTemp?['nozzle','nozzle_target','bed','bed_target']:['z','velocity'];const values=keys.map(k=>state.series.map(p=>p[k]).filter(Number.isFinite));const limits=isTemp?[Math.max(50,...values.flat())*1.08,0]:[Math.max(1,...values[0])*1.1,Math.max(1,...values[1])*1.1];const sx=ts=>l+(ts-from)/state.window*pw;
    x.strokeStyle=colours.grid;x.fillStyle=colours.muted;x.lineWidth=1;x.font='10px ui-monospace';
    for(let i=0;i<=4;i++){const y=t+ph*i/4;x.beginPath();x.moveTo(l,y);x.lineTo(l+pw,y);x.stroke();x.textAlign='right';x.fillText(num(limits[0]*(1-i/4),isTemp?0:1),l-7,y+3);if(!isTemp){x.textAlign='left';x.fillText(num(limits[1]*(1-i/4),0),l+pw+7,y+3);}}
    for(let i=0;i<=3;i++){x.textAlign=i===0?'left':i===3?'right':'center';x.fillText(new Date((from+state.window*i/3)*1000).toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit'}),l+pw*i/3,h-7);}
    keys.forEach((k,i)=>{x.strokeStyle=(isTemp?i<2:i===0)?colours.teal:colours.amber;x.lineWidth=1.7;x.setLineDash(isTemp && k.endsWith('target')?[5,4]:[]);x.beginPath();let prev=null;
      for(const p of state.series){if(!Number.isFinite(p[k])){prev=null;continue;}const xx=sx(p.ts),yy=t+ph-p[k]/limits[isTemp?0:i]*ph;const gap=prev===null || p.ts-prev>Math.max(2,g.poll_interval_ms/250,state.window/Math.min(600,state.meta?.series_max_points??1200)*2.2);if(gap)x.moveTo(xx,yy);else x.lineTo(xx,yy);prev=p.ts;}x.stroke();
    });x.setLineDash([]);x.strokeStyle=colours.text;x.lineWidth=1;const marker=Math.max(l,Math.min(l+pw,sx(state.displayTs)));x.beginPath();x.moveTo(marker,t);x.lineTo(marker,t+ph);x.stroke();x.fillStyle=colours.text;x.beginPath();x.moveTo(marker-4,t);x.lineTo(marker+4,t);x.lineTo(marker,t+6);x.fill();if(!state.series.length){x.fillStyle=colours.muted;x.textAlign='center';x.fillText('No samples in this time window',w/2,h/2);}
  }
}
function timeButton(ts){const b=el('button',clock(ts));b.type='button';const lo=state.meta?.first_ts,hi=state.meta?.last_ts;b.disabled=!Number.isFinite(lo)||ts<lo||ts>hi;b.title=b.disabled?'No matching snapshot was captured':'Rewind to this event';b.onclick=()=>jump(ts);return b;}
function renderHistory(){const rows=[...state.rounds].sort((a,b)=>b.run_id-a.run_id||a.round-b.round);$('qgl-count').textContent=`${rows.length} rounds shown`;const body=$('qgl');body.replaceChildren();if(!rows.length){const tr=el('tr'),td=el('td','No levelling rounds recorded before this moment.','empty');td.colSpan=8;tr.append(td);body.append(tr);return;}for(const r of rows){const tr=el('tr');tr.append(el('td',`${r.run_id} / ${r.round+1}`));const time=el('td');time.append(timeButton(r.ts));tr.append(time,el('td',`${num(r.range_mm,4)} mm`));for(const k of ['z0','z1','z2','z3'])tr.append(el('td',num(r[k],5)));const result=el('td',r.pass?'PASS':'FAIL',r.pass?'badge':'warn');result.title=`This round only; comparison tolerance ${num(r.tolerance_mm,4)} mm`;tr.append(result);body.append(tr);}}
function renderConsole(){const root=$('console');$('event-count').textContent=`${state.events.length} messages · selected chart window`;root.replaceChildren();if(!state.events.length){root.append(el('p','No messages in this time window.','empty'));return;}for(const e of state.events){const bad=/error|fail|offline|position_warning/.test(e.kind)||/!!|Retries aborting:/i.test(e.message);const row=el('div',undefined,`console-entry ${bad?'bad':e.kind==='driver_warning'?'warn':''}`);row.append(timeButton(e.ts),el('pre',`${e.kind} · ${e.message}`));root.append(row);}}
$('live').onclick=()=>{state.mode='live';state.cursor=null;refresh(true);};$('rewind').onclick=()=>jump(state.snapshot?.sample?.ts??state.meta?.last_ts);
let scrubTimer;$('scrubber').oninput=e=>{clearTimeout(scrubTimer);const value=Number(e.target.value);scrubTimer=setTimeout(()=>jump(value),65);};
document.querySelectorAll('[data-back]').forEach(b=>b.onclick=()=>{const latest=state.mode==='rewind'?state.displayTs:state.meta?.last_ts;jump(Math.max(state.meta?.first_ts??0,latest-Number(b.dataset.back)));});
$('head-select').onclick=()=>select('head');$('window').onchange=e=>{state.window=Number(e.target.value);refresh(true);};
$('machine').onclick=e=>{const r=e.currentTarget.getBoundingClientRect(),x=e.clientX-r.left,y=e.clientY-r.top;const hit=state.hits.find(p=>Math.hypot(p.x-x,p.y-y)<p.r);if(hit)select(hit.name);};
for(const id of ['temperature','motion'])$(id).onclick=e=>{const r=e.currentTarget.getBoundingClientRect(),f=(e.clientX-r.left-43)/(r.width-83);if(f>=0&&f<=1)jump(Math.max(state.meta?.first_ts??0,state.displayTs-state.window+f*state.window));};
new ResizeObserver(()=>{if(state.snapshot){drawMap();drawCharts();}}).observe(document.querySelector('main'));
refresh();setInterval(()=>{if(document.visibilityState==='visible' && (state.mode==='live'||!state.snapshot))refresh();},500);
