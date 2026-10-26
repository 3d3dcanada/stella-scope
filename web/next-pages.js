// Stella Scope NEXT pages (NEXT review 2026-09-24). the owner: "every single clickable goes right to the
// exact same thing it used to be" and "the rest of the whole thing should really feel like this".
// Everything the old HUD opened as a card or a separate page opens here as a NEXT sheet: the part
// panels (bed with its mesh, motors, fans, heaters, probe, boards), the file details with the slicer
// that made the file, and the pages of the rail (levelling, heat and fans, signals, timeline, parts).
// Data comes from the same recorder endpoints the old pages read; commands go through ctx.send.
import {partWiring,partSignals,formatSignal} from './hud-core.js';
import {fmtTemp,fmtDuration,fmtPct,fileTitle,heatColor,xyTravel,filamentSteps,filamentCanRecord,filamentProgress,filamentSettings,filamentShakeVerdict,FILAMENT} from './next-core.js';
import {el,icon,tip,button,busy,openSheet,openModal,led,closeSheet,section,segmented,toast,stat,thermoCard,numberInput,addFromStandard,ink,inkA} from './next-ui.js';
import {control as kctl,key as nkey,CONTROLS as KC,click as kclick} from './next-control.js';
import {printReport} from './next-report.js';

let ctx=null;const live=new Set();
export const pageCtx=()=>ctx;   // the page context, for the modules that build pages of their own (next-calibrate.js)
export function initPages(c){ctx=c;}
// Open sheets that show live values register a painter; next.js calls paintPages() on every render.
export function paintPages(){for(const f of live)try{f();}catch{}}
const finite=v=>typeof v==='number'&&Number.isFinite(v)?v:null;
const getJson=u=>fetch(u,{cache:'no-store'}).then(async r=>{const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status})`);return d;});
const point=()=>ctx.state.snapshot?.point||null;
const ago=ts=>{const s=Date.now()/1000-ts;return s<90?Math.round(s)+' s ago':s<5400?Math.round(s/60)+' min ago':s<172800?(s/3600).toFixed(1)+' h ago':Math.round(s/86400)+' days ago';};
const mm=(v,d=3)=>finite(v)===null?'—':(v>=0?'+':'')+v.toFixed(d)+' mm';
// UI board U06 (2026-10-20): every page of the printer is in the LED language. The sheet body carries .nx-u06 (web/next-pages.css restyles keys, chips, tabs, figures and cards inside it and nowhere else, so the other sheets
// are untouched), and one capturing listener gives every key the kit's click. A number that is set is the kit's control (ctl() below); a number that is read is an LED figure (stat()).
const KCS=Object.fromEntries(KC.map(c=>[c.key,c]));
export function u06(body){body.classList.add('nx-u06');if(body._u06)return;body._u06=1;body.addEventListener('click',e=>{if(e.target.closest('.nx-btn,.nx-seg button,.nx-chip,.nx-part,.nx-motor,.nx-matcard,.nx-raw>summary'))kclick(false);},true);}
// A kit control inside a sheet. The sheet scrolls and clips, and the kit's dim sits over it, so a held control steps out of the sheet into the page (where the dim is) while it is held,
// leaving a blank of its own size, and goes back after its spring (2026-10-20: the control grows where it is, never a pop-up).
export function lift(c){const e=c.el;let ph=null,t=0;
 e.addEventListener('pointerdown',()=>{if(e.hasAttribute('data-locked')||e.hasAttribute('data-unknown')||'readonly' in e.dataset||!e.parentNode||!e.isConnected)return;clearTimeout(t);if(ph){ph.replaceWith(e);ph=null;}
  const r=e.getBoundingClientRect();ph=document.createElement('span');ph.style.cssText='display:block;flex:none;width:'+r.width+'px;height:'+r.height+'px';e.replaceWith(ph);
  Object.assign(e.style,{position:'fixed',left:r.left+'px',top:r.top+'px',width:r.width+'px',height:r.height+'px',margin:'0'});e.style.setProperty('--nxctl-size',r.width+'px');document.body.append(document.getElementById('nxctl-dim'),e);},true);   // the sheet is above the page's own layer (#nx-ui), so for this hold the dim and the control are in the root: the sheet dims, the control does not
 const back=()=>{if(!ph)return;clearTimeout(t);t=setTimeout(()=>{if(!ph)return;for(const k of ['position','left','top','width','height','margin'])e.style.removeProperty(k);e.style.removeProperty('--nxctl-size');if(ph.isConnected)ph.replaceWith(e);else e.remove();ph=null;const ui=document.getElementById('nx-ui'),dim=document.getElementById('nxctl-dim');if(ui&&dim&&!dim.classList.contains('on'))ui.append(dim);},700);};
 e.addEventListener('pointerup',back);e.addEventListener('pointercancel',back);}
export function ctl(key,o={}){const c=kctl({...KCS[key],...o});lift(c);return c;}
// plain:true leaves a sheet as it was: the file details, Alerts and Camera belong to the sheets work order (U05), which brings them to the same language itself.
function sheet(opts){let fn=null;return openSheet({...opts,onClose:()=>{if(fn)live.delete(fn);opts.onClose?.();},build:(body,host)=>{if(!opts.plain)u06(body);const r=opts.build(body,host);if(typeof r==='function'){fn=r;live.add(fn);fn();}}});}
function cmdButton(label,ico,id,does,extra={},kind='default'){return button({icon:ico,label,size:'row',kind,cmd:id,title:label,does,onPress:async b=>{busy(b,true);try{await ctx.send(id,extra);}finally{busy(b,false);}}});}
function statGrid(...stats){const g=el('div','nx-stats');g.append(...stats.map(s=>s.el));return g;}
// ---------- sparkline: one recorded signal over the last hour, then every new sample ----------
export function spark(object,field,{label,unit='',digits=1,scale=1,window=3600}={}){
 const w=el('div','nx-spark'),head=el('div','nx-spark-h'),v=el('b','','—'),cv=el('canvas','nx-spark-cv'),foot=el('div','nx-spark-f','Reading the recording…');head.append(el('span','',label),v);w.append(head,cv,foot);
 let pts=[];const at=point()?.ts??Date.now()/1000;
 getJson(`/api/signal-history?object=${encodeURIComponent(object)}&field=${encodeURIComponent(field)}&window=${window}&points=90&ts=${at}`)
  .then(d=>{pts=(d.points||[]).filter(p=>finite(p.value)!==null).map(p=>[p.ts,p.value*scale]).concat(pts);draw();}).catch(e=>{foot.textContent='History unavailable: '+e.message;});
 function draw(){const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||260,H=cv.clientHeight||54;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);g.clearRect(0,0,W,H);
  if(pts.length<2){foot.textContent='Not enough recorded history for a line yet.';return;}
  const t1=pts.at(-1)[0],t0=Math.min(pts[0][0],t1-window),vs=pts.map(p=>p[1]);let lo=Math.min(...vs),hi=Math.max(...vs);if(hi-lo<1e-9){lo-=1;hi+=1;}const pad=(hi-lo)*.12;lo-=pad;hi+=pad;
  const X=t=>(t-t0)/(t1-t0||1)*W,Y=x=>H-2-(x-lo)/(hi-lo)*(H-4);g.beginPath();pts.forEach(([t,x],i)=>i?g.lineTo(X(t),Y(x)):g.moveTo(X(t),Y(x)));
  g.strokeStyle=ink('--accent','#04D9C4');g.lineWidth=1.6;g.stroke();g.lineTo(X(t1),H);g.lineTo(X(pts[0][0]),H);g.closePath();const gr=g.createLinearGradient(0,0,0,H);gr.addColorStop(0,inkA(.28));gr.addColorStop(1,inkA(0));g.fillStyle=gr;g.fill();
  const lo2=Math.min(...vs),hi2=Math.max(...vs);foot.textContent=`${lo2.toFixed(digits)} to ${hi2.toFixed(digits)}${unit} over the last ${Math.round((t1-t0)/60)} min`;}
 return {el:w,paint(){const p=point(),s=(p?.telemetry?.signals||[]).find(x=>x.object===object&&x.field===field);if(!s||finite(s.value)===null)return;const x=s.value*scale;v.textContent=x.toFixed(digits)+unit;
  if(finite(p.ts)!==null&&(!pts.length||p.ts>pts.at(-1)[0]+2)){pts.push([p.ts,x]);if(pts.length>240)pts.shift();draw();}}};}
function sparks(body,list){const g=el('div','nx-sparks'),ss=list.map(a=>spark(...a));g.append(...ss.map(x=>x.el));body.append(section('Last hour',g));return ()=>ss.forEach(x=>x.paint());}

// ---------- mesh colour and the mesh viewer ----------
// Level is green, low is blue, high is red, symmetric about the mesh mean, like a contour map.
const STOPS=[[-1,[47,107,255]],[-.5,[25,184,255]],[0,[24,227,107]],[.5,[255,214,10]],[1,[255,45,72]]];
export function meshRgb(t){t=Math.max(-1,Math.min(1,t));for(let i=1;i<STOPS.length;i++){const [a,ca]=STOPS[i-1],[b,cb]=STOPS[i];if(t<=b){const k=(t-a)/(b-a);return ca.map((c,j)=>Math.round(c+(cb[j]-c)*k));}}return STOPS[4][1];}
const css=rgb=>`rgb(${rgb[0]},${rgb[1]},${rgb[2]})`;
function bilinear(grid,u,v){const rows=grid.length,cols=grid[0].length,x=u*(cols-1),y=v*(rows-1),c0=Math.floor(x),r0=Math.floor(y),c1=Math.min(cols-1,c0+1),r1=Math.min(rows-1,r0+1),fx=x-c0,fy=y-r0;
 const g=(r,c)=>finite(grid[r][c])??0;return g(r0,c0)*(1-fx)*(1-fy)+g(r0,c1)*fx*(1-fy)+g(r1,c0)*(1-fx)*fy+g(r1,c1)*fx*fy;}
// The newest mesh in the recording at or before `to`, with its grid and where it was probed.
async function withGrid(m){const w=await getJson(`/api/workspace?ts=${m.ts}`).catch(()=>null),g=w?.point?.mesh;
 return {...m,grid:Array.isArray(g?.grid)&&g.grid.length?g.grid:null,min:g?.min||null,max:g?.max||null,limits:w?.point?.limits,probes:(w?.point?.probe_points||[]).filter(p=>p.ts<=m.ts+5&&m.ts-p.ts<2400)};}
// Two views of one mesh: observed printer travel from above with the probed area as a smooth colour field
// and every probe point labelled, and a 3D surface you can turn with a drag, heights exaggerated.
export function meshViewer({height=320}={}){
 const w=el('div','nx-mesh'),bar=el('div','nx-mesh-bar'),cv=el('canvas','nx-mesh-cv'),legend=el('div','nx-mesh-legend'),facts=el('div','nx-stats');
 let view='mesh',mesh=null,az=-35,elv=32,exag=null,bed=null;
 bar.append(segmented(['mesh','map','3d'],view,v=>{view=v;draw();},v=>({mesh:'Mesh',map:'On the bed',['3d']:'3D'}[v])));
 const note=el('span','nx-mesh-note','Drag to turn');note.hidden=true;const badge=el('span','nx-mesh-live');badge.hidden=true;bar.append(badge,note);let liveP=[],liveKey='';
 w.append(bar,cv,legend,facts);cv.style.height=height+'px';cv.setAttribute('role','img');
 const S={range:stat('RANGE'),lohi:stat('LOW / HIGH'),corner:stat('WORST CORNER'),pts:stat('POINTS'),prof:stat('PROFILE'),when:stat('PROBED')};facts.append(...Object.values(S).map(s=>s.el));
 let drag=null;cv.addEventListener('pointerdown',e=>{if(view!=='3d')return;drag={x:e.clientX,y:e.clientY,az,elv};try{cv.setPointerCapture(e.pointerId);}catch{}});
 cv.addEventListener('pointermove',e=>{if(!drag)return;az=drag.az+(e.clientX-drag.x)*.5;elv=Math.max(8,Math.min(80,drag.elv+(e.clientY-drag.y)*.3));draw();});
 cv.addEventListener('pointerup',()=>{drag=null;});cv.addEventListener('pointercancel',()=>{drag=null;});
 function size(){const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||600,H=cv.clientHeight||height;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);return {g,W,H};}
 function stats(){const m=mesh;if(!m||m.error){for(const s of Object.values(S))s.set('—');return;}
  if(m.diff){S.range.set(m.range_mm.toFixed(3)+' mm',{note:'spread of the change across the overlap',tone:m.range_mm<=.03?'ok':m.range_mm<=.08?'':'warn'});
   S.lohi.set(mm(m.min_mm)+' / '+mm(m.max_mm),{note:'most lower / most higher, newer minus older'});S.corner.set(mm(m.mean_mm),{note:'even shift: usually the Z reference, not the plate'});
   S.pts.set(`${m.rows} × ${m.cols}`,{note:'sampled over the area both meshes cover'});S.prof.set('Compare');S.when.set(m.label,{note:'older → newer'});return;}
  S.range.set(m.range_mm.toFixed(3)+' mm',{note:m.range_mm<=.1?'flat, well within first-layer tolerance':m.range_mm<=.2?'fine, the mesh compensates':'large: check the plate and level the gantry',tone:m.range_mm<=.1?'ok':m.range_mm<=.2?'':'warn'});
  S.lohi.set(m.min_mm.toFixed(3)+' / '+m.max_mm.toFixed(3),{note:'mm, as probed'});S.corner.set(mm(m.worst_corner?.from_mean_mm),{note:m.worst_corner?.corner+' from the mean'});
  S.pts.set(`${m.rows} × ${m.cols}`,{note:m.adaptive?'adaptive: only under the print':'full bed'});S.prof.set(m.profile||'unnamed');S.when.set(m.ts?ago(m.ts):'saved',{note:m.ts?new Date(m.ts*1000).toLocaleString():'the mesh print start loads'});}
 function legendPaint(lo,hi,mean){legend.replaceChildren();const g=el('div','nx-mesh-grad'),l=el('span','',lo.toFixed(3)),m=el('span','',mesh?.diff?'no change':'level '+mean.toFixed(3)),h=el('span','',hi.toFixed(3));legend.append(l,g,m,h);}
 function draw(){const {g,W,H}=size();g.clearRect(0,0,W,H);bed=xyTravel(mesh?.limits)||xyTravel(ctx.tick()?.point?.limits)||xyTravel(ctx.state.snapshot?.point?.limits)||xyTravel({min:mesh?.min,max:mesh?.max});note.hidden=view!=='3d';badge.hidden=!liveP.length;badge.textContent=`LIVE · ${liveP.length} point${liveP.length===1?'':'s'} probed`;
  if(!bed&&(view==='map'||mesh?.grid)){g.fillStyle='#8B95A1';g.font='500 14px Geist,sans-serif';g.textAlign='center';g.fillText('Printer travel and mesh bounds are unavailable.',W/2,H/2);return;}
  if(!mesh?.grid){if(view==='map'&&liveP.length){drawMap(g,W,H,null,null);return;}g.fillStyle='#8B95A1';g.font='500 14px Geist,sans-serif';g.textAlign='center';g.fillText(mesh?.error||(mesh?'This mesh was recorded without its grid.':'No mesh in the recording yet.'),W/2,H/2);return;}
  // A comparison is coloured about zero (no change); a mesh about its own mean (level).
  const grid=mesh.grid,flat=grid.flat().filter(v=>finite(v)!==null),mean=mesh.diff?0:flat.reduce((a,b)=>a+b,0)/flat.length,span=Math.max(mesh.diff?.01:.02,...flat.map(v=>Math.abs(v-mean))),col=v=>meshRgb((v-mean)/span);
  legendPaint(Math.min(...flat),Math.max(...flat),mean);
  if(view==='mesh')drawGrid(g,W,H,grid,col);else if(view==='map')drawMap(g,W,H,grid,col);else draw3d(g,W,H,grid,col,mean,span);}
 // The mesh itself, cell by cell: a smooth colour field between the probe points, each point's
 // height written on it, and the X and Y of every column and row along the edges.
 function drawGrid(g,W,H,grid,col){const rows=grid.length,cols=grid[0].length,[x0,y0]=mesh.min||[bed?.x0??0,bed?.y0??0],[x1,y1]=mesh.max||[bed?.x1??1,bed?.y1??1],B=30,T=16,
  // drawn to scale (2026-09-25: a square bed mesh showed as a flat rectangle)
  sc=Math.min((W-190)/Math.max(1,x1-x0),(H-T-B-14)/Math.max(1,y1-y0)),w=(x1-x0)*sc,h=(y1-y0)*sc,L=Math.max(90,(W-w)/2),X=u=>L+u*w,Y=v=>T+(1-v)*h;
  const px=3;for(let yy=0;yy<h;yy+=px)for(let xx=0;xx<w;xx+=px){g.fillStyle=css(col(bilinear(grid,xx/w,1-yy/h)));g.fillRect(L+xx,T+yy,px+.5,px+.5);}
  g.strokeStyle='rgba(7,9,12,.28)';g.lineWidth=1;for(let c=0;c<cols;c++){g.beginPath();g.moveTo(X(c/(cols-1||1)),T);g.lineTo(X(c/(cols-1||1)),T+h);g.stroke();}for(let r=0;r<rows;r++){g.beginPath();g.moveTo(L,Y(r/(rows-1||1)));g.lineTo(L+w,Y(r/(rows-1||1)));g.stroke();}
  g.strokeStyle='rgba(255,255,255,.35)';g.strokeRect(L,T,w,h);
  g.font='500 10.5px "Geist Mono",monospace';g.fillStyle='#A7B0BA';g.textAlign='center';const skip=w/Math.max(1,cols-1)<34?2:1;   // U06 (2026-10-20): on a phone the column numbers ran into each other
  for(let c=0;c<cols;c+=skip)g.fillText((x0+(x1-x0)*c/(cols-1||1)).toFixed(0),X(c/(cols-1||1)),T+h+26);
  g.textAlign='right';for(let r=0;r<rows;r++)g.fillText((y0+(y1-y0)*r/(rows-1||1)).toFixed(0),L-46,Y(r/(rows-1||1))+4);
  g.fillStyle='#8B95A1';g.textAlign='left';g.fillText('X mm',L,H-2);g.save();g.translate(11,T+h/2);g.rotate(-Math.PI/2);g.textAlign='center';g.fillText(h>=200?'Y mm · front at the bottom':'Y mm',0,0);g.restore();
  const cell=Math.min(w/Math.max(1,cols-1),h/Math.max(1,rows-1)),big=cell>=70;g.font=`600 ${big?13:cell>=54?11:9.5}px "Geist Mono",monospace`;g.textAlign='center';g.textBaseline='middle';
  if(cell>=38)for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){const v=grid[r][c];if(finite(v)===null)continue;const x=X(c/(cols-1||1)),y=Y(r/(rows-1||1)),t=(v>=0?'+':'')+v.toFixed(3),tw=g.measureText(t).width+10;
   const bx=Math.max(2,Math.min(W-tw-2,x-tw/2)),by=Math.max(2,Math.min(H-B-2,y-10));g.fillStyle='rgba(7,9,12,.74)';g.fillRect(bx,by,tw,20);g.fillStyle='#EEF2F6';g.fillText(t,bx+tw/2,by+10.5);}
  g.textBaseline='alphabetic';cv.setAttribute('aria-label',`Bed mesh ${rows} by ${cols}, X ${x0} to ${x1} mm, Y ${y0} to ${y1} mm, each point's height written on it`);}
 function drawMap(g,W,H,grid,col){const pad=26,s=Math.min((W-2*pad)/bed.width,(H-2*pad)/bed.height),ox=(W-bed.width*s)/2,oy=(H-bed.height*s)/2,X=x=>ox+(x-bed.x0)*s,Y=y=>oy+(bed.y1-y)*s;
  // bed plate and a 50 mm grid
  g.fillStyle='rgba(255,255,255,.035)';g.strokeStyle='rgba(255,255,255,.16)';g.lineWidth=1;g.beginPath();g.rect(X(bed.x0),Y(bed.y1),bed.width*s,bed.height*s);g.fill();g.stroke();
  g.strokeStyle='rgba(255,255,255,.05)';for(let v=Math.ceil(bed.x0/50)*50;v<bed.x1;v+=50){g.beginPath();g.moveTo(X(v),Y(bed.y0));g.lineTo(X(v),Y(bed.y1));g.stroke();}for(let v=Math.ceil(bed.y0/50)*50;v<bed.y1;v+=50){g.beginPath();g.moveTo(X(bed.x0),Y(v));g.lineTo(X(bed.x1),Y(v));g.stroke();}
  g.fillStyle='#8B95A1';g.font='500 10px "Geist Mono",monospace';g.textAlign='center';for(let v=Math.ceil(bed.x0/100)*100;v<=bed.x1;v+=100)g.fillText(v,X(v),Y(bed.y0)+14);g.textAlign='right';for(let v=Math.ceil(bed.y0/100)*100;v<=bed.y1;v+=100)g.fillText(v,X(bed.x0)-5,Y(v)+3);
  g.textAlign='left';g.fillText('FRONT',X(bed.x0),Y(bed.y0)+24);
  if(!grid){livePts(g,X,Y);return;}const [x0,y0]=mesh.min||[bed.x0,bed.y0],[x1,y1]=mesh.max||[bed.x1,bed.y1],rows=grid.length,cols=grid[0].length;
  // smooth field: 4 px cells, bilinear between probe points
  const px=Math.max(2,Math.round(4)),w=(x1-x0)*s,h=(y1-y0)*s;
  for(let yy=0;yy<h;yy+=px)for(let xx=0;xx<w;xx+=px){const v=bilinear(grid,xx/Math.max(1,w),1-yy/Math.max(1,h));g.fillStyle=css(col(v));g.fillRect(X(x0)+xx,Y(y1)+yy,px+.5,px+.5);}
  g.strokeStyle='rgba(255,255,255,.55)';g.lineWidth=1.2;g.strokeRect(X(x0),Y(y1),w,h);
  // probe points with their values
  const cw=cols>1?w/(cols-1):w,chh=rows>1?h/(rows-1):h,label=cw>=46&&chh>=26;
  for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){const v=grid[r][c],x=X(x0)+(cols>1?c*cw:w/2),y=Y(y0)-(rows>1?r*chh:h/2);
   g.beginPath();g.arc(x,y,3.2,0,Math.PI*2);g.fillStyle='#fff';g.fill();g.strokeStyle='rgba(0,0,0,.6)';g.stroke();
   if(label&&finite(v)!==null){g.font='600 11px "Geist Mono",monospace';g.textAlign='center';const t=(v>=0?'+':'')+v.toFixed(3);g.fillStyle='rgba(7,9,12,.72)';const tw=g.measureText(t).width+8;g.fillRect(x-tw/2,y-19,tw,14);g.fillStyle='#EEF2F6';g.fillText(t,x,y-8);}}
  // the toolhead, live
  const p=ctx.state.readings?.position;if(finite(p?.x)!==null&&finite(p?.y)!==null){g.beginPath();g.arc(X(p.x),Y(p.y),6,0,Math.PI*2);g.strokeStyle=ink('--accent','#04D9C4');g.lineWidth=2;g.stroke();g.beginPath();g.moveTo(X(p.x)-10,Y(p.y));g.lineTo(X(p.x)+10,Y(p.y));g.moveTo(X(p.x),Y(p.y)-10);g.lineTo(X(p.x),Y(p.y)+10);g.stroke();}
  livePts(g,X,Y);cv.setAttribute('aria-label',`Bed mesh ${rows} by ${cols} over X ${x0} to ${x1}, Y ${y0} to ${y1}; front edge at the bottom`);}
 // Probe landings as they happen (a mesh or level run in progress): newest glows, each shows its Z.
 function livePts(g,X,Y){if(!liveP.length)return;const zs=liveP.map(p=>p.z),m=zs.reduce((a,b)=>a+b,0)/zs.length,sp=Math.max(.02,...zs.map(z=>Math.abs(z-m)));
  liveP.forEach((p,i)=>{const x=X(p.x),y=Y(p.y),last=i===liveP.length-1;g.beginPath();g.arc(x,y,last?8:5.5,0,Math.PI*2);g.fillStyle=css(meshRgb((p.z-m)/sp));g.fill();g.lineWidth=last?3:1.5;g.strokeStyle=last?'#fff':'rgba(7,9,12,.8)';g.stroke();
   if(last){g.font='600 12px "Geist Mono",monospace';g.textAlign='left';g.fillStyle='rgba(7,9,12,.8)';const t=p.z.toFixed(4);g.fillRect(x+11,y-9,g.measureText(t).width+10,18);g.fillStyle='#FFD60A';g.fillText(t,x+16,y+4);}});}
 function draw3d(g,W,H,grid,col,mean,span){const rows=grid.length,cols=grid[0].length,[x0,y0]=mesh.min||[bed?.x0??0,bed?.y0??0],[x1,y1]=mesh.max||[bed?.x1??1,bed?.y1??1];
  const sx=x1-x0||1,sy=y1-y0||1,k=Math.max(sx,sy),ex=exag??(k*.22/Math.max(span,.01)),a=az*Math.PI/180,e=elv*Math.PI/180;
  const P=(u,v,z)=>{const x=(u-.5)*sx/k,y=(v-.5)*sy/k,zz=(z-mean)*ex/k;const rx=x*Math.cos(a)-y*Math.sin(a),ry=x*Math.sin(a)+y*Math.cos(a);return {x:rx,y:ry*Math.sin(e)-zz*Math.cos(e),d:ry*Math.cos(e)+zz*Math.sin(e)};};
  const SUB=8,quads=[];let scale=1,cx=0,cy=0;
  for(let i=0;i<SUB*(rows-1||1);i++)for(let j=0;j<SUB*(cols-1||1);j++){const n=SUB*(cols-1||1),m=SUB*(rows-1||1),u0=j/n,u1=(j+1)/n,v0=i/m,v1=(i+1)/m;
   const c=[[u0,v0],[u1,v0],[u1,v1],[u0,v1]].map(([u,v])=>({u,v,z:bilinear(grid,u,v)}));const pts=c.map(q=>P(q.u,q.v,q.z));
   quads.push({pts,d:pts.reduce((s,p)=>s+p.d,0)/4,z:c.reduce((s,q)=>s+q.z,0)/4,edge:j%SUB===0||i%SUB===0});}
  // fit the turned surface and its level plane into the canvas
  {const all=quads.flatMap(q=>q.pts).concat([[0,0],[1,0],[1,1],[0,1]].map(([u,v])=>P(u,v,mean)));const xs=all.map(p=>p.x),ys=all.map(p=>p.y),x0=Math.min(...xs),x1=Math.max(...xs),y0=Math.min(...ys),y1=Math.max(...ys);
   scale=Math.min((W-40)/(x1-x0||1),(H-50)/(y1-y0||1));cx=W/2-(x0+x1)/2*scale;cy=(H-16)/2-(y0+y1)/2*scale;}
  // the level plane under the surface
  g.strokeStyle='rgba(255,255,255,.12)';g.setLineDash([4,4]);g.beginPath();[[0,0],[1,0],[1,1],[0,1],[0,0]].forEach(([u,v],i)=>{const p=P(u,v,mean);const X=cx+p.x*scale,Y=cy+p.y*scale;if(i)g.lineTo(X,Y);else g.moveTo(X,Y);});g.stroke();g.setLineDash([]);
  quads.sort((A,B)=>B.d-A.d);for(const q of quads){g.beginPath();q.pts.forEach((p,i)=>{const X=cx+p.x*scale,Y=cy+p.y*scale;if(i)g.lineTo(X,Y);else g.moveTo(X,Y);});g.closePath();g.fillStyle=css(col(q.z));g.fill();g.strokeStyle=q.edge?'rgba(7,9,12,.35)':css(col(q.z));g.lineWidth=q.edge?.8:.6;g.stroke();}
  g.fillStyle='#A7B0BA';g.font='500 11px "Geist Mono",monospace';g.textAlign='left';g.fillText(`heights × ${Math.round(ex)}`,12,H-12);
  const f=P(.5,0,mean);g.textAlign='center';g.fillText('FRONT',cx+f.x*scale,cy+f.y*scale+18);
  cv.setAttribute('aria-label','3D bed mesh surface, heights exaggerated '+Math.round(ex)+' times');}
 const ro=new ResizeObserver(()=>draw());ro.observe(cv);
 return {el:w,draw,async load(m){mesh=m;stats();draw();},get mesh(){return mesh;},
  // points: probe landings newer than this mesh; the first landing of a run switches to the bed view
  live(points){const k=points.length+':'+(points.at(-1)?.ts??'');if(k===liveKey)return;const was=liveP.length;liveKey=k;liveP=points;if(!was&&points.length&&view!=='map'){view='map';for(const b of bar.querySelectorAll('.nx-seg button'))b.setAttribute('aria-pressed',String(b.textContent==='On the bed'));}draw();}};}

// ---------- part panels ----------
const Z_OF={stepper_z:'z0',stepper_z1:'z1',stepper_z2:'z2',stepper_z3:'z3'},CORNER_OF={stepper_z:'front left',stepper_z1:'back left',stepper_z2:'back right',stepper_z3:'front right'};
function wiringFor(e){const sc=ctx.scene(),b=sc?.geometryBounds?.(e.id);return partWiring(e,b?[(b.min[0]+b.max[0])/2,(b.min[1]+b.max[1])/2]:null,sc?.bedCenter);}
const sig=(sigs,o,f)=>sigs.find(s=>s.object===o&&s.field===f)?.value;
export function openPartByKey(k){const e=ctx.scene()?.entries?.find(x=>wiringFor(x)?.key===k);if(e)openPart(e.id);return !!e;}
// "Show me" (2026-09-28: "Show the motor" opened the slide-out and showed nothing). The named part is
// framed with the machine around it, lit, flashed three times and circled on screen, everything else
// faded; the panel is one tap away on the circle's label. It is not opened for you: on a phone it would
// cover the very thing you asked to see. The circle follows the camera and goes after nine seconds, or
// when Show all, Reset or another selection clears the spotlight.
let spot=null;
function dropSpot(){if(!spot)return;document.removeEventListener('keydown',spot.key);document.removeEventListener('stella-camera-change',spot.move);document.removeEventListener('stella-selection-change',spot.sel);spot.el.remove();spot=null;}
export function showPart(id){const sc=ctx.scene();if(!sc?.spotlight?.(id))return false;closeSheet();dropSpot();
 const e=sc.entries?.find(x=>x.id===id),name=(e?.name||'part').replace(/:\d+$/,'').replace(/(\s*\(\d+\))+$/,'');
 const ring=el('div','nx-spot');ring.dataset.id=id;const tag=el('div','nx-spot-tag nx-glass'),lab=el('b','',name);
 tag.append(lab,button({label:'Open its panel',size:'row',kind:'primary',title:'Open the panel',does:'Opens this part\'s live panel and details.',onPress:()=>{dropSpot();openPart(id);}}),
  button({icon:'i-close',size:'icon',kind:'ghost',title:'Clear',does:'Clears the highlight and shows every part again.',onPress:()=>{dropSpot();sc.showAll?.();}}));
 ring.append(el('i','nx-spot-ring'),tag);document.getElementById('nx-ui').append(ring);
 const move=()=>{const p=sc.project?.(id);ring.hidden=!p||!p.on;if(!p||!p.on)return;ring.dataset.side=p.x>innerWidth-400?'left':'right';ring.style.setProperty('--x',Math.round(p.x)+'px');ring.style.setProperty('--y',Math.round(p.y)+'px');ring.style.setProperty('--r',Math.round(Math.max(30,p.r*1.25))+'px');};
 const sel=()=>{if(sc.selected?.id!==id)dropSpot();};
 const key=e=>{if(e.key==='Escape'){dropSpot();sc.showAll?.();}};spot={el:ring,move,sel,key};document.addEventListener('stella-camera-change',move);document.addEventListener('keydown',key);
 // select() inside spotlight() fires stella-selection-change once for this very part; listen after it.
 setTimeout(()=>{if(spot)document.addEventListener('stella-selection-change',sel);},50);move();setTimeout(move,120);return true;}
export function showPartByKey(k){const e=ctx.scene()?.entries?.find(x=>wiringFor(x)?.key===k);if(!e){toast('That part is not in the 3D view.','alarm');return false;}return showPart(e.id);}
// One pane at a time under a row of tabs; the last tab chosen for this kind of part is remembered.
function tabbed(body,defs){body.classList.add('nx-sheet-body--tabs');const mem='nx-tab-'+defs.map(d=>d[0]).join('.');let cur=defs[0][0];try{const m=localStorage.getItem(mem);if(defs.some(d=>d[0]===m))cur=m;}catch{}
 const panes={};const bar=segmented(defs.map(d=>d[0]),cur,k=>{for(const [n,p] of Object.entries(panes))p.hidden=n!==k;try{localStorage.setItem(mem,k);}catch{}document.dispatchEvent(new Event('nx-tab'));},k=>defs.find(d=>d[0]===k)[1]);
 bar.classList.add('nx-tabs');body.append(bar);for(const [k] of defs){const p=el('div','nx-tabpane');p.hidden=k!==cur;panes[k]=p;body.append(p);}return panes;}
export function openPart(id){const sc=ctx.scene(),e=sc?.entries?.find(x=>x.id===id);if(!e)return;
 const w=wiringFor(e),key=w?.key||null;
 sheet({title:e.name.replace(/:\d+$/,'').replace(/(\s*\(\d+\))+$/,''),eyebrow:w?w.label.toUpperCase():(e.slot_id||e.layer||'part').toUpperCase(),icon:key==='bed'?'i-bed':/fan/.test(key||'')?'i-fan':/stepper|motor/.test(key||'')?'i-motors-off':key==='probe'?'i-probe':key==='hotend'?'i-heat':'i-nav-parts',wide:key==='bed',build:body=>{
  // Tabs, not one long scroll (2026-09-25): the part's own live view first, then details and look.
  const T=tabbed(body,key==='bed'?[['heater','Heater'],['sheet','Sheet & Z'],['mesh','Mesh'],['history','Mesh history'],['details','Details']]:[['live','Live'],['details','Details']]);
  const live=T.live||T.heater,painters=[];
  if(key==='bed')painters.push(bedPanel(T,e));
  else if(key==='hotend')painters.push(heaterPanel(live,'nozzle'));
  else if(key&&/fan$/.test(key))painters.push(fanPanel(live,key));
  else if(key&&/^stepper_/.test(key))painters.push(motorPanel(live,key));
  else if(key==='extruder')painters.push(extruderPanel(live));
  else if(key==='probe')painters.push(probePanel(live));
  else if(key==='toolboard'||key==='controller'||key==='host')painters.push(boardPanel(live,key));
  else if(key==='bed_relay')painters.push(heaterPanel(live,'bed'));
  if(w){const raw=el('details','nx-raw'),list=el('div','nx-raw-list');raw.append(el('summary','','Raw signals · '+w.signals.map(([o])=>o).join(', ')),list);T.details.append(raw);
   painters.push(()=>{if(!raw.open)return;const sigs=partSignals(w,point());list.replaceChildren();if(!sigs.length)list.append(el('p','nx-note','None of this part\'s signals are in the newest sample.'));
    for(const s of sigs){const r=el('div','nx-raw-row');r.append(el('span','',s.field),el('b','',formatSignal(s.object,s.field,s.value)));list.append(r);}});raw.addEventListener('toggle',()=>painters.at(-1)());}
  if(!live.childElementCount)live.append(el('p','nx-note','This part has no live signals of its own. Details has what is known about it.'));
  T.details.append(specs(e,w),...partActions(e));ctx.lockControls(body);return ()=>painters.forEach(f=>f());}});
 sc.select?.(id);}
function specs(e,w){const a=e.asset||{},sc=ctx.scene(),size=sc?.geometryBounds?.(e.id)?.size,dl=el('dl','nx-specs'),f=(k,v)=>{if(v!=null&&v!==''){dl.append(el('dt','',k),el('dd','',String(v)));}};
 f('Size',size?size.map(v=>v.toFixed(1)).join(' × ')+' mm':null);f('Wired to',w?w.signals.map(([o])=>o==='host'?'Printer computer':o).join(', '):'No signals of its own');
 f('Model',a.model);f('Maker',a.manufacturer);f('Revision',a.revision);f('Placement',e.placement?.status);f('CAD file',a.source_name);f('Licence',a.license);f('Notes',a.notes);f('In the assembly',e.full);
 const d=el('details','nx-raw');d.append(el('summary','','Specifications and source'),dl);return d;}
function partActions(e){const sc=ctx.scene(),g=el('div','nx-grid4');
 g.append(button({icon:'i-view',label:'Zoom',size:'row',title:'Zoom to it',does:'Frames the camera on this part.',onPress:()=>sc.focus?.(e.id)}),
  button({icon:'i-panels',label:'Isolate',size:'row',title:'Isolate',does:'Lights this part where it sits and fades the rest; Show all brings the default view back.',onPress:()=>{sc.select?.(e.id);sc.isolate?.();closeSheet();}}),
  button({icon:'i-close',label:'Hide',size:'row',title:'Hide',does:'Hides this part; Show all brings it back.',onPress:()=>{sc.select?.(e.id);sc.hideSelected?.();closeSheet();}}),
  button({icon:'i-nav-hud',label:'Show all',size:'row',title:'Show all',does:'Back to the default view of the printer: every part, none faded.',onPress:()=>{dropSpot();sc.showAll?.();}}));return [section('On the printer',g),paintSection(e)];}
// Part colour (2026-09-25, the classic HUD had it): swatches, any colour, and the whole assembly
// the part belongs to (screws left alone). Saved in this browser only; nothing goes to the printer.
const PALETTE=['#1b1d20','#f2f2ef','#82949c','#d6262f','#ff7a1a','#9be15d','#04d9c4','#8b5cf6'];
function assemblyOf(e){const sc=ctx.scene(),f=(e.full||'').split(' / '),i=f.indexOf(e.name);if(i<1)return null;const path=f.slice(0,i).join(' / ')+' / ';
 const members=(sc?.entries||[]).filter(x=>(x.full||'').startsWith(path)&&!/shcs|bhcs|fhcs|screw|bolt|nut|washer|insert|magnet/i.test(x.name||''));return members.length>1?{name:f[i-1],members}:null;}
function paintSection(e){const sc=ctx.scene(),asm=assemblyOf(e),wrap=el('div','nx-paint'),sw=el('div','nx-swatches'),status=el('span','nx-note','');
 let whole=false;const targets=()=>whole&&asm?asm.members:[e];
 const paint=c=>{try{for(const m of targets())sc.setColor(m.id,c);status.textContent=whole&&asm?`${asm.members.length} parts painted`:'Saved in this browser';mark(c);}catch(err){status.textContent=err.message;}};
 const pick=el('input');pick.type='color';pick.value=sc.color?.(e.id)||'#82949c';pick.className='nx-swatch nx-swatch--any';pick.title='Any colour';pick.setAttribute('aria-label','Any colour for '+e.name);pick.addEventListener('input',()=>paint(pick.value));
 const mark=c=>{for(const b of sw.querySelectorAll('button'))b.setAttribute('aria-pressed',String(b.dataset.c===c.toLowerCase()));};
 for(const c of PALETTE){const b=el('button','nx-swatch');b.type='button';b.dataset.c=c;b.style.setProperty('--c',c);b.title=c;b.setAttribute('aria-label','Paint '+c);b.addEventListener('click',()=>{pick.value=c;paint(c);});sw.append(b);}
 sw.append(pick);mark(pick.value);wrap.append(sw);
 const row=el('div','nx-row');
 if(asm){const seg=segmented(['part','all'],'part',v=>{whole=v==='all';status.textContent=whole?`Colours go to all ${asm.members.length} parts`:'';},v=>v==='part'?'This part':'Whole '+asm.name.replace(/:\d+$|\s*\(.*?\)/g,'').toLowerCase());row.append(seg);}
 row.append(status);wrap.append(row);return section('Colour',wrap);}
function heaterPanel(body,which){const r=()=>ctx.state.readings[which],id=which==='nozzle'?'extruder':'heater_bed';
 // U06 (2026-10-20): the target is the kit's control (nozzle 5°, bed 1°), sent once when the hand lets go; a turn past the service's limit is sent as the limit
 const lim=HEAT_MAX[which],k=ctl(which,{value:0,now:null,onSet:v=>{if(v>lim){v=lim;k.spec.value=v;k.paint();}return ctx.send('set_heater',{target:id,value:v});}});tip(k.el,which==='nozzle'?'Nozzle target':'Bed target','Press and hold, then turn; letting go sends it once.');
 const T=stat('NOW'),G=stat('TARGET'),P=stat('HEATER OUTPUT');const hero=el('div','nx-part-hero');hero.append(k.el,statGrid(T,G,P));body.append(hero);
 // Quick targets, and how the heat is going (2026-09-25: fill the cards): the rate over the last
 // minute, when it will get there, and how long it has held the target (a thick plate wants a soak).
 const quick=el('div','nx-heatquick');for(const v of which==='nozzle'?[0,180,200,220,240,250,260]:[0,60,70,80,90,100,110]){const b=button({label:v?v+'°':'Off',size:'row',kind:v?'default':'ghost',cmd:'set_heater',title:v?`${which==='nozzle'?'Nozzle':'Bed'} to ${v}°`:'Heater off',does:v?`Sets the ${which} target to ${v}°.`:'Turns this heater off.',onPress:()=>ctx.send('set_heater',{target:id,value:v})});b.dataset.v=v;quick.append(b);}
 body.append(section('Set it to',quick));
 const RATE=stat('RATE'),ETA=stat('REACHES TARGET'),HOLD=stat('HELD AT TARGET');body.append(statGrid(RATE,ETA,HOLD));
 const sp=sparks(body,[[id,'temperature',{label:'Temperature',unit:'°'}],[id,'power',{label:'Heater output',unit:'%',scale:100,digits:0}]]);
 const heatFacts=x=>{const hist=ctx.state.hist?.[which]||[],last=hist.at(-1);if(!last||x.temp===null){RATE.set('—');ETA.set('—');HOLD.set('—');return;}
  const past=hist.filter(q=>last[0]-q[0]<=60&&q[1]!==null),first=past[0],rate=first&&last[0]-first[0]>10?(last[1]-first[1])/(last[0]-first[0])*60:null;
  RATE.set(rate===null?'—':(rate>=0?'+':'')+rate.toFixed(1)+'°/min',{note:'over the last minute'});
  if(!(x.target>0)){ETA.set('off',{note:'no target'});HOLD.set('—');return;}
  const gap=x.target-x.temp;if(Math.abs(gap)<=2){ETA.set('there',{tone:'ok'});}else if(rate&&Math.sign(rate)===Math.sign(gap)&&Math.abs(rate)>.2){const m=Math.abs(gap/rate);ETA.set(m<1?'<1 min':`~${Math.round(m)} min`,{note:`${Math.abs(gap).toFixed(0)}° to go`});}else ETA.set('—',{note:`${Math.abs(gap).toFixed(0)}° to go`});
  let held=0;for(let i=hist.length-1;i>=0;i--){const q=hist[i];if(q[1]===null||Math.abs(q[1]-x.target)>2)break;held=last[0]-q[0];}
  HOLD.set(held<30?'—':held<600?`${Math.floor(held/60)} min ${Math.round(held%60)} s`:'10+ min',{note:which==='bed'?'a thick plate wants about 10 min before a mesh':'at the target within 2°',tone:which==='bed'&&held>=600?'ok':''});};
 return ()=>{sp();const x=r(),h=heatColor(x.temp,x.target,which);heatFacts(x);for(const b of quick.children)b.toggleAttribute('data-on',Number(b.dataset.v)===Math.round(x.target||0));k.setUnknown(x.target===null||x.target===undefined);k.setNow(finite(x.temp));if(finite(x.target)!==null)k.set(x.target);k.setLocked(ctx.can('set_heater')?'':ctx.why('set_heater'));
  T.set(fmtTemp(x.temp),{tone:h.tone==='danger'?'alarm':''});G.set(x.target>0?fmtTemp(x.target):'off');P.set(x.power===null||x.power===undefined?'—':Math.round(x.power*100)+' %',{note:'commanded, not measured watts'});};}
function bedPanel(T){const paint=heaterPanel(T.heater,'bed'),Z=stat('Z OFFSET'),Q=stat('GANTRY');T.heater.append(statGrid(Z,Q));
 const sheetPaint=sheetPicker(T.sheet),bsPaint=babystep(T.sheet);
 // Mesh: the full mesh Klipper has saved (what print_start loads), never an adaptive one.
 const mv=meshViewer({height:455}),acts=el('div','nx-grid4');
 acts.append(cmdButton('Home','i-home','home','G28.'),cmdButton('Level gantry','i-level','qgl','Probes the four corners and levels the gantry.'),cmdButton('Bed mesh','i-mesh','mesh','Probes the bed and saves a fresh mesh.'),
  button({icon:'i-mesh',label:ctx.meshOnPrinter()?'Hide on printer':'Show on printer',size:'row',title:'Mesh on the printer',does:'Drapes the newest mesh over the bed in the 3D view, heights exaggerated.',onPress:b=>{const on=!ctx.meshOnPrinter();ctx.meshOnPrinter(on);b.querySelector('.nx-lbl').textContent=on?'Hide on printer':'Show on printer';}}));
 T.mesh.append(mv.el,acts);savedMesh(mv);
 // History: every recorded mesh, labelled with the build plate that was on the bed then.
 const hv=meshViewer({height:400}),hist=el('div','nx-meshhist');T.history.append(hist,hv.el);meshHistory(hist,hv);
 document.addEventListener('nx-tab',()=>{mv.draw();hv.draw();});
 return ()=>{paint();sheetPaint();bsPaint();mv.live(liveProbes(mv.mesh?.ts));const p=point();Z.set(finite(p?.z_offset)===null?'—':mm(p.z_offset),{note:'SET_GCODE_OFFSET Z'});Q.set(p?.qgl_applied===true?'Levelled':p?.qgl_applied===false?'Not levelled':'—',{note:'quad gantry level this session',tone:p?.qgl_applied===true?'ok':p?.qgl_applied===false?'warn':''});};}
// A Klipper profile as a mesh the viewer draws: the stats the recorder would have worked out.
function profileMesh(pf){const g=pf.grid,flat=g.flat(),lo=Math.min(...flat),hi=Math.max(...flat),mean=flat.reduce((a,b)=>a+b,0)/flat.length;
 const cs=[['front left',g[0][0]],['front right',g[0].at(-1)],['back left',g.at(-1)[0]],['back right',g.at(-1).at(-1)]].map(([corner,z])=>({corner,z_mm:z,from_mean_mm:z-mean}));
 const worst=cs.reduce((a,b)=>Math.abs(b.from_mean_mm)>Math.abs(a.from_mean_mm)?b:a);
 return {grid:g,rows:pf.rows,cols:pf.cols,min:pf.min.map(Number),max:pf.max.map(Number),profile:pf.name+' · saved in Klipper',adaptive:false,range_mm:hi-lo,min_mm:lo,max_mm:hi,mean_mm:mean,worst_corner:worst,ts:null};}
async function savedMesh(mv){try{const d=await getJson('/api/print/mesh-profiles');const pf=(d.profiles||[]).find(p=>p.name==='default')||(d.profiles||[])[0];mv.load(pf?profileMesh(pf):{error:d.error||'Klipper has no saved mesh yet. Make a bed mesh first.'});}
 catch(err){mv.load({error:'Saved mesh unavailable: '+err.message});}}
// Which plate was on the bed at a moment: the newest plate record at or before it.
export function plateAt(ts){const pr=ctx.state.presets||{},log=[...(pr.sheet_log||[]),...(pr.sheet?.at?[pr.sheet]:[])].sort((a,b)=>a.at-b.at);let name=null;for(const x of log)if(x.at<=ts+1)name=x.name;return name;}
// Probe landings from the newest tick that came after the shown mesh and within the last ten
// minutes: a run in progress. Older points belong to a mesh already in the history.
export function liveProbes(after=0){const pp=ctx.probes();const now=ctx.tick()?.point?.ts??Date.now()/1000;
 // The newest run: points with no gap over two minutes between them (2026-09-25: a 9x9 mesh
 // takes twenty minutes, and a ten-minute window dropped the front rows while it was still going).
 const run=pp.filter(p=>p.ts>(after||0)+1).sort((a,b)=>b.ts-a.ts);if(!run.length||now-run[0].ts>1800)return [];
 let i=1;while(i<run.length&&run[i-1].ts-run[i].ts<120)i++;return run.slice(0,i).reverse();}
// ---------- build sheets: which one is on the bed, and the Z offset it prints at ----------
// Klipper cannot know which sheet is fitted, so this is our record, kept in the presets store.
function sheetPicker(body){const box=el('div','nx-sheetpick'),chips=el('div','nx-chips'),line=el('div','nx-sheetline'),msg=el('span'),apply=button({icon:'i-check',label:'Apply offset',size:'row',kind:'primary',cmd:'z_offset',title:'Apply this plate\'s Z offset',does:'Changes the printer\'s Z offset by the difference, once, after showing you the command.',onPress:async b=>{busy(b,true);try{await applySheet();}finally{busy(b,false);}}});
 const edit=button({icon:'i-panels',label:'Edit plates',size:'row',title:'Edit plates',does:'Name your plates and the Z offset each prints at. Nothing is kept until Save.',onPress:()=>sheetEditor()});
 line.append(msg,apply);box.append(chips,line,edit);body.append(section('Build plate',box));let key='';
 const sheets=()=>ctx.state.presets?.sheets||[],fitted=()=>sheets().find(x=>x.name===ctx.state.presets?.sheet?.name)||null;
 async function pick(name){const pr=ctx.state.presets,at=Date.now()/1000;try{await ctx.savePresets({...pr,sheet:{name,at},sheet_log:[...(pr?.sheet_log||[]),{name,at}].slice(-200)});toast(name+' is on the bed.','ok');}catch(e){toast(e.message,'alarm');}}
 async function applySheet(){const f=fitted(),z=finite(point()?.z_offset);if(!f||z===null)return;const d=Math.round((f.z_offset-z)*1000)/1000;if(!d){toast('Already at '+mm(z)+'.','ok');return;}
  if(Math.abs(d)>1){toast(`That is a ${Math.abs(d).toFixed(3)} mm change; the offset control moves at most 1 mm at a time.`,'alarm');return;}await ctx.send('z_offset',{value:d});}
 return ()=>{const list=sheets(),f=fitted(),z=finite(point()?.z_offset),k=JSON.stringify([list,f?.name,z]);if(k===key)return;key=k;chips.replaceChildren();
  for(const x of list){const b=el('button','nx-chip',x.measured===false?`${x.name} · offset not set`:`${x.name} · ${mm(x.z_offset)}`);b.type='button';b.setAttribute('aria-pressed',String(f?.name===x.name));b.addEventListener('click',()=>pick(x.name));tip(b,x.name,'Marks this plate as the one on the bed.');chips.append(b);}
  if(!list.length){msg.textContent='No plates yet. Add them with Edit plates: a name and the Z offset each prints at.';apply.hidden=true;return;}
  if(!f){msg.textContent='Which plate is on the bed? Tap it.';apply.hidden=true;return;}
  if(f.measured===false){apply.hidden=true;line.dataset.tone='';msg.textContent=`${f.name} is on. Its Z offset is not set yet: set it in Edit sheets once you have measured it.`;ctx.lockControls(box);return;}
  const d=z===null?null:f.z_offset-z;apply.hidden=d===null||Math.abs(d)<0.0005;line.dataset.tone=apply.hidden?'ok':'warn';
  msg.textContent=d===null?`${f.name} prints at ${mm(f.z_offset)}; the printer's offset is not in the sample.`:apply.hidden?`${f.name} is on, and the printer is at its offset ${mm(z)}.`:`${f.name} prints at ${mm(f.z_offset)}; the printer is at ${mm(z)}.`;ctx.lockControls(box);};}
function sheetEditor(){const saved=(ctx.state.presets?.sheets||[]).map(x=>({...x}));const rows=saved.map(x=>({...x}));
 openSheet({title:'Build plates',icon:'i-panels',build:body=>{const list=el('div','nx-list'),status=el('p','nx-note');
  const paint=()=>{list.replaceChildren();rows.forEach((r,i)=>{const w=el('div','nx-item'),name=el('input','nx-input'),z=el('input','nx-input');name.value=r.name;name.setAttribute('aria-label','Sheet name');name.placeholder='Textured PEI';
   z.type='number';z.step='0.005';z.value=r.z_offset;z.setAttribute('aria-label','Z offset in mm');name.style.flex='2';z.style.flex='1';name.oninput=()=>{r.name=name.value;status.textContent='Unsaved changes.';};z.oninput=()=>{r.z_offset=Number(z.value);r.measured=true;status.textContent='Unsaved changes.';};if(r.measured===false){z.value='';z.placeholder='not set';}
   w.append(name,z,button({icon:'i-close',size:'icon',kind:'danger',title:'Delete plate',does:'Removes it when you press Save.',onPress:()=>{rows.splice(i,1);paint();}}));list.append(w);});};
  const add=button({icon:'i-plus',label:'Add plate',size:'row',title:'Add plate',does:'Adds a row.',onPress:()=>{rows.push({name:'',z_offset:0,measured:false});paint();}});
  const std=button({icon:'i-plus',label:'Add from the standard list',size:'row',title:'Add from the standard list',does:'Shows the common build plates you do not have. Their offsets start not set.',
   onPress:b=>addFromStandard(b,'sheets',rows.map(r=>r.name),list=>{for(const x of list)rows.push({...x});paint();status.textContent='Unsaved changes.';},{noun:'plate'})});
  const adds=el('div','nx-grid2');adds.append(add,std);
  const save=button({icon:'i-check',label:'Save',kind:'primary',size:'row',title:'Save plates',does:'Stores the plates in Stella Scope.',onPress:async b=>{busy(b,true);try{const pr=ctx.state.presets,keep=rows.some(r=>r.name.trim()===pr?.sheet?.name);
   await ctx.savePresets({...pr,sheets:rows.map(r=>({name:r.name.trim(),z_offset:Number.isFinite(r.z_offset)?r.z_offset:0,measured:r.measured!==false})),sheet:keep?pr.sheet:null});toast('Plates saved.','ok');closeSheet();}catch(e){status.textContent=e.message;}busy(b,false);}});
  const acts=el('div','nx-grid2');acts.append(button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Discards the changes.',onPress:()=>closeSheet()}),save);
  paint();body.append(el('p','nx-note','Name, and the Z offset that plate prints at. Up to eight. Nothing is kept until Save.'),list,adds,status,acts);}});}
// Mesh history chips, newest first. Compare: the shown mesh is the older one, the next chip pressed
// is the newer, and the map shows newer minus older over the area both cover.
// Mesh history (2026-09-25: "adaptive meshes and bed meshes should be two different saves"):
// full-bed meshes and print-area (adaptive) meshes are two lists behind two tabs, never mixed. Each
// chip names the plate that was on the bed. Compare: the next chip pressed is diffed with the shown one.
async function meshHistory(host,mv){try{const h=await getJson('/api/history/mesh?limit=60');const all=h.meshes||[];host.replaceChildren();
  const lists={full:all.filter(m=>!m.adaptive),area:all.filter(m=>m.adaptive)};let kind=lists.full.length?'full':'area',shown=null,cmp=false;
  const tabs=segmented(['full','area'],kind,v=>{kind=v;draw();},v=>v==='full'?`Full bed · ${lists.full.length}`:`Print area · ${lists.area.length}`);tabs.classList.add('nx-meshkinds');
  const chips=el('div','nx-chips nx-chips--scroll');host.append(tabs,chips);
  const cmpBtn=el('button','nx-chip nx-chip--tool','Compare');cmpBtn.type='button';tip(cmpBtn,'Compare two meshes','Press, then pick a second mesh: the map shows how the bed changed between them, over the area both cover.');
  cmpBtn.addEventListener('click',()=>{cmp=!cmp;cmpBtn.toggleAttribute('aria-pressed',cmp);if(cmp)toast('Now pick the mesh to compare with.');});
  async function draw(){chips.replaceChildren();const ms=lists[kind];if(ms.length>1)chips.append(cmpBtn);
   ms.forEach((m,i)=>{const b=el('button','nx-chip',`${plateAt(m.ts)||'plate not recorded'} · ${ago(m.ts)} · ${m.rows}×${m.cols} · ${m.range_mm.toFixed(3)} mm`);b.type='button';b.setAttribute('aria-pressed',String(i===0));tip(b,'Mesh '+new Date(m.ts*1000).toLocaleString(),`${m.profile||'unnamed'}; range ${m.range_mm.toFixed(3)} mm.`);
    b.addEventListener('click',async()=>{const full=await withGrid(m);
     if(cmp&&shown&&shown.ts!==full.ts){const [a,c]=shown.ts<full.ts?[shown,full]:[full,shown];b.dataset.b='';mv.load(diffMesh(a,c));cmp=false;cmpBtn.removeAttribute('aria-pressed');return;}
     for(const x of chips.children){x.setAttribute('aria-pressed','false');delete x.dataset.b;}b.setAttribute('aria-pressed','true');shown=full;mv.load(full);});chips.append(b);});
   if(ms[0]){shown=await withGrid(ms[0]);mv.load(shown);}else{shown=null;mv.load({error:kind==='full'?'No full-bed mesh recorded yet. Make a bed mesh first.':'No print-area mesh recorded yet.'});}}
  draw();}catch(e){host.replaceChildren(el('p','nx-note','Mesh history unavailable: '+e.message));mv.load(null);}}
export function diffMesh(a,b){if(!a?.grid||!b?.grid||!a.min||!b.min)return {error:'One of these meshes was recorded without its grid.'};
 const x0=Math.max(a.min[0],b.min[0]),x1=Math.min(a.max[0],b.max[0]),y0=Math.max(a.min[1],b.min[1]),y1=Math.min(a.max[1],b.max[1]);
 if(x1-x0<1||y1-y0<1)return {error:'These two meshes do not cover the same part of the bed.'};
 const cols=Math.max(3,a.cols,b.cols),rows=Math.max(3,a.rows,b.rows),at=(m,x,y)=>bilinear(m.grid,(x-m.min[0])/((m.max[0]-m.min[0])||1),(y-m.min[1])/((m.max[1]-m.min[1])||1));
 const grid=[];for(let r=0;r<rows;r++){const row=[];for(let c=0;c<cols;c++){const x=x0+(x1-x0)*c/(cols-1),y=y0+(y1-y0)*r/(rows-1);row.push(at(b,x,y)-at(a,x,y));}grid.push(row);}
 const flat=grid.flat(),lo=Math.min(...flat),hi=Math.max(...flat);
 return {diff:true,grid,rows,cols,min:[x0,y0],max:[x1,y1],ts:b.ts,min_mm:lo,max_mm:hi,range_mm:hi-lo,mean_mm:flat.reduce((s,v)=>s+v,0)/flat.length,label:`${ago(a.ts)} → ${ago(b.ts)}`};}
function fanPanel(body,key){const o=key==='part_fan'?'part':key==='hotend_fan'?'hotend':'controller',ring=ctl(o==='part'?'part':o==='hotend'?'hotend':'board',{value:0,readOnly:o!=='part',onSet:o==='part'?v=>ctx.send('set_fan',{value:v/100}):null}),S=stat('SPEED'),T=stat('TACHOMETER');
 const hero=el('div','nx-part-hero');hero.append(ring.el,statGrid(S,T));body.append(hero);   // U06: the fan is the kit's control (the part fan turns, 5% a click; the other two are Klipper's)
 if(o==='part'){const g=el('div','nx-grid3');for(const v of [0,25,50,75,100])g.append(cmdButton(v+'%','i-fan','set_fan',`Sets the part cooling fan to ${v}%.`,{value:v/100}));body.append(section('Set the part fan',g));}
 else if(o==='hotend')body.append(el('p','nx-note','Automatic: Klipper runs it whenever the nozzle is above its start temperature. It cannot be set by a command; its start temperature can.'),heaterFanEditor());
 else body.append(el('p','nx-note','Automatic: Klipper runs the bay fans while the steppers or heaters are on. They cannot be set by a command; their rules live in printer.cfg.'));
 const fo=o==='part'?'fan':o==='hotend'?'heater_fan hotend_fan':'controller_fan controller_fan',sp=sparks(body,[[fo,'speed',{label:'Fan command',unit:'%',scale:100,digits:0}]]);
 return ()=>{sp();const stale=document.body.hasAttribute('data-stale'),f=stale?{speed:null}:ctx.state.readings.fans[o];ring.setUnknown(f?.speed===null||f?.speed===undefined);if(finite(f?.speed)!==null)ring.set(Math.round(f.speed*100));if(o==='part')ring.setLocked(ctx.can('set_fan')?'':ctx.why('set_fan'));S.set(fmtPct(f?.speed),{note:'the command Klipper sent'});T.set('not fitted',{note:'no speed sensor wire, so this is the speed asked for'});};}
// The hotend fan's start temperature (heater_temp in printer.cfg), as the classic HUD's L2 editor:
// preview shows the diff, apply writes a dated backup, uploads it and restarts the firmware.
function heaterFanEditor(){const box=el('div','nx-sheetpick');box.append(el('p','nx-note','Reading printer.cfg…'));
 getJson('/api/config/heater-fan').then(cfg=>{box.replaceChildren();const lo=cfg.min??30,hi=cfg.max??60,st=ctl('hfs',{label:'STARTS AT',unit:'°',min:lo,max:hi,step:1,turn:300,value:Math.round(cfg.heater_temp),color:'#3D8BFF',color2:'#9EC5FF'}),diff=el('pre','nx-review'),msg=el('p','nx-note');diff.hidden=true;let token=null;
  const apply=button({icon:'i-restart',label:'Apply and restart firmware',size:'row',kind:'danger',hold:true,title:'Apply',does:'Writes a dated backup, uploads printer.cfg and restarts the firmware.',onPress:async b=>{if(!token)return;busy(b,true);try{const d=await ctx.jsend('/api/config/heater-fan/apply','POST',{token});msg.textContent=d.message||'Written. Firmware restarting.';token=null;apply.hidden=true;}catch(e){msg.textContent=e.message;}busy(b,false);}});apply.hidden=true;
  const prev=button({icon:'i-view',label:'Preview the change',size:'row',kind:'primary',title:'Preview',does:'Shows the printer.cfg diff. Nothing is written yet.',onPress:async b=>{busy(b,true);try{const d=await ctx.jsend('/api/config/heater-fan','POST',{heater_temp:st.spec.value});token=d.token;diff.textContent=d.diff||'(no change)';diff.hidden=false;apply.hidden=!token;msg.textContent='Check the diff. Apply writes it and restarts the firmware.';}catch(e){msg.textContent=e.message;}busy(b,false);}});
  box.append(el('p','nx-note',`Starts at ${cfg.heater_temp} °C now (printer.cfg line ${cfg.line??'?'}). ${lo} to ${hi} °C; never off.`),st.el,prev,diff,apply,msg);})
 .catch(e=>{box.replaceChildren(el('p','nx-note','printer.cfg cannot be read right now: '+e.message));});return section('Start temperature',box);}
function motorPanel(body,key){const Cur=stat('RUN CURRENT'),Hold=stat('HOLD CURRENT'),En=stat('DRIVER'),Sc=stat('CURRENT SCALE'),Fl=stat('FLAGS'),Pos=stat('POSITION');
 body.append(statGrid(Cur,Hold,En,Sc,Fl,Pos));
 if(Z_OF[key]){const Q=stat('LAST LEVEL CORRECTION'),R=stat('LEVEL RUN');body.append(statGrid(Q,R));getJson('/api/qgl').then(d=>{const r=d.rounds?.[0];if(!r){Q.set('—',{note:'no gantry levelling recorded'});return;}const v=r[Z_OF[key]];
  Q.set(mm(v),{note:`${CORNER_OF[key]} corner, round ${r.round}`,tone:Math.abs(v)>0.05?'warn':''});R.set(r.pass?'Passed':'Retried',{note:`range ${r.range_mm.toFixed(3)} of ${r.tolerance_mm} mm · ${ago(r.ts)}`,tone:r.pass?'ok':'warn'});}).catch(e=>Q.set('—',{note:e.message}));
  body.append(section('',cmdButton('Level gantry','i-level','qgl','Probes the four corners and moves each Z motor to level the gantry.')));}
 const sp=sparks(body,[['tmc2209 '+key,'drv_status.cs_actual',{label:'Motor driver current scale',unit:' / 31',digits:0}],Z_OF[key]?['motion_report','live_position.2',{label:'Z height',unit:' mm',digits:2}]:['motion_report','live_velocity',{label:'Head speed',unit:' mm/s',digits:0}]]);
 return ()=>{sp();const m=point()?.motors?.[key],r=ctx.state.readings;
  Cur.set(finite(m?.current)===null?'—':m.current.toFixed(2)+' A');Hold.set(finite(m?.hold)===null?'—':m.hold.toFixed(2)+' A');
  En.set(m?m.energised?(m.standstill?'Holding':'Stepping'):'Off':'—',{note:m?m.energised?'energised':'motors off, not homed':'not in the sample',tone:m?.energised?'ok':''});
  Sc.set(finite(m?.scale)===null?'—':m.scale+' / 31',{note:'TMC2209 CS_ACTUAL'});Fl.set(m?(m.flags?.length?m.flags.join(', '):'none'):'—',{tone:m?.flags?.length?'alarm':''});
  const ax=key==='stepper_x'||key==='stepper_y'?null:'z';Pos.set(ax?(finite(r.position.z)===null?'—':r.position.z.toFixed(2)+' mm'):`${r.position.x?.toFixed(1)??'—'} · ${r.position.y?.toFixed(1)??'—'}`,{note:ax?'Z':'X · Y (CoreXY: both motors move both)'});};}
function extruderPanel(body){const F=stat('FILAMENT FEED'),V=stat('FLOW RATE'),PA=stat('PRESSURE ADVANCE'),C=stat('EXTRUSION');body.append(statGrid(F,V,PA,C));
 const sp=sparks(body,[['motion_report','live_extruder_velocity',{label:'Filament feed',unit:' mm/s',digits:2}],['tmc2209 extruder','drv_status.cs_actual',{label:'Motor driver current scale',unit:' / 31',digits:0}]]);
 return ()=>{sp();const r=ctx.state.readings,sigs=point()?.telemetry?.signals||[],pa=sig(sigs,'extruder','pressure_advance'),ce=sig(sigs,'extruder','can_extrude');
  F.set(finite(r.extruderVelocity)===null?'—':Math.abs(r.extruderVelocity).toFixed(2)+' mm/s',{note:r.extruderVelocity<0?'retracting':'1.75 mm filament'});
  V.set(finite(r.extruderVelocity)===null?'—':(Math.max(0,r.extruderVelocity)*Math.PI*0.765625).toFixed(1)+' mm³/s');PA.set(finite(pa)===null?'—':pa.toFixed(3)+' s');C.set(ce===true?'Allowed':ce===false?'Blocked, too cold':'—',{tone:ce===false?'warn':''});};}
function probePanel(body){const T=stat('STATE'),Z=stat('LAST Z'),N=stat('RECENT POINTS'),list=el('div','nx-raw-list');
 // The BLTouch's own red LED, as the app reads it (Klipper does not report the LED itself).
 const led=el('div','nx-led'),dot=el('i'),lt=el('div');lt.append(el('b'),el('span'));led.append(dot,lt);
 const reset=button({icon:'i-restart',label:'Reset the BLTouch',size:'row',kind:'danger',cmd:'console',title:'Reset the BLTouch',does:'Resets the BLTouch once, after showing you the command, to clear the alarm.',onPress:()=>ctx.send('console',{script:'BLTOUCH_DEBUG COMMAND=reset'})});
 body.append(section('Probe light',led,reset));
 const LED={on:['On, steady','Pin up and ready. The red light stays on while the probe is stowed.'],probing:['Flashing with each touch','Probing now: the light goes out while the pin is down and comes on as it touches.'],
  alarm:['Flashing fast: alarm','The probe reported a failure. Check the pin and the mount, then reset it.'],off:['Off','The printer is off or not answering, so the probe has no power.']};
 body.append(statGrid(T,Z,N),section('Newest probe points',list),section('',el('div','nx-grid2')));
 body.lastChild.lastChild.append(cmdButton('Check probe','i-probe','probe_query','Asks whether the probe is triggered right now.'),cmdButton('Probe accuracy','i-probe','probe_accuracy','Probes the centre ten times.'));
 return ()=>{const ls=ctx.state.bltouch||'off',[lb,ld]=LED[ls]||LED.off;led.dataset.state=ls;lt.firstChild.textContent=lb;lt.lastChild.textContent=ld+(ls==='alarm'&&ctx.state.bltouchWhy?' '+ctx.state.bltouchWhy:'');reset.hidden=ls!=='alarm';
  const p=point(),pr=p?.probe,pts=(p?.probe_points||[]).slice(-12).reverse();T.set(pr?.triggered===true?'Triggered':pr?.triggered===false?'Open':'—',{note:'cached from the last query'});Z.set(finite(pr?.last_z_result)===null?'—':pr.last_z_result.toFixed(3)+' mm');N.set(String((p?.probe_points||[]).length));
  list.replaceChildren();for(const q of pts){const r=el('div','nx-raw-row');r.append(el('span','',`X ${q.x.toFixed(1)} · Y ${q.y.toFixed(1)}`),el('b','',q.z.toFixed(4)+' mm'));list.append(r);}if(!pts.length)list.append(el('p','nx-note','No probe points in the newest sample.'));};}
function boardPanel(body,key){const o=key==='toolboard'?'mcu EBBCan':'mcu',A=stat(key==='host'?'CPU':'MCU AWAKE'),R=stat(key==='host'?'MEMORY':'RESENT BYTES'),T=stat(key==='host'?'SAMPLE':'ROUND TRIP'),F=stat(key==='host'?'LOAD':'FIRMWARE');body.append(statGrid(A,R,T,F));
 const sp=key==='host'?()=>{}:sparks(body,[[o,'last_stats.bytes_retransmit',{label:'Resent bytes',digits:0}],[o,'last_stats.mcu_awake',{label:'MCU awake',unit:' %',scale:100,digits:2}]]);
 return ()=>{sp();const p=point(),s=p?.telemetry?.signals||[];if(key==='host'){const h=p?.telemetry?.host?.data||{};A.set(finite(h.cpu_temp)===null?'—':h.cpu_temp.toFixed(1)+' °C');R.set(finite(h.mem_used_pct)===null?(h.memory??'—'):h.mem_used_pct.toFixed(0)+' %');T.set(p?.telemetry?.host?.captured_at?new Date(p.telemetry.host.captured_at*1000).toLocaleTimeString():'—');F.set(h.load_avg??h.sysload??'—');return;}
  const a=sig(s,o,'last_stats.mcu_awake'),rt=sig(s,o,'last_stats.bytes_retransmit'),srtt=sig(s,o,'last_stats.srtt');A.set(finite(a)===null?'—':(a*100).toFixed(2)+' %');R.set(finite(rt)===null?'—':rt+' bytes',{note:'since the last restart',tone:rt>0?'warn':''});T.set(finite(srtt)===null?'—':(srtt*1000).toFixed(1)+' ms');F.set(sig(s,o,'mcu_version')??'—');};}

// ---------- file details: the slicer that made it and its settings ----------
export function openFile(path){sheet({plain:true,title:fileTitle(path),eyebrow:'G-CODE FILE',icon:'i-files',build:body=>{
 const hero=el('div','nx-file-hero'),img=el('img','nx-file-thumb');img.alt='';img.addEventListener('error',()=>{const d=el('div','nx-file-thumb nx-file-thumb--none');d.append(icon('i-files'));img.replaceWith(d);});if(ctx.has('thumbnails'))img.src='/api/print/thumbnail?filename='+encodeURIComponent(path);else queueMicrotask(()=>img.dispatchEvent(new Event('error')));
 const side=el('div','nx-file-side'),slicer=el('div','nx-file-slicer','Reading the file…'),acts=el('div','nx-grid2');
 acts.append(button({icon:'i-plus',label:'Queue',size:'row',cmd:'print_queue_add',title:'Add to queue',does:'Adds this file to the print queue.',onPress:()=>ctx.send('print_queue_add',{filename:path})}),
  button({icon:'i-print',label:'Print now',kind:'primary',size:'row',cmd:'print_start',title:'Print now',does:'Starts this file now. Shows the command once first.',onPress:async b=>{busy(b,true);if(await ctx.send('print_start',{filename:path}))closeSheet();busy(b,false);}}),
  // WO-02: every move of the file with no plastic, heaters off, the head 30 mm up. Asks once.
  button({icon:'i-play',label:'Dry run',size:'row',cmd:'dry_run',title:'Dry run',does:'Runs every move of this file with nothing extruded, the heaters off and the head 30 mm above the plate. Shows the command once first.',onPress:async b=>{busy(b,true);if(await ctx.send('dry_run',{filename:path}))closeSheet();busy(b,false);}}));
 side.append(slicer,acts);hero.append(img,side);body.append(hero);
 body.prepend(button({icon:'i-back',label:'All files',size:'row',kind:'ghost',title:'Back to the files',does:'Returns to the file list.',onPress:()=>ctx.files?.()}));
 const S={time:stat('PRINT TIME'),layers:stat('LAYERS'),height:stat('HEIGHT'),fil:stat('FILAMENT'),temps:stat('FIRST LAYER'),nozzle:stat('NOZZLE')};body.append(statGrid(...Object.values(S)));
 const set=el('dl','nx-specs');body.append(section('Slicer settings',set));ctx.lockControls(body);
 // F06 (2026-10-19): a PrusaLink printer sends the whole file or nothing, so its slicer settings are read when the person asks (settings=1), and the facts the
 // printer cannot give (slicer, estimate, filament) come from the file's own comments once it has been read. Looking at a file never makes the printer send it.
 const paint=d=>{const m=d.metadata||{},
   // F05 (2026-10-19): PrusaSlicer writes an empty setting as two quote marks (""); it was shown as is. Empty settings are left out.
   s=Object.fromEntries(Object.entries(d.settings||{}).map(([k,v])=>[k,typeof v==="string"?v.replace(/^\s*""\s*$/,""):v]).filter(([,v])=>v!==""&&v!=null));set.replaceChildren();
  slicer.replaceChildren(el('b','',[m.slicer,m.slicer_version].filter(Boolean).join(' ')||'Slicer not recorded'),el('span','',[s.printer_settings_id,s.print_settings_id,s.filament_settings_id].filter(Boolean).join(' · ')||'no profile names in the file'),
   el('span','',[m.size?(m.size/1024/1024).toFixed(2)+' MB':'',m.modified?'sliced '+new Date(m.modified*1000).toLocaleString():''].filter(Boolean).join(' · ')));
  S.time.set(finite(m.estimated_time)===null?'—':fmtDuration(m.estimated_time),{note:'slicer estimate'});S.layers.set(m.layer_count??'—',{note:finite(m.layer_height)===null?'':`${m.layer_height} mm, first ${m.first_layer_height??'—'} mm`});
  S.height.set(finite(m.object_height)===null?'—':m.object_height+' mm');S.fil.set(finite(m.filament_total)===null?'—':(m.filament_total/1000).toFixed(2)+' m',{note:[m.filament_name||m.filament_type,!(finite(m.filament_weight_total)>0)?'':m.filament_weight_total.toFixed(1)+' g'].filter(Boolean).join(' · ')});
  if(Array.isArray(m.filament_colors)&&m.filament_colors[0])S.fil.el.style.setProperty('--swatch',m.filament_colors[0]),S.fil.el.dataset.swatch='';
  S.temps.set(finite(m.first_layer_extr_temp)===null?'—':`${m.first_layer_extr_temp}° / ${m.first_layer_bed_temp??'—'}°`,{note:'nozzle / bed'});S.nozzle.set(finite(m.nozzle_diameter)===null?'—':m.nozzle_diameter+' mm');
  const L={printer_settings_id:'Printer profile',print_settings_id:'Process',filament_settings_id:'Filament profile',layer_height:'Layer height',initial_layer_print_height:'First layer',wall_loops:'Walls',top_shell_layers:'Top layers',bottom_shell_layers:'Bottom layers',sparse_infill_density:'Infill',sparse_infill_pattern:'Infill pattern',enable_support:'Supports',support_type:'Support type',brim_type:'Brim',brim_width:'Brim width',nozzle_temperature:'Nozzle',nozzle_temperature_initial_layer:'Nozzle, first layer',bed_temperature:'Bed',hot_plate_temp:'Plate',filament_flow_ratio:'Flow ratio',pressure_advance:'Pressure advance',outer_wall_speed:'Outer wall speed',inner_wall_speed:'Inner wall speed',sparse_infill_speed:'Infill speed',travel_speed:'Travel speed',default_acceleration:'Acceleration',outer_wall_acceleration:'Outer wall accel',fan_max_speed:'Fan max',fan_min_speed:'Fan min',seam_position:'Seam',ironing_type:'Ironing',fill_density:'Infill',perimeters:'Perimeters',temperature:'Nozzle',first_layer_temperature:'Nozzle, first layer',first_layer_bed_temperature:'Bed, first layer',extrusion_multiplier:'Flow',filament_type:'Filament',nozzle_diameter:'Nozzle'};
  for(const [k,v]of Object.entries(s)){set.append(el('dt','',L[k]||k),el('dd','',v));}
  if(!Object.keys(s).length){
   if(d.settings_state==='not_read'){const mb=m.size?(m.size/1048576).toFixed(1)+' MB':'';
    const ask=el('div','nx-file-read');ask.append(el('p','nx-note',`The settings are at the end of the file, and this printer only sends a whole file. Reading them copies the file${mb?` (${mb})`:''} from the printer once; a big one takes a while, and the printer's own page is slow meanwhile.`),
     button({icon:'i-files',label:'Read the settings from the file',size:'row',title:'Read the settings',does:'Copies this file from the printer once and reads the settings the slicer wrote at its end.',onPress:async b=>{busy(b,true);try{paint(await getJson('/api/print/file?filename='+encodeURIComponent(path)+'&settings=1'));}catch(e){set.replaceChildren(el('dd','','The file could not be read: '+e.message));}}}));set.append(ask);}
   else set.append(el('dd','',{binary:'This is a binary G-code file (.bgcode): the printer reads its settings itself, and Stella Scope does not.',failed:'The printer did not send the file, so its settings could not be read. Try again in a moment.'}[d.settings_state]||'The file carries no slicer settings block.'));}};
 getJson('/api/print/file?filename='+encodeURIComponent(path)).then(paint)
 .catch(e=>{slicer.textContent='File details unavailable: '+e.message;});}});}

// ---------- rail pages ----------
// Levelling (2026-09-25: "we can actually make a new screen for this part"). Tabs, no scroll.
// Now: the four gantry corners from above, each with how far it was moved and a lit check when in
// tolerance, the rounds of the newest run against the tolerance line, and the mesh filling in live.
// Gantry history: every run. Mesh: the saved full mesh and both mesh histories.
// 2026-10-25: the plate in the top bar opened the bed heater part ("10IN MIC6 plate", the CAD's name for the bed). It opens the plates:
// which one is on the bed, the Z offset each prints at, and the list to add to or take from.
export function pagePlates(){sheet({title:'Build plates',eyebrow:'WHICH ONE IS ON THE BED',icon:'i-panels',build:body=>{const paint=sheetPicker(body);return ()=>paint();}});}
export function pageLevel(){sheet({title:'Levelling',eyebrow:'GANTRY AND BED',icon:'i-nav-leveling',wide:true,build:body=>{
 const T=tabbed(body,[['now','Now'],['gantry','Gantry history'],['mesh','Mesh']]);
 const acts=el('div','nx-grid4');acts.append(cmdButton('Home','i-home','home','Homes X, Y and Z.'),cmdButton('Level gantry','i-level','qgl','Probes the four corners and levels the gantry.',{},'primary'),cmdButton('Bed mesh','i-mesh','mesh','Probes the bed and makes a fresh bed mesh.'),cmdButton('Probe accuracy','i-probe','console','Probes one spot ten times and reports the spread.',{script:'PROBE_ACCURACY'}));
 const now=el('div','nx-lv-now'),gantry=el('div','nx-lv-gantry'),rounds=el('div','nx-lv-rounds'),live=meshViewer({height:300});
 const corner=(k,label)=>{const c=el('div','nx-lv-corner');c.dataset.k=k;c.append(el('span','',label),el('b','','not run yet'),el('em','nx-lv-dir'));return c;};   // U06: no little arrow under the number; the direction is a word on the card
 const C={z1:corner('z1','BACK LEFT'),z2:corner('z2','BACK RIGHT'),z0:corner('z0','FRONT LEFT'),z3:corner('z3','FRONT RIGHT')};
 const frame=el('div','nx-lv-frame');frame.append(el('div','nx-lv-beam'),el('span','nx-lv-front','FRONT'));gantry.append(C.z1,C.z2,frame,C.z0,C.z3);
 const verdict=el('div','nx-lv-verdict');now.append(gantry,el('div','nx-lv-side'));now.lastChild.append(verdict,rounds);
 T.now.append(acts,now,section('Mesh, as it is probed',live.el));
 const q=el('div','nx-qgl');T.gantry.append(q);qglRuns(q);
 const sv=meshViewer({height:360}),hist=el('div','nx-meshhist');T.mesh.append(section('Saved in Klipper',sv.el),section('History',hist));savedMesh(sv);const hv=meshViewer({height:300});hist.after(hv.el);meshHistory(hist,hv);
 document.addEventListener('nx-tab',()=>{live.draw();sv.draw();hv.draw();});
 let qAt=0,run=[];
 async function loadRun(){try{const d=await getJson('/api/qgl'),rs=d.rounds||[];if(!rs.length){run=[];return;}const id=rs[0].run_id;run=rs.filter(r=>r.run_id===id).sort((a,b)=>a.round-b.round);}catch{}}
 // 2026-09-28: the view "shows no numbers until the first probe round finishes and is slow to
 // update". Klipper prints a round's range only when the round ends, so until then the page shows what
 // is already known: the run has started, which corners have been probed and how far apart they read so
 // far, and the last finished run for reference. Each round's range against the tolerance follows as it
 // happens, and the page asks for it every second while a level runs.
 function paintRun(){const a=ctx.activity(),lev=!!a&&a.id==='levelling',since=lev?(a.since_ts??0):0,cur=lev?run.filter(r=>r.ts>=since-5):run,last=cur.at(-1);rounds.replaceChildren();
  if(lev&&!cur.length){
   const travel=xyTravel(ctx.tick()?.point?.limits||point()?.limits),pts=liveProbes(0).filter(p=>p.ts>=since-5),seen={};
   if(travel)for(const p of pts)seen[{'0,0':'z0','0,1':'z1','1,1':'z2','1,0':'z3'}[(p.x>travel.cx?1:0)+','+(p.y>travel.cy?1:0)]]=p.z;
   const zs=Object.values(seen),spread=zs.length>1?Math.max(...zs)-Math.min(...zs):null;
   for(const [k,c] of Object.entries(C)){const v=seen[k];c.querySelector('b').textContent=finite(v)===null?'probing…':'z '+v.toFixed(3);c.removeAttribute('data-ok');c.dataset.dir='';c.querySelector('.nx-lv-dir').textContent='';c.toggleAttribute('data-hit',finite(v)!==null);}
   verdict.textContent=`LEVELLING · round 1 · ${zs.length} of 4 corners probed${spread===null?'':`, ${spread.toFixed(3)} mm apart so far`}`;verdict.dataset.tone='live';
   const prev=run.at(-1);if(prev){const row=el('div','nx-lv-round');row.dataset.prev='';row.append(el('span','','Last run'),el('span','nx-lv-prev',`${prev.pass?'level':'not level'} · ${new Date(prev.ts*1000).toLocaleTimeString([],{hour:'numeric',minute:'2-digit'})}`),el('b','',prev.range_mm.toFixed(3)+' mm'));rounds.append(row);}
   return;}
  for(const c of Object.values(C))c.removeAttribute('data-hit');
  if(!last){verdict.textContent='No gantry levelling recorded yet. Press Level gantry.';verdict.dataset.tone='';for(const c of Object.values(C)){c.querySelector('b').textContent='not run yet';c.removeAttribute('data-ok');c.querySelector('.nx-lv-dir').textContent='';}return;}
  const tol=last.tolerance_mm??.02;verdict.textContent=last.pass?`LEVEL · ${last.range_mm.toFixed(3)} mm across, inside ${tol} mm, in ${cur.length} round${cur.length>1?'s':''}`:`${lev?'LEVELLING · ':''}Round ${last.round+1} · ${last.range_mm.toFixed(3)} mm across, needs under ${tol} mm`;verdict.dataset.tone=last.pass?'ok':lev?'live':'warn';
  for(const [k,c] of Object.entries(C)){const v=last[k];c.querySelector('b').textContent=finite(v)===null?'—':mm(v);const ok=last.pass||(finite(v)!==null&&Math.abs(v)<=tol);c.toggleAttribute('data-ok',ok);c.dataset.dir=finite(v)===null?'':v>0?'up':'down';c.querySelector('.nx-lv-dir').textContent=finite(v)===null?'':v>0?'moved up':'moved down';}
  const mx=Math.max(...cur.map(r=>r.range_mm),tol*2);for(const r of cur){const row=el('div','nx-lv-round'),track=el('div','nx-lv-track'),f=el('i'),line=el('em');f.style.width=Math.max(2,r.range_mm/mx*100)+'%';f.dataset.pass=String(r.pass);line.style.left=Math.min(100,tol/mx*100)+'%';track.append(f,line);
   row.append(el('span','',`Round ${r.round+1}`),track,el('b','',r.range_mm.toFixed(3)+' mm'));rounds.append(row);}
  if(lev&&!last.pass)rounds.append(el('p','nx-note','Probing the next round…'));}
 loadRun().then(paintRun);ctx.lockControls(body);
 return ()=>{live.live(liveProbes(0));const a=ctx.activity(),lev=!!a&&a.id==='levelling';
  if(performance.now()-qAt>(lev?1000:15000)){qAt=performance.now();loadRun().then(paintRun);}else if(lev)paintRun();};}});}
async function qglRuns(host){try{const d=await getJson('/api/qgl'),rounds=d.rounds||[];const runs=new Map();for(const r of rounds){if(!runs.has(r.run_id))runs.set(r.run_id,[]);runs.get(r.run_id).push(r);}
  host.replaceChildren();if(!runs.size){host.append(el('p','nx-note','No gantry levelling in the recording.'));return;}
  for(const rs of [...runs.values()].slice(0,6)){rs.sort((a,b)=>a.round-b.round);const last=rs.at(-1),card=el('div','nx-qgl-run');const head=el('div','nx-qgl-head');
   head.append(el('b','',new Date(rs[0].ts*1000).toLocaleString()),el('span',last.pass?'ok':'warn',last.pass?`levelled in ${rs.length} round${rs.length>1?'s':''}`:'did not finish level'));card.append(head);
   const bars=el('div','nx-qgl-bars'),mx=Math.max(...rs.map(r=>r.range_mm),last.tolerance_mm*2);
   for(const r of rs){const row=el('div','nx-qgl-bar'),f=el('i');f.style.width=Math.max(2,r.range_mm/mx*100)+'%';f.dataset.pass=String(r.pass);row.append(el('span','',`round ${r.round+1}`),Object.assign(el('div','nx-qgl-track'),{}),el('b','',r.range_mm.toFixed(3)+' mm'));row.children[1].append(f);
    const tol=el('em');tol.style.left=Math.min(100,last.tolerance_mm/mx*100)+'%';row.children[1].append(tol);bars.append(row);}
   const corners=el('div','nx-qgl-corners');for(const [k,label]of [['z1','BACK LEFT'],['z2','BACK RIGHT'],['z0','FRONT LEFT'],['z3','FRONT RIGHT']]){const c=el('div','nx-qgl-corner');const v=last[k];c.append(el('span','',label),el('b','',mm(v)));if(Math.abs(v)>0.02)c.dataset.far='';corners.append(c);}
   card.append(bars,corners);host.append(card);}}catch(e){host.replaceChildren(el('p','nx-note','Gantry levelling history unavailable: '+e.message));}}
// Heat & fans (U06, 2026-10-20): nozzle and bed are the kit's controls (nozzle 5°, bed 1°) beside their last ten minutes; the three fans are controls too (the part fan turns, the other two are run by Klipper and only read).
// Each sends once, when the hand lets go, through the same set_heater and set_fan every page uses.
const HEAT_MAX={nozzle:290,bed:120};   // the service takes 290 at most on the nozzle; the dial runs to 300, so a turn past it is sent as 290
function heatDeck(which){const nz=which==='nozzle',id=nz?'extruder':'heater_bed',lim=HEAT_MAX[which],root=el('div','nx-ht'),side=el('div','nx-ht-side');root.dataset.k=which;
 const c=ctl(which,{value:0,now:null,onSet:v=>{if(v>lim){v=lim;c.spec.value=v;c.paint();}return ctx.send('set_heater',{target:id,value:v});}});
 tip(c.el,nz?'Nozzle':'Bed',`Press and hold, then turn: the ${which} target moves ${nz?5:1}° a click. Letting go sends it once. The dot on the ring is the live temperature.`);
 const th=thermoCard({label:nz?'Nozzle':'Bed',icon:nz?'i-heat':'i-bed',heatKind:which});th.el.removeAttribute('role');th.el.removeAttribute('tabindex');
 // 2026-10-25: "now, target, heater input, output ... different size panels" read as buttons. One instrument per heater: the dial with its
 // readings beside it as plain lines, the graph of the last ten minutes the whole width under them.
 const T=stat('NOW'),G=stat('TARGET'),P=stat('HEATER POWER'),figs=el('div','nx-ht-figs'),top=el('div','nx-ht-top');figs.append(T.el,G.el,P.el);side.append(figs);top.append(c.el,side);root.append(top,th.el);
 return {el:root,set(x,locked){c.setUnknown(x.target===null||x.target===undefined);c.setNow(finite(x.temp));if(finite(x.target)!==null&&!c.el.classList.contains('live'))c.set(x.target);c.setLocked(locked);
  th.set(x);const h=heatColor(x.temp,x.target,which);T.set(fmtTemp(x.temp),{tone:h.tone==='danger'?'alarm':''});G.set(x.target>0?fmtTemp(x.target):'off');P.set(finite(x.power)===null?'—':Math.round(x.power*100)+' %');}};}
export function pageThermal(){sheet({title:'Heat & fans',eyebrow:'THERMAL',icon:'i-nav-thermal',wide:true,build:body=>{
 const n=heatDeck('nozzle'),b=heatDeck('bed'),g=el('div','nx-ht-grid');g.append(n.el,b.el);
 const part=ctl('part',{value:0,onSet:v=>ctx.send('set_fan',{value:v/100})}),hot=ctl('hotend',{value:0,readOnly:true,onTap:()=>openPartByKey('hotend_fan')}),ctrl=ctl('board',{value:0,readOnly:true,onTap:()=>openPartByKey('controller_fan')});
 tip(part.el,'Part cooling fan','Press and hold, then turn: the part cooling fan moves 5% a click. Letting go sends it once.');tip(hot.el,'Hotend fan','Automatic: Klipper runs it whenever the nozzle is hot, so it does not turn. Tap to see its panel.');tip(ctrl.el,'Electronics bay fans','Automatic: Klipper runs them while the motors or heaters are on, so they do not turn. Tap for details.');
 const fr=el('div','nx-ht-fans'),cell=(c,k,note)=>{const w=el('div','nx-ht-fan');w.dataset.k=k;w.append(c.el,el('small','',note));return w;};
 const fans={part:[part,cell(part,'part','you control it')],hotend:[hot,cell(hot,'hotend','automatic')],controller:[ctrl,cell(ctrl,'controller','automatic')]};fr.append(...Object.values(fans).map(x=>x[1]));
 for(const [k,off] of [['hotend',!ctx.has('klipper')],['controller',!ctx.has('klipper')],['part',!ctx.has('fan')]])fans[k][1].style.display=off?'none':'';   // polish WO-05: only the fans this printer reports
 // finish E18: a printer that only measures its fans (a Prusa reports their turns a minute, and takes no fan command from here) shows them as it says them
 const rpm=el('p','nx-note nx-ht-rpm');rpm.hidden=true;
 body.append(g,ctx.has('fan')||ctx.has('klipper')?section('Fans',fr):section('Fans',rpm),el('p','nx-note',ctx.has('klipper')?'The graphs show the last ten minutes. A fan figure is the speed Klipper asked for, not a measured speed.':'The graphs show the last ten minutes.'));
 const lk=ctx.can('set_heater')?'':ctx.why('set_heater');
 return ()=>{const r=ctx.state.readings,h=ctx.state.hist,stale=document.body.hasAttribute('data-stale'),lock=ctx.can('set_heater')?'':ctx.why('set_heater');
  n.set({temp:r.nozzle.temp,target:r.nozzle.target,power:r.nozzle.power,history:h.nozzle,stale},lock);b.set({temp:r.bed.temp,target:r.bed.target,power:r.bed.power,history:h.bed,stale},lock);
  for(const [k,[c]] of Object.entries(fans)){const f=stale?{speed:null}:r.fans[k],v=f?.speed;c.setUnknown(v===null||v===undefined);if(finite(v)!==null)c.set(Math.round(v*100));}
  part.setLocked(ctx.can('set_fan')?'':ctx.why('set_fan'));
  if(!ctx.has('fan')&&!ctx.has('klipper')){const R=v=>finite(v)===null?null:Math.round(v).toLocaleString()+' rpm',pf=R(r.fans.part?.rpm),hf=R(r.fans.hotend?.rpm);rpm.hidden=!(pf||hf);rpm.textContent=[pf&&'Part fan '+pf,hf&&'Hotend fan '+hf].filter(Boolean).join('  ·  ')+(pf||hf?'. Measured by the printer.':'');}};}});}
// 2026-10-23: every signal says what it is in plain words first; Klipper's own name stays beside it, small, for the config and the forums.
const SIG_OBJ=[[/^extruder$/,'Nozzle heater'],[/^heater_bed$/,'Bed heater'],[/^fan$/,'Part fan'],[/^controller_fan(\s|$)/,'Controller fan'],[/^heater_fan(\s|$)/,'Hotend fan'],[/^gcode_move$/,'Motion as G-code sees it'],
 [/^toolhead$/,'Toolhead'],[/^print_stats$/,'Print'],[/^motion_report$/,'Live motion'],[/^virtual_sdcard$/,'Print file'],[/^quad_gantry_level$/,'Gantry levelling'],[/^bed_mesh$/,'Bed mesh'],[/^probe$/,'Probe'],[/^bltouch$/,'BLTouch'],
 [/^webhooks$/,'Klipper'],[/^system_stats$/,'Host computer'],[/^idle_timeout$/,'Idle timer'],[/^display_status$/,'Display'],[/^exclude_object$/,'Object skipping'],[/^filament_switch_sensor\s/,'Runout sensor'],[/^stepper_enable$/,'Motors on/off'],[/^z_tilt$/,'Z tilt']];
const SIG_FIELD={temperature:'Temperature',target:'Target',power:'Heater power',speed:'Speed',rpm:'Fan RPM',pressure_advance:'Pressure advance',smooth_time:'Pressure advance smoothing',extrude_factor:'Flow',speed_factor:'Speed factor',
 live_velocity:'Velocity',live_extruder_velocity:'Extruder velocity',max_velocity:'Max velocity',max_accel:'Max acceleration',square_corner_velocity:'Corner speed',progress:'Progress',print_duration:'Printing time',total_duration:'Total time',
 filament_used:'Filament used',state:'State',message:'Message',run_current:'Run current',hold_current:'Hold current',cputime:'CPU time',memavail:'Free memory',sysload:'System load',applied:'Done',filament_detected:'Filament in'};
export function plainSignal(object,field){const t=String(object||''),m=SIG_OBJ.find(([r])=>r.test(t));let o=m?m[1]:'';
 if(!o){const tm=t.match(/^(tmc\d+)\s+(stepper_|extruder)(\w*)/);if(tm)o='Motor driver '+(tm[2]==='extruder'?'extruder':tm[3].toUpperCase())+' ('+tm[1].toUpperCase()+')';}
 if(!o){const ts=t.match(/^temperature_(sensor|fan)\s+(.+)/);if(ts)o=(ts[1]==='fan'?'Fan: ':'Temperature: ')+ts[2].replace(/_/g,' ');}
 if(!o)o=t.replace(/_/g,' ').replace(/^./,c=>c.toUpperCase());
 const fs0=String(field||''),ax=fs0.match(/^(\w+)\.(\d)$/),AX='XYZE';let p=SIG_FIELD[fs0]||'';
 if(!p&&ax)p=({homing_origin:'Homing offset',position:'Position',gcode_position:'G-code position',live_position:'Live position'}[ax[1]]||ax[1].replace(/_/g,' '))+' '+(AX[+ax[2]]||ax[2]);
 if(!p)p=fs0.replace(/_/g,' ').replace(/^./,c=>c.toUpperCase());return {object:o,field:p};}
export function pageSignals(){sheet({title:'Signals',eyebrow:'EVERY RECORDED SIGNAL',icon:'i-nav-signals',wide:true,build:body=>{
 const q=el('input','nx-input');q.type='search';q.placeholder='Find a signal, e.g. bed heater, motor driver Z or temperature';q.setAttribute('aria-label','Filter signals');const count=el('p','nx-note'),list=el('div','nx-sig-list');body.append(q,count,list);
 let last='';const paint=()=>{const all=point()?.telemetry?.signals||[],f=q.value.trim().toLowerCase(),rows=all.filter(s=>{if(!f)return true;const n=plainSignal(s.object,s.field);return (s.object+' '+s.field+' '+n.object+' '+n.field).toLowerCase().includes(f);});
  const key=f+'|'+rows.length+'|'+(point()?.ts||0);if(key===last)return;last=key;count.textContent=`${rows.length} of ${all.length} signals · newest sample ${point()?.ts?ago(point().ts):'—'}`;
  list.replaceChildren();let obj=null;for(const s of rows.slice(0,400)){const n=plainSignal(s.object,s.field);if(s.object!==obj){obj=s.object;const h=el('div','nx-sig-obj',n.object);h.append(el('small','nx-sig-raw',s.object));list.append(h);}const r=el('div','nx-raw-row'),nm=el('span','nx-sig-name',n.field);nm.append(el('small','nx-sig-raw',s.field));r.append(nm,el('b','',formatSignal(s.object,s.field,s.value)));list.append(r);}};
 q.addEventListener('input',()=>{last='';paint();});return paint;}});}
export function pageTimeline(){sheet({title:'Timeline',eyebrow:'EVERYTHING THE PRINTER SAID',icon:'i-nav-timeline',wide:true,build:body=>{
 let kind='all';const list=el('div','nx-tl'),seg=segmented(['all','command','response','alert'],kind,v=>{kind=v;draw();},v=>({all:'All',command:'Commands',response:'Replies',alert:'Alerts'}[v]));body.append(seg,list);let rows=[];
 const draw=()=>{list.replaceChildren();const keep=rows.filter(e=>kind==='all'||(kind==='alert'?!['command','response','connection','recorder'].includes(e.kind):e.kind===kind));let day='';
  for(const e of keep.slice(0,300)){const d=new Date(e.ts*1000),ds=d.toLocaleDateString();if(ds!==day){day=ds;list.append(el('div','nx-tl-day',ds));}
   const r=el('div','nx-tl-row');r.dataset.kind=e.kind;r.append(el('span','nx-tl-time',d.toLocaleTimeString()),el('span','nx-tl-kind',e.kind.replace('_',' ')),el('span','nx-tl-msg',e.message||''));list.append(r);}
  if(!keep.length)list.append(el('p','nx-note','Nothing of this kind recorded yet.'));};
 getJson('/api/events?limit=500').then(d=>{rows=(d.events||[]).filter((e,i,a)=>!(e.kind==='printer_offline'&&a[i-1]?.kind==='printer_offline'));draw();}).catch(e=>list.append(el('p','nx-note',e.message)));}});}
export function pageParts(filter=null){sheet({title:filter==='electronics'?'Electronics':'Parts',eyebrow:filter==='electronics'?'BOARDS, HOST AND FANS':'WHAT IS FITTED',icon:filter==='electronics'?'i-nav-electronics':'i-nav-parts',wide:true,build:body=>{
 const sc=ctx.scene(),q=el('input','nx-input');q.type='search';q.placeholder='Find a part';q.setAttribute('aria-label','Find a part');const list=el('div','nx-parts');body.append(q,list);
 if(!sc?.entries?.length){list.append(el('p','nx-note',document.body.dataset.scene==='off'?'This list comes from the detailed drawing of the printer, which is off on this computer. Click any part of the model instead: its name and the page that shows it live open.':'The 3D model is still loading.'));return;}
 const seen=new Set(),wired=[],rest=[];for(const e of sc.entries){if(e.sourceOnly||/shcs|bhcs|fhcs|screw|bolt|nut|washer|insert|magnet|^\d/i.test(e.name||''))continue;const w=wiringFor(e),n=(e.name||'').replace(/:\d+$/,'').replace(/(\s*\(\d+\))+$/,'');
  const k=(w?.key||'')+n;if(seen.has(k))continue;seen.add(k);(w?wired:rest).push({e,w,n});}
 const elec=wired.filter(x=>['controller','toolboard','host','bed_relay','controller_fan'].includes(x.w.key));
 const draw=()=>{const f=q.value.trim().toLowerCase();list.replaceChildren();const groups=filter==='electronics'?[['Wired',elec]]:[['Live parts, with signals',wired],['Everything else',rest.filter(x=>/^Stella \//.test(x.e.full||'')||f)]];
  for(const [name,items]of groups){const its=items.filter(x=>!f||(x.n+' '+(x.w?.label||'')).toLowerCase().includes(f)).slice(0,160);if(!its.length)continue;list.append(el('h3','nx-parts-h',name));
   const g=el('div','nx-parts-grid');for(const x of its){const b=el('button','nx-part');b.type='button';b.append(icon(x.w?/fan/.test(x.w.key)?'i-fan':x.w.key==='bed'?'i-bed':/stepper|motor/.test(x.w.key)?'i-motors-off':'i-live':'i-nav-parts'),Object.assign(el('span','nx-part-t'),{}));
    b.lastChild.append(el('b','',x.n),el('small','',x.w?x.w.label:(x.e.slot_id||x.e.layer||'')));if(x.w)b.dataset.wired='';b.addEventListener('click',()=>openPart(x.e.id));g.append(b);}list.append(g);}};
 q.addEventListener('input',draw);draw();}});}

// ---------- live activity card: a gantry level or mesh while it runs ----------
// Shown on the main view while the recorder's activity says levelling or probing. The four corners
// light as the probe lands near them and show each round's correction; a mesh shows its points
// arriving on a small bed map. Tap it for the full levelling page.
export function activityCard(){const w=el('button','nx-activity nx-glass nx-glass--raised');w.type='button';w.hidden=true;
 const head=el('div','nx-act-head'),title=el('b'),detail=el('span');head.append(icon('i-level'),el('div','nx-act-t'));head.lastChild.append(title,detail);
 const corners=el('div','nx-act-corners'),C={};for(const [k,l]of [['z1','BACK LEFT'],['z2','BACK RIGHT'],['z0','FRONT LEFT'],['z3','FRONT RIGHT']]){const c=el('div','nx-act-c');c.append(el('span','',l),el('b','','—'));C[k]=c;corners.append(c);}
 const round=el('div','nx-act-round'),map=el('canvas','nx-act-map');
 // 2026-09-25: "once it's all level, I should be able to close the levelling thing out."
 const x=el('span','nx-act-x');x.setAttribute('role','button');x.setAttribute('aria-label','Close');x.append(icon('i-close'));tip(x,'Close','Hides this until the next levelling or mesh starts.');
 let dismissed='';x.addEventListener('click',e=>{e.stopPropagation();dismissed=key;w.hidden=true;});head.append(x);
 w.append(head,corners,round,map);w.addEventListener('click',()=>pageLevel());
 let qAt=0,rounds=[],since=0,key='';
 function paint(a){const on=a&&['levelling','probing','meshing'].includes(a.id);if(on)key=a.id+':'+(a.since_ts??'');w.hidden=!on||dismissed===key;if(!on||dismissed===key){if(!on)since=0;return;}
  if(!since)since=(a.since_ts??Date.now()/1000)-30;const qgl=a.id==='levelling'||/gantry/i.test(a.detail||''),mesh=!qgl&&(a.id==='meshing'||/mesh/i.test(a.detail||''));
  w.dataset.kind=qgl?'qgl':mesh?'mesh':'probe';title.textContent=qgl?'Levelling the gantry':mesh?'Probing the bed mesh':a.label||'Probing';detail.textContent=a.detail||'';
  const pts=liveProbes(since),travel=xyTravel(ctx.tick()?.point?.limits||point()?.limits);corners.hidden=!qgl;round.hidden=!qgl;map.hidden=!mesh||!travel;
  if(qgl){if(performance.now()-qAt>3000){qAt=performance.now();getJson('/api/qgl').then(d=>{const t0=(a.since_ts??since+30)-3;rounds=(d.rounds||[]).filter(r=>r.ts>=t0);}).catch(()=>{});}
   const r=rounds[0],now=ctx.tick()?.point?.ts??Date.now()/1000;
   for(const [k,c]of Object.entries(C)){const v=r?.[k],q0={z0:[0,0],z1:[0,1],z2:[1,1],z3:[1,0]}[k],hit=travel?pts.filter(p=>(p.x>travel.cx?1:0)===q0[0]&&(p.y>travel.cy?1:0)===q0[1]).at(-1):null;c.lastChild.textContent=finite(v)!==null?mm(v):finite(hit?.z)!==null?'probed '+mm(hit.z):'waiting';c.toggleAttribute('data-far',finite(v)!==null&&Math.abs(v)>(r?.tolerance_mm??.02));c.toggleAttribute('data-ok',!!r?.pass||(finite(v)!==null&&Math.abs(v)<=(r?.tolerance_mm??.02)));
    const q={z0:[0,0],z1:[0,1],z2:[1,1],z3:[1,0]}[k];c.toggleAttribute('data-hit',!!travel&&pts.some(p=>now-p.ts<5&&(p.x>travel.cx?1:0)===q[0]&&(p.y>travel.cy?1:0)===q[1]));}
   round.textContent=r?`Round ${r.round+1} · range ${r.range_mm.toFixed(3)} of ${r.tolerance_mm} mm${r.pass?' · LEVEL ✓':''}`:`Round 1 · ${travel?new Set(pts.map(p=>(p.x>travel.cx?1:0)+','+(p.y>travel.cy?1:0))).size:pts.length} of 4 corners probed…`;w.toggleAttribute('data-level',!!r?.pass);}
  if(mesh&&travel){const dpr=Math.min(devicePixelRatio||1,2),W=map.clientWidth||200,H=map.clientHeight||120;map.width=W*dpr;map.height=H*dpr;const g=map.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);g.clearRect(0,0,W,H);
   const s=Math.min(W/travel.width,H/travel.height),ox=(W-travel.width*s)/2,oy=(H-travel.height*s)/2;g.strokeStyle='rgba(255,255,255,.2)';g.strokeRect(ox,oy,travel.width*s,travel.height*s);
   const zs=pts.map(p=>p.z),m=zs.length?zs.reduce((a,b)=>a+b,0)/zs.length:0,sp=Math.max(.02,...zs.map(z=>Math.abs(z-m)));
   pts.forEach((p,i)=>{g.beginPath();g.arc(ox+(p.x-travel.x0)*s,oy+(travel.y1-p.y)*s,i===pts.length-1?4:2.6,0,Math.PI*2);g.fillStyle=css(meshRgb((p.z-m)/sp));g.fill();});}if(mesh)detail.textContent=`${pts.length} point${pts.length===1?'':'s'} so far · ${travel?'':'Travel limits unavailable · '}${a.detail||''}`;}
 return {el:w,paint};}

// ---------- print page ----------
export function pagePrint(){sheet({title:'Print',eyebrow:'THE JOB ON THE PLATE',icon:'i-print',wide:true,build:body=>{
 const hero=el('div','nx-print-hero'),img=el('img','nx-file-thumb');img.alt='';img.addEventListener('error',()=>{img.hidden=true;});   /* F04 (2026-10-19): a file with no picture shows no picture (a broken-image mark sat in the corner of the box) */const info=el('div','nx-print-info'),name=el('b'),meta=el('span'),bar=el('div','nx-print-bar'),fill=el('i');bar.append(fill);info.append(name,meta,bar);
 const acts=el('div','nx-grid3');acts.append(cmdButton('Pause','i-pause','pause','Pauses the print.'),cmdButton('Resume','i-play','resume','Resumes the paused print.'),button({icon:'i-cancel',label:'Cancel',size:'row',kind:'danger',cmd:'cancel',hold:true,title:'Cancel print',does:'Cancels the running print.',onPress:()=>ctx.send('cancel')}));
 info.append(acts);hero.append(img,info);body.append(hero);
 const S={layer:stat('LAYER'),time:stat('TIME LEFT'),fil:stat('FILAMENT USED'),speed:stat('SPEED'),flow:stat('FLOW'),vol:stat('FLOW RATE')};body.append(statGrid(...Object.values(S)));const bsPaint=babystep(body);
 const view=el('div','nx-layer'),cv=el('canvas','nx-layer-cv'),scrub=el('input','nx-input');scrub.type='range';scrub.min=1;scrub.setAttribute('aria-label','Layer to show');let follow=true;
 const lnote=el('p','nx-note');scrub.addEventListener('input',()=>{follow=false;draw();});const back=button({icon:'i-live',label:'Follow the print',size:'row',title:'Follow the print',does:'Shows the layer being printed again.',onPress:()=>{follow=true;draw();}});
 view.append(cv,scrub,lnote,back);body.append(section('Layer',view));
 const objs=el('div','nx-objs');if(ctx.has('klipper'))body.append(section('Objects on the plate',objs));let objKey='';   // polish WO-05 (2026-10-18): skipping an object is Klipper's own (exclude_object)
 function draw(){const tp=ctx.toolpath(),p=tp?.parsed;const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||600,H=cv.clientHeight||420;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);g.clearRect(0,0,W,H);
  back.hidden=follow;scrub.hidden=!p;if(!p){lnote.textContent=tp?.error||'No layers to show yet. They are read from the printer\'s file while a print runs.';return;}
  const n=p.layers.length;scrub.max=n;const cur=tp.layer??0;const L=follow?cur:Math.max(0,Math.min(n-1,Number(scrub.value)-1));if(follow)scrub.value=cur+1;
  const b=p.bounds,span=Math.max(b.max[0]-b.min[0],b.max[1]-b.min[1],10)*1.1,cx=(b.min[0]+b.max[0])/2,cy=(b.min[1]+b.max[1])/2,s=Math.min(W,H)/span,X=x=>W/2+(x-cx)*s,Y=y=>H/2-(y-cy)*s;
  const seg=(layer,style,upto=Infinity)=>{const l=p.layers[layer];if(!l)return;g.beginPath();for(let i=0;i<l.extrude.length;i++){if(!l.extrude[i])continue;if(l.offsets&&l.offsets[i]>upto)break;const q=l.segments.subarray(i*6,i*6+6);g.moveTo(X(q[0]),Y(q[1]));g.lineTo(X(q[3]),Y(q[4]));}g.strokeStyle=style;g.stroke();};
  g.lineCap='round';g.lineWidth=Math.max(1,.45*s);if(L>0)seg(L-1,'rgba(147,168,180,.18)');seg(L,'rgba(147,168,180,.55)');
  if(L===cur&&follow&&finite(tp.filePos)!==null){g.lineWidth=Math.max(1.2,.5*s);seg(L,'#FFC24A',tp.filePos);}
  else if(L<cur){g.lineWidth=Math.max(1.2,.5*s);seg(L,ink('--accent','#04D9C4'));}
  const pos=ctx.state.readings.position;if(L===cur&&finite(pos.x)!==null){g.beginPath();g.arc(X(pos.x),Y(pos.y),5,0,Math.PI*2);g.fillStyle='#fff';g.fill();}
  lnote.textContent=`Layer ${L+1} of ${n} · Z ${p.layers[L]?.z?.toFixed(2)??'—'} mm${L===cur?' · printing now (orange is done)':L<cur?' · printed':' · still to come'}${p.truncated?' · file truncated for drawing':''}${tp.source?' · '+tp.source:''}`;}
 async function objects(){const tp=ctx.toolpath();let d;try{d=await getJson('/api/print/objects');}catch(e){objs.replaceChildren(el('p','nx-note',e.message));return;}
  const list=d.objects||[],ex=new Set(d.excluded||[]),k=JSON.stringify([list.map(o=>o.name),[...ex],d.current_object,!!tp?.parsed]);if(k===objKey)return;objKey=k;objs.replaceChildren();
  if(!list.length){objs.append(el('p','nx-note','Nothing to skip in this print. Turn on Label objects in your slicer and each part of the plate can be skipped on its own.'));return;}
  for(const o of list){const card=el('div','nx-obj'),pc=el('canvas','nx-obj-cv');card.append(pc,el('b','',o.name));if(ex.has(o.name))card.dataset.excluded='';if(d.current_object===o.name)card.dataset.current='';
   card.append(ex.has(o.name)?el('span','nx-obj-state','skipped'):button({icon:'i-exclude',label:'Skip',size:'row',kind:'danger',cmd:'exclude_object',hold:true,title:'Skip '+o.name,does:'Stops printing this object; the rest carries on.',onPress:()=>ctx.send('exclude_object',{name:o.name})}));
   objs.append(card);requestAnimationFrame(()=>objectPicture(pc,o,tp?.parsed));}ctx.lockControls(objs);}
 objects();let oAt=performance.now();
 return ()=>{bsPaint();const r=ctx.state.readings,j=r.job,t=ctx.tick()?.job||{},tp=ctx.toolpath(),active=['printing','paused'].includes(j.state);
  if(img.dataset.f!==j.name){img.dataset.f=j.name;img.hidden=!j.name;if(j.name&&ctx.has('thumbnails'))img.src='/api/print/thumbnail?filename='+encodeURIComponent(j.name);else if(!ctx.has('thumbnails'))img.hidden=true;}
  name.textContent=j.name?fileTitle(j.name):'No print on the plate';const pr=finite(j.progress);fill.style.width=((pr??0)*100).toFixed(1)+'%';
  const dur=finite(j.duration),rem=pr!==null&&pr>0.01&&dur!==null?Math.round(dur/pr-dur):null;const said={printing:'Printing',paused:'Paused',complete:'Finished',cancelled:'Cancelled',error:'Stopped by an error',standby:'Ready'}[j.state]||j.state;meta.textContent=j.name?[said,pr===null?'':Math.round(pr*100)+'%',dur===null?'':fmtDuration(dur)+(active?' so far':' printing')].filter(Boolean).join(' · '):'Pick a file to print.';
  const lay=finite(t.layer)??(tp?.parsed&&tp.layer!=null?tp.layer+1:null),lays=finite(t.layers)??tp?.parsed?.totalLayers??null;S.layer.set(lay===null?'—':`${lay} / ${lays??'?'}`,{note:lay===null&&active?'the slicer did not write layer numbers':''});S.time.set(active?(rem===null?'—':fmtDuration(rem)):'done',{note:active&&rem===null?'worked out once it has printed a little':''});
  S.fil.set(finite(j.filament)===null?'—':(j.filament/1000).toFixed(2)+' m');S.speed.set(r.speedFactor===null?'—':Math.round(r.speedFactor*100)+' %',{note:r.velocity===null?'':Math.round(r.velocity)+' mm/s now'});
  S.flow.set(r.extrudeFactor===null?'—':Math.round(r.extrudeFactor*100)+' %');S.vol.set(finite(r.extruderVelocity)===null?'—':(Math.max(0,r.extruderVelocity)*Math.PI*0.765625).toFixed(1)+' mm³/s');S.vol.el.hidden=!active;S.time.el.hidden=!active&&!j.name;
  for(const b of acts.querySelectorAll('.nx-btn'))b.hidden=!active||(b.dataset.cmd==='pause'&&j.state==='paused')||(b.dataset.cmd==='resume'&&j.state!=='paused');
  if(follow||!tp?.parsed)draw();if(performance.now()-oAt>5000){oAt=performance.now();objects();}ctx.lockControls(body);};}});}
// One object's footprint from the G-code: every extrusion between its EXCLUDE_OBJECT_START and END.
function objectPicture(cv,o,p){const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||120,H=cv.clientHeight||90;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);
 const po=p?.objects?.find(x=>x.name===o.name);const segs=[];if(po){let base=0;for(const l of p.layers){const n=l.extrude.length;if(base+n>po.start&&base<po.end)for(let i=Math.max(0,po.start-base);i<Math.min(n,po.end-base);i++)if(l.extrude[i])segs.push(l.segments.subarray(i*6,i*6+6));base+=n;if(base>=po.end)break;}}
 const poly=Array.isArray(o.polygon)?o.polygon:[];const xs=[...segs.flatMap(q=>[q[0],q[3]]),...poly.map(v=>v[0])],ys=[...segs.flatMap(q=>[q[1],q[4]]),...poly.map(v=>v[1])];
 if(!xs.length){g.fillStyle='#8B95A1';g.font='500 11px Geist,sans-serif';g.textAlign='center';g.fillText('no outline',W/2,H/2);return;}
 const x0=Math.min(...xs),x1=Math.max(...xs),y0=Math.min(...ys),y1=Math.max(...ys),s=Math.min((W-12)/((x1-x0)||1),(H-12)/((y1-y0)||1)),X=x=>W/2+(x-(x0+x1)/2)*s,Y=y=>H/2-(y-(y0+y1)/2)*s;
 if(poly.length){g.beginPath();poly.forEach((v,i)=>i?g.lineTo(X(v[0]),Y(v[1])):g.moveTo(X(v[0]),Y(v[1])));g.closePath();g.fillStyle=inkA(.10);g.fill();g.strokeStyle=inkA(.6);g.stroke();}
 g.beginPath();for(const q of segs.length>20000?segs.filter((_,i)=>i%Math.ceil(segs.length/20000)===0):segs){g.moveTo(X(q[0]),Y(q[1]));g.lineTo(X(q[3]),Y(q[4]));}g.strokeStyle='rgba(238,242,246,.35)';g.lineWidth=.6;g.stroke();}

// ---------- alerts: the week's trouble in one list ----------
const KLIPPER_TROUBLE=new RegExp(['^!!','error','shut'+'down','lost communication','ADC out of range','timer too close','move out of range'].join('|'),'i');
export function pageAlerts(){sheet({plain:true,title:'Alerts',eyebrow:'THE LAST SEVEN DAYS',icon:'i-warn',wide:true,build:body=>{
 // polish WO-05 (2026-10-18): driver flags, Klipper's own errors and retransmits exist only for Klipper; another brand has offline and restarts.
 const kp=ctx.has('klipper'),S={off:stat('PRINTER OFFLINE'),...(kp?{drv:stat('MOTOR WARNINGS'),kl:stat('KLIPPER ERRORS')}:{}),rec:stat('APP RESTARTS'),...(kp?{rt:stat('RESENT DATA · 2 H')}:{})};body.append(statGrid(...Object.values(S)));
 const list=el('div','nx-tl');body.append(section('What happened',list),el('p','nx-note',kp?'From Stella Scope\'s own event log. Resent data is what the printer\'s boards had to send twice in the last two hours. A little after a restart is normal; a steady climb points at a loose USB or CAN wire.':'From Stella Scope\'s own event log.'));
 const now=Date.now()/1000,from=now-7*86400;
 getJson(`/api/events?from=${from}&to=${now}`).then(d=>{const ev=(d.events||[]).slice().sort((a,b)=>a.ts-b.ts),periods=[];
  // Offline: printer_offline events a minute and a half or less apart are one period.
  let off=null;for(const e of ev){if(e.kind!=='printer_offline')continue;if(off&&e.ts-off.end<=90)off.end=e.ts;else{if(off)periods.push(off);off={kind:'offline',ts:e.ts,end:e.ts,message:e.message};}}if(off)periods.push(off);
  const drv=!kp?[]:ev.filter(e=>e.kind==='driver_status'&&/"flags":\s*\[\s*"/.test(e.message||'')).map(e=>({kind:'driver',ts:e.ts,message:e.message}));
  const kl=!kp?[]:ev.filter(e=>e.kind==='response'&&KLIPPER_TROUBLE.test(e.message||'')).map(e=>({kind:'klipper',ts:e.ts,message:e.message}));
  const rec=ev.filter(e=>e.kind==='recorder').map(e=>({kind:'recorder',ts:e.ts,message:e.message}));
  S.off.set(String(periods.length),{note:periods.length?'longest '+fmtDuration(Math.max(...periods.map(p=>p.end-p.ts))+10):'none recorded',tone:periods.length?'warn':'ok'});
  S.drv?.set(String(drv.length),{tone:drv.length?'alarm':'ok',note:drv.length?'warning flags from a motor driver':'no warning flags'});S.kl?.set(String(kl.length),{tone:kl.length?'alarm':'ok'});S.rec.set(String(rec.length),{note:'Stella Scope restarts'});
  const all=[...periods,...drv,...kl,...rec].sort((a,b)=>b.ts-a.ts);list.replaceChildren();let day='';
  for(const e of all.slice(0,300)){const d=new Date(e.ts*1000),ds=d.toLocaleDateString();if(ds!==day){day=ds;list.append(el('div','nx-tl-day',ds));}const r=el('div','nx-tl-row');r.dataset.kind=e.kind;
   r.append(el('span','nx-tl-time',d.toLocaleTimeString()),el('span','nx-tl-kind',e.kind),el('span','nx-tl-msg',e.kind==='offline'?`Printer not answering for ${fmtDuration(e.end-e.ts+10)}.`:e.message));list.append(r);}
  if(!all.length)list.append(el('p','nx-note','Nothing to report in the last seven days of the recording.'));
  if(d.limit&&(d.events||[]).length>=d.limit)list.append(el('p','nx-note',`Only the newest ${d.limit} events are read; older trouble in the week may be missing here.`));})
 .catch(e=>list.append(el('p','nx-note','Events unavailable: '+e.message)));
  if(!kp)return;
 Promise.all(['mcu','mcu EBBCan'].map(o=>getJson(`/api/signal-history?object=${encodeURIComponent(o)}&field=last_stats.bytes_retransmit&window=7200&points=40`).then(d=>{const v=(d.points||[]).map(p=>p.value).filter(x=>finite(x)!==null);
  let rise=0;for(let i=1;i<v.length;i++)if(v[i]>v[i-1])rise+=v[i]-v[i-1];return rise;}).catch(()=>null))).then(([a,b])=>{const t=(a??0)+(b??0);S.rt.set(a===null&&b===null?'—':String(t),{note:`controller ${a??'—'} · toolhead ${b??'—'} bytes`,tone:t>1000?'warn':'ok'});});}});}

// ---------- suspected causes (the classic diagnostics, same rules) ----------
export function pageCauses(){import('./service-core.js').then(({diagnose})=>sheet({title:'Suspected causes',eyebrow:'EVIDENCE FIRST',icon:'i-nav-diagnostics',wide:true,build:body=>{
 let symptom='auto';const list=el('div','nx-causes');
 const pick=segmented(['auto','corner','probe','driver','heater','fan','communication'],symptom,v=>{symptom=v;draw();},v=>({auto:'Only what was recorded',corner:'Corner slips',probe:'BLTouch',driver:'Motor driver hot',heater:'Heating',fan:'Fan stopped',communication:'Connection'}[v]));pick.classList.add('nx-seg--wrap');
 body.append(el('p','nx-note','Warnings the printer recorded, with a safe check for each. The readings cannot see a loose screw or a slack belt, so treat these as places to look, not a diagnosis.'),pick,list);
 function draw(){const found=diagnose(ctx.state.snapshot,symptom);list.replaceChildren();
  if(!found.length){list.append(el('p','nx-note','No matching recorded warning in the last ten minutes. That is not a clean bill of health: pick what you saw above for its checks.'));return;}
  for(const r of found){const c=el('article','nx-cause');c.append(el('span','nx-sheet-eyebrow',r.origin==='operator'?'WHAT YOU SAW · NOT DETECTED':'RECORDED EVIDENCE · CAUSE NOT CONFIRMED'),el('h3','',r.title),el('p','nx-note',r.evidence),el('p','nx-cause-why',r.why));
   for(const [where,what,how,why]of r.checks){const d=el('details','nx-raw');d.append(el('summary','',where+' · '+what),el('p','nx-note',how),el('p','nx-cause-why','Why: '+why));c.append(d);}
   const locs=el('div','nx-chips');for(const id of r.locations){const e=ctx.scene()?.entries?.find(x=>x.slot_id===id);const b=el('button','nx-chip',e?e.name.replace(/:\d+$/,''):id);b.type='button';b.disabled=!e;if(e)b.addEventListener('click',()=>openPart(e.id));locs.append(b);}
   c.append(locs,button({icon:'i-panels',label:'Record it in the Service journal',size:'row',title:'Record it',does:'Opens the journal form with this finding filled in. Nothing is sent to the printer.',onPress:()=>pageJournal({asset:r.locations[0],note:r.title+': '+r.evidence})}));list.append(c);}}
 draw();}}));}

// ---------- service journal (the classic parts book, same browser store and rules) ----------
export function pageJournal(prefill=null){import('./service-core.js').then(({validateBook,emptyBook,journal})=>{
 const key=(ctx.state.snapshot?.recording_origin==='demo'?'stella-demo:':'stella-live:')+'parts:v1:stella-voron-350';
 const load=()=>{const raw=localStorage.getItem(key);return raw?validateBook(JSON.parse(raw)):emptyBook();};
 sheet({title:'Service journal',eyebrow:'WORK DONE ON THIS PRINTER',icon:'i-nav-maintenance',wide:true,build:body=>{let book;try{book=load();}catch(e){body.append(el('p','nx-note','The parts book cannot be read, so nothing is shown or overwritten: '+e.message));return;}
  const form=el('div','nx-journal-form'),asset=el('input','nx-input'),note=el('textarea','nx-input'),evid=el('textarea','nx-input');let st='open';
  asset.placeholder='Where: e.g. front left Z drive';asset.value=prefill?.asset||'';note.placeholder='What, where and why';note.value=prefill?.note||'';evid.placeholder='What you saw afterwards (needed for Inspected and Replaced)';note.rows=3;evid.rows=2;
  for(const [f,l]of [[asset,'Location'],[note,'What, where and why'],[evid,'Observed outcome']])f.setAttribute('aria-label',l);
  const states=segmented(['open','checked','replaced','deferred'],st,v=>{st=v;},v=>({open:'Open',checked:'Inspected',replaced:'Replaced',deferred:'Deferred'}[v]));
  const msg=el('p','nx-note');const add=button({icon:'i-plus',label:'Add to the journal',kind:'primary',size:'row',title:'Add entry',does:'Appends this entry. Nothing is sent to the printer.',onPress:()=>{try{
   if(['checked','replaced'].includes(st)&&!evid.value.trim())throw Error('Write what you saw before marking it inspected or replaced.');
   const cur=load();if(cur.version!==book.version)throw Error('The journal changed in another tab. Close and reopen this page; nothing was overwritten.');
   book=journal(book,asset.value.trim(),st,note.value,evid.value);localStorage.setItem(key,JSON.stringify(book));note.value='';evid.value='';msg.textContent='Added.';draw();}catch(e){msg.textContent=e.message;}}});
  form.append(asset,note,states,evid,add,msg);body.append(section('Record work',form));const list=el('div','nx-tl');body.append(section('Entries',list));
  function draw(){list.replaceChildren();const w=[...book.work].sort((a,b)=>b.at-a.at);if(!w.length){list.append(el('p','nx-note','No work recorded yet. Entries are kept on this computer.'));return;}
   for(const x of w){const r=el('div','nx-tl-row');r.dataset.kind=x.state;const d=new Date(x.at*1000);r.append(el('span','nx-tl-time',d.toLocaleDateString()),el('span','nx-tl-kind',{open:'open',checked:'inspected',replaced:'replaced',deferred:'deferred'}[x.state]),el('span','nx-tl-msg',[x.asset,x.note,x.evidence?'Seen: '+x.evidence:''].filter(Boolean).join('\n')));list.append(r);}}
  draw();}});});}

// ---------- assembly: every model the 3D view is built from ----------
export function pageAssembly(){sheet({title:'Assembly',eyebrow:'THE CAD THE 3D VIEW IS DRAWN FROM',icon:'i-nav-assembly',wide:true,build:body=>{const list=el('div','nx-asm');body.append(list);
 getJson('/api/assets').then(d=>{const as=d.assets||[];if(!as.length){list.append(el('p','nx-note','No models in the library.'));return;}
  for(const a of as){const c=el('article','nx-asm-card'),dl=el('dl','nx-specs'),f=(k,v)=>{if(v!=null&&v!==''){dl.append(el('dt','',k),el('dd',/^[\d.,]+$/.test(String(v))?'nx-led-dd':'',String(v)));}};   // U06: a read number is an LED figure (Meshes)
  
   c.append(el('span','nx-sheet-eyebrow',(a.format||'').toUpperCase()+(a.status?' · '+String(a.status).toUpperCase():'')),el('h3','',a.name||a.id));
   f('Model',a.model);f('Maker',a.manufacturer);f('Revision',a.revision);f('Licence',a.license);f('Meshes',a.mesh_count);f('Imported',a.imported_at?new Date(a.imported_at).toLocaleDateString():null);f('Notes',a.notes);c.append(dl);
   const row=el('div','nx-grid2'),hits=()=>ctx.scene()?.entries?.filter(e=>e.asset?.id===a.id)||[];
   row.append(button({icon:'i-view',label:'Show on the printer',size:'row',title:'Show on the printer',does:'Frames this model in the 3D view, lit and circled, with the rest faded; Show all brings the default view back.',onPress:()=>{const h=hits();if(!h.length){toast('This model is not placed on the printer.');return;}showPart(h[0].id);}}));
   if(a.original_url){const l=el('a','nx-btn');l.dataset.size='row';l.href=a.original_url;l.download='';l.append(icon('i-files'),el('span','nx-lbl','Download source'));row.append(l);}
   c.append(row);list.append(c);}}).catch(e=>list.append(el('p','nx-note','Model library unavailable: '+e.message)));}});}

// ---------- motion and motors (the classic workspace's live half) ----------
// X, Y and Z as three LED windows in one card (U06), the axis letter green as on the bottom bar.
function xyz(label){const w=el('div','nx-stat nx-stat--xyz'),row=el('div','nx-xyz'),v=['X','Y','Z'].map(a=>{const d=el('span','nx-stat-v'),i=el('i','',a);d.append(i,el('b','','—'));row.append(d);return d;});w.append(el('div','nx-stat-l',label),row);
 return {el:w,set(a){a.forEach((t,i)=>{v[i].lastChild.textContent=t;});}};}
// UI board U06 (2026-10-20): a relative wheel is a kit control that always rests at 0: turned, it counts clicks of the step the switch has chosen, and when the hand lets go it is sent ONCE and springs back to 0.
// The jog wheel moves the head (one click is 0.1, 1 or 10 mm, chosen by a three-way switch of square keys; the existing jog command, which asks to be confirmed); the Recording page's shuttle moves through time the same way.
export function relWheel({label,unit,steps,send,lock=()=>'',color='var(--accent)',color2='var(--accent-2)',fmt}){const KS=steps.map(x=>({...x}));let cur=KS[0];
 const dec=v=>(String(v).split('.')[1]||'').length,show=(v,u)=>(v>0?'+':v<0?'\u2212':'')+Math.abs(v).toFixed(dec(cur.step))+' '+u;
 const c=ctl('jog',{key:'jog',label,unit,min:-cur.span,max:cur.span,step:cur.step,turn:cur.turn||300,value:0,color,color2,format:v=>fmt?fmt(v,cur):show(v,unit),
  onSet:v=>{c.spec.value=0;c.paint();if(v)send(v,cur);}});
 const keys=el('div','nx-jw-keys');const ks=KS.map((x,i)=>nkey({label:x.label,toggle:true,pressed:i===0,onPress:()=>pick(i)}));keys.append(...ks.map(k=>k.el));
 function pick(i){cur=KS[i];ks.forEach((k,j)=>k.setPressed(j===i));const p=c.spec;p.min=-cur.span;p.max=cur.span;p.step=cur.step;p.turn=cur.turn||300;p.value=0;c.paint();}
 return {kit:c,keys,pick,lock:why=>{c.setLocked(why);}};}
const JOG_DIR={X:'toward the right',Y:'toward the back',Z:'up, away from the bed'};
export function pageMotion(){sheet({title:'Motion & motors',eyebrow:'WHAT MOVES AND HOW HARD',icon:'i-nav-workspace',wide:true,build:body=>{
 const P={pos:xyz('POSITION'),vel:stat('SPEED NOW'),lim:stat('LIMITS'),hom:stat('HOMED')};   // F04 (2026-10-19): X, Y and Z side by side need a whole row on a phone, not half of one; U06: they are three LED windows now
 body.append(statGrid(...Object.values(P)));   // U06 (2026-10-20): this line sat inside the comment above since F04, so the four readings were never drawn; they are again
 // the jog wheel: pick the axis, pick what one click is, turn, let go
 let axis='X';
 const jw=relWheel({label:'JOG X',unit:'mm',steps:[{label:'0.1 mm',step:.1,span:5,turn:300},{label:'1 mm',step:1,span:25,turn:300},{label:'10 mm',step:10,span:50,turn:300}],
  send:(v)=>ctx.send('jog',{axis,value:Math.round(v*100)/100})});
 const ax=el('div','nx-jw-keys'),aks=['X','Y','Z'].map((a,i)=>nkey({label:a,toggle:true,pressed:i===0,onPress:()=>{axis=a;aks.forEach((k,j)=>k.setPressed(j===i));jw.kit.el.querySelector('.nxc-l').textContent='JOG '+a;jw.kit.spec.label='JOG '+a;jw.kit.paint();hint.textContent=`One click is the step you chose. Clockwise moves ${a} ${JOG_DIR[a]}; the head moves once, when you let go.`;}}));ax.append(...aks.map(k=>k.el));
 const hint=el('p','nx-note','One click is the step you chose. Clockwise moves X toward the right; the head moves once, when you let go.'),deck=el('div','nx-jw'),side=el('div','nx-jw-side');
 tip(jw.kit.el,'Jog wheel','Press and hold, then turn: one click moves the head by the step you chose. Letting go moves it once, after showing you the command.');
 side.append(el('span','nx-jw-l','Axis'),ax,el('span','nx-jw-l','One click is'),jw.keys,hint);deck.append(jw.kit.el,side);body.append(section('Jog wheel',deck));
 const sp=sparks(body,[['motion_report','live_velocity',{label:'Head speed',unit:' mm/s',digits:0}],['motion_report','live_position.2',{label:'Z height',unit:' mm',digits:2}],['motion_report','live_extruder_velocity',{label:'Filament feed',unit:' mm/s',digits:2}]]);
 const grid=el('div','nx-motors');body.append(section('Motor drivers',grid),el('p','nx-note','Status of each motor driver (TMC2209) from the newest reading: current, whether it is stepping or holding, its current scale out of 31 and any warning flags. Tap one for its panel.'));
 const names={stepper_x:'B motor · X',stepper_y:'A motor · Y',stepper_z:'Z front left',stepper_z1:'Z back left',stepper_z2:'Z back right',stepper_z3:'Z front right',extruder:'Extruder'};let key='';
 return ()=>{sp();const r=ctx.state.readings,p=point();P.pos.set(['x','y','z'].map(a=>finite(r.position[a])===null?'—':r.position[a].toFixed(1)));P.vel.set(r.velocity===null?'—':Math.round(r.velocity)+' mm/s');
  P.lim.set(r.maxVelocity===null?'—':`${Math.round(r.maxVelocity)} mm/s`,{note:r.maxAccel===null?'':`${Math.round(r.maxAccel)} mm/s² · Z ${p?.max_z_velocity??'—'} mm/s`});P.hom.set(r.homedAxes?r.homedAxes.toUpperCase():'none',{tone:r.homedAxes==='xyz'?'ok':'warn'});
  const hm=r.homed||{};jw.lock(!ctx.can('jog')?ctx.why('jog'):(hm.x&&hm.y&&hm.z)?'':'Home first: jogging needs X, Y and Z homed.');
  const k=JSON.stringify(p?.motors||null);if(k===key)return;key=k;grid.replaceChildren();
  for(const [id,label]of Object.entries(names)){const m=p?.motors?.[id],c=el('button','nx-motor');c.type='button';c.append(el('b','',label),el('span','nx-motor-v',m?`${m.current?.toFixed(2)??'—'} A`:'—'),el('span','nx-motor-s',m?(m.energised?(m.standstill?'holding':'stepping'):'off'):'not in the sample'),el('small','',m?`scale ${m.scale??'—'}/31${m.flags?.length?' · '+m.flags.join(', '):''}`:''));
   if(m?.flags?.length)c.dataset.alarm='';else if(m?.energised)c.dataset.on='';const e=ctx.scene()?.entries?.find(x=>{const w=wiringFor(x);return w&&w.key===id;});if(e)c.addEventListener('click',()=>openPart(e.id));else c.disabled=true;grid.append(c);}};}});}

// ---------- Z babystep (2026-09-24): nudge the first layer and keep it with the sheet ----------
// Each press sends SET_GCODE_OFFSET Z_ADJUST once. Klipper forgets the offset on a restart, so the
// fitted build sheet can keep it: Save writes the printer's offset into that sheet's record, and
// picking the sheet later offers to apply it again.
export function babystep(body){if(!ctx.has('z_offset'))return ()=>{};   // polish WO-05 (2026-10-18): no Klipper offset on a printer that cannot take one
 const V=stat('Z OFFSET NOW'),row=el('div','nx-baby'),sheetLine=el('div','nx-sheetline'),msg=el('span');
 for(const d of [-0.05,-0.01,0.01,0.05])row.append(button({icon:d>0?'i-arrow-up':'i-arrow-down',label:(d>0?'+':'−')+Math.abs(d).toFixed(2)+(d>0?' up':' down'),size:'row',kind:'default',cmd:'z_offset',title:`${d>0?'Raise':'Lower'} the nozzle ${Math.abs(d)} mm`,does:`Changes the Z offset by ${d.toFixed(2)} mm once. ${d>0?'More gap: a first layer that is too squashed.':'Less gap: a first layer that does not stick.'}`,onPress:async b=>{busy(b,true);try{await ctx.send('z_offset',{value:d});}finally{busy(b,false);}}}));
 const save=button({icon:'i-check',label:'Save to sheet',size:'row',kind:'primary',title:'Save this offset to the sheet on the bed',does:'Stores the printer\'s current Z offset in the fitted build sheet\'s record. Nothing is sent to the printer.',onPress:async b=>{busy(b,true);try{const pr=ctx.state.presets,z=finite(point()?.z_offset),f=pr?.sheet?.name;if(z===null||!f)return;
  await ctx.savePresets({...pr,sheets:(pr.sheets||[]).map(x=>x.name===f?{...x,z_offset:Math.round(z*1000)/1000}:x)});toast(`${f} now prints at ${mm(z)}.`,'ok');}catch(e){toast(e.message,'alarm');}finally{busy(b,false);}}});
 sheetLine.append(msg,save);body.append(section('First layer · Z offset',statGrid(V),el('p','nx-note nx-baby-key','Down brings the nozzle closer to the bed for more squish. Up lifts it for less.'),row,sheetLine));ctx.lockControls(row);
 return ()=>{const z=finite(point()?.z_offset),pr=ctx.state.presets,f=(pr?.sheets||[]).find(x=>x.name===pr?.sheet?.name);V.set(z===null?'—':mm(z),{note:'lower is closer'});
  save.hidden=!f||z===null||Math.abs(f.z_offset-z)<0.0005;msg.textContent=!f?'Say which sheet is on the bed (Gantry & bed) and this offset is kept with that sheet.':save.hidden?`${f.name} keeps ${mm(f.z_offset)}.`:`${f.name} is saved at ${mm(f.z_offset)}.`;};}

// ---------- filament: load and unload (2026-09-24, rebuilt 2026-09-25, again 2026-10-12) ----------
// 2026-09-28, first real use of 0.4.2: "Load filament never reaches the printer", Go "does nothing",
// "a step number blinks", "pressing again marked it loaded instantly". What was wrong, found 2026-10-12:
//  1. The page rebuilt its step list and its buttons on every render tick (several a second), so a press
//     that began on a button was lost when that node was replaced, and each new step marker restarted
//     its glow (the blink). It is built once now; a tick only changes what actually changed.
//  2. Go sent the free-text `macro` command, a T3 that stops at a review dialog; nothing reached Klipper
//     (gcode_store held no LOAD_FILAMENT). It sends the named filament_load / filament_unload now:
//     LOAD_FILAMENT TEMP=<n>, no dialog, refused mid-print by the service.
//  3. Mark loaded did not need the feed to have run. It does now (filamentCanRecord).
// The dock's Load and Unload (filamentQuick) are the same job started with one press: it heats, and the
// feed starts by itself when hot. The job lives outside the sheet; closed mid-way, a glowing marker
// stays on screen and brings it back.
// Filament board 2026-10-22 (the owner, who likes it and uses it): Load and Unload now ask what is being loaded or unloaded first (the picker: the material, the temperature it implies,
// nothing heats until it is confirmed); after an unload the heater stays on for a wait you set, then goes off by itself; after a load it goes off at once and a few seconds later the
// head gives a quick shake; every step says what it is doing and has a way out. The timers run in the open page; the service's heat guard (filGuard) turns the nozzle off if the page goes away.
export const fil={step:'idle',kind:null,mat:null,t:null,moved:0,lastTs:null,error:'',endedAt:0,heatAttempted:false,auto:false,ran:false,cancelled:false,until:0,waitMs:0,shakeAt:0,shake:'',note:'',cooledAt:0,busy:false};
const FIL_IDLE=['idle','done','confirmed','failed'];
const filOpen=()=>!FIL_IDLE.includes(fil.step);
function filReset(){Object.assign(fil,{step:'idle',kind:null,t:null,moved:0,lastTs:null,error:'',endedAt:0,heatAttempted:false,auto:false,ran:false,cancelled:false,until:0,waitMs:0,shakeAt:0,shake:'',note:'',cooledAt:0,busy:false});paintFilMarker();}
const filMacro=k=>k==='load'?'LOAD_FILAMENT':'UNLOAD_FILAMENT',filCmd=k=>k==='load'?'filament_load':'filament_unload';
const filMaterials=()=>ctx.state.presets?.materials||[];
// Filament board 2026-10-22 (the owner): three settings, kept on this device: how long an unloaded printer waits for a reload before the nozzle heater goes off (2 min), how long after a load
// before the head is shaken (4 s), and whether it is shaken at all. The clamping is next-core filamentSettings.
const FIL_SET_KEY='nx-filament';
export function filSettings(){try{return filamentSettings(JSON.parse(localStorage.getItem(FIL_SET_KEY)||'{}'));}catch{return filamentSettings({});}}
function filSave(p){try{localStorage.setItem(FIL_SET_KEY,JSON.stringify({...filSettings(),...p}));}catch{}}
const fmtWait=s=>s>=60?(s%60?Math.floor(s/60)+' min '+(s%60)+' s':s/60+' min'):s+' s';
const fmtClock=s=>Math.floor(s/60)+':'+String(s%60).padStart(2,'0');
async function filStart(kind,mat,t,auto=false){if(filOpen())return false;
 Object.assign(fil,{step:'checking',kind,mat,t,moved:0,lastTs:null,error:'',heatAttempted:false,auto,ran:false,cancelled:false,until:0,waitMs:0,shakeAt:0,shake:'',note:'',cooledAt:0});paintFilMarker();
 const macro=filMacro(kind);
 try{const d=await getJson('/api/macros');if(!d.macros?.some(m=>m.name===macro&&m.macro===true))throw Error('missing');}
 catch{fil.step='failed';fil.error=`${macro} could not be verified on this printer. No heat command was sent.`;paintFilMarker();return false;}
 fil.step='heating';fil.heatAttempted=true;paintFilMarker();
 const hot=ctx.state.readings.nozzle.temp;
 if(hot!==null&&hot>=t-3){filamentTick();return true;}
 if(!await ctx.send('set_heater',{target:'extruder',value:t})){fil.step='failed';fil.error='The heat request was not accepted. Check the nozzle target; Cool is available.';paintFilMarker();return false;}
 return true;}
async function filRun(){if(!['ready','heating'].includes(fil.step))return;const kind=fil.kind;
 fil.step='running';fil.moved=0;fil.lastTs=null;paintFilMarker();
 const r=await ctx.send(filCmd(kind),{value:Math.round(fil.t)});
 if(fil.step!=='running')return;   // closed or taken over while the macro ran
 if(!r){fil.step='failed';fil.error=`${filMacro(kind)} was not sent. The nozzle may still be hot; use Cool.`;paintFilMarker();return;}
 fil.ran=true;fil.endedAt=Date.now();
 if(fil.cancelled){await filCool('You stopped it. The printer finished its move first, then the heater went off.');return;}
 if(kind==='unload'){const S=filSettings();fil.waitMs=S.wait*1000;fil.until=Date.now()+fil.waitMs;fil.step='waiting';toast(`${filMacro(kind)} finished. Waiting ${fmtWait(S.wait)} for a reload, then the heater goes off.`,'ok');}
 else await filCool();
 paintFilMarker();}
// The nozzle heater off: at once after a load, after the wait (or when you say so) after an unload. After a load the shake is booked for a few seconds later (if it is on).
async function filCool(note=''){if(fil.busy)return;fil.busy=true;
 try{if(!await ctx.send('set_heater',{target:'extruder',value:0})){fil.step='failed';fil.error='The heater would not turn off. Use Cool down, or STOP.';return;}
  const S=filSettings();fil.step='cooling';fil.note=note;fil.until=0;fil.cooledAt=Date.now();
  if(fil.kind==='load'&&S.shake&&!fil.cancelled){fil.shake='pending';fil.shakeAt=Date.now()+S.pause*1000;}
  else{fil.shake='skipped';if(fil.kind==='load'){fil.step='done';fil.endedAt=Date.now();}}}   // a load with no shake to wait for is finished: the heater is off
 finally{fil.busy=false;paintFilMarker();}}
// The shake: small fast moves along X, in place (the service's filament_shake). Never during a print, only when homed, and not if the setting is off.
async function filShake(){if(fil.busy||fil.step!=='cooling'||fil.shake!=='pending')return;
 const S=filSettings(),v=filamentShakeVerdict({enabled:S.shake,homed:ctx.state.readings.homed,mode:ctx.state.mode,allowed:ctx.can('filament_shake')});fil.shakeAt=0;
 if(!v.ok){fil.shake='skipped';fil.note=v.why;fil.step='done';fil.endedAt=Date.now();paintFilMarker();return;}
 fil.busy=true;fil.step='shaking';fil.shake='running';paintFilMarker();
 try{const r=await ctx.send('filament_shake');fil.shake=r?'done':'failed';if(!r)fil.note='The shake was not sent. The filament is loaded all the same.';}
 finally{fil.busy=false;if(fil.step==='shaking'){fil.step='done';fil.endedAt=Date.now();}paintFilMarker();}}
// A way out at every step. What it does depends on the step, and the key says so: heating and ready: cancel and cool; feeding or pulling: stop after this move (a running macro cannot be
// interrupted safely, so the printer finishes it and the nozzle cools after); waiting: cool now; cooling: skip the shake or close; shaking: close.
const FIL_CANCEL={checking:'Cancel',heating:'Cancel and cool',ready:'Cancel and cool',running:'Stop after this move',waiting:'Cool now',cooling:'Close',shaking:'Close'};
async function filCancel(){const st=fil.step;
 if(['checking','heating','ready'].includes(st)){if(await ctx.send('set_heater',{target:'extruder',value:0}))filReset();else{fil.step='failed';fil.error='Cooling was not accepted. Check the nozzle target; Cool is available.';paintFilMarker();}return;}
 if(st==='running'){fil.cancelled=true;toast('Stopping after this move. The printer cannot interrupt its own macro; the heater goes off when it returns.','ok');paintFilMarker();return;}
 if(st==='waiting'){await filCool('You cooled it yourself, before the wait ran out.');return;}
 if(st==='cooling'&&fil.kind==='load'&&fil.shake==='pending'){fil.shakeAt=0;fil.shake='skipped';fil.note='You skipped the shake.';fil.step='done';fil.endedAt=Date.now();paintFilMarker();return;}
 filReset();}
/** Called every render: the heat reaching its target (an unload, or a load started from the dock, goes by itself), the travelled length, the wait for a reload, the shake, the cool-down. */
export function filamentTick(){if(fil.step==='idle')return;const r=ctx.state.readings,t=r.nozzle.temp;
 if(fil.step==='heating'&&t!==null&&fil.t&&t>=fil.t-3){if(fil.kind==='unload'||fil.auto)filRun();else{fil.step='ready';}}
 if(fil.step==='running'){const ts=ctx.tick()?.point?.ts,v=finite(r.extruderVelocity);if(ts&&fil.lastTs&&v!==null)fil.moved+=Math.abs(v)*Math.min(1,ts-fil.lastTs);if(ts)fil.lastTs=ts;}
 if(fil.step==='waiting'&&fil.until&&Date.now()>=fil.until)filCool(`No reload in ${fmtWait(Math.round(fil.waitMs/1000))}: the heater went off by itself.`);
 if(fil.step==='cooling'&&fil.shake==='pending'&&fil.shakeAt&&Date.now()>=fil.shakeAt)filShake();
 if(fil.step==='cooling'&&fil.kind==='unload'&&fil.cooledAt&&((t!==null&&t<60)||Date.now()-fil.cooledAt>600000)){fil.step='done';fil.endedAt=Date.now();}
 paintFilMarker();}
/** Before anything heats, Load and Unload ask what is being loaded or unloaded (2026-10-22): the material, with the nozzle temperature it implies; an unload starts on what is
 *  in the extruder. Nothing is sent until its key is pressed. \`auto\`: the dock's way (it feeds by itself once hot); the page's load asks you to push the filament in first. */
export function filPicker(kind,{auto=false}={}){
 if(filOpen()&&fil.step!=='waiting'){toast('A filament job is already open. Showing it.','ok');pageFilament();return;}
 const mats=filMaterials(),inName=ctx.state.presets?.loaded?.name,load=kind==='load',OTHER='\u0000other';
 // Opus polish P2: a printer with no saved materials (a fresh install) had a dead end here. The last card is always OTHER: type the nozzle and bed temperatures, and give it
 // a name to keep it with your materials (the same list as the Heat page).
 let pick=load?(mats.some(m=>m.name===fil.mat)?fil.mat:null):(mats.some(m=>m.name===inName)?inName:null);if(!mats.length)pick=OTHER;
 openModal({title:load?'Load filament':'Unload filament',icon:load?'i-filament-load':'i-filament-unload',build:(body,close)=>{u06(body);body.classList.add('nx-filpick');
  const ask=el('p','nx-note',load?'What are you loading?':'What is in the extruder?'),cards=el('div','nx-matcards'),heat=el('div','nx-filpick-heat'),say=el('p','nx-note'),keys=el('div','nx-grid2');
  const other=el('div','nx-filpick-other');const oNoz=numberInput({label:'Nozzle',unit:'°',min:170,max:290,step:5,value:240,onSet:()=>draw()}),oBed=numberInput({label:'Bed',unit:'°',min:0,max:120,step:5,value:60});
  const oName=el('input','nx-filpick-name');oName.type='text';oName.maxLength=24;oName.placeholder='Name it to keep it (optional)';oName.setAttribute('aria-label','Material name, optional');oName.addEventListener('input',()=>draw());
  const oNameW=el('label','nx-filpick-namew');oNameW.append(el('small','nx-numi-l','Name'),oName);other.append(oNoz,oBed,oNameW);
  const typedName=()=>oName.value.trim();
  const m0=()=>pick===OTHER?{name:typedName()||`${oNoz.get()}° filament`,nozzle:oNoz.get(),bed:oBed.get(),other:true}:mats.find(m=>m.name===pick)||null;
  const nameClash=()=>pick===OTHER&&typedName()&&(mats.some(m=>m.name.toLowerCase()===typedName().toLowerCase())||typedName().toLowerCase()==='cool down');
  const go=button({icon:load?'i-filament-load':'i-filament-unload',label:'Pick a material',kind:'primary',size:'row',title:load?'Heat and load':'Heat and unload',does:'Heats the nozzle to the temperature shown, then '+(load?(auto?'feeds by itself when hot':'asks you to push the filament in'):'unloads by itself when hot')+'. Nothing heats before this key. A named OTHER is kept with your materials.',onPress:async b=>{const m=m0();if(!m||nameClash())return;
   if(m.other&&typedName()){if(mats.length>=24){toast('You have 24 materials, the most there can be. It heats all the same; it is not kept.','alarm');}
    else{busy(b,true);try{await ctx.savePresets({...ctx.state.presets,materials:[...mats,{name:m.name,nozzle:m.nozzle,bed:m.bed}]});}catch(e){toast('Not kept: '+e.message+'. It heats all the same.','alarm');}finally{busy(b,false);}}}
   close();if(fil.step==='waiting')filReset();filStart(kind,m.name,m.nozzle,auto).then(ok=>{if(ok&&auto)toast(load?`Heating for ${m.name}. Push the filament in now; it feeds by itself when hot.`:`Heating for ${m.name}. It unloads by itself when hot.`,'ok');});}});
  const card=(name,top,bottom,id,extra)=>{const c=el('button','nx-matcard');c.type='button';c.setAttribute('aria-pressed',String(id===pick));c.append(el('b','',name),el('span','nx-matcard-t',top),el('span','nx-matcard-b',bottom));if(extra)c.append(extra);
   c.addEventListener('click',()=>{pick=id;draw();});return c;};
  function draw(){cards.replaceChildren(...mats.map(m=>card(m.name,`${m.nozzle}° nozzle`,`${m.bed}° bed`,m.name,inName===m.name?el('i','nx-matcard-in','IN THE EXTRUDER'):null)),card('Other','your own','temperatures',OTHER));
   cards.lastChild.classList.add('nx-matcard-other');other.hidden=pick!==OTHER;
   const m=m0();heat.replaceChildren(el('small','','NOZZLE WILL HEAT TO'),m?led(m.nozzle,'°'):el('b','nx-filpick-none','pick one'));
   go.querySelector('.nx-lbl').textContent=nameClash()?'That name is taken':m?`Heat to ${m.nozzle}° and ${load?'load':'unload'}`:'Pick a material';go.disabled=!m||nameClash();
   say.textContent=nameClash()?`${typedName()} is already one of your materials: pick its card, or give this another name.`:m?`Nothing heats until you press the key below. ${m.other?(typedName()?`${typedName()} is kept with your materials (nozzle ${m.nozzle}°, bed ${m.bed}°).`:'Name it to keep it for next time.'):load?'':'It starts on what is recorded as in the extruder; pick another if that is not what is in there.'}`:'Nothing heats until you pick a material and press the key.';}
  const cancel=button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'Closes this. Nothing was heated.',onPress:()=>close()});
  keys.append(cancel,go);draw();body.append(ask,cards,other,heat,say,keys);ctx.lockControls(body);}});}
/** 2026-10-24 (radial menus everywhere): LOAD and UNLOAD open a wheel of the materials. A pick heats to that material and runs by itself when hot,
 * exactly what the picker's key does; OTHER opens the picker for your own temperatures. What is in the extruder comes first for an unload. */
export function filamentWheel(kind){const load=kind==='load',inName=ctx.state.presets?.loaded?.name,mats=filMaterials().slice().sort((a,b)=>load?0:(b.name===inName)-(a.name===inName));
 const go=async m=>{if(filOpen()&&fil.step!=='waiting'){toast('A filament job is already open. Showing it.','ok');pageFilament();return;}if(fil.step==='waiting')filReset();
  if(await filStart(kind,m.name,m.nozzle,true))toast(load?`Heating for ${m.name}. Push the filament in now; it feeds by itself when hot.`:`Heating for ${m.name}. It unloads by itself when hot.`,'ok');};
 return [...mats.map(m=>({key:'fil-'+m.name,icon:load?'i-filament-load':'i-filament-unload',label:m.name.toUpperCase(),sub:`${m.nozzle}°`,tone:'heat',cmd:load?'filament_load':'filament_unload',
   title:(load?'Load ':'Unload ')+m.name+(!load&&m.name===inName?' (in the extruder)':''),does:`Heats the nozzle to ${m.nozzle}°, then ${load?'feeds by itself once hot: push the filament in':'pulls the filament out by itself once hot'}.`,run:()=>go(m)})),
  {key:'fil-other',icon:'i-plus',label:'OTHER',title:'Other material',does:'Your own temperatures, and a name to keep it with your materials.',run:()=>filPicker(kind,{auto:true})},
  {key:'fil-page',icon:'i-panels',label:'FILAMENT PAGE',title:'Filament page',does:'Every step of a load or unload, and what is in the extruder.',run:()=>pageFilament()}];}
/** STOP on the deck: ends a filament job the way its own Cancel does. False when none is running. */
export async function filamentStop(){if(fil.step==='idle')return false;await filCancel();return true;}
export function filamentBusy(){return fil.step!=='idle'&&!(fil.step==='confirmed'&&Date.now()-fil.endedAt>120000);}
/** The dock's Load and Unload: one press asks what is being loaded or unloaded, then the job starts with that material's nozzle temperature. */
export async function filamentQuick(kind){filPicker(kind,{auto:true});}
function filWords(){const r=ctx.state.readings,t=r.nozzle.temp,load=fil.kind==='load',pct=Math.round(filamentProgress(fil,t)*100),mm0=FILAMENT[load?'load':'unload'].mm,tt=t===null?'—':Math.round(t);
 switch(fil.step){case 'checking':return ['Checking printer macros…','No heat command has been sent.'];
  case 'heating':return [`Heating for ${load?'loading':'unloading'} ${fil.mat}`,`Nozzle ${tt}° of ${fil.t}°, ${pct}%. ${!load?'The unload starts by itself when hot.':fil.auto?`Push the ${fil.mat} in now; it feeds by itself when hot.`:'It asks you when hot.'}`];
  case 'ready':return [`Hot. Push the ${fil.mat} in.`,'Cut the end square, push it into the extruder until the gears grab it, then press Go.'];
  case 'running':return [load?'Feeding':'Pulling out',`${Math.round(fil.moved)} of about ${mm0} mm moved. The printer runs ${filMacro(fil.kind)} at ${Math.round(fil.t)}°.${fil.cancelled?' Stopping after this move: the heater goes off when it returns.':''}`];
  case 'waiting':{const left=Math.max(0,Math.ceil((fil.until-Date.now())/1000));return ['Waiting for a reload',`${fmtClock(left)} left. Load now to keep the nozzle hot, or the heater goes off by itself at 0:00. Nozzle ${tt}°.`];}
  case 'cooling':return [fil.kind==='load'?'Heater off, cooling':'Heater off',`${fil.note?fil.note+' ':''}Nozzle ${tt}° and falling.${fil.shake==='pending'&&fil.shakeAt?` A quick shake of the head follows in ${Math.max(0,Math.ceil((fil.shakeAt-Date.now())/1000))} s, to flick the filament tail off.`:''}`];
  case 'shaking':return ['Shaking the head','Small fast moves along X, in place, to flick the filament tail off the nozzle. About a second.'];
  case 'done':return [`${filMacro(fil.kind)} finished`,`${load?`Check that ${fil.mat} has fed and purged, then mark it loaded.`:'Check that the filament has retracted and remove it, then mark it unloaded.'} The nozzle heater is off${fil.shake==='done'?' and the head was shaken':''}.${fil.note&&fil.shake!=='done'?' '+fil.note:''}`];
  case 'confirmed':return [load?`${fil.mat} recorded as loaded`:'Filament recorded as unloaded','You confirmed the physical filament state. Cool the nozzle if you are finished.'];
  case 'failed':return ['Action not completed',fil.error||'The printer refused it.'];}return ['',''];}
// The service's heat guard (Opus review 2026-10-22): while this page runs a hot step it keeps telling the service when to turn the nozzle off if the page is gone: 2 min
// from the last word while heating or ready, 5 min while the printer runs its macro, the reload wait's own end (+5 s) while waiting. Renewed every 20 s; taken away
// (0) when the job leaves its hot steps. A page that never armed it never clears it, so a second screen cannot cancel the first one's guard.
const filGuardSent={step:'',n:0,at:0};
function filGuard(){const st=fil.step,now=Date.now();
 const n=['checking','heating','ready'].includes(st)?120:st==='running'?300:st==='waiting'&&fil.until?Math.max(1,Math.ceil((fil.until-now)/1000)+5):0;
 if(n===0&&filGuardSent.n===0)return;
 if(st===filGuardSent.step&&(n===0||now-filGuardSent.at<20000))return;
 Object.assign(filGuardSent,{step:st,n,at:now});
 ctx.jsend('/api/filament/guard','POST',{off_in_s:Math.min(900,n)}).catch(()=>{});}
// The glowing marker while a filament job is open and its page is not.
function paintFilMarker(){filGuard();let m=document.getElementById('nx-filmark');const open=document.querySelector('#nx-sheet[data-open] .nx-fil');
 const show=fil.step!=='idle'&&!open&&!(fil.step==='confirmed'&&Date.now()-fil.endedAt>120000);
 if(!m){if(!show)return;m=el('button','nx-filmark nx-glass');m.id='nx-filmark';m.type='button';m.append(icon('i-filament-unload'),el('b'),el('span'),el('i','nx-filmark-bar'));m.addEventListener('click',()=>pageFilament());(document.getElementById('nx-lane')||document.getElementById('nx-ui')).append(m);}
 m.hidden=!show;if(!show)return;const [a,b]=filWords();m.dataset.step=fil.step;m.querySelector('b').textContent=a;m.querySelector('span').textContent=fil.step==='ready'?'Tap to continue':fil.step==='done'||fil.step==='failed'?'Tap for details':b.split('. ')[0];
 m.style.setProperty('--p',filamentProgress(fil,ctx.state.readings.nozzle.temp).toFixed(3));}
export function pageFilament(){sheet({title:'Filament',eyebrow:'LOAD AND UNLOAD',icon:'i-filament-load',wide:true,build:body=>{body.classList.add('nx-fil');
 const loaded=stat('IN THE EXTRUDER'),sens=stat('RUNOUT SENSOR'),noz=stat('NOZZLE');body.append(statGrid(loaded,sens,noz));
 // the job: a step list, a ring, the words, the keys for this step. Built once; paint() flips states.
 const job=el('div','nx-filjob'),steps=el('ol','nx-filsteps'),ring=el('div','nx-filring nx-round'),words=el('div','nx-filwords'),title=el('b'),line=el('p'),acts=el('div','nx-filacts'),main=el('div','nx-filmain'),ringText=el('span');
 ring.append(ringText);words.append(title,line);main.append(ring,words);job.append(steps,main,acts);
 const B={load:button({icon:'i-filament-load',label:'Load',kind:'primary',size:'row',cmd:'filament_load',title:'Load filament',does:'Asks what you are loading, shows the nozzle temperature it implies, and heats only when you confirm. Then it asks you to push the filament in and press Go.',onPress:()=>filPicker('load')}),
  unload:button({icon:'i-filament-unload',label:'Unload',kind:'primary',size:'row',cmd:'filament_unload',title:'Unload filament',does:'Asks what is in the extruder, shows the nozzle temperature it implies, and heats only when you confirm. Then it unloads by itself when hot.',onPress:()=>filPicker('unload')}),
  go:button({icon:'i-play',label:'Go: feed it',kind:'primary',size:'row',cmd:'filament_load',title:'Go',does:'Feeds the filament at the material\'s temperature, once.',onPress:()=>filRun()}),
  cancel:button({icon:'i-close',label:'Cancel',size:'row',title:'Cancel',does:'A way out of this step. What it does is written on the key: cancel and cool, stop after this move, cool now, or close.',onPress:()=>filCancel()}),
  loadnow:button({icon:'i-filament-load',label:'Load now',kind:'primary',size:'row',cmd:'filament_load',title:'Load now',does:'Keeps the nozzle hot and asks what you are loading. The wait for a reload stops.',onPress:()=>filPicker('load')}),
  cool:button({icon:'i-close',label:'Cool down',size:'row',cmd:'set_heater',title:'Cool down',does:'Turns the nozzle heater off.',onPress:async()=>{if(await ctx.send('set_heater',{target:'extruder',value:0})){fil.heatAttempted=false;paint();}else{fil.step='failed';fil.error='Cooling was not accepted. Check the nozzle target.';paintFilMarker();}}}),
  record:button({icon:'i-check',label:'Mark loaded',kind:'primary',size:'row',title:'Confirm',does:'After you see the filament feed and purge, record this material as loaded. Needs the feed to have run.',onPress:async b=>{if(!filamentCanRecord(fil)){toast('The feed has not run yet.','alarm');return;}
   try{await ctx.savePresets({...ctx.state.presets,loaded:fil.kind==='load'?{name:fil.mat,at:Date.now()/1000}:null});if(fil.kind==='load')ctx.applyMaterial?.(fil.mat);fil.step='confirmed';fil.endedAt=Date.now();paint();paintFilMarker();}catch(e){toast(e.message,'alarm');}finally{busy(b,false);}}}),
  again:button({icon:'i-back',label:'Dismiss',size:'row',title:'Dismiss',does:'Closes this step without recording a filament change. Cool separately if needed.',onPress:()=>filReset()})};
 body.append(section('Load or unload',job));
 // the three settings (kept on this device)
 {const S=filSettings(),wait=ctl('fwait',{label:'WAIT FOR RELOAD',unit:'s',min:10,max:600,step:10,turn:300,value:S.wait,color:'#FF6A00',color2:'#FFB067',format:fmtWait,onSet:v=>filSave({wait:v})}),
   pause=ctl('fpause',{label:'SHAKE AFTER',unit:'s',min:1,max:15,step:1,turn:300,value:S.pause,color:'var(--accent)',color2:'var(--accent-2)',onSet:v=>filSave({pause:v})});
  const shake=nkey({label:S.shake?'SHAKE ON':'SHAKE OFF',icon:'filament-unload',toggle:true,pressed:S.shake,onPress:on=>{filSave({shake:on});shake.el.querySelector('.nx-lbl').textContent=on?'SHAKE ON':'SHAKE OFF';}});
  tip(shake.el,'Shake after a load','After a load the nozzle heater goes off at once, then, a few seconds later, the head gives a quick shake along X to flick the filament tail off. Only when the printer is homed and not printing.');
  const field=(c,label)=>{const w=el('div','nx-numf');w.style.setProperty('--nsz','104px');w.append(c.el,el('small','nx-numf-l',label));return w;};   // the word under the ring, as every number set in a panel
  const row=el('div','nx-fil-set');row.append(field(wait,'Wait for a reload'),field(pause,'Shake after'),shake.el);
  body.append(section('After a load or unload',row,el('p','nx-note','After an unload the heater stays on for the wait you set, so a reload is quick, then goes off by itself. After a load it goes off at once, and the head is shaken a few seconds later if the shake is on. Kept on this device.')));}
 const man=el('div','nx-grid4');for(const v of [-25,-5,5,25])man.append(cmdButton((v>0?'Push ':'Pull ')+Math.abs(v)+' mm',v>0?'i-arrow-down':'i-arrow-up','extrude',`${v>0?'Pushes':'Pulls'} ${Math.abs(v)} mm of filament. Klipper refuses below its minimum temperature.`,{value:v}));
 body.append(section('Move it by hand',man));
 let stepsKey='',actsKey='';
 function paint(){const kind=fil.kind||'unload',st=fil.step;
  // the markers: the list is rebuilt only when the kind changes; after that only data-state moves
  const spec=filamentSteps(kind,st),sk=spec.map(s=>s.key).join();
  if(sk!==stepsKey){stepsKey=sk;steps.replaceChildren(...spec.map(s=>{const li=el('li','',s.label);li.dataset.k=s.key;return li;}));steps.style.setProperty('--n',String(spec.length));}
  spec.forEach((s,i)=>{const li=steps.children[i];if(li&&(li.dataset.state||'')!==s.state){if(s.state)li.dataset.state=s.state;else delete li.dataset.state;}});
  const pct=filamentProgress(fil,ctx.state.readings.nozzle.temp);ring.style.setProperty('--p',pct.toFixed(3));
  const rt=st==='idle'||st==='checking'?'':st==='cooling'||st==='shaking'?(ctx.state.readings.nozzle.temp===null?'':Math.round(ctx.state.readings.nozzle.temp)+'°'):st==='done'?'?':st==='confirmed'?'✓':st==='failed'?'!':st==='running'?Math.round(fil.moved)+' mm':st==='waiting'?fmtClock(Math.max(0,Math.ceil((fil.until-Date.now())/1000))):Math.round(pct*100)+'%';
  if(ringText.textContent!==rt)ringText.textContent=rt;if(ring.dataset.step!==st){ring.dataset.step=st;job.dataset.step=st;}
  const [a,b]=st==='idle'?['Ready to load or unload.','You pick the material first. Nothing heats until you confirm. An unload runs by itself once hot; a load asks you to push the filament in.']:filWords();
  if(title.textContent!==a)title.textContent=a;if(line.textContent!==b)line.textContent=b;
  // the keys: replaced only when the set for this step changes, so a press is never lost to a repaint
  const coolable=fil.heatAttempted||ctx.state.readings.nozzle.target>0;
  B.record.querySelector('.nx-lbl')&&(B.record.querySelector('.nx-lbl').textContent=fil.kind==='load'?'Mark loaded':'Mark unloaded');
  const cw=B.cancel.querySelector('.nx-lbl'),want_cancel=FIL_CANCEL[st]==='Close'&&st==='cooling'&&fil.kind==='load'&&fil.shake==='pending'?'Skip the shake':FIL_CANCEL[st];if(cw&&want_cancel&&cw.textContent!==want_cancel)cw.textContent=want_cancel;
  const want=[];if(st==='idle'||st==='failed')want.push('load','unload');if(st==='ready')want.push('go');if(FIL_CANCEL[st])want.push('cancel');if(st==='waiting')want.push('loadnow');
  if(['waiting','cooling','shaking','done'].includes(st)&&filamentCanRecord(fil))want.push('record');if(['done','confirmed','failed'].includes(st)){if(coolable)want.push('cool');want.push('again');}
  const key=want.join();if(key!==actsKey){actsKey=key;acts.replaceChildren(...want.map(k=>B[k]));ctx.lockControls(acts);}
  B.record.disabled=!filamentCanRecord(fil);}
 return ()=>{const r=ctx.state.readings,l=ctx.state.presets?.loaded,ss=ctx.tick()?.filament_sensors;
  loaded.set(l?.name||'Not set yet',{note:l?.at?'since '+new Date(l.at*1000).toLocaleString():'set when you load'});
  if(!Array.isArray(ss))sens.set('—',{note:'no reading'});else if(!ss.length)sens.set('None fitted',{note:'Klipper has no runout sensor'});
  else{const s=ss[0];sens.set(s.enabled===false?'Off':s.filament_detected===true?'Filament in':s.filament_detected===false?'No filament':'Unknown',{note:`${s.name} · a sensor that is not plugged in reads as no filament`,tone:s.filament_detected===false&&s.enabled!==false?'warn':''});}
  noz.set(r.nozzle.temp===null?'—':Math.round(r.nozzle.temp)+'°',{note:r.nozzle.target>0?'target '+Math.round(r.nozzle.target)+'°':'heater off',tone:r.nozzle.temp>60&&fil.step==='done'?'warn':''});
  paint();};},onClose:()=>setTimeout(paintFilMarker,300)});}

// ---------- print history and stats ----------
// Moonraker's own job history (every print since it was installed) when the printer answers; the
// recorder's print starts when it does not.
export function pageHistory(){sheet({title:'Print history',eyebrow:'EVERY PRINT AND HOW IT WENT',icon:'i-rewind',wide:true,build:body=>{
 const S={n:stat('PRINTS'),ok:stat('FINISHED'),hrs:stat('PRINT TIME'),fil:stat('FILAMENT'),long:stat('LONGEST'),avg:stat('AVERAGE')};body.append(statGrid(...Object.values(S)));
 const chart=el('canvas','nx-hist-cv'),note=el('p','nx-note'),list=el('div','nx-tl');body.append(section('Last 30 days',chart,note),section('Prints',list));
 getJson('/api/print/history?limit=200').then(d=>{const all=d.jobs||[],t=d.totals||{};
  // finish-2 F02: a dry run prints nothing: it is listed, not counted (the CORE One L's cancelled dry run made the rate 50 %)
  const dry=j=>/\(dry run\)/i.test(j.filename||''),jobs=all.filter(j=>!dry(j));
  // 2026-10-25 audit: an empty history was a wall of dashed boxes and an empty chart. With no print yet it says so in one line.
  if(!all.length){body.replaceChildren(el('p','nx-note nx-empty-line','No prints recorded yet. Every print from now on is kept here: how long it took, how it ended, and the filament.'));return;}
  const done=jobs.filter(j=>j.status==='completed'),hrs=(t.total_print_time??jobs.reduce((a,j)=>a+(j.print_duration||0),0))/3600,fil=(t.total_filament_used??jobs.reduce((a,j)=>a+(j.filament_used||0),0))/1000;
  S.n.set(String(t.total_jobs??jobs.length),{note:/^Moonraker/.test(d.source||'')?'from the printer\'s own history':/start/.test(d.source||'')?'recorded here; the printer\'s history did not answer':'recorded by Stella Scope'});const ended=jobs.filter(j=>!['recorded start','in_progress'].includes(j.status));S.ok.set(ended.length?Math.round(done.length/ended.length*100)+' %':'—',{note:ended.length?`${done.length} of the last ${ended.length}`:'no print with a known end yet',tone:ended.length&&done.length/ended.length<.8?'warn':ended.length?'ok':''});
  // finish-2 F02: a printer that never says how much filament it used (PrusaLink) is not "0.0 m": nothing was reported
  const filKnown=t.total_filament_used!=null||jobs.some(j=>finite(j.filament_used)!==null);
  const hrsKnown=t.total_print_time!=null||jobs.some(j=>finite(j.print_duration)!==null);S.hrs.set(hrsKnown?hrs.toFixed(1)+' h':'—',{note:hrsKnown?'':'no print with a known length yet'});if(filKnown)S.fil.set(fil.toFixed(1)+' m',{note:`about ${(fil*2.98).toFixed(0)} g of 1.75 mm`});else S.fil.set('—',{note:'not reported by this printer'});
  const longest=t.longest_print??Math.max(0,...jobs.map(j=>j.print_duration||0));S.long.set(longest>0?fmtDuration(longest):'—');S.avg.set(done.length?fmtDuration(done.reduce((a,j)=>a+(j.print_duration||0),0)/done.length):'—',{note:'finished prints'});
  drawDays(chart,jobs);note.textContent='Hours printed each day: teal finished, orange cancelled or failed.';
  list.replaceChildren();let day='';for(const j of all){const dt=new Date((j.start_time||0)*1000),ds=dt.toLocaleDateString();if(ds!==day){day=ds;list.append(el('div','nx-tl-day',ds));}
   const r=el('button','nx-hist-row');r.type='button';r.dataset.status=j.status;const img=el('img','nx-hist-thumb');img.alt='';img.loading='lazy';img.addEventListener('error',()=>{img.style.visibility='hidden';});if(ctx.has('thumbnails'))img.src='/api/print/thumbnail?filename='+encodeURIComponent(j.filename);else img.style.visibility='hidden';
   const tx=el('span','nx-hist-t');tx.append(el('b','',fileTitle(j.filename)),el('small','',[dt.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'}),fmtDuration(j.print_duration||0),j.filament_used?(j.filament_used/1000).toFixed(2)+' m':'',j.metadata?.filament_type].filter(Boolean).join(' · ')));
   r.append(img,tx,el('span','nx-hist-s',{completed:'finished',cancelled:'cancelled',error:'failed',klippy_shutdown:'Klipper stopped',klippy_disconnect:'disconnected',in_progress:'printing',interrupted:'interrupted','recorded start':'no end recorded'}[j.status]||j.status));
   // finish-2 F02: a print in the history is watched again where the recording has it (the row used to open the file)
   // 2026-10-26: a print opens its report (figures, grams and cost, the timelapse, what the printer said); watching it again is in the report
   tip(r,'Print report','How it went, the filament in grams and money, its timelapse and what the printer said while it ran.');
   r.addEventListener('click',()=>printReport(j));list.append(r);}
  if(!all.length)list.append(el('p','nx-note','No prints recorded yet.'));})
 .catch(e=>{S.n.set('—',{note:e.message});list.append(el('p','nx-note','History unavailable: '+e.message));});}});}
function drawDays(cv,jobs){const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||700,H=cv.clientHeight||150;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);
 const days=30,now=new Date();now.setHours(0,0,0,0);const t0=now.getTime()/1000-(days-1)*86400,ok=new Array(days).fill(0),bad=new Array(days).fill(0);
 for(const j of jobs){const i=Math.floor(((j.start_time||0)-t0)/86400);if(i<0||i>=days)continue;(j.status==='completed'?ok:bad)[i]+=(j.print_duration||0)/3600;}
 const mx=Math.max(1,...ok.map((v,i)=>v+bad[i])),bw=W/days;g.clearRect(0,0,W,H);
 for(let i=0;i<days;i++){const x=i*bw+2,h1=ok[i]/mx*(H-22),h2=bad[i]/mx*(H-22);g.fillStyle=ink('--accent','#04D9C4');g.fillRect(x,H-16-h1,bw-4,h1);g.fillStyle='#FF9C55';g.fillRect(x,H-16-h1-h2,bw-4,h2);}
 g.fillStyle='#8B95A1';g.font='500 10px "Geist Mono",monospace';g.textAlign='left';g.fillText(new Date(t0*1000).toLocaleDateString([], {month:'short',day:'numeric'}),2,H-2);g.textAlign='right';g.fillText('today',W-2,H-2);g.fillText(mx.toFixed(1)+' h',W-2,10);}

// ---------- maintenance reminders, by print hours ----------
// Hours come from Moonraker's job totals (every print since it was installed), kept on this device
// for when the printer is off. Each task remembers the hour count and date it was last done.
export const VORON_SCHEDULE=[['Clean the nozzle and check for plastic build-up',50],['Clean the build sheet (IPA or dish soap)',20],['Check A and B belt tension (Input shaper: measure belts)',100],
 ['Lube the linear rails',200],['Clean the extruder gears',250],['Check the Z belts and Z pulleys',300],['Check frame and gantry bolts',300],['Check the umbilical and cable chains for wear',300],['Replace the nozzle',500]];
export async function printHours(){try{const d=await getJson('/api/print/history?limit=1'),s=d.totals?.total_print_time;if(finite(s)!==null){const h=s/3600;try{localStorage.setItem('nx-print-hours',JSON.stringify([h,Date.now()/1000]));}catch{}return {h,live:true};}}catch{}
 try{const [h,at]=JSON.parse(localStorage.getItem('nx-print-hours')||'null')||[];if(finite(h)!==null)return {h,live:false,at};}catch{}return {h:null,live:false};}
// 2026-10-25: "I'm getting maintenance notifications right off the beginning." A job never marked done (a standard job just added) is never due:
// it starts counting from the hours when it was added (adoptJobs), not from the printer's first hour.
export function dueTasks(hours){return (ctx.state.presets?.maintenance||[]).filter(t=>hours!==null&&t.last_h!=null&&hours-t.last_h>=t.every_h);}
/** The standard jobs start counting from now: each one without a last-done hour takes the hours the printer has now. */
export async function adoptJobs(hours){const tasks=ctx.state.presets?.maintenance||[];if(hours===null||!tasks.some(t=>t.last_h==null&&t.last_at==null))return;
 await ctx.savePresets({...ctx.state.presets,maintenance:tasks.map(t=>t.last_h==null&&t.last_at==null?{...t,last_h:hours,last_at:Date.now()/1000}:t)}).catch(()=>{});}
export function pageMaintenance(){sheet({title:'Maintenance',eyebrow:'BY HOURS PRINTED',icon:'i-nav-maintenance',wide:true,build:body=>{
 const H=stat('HOURS PRINTED'),D=stat('DUE NOW'),list=el('div','nx-maint');body.append(statGrid(H,D),list);let hours=null,src='';
 const save=async tasks=>{await ctx.savePresets({...ctx.state.presets,maintenance:tasks});draw();};
 function draw(){const tasks=ctx.state.presets?.maintenance||[];list.replaceChildren();H.set(hours===null?'—':hours.toFixed(1)+' h',{note:src});const due=dueTasks(hours);D.set(String(due.length),{tone:due.length?'warn':'ok'});
  if(!tasks.length){const v=ctx.has('klipper');   // polish WO-05: the starting list names no Voron and no Input shaper on a printer that is neither
   list.append(el('p','nx-note',`No schedule yet. Start from the ${v?'Voron':'standard'} one and change the hours to suit you.`),button({icon:'i-plus',label:`Use the ${v?'Voron':'standard'} schedule`,kind:'primary',size:'row',title:`Use the ${v?'Voron':'standard'} schedule`,does:v?'Adds the common Voron 2.4 jobs with their hours. Nothing is sent to the printer.':'Adds the common jobs with their hours: nozzle, build sheet, rails, extruder, bolts. Nothing is sent to the printer.',
   onPress:()=>save(VORON_SCHEDULE.map(([name,every_h])=>({name:v?name:name.replace(' (Input shaper: measure belts)',''),every_h,last_h:hours,last_at:Date.now()/1000})))}));return;}
  // rc9: a standard job (last_h null) starts counting the first time the page knows the hours, so a printer with 900 hours behind it is not told every job is due
  if(hours!==null&&tasks.some(t=>t.last_h==null&&t.last_at==null)){save(tasks.map(t=>t.last_h==null&&t.last_at==null?{...t,last_h:hours,last_at:Date.now()/1000}:t));return;}
  tasks.forEach((t,i)=>{const since=hours===null?null:hours-(t.last_h??0),f=since===null?0:Math.min(1,since/t.every_h),row=el('div','nx-maint-row'),txt=el('div','nx-maint-t'),bar=el('div','nx-print-bar'),fl=el('i');bar.append(fl);fl.style.width=(f*100).toFixed(0)+'%';
   row.dataset.due=String(since!==null&&since>=t.every_h);txt.append(el('b','',t.name),el('small','',`every ${t.every_h} h · ${since===null?'hours unknown':since.toFixed(0)+' h since'}${t.last_at?' · last done '+new Date(t.last_at*1000).toLocaleDateString():''}`),bar);
   // U06 (2026-10-20): the interval is a number that is set, so it is the kit's control (10 hours a click), saved once when the hand lets go
   const every=ctl('maint',{label:'EVERY',unit:'h',min:10,max:600,step:10,turn:300,value:t.every_h,color:'#FF6A00',color2:'#FFB067',onSet:v=>save(tasks.map((x,k)=>k===i?{...x,every_h:v}:x))});tip(every.el,'Every how many hours','Press and hold, then turn: the job is due every so many print hours, 10 a click. Letting go saves it.');
   row.append(txt,every.el,button({icon:'i-check',label:'Done',size:'row',kind:since!==null&&since>=t.every_h?'primary':'default',title:'Mark done',does:'Records it as done now, at today\'s hour count, and adds a line to the service journal.',
    onPress:async()=>{await save(tasks.map((x,k)=>k===i?{...x,last_h:hours,last_at:Date.now()/1000}:x));try{const {validateBook,emptyBook,journal}=await import('./service-core.js'),key='stella-live:parts:v1:stella-voron-350',raw=localStorage.getItem(key),b=raw?validateBook(JSON.parse(raw)):emptyBook();localStorage.setItem(key,JSON.stringify(journal(b,'maintenance','checked',t.name,`Done at ${hours===null?'unknown':hours.toFixed(1)} print hours (Maintenance page).`)));}catch{}toast(t.name+': done.','ok');}}),
    button({icon:'i-close',size:'icon',kind:'danger',title:'Take this job away',does:'Removes it from the schedule. Add it back from the standard list at any time.',onPress:()=>save(tasks.filter((_,k)=>k!==i))}));list.append(row);});
  list.append(button({icon:'i-plus',label:'Add from the standard list',size:'row',title:'Add from the standard list',does:'Shows the standard jobs you do not have, with their hours.',
   onPress:b=>addFromStandard(b,'maintenance',tasks.map(t=>t.name),add=>save([...tasks,...add.map(x=>({...x,last_h:hours,last_at:hours===null?null:Date.now()/1000}))]),{noun:'job',detail:x=>`every ${x.every_h} h`})}));}
 printHours().then(r=>{hours=r.h;src=r.live?'from the printer':r.at?'last known, '+new Date(r.at*1000).toLocaleDateString():'not reported yet';draw();});draw();}});}

// ---------- input shaper and belts ----------
// Klipper's shaper definitions (shaper_defs.py), for the response curves drawn over the PSD.
function shaperAT(type,f,z=0.1){const df=Math.sqrt(1-z*z),K=Math.exp(-z*Math.PI/df),td=1/(f*df),v=0.05;
 if(type==='zv')return [[1,K],[0,.5*td]];
 if(type==='mzv'){const K2=Math.exp(-.75*z*Math.PI/df),a1=1-1/Math.SQRT2,a2=(Math.SQRT2-1)*K2,a3=a1*K2*K2;return [[a1,a2,a3],[0,.375*td,.75*td]];}
 if(type==='ei'){const a1=.25*(1+v),a2=.5*(1-v)*K,a3=a1*K*K;return [[a1,a2,a3],[0,.5*td,td]];}
 if(type==='2hump_ei'){const V2=v*v,X=Math.pow(V2*(Math.sqrt(1-V2)+1),1/3),a1=(3*X*X+2*X+3*V2)/(16*X),a2=(.5-a1)*K,a3=a2*K,a4=a1*K*K*K;return [[a1,a2,a3,a4],[0,.5*td,td,1.5*td]];}
 if(type==='3hump_ei'){const K2=K*K,a1=.0625*(1+3*v+2*Math.sqrt(2*(v+1)*v)),a2=.25*(1-v)*K,a3=(.5*(1+v)-2*a1)*K2,a4=a2*K2,a5=a1*K2*K2;return [[a1,a2,a3,a4,a5],[0,.5*td,td,1.5*td,2*td]];}
 return null;}
export function shaperResponse(type,freq,grid,z=0.1){const at=shaperAT(type,freq);if(!at)return null;const [A,T]=at,sa=A.reduce((a,b)=>a+b,0),tl=T.at(-1);
 return grid.map(f=>{const w=2*Math.PI*f,d=z*w,wd=w*Math.sqrt(1-z*z);let s=0,c=0;for(let i=0;i<A.length;i++){const W=A[i]*Math.exp(-d*(tl-T[i]));s+=W*Math.sin(wd*T[i]);c+=W*Math.cos(wd*T[i]);}return Math.sqrt(s*s+c*c)/sa;});}
const SHAPER_COL={zv:'#8FA3FF',mzv:'#04D9C4',ei:'#FFD60A',['2hump_ei']:'#FF9C55',['3hump_ei']:'#FF2D92'};
export function pageShaper(){sheet({title:'Input shaper',eyebrow:'RESONANCES AND BELTS',icon:'i-nav-signals',wide:true,build:body=>{
 const acts=el('div','nx-grid3');for(const [ax,l,d]of [['X','Measure X','Shakes the head along X and records the accelerometer.'],['Y','Measure Y','Same along Y.'],['belts','Compare belts','Shakes along both diagonals, one belt each, so A and B can be compared.']])
  acts.append(button({icon:'i-nav-signals',label:l,size:'row',kind:ax==='belts'?'primary':'default',cmd:'resonance_capture',title:l,does:d+' Needs the printer homed and the accelerometer on the toolhead board. Shows the commands first.',onPress:async b=>{busy(b,true);try{if(await ctx.send('resonance_capture',{axis:ax}))setTimeout(loadRuns,4000);}finally{busy(b,false);}}}));
 const runs=el('div','nx-chips'),view=el('div','nx-shaper');body.append(acts,section('Measurements',runs),view);
 async function loadRuns(){try{const d=await getJson('/api/resonance'),rs=d.runs||[];runs.replaceChildren();
   if(!rs.length){runs.append(el('p','nx-note','No measurements yet. Home, then press Measure X, Measure Y or Compare belts. Each takes about a minute and the printer shakes and hums.'));view.replaceChildren();return;}
   rs.forEach((r,i)=>{const b=el('button','nx-chip',`${new Date(r.ts*1000).toLocaleDateString()} · ${r.axis==='belts'?'belts':r.axis}${r.status!=='complete'?' · '+r.status:r.recommended?.type?' · '+r.recommended.type+' '+r.recommended.freq+' Hz':''}`);b.type='button';b.setAttribute('aria-pressed',String(i===0));
    b.addEventListener('click',()=>{for(const x of runs.children)x.setAttribute('aria-pressed','false');b.setAttribute('aria-pressed','true');show(r.id);});runs.append(b);});show(rs[0].id);
   if(rs.some(r=>r.status==='running'))setTimeout(loadRuns,5000);}catch(e){runs.replaceChildren(el('p','nx-note','Measurements unavailable: '+e.message));}}
 async function show(id){view.replaceChildren(el('p','nx-note','Reading…'));let d;try{d=await getJson('/api/resonance/'+id);}catch(e){view.replaceChildren(el('p','nx-note',e.message));return;}
  view.replaceChildren();if(d.status!=='complete'){view.append(el('p','nx-note',d.status==='running'?'Still measuring…':'This measurement did not finish'+(d.error?': '+d.error:'.')));return;}
  const cv=el('canvas','nx-shaper-cv');view.append(cv);const legend=el('div','nx-chips');view.append(legend);
  if(d.belts)beltsView(view,d,cv);else axisView(view,d,cv,legend);}
 function axisView(view,d,cv,legend){const grid=d.psd?.freq||[],sum=d.psd?.sum||[],rec=d.recommended;const lines=[['sum','#EEF2F6',sum,2],['x','#FF6A6A',d.psd?.x,1],['y','#6AC8FF',d.psd?.y,1],['z','#8A93A0',d.psd?.z,1]];
  const shapers=(d.shapers||[]).map(s=>({...s,resp:shaperResponse(s.type,s.freq,grid)}));
  plot(cv,grid,lines.filter(l=>Array.isArray(l[2])),shapers.map(s=>[s.type,SHAPER_COL[s.type]||'#fff',s.resp,s.type===rec?.type?2.2:1]));
  for(const [n,c]of [['PSD sum','#EEF2F6'],['X','#FF6A6A'],['Y','#6AC8FF'],['Z','#8A93A0'],...shapers.map(s=>[s.type+' '+s.freq+' Hz',SHAPER_COL[s.type]])]){const k=el('span','nx-key',n);k.style.setProperty('--c',c);legend.append(k);}
  const S=statGrid(stat('RECOMMENDED'),stat('MAX ACCEL'),stat('PEAK'));S.children[0].querySelector('.nx-stat-v').textContent=rec?`${rec.type} ${rec.freq} Hz`:'—';S.children[1].querySelector('.nx-stat-v').textContent=rec?Math.round(rec.max_accel)+' mm/s²':'—';S.children[2].querySelector('.nx-stat-v').textContent=d.peak_hz?d.peak_hz+' Hz':'—';view.append(S);
  const t=el('table','nx-table'),th=el('thead'),hr=el('tr');for(const h of ['Shaper','Frequency','Vibrations','Smoothing','Max accel'])hr.append(el('th','',h));th.append(hr);t.append(th);const tb=el('tbody');
  for(const s of d.shapers||[]){const tr=el('tr');if(s.type===rec?.type)tr.dataset.best='';for(const v of [s.type,s.freq+' Hz',s.vibrations_pct+' %',s.smoothing,Math.round(s.max_accel)+' mm/s²'])tr.append(el('td','',String(v)));tb.append(tr);}t.append(tb);const wrap=el('div','nx-table-wrap');wrap.append(t);view.append(wrap);
  if(rec){const ax=String(d.axis).toUpperCase(),line=`SET_INPUT_SHAPER SHAPER_TYPE_${ax}=${rec.type} SHAPER_FREQ_${ax}=${rec.freq}`;
   view.append(section('Use it',el('p','nx-note',`Try it now for this session with the line below; to keep it, put shaper_type_${ax.toLowerCase()}: ${rec.type} and shaper_freq_${ax.toLowerCase()}: ${rec.freq} under [input_shaper] in printer.cfg.`),el('pre','nx-review',line),
    button({icon:'i-play',label:'Try it now',size:'row',kind:'primary',cmd:'console',title:'Try this shaper',does:'Sends the line once, after showing it. Klipper forgets it on a restart.',onPress:()=>ctx.send('console',{script:line})})));}
  ctx.lockControls(view);}
 function beltsView(view,d,cv){const grid=d.psd?.freq||[],a=d.belts.a||[],b=d.belts.b||[];plot(cv,grid,[['A','#FF9C55',a,2],['B',ink('--accent','#04D9C4'),b,2]],[]);
  const pk=arr=>{let bi=0;for(let i=1;i<arr.length;i++)if(arr[i]>arr[bi])bi=i;return grid[bi];},pa=pk(a),pb=pk(b),diff=pa-pb,sim=d.belts.similarity;
  // TEST_RESONANCES AXIS=1,1 moves only stepper_x (the B motor on a Voron) and 1,-1 only stepper_y (A).
  const leg=d.legs||[],nameA=leg[0]?.name||'1,1',nameB=leg[1]?.name||'1,-1';
  const S=statGrid(stat('SIMILARITY'),stat(`PEAK · ${nameA}`),stat(`PEAK · ${nameB}`));S.children[0].querySelector('.nx-stat-v').textContent=finite(sim)===null?'—':Math.round(sim*100)+' %';S.children[1].querySelector('.nx-stat-v').textContent=pa?pa.toFixed(1)+' Hz':'—';S.children[2].querySelector('.nx-stat-v').textContent=pb?pb.toFixed(1)+' Hz':'—';view.append(S);
  const tighter=diff>0?`the ${nameA} path (stepper_x, the B belt)`:`the ${nameB} path (stepper_y, the A belt)`,looser=diff>0?`the ${nameB} path (stepper_y, the A belt)`:`the ${nameA} path (stepper_x, the B belt)`;
  const advice=Math.abs(diff)<=1.5&&(sim??0)>=.9?'The two belts match well. Leave them.':Math.abs(diff)<=1.5?`The peaks line up (${Math.abs(diff).toFixed(1)} Hz apart) but the curves differ (${Math.round((sim??0)*100)} % alike): look for a rubbing belt, a loose pulley grub screw or an idler that does not spin freely.`
   :`${tighter[0].toUpperCase()+tighter.slice(1)} rings ${Math.abs(diff).toFixed(1)} Hz higher, so it is tighter. Loosen it a little or tighten ${looser}, a quarter turn of the tensioner at a time, and measure again until the peaks are within about 1 Hz.`;
  view.append(section('Belts',el('p','nx-belt-advice',advice),el('p','nx-note','A tighter belt rings at a higher frequency. Which physical belt each diagonal drives is taken from Klipper\'s CoreXY maths; if yours is wired the other way round, swap the names.')));}
 function plot(cv,grid,lines,shapers){const dpr=Math.min(devicePixelRatio||1,2),W=cv.clientWidth||700,H=cv.clientHeight||320;cv.width=W*dpr;cv.height=H*dpr;const g=cv.getContext('2d');g.setTransform(dpr,0,0,dpr,0,0);g.clearRect(0,0,W,H);
  const L=48,R=shapers.length?40:12,T=12,B=26,w=W-L-R,h=H-T-B,fmax=Math.min(200,grid.at(-1)||200),X=f=>L+f/fmax*w;let mx=0;for(const [,,a]of lines)for(const v of a||[])if(finite(v)!==null)mx=Math.max(mx,v);mx=mx||1;const Y=v=>T+h-(v/mx)*h;
  g.strokeStyle='rgba(255,255,255,.06)';g.fillStyle='#8B95A1';g.font='500 10px "Geist Mono",monospace';for(let f=0;f<=fmax;f+=25){g.beginPath();g.moveTo(X(f),T);g.lineTo(X(f),T+h);g.stroke();g.textAlign='center';g.fillText(f,X(f),H-8);}g.textAlign='left';g.fillText('Hz',W-R+4,H-8);
  g.save();g.translate(12,T+h/2);g.rotate(-Math.PI/2);g.textAlign='center';g.fillText('power',0,0);g.restore();
  for(const [,c,a,lw]of lines){if(!a)continue;g.beginPath();a.forEach((v,i)=>{if(grid[i]>fmax)return;const x=X(grid[i]),y=Y(v||0);i?g.lineTo(x,y):g.moveTo(x,y);});g.strokeStyle=c;g.lineWidth=lw;g.stroke();}
  for(const [,c,resp,lw]of shapers){if(!resp)continue;g.beginPath();resp.forEach((v,i)=>{if(grid[i]>fmax)return;const x=X(grid[i]),y=T+h-v*h;i?g.lineTo(x,y):g.moveTo(x,y);});g.strokeStyle=c;g.lineWidth=lw;g.setLineDash([5,4]);g.stroke();g.setLineDash([]);}
  if(shapers.length){g.fillStyle='#8B95A1';g.textAlign='left';g.fillText('1.0',W-R+4,T+8);g.fillText('0',W-R+4,T+h);}}
 loadRuns();ctx.lockControls(acts);}});}

// ---------- camera ----------
// The camera's own MJPEG stream, passed through the service (see src/camera.rs): continuous, not
// Mainsail's snapshot-at-a-time adaptive mode. The stream closes when the view closes.
export async function cameras(){const d=await getJson('/api/camera');return d.cameras||[];}
// WO-14 item 3 (measured: ~15 s of LOADING before the first frame): show the latest snapshot the
// instant the card opens, and swap to the MJPEG stream only after ITS first frame has arrived, so
// the card shows a real picture at once and the live stream fades in under it. Any two images at a
// time is inside the recorder's two-camera budget: the snapshot request closes before the stream.
export function camImg(cam){const q='?name='+encodeURIComponent(cam.name||'');
 // On a per-printer page the camera belongs to that printer; on / it is the first printer.
 const m=location.pathname.match(/^\/p\/([^/]+)/);const api=m?`/api/p/${m[1]}`:'/api';
 const img=el('img','nx-cam-img');img.alt=(cam.name||'Camera')+', live';img.decoding='async';
 img.src=api+'/camera/snapshot'+q;
 const live=new Image();live.decoding='async';
 live.onload=()=>{img.src=api+'/camera/stream'+q;};
 live.onerror=()=>{const t=setInterval(()=>{if(!img.isConnected){clearInterval(t);return;}img.src=api+'/camera/snapshot'+q+'&t='+Date.now();},4000);img.addEventListener('load',()=>clearInterval(t),{once:true});};
 live.src=api+'/camera/stream'+q;
 const t=[cam.flip_horizontal?'scaleX(-1)':'',cam.flip_vertical?'scaleY(-1)':'',cam.rotation?`rotate(${cam.rotation}deg)`:''].filter(Boolean).join(' ');if(t)img.style.transform=t;return img;}
export function pageCamera(){sheet({plain:true,title:'Camera',eyebrow:'LIVE',icon:'i-view',wide:true,onClose:()=>{for(const i of document.querySelectorAll('#nx-sheet .nx-cam-img'))i.src='';},build:body=>{
 const box=el('div','nx-cam');body.append(box);box.append(el('p','nx-note','Finding the camera…'));
 cameras().then(cams=>{box.replaceChildren();if(!cams.length){box.append(el('p','nx-note','No camera for this printer yet. A Klipper or FlashForge camera appears here by itself. For a Prusa, type its camera address under Printers, then Settings.'),button({icon:'i-printers',label:'Open Printers',size:'row',title:'Open Printers',does:'Lists the printers in the farm; each has Settings.',onPress:()=>import('./next-farm.js').then(m=>m.sheetPrinters())}));return;}
  for(const c of cams){const fig=el('figure','nx-cam-fig'),img=camImg(c),cap=el('figcaption');cap.append(el('b','',c.name||'Camera'),el('span','',`${ctx.has('klipper')?c.service||'':''}${c.target_fps?' · camera set to '+c.target_fps+' fps':''}`));
   img.addEventListener('error',()=>{cap.lastChild.textContent='The stream stopped: the printer or camera is not answering.';});fig.append(img,cap);box.append(fig);}
  const pin=button({icon:'i-panels',label:ctx.camPinned()?'Back to the 3D machine':'Make it the background',size:'row',title:'Camera as the background',does:'Fills the whole main view with the live camera, with light glass over it. One click on the picture grows it.',onPress:b=>{const on=!ctx.camPinned();ctx.camPinned(on);b.querySelector('.nx-lbl').textContent=on?'Back to the 3D machine':'Make it the background';}});
  box.append(pin,el('p','nx-note',ctx.has('klipper')?'If it is still slow here, the camera itself is set to a low frame rate or resolution in crowsnest.conf on the printer.':'If it is still slow here, the camera itself is set to a low frame rate or resolution in its own settings.'));})
 .catch(e=>{box.replaceChildren(el('p','nx-note',camWords(e.message)));});}});}
// 2026-10-25: the camera page printed "cannot reach Moonraker ... os error 111 ... set STELLA_MOONRAKER_API_KEY". Plain words, by what went wrong.
function camWords(m){const t=String(m||'');
 if(/cannot reach|refused|timed? ?out|not answering|unreachable|no route/i.test(t))return 'The camera is not reachable: the printer is off or not on the network. It shows here by itself when the printer is on.';
 if(/401|403|unauthori|forbidden|api.?key/i.test(t))return 'The printer refused to share its camera. It needs its login: open Printers, then this printer\'s settings.';
 return 'The camera is not available right now. It shows here by itself when it answers.';}
