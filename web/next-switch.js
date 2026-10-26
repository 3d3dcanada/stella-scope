// Printer switcher (farm board WO-07, 2026-10-12). The identity block at the top left opens the printers
// of this service when there is more than one; keys 1 to 9 go to the nth printer; on a phone a swipe
// along the top bar goes to the previous or next one. The printer is in the URL (/p/<id>, or / for the
// first), so a link or a phone opens the right one.
import {icon,openPopover} from './next-ui.js';

const WORDS={printing:'Printing',heating:'Heating',paused:'Paused',finished:'Done',idle:'Ready',error:'Needs attention',offline:'Offline'};

export function initSwitcher({openPrinters}){
 const anchor=document.querySelector('.nx-id'),top=document.getElementById('nx-top'),me=window.__stellaPrinter||null;
 if(!anchor)return;
 let list=[],caret=null,quiet=0;
 const href=p=>p.default?'/':'/p/'+p.id;
 const at=()=>list.findIndex(p=>me?p.id===me:p.default);
 async function load(){
  try{list=(await (await fetch('/api/printers',{cache:'no-store'})).json()).printers||[];}catch{}
  if(list.length>1&&!caret){caret=icon('i-arrow-down','nx-ico nx-id-caret');anchor.append(caret);}
  if(list.length<2&&caret){caret.remove();caret=null;}
  anchor.title=list.length>1?'Your printers':'';
  if(list.length>1)anchor.dataset.farm='1';else delete anchor.dataset.farm;
 }
 function step(n){if(list.length<2)return;const i=at(),j=((i<0?0:i)+n+list.length)%list.length;location.href=href(list[j]);}
 anchor.addEventListener('click',async e=>{
  if(performance.now()<quiet){e.preventDefault();return;}
  if(list.length<2)return; // one printer, or the list has not arrived: the link goes home as it always did
  e.preventDefault();
  await load();if(list.length<2)return;
  const phone=document.body.dataset.layout==='phone';
  const items=list.map(p=>({icon:'i-printers',label:p.name||p.id,does:(WORDS[p.state]||p.state)+(p.file&&['printing','paused'].includes(p.state)?', '+p.file:''),href:href(p),state:p.state,accent:p.accent,current:p.id===(me||list.find(x=>x.default)?.id)}));
  items.push({icon:'i-nav-workspace',label:'Farm wall',does:'Every printer at a glance.',href:'/farm'},{icon:'i-more',label:'Manage printers',does:'Find and add a printer, rename or remove one.',act:true});
  const pop=openPopover(anchor,items,{title:phone?'Printers · swipe the top bar to change':'Printers · keys 1 to 9',onPick:it=>{if(it.act)openPrinters();else if(!it.current)location.href=it.href;}});
  [...pop.querySelectorAll('.nx-btn')].forEach((b,i)=>{const it=items[i];if(!it||!it.state)return;const d=document.createElement('i');d.className='nx-sw-dot';d.dataset.s=it.state;if(it.accent)d.style.setProperty('--c',it.accent);b.querySelector('.nx-lbl')?.before(d);if(it.current)b.setAttribute('aria-current','true');});
 });
 document.addEventListener('keydown',e=>{
  if(e.ctrlKey||e.metaKey||e.altKey||/^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)||e.target.isContentEditable||document.querySelector('.nx-modal,#nx-sheet:not([hidden])'))return;
  const n=Number(e.key);if(n>=1&&n<=9&&list[n-1]&&n-1!==at())location.href=href(list[n-1]);
 });
 // a swipe along the top bar, on a touch screen
 let sx=0,sy=0,st=0,on=false;
 top?.addEventListener('pointerdown',e=>{on=e.pointerType==='touch'&&!e.target.closest('button,input');sx=e.clientX;sy=e.clientY;st=performance.now();});
 top?.addEventListener('pointerup',e=>{if(!on)return;on=false;const dx=e.clientX-sx,dy=e.clientY-sy;
  if(performance.now()-st<700&&Math.abs(dx)>60&&Math.abs(dy)<40){quiet=performance.now()+500;step(dx<0?1:-1);}});
 load();
}
