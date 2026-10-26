// Accessibility basics of the farm wall and a printer page (polish WO-06 item 5, 2026-10-18), against a scratch instance of demo printers:
//   A11Y_URL=http://127.0.0.1:7423 PLAYWRIGHT_MODULE=/path/to/playwright-core node scripts/a11y-smoke.mjs
// Names (every control a person can see has one), focus (Tab walks in page order, every stop has a name and shows a ring), contrast
// (every piece of text meets WCAG AA against what it sits on, dark theme), and a panel (it takes focus when it opens, Escape closes
// it and gives focus back to what opened it). Nothing is clicked that sends a command.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const base=process.env.A11Y_URL;
if(!base||!/^http:\/\/127\.0\.0\.1:\d+$/.test(base))throw Error('A11Y_URL must be a loopback scratch instance of demo printers.');
const ids=(await (await fetch(base+'/api/printers')).json()).printers.filter(p=>p.driver==='demo').map(p=>p.id);
if(!ids.length)throw Error('the scratch instance needs a demo printer (Add a printer, Try the demo farm).');
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=gl','--ignore-gpu-blocklist']});
let checks=0;const ok=(c,m)=>{assert.ok(c,m);checks++;console.log('PASS '+m);};
try{
 for(const [path,label] of [['/farm','the farm wall'],['/p/'+ids[0],'a demo printer page']]){
  const page=await (await browser.newContext({viewport:{width:1366,height:768}})).newPage();
  await page.goto(base+path);await page.waitForTimeout(9000);
  const names=await page.evaluate(()=>{const vis=e=>{const r=e.getBoundingClientRect(),s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'&&!e.closest('[hidden],[inert]');};
   const text=e=>(e.textContent||'').replace(/\s+/g,' ').trim();
   const nameOf=e=>e.getAttribute('aria-label')?.trim()||(e.labels&&[...e.labels].map(text).join(' ').trim())||(e.tagName==='IMG'?(e.hasAttribute('alt')?'alt':''):text(e))||e.getAttribute('title')||e.dataset.tipTitle||'';
   const all=[...document.querySelectorAll('button,a[href],input:not([type=hidden]),select,textarea,img,[role=button],[role=slider],[role=switch],[role=tab],[role=option],[tabindex="0"]')].filter(vis);
   return {n:all.length,bad:all.filter(e=>!nameOf(e)).map(e=>e.tagName+'.'+e.className)};});
  ok(names.n>20&&names.bad.length===0,`${label}: every one of ${names.n} controls has a name ${names.bad.join(', ')}`);
  // focus: 40 stops
  let named=0,ringed=0,prevY=-1e9,backJumps=0,stops=0;
  await page.evaluate(()=>{document.activeElement?.blur();scrollTo(0,0);});
  for(let i=0;i<40;i++){await page.keyboard.press('Tab');await page.waitForTimeout(300);   // a ring fades in
   const s=await page.evaluate(()=>{const e=document.activeElement;if(!e||e===document.body)return null;const r=e.getBoundingClientRect(),cs=getComputedStyle(e),box=e.closest('.nx-cmd-box');
    const ring=(cs.outlineStyle!=='none'&&parseFloat(cs.outlineWidth)>0)||cs.boxShadow!=='none'||(box&&(getComputedStyle(box).boxShadow!=='none'||getComputedStyle(box).outlineStyle!=='none'));
    return {name:(e.getAttribute('aria-label')||(e.labels&&e.labels[0]?.textContent)||e.textContent||e.getAttribute('title')||e.dataset.tipTitle||'').trim(),ring:!!ring,y:r.y+scrollY,h:r.height};});
   if(!s)continue;stops++;if(s.name)named++;if(s.ring)ringed++;if(s.y+s.h<prevY-768)backJumps++;prevY=s.y;}
  ok(stops>=15&&named===stops,`${label}: Tab reaches ${stops} stops and every one has a name`);
  ok(ringed===stops,`${label}: every stop shows where focus is (${ringed} of ${stops})`);
  ok(backJumps===0,`${label}: Tab never jumps back up the page by more than a screenful`);
  const low=await page.evaluate(()=>{
   const parse=c=>{const m=c.match(/rgba?\(([^)]+)\)/);if(!m)return null;const p=m[1].split(/[ ,\/]+/).filter(Boolean).map(Number);return [p[0],p[1],p[2],p[3]===undefined?1:p[3]];};
   const over=(t,b)=>{const a=t[3];return [t[0]*a+b[0]*(1-a),t[1]*a+b[1]*(1-a),t[2]*a+b[2]*(1-a),1];};
   const lum=c=>{const f=v=>{v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4);};return 0.2126*f(c[0])+0.7152*f(c[1])+0.0722*f(c[2]);};
   const ratio=(a,b)=>{const x=lum(a),y=lum(b);return (Math.max(x,y)+0.05)/(Math.min(x,y)+0.05);};
   const avg=img=>{const cols=[...img.matchAll(/rgba?\([^)]+\)/g)].map(m=>parse(m[0])).filter(Boolean);if(!cols.length)return null;const n=cols.length;return [cols.reduce((s,c)=>s+c[0],0)/n,cols.reduce((s,c)=>s+c[1],0)/n,cols.reduce((s,c)=>s+c[2],0)/n,Math.min(1,cols.reduce((s,c)=>s+c[3],0)/n)];};
   const bgOf=e=>{const L=[];for(let n=e;n&&n.nodeType===1;n=n.parentElement){const s=getComputedStyle(n),c=parse(s.backgroundColor);if(c&&c[3]>0)L.push(c);const g=s.backgroundImage&&s.backgroundImage!=='none'?avg(s.backgroundImage):null;if(g&&g[3]>0)L.push(g);if(L.some(l=>l[3]>=0.98))break;}let acc=[10,14,15,1];for(const l of L.reverse())acc=over(l,acc);return acc;};
   const out=[],seen=new Set(),w=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT);let t;
   while((t=w.nextNode())){const e=t.parentElement;if(!e||seen.has(e)||e.closest('svg,[hidden],[inert],script,style'))continue;const txt=t.textContent.replace(/\s+/g,' ').trim();if(!txt||/^[\s·•|/–—-]+$/.test(txt))continue;
    const r=e.getBoundingClientRect(),st=getComputedStyle(e);if(!r.width||!r.height||st.visibility==='hidden'||st.display==='none'||parseFloat(st.opacity)<0.05||r.top>innerHeight*3)continue;seen.add(e);
    const bg=bgOf(e);let fg=parse(st.color),op=1;for(let n=e;n&&n.nodeType===1;n=n.parentElement)op*=parseFloat(getComputedStyle(n).opacity);fg=over([fg[0],fg[1],fg[2],fg[3]*op],bg);
    const px=parseFloat(st.fontSize),large=px>=24||(px>=18.66&&parseInt(st.fontWeight)>=700),need=large?3:4.5,rt=ratio(fg,bg);if(rt<need)out.push(`${txt.slice(0,24)} ${rt.toFixed(2)}<${need}`);}
   return out;});
  ok(low.length===0,`${label}: every text meets WCAG AA on the dark theme ${low.slice(0,6).join('; ')}`);
  if(path.startsWith('/p/')){
   ok(await page.evaluate(()=>{const c=document.getElementById('nx-state'),t=c.querySelector('.nx-state-label').textContent.trim().toLowerCase();return c.getAttribute('aria-label').toLowerCase().includes(t);}),`${label}: the state button's name says what the state is, as its text does`);
   // a panel: opened with the keyboard it takes focus, Escape closes it and gives focus back to its button
   const btn=page.locator('[data-tip-title="Alerts"]').first();await btn.focus();await page.keyboard.press('Enter');await page.waitForTimeout(1500);
   const inside=await page.evaluate(()=>{const sh=document.getElementById('nx-sheet');return !sh.hidden&&sh.contains(document.activeElement);});
   ok(inside,`${label}: a panel takes focus when it opens`);
   ok(await page.evaluate(()=>document.getElementById('nx-sheet').getAttribute('aria-label')==='Alerts'&&document.getElementById('nx-sheet').getAttribute('aria-modal')===null),`${label}: the panel is named and is not announced as modal (it can stay open beside the printer)`);
   await page.keyboard.press('Escape');await page.waitForTimeout(700);
   ok(await page.evaluate(()=>document.activeElement?.getAttribute('data-tip-title')==='Alerts'),`${label}: Escape gives focus back to the button that opened the panel`);
  }
  await page.context().close();
 }
 console.log(`${checks} checks passed`);
}finally{await browser.close();}
