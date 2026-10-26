// Health over time (2026-10-26, "build those five": belt tension and motor health over time). What the recording already holds, read as a
// trend so a slow change shows before it is a failed print: how alike the two belts answer (each belts run of the Input shaper page), each axis's
// resonance over time (a belt going slack lowers it), each motor driver's current scaling and the flags it raised, each heater's heat-up speed.
import {el,button,openSheet,section} from './next-ui.js';
import {pageCtx,u06} from './next-pages.js';

// the verdict on two belts that answer alike (1.0) or not (Shake&Tune's similarity)
export function beltWord(s){if(!Number.isFinite(s))return ['—','',''];return s>=0.95?['Matched','ok','The two belts answer alike.']:s>=0.85?['Close','warn','A little apart: check the tension of both belts.']:['Apart','alarm','The belts answer differently: tension them to match.'];}
// a resonance that falls more than 5 % from its first measured value is a belt going slack (or a frame that moved)
export function peakWord(first,last){if(!Number.isFinite(first)||!Number.isFinite(last)||first<=0)return ['—',''];const d=(last/first-1)*100;return Math.abs(d)<5?[`steady (${d>=0?'+':''}${d.toFixed(1)} %)`,'ok']:d<0?[`down ${Math.abs(d).toFixed(1)} %: check the belt`,'warn']:[`up ${d.toFixed(1)} %`,'ok'];}
export function heaterWord(pct){if(!Number.isFinite(pct))return ['—',''];return pct>-15?[`as usual (${pct>=0?'+':''}${pct.toFixed(0)} %)`,'ok']:[`${Math.abs(pct).toFixed(0)} % slower than usual: check the heater and thermistor`,'warn'];}

// a small line of values over time, oldest to newest
function spark(vals,{lo=null,hi=null,ok=null}={}){const NS='http://www.w3.org/2000/svg',s=document.createElementNS(NS,'svg');s.setAttribute('viewBox','0 0 200 48');s.setAttribute('class','nx-hl-spark');s.setAttribute('aria-hidden','true');
 const v=vals.filter(Number.isFinite);if(v.length<2){return s;}const a=lo??Math.min(...v),b=hi??Math.max(...v),span=b-a||1,X=i=>4+i*(192/(v.length-1)),Y=x=>44-(x-a)/span*40;
 if(ok!==null){const l=document.createElementNS(NS,'line');l.setAttribute('x1','0');l.setAttribute('x2','200');l.setAttribute('y1',String(Y(ok)));l.setAttribute('y2',String(Y(ok)));l.setAttribute('class','ok-line');s.append(l);}
 const p=document.createElementNS(NS,'polyline');p.setAttribute('points',v.map((x,i)=>`${X(i).toFixed(1)},${Y(x).toFixed(1)}`).join(' '));s.append(p);
 const c=document.createElementNS(NS,'circle');c.setAttribute('cx',String(X(v.length-1)));c.setAttribute('cy',String(Y(v[v.length-1])));c.setAttribute('r','3');s.append(c);return s;}

function card(title,big,word,tone,note,vals,opts){const c=el('article','nx-hl-card');c.dataset.tone=tone||'';const top=el('div','nx-hl-top');top.append(el('small','',title),el('span','nx-hl-word',word||''));
 c.append(top,el('b','nx-hl-big',big),spark(vals||[],opts));if(note)c.append(el('p','',note));return c;}

export function pageHealth(){const ctx=pageCtx();if(!ctx)return;
 openSheet({title:'Health',icon:'i-chart',wide:true,eyebrow:'BELTS, MOTORS AND HEATERS OVER TIME',build:body=>{u06(body);body.classList.add('nx-health');
  const belts=el('div','nx-hl-grid'),motors=el('div','nx-hl-grid'),heat=el('div','nx-hl-grid');
  body.append(section('Belts',belts),section('Motors',motors),section('Heaters',heat));
  const wait=h=>h.append(el('p','nx-note','Reading the recording…'));[belts,motors,heat].forEach(wait);
  // belts: every belts run, and the X and Y resonance runs
  fetch('/api/resonance',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(d=>{belts.replaceChildren();const runs=(d?.runs||[]).filter(r=>r.status==='done'||r.status==='complete'||r.peak_hz!=null).slice().reverse();
   const bel=runs.filter(r=>/belt/i.test(r.axis)&&Number.isFinite(r.belts_similarity)),last=bel[bel.length-1];
   const [w,tone,why]=beltWord(last?.belts_similarity);
   belts.append(card('BELT MATCH',last?`${Math.round(last.belts_similarity*100)} %`:'—',w,tone,last?`${why} ${bel.length} measurement${bel.length===1?'':'s'}, the last ${new Date(last.ts*1000).toLocaleDateString()}.`:'No belt measurement yet: Input shaper › Compare belts.',bel.map(r=>r.belts_similarity),{lo:0.7,hi:1,ok:0.95}));
   for(const ax of ['X','Y']){const rs=runs.filter(r=>String(r.axis).toUpperCase()===ax&&Number.isFinite(r.peak_hz));const [pw,pt]=peakWord(rs[0]?.peak_hz,rs[rs.length-1]?.peak_hz);
    belts.append(card(`${ax} RESONANCE`,rs.length?`${rs[rs.length-1].peak_hz.toFixed(1)} Hz`:'—',rs.length>1?pw:'',pt,rs.length?`${rs.length} measurement${rs.length===1?'':'s'}${rs[rs.length-1].recommended?.name?`, shaper ${rs[rs.length-1].recommended.name}`:''}.`:`No ${ax} measurement yet: Input shaper › Measure ${ax}.`,rs.map(r=>r.peak_hz)));}
   belts.append(button({icon:'i-nav-signals',label:'Measure on the Input shaper page',size:'row',title:'Input shaper',does:'Measure X, Y and compare the belts.',onPress:()=>ctx.openPage?.('shaper')}));
  }).catch(()=>{belts.replaceChildren(el('p','nx-note','The resonance measurements are not available on this printer.'));});
  // motors: each driver's current scaling over the recording, and the flags it raised
  fetch('/api/history/motor-drift?limit=500',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(d=>{motors.replaceChildren();const ms=d?.motors||[];
   if(!ms.length){motors.append(el('p','nx-note','No motor driver readings recorded: this printer\'s drivers do not report them.'));return;}
   const seen=ms.filter(m=>(m.samples||0)>0||(m.flag_events||0)>0);if(!seen.length){motors.append(el('p','nx-note','No motor driver readings recorded yet: they appear once the printer has moved with its drivers reporting.'));return;}
   for(const m of seen){const flags=m.flag_events||0,drift=Number(m.drift),tone=flags?'alarm':Math.abs(drift)>=4?'warn':'ok';
    const word=flags?`${flags} warning${flags===1?'':'s'}${m.flags_seen?.length?': '+m.flags_seen.join(', '):''}`:Math.abs(drift)>=4?`working ${drift>0?'harder':'less'} than before`:'steady';
    motors.append(card((m.label||m.motor||'').toUpperCase(),Number.isFinite(m.last?.cs_actual)?`${m.last.cs_actual} / 31`:'—',word,tone,`Current scaling ${m.min_cs_actual??'—'} to ${m.max_cs_actual??'—'} over ${m.samples} readings${Number.isFinite(drift)?`, ${drift>=0?'+':''}${drift} since the first`:''}.`,[m.first?.cs_actual,m.min_cs_actual,m.max_cs_actual,m.last?.cs_actual].filter(Number.isFinite),{lo:0,hi:31}));}
  }).catch(()=>{motors.replaceChildren(el('p','nx-note','The motor readings are not available.'));});
  // heaters: how fast each heats up, each time, against its own usual
  fetch('/api/history/thermal?limit=200',{cache:'no-store'}).then(r=>r.ok?r.json():null).then(d=>{heat.replaceChildren();const ss=d?.sessions||[],dr=d?.drift||{};
   // a heat-up that took no time (the heater was already at its target) says nothing about the heater
   const real=ss.filter(s=>Number.isFinite(s.heat_up_c_per_s)&&s.heat_up_c_per_s>0&&s.seconds_to_target>0),names=[...new Set(real.map(s=>s.heater))];if(!names.length){heat.append(el('p','nx-note','No heat-up recorded yet: one appears the next time a heater warms up from cold.'));return;}
   for(const h of names){const mine=real.filter(s=>s.heater===h).slice().reverse(),x=dr[h],[w,t]=heaterWord(x?.latest_vs_mean_pct),last=mine[mine.length-1];
    heat.append(card(h==='extruder'?'NOZZLE HEATER':h==='heater_bed'?'BED HEATER':h.toUpperCase(),last?`${last.heat_up_c_per_s.toFixed(2)} °/s`:'—',w,t,last?`${mine.length} heat-up${mine.length===1?'':'s'}; the last to ${Math.round(last.target_c)}° in ${Math.round(last.seconds_to_target)} s${Number.isFinite(last.overshoot_c)?`, overshoot ${last.overshoot_c.toFixed(1)}°`:''}.`:'',mine.map(s=>s.heat_up_c_per_s)));}
  }).catch(()=>{heat.replaceChildren(el('p','nx-note','The heat-ups are not available.'));});
 }});}
