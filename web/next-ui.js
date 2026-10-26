// Stella Scope NEXT DOM components (docs/next-ui/BLUEPRINT.md section 6). No printer logic lives
// here: every component takes plain values and callbacks. Geometry and colour rules come from
// next-core.js (C1), touch math from next-gesture.js (C2).
import {arcPath,dialModel,heatColor,DIAL,polar,paletteItems,accessibleName,holdWords,HOLD_NOT_SENT} from './next-core.js';
import {bindPress,bindHold,bindSwipe,bindLongPress} from './next-gesture.js';
import {click,control as kitControl} from './next-control.js';

const SVGNS='http://www.w3.org/2000/svg';
export const $=id=>document.getElementById(id);
export function el(tag,cls='',text=''){const e=document.createElement(tag);if(cls)e.className=cls;if(text!=='')e.textContent=text;return e;}
// Palette 2026-10-25: a canvas cannot read a CSS variable, so a drawing asks for the colour the page is wearing now (the accent follows the chosen palette).
export const ink=(v,fb)=>{try{return getComputedStyle(document.documentElement).getPropertyValue(v).trim()||fb;}catch{return fb;}};
export const inkA=a=>`rgba(${ink("--accent-rgb","4,217,196")},${a})`;
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
// The line that says what a held key does: above the key from the first touch, and "not sent" if the hand lets go early.
let holdTimer=0;
export function holdLine(near,text,tone='',ms=0){clearTimeout(holdTimer);let h=document.getElementById('nx-holdline');if(!h){h=el('div','nx-holdline nx-glass');h.id='nx-holdline';h.setAttribute('role','status');document.body.append(h);}
 h.textContent=text;h.dataset.tone=tone;h.hidden=false;const r=near.getBoundingClientRect(),w=h.offsetWidth,hh=h.offsetHeight;
 h.style.left=Math.max(8,Math.min(innerWidth-w-8,r.left+r.width/2-w/2))+'px';h.style.top=Math.max(8,r.top-hh-12)+'px';if(ms)holdTimer=setTimeout(()=>{h.hidden=true;},ms);}
export function holdLineOff(){clearTimeout(holdTimer);const h=document.getElementById('nx-holdline');if(h)h.hidden=true;}
export function button({icon:ico,label='',kind='default',size='row',cmd=null,onPress,hold=false,attention=false,title,does='',sub=null}={}){
 const b=el('button','nx-btn');b.type='button';b.dataset.kind=kind;b.dataset.size=size;if(cmd)b.dataset.cmd=cmd;if(attention)b.dataset.attention='';
 if(ico)b.append(icon(ico));if(label&&size!=='icon')b.append(el('span','nx-lbl',label));if(sub!==null){const s=el('span','nx-sub',sub);b.append(s);b.sub=s;}
 // finish-2 F07: the name a screen reader gives holds the words on the button (the plate check button showed "Set up the plate check" and was called "Plate check")
 b.setAttribute('aria-label',accessibleName(size!=='icon'?label:'',title||label));tip(b,title||label,does);bindPress(b);
 // Opus polish 2026-10-22: pages rewrite a key's words ("Pick a material" becomes "Heat to 265° and load"); its spoken name follows them
 const span=b.querySelector('.nx-lbl');if(span)new MutationObserver(()=>b.setAttribute('aria-label',accessibleName(span.textContent,title||span.textContent))).observe(span,{childList:true,characterData:true,subtree:true});
 if(hold){b.dataset.hold='';b.append(el('i','nx-hold'));bindHold(b,{ms:900,onStart:()=>holdLine(b,holdWords(cmd,title||label)),onConfirm:()=>{holdLineOff();click(true);onPress?.(b);},onCancel:()=>holdLine(b,HOLD_NOT_SENT,'warn',3500)});
  b.dataset.tipDoes=(does?does+' ':'')+'Press and hold to confirm.';}
 else if(onPress)b.addEventListener('click',()=>{if(!b.disabled&&b.getAttribute('aria-disabled')!=='true'){click(false);onPress(b);}});   // U05: a key clicks (the kit's click: sound, and a buzz on a phone)
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

// ---------- fan ring ----------
// Fan ring (2026-09-24: "you should be able to change the speed on the fly"). With onSet the
// ring is a control: drag round it (top is 0, clockwise to 100 %), it snaps to 5 % and sends on
// release; a tap opens its panel. Without onSet (automatic fans) a tap opens the panel only.
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
 tip(b,title||label+' fan',does||'The speed the printer was told to run, as a percent. It is the command, not a measured speed.');
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
// second press on it closes the thing instead of opening it again (2026-09-24).
export function markOpener(btn){clearOpener();btn?.setAttribute('data-showing','');}
export function clearOpener(){for(const b of document.querySelectorAll('[data-showing]'))b.removeAttribute('data-showing');}
// Keyboard users (polish WO-06 item 5, 2026-10-18): a panel or dialog takes focus when it opens, a modal one keeps Tab inside, and focus goes
// back to what opened it when it closes. Returns {restore, release}: release only lets go of the Tab trap (a panel replaced by another).
function takeFocus(host,{trap=false}={}){
 const opener=document.activeElement;host.tabIndex=-1;try{host.focus({preventScroll:true});}catch{}
 const key=e=>{if(!trap||e.key!=='Tab')return;
  const f=[...host.querySelectorAll('a[href],button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]')].filter(x=>x.getClientRects().length);
  if(!f.length){e.preventDefault();return;}const a=document.activeElement,i=f.indexOf(a);
  if(e.shiftKey&&(a===host||i===0)){e.preventDefault();f.at(-1).focus();}else if(!e.shiftKey&&i===f.length-1){e.preventDefault();f[0].focus();}};
 host.addEventListener('keydown',key);
 const release=()=>host.removeEventListener('keydown',key);
 return {release,restore(){release();if(opener&&opener!==document.body&&opener.isConnected&&(host.contains(document.activeElement)||document.activeElement===document.body||document.activeElement===host))try{opener.focus({preventScroll:true});}catch{}}};}
// ---------- U05 (2026-10-20: "nothing pops up"): a sheet or a dialog grows out of the key or tile that opened it and folds back into it ----------
// The spring is the kit's own (next-control.css --nxctl-spring). What a panel grows out of is what was pressed last (recorded when the press began, so it is still known
// when the press closed the menu that held it). A panel opened from inside another one grows out of the pressed item but folds back into the key that opened the first.
const SPRING='cubic-bezier(.34,1.56,.64,1)',FOLD='cubic-bezier(.5,-.18,.2,1)';
const calm=()=>matchMedia('(prefers-reduced-motion: reduce)').matches;
let lastPress=null;
const noteKey=n=>{if(n&&n!==document.body&&n.getBoundingClientRect)lastPress={el:n,rect:n.getBoundingClientRect().toJSON(),at:performance.now()};};
if(typeof document!=='undefined'){   // (the pure modules that import this file are tested in node, where there is no page)
 document.addEventListener('pointerdown',e=>noteKey(e.target?.closest?.('button,a[href],[role=button],.nxctl,.fm-tile,[data-fm-act]')),true);
 document.addEventListener('keydown',e=>{if((e.key==='Enter'||e.key===' ')&&!e.repeat)noteKey(document.activeElement);},true);}
const homeRect=h=>h&&(h.el?.isConnected&&h.el.getClientRects().length?h.el.getBoundingClientRect():h.rect);
function openedBy(){const p=lastPress&&performance.now()-lastPress.at<3000?lastPress:null;if(!p)return {from:null,home:null};
 const host=p.el.closest?.('.nx-sheet,.nx-modal-card');return {from:p.rect,home:host?host._home||null:{el:p.el,rect:p.rect}};}
const spot=(node,r,from)=>{const cx=from?from.left+from.width/2:r.left+r.width/2,cy=from?from.top+from.height/2:r.top+r.height/2;
 return {o:`${cx-r.left}px ${cy-r.top}px`,s:from?Math.max(.03,Math.min(.35,from.width/r.width,from.height/r.height)):.12};};
/** Grow `node` out of the rectangle `from` (its own middle when there is none): a uniform scale about that point on the spring, so nothing stretches. */
export function grow(node,from){node.getAnimations().forEach(a=>a.cancel());const r=node.getBoundingClientRect();if(!r.width||!r.height)return;
 if(calm()){node.animate([{opacity:0},{opacity:1}],{duration:140});return;}
 const {o,s}=spot(node,r,from);
 node.animate([{opacity:0},{opacity:1,offset:.3},{opacity:1}],{duration:560,easing:'linear'});   // the fade does not ride the spring
 node.animate([{transformOrigin:o,transform:`scale(${s})`},{transformOrigin:o,transform:'scale(1)'}],{duration:560,easing:SPRING});}
/** A full-screen view that is not a sheet (the cameras) grows out of what was pressed; `.back(after)` folds it into that. */
export function emerge(node){const o=openedBy();grow(node,o.from);return {back:after=>fold(node,homeRect(o.home),after)};}
/** Fold `node` back into `to`, then call `after`. */
export function fold(node,to,after){const r=node.getBoundingClientRect();
 if(!r.width||!r.height||calm()){const a=node.animate([{opacity:1},{opacity:0}],{duration:140,fill:'forwards'});a.onfinish=a.oncancel=()=>after?.();return;}
 const {o,s}=spot(node,r,to),cur=getComputedStyle(node).transform;
 node.animate([{opacity:1},{opacity:1,offset:.55},{opacity:0}],{duration:360,easing:'linear',fill:'forwards'});
 const a=node.animate([{transformOrigin:o,transform:cur==='none'?'scale(1)':cur},{transformOrigin:o,transform:`scale(${s})`}],{duration:360,easing:FOLD,fill:'forwards'});
 a.onfinish=a.oncancel=()=>after?.();}
const SHEET_SIZE={'Heat':'compact','Look':'compact','Calibrate':'wide','Materials':'wide','Print report':'wide','Timelapses':'wide','Health':'wide','Alerts':'compact','Build plates':'compact','Pages':'compact','Camera':'regular','Messages':'compact','Fan':'compact','Speed':'compact','Flow':'compact','Macros':'compact','Exclude objects':'compact',
 'Levelling':'wide','Things to print':'wide','Signals':'wide','Parts':'wide','Timeline':'wide','Assembly':'wide','Motion & motors':'wide','Electronics':'wide'};
export function openSheet({title,icon:ico='i-more',build,onClose,wide=false,eyebrow=''}){
 const host=$('nx-sheet');const wasFocus=document.activeElement;const open=openedBy();closeSheet(true);host.className='nx-sheet nx-glass nx-glass--raised'+(wide?' nx-sheet--wide':'');host.hidden=false;host._home=open.home;host.replaceChildren();
 // 2026-10-25: "these should be responsive to the size of what they're needed for". A panel's width follows what it holds: a short list is narrow,
 // a page of figures is regular, a map or a grid of files is wide. Its height follows its content (next-pro.css).
 host.dataset.size=SHEET_SIZE[title]||'regular';
 const grip=el('div','nx-sheet-grip');grip.append(el('i'));
 const head=el('div','nx-sheet-head');const x=()=>button({icon:'i-close',size:'icon',kind:'ghost',title:'Close',does:'Closes this panel.',onPress:()=>closeSheet()});const tt=el('div','nx-sheet-title',title);
 // 2026-09-25: panels drag, resize, pin. Drag the header to float it anywhere (the corner
 // resizes it); Dock puts it back on the right; Pin keeps it on screen while other panels open.
 const tools=el('div','nx-sheet-tools');tools.append(button({icon:'i-dock-right',size:'icon',kind:'ghost',title:'Dock to the side',does:'Puts this panel back on the right edge.',onPress:()=>{panelDock(host,title);}}),
  button({icon:'i-pin',size:'icon',kind:'ghost',title:'Pin',does:'Keeps this panel on screen while you open others. Drag it by its header, resize it from the corner.',onPress:()=>pinSheet(host,title)}));
 if(eyebrow){const w=el('div','nx-sheet-titles');w.append(el('div','nx-sheet-eyebrow',eyebrow),tt);head.append(icon(ico),w,tools,x());}else head.append(icon(ico),tt,tools,x());
 bindPanelDrag(host,head,title);
 const body=el('div','nx-sheet-body');host.append(grip,head,body);
 const unbind=bindSwipe(host,grip,{onDismiss:()=>closeSheet()});
 const esc=e=>{if(e.key==='Escape')closeSheet();};document.addEventListener('keydown',esc);
 // focus goes back to what opened the panel, not to whatever it replaced
 const opener=wasFocus&&host.contains(wasFocus)?null:wasFocus;
 let focus=null;sheetClose=(restore)=>{unbind?.();document.removeEventListener('keydown',esc);onClose?.();if(restore)focus?.restore();else focus?.release();};
 build(body,host);document.body.dataset.sheet=wide?'wide':'';requestAnimationFrame(()=>requestAnimationFrame(()=>host.dataset.open=''));host.setAttribute('aria-label',title);grow(host,open.from);if(open.home?.el?.matches?.('.nx-btn,.nxkey')&&open.home.el.isConnected)markOpener(open.home.el);   // the key stays pressed while its sheet is open
 if(opener){document.activeElement===opener||opener.focus?.({preventScroll:true});}focus=takeFocus(host);
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
export function closeSheet(instant=false){const host=$('nx-sheet');if(!host||host.hidden)return;if(!instant&&host.dataset.closing!==undefined)return;const done=sheetClose;sheetClose=null;done?.(!instant);if(!instant)clearOpener();
 delete host.dataset.open;delete document.body.dataset.sheet;
 if(instant){host.getAnimations().forEach(a=>a.cancel());delete host.dataset.closing;host.style.removeProperty('--drag');host.hidden=true;return;}
 host.dataset.closing='';   // folds back into the key it grew from (U05); nothing under it answers while it folds
 fold(host,homeRect(host._home),()=>{if(host.dataset.closing===undefined)return;delete host.dataset.closing;host.getAnimations().forEach(a=>a.cancel());host.style.removeProperty('--drag');if(!host.dataset.open)host.hidden=true;});}
export function section(title,...children){const s=el('section','nx-section');if(title)s.append(el('h3','',title));s.append(...children);return s;}

// ---------- 6.5 radial: the wheel ----------
// 2026-10-24: "I asked for radial menus in the beginning ... each one of these buttons could be radial menus. Think Skyrim." Every key's choices
// open as a weapon wheel over the machine: the choices round a ring, the one under the pointer lit, its name and what it does in the hub, and the
// key's own figures (where the head is, the gantry, the temperatures) in the hub when nothing is lit. Solid surfaces only: the desktop window's
// WebKit draws no backdrop blur, so nothing here is see-through. Up to eight round the ring; a longer list pages round it (MORE, or the mouse wheel).
// A choice that changes the printer and must be held (a cancel) asks for a second click. Arrows turn it, Enter picks, Escape or the hub closes it.
export function openRadial(items,{x,y,from=null,onPick,title='',center=null}={}){
 const host=$('nx-radial');clearTimeout(host._t);delete host.dataset.closing;host.getAnimations().forEach(a=>a.cancel());host.hidden=false;host.replaceChildren();host.className='nx-radial nx-wheel-host';
 // centred between the top bar and the deck; placed again when the window changes size
 const room=()=>{const top=document.getElementById('nx-top')?.getBoundingClientRect(),dock=document.getElementById('nx-dock')?.getBoundingClientRect();
  const y0=(top?.bottom||0)+10,y1=(dock&&dock.height?dock.top:innerHeight)-10;return {cx:innerWidth/2,cy:(y0+y1)/2,R:Math.max(130,Math.min(270,(y1-y0)/2-6,innerWidth/2-14))};};
 const {cx,cy,R}=room(),r=R*.44,PAGE=8;
 const pages=[];if(items.length<=PAGE)pages.push(items);else for(let i=0;i<items.length;i+=PAGE-1)pages.push(items.slice(i,i+PAGE-1));
 let page=0,hot=-1,armed=-1,list=[];
 const ring=el('div','nx-wheel');Object.assign(ring.style,{left:cx+'px',top:cy+'px',width:2*R+'px',height:2*R+'px'});
 const svg=sv('svg',{class:'nx-wheel-svg',viewBox:`${-R} ${-R} ${2*R} ${2*R}`,'aria-hidden':'true'});ring.append(svg);
 const hub=el('div','nx-wheel-hub');Object.assign(hub.style,{width:2*r-12+'px',height:2*r-12+'px'});
 const hIco=el('div','nx-wheel-hub-ico'),hT=el('b','nx-wheel-hub-t'),hD=el('p','nx-wheel-hub-d'),hC=el('div','nx-wheel-hub-c');hub.append(hIco,hT,hD,hC);ring.append(hub);
 const pager=el('div','nx-wheel-page');ring.append(pager);host.append(ring);if(R<200)ring.dataset.small='';   // a small wheel (a phone) keeps its hub to the name and the figures
 const arc=(a0,a1,ro,ri)=>{const p=(a,rr)=>[(rr*Math.cos(a)).toFixed(2),(rr*Math.sin(a)).toFixed(2)],big=a1-a0>Math.PI?1:0,[x0,y0_]=p(a0,ro),[x1,y1_]=p(a1,ro),[x2,y2]=p(a1,ri),[x3,y3]=p(a0,ri);
  return `M${x0} ${y0_}A${ro} ${ro} 0 ${big} 1 ${x1} ${y1_}L${x2} ${y2}A${ri} ${ri} 0 ${big} 0 ${x3} ${y3}Z`;};
 const words=it=>({t:it.title||it.label,d:it.does||''});
 const centreText=()=>{const c=typeof center==='function'?center():center;return c||'';};
 function showHub(i){const it=list[i];hIco.replaceChildren();if(!calm()&&hub.isConnected)hub.animate([{opacity:.55},{opacity:1}],{duration:160,easing:'ease-out'});
  if(!it){hIco.append(icon(items[0]?.icon||'i-more'));hT.textContent=title||'Choose';hD.textContent=pages.length>1?`Page ${page+1} of ${pages.length}`:'';hC.textContent=centreText();hub.dataset.tone='';return;}
  const w=words(it);hIco.append(icon(it.icon||'i-more'));hT.textContent=it.page?'More choices':w.t;hD.textContent=armed===i?'Click again to confirm.':it.page?`Page ${page+2>pages.length?1:page+2} of ${pages.length}`:(it.sub?it.sub+'. ':'')+w.d;hC.textContent=it.page?'':centreText();hub.dataset.tone=it.tone||'';}
 function build(){svg.replaceChildren();for(const b of ring.querySelectorAll('.nx-wheel-key'))b.remove();
  list=[...pages[page]];if(pages.length>1)list.push({key:'__page',page:true,icon:'i-arrow-right',label:'MORE'});
  // a short list still makes a whole wheel: the unused slots stand empty and dim, as a game's do
  while(list.length<6)list.push({key:'__empty'+list.length,empty:true,label:''});
  const n=list.length,step=2*Math.PI/n,gap=Math.min(.035,step*.08);
  list.forEach((it,i)=>{const a0=-Math.PI/2-step/2+i*step+gap,a1=a0+step-2*gap,mid=(a0+a1)/2;
   const path=sv('path',{d:arc(a0,a1,R,r),class:'nx-wheel-seg'});path.dataset.i=i;if(it.tone)path.dataset.tone=it.tone;if(it.empty)path.dataset.empty='';svg.append(path);
   // 2026-10-25 ("the radial menu is not animated as well as it could be"): a lit choice lifts out along its own line from the middle
   const ox=(6*Math.cos(mid)).toFixed(1),oy=(6*Math.sin(mid)).toFixed(1);path.style.setProperty('--dx',ox+'px');path.style.setProperty('--dy',oy+'px');
   if(!calm())path.animate([{opacity:0,transform:'scale(.55) rotate(-14deg)'},{opacity:1,transform:'scale(1) rotate(0deg)'}],{duration:320,delay:30+i*28,easing:'cubic-bezier(.2,.9,.25,1.15)',fill:'backwards'});
   if(it.empty)return;
   const lr=(R+r)/2,bx=lr*Math.cos(mid),by=lr*Math.sin(mid);
   const b=button({icon:it.icon,label:it.label,size:'dock',kind:it.danger?'danger':'default',cmd:it.page?null:it.cmd,title:it.title||it.label,does:it.does||'',onPress:()=>pick(i)});
   b.classList.add('nx-wheel-key');if(it.sub)b.append(el('span','nx-radial-sub',it.sub));if(it.num)b.classList.add('nx-radial-num');if(it.tone)b.dataset.tone=it.tone;
   b.style.left=`calc(50% + ${bx.toFixed(1)}px)`;b.style.top=`calc(50% + ${by.toFixed(1)}px)`;b.style.setProperty('--seg',Math.min(2*lr*Math.sin(step/2)-14,150).toFixed(0)+'px');b.style.setProperty('--dx',ox+'px');b.style.setProperty('--dy',oy+'px');b.dataset.i=i;ring.append(b);
   if(!calm())b.animate([{opacity:0,transform:`translate(-50%,-50%) translate(${(-bx*.55).toFixed(1)}px,${(-by*.55).toFixed(1)}px) scale(.6)`},{opacity:1,transform:'translate(-50%,-50%)'}],{duration:340,delay:60+i*28,easing:'cubic-bezier(.2,.9,.25,1.1)',fill:'backwards'});});
  pager.textContent=pages.length>1?`${page+1} / ${pages.length}`:'';hot=-1;armed=-1;paint();showHub(-1);}
 function paint(){const keys=[...ring.querySelectorAll('.nx-wheel-key')],by=i=>keys.find(b=>+b.dataset.i===i);svg.querySelectorAll('.nx-wheel-seg').forEach((p,i)=>{p.toggleAttribute('data-hot',i===hot);p.toggleAttribute('data-off',!!by(i)?.disabled);});keys.forEach(b=>b.toggleAttribute('data-hot',+b.dataset.i===hot));}
 const sync=setInterval(paint,400);
 function pick(i){const it=list[i];if(!it||it.empty)return;if(it.page){page=(page+1)%pages.length;build();return;}
  const b=[...ring.querySelectorAll('.nx-wheel-key')].find(x=>+x.dataset.i===i);if(b?.disabled){hot=i;paint();showHub(i);hD.textContent=(b.dataset.why||b.title||'')||'Not available right now.';return;}
  if(it.hold&&armed!==i){armed=i;hot=i;paint();showHub(i);return;}
  close();onPick?.(it);}
 const place=()=>{const p=room();ring.style.left=p.cx+'px';ring.style.top=p.cy+'px';};addEventListener('resize',place);
 const close=()=>{clearInterval(sync);removeEventListener('resize',place);clearOpener();document.removeEventListener('keydown',key);if(host.dataset.closing!==undefined)return;
  if(calm()){host.hidden=true;host.replaceChildren();return;}host.dataset.closing='';
  svg.querySelectorAll('.nx-wheel-seg').forEach((p,k)=>p.animate([{opacity:1,transform:'scale(1)'},{opacity:0,transform:'scale(.7) rotate(10deg)'}],{duration:170,delay:k*12,easing:'ease-in',fill:'forwards'}));
  ring.animate([{transform:'translate(-50%,-50%) scale(1)',opacity:1},{transform:'translate(-50%,-50%) scale(.9)',opacity:0}],{duration:200,delay:60,easing:'ease-in',fill:'forwards'});
  host.animate([{opacity:1},{opacity:0}],{duration:260,easing:'ease-in',fill:'forwards'});
  host._t=setTimeout(()=>{delete host.dataset.closing;host.getAnimations().forEach(a=>a.cancel());host.hidden=true;host.replaceChildren();},280);};
 const at=e=>{const b=ring.getBoundingClientRect(),dx=e.clientX-(b.left+b.width/2),dy=e.clientY-(b.top+b.height/2),d=Math.hypot(dx,dy);
  if(d<r*.92)return {zone:'hub'};if(d>R+70)return {zone:'out'};const n=list.length,step=2*Math.PI/n;let a=Math.atan2(dy,dx)+Math.PI/2+step/2;a=((a%(2*Math.PI))+2*Math.PI)%(2*Math.PI);return {zone:'ring',i:Math.floor(a/step)%n};};
 host.addEventListener('pointermove',e=>{const z=at(e),i=z.zone==='ring'&&!list[z.i]?.empty?z.i:-1;if(i!==hot){hot=i;if(armed!==i)armed=-1;paint();showHub(i);}});
 host.addEventListener('click',e=>{if(e.target.closest('.nx-wheel-key'))return;const z=at(e);if(z.zone==='ring')pick(z.i);else close();});
 host.addEventListener('wheel',e=>{if(pages.length<2)return;e.preventDefault();page=(page+(e.deltaY>0?1:pages.length-1))%pages.length;build();},{passive:false});
 const key=e=>{if(e.key==='Escape'){e.preventDefault();close();return;}const n=list.length;
  const step=d=>{let i=hot;for(let k=0;k<n;k++){i=(i+d+n)%n;if(!list[i].empty)break;}hot=i;paint();showHub(hot);};
  if(['ArrowRight','ArrowDown'].includes(e.key)){e.preventDefault();step(1);}
  else if(['ArrowLeft','ArrowUp'].includes(e.key)){e.preventDefault();step(-1);}
  else if(e.key==='Enter'&&hot>=0){e.preventDefault();pick(hot);}};
 document.addEventListener('keydown',key);
 if(!calm()){ring.animate([{transform:'translate(-50%,-50%) scale(.82)',opacity:0},{transform:'translate(-50%,-50%) scale(1)',opacity:1}],{duration:220,easing:'cubic-bezier(.2,.9,.3,1.1)'});host.animate([{opacity:0},{opacity:1}],{duration:180});}
 build();
 const btns=()=>[...ring.querySelectorAll('.nx-wheel-key')];
 // Touch: press on the key, drag onto a choice, let go (the pie menu gesture the old radial had).
 let tracking=false;
 return {close,start:()=>{tracking=true;},release:e=>{if(!tracking)return;tracking=false;const z=at(e);if(z.zone==='ring'&&!list[z.i]?.hold)pick(z.i);},get btns(){return btns();}};}

// ---------- 6.9 toast ----------
// U07 (2026-10-20: every message grows from where it belongs and settles back): a toast grows out of what it is about (`opts.from`: a tile, a key or a rectangle) or,
// when the page does not say, out of the key that was just pressed, and folds back into it when it goes. With neither (a message nobody asked for) it grows from its own place.
// 2026-10-24: notifications never sit over the machine. On a computer they slide into the bottom deck's lane; every one is kept for the bell (notices).
export const notices=[];let noticeHook=null;export function onNotice(f){noticeHook=f;}
export function toast(text,tone='info',opts={}){if(text){notices.push({text:String(text),tone,at:Date.now()});if(notices.length>60)notices.shift();noticeHook?.();}
 const lane=document.body.dataset.layout!=='phone'?$('nx-lane-toasts'):null,host=lane||$('nx-toasts');if(!host||!text)return;const t=el('div','nx-toast nx-glass nx-glass--raised');t.dataset.tone=tone;
 t.append(icon(tone==='alarm'?'i-warn':tone==='ok'?'i-check':'i-live'),el('span','',text));host.append(t);
 const src=opts.from||(lastPress&&performance.now()-lastPress.at<6000?lastPress.el:null),keep=src?.getBoundingClientRect?.().toJSON?.()||(lastPress&&!opts.from?lastPress.rect:null);
 const where=()=>(src&&src.isConnected&&src.getClientRects?.().length?src.getBoundingClientRect():keep)||null;
 const gone=()=>{if(t._out)return;t._out=true;fold(t,where(),()=>t.remove());};t._gone=gone;
 for(let live=[...host.children].filter(c=>!c._out);live.length>2;live.shift())live[0]._gone();   // two at most: the oldest folds away
 grow(t,where());setTimeout(gone,tone==='alarm'?7000:4000);}

// ---------- small widgets used by sheets ----------
export function segmented(values,current,onPick,fmt=v=>String(v)){const s=el('div','nx-seg');for(const v of values){const b=el('button','',fmt(v));b.type='button';b.setAttribute('aria-pressed',String(v===current));
 b.addEventListener('click',()=>{for(const x of s.children)x.setAttribute('aria-pressed','false');b.setAttribute('aria-pressed','true');onPick(v);});s.append(b);}return s;}
export function stepper({value,unit='',step=5,min=0,max=300,onChange}){const w=el('div','nx-stepper');const v=el('div','nx-stepper-value');let cur=value;
 const paint=()=>{v.replaceChildren(String(cur));v.append(el('small','',unit));};paint();
 const minus=button({icon:'i-minus',size:'icon',title:'Lower',does:`Lowers by ${step}${unit}.`,onPress:()=>{cur=Math.max(min,cur-step);paint();onChange?.(cur);}});
 const plus=button({icon:'i-plus',size:'icon',title:'Raise',does:`Raises by ${step}${unit}.`,onPress:()=>{cur=Math.min(max,cur+step);paint();onChange?.(cur);}});
 w.append(minus,v,plus);w.get=()=>cur;w.setValue=x=>{cur=x;paint();};return w;}
export {bindLongPress};
// ---------- U05 (2026-10-20): a number that is set in a sheet or a dialog is the kit's control, never a typed field ----------
// An LED gauge at rest (in a panel it is smaller than on the printer page), a black knob when held, the new value sent once on release (`onSet`). The caption is ours
// because the kit's own label is too small at this size. `get()` is the value now; `set(v)` is the page moving it (never calls onSet).
export function numberField({label,unit='',prefix='',min=0,max=100,step=1,value=0,turn=300,color='var(--accent)',color2='var(--accent-2)',size=92,onSet,onChange}={}){
 const w=el('div','nx-numf'),c=kitControl({label,unit,prefix,min,max,step,value,turn,color,color2,onSet,onChange});c.el.setAttribute('aria-label',label);
 w.style.setProperty('--nsz',size+'px');w.append(c.el,el('small','nx-numf-l',label));w.get=()=>c.spec.value;w.set=w.setValue=v=>c.set(v);w.kit=c;return w;}
/** An LED figure: the value lit over a ghost of unlit digits (the panel language of the farm wall's counts); `unit` is a small word after it. */
export function led(value,unit='',cls=''){const b=el('b','nx-fig'+(cls?' '+cls:'')),n=el('i','nx-fig-n',String(value));n.dataset.ghost='8'.repeat(Math.max(2,String(value).length));b.append(n);if(unit)b.append(el('small','',unit));return b;}

// ---------- instrument gauge (NEXT review 2026-09-24: "the gauges look half done; speed and
// flow definitely belong in a round gauge") ----------
// A real instrument: machined bezel, numbered scale over a 240° sweep, value band, red zone, a
// tapered needle on a hub, digital readout under the hub, glass sheen. The needle rotates with a
// CSS transition, so it eases between readings without a JS animation loop.
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
// Polish WO-06 (2026-10-18): free text is never sent from here (next-core.js paletteItems); typed G-code
// goes through the Console (`openConsole`). A command the printer lists, with its arguments, still runs.
export function commandBar({host,source,run,locked,openConsole,hasConsole}){
 const wrap=el('div','nx-cmd'),box=el('label','nx-cmd-box'),input=el('input','nx-cmd-input'),kbd=el('kbd','','/');
 input.type='text';input.spellcheck=false;input.autocomplete='off';input.placeholder='Find a command…';input.setAttribute('aria-label','Find a command');input.setAttribute('role','combobox');input.setAttribute('aria-expanded','false');
 box.append(icon('i-console'),input,kbd);const list=el('div','nx-cmd-list nx-glass nx-glass--raised');list.setAttribute('role','listbox');list.hidden=true;wrap.append(box,list);host.append(wrap);
 let items=[],active=0,data=null;
 async function refresh(){data=data||await source();items=paletteItems(data,input.value,{console:!!openConsole&&(hasConsole?hasConsole():true)}).map(it=>it.consoleText!==undefined?{...it,go:()=>openConsole(it.consoleText)}:it);active=0;paint();}
 function paint(){list.replaceChildren();list.hidden=!items.length||document.activeElement!==input;input.setAttribute('aria-expanded',String(!list.hidden));
  items.forEach((it,i)=>{const row=el('div','nx-cmd-item');row.setAttribute('role','option');row.setAttribute('aria-selected',String(i===active));
   row.append(icon(it.icon||(it.kind==='history'?'i-rewind':it.kind==='macro'?'i-macro':'i-console')),el('span','nx-cmd-name',it.label),el('span','nx-cmd-help',it.help||(it.kind==='history'?'from your history':it.kind==='macro'?'your macro':it.kind==='action'?'action':'')));
   row.addEventListener('pointerdown',e=>{e.preventDefault();pick(it);});list.append(row);});}
 async function pick(it){if(!it||it.kind==='none')return;if(it.kind==='action'){input.value='';list.hidden=true;input.blur();return run(it);}
  if(it.kind==='gcode'||it.kind==='macro'){if(input.value.trim().split(/\s+/)[0].toUpperCase()!==it.label.toUpperCase()){input.value=it.label+' ';refresh();return;}}
  const line=(it.kind==='gcode'||it.kind==='macro')?input.value.trim()||it.script:it.script;input.blur();list.hidden=true;
  const ok=await run({kind:'script',script:line});if(ok){input.value='';data=null;}}
 input.addEventListener('focus',()=>{const why=locked();if(why){wrap.dataset.locked='';tip(box,'Command search',why);}else{delete wrap.dataset.locked;}refresh();});
 input.addEventListener('input',refresh);input.addEventListener('blur',()=>setTimeout(()=>{list.hidden=true;input.setAttribute('aria-expanded','false');},120));
 input.addEventListener('keydown',e=>{if(e.key==='ArrowDown'){active=Math.min(items.length-1,active+1);paint();e.preventDefault();}
  else if(e.key==='ArrowUp'){active=Math.max(0,active-1);paint();e.preventDefault();}
  else if(e.key==='Enter'){e.preventDefault();pick(items[active]);}
  else if(e.key==='Tab'&&items[active]&&!['action','none'].includes(items[active].kind)){e.preventDefault();input.value=items[active].label+' ';refresh();}
  else if(e.key==='Escape'){input.value='';input.blur();}});
 document.addEventListener('keydown',e=>{if(e.key==='/'&&!/input|textarea|select/i.test(document.activeElement?.tagName||'')){e.preventDefault();input.focus();}});
 tip(box,'Command search','Find any control or page by its name, such as Heat, Speed, Move, Failure watch or Motors off. Enter opens or runs the one picked; anything sent to the printer is shown for review first. G-code you type goes in the Console. Press / to jump here.');
 return {focus:()=>input.focus(),el:wrap};}

// ---------- instrument controls, style A (knob) and B (bar) — NEXT review 2026-09-24 ----------
// the owner picked the climate knob (A) and wants the Control-Center bar (B) as a switchable style: one
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
 // Detents (the owner: "hard locks where it clicks into place and glows"): common stops marked on the
 // ring or bar; a drag within `pull` of one snaps to it, the mark lights and touch devices tick.
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
 // 2026-10-23: a figure with nothing to show never stands as a bare dash: it is dimmed and says why (its own note, else that nothing came yet).
 const set=(x,{note:t=note,tone:k=tone}={})=>{const empty=x===null||x===undefined||x===''||x==='—';v.textContent=empty?'—':String(x);n.textContent=t||(empty?'nothing reported yet':'');w.toggleAttribute('data-empty',empty);if(k)w.dataset.tone=k;else delete w.dataset.tone;};set(value);return {el:w,set};}

// ---------- modal: a question over whatever is open (the command review) ----------
// A sheet would replace the page that asked (our filament wizard lost its place), so a review
// opens on top instead and the page underneath carries on when it closes.
// anchor: the review opens as a drop-down under that element (the command bar) instead of over the
// whole screen (2026-09-25).
export function openModal({title,icon:ico='i-console',build,onClose,anchor=null}){const open=openedBy(),from=anchor?anchor.getBoundingClientRect():open.from;
 const back=el('div','nx-modal'+(anchor?' nx-modal--anchored':'')),card=el('div','nx-modal-card nx-glass nx-glass--raised');card.setAttribute('role','dialog');card.setAttribute('aria-modal','true');card.setAttribute('aria-label',title);card._home=anchor?{el:anchor,rect:from}:open.home;
 const head=el('div','nx-sheet-head'),body=el('div','nx-modal-body');let done=false,focus=null;
 // U05: the dialog folds back into the key it grew from; the page behind it answers again at once
 const close=()=>{if(done)return;done=true;document.removeEventListener('keydown',esc,true);focus?.restore();onClose?.();back.dataset.closing='';
  back.animate([{opacity:1},{opacity:0}],{duration:300,easing:'ease-in',fill:'forwards'});fold(card,homeRect(card._home),()=>back.remove());};
 const esc=e=>{if(e.key==='Escape'){e.stopPropagation();close();}};document.addEventListener('keydown',esc,true);
 head.append(icon(ico),el('div','nx-sheet-title',title),button({icon:'i-close',size:'icon',kind:'ghost',title:'Close',does:'Closes this without sending anything.',onPress:close}));
 card.append(head,body);back.append(card);if(anchor){const r=anchor.getBoundingClientRect(),w=Math.min(Math.max(r.width,440),innerWidth-24);card.style.width=w+'px';card.style.left=Math.max(12,Math.min(r.left,innerWidth-w-12))+'px';card.style.top=(r.bottom+8)+'px';}
 back.addEventListener('pointerdown',e=>{if(e.target===back)close();});document.body.append(back);focus=takeFocus(card,{trap:true});build(body,close);card.querySelector('.nx-btn[data-kind=primary]:not(:disabled)')?.focus({preventScroll:true});
 // U05: it opens over the key it grew from (as near to it as the window allows), not in the middle of the screen, and the dim gathers there
 if(!anchor){const r=card.getBoundingClientRect(),cx=from?from.left+from.width/2:innerWidth/2,cy=from?from.top+from.height/2:innerHeight/2;
  card.style.width=r.width+'px';card.style.position='fixed';card.style.left=Math.max(12,Math.min(innerWidth-r.width-12,cx-r.width/2))+'px';card.style.top=Math.max(12,Math.min(innerHeight-r.height-12,cy-r.height/2))+'px';
  back.style.setProperty('--ox',cx+'px');back.style.setProperty('--oy',cy+'px');}
 // Opus polish 2026-10-22: a panel placed once at open time slid off the bottom of the window when its content grew (the filament picker's OTHER
 // fields), and its own primary key could not be reached. It moves up as it grows, never above the window's top; past that it scrolls inside.
 const keepIn=()=>{if(done)return ro.disconnect();const h=card.offsetHeight,t=parseFloat(card.style.top);if(isFinite(t)&&t+h>innerHeight-12)card.style.top=Math.max(12,innerHeight-h-12)+'px';};
 const ro=new ResizeObserver(keepIn);ro.observe(card);
 grow(card,from);back.animate([{opacity:0},{opacity:1}],{duration:260,easing:'ease-out'});
 return {close};}

// ---------- Review 2026-10-20 (Opus): a number that is typed, not turned ----------
// the owner: dials only where a dial is the right input. A price, a weight, a bed size, a nozzle in millimetres or a number of copies is
// typed: an LED-figure field with square − and + keys either side (one step a press, held repeats). Same interface as numberField
// (`get`, `set`, `onSet` once when the value settles) so a page swaps one for the other.
export function numberInput({label,unit='',prefix='',min=0,max=100,step=1,value=0,onSet}={}){
 const d=String(step).includes('.')?String(step).split('.')[1].length:0,clamp=v=>Math.min(max,Math.max(min,+(+v).toFixed(d)));
 let cur=clamp(value),t=0;const w=el('div','nx-numi'),cap=el('small','nx-numi-l',label),row=el('div','nx-numi-row');
 const inp=el('input','nx-numi-v');inp.type='text';inp.inputMode=d?'decimal':'numeric';inp.setAttribute('aria-label',label);
 const show=()=>{inp.value=(prefix||'')+cur.toFixed(d)+(unit?unit:'');};
 const commit=()=>{clearTimeout(t);t=setTimeout(()=>{if(onSet)onSet(cur);},500);};
 const to=v=>{const n=clamp(v);if(n!==cur){cur=n;show();commit();}else show();};
 const k=(ic,dir)=>{const b=button({icon:ic,label:'',size:'icon',title:dir>0?'More':'Less',does:`${dir>0?'Adds':'Takes off'} ${step}${unit}.`,onPress:()=>to(cur+dir*step)});b.classList.add('nx-numi-k');
  let rep=0;b.addEventListener('pointerdown',()=>{rep=setTimeout(function r(){to(cur+dir*step);rep=setTimeout(r,70);},450);});
  for(const e of ['pointerup','pointerleave','pointercancel'])b.addEventListener(e,()=>clearTimeout(rep));return b;};
 inp.addEventListener('focus',()=>{inp.value=cur.toFixed(d);inp.select();});
 inp.addEventListener('blur',()=>{const n=parseFloat(String(inp.value).replace(/[^0-9.\-]/g,''));to(isFinite(n)?n:cur);});
 inp.addEventListener('keydown',e=>{if(e.key==='Enter')inp.blur();if(e.key==='ArrowUp'){e.preventDefault();to(cur+step);}if(e.key==='ArrowDown'){e.preventDefault();to(cur-step);}});
 row.append(k('i-minus',-1),inp,k('i-plus',1));w.append(row,cap);show();
 w.get=()=>cur;w.set=w.setValue=v=>{cur=clamp(v);show();};return w;}

// rc9 WO-01 (2026-10-23: "you should be able to choose what to add, what to take away"): every list that has a
// standard one (materials, build sheets, maintenance) offers the standard entries it does not hold, one at a time or all.
let standardLists=null;
export async function addFromStandard(anchor,kind,have,onAdd,{noun='entry',detail=()=> ''}={}){
 try{standardLists||=await (await fetch('/api/presets/standard')).json();}catch{toast('The standard list did not load. Try again.','alarm');return;}
 const taken=new Set(have.map(n=>String(n).trim().toLowerCase())),missing=(standardLists[kind]||[]).filter(x=>!taken.has(String(x.name).toLowerCase()));
 if(!missing.length){toast(`Every standard ${noun} is already in the list.`,'ok');return;}
 const items=[...(missing.length>1?[{icon:'i-plus',label:`Add all ${missing.length}`,all:true,does:`Adds every standard ${noun} you do not have.`}]:[]),
  ...missing.map(x=>({icon:'i-plus',label:x.name+(detail(x)?' · '+detail(x):''),entry:x,does:`Adds ${x.name}. You can change it or take it away again.`}))];
 openPopover(anchor,items,{title:`Add from the standard list`,onPick:it=>onAdd(it.all?missing.map(x=>({...x})):[{...it.entry}])});}

// ---------- the Look panel (2026-10-25: "be able to change the colour palette and how everything looks on the fly") ----------
// The colour, the finish and the dial style, kept on this device (stella-paint) and put on <html> at once; paint.js puts them back before the next page draws.
export const PAINT={
 accent:[['silver','Silver','#D9DEE3'],['racing-yellow','Racing Yellow','#FFC21A'],['guards-red','Guards Red','#E8323A'],['miami-blue','Miami Blue','#16AEE6'],
  ['python-green','Python Green','#58C46A'],['lava-orange','Lava Orange','#FF6A2A'],['ultraviolet','Ultraviolet','#9B7BFF'],['stella-teal','Stella Teal','#04D9C4']],
 finish:[['graphite','Graphite','linear-gradient(180deg,#22262b,#111316)'],['carbon','Carbon','repeating-linear-gradient(45deg,#1a1a1c 0 3px,#0b0b0c 3px 6px)'],['midnight','Midnight','linear-gradient(180deg,#1b2633,#0c1219)']],
 dials:[['led','LED'],['smooth','Smooth']],
 chips:[['on','Readings on the machine'],['off','Gauges only']]};
export function paintGet(){const d=document.documentElement.dataset;return {accent:d.accent||'silver',finish:d.finish||'graphite',dials:d.dials||'led',chips:d.chips||'on'};}
export function paintSet(k,v){const d=document.documentElement;d.dataset[k]=v;const p=paintGet();try{localStorage.setItem('stella-paint',JSON.stringify(p));}catch{}
 document.dispatchEvent(new CustomEvent('stella-paint',{detail:p}));return p;}
const dialPic=smooth=>{const s=sv('svg',{viewBox:'-60 -60 120 120','aria-hidden':'true'}),n=18;for(let i=0;i<n;i++){const a0=(-135+i*15+(smooth?0:2))*Math.PI/180,a1=(-135+(i+1)*15-(smooth?0:2))*Math.PI/180,p=(a,r)=>[(Math.sin(a)*r).toFixed(1),(-Math.cos(a)*r).toFixed(1)];
  const [x0,y0]=p(a0,50),[x1,y1]=p(a1,50),[x2,y2]=p(a1,40),[x3,y3]=p(a0,40);s.append(sv('path',{d:`M${x0} ${y0}A50 50 0 0 1 ${x1} ${y1}L${x2} ${y2}A40 40 0 0 0 ${x3} ${y3}Z`,class:i<12?(i===11?'tip':'on'):'off'}));}return s;};
export function lookPanel(){const root=el('div','nx-paint'),cur=paintGet();
 const group=(cls,list,k,make)=>{const g=el('div',cls);for(const it of list){const b=el('button');b.type='button';b.setAttribute('aria-pressed',String(cur[k]===it[0]));make(b,it);b.title=it[1];
  b.addEventListener('click',()=>{paintSet(k,it[0]);cur[k]=it[0];for(const x of g.children)x.setAttribute('aria-pressed',String(x===b));});g.append(b);}return g;};
 const sw=group('nx-paint-sw',PAINT.accent,'accent',(b,[,name,c])=>{const i=el('i');i.style.setProperty('--sw',c);b.append(i,name);});
 const fin=group('nx-paint-fin',PAINT.finish,'finish',(b,[,name,bg])=>{const i=el('i');i.style.setProperty('--sw',bg);b.append(i,name);});
 const dial=group('nx-paint-dial',PAINT.dials,'dials',(b,[key,name])=>{b.append(dialPic(key==='smooth'),name);});
 const chips=group('nx-paint-dial nx-paint-chips',PAINT.chips,'chips',(b,[,name])=>{b.append(name);});
 root.append(section('Colour',sw),section('Finish',fin),section('Dials',dial),section('On the machine',chips),el('p','','Kept on this screen. Every page follows it at once.'));return root;}
