// UI board U07 (2026-10-20): the failure alarm takes the screen, the way docs/design/real-parts-2026-10-20/shots/1-computer-alarm.png and 1-phone-alarm.png show it:
// the camera picture with a box drawn where the failure watch saw it, the three answers as big square keys, a red wash at the edges of the screen, and the day's alarms beside it.
// It grows out of the tile (the farm) or the state chip (a printer's page) it is about and folds back into it. Nothing is stopped by it: a print is paused only by the first key.
// Used by next-fleet.js (the pop-ups poll) on both pages; the words and the boxes are next-core.js (node-tested).
import {el,icon,button,toast,grow,fold} from './next-ui.js';
import {key} from './next-control.js';
import {alarmBoxes,alarmWords} from './next-core.js';

const $=id=>document.getElementById(id);
const clock=ts=>new Date(ts*1000).toLocaleTimeString([],{hour:'numeric',minute:'2-digit'});
const SEVN={alarm:'alarm',warn:'warning',ok:'done',info:'note'};
const KINDS={done:'print ended',plate_clear:'plate to clear',plate_failed:'plate check',offline:'offline',online:'back online',failure_watch:'failure watch',attention:'needs attention',queue_empty:'queue empty',spool_low:'spool low',test:'test'};
let current=null;const waiting=[];

// where the alarm grows from: the printer's tile on the farm, its state chip on its own page, else the bell
const homeOf=e=>document.querySelector(`.fm-tile[data-id="${CSS.escape(e.printer||'')}"]`)||$('nx-state')||document.querySelector('.fl-bell');
const rectOf=n=>n&&n.isConnected&&n.getClientRects().length?n.getBoundingClientRect():null;

async function pause(e,say){
 const c=await fetch(`/api/p/${e.printer}/controls`,{cache:'no-store'}).then(r=>r.json()).catch(()=>({}));
 if(!c.enabled||!c.token){say(c.reason==='controls_enabled is false'?'Printer controls are switched off in Stella Scope, so it cannot be paused from here.':'This device can watch but not control. Pause it from the computer or a paired phone.');return false;}
 const row=(c.commands||[]).find(x=>x.id==='pause');if(row&&row.allowed===false){say(row.reason||'The printer cannot be paused right now.');return false;}
 try{const r=await fetch(`/api/p/${e.printer}/commands`,{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':c.token},body:JSON.stringify({id:'pause',confirm:false})});
  const j=await r.json().catch(()=>({}));if(!r.ok)throw Error(j.error||'The printer refused.');return true;}
 catch(x){say(x.message);return false;}}

/** Show the failure alarm for feed event `e`. `known` is the farm's printers by id; `onWatch(e)` is the page's way to look at the live camera. */
export function raiseAlarm(e,{known=new Map(),onWatch=null}={}){
 if(current){if(current.e.seq!==e.seq&&!waiting.some(w=>w.e.seq===e.seq))waiting.push({e,known,onWatch});current.more();return;}
 const p=known.get(e.printer),words=alarmWords(e,p),boxes=alarmBoxes(e.detail);
 const root=el('div','fl-alarm');root.id='fl-alarm';root.setAttribute('role','alertdialog');root.setAttribute('aria-modal','true');root.setAttribute('aria-labelledby','fl-alarm-title');
 // the day's alarms, beside it (the design's rail): the newest few, each with its dot, its words and the time
 const rail=el('aside','fl-alarm-rail nx-glass');rail.append(el('h3','','Alarms today'));const list=el('div','fl-alarm-list');rail.append(list);
 fetch('/api/alerts/feed?limit=40',{cache:'no-store'}).then(r=>r.json()).then(d=>{const today=new Date().toDateString();
  const rows=(d.events||[]).filter(x=>new Date(x.at*1000).toDateString()===today).reverse().slice(0,4);
  for(const x of rows){const r=el(x.printer?'button':'div','fl-alarm-row');if(x.printer){r.type='button';r.addEventListener('click',()=>{location.href='/p/'+x.printer;});}r.dataset.sev=x.severity||'info';
   const t=el('div','fl-alarm-rowtext');t.append(el('b','',x.name||'Stella Scope'),el('span','',String(x.text||'').replace(new RegExp('^'+(x.name||'').replace(/[.*+?^${}()|[\]\\]/g,'\\$&')+':\\s*'),'')),el('small','',`${clock(x.at)} · ${KINDS[x.kind]||SEVN[x.severity]||x.kind}`));
   r.append(el('i','fl-dot'),t);list.append(r);}}).catch(()=>{});
 // the card: the picture with the box, then the words and the three answers
 const card=el('section','fl-alarm-card nx-glass nx-glass--raised');
 const shot=el('div','fl-alarm-shot'),img=el('img');img.alt=`${p?.name||e.name||'The printer'}, as the camera sees it`;shot.append(img);
 for(const b of boxes){/* the strongest box is named; the others are only outlined */ const bx=el('i','fl-alarm-box');if(b!==boxes[0])bx.dataset.minor='';bx.style.left=b.x1*100+'%';bx.style.top=b.y1*100+'%';bx.style.width=(b.x2-b.x1)*100+'%';bx.style.height=(b.y2-b.y1)*100+'%';
  if(b===boxes[0])bx.append(el('b','',b.pct!==null?`POSSIBLE FAILURE · ${b.pct}%`:'POSSIBLE FAILURE'));if(b.y1<.12)bx.dataset.low='';if(b.x1>.5)bx.dataset.right='';/* the name sits on the side with room, so it is never cut by the picture's edge */shot.append(bx);}
 const none=el('div','fl-alarm-nocam');none.append(icon('i-camera'),el('span','','No camera picture from this printer.'));none.hidden=true;shot.append(none);
 let alive=true,timer=0;
 const snap=()=>{if(!alive||!e.printer)return;const n=new Image();n.onload=()=>{if(!alive)return;img.src=n.src;shot.dataset.ok='';none.hidden=true;shot.style.setProperty('--ar',String(n.naturalWidth/n.naturalHeight));};
  n.onerror=()=>{if(!alive)return;if(!shot.dataset.ok){none.hidden=false;for(const b of shot.querySelectorAll('.fl-alarm-box'))b.hidden=true;}};n.src=`/api/p/${e.printer}/camera/snapshot?t=${Date.now()}`;};
 snap();timer=setInterval(snap,2000);
 const body=el('div','fl-alarm-body'),kick=el('div','fl-alarm-kicker'),more=el('span','fl-alarm-more');
 kick.append(icon('i-warn'),el('span','',`${words.kicker} · ${clock(e.at)}`),more);
 const h=el('h2','',words.title);h.id='fl-alarm-title';
 const say=el('p','fl-alarm-say');say.setAttribute('role','status');say.hidden=true;
 const keys=el('div','fl-alarm-keys');
 const pk=key({icon:'pause',label:'Pause it now',onPress:async()=>{if(pk.el.dataset.busy!==undefined)return;pk.el.dataset.busy='';say.hidden=true;
  const ok=await pause(e,m=>{say.textContent=m;say.hidden=false;});delete pk.el.dataset.busy;
  if(ok){toast(`${e.name||p?.name||'The printer'}: paused.`,'ok',{from:homeOf(e)});answer();}}});
 pk.el.dataset.kind='danger';pk.el.classList.add('fl-alarm-key','fl-alarm-key--pause');
 const wk=key({icon:'camera',label:'Watch',onPress:()=>{answer();(onWatch||(()=>{location.href='/p/'+e.printer;}))(e);}});wk.el.classList.add('fl-alarm-key');
 const fk=key({icon:'check',label:'It is fine',onPress:()=>answer()});fk.el.classList.add('fl-alarm-key');
 keys.append(pk.el,wk.el,fk.el);
 body.append(kick,h,el('p','',words.body),say,keys);card.append(shot,body);
 root.append(rail,card);document.body.append(root);document.body.dataset.alarm='';
 const from=rectOf(homeOf(e));grow(card,from);
 root.animate([{opacity:0},{opacity:1}],{duration:320,easing:'ease-out'});
 const wasFocus=document.activeElement;card.tabIndex=-1;card.focus({preventScroll:true});   // the card, not a key: Tab goes to the first key, and no answer looks chosen for you
 const esc=ev=>{if(ev.key==='Escape')answer(true);else if(ev.key==='Tab'){const f=[...root.querySelectorAll('button:not(:disabled)')];if(!f.length)return;const i=f.indexOf(document.activeElement);ev.preventDefault();f[(i+(ev.shiftKey?-1:1)+f.length)%f.length].focus();}};
 document.addEventListener('keydown',esc,true);
 // folds back into where it grew from, then the next waiting alarm (if any) takes the screen
 function answer(){if(!current||current.gone)return;current.gone=true;alive=false;clearInterval(timer);document.removeEventListener('keydown',esc,true);
  root.style.pointerEvents='none';root.animate([{opacity:1},{opacity:0}],{duration:360,easing:'ease-in',fill:'forwards'});delete document.body.dataset.alarm;
  fold(card,rectOf(homeOf(e)),()=>{root.remove();current=null;if(wasFocus?.isConnected)try{wasFocus.focus({preventScroll:true});}catch{}const n=waiting.shift();if(n)raiseAlarm(n.e,n);});}
 current={e,more:()=>{more.textContent=waiting.length?`${waiting.length} MORE`:'';},gone:false};current.more();}
