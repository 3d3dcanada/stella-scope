// Stella Scope UI board U01 (2026-10-20): the control kit, the one module every page imports.
// A number is one piece: at rest an LED gauge (36 chiclets lit to the value, the value in the middle),
// held and turned a black anodised knob with a lit index that grows where it is — on a phone it rises
// to the middle — and springs back a gauge with the new value. Never a pop-up, never a dialog for a
// number. Everything that is not a number is a square key. Steps, colours, segment count and turn
// travel are the ones the owner approved (docs/design/knob-2026-10-20/src/led.html): use them, do not
// redesign them here.
const SV='http://www.w3.org/2000/svg',SEGS=36,A0=-135,SPAN=270,SMALL=900;
let SWEEPS=0;   // gauges swept so far on this page: each starts a little after the one before
// Each control its own step and its own speed: `turn` is degrees of finger travel for the whole
// range, so nothing runs away under the finger.
export const CONTROLS=[
 {key:'nozzle',label:'NOZZLE',unit:'°',min:0,max:290,step:5,turn:300,color:'var(--heat,#FF6A00)',color2:'var(--heat-2,#FFB067)'},
 {key:'bed',label:'BED',unit:'°',min:0,max:120,step:1,turn:300,color:'var(--heat,#FF6A00)',color2:'var(--heat-2,#FFB067)'},
 {key:'speed',label:'SPEED',unit:'%',min:10,max:200,step:5,turn:300,color:'var(--gauge,var(--accent))',color2:'var(--gauge-2,var(--accent-2))'},
 {key:'flow',label:'FLOW',unit:'%',min:50,max:150,step:1,turn:300,color:'var(--gauge,var(--accent))',color2:'var(--gauge-2,var(--accent-2))'},
 {key:'part',label:'PART FAN',unit:'%',min:0,max:100,step:5,turn:270,color:'var(--cool,#3D8BFF)',color2:'var(--cool-2,#9EC5FF)'},
 {key:'hotend',label:'HOTEND FAN',unit:'%',min:0,max:100,step:5,turn:270,color:'var(--cool,#3D8BFF)',color2:'var(--cool-2,#9EC5FF)'},
 {key:'board',label:'BAY FANS',unit:'%',min:0,max:100,step:5,turn:270,color:'var(--cool,#3D8BFF)',color2:'var(--cool-2,#9EC5FF)'}];
let ac=null;
// The click: a band-passed noise burst, deep for the grab, the release and the stops, light per detent.
export function tick(deep){try{ac=ac||new (window.AudioContext||window.webkitAudioContext)();if(ac.state==='suspended')ac.resume();
 const t=ac.currentTime,n=ac.createBuffer(1,Math.max(1,Math.floor(ac.sampleRate*.01)),ac.sampleRate),d=n.getChannelData(0);
 for(let i=0;i<d.length;i++)d[i]=(Math.random()*2-1)*Math.pow(1-i/d.length,5);
 const s=ac.createBufferSource();s.buffer=n;const f=ac.createBiquadFilter();f.type='bandpass';f.frequency.value=deep?1100:4200;f.Q.value=deep?2:8;
 const g=ac.createGain();g.gain.setValueAtTime(deep?.8:.45,t);g.gain.exponentialRampToValueAtTime(.001,t+.025);s.connect(f).connect(g).connect(ac.destination);s.start(t);}catch(e){}}
// U08 (2026-10-20): inside the Stella phone app the phone's own haptic engine clicks (Tauri haptics plugin: a selection tick per detent, a
// medium impact at grab, release and the stops); in a phone browser the vibration motor; on a PC nothing.
const tauriInvoke=()=>window.__TAURI__?.core?.invoke;
export const buzz=deep=>{const inv=tauriInvoke();if(inv){inv(deep?'plugin:haptics|impact_feedback':'plugin:haptics|selection_feedback',deep?{style:'medium'}:{}).catch(()=>{});return;}
 if(navigator.vibrate)try{navigator.vibrate(deep?14:4);}catch(e){}};
export const click=(deep=true)=>{tick(deep);buzz(deep);};
const clamp=(v,a,b)=>Math.max(a,Math.min(b,v));
const step=c=>c.step>0?c.step:1;
// U06 (2026-10-20): a fractional step (the jog wheel's 0.1 mm) must not leave float noise (0.30000000000000004): the result is rounded to the step's own decimals.
const snap=(c,v)=>{const s=step(c),d=(String(s).split('.')[1]||'').length;return clamp(+(Math.round(v/s)*s).toFixed(d),c.min,c.max);};
const frac=(c,v)=>(v-c.min)/(c.max-c.min||1);
// A value that arrives (or is typed) is kept as it is, clamped and rounded to the step's decimals; only the knob clicks by whole steps. A wheel or key step from a value that is off the
// step (a nozzle of 212°) goes to the next click in its direction (215°, or 210°), never a part of one.
const fit=(c,v)=>clamp(+(+v).toFixed(dp(c)),c.min,c.max);
const bump=(c,v,dir)=>{const s=step(c),q=v/s,n=dir>0?Math.floor(q+1e-9)+1:Math.ceil(q-1e-9)-1;return clamp(+(n*s).toFixed(dp(c)),c.min,c.max);};
// U06: a page may give its own words for a value (`format`: the jog wheel shows +2.5 mm); the default is the kit's own.
// U05 (2026-10-20): a step of 0.05 (a nozzle in millimetres) or 0.5 snaps and shows to its own decimals, never 0.35000000000000003
const dp=c=>{const t=String(step(c));return t.includes('.')?t.split('.')[1].length:0;};
const fmt=(c,v)=>(c.prefix||'')+v.toFixed(dp(c));   // `prefix` is for a currency ($25.50)
const shown=(c,v)=>c.format?c.format(v):v===0&&c.unit==='°'?'OFF':fmt(c,v)+c.unit;
// One shared set of defs (the LED glow filter) and one dim overlay, installed the first time a
// control is made, so every page that imports the kit gets them without duplicating ids.
function install(){if(document.getElementById('nxctl-kit'))return;
 const svg=document.createElementNS(SV,'svg');svg.id='nxctl-kit';svg.setAttribute('width','0');svg.setAttribute('height','0');svg.setAttribute('aria-hidden','true');svg.style.position='absolute';
 const blur=document.createElementNS(SV,'feGaussianBlur');blur.setAttribute('stdDeviation','1.6');blur.setAttribute('result','b');
 const merge=document.createElementNS(SV,'feMerge');for(let i=0;i<2;i++){const n=document.createElementNS(SV,'feMergeNode');n.setAttribute('in','b');merge.append(n);}
 const src=document.createElementNS(SV,'feMergeNode');src.setAttribute('in','SourceGraphic');merge.append(src);
 const f=document.createElementNS(SV,'filter');f.id='nxctl-glow';f.setAttribute('x','-80%');f.setAttribute('y','-80%');f.setAttribute('width','260%');f.setAttribute('height','260%');f.append(blur,merge);
 svg.append(f);document.body.append(svg);
 const dim=document.createElement('div');dim.id='nxctl-dim';document.body.append(dim);}
// chiclets: rounded LED blocks on an arc, cut as wedges so they line up like a real LED ring
function segs(){let h='';const r0=88,r1=104,gap=2.2,each=SPAN/SEGS;
 for(let i=0;i<SEGS;i++){const a0=(A0+i*each+gap/2)*Math.PI/180,a1=(A0+(i+1)*each-gap/2)*Math.PI/180,p=(a,r)=>[Math.sin(a)*r,-Math.cos(a)*r];
  const [x0,y0]=p(a0,r1),[x1,y1]=p(a1,r1),[x2,y2]=p(a1,r0),[x3,y3]=p(a0,r0);
  h+=`<path class="nxc-seg" d="M${x0} ${y0}A${r1} ${r1} 0 0 1 ${x1} ${y1}L${x2} ${y2}A${r0} ${r0} 0 0 0 ${x3} ${y3}Z" stroke-linejoin="round"/>`;}return h;}
// The control: one piece. Returns {el,spec,set,setNow,paint}; `set` is what a page calls when the
// live value or the setting changes, `spec.onSet` is called once with the new value when the hand lets go; `spec.onChange` while turning.
// UI rebuild 2026-10-22 (the owner): a press that is lifted at once, without moving, is a TAP: it opens the typed entry (the LED number field), so a value can be entered directly.
// A press that is held (HOLD_MS) or dragged (MOVE_PX) is the KNOB. The knob is always drawn on the page itself, as a copy of the gauge at one fixed, sensible size (HELD_PC /
// HELD_PHONE px), never as the gauge scaled up: its word and its live value are inside the knob and stay readable the whole time it is held, whatever size the gauge was.
const HOLD_MS=220,MOVE_PX=6,HELD_PC=200,HELD_PHONE=250;
export function control(spec={}){install();
 const c={key:'',label:'',unit:'',min:0,max:100,value:0,now:null,step:1,turn:300,color:'var(--accent)',color2:'var(--accent-2)',onSet:null,...spec};
 c.value=snap(c,c.value);
 const el=document.createElement('div');el.className='nxctl';el.style.setProperty('--c',c.color);el.style.setProperty('--c2',c.color2);
 el.setAttribute('role','slider');el.tabIndex=0;el.setAttribute('aria-label',c.label);
 el.setAttribute('aria-valuemin',String(c.min));el.setAttribute('aria-valuemax',String(c.max));
 el.innerHTML=`<svg viewBox="-130 -130 260 260" aria-hidden="true"><g class="nxc-face"><circle r="78" fill="#090c10" stroke="rgba(255,255,255,.06)"/><circle r="72" fill="none" stroke="rgba(255,255,255,.03)"/></g>${segs()}<g class="nxc-now"></g></svg>
 <div class="nxc-knob" aria-hidden="true"><div class="nxc-body"></div><div class="nxc-top"></div><div class="nxc-idx"><i></i></div></div>
 <div class="nxc-read"><b></b>${c.now!==null?'<span class="nxc-n"></span>':''}<span class="nxc-l"></span></div>`;
 // `views` is the gauge, and the held copy of it while it exists: both are painted from the one value.
 const view=r=>({r,segEls:[...r.querySelectorAll('.nxc-seg')],read:r.querySelector('.nxc-read b'),nowT:r.querySelector('.nxc-n'),lab:r.querySelector('.nxc-l'),nowRing:r.querySelector('.nxc-now')});
 const views=[view(el)];views[0].lab.textContent=c.label;
 // U02 (2026-10-20): a page can say "no reading" (a printer that is off) and "locked" (the printer refuses this command); neither is a value, so the gauge shows a dash with no LEDs lit, and a locked one does not grow.
 let unknown=false,locked='';
 // 2026-10-25 ("feel really alive"): the first real reading sweeps the ring to full and back to it, as a car's cluster does when it switches on.
 // Once per gauge per page, a little after the one before it; a person who asks for less motion gets the reading at once.
 let swept=false,sweeping=false;
 function sweep(){swept=true;if(matchMedia('(prefers-reduced-motion: reduce)').matches||!el.isConnected)return false;sweeping=true;const order=SWEEPS++,t0=performance.now()+order*90,D=1000;
  const step=t=>{const k=Math.max(0,Math.min(1,(t-t0)/D)),goal=unknown?0:Math.round(frac(c,c.value)*SEGS),e=k<.5?(k/.5)**.7:1-((k-.5)/.5)**1.6,lit=k<.5?Math.round(SEGS*e):Math.round(goal+(SEGS-goal)*e);
   for(const {segEls} of views)segEls.forEach((x,i)=>{x.classList.toggle('on',i<lit);x.classList.toggle('hot',i===lit-1);});
   if(k<1)requestAnimationFrame(step);else{sweeping=false;paint();}};requestAnimationFrame(step);return true;}
 function paint(){const f=frac(c,c.value),lit=unknown?0:Math.round(f*SEGS),text=unknown?'no reading':shown(c,c.value);
  if(c.sweep&&!swept&&!unknown&&el.isConnected)sweep();
  for(const {r,segEls,read,nowT,nowRing} of views){
   if(!sweeping)segEls.forEach((s,i)=>{s.classList.toggle('on',i<lit);s.classList.toggle('hot',i===lit-1);});
   if(c.now!==null){const a=(A0+SPAN*frac(c,c.now))*Math.PI/180;
    nowRing.innerHTML=`<circle cx="${Math.sin(a)*114}" cy="${-Math.cos(a)*114}" r="3.4"/>`;}
   r.style.setProperty('--a',(A0+SPAN*f)+'deg');
   read.innerHTML=unknown?'<small class="nxc-none">no reading</small>':c.format?c.format(c.value):c.value===0&&c.unit==='°'?'OFF':`${fmt(c,c.value)}<small>${c.unit}</small>`;r.toggleAttribute('data-unknown',unknown);
   if(nowT)nowT.textContent=c.now===null||unknown?'':`NOW ${Math.round(c.now)}${c.unit}`;}
  el.setAttribute('aria-valuenow',String(c.value));el.setAttribute('aria-valuetext',text);}
 // U02 (2026-10-20): the printer or the page moved it: shown, never sent back. A hand that is mid-adjust (a wheel or key step waiting its 600 ms to be sent, or a knob held) is not
 // overwritten by the printer's last reading arriving in between: that sent the value from before the step (found on the part fan: three wheel clicks, command for the old 45%).
 // A value that arrives is shown as it is (a nozzle of 212°), clamped and rounded to the step's decimals: only the knob clicks by whole steps.
 let touched=-1e9;const mark=()=>{touched=performance.now();};
 function set(v){if(el.classList.contains('held')||performance.now()-touched<900)return;const nv=fit(c,v);if(nv===c.value)return;c.value=nv;paint();}
 const setNow=v=>{c.now=v;paint();};
 const setUnknown=v=>{v=!!v;if(unknown!==v){unknown=v;paint();}};
 const setLocked=why=>{locked=why||'';el.toggleAttribute('data-locked',!!locked);};
 // Review 2026-10-20 (Opus): the printer is told once, when the hand lets go (or 600 ms after the last wheel or key step), never once per click;
 // a heater sent fifty set-points in one turn would be fifty commands. `onChange` is the live value while turning, for the display only.
 let lift=null,timer=0;const commit=()=>{clearTimeout(timer);if(c.onSet)c.onSet(c.value,c);};const later=()=>{clearTimeout(timer);timer=setTimeout(commit,600);};
 const nudge=dir=>{const v=bump(c,c.value,dir),stop=v===c.min||v===c.max;if(v!==c.value){mark();c.value=v;paint();click(stop);if(c.onChange)c.onChange(c.value,c);later();}};
 // ---- the knob: a copy of the gauge, drawn on the page at its own size, grown out of the gauge and sprung back into it ----
 const heldSize=()=>innerWidth<SMALL?Math.round(Math.min(innerWidth*.72,HELD_PHONE)):HELD_PC;
 function grab(d){if(d.on)return;d.on=true;clearTimeout(d.timer);
  const r=el.getBoundingClientRect(),S=heldSize(),pad=12,gx=r.left+r.width/2,gy=r.top+r.height/2;
  // a finger would cover the knob where it is, so on a touch screen the knob rises above it; on a computer it opens where the gauge stands; both stay inside the window
  const cx=clamp(gx,S/2+pad,innerWidth-S/2-pad),cy=clamp(d.touch?gy-S*.82:gy,S/2+pad,innerHeight-S/2-pad);
  lift=el.cloneNode(true);lift.classList.remove('nxctl--cell','held');lift.classList.add('nxctl--lift');
  for(const a of ['tabindex','role','data-tip-title','data-tip-does','aria-label','aria-valuenow','aria-valuetext','aria-valuemin','aria-valuemax'])lift.removeAttribute(a);lift.setAttribute('aria-hidden','true');
  Object.assign(lift.style,{position:'fixed',left:cx-S/2+'px',top:cy-S/2+'px',width:S+'px',height:S+'px',margin:'0',zIndex:'41',pointerEvents:'none'});lift.style.setProperty('--nxctl-size',S+'px');
  const k=Math.min(1,r.width/S),from=`translate(${gx-cx}px,${gy-cy}px) scale(${k})`;d.from=from;
  lift.style.transition='none';lift.style.transform=from;document.body.append(lift);views.push(view(lift));lift.getBoundingClientRect();
  lift.style.transition='';lift.style.transform='none';lift.classList.add('live');el.classList.add('held');
  const dim=document.getElementById('nxctl-dim');dim.style.setProperty('--x',cx+'px');dim.style.setProperty('--y',cy+'px');dim.classList.add('on');
  d.cx=cx;d.cy=cy;d.last=null;d.acc=c.value;d.start=c.value;paint();click(true);}
 function release(cancelled){const d=el._d;el._d=null;if(!d||!d.on)return;if(!cancelled&&c.value!==d.start)commit();
  const k=lift;lift=null;if(k){k.classList.remove('live');k.style.transform=d.from;setTimeout(()=>{k.remove();const i=views.findIndex(v=>v.r===k);if(i>0)views.splice(i,1);el.classList.remove('held');},420);}else el.classList.remove('held');
  document.getElementById('nxctl-dim').classList.remove('on');click(true);}
 el.addEventListener('pointerdown',e=>{if(locked||c.readOnly||unknown||(e.pointerType==='mouse'&&e.button!==0))return;el.setPointerCapture(e.pointerId);
  const d=el._d={x:e.clientX,y:e.clientY,on:false,touch:e.pointerType!=='mouse',timer:0};d.timer=setTimeout(()=>{if(el._d===d)grab(d);},HOLD_MS);});
 el.addEventListener('pointermove',e=>{const d=el._d;if(!d)return;
  if(!d.on){if(Math.hypot(e.clientX-d.x,e.clientY-d.y)<MOVE_PX)return;grab(d);}
  const a=Math.atan2(e.clientX-d.cx,-(e.clientY-d.cy));if(d.last===null){d.last=a;return;}let x=a-d.last;
  if(x>Math.PI)x-=2*Math.PI;if(x<-Math.PI)x+=2*Math.PI;d.last=a;
  d.acc=clamp(d.acc+x*180/Math.PI*(c.max-c.min)/c.turn,c.min,c.max);
  const v=snap(c,d.acc),stop=v===c.min||v===c.max;if(v!==c.value){mark();c.value=v;paint();click(stop);if(c.onChange)c.onChange(c.value,c);}});
 el.addEventListener('pointerup',()=>{const d=el._d;if(!d)return;if(d.on)return release(false);clearTimeout(d.timer);el._d=null;tapped();});   // lifted before it was held or dragged: a tap
 el.addEventListener('pointercancel',()=>{const d=el._d;if(!d)return;clearTimeout(d.timer);release(true);el._d=null;});
 el.addEventListener('wheel',e=>{if(locked||c.readOnly||unknown)return;e.preventDefault();nudge(e.deltaY<0?1:-1);},{passive:false});
 // 2026-10-23: a right click on the number types it too (a click does already); the browser's own menu never opens over a gauge.
 el.addEventListener('contextmenu',e=>{e.preventDefault();const d=el._d;if(d){clearTimeout(d.timer);el._d=null;}openEntry();});
 el.addEventListener('keydown',e=>{const k=e.key;let d=0;if(c.readOnly&&k==='Enter'){e.preventDefault();if(c.onTap)c.onTap(c);return;}if(locked||c.readOnly||unknown)return;
  if(k==='Enter'){e.preventDefault();openEntry();return;}
  if(k==='ArrowUp'||k==='ArrowRight')d=1;else if(k==='ArrowDown'||k==='ArrowLeft')d=-1;else if(k==='PageUp')d=5;else if(k==='PageDown')d=-5;
  else if(k==='Home'||k==='End'){e.preventDefault();const v=k==='Home'?c.min:c.max;if(v!==c.value){mark();c.value=v;paint();click(true);later();}return;}else return;
  e.preventDefault();for(let i=0;i<Math.abs(d);i++)nudge(Math.sign(d));});
 // ---- typed entry: a tap opens the LED number field (next-ui numberInput) in a card that grows out of the gauge; Enter sets, Escape cancels, the limits are the control's own ----
 let entry=null;
 // a tap: the page's own wheel when it gives one (a heater's presets), else the typed number
 function tapped(){if(locked||c.readOnly||unknown)return;if(c.onTapWheel){c.onTapWheel(api,{open:openEntry});return;}openEntry();}
 async function openEntry(){if(entry||locked||c.readOnly||unknown)return;entry=true;
  let ui;try{ui=await import('./next-ui.js');}catch{entry=null;return;}
  const r=el.getBoundingClientRect(),small=innerWidth<SMALL,W=Math.min(300,innerWidth-24),unitStep=c.step<1?c.step:1,range=`${fmt(c,c.min)} to ${fmt(c,c.max)}${c.unit}`;
  const card=document.createElement('div');card.className='nxc-entry';card.setAttribute('role','dialog');card.setAttribute('aria-label','Type a value for '+c.label);card.style.width=W+'px';card.style.setProperty('--c',c.color);card.style.setProperty('--c2',c.color2);
  const head=document.createElement('div');head.className='nxc-entry-h';const hb=document.createElement('b');hb.textContent=c.label;const hs=document.createElement('small');hs.textContent='TYPE A VALUE';head.append(hb,hs);
  const field=ui.numberInput({label:'',unit:c.unit,prefix:c.prefix||'',min:c.min,max:c.max,step:unitStep,value:c.value});
  const inp=field.querySelector('input'),note=document.createElement('p');note.className='nxc-entry-n';
  const coarse=matchMedia('(pointer: coarse)').matches,hint=coarse?`${range}  ·  type it, then press SET`:`${range}  ·  ENTER sets  ·  ESC cancels`;   // a phone has no Enter to press or Escape to cancel with
  const say=(t,warn)=>{note.textContent=t;note.toggleAttribute('data-warn',!!warn);};say(hint);
  let raw=null,over=false;inp.addEventListener('input',()=>{const n=parseFloat(String(inp.value).replace(/[^0-9.\-]/g,''));raw=Number.isFinite(n)?n:null;over=false;say(hint);});
  const keys=document.createElement('div');keys.className='nxc-entry-k';
  const close=(set)=>{if(!entry)return;entry=null;document.removeEventListener('pointerdown',away,true);document.removeEventListener('keydown',esc,true);
   document.getElementById('nxctl-dim').classList.remove('on');
   if(set!==undefined&&set!==c.value){mark();c.value=set;paint();click(true);commit();}
   ui.fold(card,r,()=>card.remove());try{el.focus({preventScroll:true});}catch{}};
  const take=()=>{inp.blur();const typed=raw,v=fit(c,field.get());   // the field has already clamped to the limits; a typed number past them is shown clamped and needs a second Enter
   if(typed!==null&&(typed>c.max||typed<c.min)&&!over){over=true;say(`The limit is ${typed>c.max?fmt(c,c.max):fmt(c,c.min)}${c.unit}. Press ${coarse?'SET':'ENTER'} again to set that.`,true);inp.focus();inp.select();return;}
   close(v);};
  const away=e=>{if(!card.contains(e.target))close();};
  const esc=e=>{if(e.key==='Escape'){e.preventDefault();e.stopPropagation();close();}else if(e.key==='Enter'&&card.contains(document.activeElement)&&document.activeElement===inp){e.preventDefault();e.stopPropagation();take();}};
  const cancelKey=key({label:'CANCEL',icon:'close',onPress:()=>close()}),setKey=key({label:'SET',icon:'check',onPress:()=>take()});setKey.el.dataset.kind='primary';
  keys.append(cancelKey.el,setKey.el);card.append(head,field,note,keys);
  // where it opens: over the gauge on a computer; high on a phone, above the on-screen keyboard
  Object.assign(card.style,{position:'fixed',left:'0',top:'0',zIndex:'42',visibility:'hidden'});document.body.append(card);
  const h=card.getBoundingClientRect().height,gx=r.left+r.width/2,gy=r.top+r.height/2;
  card.style.left=clamp(gx-W/2,12,innerWidth-W-12)+'px';card.style.top=(small?Math.max(12,Math.min(innerHeight*.22,innerHeight-h-12)):clamp(gy-h/2,12,innerHeight-h-12))+'px';card.style.visibility='';
  const dim=document.getElementById('nxctl-dim');dim.style.setProperty('--x',gx+'px');dim.style.setProperty('--y',gy+'px');dim.classList.add('on');
  document.addEventListener('pointerdown',away,true);document.addEventListener('keydown',esc,true);
  ui.grow(card,r);click(false);inp.focus();inp.select();}
 // A read-only control (an automatic fan the printer runs by itself) is a gauge that does not turn; a tap on it asks the page for its details.
 if(c.readOnly){el.dataset.readonly='';el.addEventListener('click',()=>{if(c.onTap)c.onTap(c);});}
 const api={el,spec:c,set,setNow,setUnknown,setLocked,paint,isHeld:()=>el.classList.contains('held'),open:()=>openEntry()};paint();return api;}
// The square key: square corners, depth under it, a pressed state that eats the depth, and a click.
// `toggle` latches (aria-pressed); anything else is momentary. Returns {el,spec,setPressed}.
export function key(spec={}){install();
 const k={label:'',icon:'',toggle:false,pressed:false,disabled:false,onPress:null,...spec};
 const b=document.createElement('button');b.type='button';b.className='nxkey';
 if(k.icon)b.innerHTML=`<svg class="nx-ico" aria-hidden="true"><use href="#i-${k.icon}"/></svg>`;
 const lbl=document.createElement('span');lbl.className='nx-lbl';lbl.textContent=k.label;b.append(lbl);
 const setPressed=v=>{k.pressed=v;if(k.toggle)b.setAttribute('aria-pressed',String(!!v));else b.toggleAttribute('data-pressed',!!v);};
 if(k.toggle)setPressed(!!k.pressed);
 if(k.disabled)b.disabled=true;
 b.addEventListener('click',()=>{if(k.toggle)setPressed(!k.pressed);click(false);if(k.onPress)k.onPress(k.toggle?!!k.pressed:null,b);});
 return {el:b,spec:k,setPressed};}
