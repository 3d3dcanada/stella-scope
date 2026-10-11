// Stella Scope NEXT · instrument style lab. Three candidate designs for the
// instruments, all interactive, none connected to the printer: the user picks, then the winner is
// built into /next. Current readings are simulated so heating and settling can be felt.
import {heatColor,arcPath,polar} from './next-core.js';

const CH=[
 {key:'nozzle',label:'NOZZLE',unit:'°',min:0,max:300,major:6,step:5,cur:24,target:250,kind:'heat',heat:'nozzle'},
 {key:'bed',label:'BED',unit:'°',min:0,max:120,major:6,step:5,cur:23,target:80,kind:'heat',heat:'bed'},
 {key:'speed',label:'SPEED',unit:'%',min:50,max:200,major:6,step:5,cur:100,target:100,kind:'factor',live:()=>Math.round(186+Math.sin(performance.now()/700)*14)+' mm/s'},
 {key:'flow',label:'FLOW',unit:'%',min:80,max:120,major:4,step:1,cur:100,target:100,kind:'factor',live:()=>(15.2+Math.sin(performance.now()/900)*1.1).toFixed(1)+' mm³/s'}];
const el=(t,c='',x='')=>{const e=document.createElement(t);if(c)e.className=c;if(x!=='')e.textContent=x;return e;};
const clamp=(v,a,b)=>Math.max(a,Math.min(b,v));
const snap=(c,v)=>clamp(Math.round(v/c.step)*c.step,c.min,c.key==='nozzle'?290:c.max);
const frac=(c,v)=>(v-c.min)/(c.max-c.min);
const ink=c=>c.kind==='heat'?heatColor(c.cur,c.target,c.heat).ink:'#04D9C4';
function keys(node,c,get,set){node.tabIndex=0;node.setAttribute('role','slider');node.setAttribute('aria-label',c.label);
 node.addEventListener('keydown',e=>{const d={ArrowUp:1,ArrowRight:1,ArrowDown:-1,ArrowLeft:-1,PageUp:5,PageDown:-5}[e.key];if(d){e.preventDefault();set(snap(c,get()+d*c.step));}});}

// ---------- A. climate knob ----------
function knob(c,state){
 const w=el('div','knob');const ring=el('div','knob-ring'),track=el('div','knob-track'),tick=el('i','knob-cur'),handle=el('i','knob-handle'),face=el('div','knob-face');
 const big=el('div','knob-big'),now=el('div','knob-now'),lab=el('div','knob-label',c.label);face.append(big,now,lab);w.append(track,ring,tick,handle,face);
 const A0=225,SW=270;
 function paint(){const f=frac(c,state[c.key].target),fc=frac(c,state[c.key].cur),col=ink(state[c.key]);
  w.style.setProperty('--col',col);w.style.setProperty('--deg',(SW*f)+'deg');
  const a=(A0+SW*f)*Math.PI/180,r=47.2;handle.style.left=(50+r*Math.sin(a))+'%';handle.style.top=(50-r*Math.cos(a))+'%';
  const b=(A0+SW*fc);tick.style.transform=`rotate(${b}deg)`;
  big.replaceChildren(String(state[c.key].target));big.append(el('small','',c.unit));
  now.textContent=c.kind==='heat'?`NOW ${Math.round(state[c.key].cur)}°`:c.live();
  w.dataset.active=String(c.kind==='heat'?Math.abs(state[c.key].cur-state[c.key].target)>3&&state[c.key].target>0:false);}
 const set=v=>{state[c.key].target=v;paint();};
 w.addEventListener('pointerdown',e=>{w.setPointerCapture(e.pointerId);w.dataset.drag='';move(e);});
 const move=e=>{if(!('drag' in w.dataset))return;const r=w.getBoundingClientRect(),dx=e.clientX-r.left-r.width/2,dy=e.clientY-r.top-r.height/2;
  let deg=Math.atan2(dx,-dy)*180/Math.PI;if(deg<0)deg+=360;let rel=(deg-A0+360)%360;if(rel>SW)rel=rel>SW+45?0:SW;set(snap(c,c.min+(c.max-c.min)*rel/SW));};
 w.addEventListener('pointermove',move);w.addEventListener('pointerup',()=>delete w.dataset.drag);keys(w,c,()=>state[c.key].target,set);
 return {el:w,paint};}

// ---------- B. control-center sliders ----------
function pill(c,state){
 const w=el('div','pill'),fill=el('div','pill-fill'),cur=el('i','pill-cur'),top=el('div','pill-top'),val=el('div','pill-val'),bot=el('div','pill-bot'),lab=el('div','pill-lab',c.label),now=el('div','pill-now');
 top.append(val);bot.append(lab,now);w.append(fill,cur,top,bot);
 function paint(){const s=state[c.key],f=frac(c,s.target);w.style.setProperty('--col',ink(s));w.style.setProperty('--f',f);w.style.setProperty('--c',frac(c,s.cur));
  val.replaceChildren(String(s.target));val.append(el('small','',c.unit));now.textContent=c.kind==='heat'?`now ${Math.round(s.cur)}°`:c.live();cur.hidden=c.kind!=='heat';}
 const set=v=>{state[c.key].target=v;paint();};let start=null;
 w.addEventListener('pointerdown',e=>{w.setPointerCapture(e.pointerId);start={y:e.clientY,v:state[c.key].target};w.dataset.drag='';});
 w.addEventListener('pointermove',e=>{if(!start)return;const h=w.getBoundingClientRect().height;set(snap(c,start.v+(start.y-e.clientY)/h*(c.max-c.min)));});
 w.addEventListener('pointerup',()=>{start=null;delete w.dataset.drag;});keys(w,c,()=>state[c.key].target,set);
 return {el:w,paint};}

// ---------- C. cluster with a target bug on the bezel ----------
const SV='http://www.w3.org/2000/svg';const sv=(t,a={})=>{const e=document.createElementNS(SV,t);for(const[k,v]of Object.entries(a))e.setAttribute(k,v);return e;};
function cluster(c,state){
 const w=el('div','clu');const s=sv('svg',{viewBox:'0 0 240 240'}),C=120,G0=150,GS=240,A=f=>G0+GS*clamp(f,0,1);
 s.append(sv('circle',{cx:C,cy:C,r:116,class:'c-bezel'}),sv('circle',{cx:C,cy:C,r:104,class:'c-face'}),sv('path',{class:'c-track',d:arcPath(C,C,96,G0,G0+GS)}));
 const set_=sv('path',{class:'c-set'}),now_=sv('path',{class:'c-now'});s.append(set_,now_);
 const g=sv('g',{class:'c-scale'});const n=c.major*5;for(let i=0;i<=n;i++){const a=G0+GS*i/n,m=i%5===0,p0=polar(C,C,m?80:84,a),p1=polar(C,C,89,a);g.append(sv('line',{x1:p0.x,y1:p0.y,x2:p1.x,y2:p1.y,class:m?'m':''}));
  if(m){const p=polar(C,C,68,a),t=sv('text',{x:p.x,y:p.y+4});t.textContent=String(Math.round(c.min+(c.max-c.min)*i/n));g.append(t);}}
 s.append(g);const needle=sv('g',{class:'c-needle'});needle.append(sv('path',{d:`M${C-3} ${C+12} L${C} ${C-88} L${C+3} ${C+12} Z`}));s.append(needle,sv('circle',{cx:C,cy:C,r:7,class:'c-hub'}));
 const bug=sv('g',{class:'c-bug'});bug.append(sv('path',{d:`M${C} ${C-110} l-8 -10 h16 z`}));s.append(bug);
 const read=el('div','clu-read'),val=el('div','clu-val'),lab=el('div','clu-lab',c.label),set=el('div','clu-set');read.append(val,lab,set);w.append(s,read);
 function paint(){const st=state[c.key],ft=frac(c,st.target),fc=frac(c,st.cur);w.style.setProperty('--col',ink(st));
  set_.setAttribute('d',arcPath(C,C,96,G0,A(ft)));now_.setAttribute('d',fc>0.003?arcPath(C,C,96,G0,A(fc)):'');
  needle.style.transform=`rotate(${A(fc)-270}deg)`;bug.style.transform=`rotate(${A(ft)-270}deg)`;
  val.replaceChildren(c.kind==='heat'?String(Math.round(st.cur)):String(st.target));val.append(el('small','',c.unit));
  set.textContent=c.kind==='heat'?`SET ${st.target}°`:c.live();}
 const setV=v=>{state[c.key].target=v;if(c.kind==='factor')state[c.key].cur=v;paint();};
 w.addEventListener('pointerdown',e=>{w.setPointerCapture(e.pointerId);w.dataset.drag='';move(e);});
 const move=e=>{if(!('drag' in w.dataset))return;const r=w.getBoundingClientRect(),dx=e.clientX-r.left-r.width/2,dy=e.clientY-r.top-r.height/2;
  let deg=Math.atan2(dy,dx)*180/Math.PI;if(deg<0)deg+=360;let rel=(deg-G0+360)%360;if(rel>GS)rel=rel>GS+60?0:GS;setV(snap(c,c.min+(c.max-c.min)*rel/GS));};
 w.addEventListener('pointermove',move);w.addEventListener('pointerup',()=>delete w.dataset.drag);keys(w,c,()=>state[c.key].target,setV);
 return {el:w,paint};}

// ---------- page ----------
const variants=[['A','Climate knob','Drag around the ring like a thermostat or a car\'s climate knob. The big number is what you set; NOW is the reading.',knob],
 ['B','Control Center sliders','Slide up and down anywhere on the bar, like the iPhone or Tesla climate bars. The fill is what you set; the white line is the reading.',pill],
 ['C','Cluster with a target marker','The needle is the live reading. Drag the triangle around the rim to set the target; the dim arc shows what you set.',cluster]];
const lab=document.getElementById('lab'),all=[];
for(const [id,name,desc,make]of variants){const state=Object.fromEntries(CH.map(c=>[c.key,{...c}]));
 const sec=el('section','lab-var');const h=el('div','lab-var-head');h.append(el('span','lab-id',id),el('h2','',name),el('p','',desc));
 const row=el('div','lab-row lab-row-'+id);for(const c of CH){const inst=make(c,state);inst.paint();row.append(inst.el);all.push([state,c,inst]);}
 sec.append(h,row);lab.append(sec);}
// simulated readings: heaters chase their targets, factors are what you set
setInterval(()=>{for(const [st,c,inst]of all){const s=st[c.key];if(c.kind==='heat'){const d=s.target-s.cur;s.cur+=d>0?Math.min(d,Math.max(.6,d*.04)):Math.max(d,d*.03-.2);}else s.cur=s.target;inst.paint();}},120);
