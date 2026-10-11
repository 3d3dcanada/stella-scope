// Stella Scope NEXT DOM components. No printer logic lives
// here: every component takes plain values and callbacks. Geometry and colour rules come from
// next-core.js (C1), touch math from next-gesture.js (C2).
import {arcPath,dialModel,heatColor,DIAL,polar} from './next-core.js';
import {bindPress,bindHold,bindSwipe,bindLongPress,radialLayout,radialIndex} from './next-gesture.js';

const SVGNS='http://www.w3.org/2000/svg';
export const $=id=>document.getElementById(id);
export function el(tag,cls='',text=''){const e=document.createElement(tag);if(cls)e.className=cls;if(text!=='')e.textContent=text;return e;}
function sv(tag,attrs={}){const e=document.createElementNS(SVGNS,tag);for(const[k,v]of Object.entries(attrs))e.setAttribute(k,v);return e;}
export function icon(id,cls='nx-ico'){const s=sv('svg',{class:cls,'aria-hidden':'true'});s.append(sv('use',{href:'#'+id}));return s;}

// The sprite is fetched once and put in the page so <use href="#i-x"> works under the CSP; the
// dial bezel gradient and glow blur live beside it so every dial shares one definition.
export async function loadSprite(){
 const text=await (await fetch('/next-icons.svg')).text();
 const host=el('div');host.hidden=true;host.setAttribute('aria-hidden','true');
 host.innerHTML=text.replace('<svg ','<svg width="0" height="0" ');
 const defs=sv('svg',{width:0,height:0});defs.innerHTML='<defs><linearGradient id="nx-bezel" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="rgba(255,255,255,.28)"/><stop offset=".5" stop-color="rgba(255,255,255,.05)"/><stop offset="1" stop-color="rgba(255,255,255,.14)"/></linearGradient><filter id="nx-blur" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="5"/></filter><linearGradient id="nx-gbezel" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5b6570"/><stop offset=".35" stop-color="#2a3139"/><stop offset=".7" stop-color="#171b21"/><stop offset="1" stop-color="#3a434d"/></linearGradient><radialGradient id="nx-gface" cx=".5" cy=".38" r=".7"><stop offset="0" stop-color="#1c2430"/><stop offset=".6" stop-color="#10151c"/><stop offset="1" stop-color="#080b0f"/></radialGradient><radialGradient id="nx-ghub" cx=".35" cy=".3" r=".8"><stop offset="0" stop-color="#9aa6b2"/><stop offset=".5" stop-color="#3b444e"/><stop offset="1" stop-color="#14181d"/></radialGradient><linearGradient id="nx-gsheen" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".10"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient></defs>';
 host.append(defs);document.body.prepend(host);
}

// ---------- 6.10 tooltip: what it is, what it does, why it is locked ----------
const tipBox=()=>$('nx-tip');let tipTimer=0,tipFor=null;
export function tip(node,title,does='',why=''){node.dataset.tipTitle=title;node.dataset.tipDoes=does;if(why)node.dataset.tipWhy=why;else delete node.dataset.tipWhy;
 if(!node.getAttribute('aria-label'))node.setAttribute('aria-label',title);return node;}
export function setWhy(node,why){if(why)node.dataset.tipWhy=why;else delete node.dataset.tipWhy;if(tipFor===node)showTip(node);}
function showTip(node){const box=tipBox();if(!box||!node.dataset.tipTitle)return;box.replaceChildren(el('b','',node.dataset.tipTitle));
 if(node.dataset.tipDoes)box.append(el('span','',node.dataset.tipDoes));if(node.dataset.tipWhy)box.append(el('em','',node.dataset.tipWhy));
 box.className='nx-tip nx-glass nx-glass--raised';box.hidden=false;const r=node.getBoundingClientRect(),b=box.getBoundingClientRect();
 let x=r.left+r.width/2-b.width/2,y=r.top-b.height-10;if(y<8)y=r.bottom+10;x=Math.max(8,Math.min(innerWidth-b.width-8,x));
 box.style.left=x+'px';box.style.top=y+'px';tipFor=node;}
export function hideTip(){clearTimeout(tipTimer);const b=tipBox();if(b)b.hidden=true;tipFor=null;}
export function initTips(){
 // A pressed control keeps quiet until the pointer leaves it: opening a sheet re-fires pointerover
 // on what is under the pointer, and the card then sat over the sheet it had just opened.
 let pressed=null;document.addEventListener('pointerdown',e=>{pressed=e.target.closest?.('[data-tip-title]')||null;},true);
 document.addEventListener('pointerover',e=>{if(e.pointerType!=='mouse')return;const n=e.target.closest?.('[data-tip-title]');if(n===tipFor)return;hideTip();if(pressed&&n!==pressed)pressed=null;if(n&&n!==pressed)tipTimer=setTimeout(()=>showTip(n),400);});
 document.addEventListener('pointerout',e=>{if(!e.relatedTarget||!e.relatedTarget.closest?.('[data-tip-title]'))hideTip();});
 for(const ev of ['pointerdown','wheel','keydown','scroll'])document.addEventListener(ev,hideTip,true);
 // A card must not outlive what it describes: a control rebuilt under the pointer (knob to bar, a
 // dock that changed with the printer's state) never sends pointerout, and its card stayed up.
 setInterval(()=>{if(tipFor&&(!tipFor.isConnected||!tipFor.matches(':hover')))hideTip();},400);
 // Touch (6.10): a 500 ms press shows the same card; moving 10 px or lifting cancels it.
 let lp=null;document.addEventListener('pointerdown',e=>{if(e.pointerType==='mouse')return;const n=e.target.closest?.('[data-tip-title]');if(!n)return;
  lp={x:e.clientX,y:e.clientY,t:setTimeout(()=>{showTip(n);lp=null;},500)};},true);
 const stop=e=>{if(lp&&(e.type!=='pointermove'||Math.hypot(e.clientX-lp.x,e.clientY-lp.y)>10)){clearTimeout(lp.t);lp=null;}};
 for(const ev of ['pointerup','pointercancel','pointermove'])document.addEventListener(ev,stop,true);
}

// ---------- 6.1 button ----------
// kind: default|primary|danger|ghost. size: dock|row|icon|tile. cmd: the printer command it needs,
// so lockControls() can disable it with the reason. hold: press-and-hold to confirm (6.1).
export function button({icon:ico,label='',kind='default',size='row',cmd=null,onPress,hold=false,attention=false,title,does='',sub=null}={}){
 const b=el('button','nx-btn');b.type='button';b.dataset.kind=kind;b.dataset.size=size;if(cmd)b.dataset.cmd=cmd;if(attention)b.dataset.attention='';
 if(ico)b.append(icon(ico));if(label&&size!=='icon')b.append(el('span','nx-lbl',label));if(sub!==null){const s=el('span','nx-sub',sub);b.append(s);b.sub=s;}
 tip(b,title||label,does);bindPress(b);
 if(hold){b.dataset.hold='';b.append(el('i','nx-hold'));bindHold(b,{ms:900,onConfirm:()=>onPress?.(b),onCancel:()=>{}});
  b.dataset.tipDoes=(does?does+' ':'')+'Press and hold to confirm.';}
 else if(onPress)b.addEventListener('click',()=>{if(!b.disabled&&b.getAttribute('aria-disabled')!=='true')onPress(b);});
 return b;}
export function busy(b,on){if(on)b.dataset.busy='';else delete b.dataset.busy;}

// ---------- 6.2 dial ----------
const R=76,RP=61;
function ticks(g){for(let i=0;i<=50;i++){const a=DIAL.start+DIAL.sweep*i/50,major=i%5===0,p0=polar(100,100,major?83:85.5,a),p1=polar(100,100,89,a);
 g.append(sv('line',{x1:p0.x,y1:p0.y,x2:p1.x,y2:p1.y,class:major?'tick major':'tick'}));}}
// kind 'thermal': value = temperature, colour from heatColor; 'speed' and 'flow': value on a
// neutral/accent arc. model: {value,target,power,min,max,heatKind,tone,sub,unit}
export function dial({label,icon:ico,unit='',min=0,max=100,kind='thermal',heatKind='nozzle',onOpen,onDrag}={}){
 const root=el('div','nx-dial');root.setAttribute('role','button');root.tabIndex=0;root.dataset.kind=kind;
 const s=sv('svg',{viewBox:'0 0 200 200'});
 s.append(sv('circle',{cx:100,cy:100,r:96,class:'bezel'}));const tg=sv('g');ticks(tg);s.append(tg);
 const track=sv('path',{class:'track',d:arcPath(100,100,R,DIAL.start,DIAL.start+DIAL.sweep)});
 const ptrack=sv('path',{class:'power-track',d:arcPath(100,100,RP,DIAL.start,DIAL.start+DIAL.sweep)});
 const glow=sv('path',{class:'glow'}),value=sv('path',{class:'value'}),ghost=sv('path',{class:'ghost'}),power=sv('path',{class:'power'});
 const target=sv('rect',{class:'target',x:100+R-8,y:98,width:16,height:4,rx:2}),dot=sv('circle',{class:'attemp',r:3.4,cx:100+R+13,cy:100});
 const tgt=sv('g');tgt.append(target,dot);
 s.append(ptrack,power,track,glow,value,ghost,tgt);
 const center=el('div','nx-dial-center'),val=el('div','nx-dial-value','—'),lab=el('div','nx-dial-label'),sub=el('div','nx-dial-sub');
 if(ico)lab.append(icon(ico));lab.append(label);center.append(val,lab,sub);root.append(s,center);
 let shown=null,goal=null,anim=0,model={},shownValue=null,goalValue=null;const finiteNum=v=>typeof v==='number'&&Number.isFinite(v);
 const draw=f=>{const d=f===null||f<=0.0005?'':arcPath(100,100,R,DIAL.start,DIAL.start+DIAL.sweep*f);value.setAttribute('d',d);glow.setAttribute('d',d);};
 // The arc sweeps to a new reading over 600 ms (4) instead of jumping; reduced motion snaps.
 const sweep=()=>{cancelAnimationFrame(anim);if(goal===null||shown===null||matchMedia('(prefers-reduced-motion: reduce)').matches){shown=goal;draw(shown);return;}
  const from=shown,to=goal,t0=performance.now(),v0=shownValue,v1=goalValue,counting=finiteNum(v0)&&finiteNum(v1)&&Math.abs(v1-v0)>=2&&model.text===undefined;
  const step=t=>{const k=Math.min(1,(t-t0)/600),e=1-Math.pow(1-k,3);shown=from+(to-from)*e;draw(shown);
   if(counting){const v=Math.round(v0+(v1-v0)*e);if(val.firstChild&&val.firstChild.nodeType===3&&val.firstChild.data!==String(v))val.firstChild.data=String(v);}
   if(k<1)anim=requestAnimationFrame(step);else shownValue=v1;};anim=requestAnimationFrame(step);};
 function set(m){model=m;const lo=m.min??min,hi=m.max??max,dm=dialModel({value:m.value,min:lo,max:hi,target:m.target,power:m.power});
  const unknown=dm.frac===null;root.toggleAttribute('data-unknown',unknown);
  const text=unknown?'—':String(m.text??Math.round(m.value));
  val.replaceChildren(text);if(!unknown&&(m.unit??unit))val.append(el('small','',m.unit??unit));
  let ink='var(--ink-2)',g=0;
  if(kind==='thermal'){const h=heatColor(m.value,m.target,heatKind);ink=h.tone==='unknown'?'var(--ink-3)':h.ink;g=h.glow;
   const heating=m.target>0&&m.value!==null&&Math.abs(m.value-m.target)>3,at=m.target>0&&m.value!==null&&Math.abs(m.value-m.target)<=3;
   root.toggleAttribute('data-heating',heating);dot.style.display=at?'':'none';}
  else{ink=m.tone==='over'?'var(--alarm)':m.tone==='high'?'var(--heat)':'var(--accent)';g=m.tone==='over'?1:.35;dot.style.display='none';}
  root.style.setProperty('--arc',ink);root.style.setProperty('--glow',g);
  if(dm.targetAngle!==null&&m.target>0){tgt.style.display='';tgt.setAttribute('transform',`rotate(${dm.targetAngle} 100 100)`);}else tgt.style.display='none';
  power.setAttribute('d',dm.powerFrac>0.005?arcPath(100,100,RP,DIAL.start,DIAL.start+DIAL.sweep*dm.powerFrac):'');
  sub.replaceChildren();if(m.sub)for(const part of [].concat(m.sub)){if(typeof part==='string')sub.append(part);else sub.append(el('b','',part.b));}
  const prevValue=goalValue;goalValue=unknown?null:m.value;shownValue=prevValue;
  goal=unknown?null:dm.frac;if(shown===null){shown=goal;draw(shown);shownValue=goalValue;}else if(goal!==shown||goalValue!==prevValue)sweep();}
 // Drag on the arc sets a pending value (ghost arc); release hands it to onDrag. A tap opens.
 let drag=null;
 const fracAt=e=>{const r=root.getBoundingClientRect(),x=e.clientX-r.left-r.width/2,y=e.clientY-r.top-r.height/2;
  let a=Math.atan2(y,x)*180/Math.PI;if(a<0)a+=360;let rel=a-DIAL.start;if(rel<0)rel+=360;if(rel>DIAL.sweep)rel=rel>DIAL.sweep+45?0:DIAL.sweep;return rel/DIAL.sweep;};
 root.addEventListener('pointerdown',e=>{if(e.button!==0)return;const r=root.getBoundingClientRect(),d=Math.hypot(e.clientX-r.left-r.width/2,e.clientY-r.top-r.height/2)/(r.width/2);
  drag={x:e.clientX,y:e.clientY,ring:d>.62&&!!onDrag,moved:false};(()=>{try{root.setPointerCapture(e.pointerId);}catch{}})();root.dataset.pressed='';});
 root.addEventListener('pointermove',e=>{if(!drag)return;if(Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>6)drag.moved=true;if(!drag.ring||!drag.moved)return;
  const f=fracAt(e),lo=model.min??min,hi=model.max??max,v=lo+(hi-lo)*f;drag.value=onDrag.round?onDrag.round(v):Math.round(v);
  ghost.setAttribute('d',arcPath(100,100,R,DIAL.start,DIAL.start+DIAL.sweep*((drag.value-lo)/(hi-lo))||0.001));val.replaceChildren(String(drag.value));val.append(el('small','',model.unit??unit));});
 const end=commit=>{if(!drag)return;const d=drag;drag=null;delete root.dataset.pressed;ghost.setAttribute('d','');
  if(commit&&d.ring&&d.moved&&d.value!==undefined)onDrag(d.value);else if(commit&&!d.moved)onOpen?.();set(model);};
 root.addEventListener('pointerup',()=>end(true));root.addEventListener('pointercancel',()=>end(false));
 root.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onOpen?.();}});
 tip(root,label,onDrag?'Tap for details and presets. Drag around the ring to set a new target.':'Tap for details.');
 return {el:root,set};}

// ---------- fan ring ---------- Fan ring. With onSet the ring is a control: drag round it (top
// is 0, clockwise to 100 %), it snaps to 5 % and sends on release; a tap opens its panel.
// Without onSet (automatic fans) a tap opens the panel only.
export function fanRing({label,onOpen,onSet=null,title=null,does=null}={}){
 const b=el('button','nx-fan');b.type='button';const disc=el('div','nx-fan-disc'),C=2*Math.PI*33;
 const s=sv('svg',{class:'ring',viewBox:'0 0 76 76'});const tr=sv('circle',{cx:38,cy:38,r:33,class:'track'}),fl=sv('circle',{cx:38,cy:38,r:33,class:'fill','stroke-dasharray':C,'stroke-dashoffset':C});
 s.append(tr,fl);disc.append(s,icon('i-fan'));const pct=el('div','nx-fan-pct','—'),name=el('div','nx-fan-name',label);b.append(disc,pct,name);
 if(onSet){b.dataset.settable='';const knob=el('i','nx-fan-handle');disc.append(knob);b.knob=knob;}
 bindPress(b);let model={speed:null},pending=null,drag=null;
 const paint=()=>{const v=pending??model.speed,f=v===null||v===undefined?null:Math.max(0,Math.min(1,v));
  fl.setAttribute('stroke-dashoffset',f===null?C:C*(1-f));pct.textContent=f===null?'—':Math.round(f*100)+'%';b.toggleAttribute('data-pending',pending!==null);
  b.toggleAttribute('data-on',(model.speed??0)>0.01);if((model.speed??0)>0.01)b.style.setProperty('--spin',(0.35+(1-model.speed)*1.6).toFixed(2)+'s');
  if(b.knob){const a=(f??0)*2*Math.PI;b.knob.style.left=(50+43.4*Math.sin(a))+'%';b.knob.style.top=(50-43.4*Math.cos(a))+'%';b.knob.hidden=f===null&&pending===null;}};
 const at=e=>{const r=disc.getBoundingClientRect();let a=Math.atan2(e.clientX-r.left-r.width/2,-(e.clientY-r.top-r.height/2));if(a<0)a+=2*Math.PI;return Math.round(a/(2*Math.PI)*20)/20;};
 if(onSet){b.addEventListener('pointerdown',e=>{if(e.button!==0||b.disabled||b.hasAttribute('data-locked'))return;drag={x:e.clientX,y:e.clientY,moved:false};try{b.setPointerCapture(e.pointerId);}catch{}});
  b.addEventListener('pointermove',e=>{if(!drag)return;if(!drag.moved&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)<5)return;drag.moved=true;pending=at(e);paint();});
  b.addEventListener('pointerup',()=>{if(!drag)return;const d=drag;drag=null;if(d.moved&&pending!==null){const v=pending;Promise.resolve(onSet(v)).then(ok=>{if(!ok)pending=null;paint();});setTimeout(()=>{pending=null;paint();},6000);}else if(!d.moved)onOpen?.();});
  b.addEventListener('pointercancel',()=>{drag=null;pending=null;paint();});}
 else if(onOpen)b.addEventListener('click',onOpen);
 tip(b,title||label+' fan',does||'The speed Klipper was told to run, as a percent. No tachometer is fitted, so this is the command, not a measured RPM.');
 function set({speed=null,rpm=null}={}){model={speed,rpm};if(pending!==null&&speed!==null&&Math.abs(speed-pending)<0.02)pending=null;paint();}
 return {el:b,set,lock(why){b.toggleAttribute('data-locked',!!why);setWhy(b,why||'');}};}

// ---------- 6.3 dock ----------
// slots from next-core dockFor(); each group sits in its own glass pod; handlers.press(slot, button).
export function renderDock(host,slots,{press,message}={}){
 host.replaceChildren();host.className='nx-dock';
 if(message){host.classList.add('nx-glass');const m=el('div','nx-dock-msg');m.append(icon(message.icon||'i-warn'),el('span','',message.text));host.append(m);return [];}
 const out=[];let group=null,pod=null;
 for(const s of slots.filter(x=>x.visible!==false)){if(pod===null||s.group!==group){pod=el('div','nx-dock-pod nx-glass');host.append(pod);}group=s.group;
  const b=button({icon:s.icon,label:s.label,size:'dock',kind:s.primary?'primary':s.danger?'danger':'default',cmd:s.cmd,hold:s.hold,attention:s.attention,
   title:s.title||s.label.charAt(0)+s.label.slice(1).toLowerCase(),does:s.does||'',onPress:btn=>press(s,btn)});
  b.style.setProperty("animation-delay",(out.length*24)+"ms");b.nxSlot=s;b.dataset.key=s.key;if(/^(menu|sheet|radial):/.test(s.action||"")){const c=el("i","nx-caret");c.setAttribute("aria-hidden","true");b.append(c);b.dataset.opens=s.action.split(":")[0];}if(s.primary)pod.dataset.primary='';pod.append(b);out.push(b);}
 return out;}

// ---------- 6.4 sheet ----------
let sheetClose=null;
// A dock button that opened a sheet, pop-up or radial carries data-showing while it is open, so a
// second press on it closes the thing instead of opening it again.
export function markOpener(btn){clearOpener();btn?.setAttribute('data-showing','');}
export function clearOpener(){for(const b of document.querySelectorAll('[data-showing]'))b.removeAttribute('data-showing');}
export function openSheet({title,icon:ico='i-more',build,onClose,wide=false,eyebrow=''}){
 const host=$('nx-sheet');closeSheet(true);host.className='nx-sheet nx-glass nx-glass--raised'+(wide?' nx-sheet--wide':'');host.hidden=false;host.replaceChildren();
 const grip=el('div','nx-sheet-grip');grip.append(el('i'));
 const head=el('div','nx-sheet-head');const x=()=>button({icon:'i-close',size:'icon',kind:'ghost',title:'Close',does:'Closes this panel.',onPress:()=>closeSheet()});const tt=el('div','nx-sheet-title',title);
 // Panels drag, resize, pin. Drag the header to float it anywhere (the corner resizes it); Dock
 // puts it back on the right; Pin keeps it on screen while other panels open.
 const tools=el('div','nx-sheet-tools');tools.append(button({icon:'i-dock-right',size:'icon',kind:'ghost',title:'Dock to the side',does:'Puts this panel back on the right edge.',onPress:()=>{panelDock(host,title);}}),
  button({icon:'i-pin',size:'icon',kind:'ghost',title:'Pin',does:'Keeps this panel on screen while you open others. Drag it by its header, resize it from the corner.',onPress:()=>pinSheet(host,title)}));
 if(eyebrow){const w=el('div','nx-sheet-titles');w.append(el('div','nx-sheet-eyebrow',eyebrow),tt);head.append(icon(ico),w,tools,x());}else head.append(icon(ico),tt,tools,x());
 bindPanelDrag(host,head,title);
 const body=el('div','nx-sheet-body');host.append(grip,head,body);
 const unbind=bindSwipe(host,grip,{onDismiss:()=>closeSheet()});
 const esc=e=>{if(e.key==='Escape')closeSheet();};document.addEventListener('keydown',esc);
 sheetClose=()=>{unbind?.();document.removeEventListener('keydown',esc);onClose?.();};
 build(body,host);document.body.dataset.sheet=wide?'wide':'';requestAnimationFrame(()=>requestAnimationFrame(()=>host.dataset.open=''));host.setAttribute('aria-label',title);
 return {body,host,close:closeSheet};}
// ---------- floating, resizable and pinned panels ----------
const PANEL_KEY=t=>'nx-panel-'+t;
const floatable=()=>!['phone','phone-land','tablet-port','kiosk'].includes(document.body.dataset.layout);
function panelFloat(host,g){host.dataset.float='';Object.assign(host.style,{left:g.x+'px',top:g.y+'px',width:g.w+'px',height:g.h+'px',right:'auto',bottom:'auto'});}
function panelClamp(host){const r=host.getBoundingClientRect(),x=Math.max(8,Math.min(innerWidth-120,r.left)),y=Math.max(8,Math.min(innerHeight-60,r.top));host.style.left=x+'px';host.style.top=y+'px';}
function panelSave(host,title){const r=host.getBoundingClientRect();try{localStorage.setItem(PANEL_KEY(title),JSON.stringify({x:Math.round(r.left),y:Math.round(r.top),w:Math.round(r.width),h:Math.round(r.height)}));}catch{}}
function panelDock(host,title){delete host.dataset.float;for(const k of ['left','top','width','height','right','bottom'])host.style.removeProperty(k);try{localStorage.removeItem(PANEL_KEY(title));}catch{}}
function bindPanelDrag(host,head,title){
 try{const g=JSON.parse(localStorage.getItem(PANEL_KEY(title))||'null');if(g&&floatable())panelFloat(host,{x:Math.min(g.x,innerWidth-200),y:Math.min(g.y,innerHeight-120),w:Math.min(g.w,innerWidth-16),h:Math.min(g.h,innerHeight-16)});else panelDock(host,title);}catch{}
 head.addEventListener('pointerdown',e=>{if(!floatable()||e.button!==0||e.target.closest('.nx-btn,input,button'))return;const r=host.getBoundingClientRect(),dx=e.clientX-r.left,dy=e.clientY-r.top;
  if(!('float' in host.dataset))panelFloat(host,{x:r.left,y:r.top,w:r.width,h:r.height});try{head.setPointerCapture(e.pointerId);}catch{}host.dataset.dragging='';
  const mv=ev=>{host.style.left=(ev.clientX-dx)+'px';host.style.top=(ev.clientY-dy)+'px';};
  const up=()=>{head.removeEventListener('pointermove',mv);delete host.dataset.dragging;panelClamp(host);panelSave(host,title);};
  head.addEventListener('pointermove',mv);head.addEventListener('pointerup',up,{once:true});head.addEventListener('pointercancel',up,{once:true});});
 // a resize from the corner is remembered too
 let t=0;host._ro?.disconnect();host._ro=new ResizeObserver(()=>{if(!('float' in host.dataset)||host.hidden)return;clearTimeout(t);t=setTimeout(()=>panelSave(host,title),300);});host._ro.observe(host);}
let pinCount=0;
function pinSheet(host,title){if(host.id!=='nx-sheet')return;const r=host.getBoundingClientRect(),docked=!('float' in host.dataset);
 // a docked panel steps left out of the way of the next one
 panelFloat(host,{x:Math.max(8,docked?r.left-r.width-16-24*(pinCount%4):r.left),y:r.top+(docked?24*(pinCount%4):0),w:r.width,h:Math.min(r.height,innerHeight-40)});
 const close=sheetClose;sheetClose=null;host.id='nx-pin-'+(++pinCount);host.dataset.pinned='';clearOpener();delete document.body.dataset.sheet;
 const fresh=el('aside','nx-sheet');fresh.id='nx-sheet';fresh.setAttribute('aria-modal','true');fresh.setAttribute('role','dialog');fresh.hidden=true;host.after(fresh);
 // its close button now closes this pinned panel only
 const xBtn=host.querySelector('.nx-sheet-head > .nx-btn:last-child');if(xBtn){const nx=xBtn.cloneNode(true);xBtn.replaceWith(nx);nx.addEventListener('click',()=>{close?.();host.remove();});}
 host.querySelector('.nx-sheet-tools')?.remove();host.style.zIndex=String(6+pinCount);panelSave(host,title);}
export function closeSheet(instant=false){const host=$('nx-sheet');if(!host||host.hidden)return;const done=sheetClose;sheetClose=null;done?.();if(!instant)clearOpener();
 delete host.dataset.open;delete document.body.dataset.sheet;host.style.removeProperty('--drag');if(instant){host.hidden=true;return;}setTimeout(()=>{if(!host.dataset.open)host.hidden=true;},260);}
export function section(title,...children){const s=el('section','nx-section');if(title)s.append(el('h3','',title));s.append(...children);return s;}

// ---------- 6.5 radial ----------
export function openRadial(items,{x,y,onPick}){
 const host=$('nx-radial');host.hidden=false;host.replaceChildren();host.className='nx-radial';
 const touch=matchMedia('(pointer: coarse)').matches,rad=Math.min(touch?128:150,innerWidth/2-52,innerHeight/2-70),layout=radialLayout(items.length,rad);
 const cx=Math.max(rad+46,Math.min(innerWidth-rad-46,x)),cy=Math.max(rad+60,Math.min(innerHeight-rad-80,y));
 const hub=el('div','nx-radial-hub');hub.style.left=cx+'px';hub.style.top=cy+'px';
 const close=()=>{clearOpener();host.hidden=true;host.replaceChildren();document.removeEventListener('keydown',esc);};
 const esc=e=>{if(e.key==='Escape')close();};document.addEventListener('keydown',esc);
 const btns=items.map((it,i)=>{const b=button({icon:it.icon,label:it.label,size:'dock',kind:it.danger?'danger':'default',cmd:it.cmd,hold:it.hold,title:it.title||it.label,does:it.does||'',onPress:()=>{close();onPick(it);}});
  if(it.sub){const sb=el('span','nx-radial-sub',it.sub);b.append(sb);}if(it.tone)b.dataset.tone=it.tone;
  b.style.left=layout[i].x+'px';b.style.top=layout[i].y+'px';b.style.setProperty('--i',i);b.style.setProperty('--x',layout[i].x+'px');b.style.setProperty('--y',layout[i].y+'px');hub.append(b);return b;});
 const x0=button({icon:'i-close',size:'icon',kind:'default',title:'Close',does:'Closes this menu.',onPress:close});x0.classList.add('nx-radial-close');hub.append(x0);
 host.append(hub);host.addEventListener('pointerdown',e=>{if(e.target===host)close();},{once:true});
 // Touch: press-drag-release from the hub selects by direction (Hopkins' pie menu, 1.5).
 let tracking=false;host.addEventListener('pointermove',e=>{if(e.pointerType==='mouse'&&!tracking)return;const i=radialIndex(e.clientX-cx,e.clientY-cy,items.length);btns.forEach((b,k)=>b.toggleAttribute('data-hot',k===i));});
 return {close,start:()=>{tracking=true;},release:e=>{if(!tracking)return;tracking=false;const i=radialIndex(e.clientX-cx,e.clientY-cy,items.length);if(i>=0&&!btns[i].disabled&&!items[i].hold){close();onPick(items[i]);}},btns};}

// ---------- 6.9 toast ----------
export function toast(text,tone='info'){const host=$('nx-toasts');if(!host||!text)return;const t=el('div','nx-toast nx-glass nx-glass--raised');t.dataset.tone=tone;
 t.append(icon(tone==='alarm'?'i-warn':tone==='ok'?'i-check':'i-live'),el('span','',text));host.append(t);while(host.children.length>2)host.firstChild.remove();
 setTimeout(()=>t.remove(),tone==='alarm'?7000:4000);}

// ---------- small widgets used by sheets ----------
export function segmented(values,current,onPick,fmt=v=>String(v)){const s=el('div','nx-seg');for(const v of values){const b=el('button','',fmt(v));b.type='button';b.setAttribute('aria-pressed',String(v===current));
 b.addEventListener('click',()=>{for(const x of s.children)x.setAttribute('aria-pressed','false');b.setAttribute('aria-pressed','true');onPick(v);});s.append(b);}return s;}
export function stepper({value,unit='',step=5,min=0,max=300,onChange}){const w=el('div','nx-stepper');const v=el('div','nx-stepper-value');let cur=value;
 const paint=()=>{v.replaceChildren(String(cur));v.append(el('small','',unit));};paint();
 const minus=button({icon:'i-minus',size:'icon',title:'Lower',does:`Lowers by ${step}${unit}.`,onPress:()=>{cur=Math.max(min,cur-step);paint();onChange?.(cur);}});
 const plus=button({icon:'i-plus',size:'icon',title:'Raise',does:`Raises by ${step}${unit}.`,onPress:()=>{cur=Math.min(max,cur+step);paint();onChange?.(cur);}});
 w.append(minus,v,plus);w.get=()=>cur;w.setValue=x=>{cur=x;paint();};return w;}
export {bindLongPress};

// ---------- instrument gauge ---------- A real instrument: machined bezel, numbered scale over a
// 240° sweep, value band, red zone, a tapered needle on a hub, digital readout under the hub,
// glass sheen. The needle rotates with a CSS transition, so it eases between readings without a
// JS animation loop.
const G0=150,GS=240;
const gAng=f=>G0+GS*Math.max(0,Math.min(1,f));
export function gauge({label,icon:ico,unit='',min=0,max=100,major=6,redFrom=null,onOpen,title,does}={}){
 const root=el('div','nx-gauge');root.setAttribute('role','button');root.tabIndex=0;
 const s=sv('svg',{viewBox:'0 0 220 220'}),C=110;
 s.append(sv('circle',{cx:C,cy:C,r:106,class:'g-bezel'}),sv('circle',{cx:C,cy:C,r:100,class:'g-face'}));
 const band=sv('path',{class:'g-track',d:arcPath(C,C,90,G0,G0+GS)}),red=sv('path',{class:'g-red'}),fill=sv('path',{class:'g-fill'});
 s.append(band,red,fill);const scale=sv('g',{class:'g-scale'}),nums=sv('g',{class:'g-nums'});s.append(scale,nums);
 const needle=sv('g',{class:'g-needle'});needle.append(sv('path',{d:`M${C-3.2} ${C+14} L${C-1.1} ${C-80} L${C} ${C-84} L${C+1.1} ${C-80} L${C+3.2} ${C+14} Z`}));
 s.append(needle,sv('circle',{cx:C,cy:C,r:8,class:'g-hub'}),sv('circle',{cx:C,cy:C,r:3,class:'g-hub-dot'}),sv('path',{class:'g-sheen',d:'M28 98 A84 84 0 0 1 192 98 Q110 70 28 98 Z'}));
 const read=el('div','nx-gauge-read'),val=el('div','nx-gauge-value','—'),lab=el('div','nx-gauge-label');if(ico)lab.append(icon(ico));lab.append(label);
 const sub=el('div','nx-gauge-sub');read.append(val,lab,sub);root.append(s,read);
 let lo=min,hi=max,shownV=null,anim=0;
 function drawScale(){scale.replaceChildren();nums.replaceChildren();const n=major*5;
  for(let i=0;i<=n;i++){const a=G0+GS*i/n,maj=i%5===0,p0=polar(C,C,maj?76:80,a),p1=polar(C,C,85,a);scale.append(sv('line',{x1:p0.x,y1:p0.y,x2:p1.x,y2:p1.y,class:maj?'maj':''}));
   if(maj){const v=lo+(hi-lo)*i/n,p=polar(C,C,64,a),t=sv('text',{x:p.x,y:p.y+4});t.textContent=Number.isInteger(v)?String(v):v.toFixed(1);nums.append(t);}}
  red.setAttribute('d',redFrom!==null&&redFrom<hi?arcPath(C,C,90,gAng((redFrom-lo)/(hi-lo)),G0+GS):'');}
 drawScale();
 function set({value=null,max:m=null,text=null,sub:sb='',tone='normal'}={}){
  if(m&&m!==hi){hi=m;drawScale();}
  const f=value===null?null:(value-lo)/(hi-lo);root.toggleAttribute('data-unknown',f===null);root.dataset.tone=tone;
  needle.style.setProperty('transform',`rotate(${(f===null?G0:gAng(f))-270}deg)`);
  fill.setAttribute('d',f===null||f<=0.002?'':arcPath(C,C,90,G0,gAng(f)));
  cancelAnimationFrame(anim);const target=value;
  if(value===null){val.textContent='—';shownV=null;}
  else if(text!==null&&shownV===null){val.textContent=text;shownV=value;}
  else{const from=shownV??value,t0=performance.now(),dec=text&&text.includes('.')?1:0;
   const step=t=>{const k=Math.min(1,(t-t0)/600),e=1-Math.pow(1-k,3),v=from+(target-from)*e;val.textContent=v.toFixed(dec);if(k<1)anim=requestAnimationFrame(step);else{shownV=target;if(text!==null)val.textContent=text;}};anim=requestAnimationFrame(step);}
  if(unit&&value!==null)val.append(el('small','',unit));
  sub.replaceChildren();for(const p of [].concat(sb||[]))sub.append(typeof p==='string'?p:el('b','',p.b));}
 bindPress(root);root.addEventListener('click',()=>onOpen?.());root.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onOpen?.();}});
 tip(root,title||label,does||'Tap for its controls and presets.');return {el:root,set};}

// ---------- thermal card with a live, self-scaling graph ----------
// Temperature against target over the last minutes, the line in the heat colour with a soft area
// under it, the target dashed, the y axis rescaling to what was actually recorded (never zero-based
// when nothing near zero happened). History comes from the recording and then from every tick.
export function thermoCard({label,icon:ico,heatKind='nozzle',span=600,onOpen}={}){
 const root=el('div','nx-thermo');root.setAttribute('role','button');root.tabIndex=0;
 const head=el('div','nx-thermo-head'),name=el('div','nx-thermo-name');if(ico)name.append(icon(ico));name.append(label);
 const pill=el('div','nx-thermo-pill','OFF');head.append(name,pill);
 const row=el('div','nx-thermo-row'),val=el('div','nx-thermo-value','—'),tgt=el('div','nx-thermo-target');row.append(val,tgt);
 const W=280,H=74,chart=sv('svg',{class:'nx-thermo-chart',viewBox:`0 0 ${W} ${H}`,preserveAspectRatio:'none'});
 const gid='g'+Math.random().toString(36).slice(2,8);const defs=sv('defs');defs.innerHTML=`<linearGradient id="${gid}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="currentColor" stop-opacity=".32"/><stop offset="1" stop-color="currentColor" stop-opacity="0"/></linearGradient>`;
 const grid=sv('g',{class:'grid'});for(const y of [H*.25,H*.5,H*.75])grid.append(sv('line',{x1:0,x2:W,y1:y,y2:y}));
 const area=sv('path',{class:'area',fill:`url(#${gid})`}),line=sv('path',{class:'line'}),tline=sv('path',{class:'tline'});
 chart.append(defs,grid,area,tline,line);
 const axis=el('div','nx-thermo-axis'),aHi=el('span'),aLo=el('span'),aSpan=el('span','',`${Math.round(span/60)} min`);axis.append(aSpan,aLo,aHi);
 const power=el('div','nx-thermo-power'),pf=el('i');power.append(pf);
 const chartWrap=el('div','nx-thermo-graph');chartWrap.append(chart,axis);
 root.append(head,row,chartWrap,power);
 function set({temp=null,target=null,power:pw=null,history=[],stale=false}={}){
  const h=heatColor(temp,target,heatKind);root.style.setProperty('--ink',h.tone==='unknown'?'var(--ink-3)':h.ink);root.dataset.tone=h.tone;
  val.replaceChildren(temp===null?'—':String(Math.round(temp)));if(temp!==null)val.append(el('small','','°C'));
  tgt.replaceChildren();if(target>0){tgt.append(el('span','','TARGET'),el('b','',Math.round(target)+'°'));}else tgt.append(el('span','','TARGET'),el('b','','off'));
  const heating=target>0&&temp!==null&&Math.abs(temp-target)>3,at=target>0&&temp!==null&&!heating;
  pill.textContent=stale?'LAST SEEN':temp===null?'NO READING':h.tone==='danger'?'OVER TEMP':heating?(temp<target?'HEATING':'COOLING'):at?'AT TEMP':temp>45&&heatKind==='nozzle'||temp>35&&heatKind==='bed'?'COOLING':'OFF';
  pill.dataset.state=pill.textContent.toLowerCase().replace(/ /g,'-');
  pf.style.setProperty('width',pw===null?'0%':Math.round(Math.max(0,Math.min(1,pw))*100)+'%');power.title=pw===null?'':'Heater power '+Math.round(pw*100)+'%';
  const now=history.length?history[history.length-1][0]:0,pts=history.filter(p=>p[0]>=now-span&&p[1]!==null);
  if(pts.length<2){line.setAttribute('d','');area.setAttribute('d','');tline.setAttribute('d','');aHi.textContent='';aLo.textContent='';return;}
  let vmin=Infinity,vmax=-Infinity;for(const [,v,t] of pts){vmin=Math.min(vmin,v);vmax=Math.max(vmax,v);if(t>0){vmin=Math.min(vmin,t);vmax=Math.max(vmax,t);}}
  const pad=Math.max(3,(vmax-vmin)*.12);vmin=Math.max(0,Math.floor((vmin-pad)/5)*5);vmax=Math.ceil((vmax+pad)/5)*5;if(vmax-vmin<10)vmax=vmin+10;
  const X=t=>((t-(now-span))/span)*W,Y=v=>H-2-((v-vmin)/(vmax-vmin))*(H-6);
  const d=pts.map((p,i)=>(i?'L':'M')+X(p[0]).toFixed(1)+' '+Y(p[1]).toFixed(1)).join(' ');
  line.setAttribute('d',d);area.setAttribute('d',d+` L${X(pts[pts.length-1][0]).toFixed(1)} ${H} L${X(pts[0][0]).toFixed(1)} ${H} Z`);
  const tp=pts.filter(p=>p[2]>0);tline.setAttribute('d',tp.length>1?tp.map((p,i)=>(i?'L':'M')+X(p[0]).toFixed(1)+' '+Y(p[2]).toFixed(1)).join(' '):'');
  aHi.textContent=vmax+'°';aLo.textContent=vmin+'°';}
 bindPress(root);root.addEventListener('click',()=>onOpen?.());root.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onOpen?.();}});
 tip(root,label,'Temperature against target over the last '+Math.round(span/60)+' minutes. Tap for targets, presets and the preset editor.');return {el:root,set};}

// ---------- dock pop-up menu ----------
let popClose=null;
export function openPopover(anchor,items,{onPick,title}={}){
 closePopover();hideTip();const pop=el('div','nx-pop nx-glass nx-glass--raised');pop.setAttribute('role','menu');if(title)pop.append(el('div','nx-pop-title',title));
 for(const it of items){const b=button({icon:it.icon,label:it.label,size:'row',kind:it.danger?'danger':'default',cmd:it.cmd,hold:it.hold,title:it.label,does:it.does||'',onPress:()=>{closePopover();onPick(it);}});b.setAttribute('role','menuitem');pop.append(b);}
 document.body.append(pop);const r=anchor.getBoundingClientRect(),p=pop.getBoundingClientRect();
 let x=r.left+r.width/2-p.width/2,y=r.top-p.height-10;x=Math.max(8,Math.min(innerWidth-p.width-8,x));if(y<8)y=Math.min(innerHeight-p.height-8,r.bottom+10);
 pop.style.left=x+'px';pop.style.top=y+'px';
 const away=e=>{if(!pop.contains(e.target)&&!anchor.contains(e.target))closePopover();},esc=e=>{if(e.key==='Escape')closePopover();};
 setTimeout(()=>{document.addEventListener('pointerdown',away,true);document.addEventListener('keydown',esc);},0);
 popClose=()=>{clearOpener();document.removeEventListener('pointerdown',away,true);document.removeEventListener('keydown',esc);pop.remove();popClose=null;};
 pop.querySelector('.nx-btn:not(:disabled)')?.focus();return pop;}
export function closePopover(){popClose?.();}

// ---------- G-code command bar ----------
// Always in the top bar. Type G-code or search: Klipper's own commands (with their help), your
// macros, your history and the app's actions, filtered as you type; arrows to move, Enter to run.
// Anything typed that is not an action is sent as a console line through the reviewed path.
export function commandBar({host,source,run,locked}){
 const wrap=el('div','nx-cmd'),box=el('label','nx-cmd-box'),input=el('input','nx-cmd-input'),kbd=el('kbd','','/');
 input.type='text';input.spellcheck=false;input.autocomplete='off';input.placeholder='G-code or command…';input.setAttribute('aria-label','G-code or command');input.setAttribute('role','combobox');input.setAttribute('aria-expanded','false');
 box.append(icon('i-console'),input,kbd);const list=el('div','nx-cmd-list nx-glass nx-glass--raised');list.setAttribute('role','listbox');list.hidden=true;wrap.append(box,list);host.append(wrap);
 let items=[],active=0,data=null;
 const score=(q,s)=>{s=s.toLowerCase();if(!q)return 1;if(s.startsWith(q))return 3;if(s.includes(q))return 2;return 0;};
 async function refresh(){data=data||await source();const q=input.value.trim().toLowerCase(),first=q.split(/\s+/)[0]||'';
  const pool=[...data.actions.map(a=>({...a,kind:'action'})),...data.history.map(h=>({label:h,kind:'history',script:h})),...data.commands.map(c=>({label:c.name,help:c.help,kind:c.macro?'macro':'gcode',script:c.name}))];
  const seen=new Set();items=pool.map(it=>({it,s:Math.max(score(first,it.label),score(q,it.label)*1.1,it.help?score(q,it.help)*.5:0)})).filter(x=>x.s>0&&!seen.has(x.it.kind+x.it.label)&&seen.add(x.it.kind+x.it.label))
   .sort((a,b)=>b.s-a.s||(a.it.kind==='action'?-1:0)).slice(0,9).map(x=>x.it);
  if(q&&!items.some(i=>i.label.toLowerCase()===q))items.push({label:input.value.trim(),kind:'send',script:input.value.trim()});
  active=0;paint();}
 function paint(){list.replaceChildren();list.hidden=!items.length||document.activeElement!==input;input.setAttribute('aria-expanded',String(!list.hidden));
  items.forEach((it,i)=>{const row=el('div','nx-cmd-item');row.setAttribute('role','option');row.setAttribute('aria-selected',String(i===active));
   row.append(icon(it.icon||(it.kind==='history'?'i-rewind':it.kind==='macro'?'i-macro':it.kind==='send'?'i-play':'i-console')),el('span','nx-cmd-name',it.label),el('span','nx-cmd-help',it.kind==='send'?'send this line · reviewed once':it.help||(it.kind==='history'?'from your history':it.kind==='macro'?'your macro':it.kind==='action'?'action':'')));
   row.addEventListener('pointerdown',e=>{e.preventDefault();pick(it);});list.append(row);});}
 async function pick(it){if(!it)return;if(it.kind==='action'){input.value='';list.hidden=true;input.blur();return run(it);}
  if(it.kind==='gcode'||it.kind==='macro'){if(input.value.trim().split(/\s+/)[0].toUpperCase()!==it.label.toUpperCase()){input.value=it.label+' ';refresh();return;}}
  const line=(it.kind==='send'||it.kind==='gcode'||it.kind==='macro')?input.value.trim()||it.script:it.script;input.blur();list.hidden=true;
  const ok=await run({kind:'script',script:line});if(ok){input.value='';data=null;}}
 input.addEventListener('focus',()=>{const why=locked();if(why){wrap.dataset.locked='';tip(box,'G-code console',why);}else{delete wrap.dataset.locked;}refresh();});
 input.addEventListener('input',refresh);input.addEventListener('blur',()=>setTimeout(()=>{list.hidden=true;input.setAttribute('aria-expanded','false');},120));
 input.addEventListener('keydown',e=>{if(e.key==='ArrowDown'){active=Math.min(items.length-1,active+1);paint();e.preventDefault();}
  else if(e.key==='ArrowUp'){active=Math.max(0,active-1);paint();e.preventDefault();}
  else if(e.key==='Enter'){e.preventDefault();pick(items[active]);}
  else if(e.key==='Tab'&&items[active]&&items[active].kind!=='action'){e.preventDefault();input.value=items[active].label+' ';refresh();}
  else if(e.key==='Escape'){input.value='';input.blur();}});
 document.addEventListener('keydown',e=>{if(e.key==='/'&&!/input|textarea|select/i.test(document.activeElement?.tagName||'')){e.preventDefault();input.focus();}});
 tip(box,'G-code console','Type G-code or search commands, macros, your history and actions. Enter runs; anything sent to the printer is shown for review first. Press / to jump here.');
 return {focus:()=>input.focus(),el:wrap};}

// ---------- instrument controls, style A (knob) and B (bar) ---------- the
// user picked the climate knob (A) and wants the Control-Center bar (B) as a switchable style: one
// input model for both. value = what you set (target / factor); cur = the live reading for heaters
// (white marker); live = the actual use for speed and flow (a thin bright bar inside the lit part:
// actual speed or flow against 100%); act = 0..1 how much is happening, driving the face's ghost glow.
// Dragging only previews; release (or 600 ms after the last arrow key) calls onSet once.
export function control({style='knob',label,icon:ico,unit='',min=0,max=100,setMin=min,setMax=max,step=5,detents=[],pull=3,onSet,onOpen,title,does,readOnly=false,cls=''}={}){
const knob=style==='knob',root=el('div',(knob?'nxk':'nxb')+(cls?' '+cls:''));if(readOnly)root.dataset.readonly='';root.setAttribute('role','slider');root.tabIndex=0;
 root.setAttribute('aria-label',label);root.setAttribute('aria-valuemin',setMin);root.setAttribute('aria-valuemax',setMax);
 const parts={};
 if(knob){for(const k of ['track','ring','live','cur','handle'])parts[k]=el('i','nxk-'+k);const face=el('div','nxk-face');
  parts.big=el('div','nxk-big');parts.now=el('div','nxk-now');parts.lab=el('div','nxk-label');face.append(parts.big,parts.now,parts.lab);
  root.append(parts.track,parts.ring,parts.live,parts.cur,parts.handle,face);}
 else{for(const k of ['fill','live','cur'])parts[k]=el('i','nxb-'+k);const top=el('div','nxb-top'),bot=el('div','nxb-bot');
  parts.big=el('div','nxb-big');parts.lab=el('div','nxb-label');parts.now=el('div','nxb-now');top.append(parts.big);bot.append(parts.lab,parts.now);
  root.append(parts.fill,parts.live,parts.cur,top,bot);}
 if(ico)parts.lab.append(icon(ico));parts.lab.append(label);
 // Detents: common stops marked on the ring or bar; a drag within `pull` of one snaps to it, the
 // mark lights and touch devices tick.
 const marks=detents.map(d=>{const m=el('i',knob?'nxk-mark':'nxb-mark');m.dataset.v=d;m.style.setProperty('--mf',((d-min)/(max-min)).toFixed(4));
  m.style.setProperty('--mdeg',(225+270*(d-min)/(max-min)).toFixed(2)+'deg');if(d===100)m.dataset.home='';root.append(m);return m;});
 let lastDetent=null;
 const A0=225,SW=270,frac=v=>Math.max(0,Math.min(1,(v-min)/(max-min))),snap=v=>Math.max(setMin,Math.min(setMax,Math.round(v/step)*step));
 let model={},pending=null,pendingAt=0,drag=null,keyTimer=0;
 function paint(){const m=model,v=pending??m.value,f=v===null||v===undefined?0:frac(v);
  root.style.setProperty('--col',m.col||'var(--accent)');root.style.setProperty('--act',Math.max(0,Math.min(1,m.act||0)).toFixed(3));
  root.style.setProperty('--f',f.toFixed(4));root.style.setProperty('--deg',(SW*f).toFixed(2)+'deg');
  const lf=m.live===null||m.live===undefined?null:frac(m.live);root.style.setProperty('--lf',(lf??0).toFixed(4));root.style.setProperty('--ldeg',(SW*(lf??0)).toFixed(2)+'deg');parts.live.hidden=lf===null;
  const cf=m.cur===null||m.cur===undefined?null:frac(m.cur);parts.cur.hidden=cf===null;root.style.setProperty('--cf',(cf??0).toFixed(4));root.style.setProperty('--cdeg',(A0+SW*(cf??0)).toFixed(2)+'deg');
  if(knob){const a=(A0+SW*f)*Math.PI/180;parts.handle.style.left=(50+47*Math.sin(a))+'%';parts.handle.style.top=(50-47*Math.cos(a))+'%';parts.handle.style.setProperty('--hdeg',(A0+SW*f).toFixed(2)+'deg');}
  parts.big.replaceChildren(v===null||v===undefined?'—':String(Math.round(v)));if(v!==null&&v!==undefined&&unit)parts.big.append(el('small','',unit));
  for(const mk of marks)mk.toggleAttribute('data-set',Number(mk.dataset.v)===Math.round(v));
  parts.now.textContent=m.now||'';root.toggleAttribute('data-pending',pending!==null);root.toggleAttribute('data-unknown',m.value===null||m.value===undefined);
  root.setAttribute('aria-valuenow',String(v??''));}
 function set(m){model=m;if(pending!==null&&!drag&&(m.value===pending||performance.now()-pendingAt>6000))pending=null;paint();}
 function commit(v){pending=v;pendingAt=performance.now();paint();Promise.resolve(onSet?.(v)).then(ok=>{if(!ok){pending=null;paint();}});}
 const detent=v=>{const d=detents.find(x=>Math.abs(v-x)<=pull);for(const m of marks)m.toggleAttribute('data-on',Number(m.dataset.v)===d);
  if(d!==undefined&&d!==lastDetent){try{navigator.vibrate?.(8);}catch{}}lastDetent=d??null;return d??v;};
 const fromPointer=e=>detent(rawPointer(e));
 const rawPointer=e=>{const r=root.getBoundingClientRect();
  if(knob){const dx=e.clientX-r.left-r.width/2,dy=e.clientY-r.top-r.height/2;let deg=Math.atan2(dx,-dy)*180/Math.PI;if(deg<0)deg+=360;let rel=(deg-A0+360)%360;if(rel>SW)rel=rel>SW+45?0:SW;return snap(min+(max-min)*rel/SW);}
  return snap(drag.v+(drag.y-e.clientY)/r.height*(max-min));};
 root.addEventListener('pointerdown',e=>{if(e.button!==0||root.hasAttribute('data-locked'))return;try{root.setPointerCapture(e.pointerId);}catch{}
  drag={x:e.clientX,y:e.clientY,v:pending??model.value??min,moved:false};root.dataset.drag='';});
 root.addEventListener('pointermove',e=>{if(!drag)return;if(!drag.moved&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)<5)return;drag.moved=true;if(readOnly)return;pending=fromPointer(e);paint();});
 const end=ok=>{if(!drag)return;const d=drag;drag=null;delete root.dataset.drag;setTimeout(()=>{for(const m of marks)m.removeAttribute('data-on');},700);lastDetent=null;if(ok&&d.moved&&pending!==null&&pending!==model.value)commit(pending);else if(ok&&!d.moved){pending=null;paint();onOpen?.();}else{pending=null;paint();}};
 root.addEventListener('pointerup',()=>end(true));root.addEventListener('pointercancel',()=>end(false));
 root.addEventListener('keydown',e=>{const d={ArrowUp:1,ArrowRight:1,ArrowDown:-1,ArrowLeft:-1,PageUp:5,PageDown:-5}[e.key];
  if(e.key==='Enter'){e.preventDefault();onOpen?.();return;}if(!d||readOnly||root.hasAttribute('data-locked'))return;e.preventDefault();
  pending=snap((pending??model.value??min)+d*step);paint();clearTimeout(keyTimer);keyTimer=setTimeout(()=>commit(pending),600);});
 tip(root,title||label,does||'Drag to set; release sends it. Tap for presets and details.');
 return {el:root,set,lock(why){root.toggleAttribute('data-locked',!!why);setWhy(root,why||'');}};}

// ---------- stat tile: one reading, big, with its label and a note ----------
export function stat(label,{value='—',note='',tone=''}={}){const w=el('div','nx-stat'),v=el('div','nx-stat-v'),n=el('div','nx-stat-n');w.append(el('div','nx-stat-l',label),v,n);
 const set=(x,{note:t=note,tone:k=tone}={})=>{v.textContent=x===null||x===undefined||x===''?'—':String(x);n.textContent=t||'';if(k)w.dataset.tone=k;else delete w.dataset.tone;};set(value);return {el:w,set};}

// ---------- modal: a question over whatever is open (the command review) ---------- A sheet would
// replace the page that asked, so a review opens on top instead and the page underneath carries on
// when it closes. anchor: the review opens as a drop-down under that element (the command bar)
// instead of over the whole screen.
export function openModal({title,icon:ico='i-console',build,onClose,anchor=null}){const back=el('div','nx-modal'+(anchor?' nx-modal--anchored':'')),card=el('div','nx-modal-card nx-glass nx-glass--raised');card.setAttribute('role','dialog');card.setAttribute('aria-modal','true');card.setAttribute('aria-label',title);
 const head=el('div','nx-sheet-head'),body=el('div','nx-modal-body');let done=false;
 const close=()=>{if(done)return;done=true;document.removeEventListener('keydown',esc,true);back.remove();onClose?.();};
 const esc=e=>{if(e.key==='Escape'){e.stopPropagation();close();}};document.addEventListener('keydown',esc,true);
 head.append(icon(ico),el('div','nx-sheet-title',title),button({icon:'i-close',size:'icon',kind:'ghost',title:'Close',does:'Closes this without sending anything.',onPress:close}));
 card.append(head,body);back.append(card);if(anchor){const r=anchor.getBoundingClientRect(),w=Math.min(Math.max(r.width,440),innerWidth-24);card.style.width=w+'px';card.style.left=Math.max(12,Math.min(r.left,innerWidth-w-12))+'px';card.style.top=(r.bottom+8)+'px';}back.addEventListener('pointerdown',e=>{if(e.target===back)close();});document.body.append(back);build(body,close);card.querySelector('.nx-btn[data-kind=primary]')?.focus();return {close};}
