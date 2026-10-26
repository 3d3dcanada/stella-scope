// HUD evidence run against an approved loopback replay or demo instance. No printer operations:
// the run asserts that no POST leaves the page. Usage:
//   HUD_URL=http://127.0.0.1:17422 PLAYWRIGHT_MODULE=/path/to/playwright node scripts/browser-hud.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const base=process.env.HUD_URL;
if(!base||!/^http:\/\/127\.0\.0\.1:\d+$/.test(base))throw Error('HUD_URL must be a loopback replay or demo instance.');
const out=path.resolve(process.env.VALIDATION_DIR||'docs/work-orders/hud-2026-09-19/evidence');await fs.mkdir(out,{recursive:true});
// Polish WO-06 (2026-10-18): a check that needs our September replay recording (a printer that is off, axes not yet homed, the head at
// the CAD pose) runs only with HUD_FIXTURE=replay-2026-09; on a demo instance it is skipped, in words. HUD_CONTINUE=1 lists every failure instead of stopping.
const fixture=async(name,run)=>process.env.HUD_FIXTURE==='replay-2026-09'?check(name,await run()):console.log('SKIP (needs the 2026-09 replay recording) '+name);
const checks=[],failed=[],errors=[],requests=[];const check=(name,condition)=>{if(!condition&&process.env.HUD_CONTINUE==='1'){failed.push(name);console.log('FAIL '+name);return;}assert.ok(condition,name);checks.push(name);console.log('PASS '+name);};
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=gl','--ignore-gpu-blocklist']});
const open=async(width,height)=>{const page=await browser.newPage({viewport:{width,height}});page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>requests.push({url:r.url(),method:r.method()}));await page.goto(base+'/service/machine');await page.waitForFunction(()=>window.__stellaScene?.metrics.loads.length,{},{timeout:180000});await page.waitForTimeout(2500);return page;};
const shot=(page,name)=>page.screenshot({path:path.join(out,name),timeout:120000});
try{
 for(const [w,h,tag]of [[1600,1080,'desktop'],[1366,768,'laptop'],[390,844,'phone']]){
  const page=await open(w,h);
  // Desktop and laptop are one fixed screen. A phone stacks the controls under the machine view,
  // so that one surface may scroll; the 3D view itself still never does.
  check(`${tag}: ${w<=760?'machine view stays put while the control stack scrolls':'no page scroll'}`,
   await page.evaluate(()=>innerWidth<=760?document.querySelector('.canvas-wrap').getBoundingClientRect().height>=innerHeight*0.3
    :document.documentElement.scrollHeight<=innerHeight&&document.body.scrollHeight<=innerHeight));
  // On a phone the instruments flow under the canvas by design; on a desktop the canvas is the page.
  check(`${tag}: canvas takes the room it should`,await page.evaluate(()=>{const r=document.getElementById('machine-view').getBoundingClientRect();
   return r.width>=innerWidth-2&&(innerWidth<=760?r.height>=innerHeight*0.3:r.height>=innerHeight*0.48);}));
  check(`${tag}: six gauges carry a value or a dash (the controller-fan gauge was folded away after this check was written)`,await page.evaluate(()=>{const v=[...document.querySelectorAll('.hud-gauge-value')].map(e=>e.textContent);return v.length===6&&v.every(t=>/^(—|-?\d+(\.\d+)?[°%])$/.test(t));}));
  await shot(page,`${tag}.png`);await page.close();
 }
 const page=await open(1600,1080);
 check('genuine 350 source CAD loads',await page.locator('#machine-view').getAttribute('data-renderer')==='source-cad');
 check('published toolhead CAD loads, including the disclosed blower stand-ins',await page.evaluate(()=>{const want=['stella-rapid-bltouch-cleaned','stella-sherpa-mini-mount','stella-ebb42-carrier','annex-sherpa-mini','btt-ebb42-v12','bltouch-probe','fan-3010-axial'];const s=window.__stellaScene;return want.every(id=>{const e=s.entries.find(e=>e.asset.id===id);return e&&!e.sourceOnly&&!e.representative&&s.visibleParts.includes(e.id);})&&s.entries.filter(e=>e.id.startsWith('stella:blower')).length===2&&!s.entries.some(e=>e.representative);}));
 check('cowl bracket sits at -X (left from the front) after the documented Y mirror',await page.evaluate(()=>{const s=window.__stellaScene,e=s.entries.find(e=>e.asset.id==='stella-rapid-bltouch-cleaned'),b=s.geometryBounds(e.id);return Math.abs(b.min[0]-88)<.01&&Math.abs(b.max[0]-154.50226)<.01&&Math.abs(b.size[1]-40)<.01&&b.size[0]>66.4;}));
 check('stock Clockwork parts are replaced',await page.evaluate(()=>window.__stellaScene.entries.filter(e=>e.replacedByRapid&&/Extruder Body|Blower Housing|Hotend Fan Mount/.test(e.name)).length>=4));
 check('motion datum is established from the nozzle tip',await page.evaluate(()=>{const p=JSON.parse(document.getElementById('machine-view').dataset.pose);return p.mapped===true&&document.getElementById('machine-view').dataset.motion==='mapped';}));
 await fixture('unhomed axes rest at the CAD pose',()=>page.evaluate(()=>{const p=JSON.parse(document.getElementById('machine-view').dataset.pose).position;return Math.abs(p[0]-175)<.01&&Math.abs(p[2]-28.785)<.01;}));
 check('no card rail, no attention grid, no legacy nav',await page.evaluate(()=>['.hud-instrument','.hud-attention'].every(s=>!document.querySelector(s))&&getComputedStyle(document.getElementById('pages')).display==='none'));
 check('state rim follows the machine state',['idle','heating','printing','paused','error'].includes(await page.locator('.cockpit').getAttribute('data-state')));
 // every control popup opens, carries its G-code, and closes by clicking outside
 const openers={'Nozzle':()=>document.querySelectorAll('.hud-rail.left .hud-gauge-value')[0].click(),
  'Part cooling':()=>document.querySelector('.hud-rail.right .hud-gauge-value').click(),
  'Print speed':()=>document.querySelectorAll('.hud-bottom .hud-gauge-value')[0].click(),
  'Flow':()=>document.querySelectorAll('.hud-bottom .hud-gauge-value')[1].click(),
  'Motion':()=>[...document.querySelectorAll('.hud-actions button')].find(b=>b.textContent==='JOG+')?.click(),
  'More':()=>[...document.querySelectorAll('.hud-actions button')].find(b=>b.textContent==='More')?.click(),
  'Console':()=>[...document.querySelectorAll('.hud-actions button')].find(b=>b.textContent==='Console')?.click()};
 for(const [title,open]of Object.entries(openers)){
  await page.evaluate(open);await page.waitForTimeout(500);
  check(`${title} popup opens`,await page.evaluate(t=>{const m=[...document.querySelectorAll('.hud-modal')].find(d=>d.open&&d.querySelector('h2')?.textContent===t);return Boolean(m);},title));
  if(title==='Console')await shot(page,'console.png');
  await page.keyboard.press('Escape');await page.waitForTimeout(250);
 }
 await page.evaluate(()=>document.querySelectorAll('.hud-rail.left .hud-gauge-value')[0].click());await page.waitForTimeout(500);
 await fixture('preheat offers every material with both temperatures',()=>page.evaluate(()=>{const m=document.querySelector('.hud-modal[open] .hud-material-grid');return m&&m.children.length===5&&/215.*60/.test(m.children[0].textContent);}));
 await shot(page,'preheat.png');
 await page.evaluate(()=>{const d=document.querySelector('.hud-modal[open]');const r=d.getBoundingClientRect();d.dispatchEvent(new MouseEvent('click',{clientX:r.left+4,clientY:r.top+4,bubbles:true}));});
 await page.waitForTimeout(300);
 check('clicking outside closes a popup',await page.evaluate(()=>!document.querySelector('.hud-modal[open]')));
 check('permanent motion row reaches every axis, and the action row has home and QGL',await page.evaluate(()=>{const t=[...document.querySelectorAll('.hud-jogrow button')].map(b=>b.getAttribute('aria-label')||b.textContent);const a=[...document.querySelectorAll('.hud-actions button')].map(b=>b.textContent);return ['X−','X+','Y−','Y+','Z−','Z+'].every(x=>t.includes(x))&&['HOME','QGL'].every(x=>a.includes(x));}));
 await fixture('offline state is stated plainly, not hidden in a badge',()=>page.evaluate(()=>{const b=document.querySelector('.hud-offline');return b.dataset.on==='1'&&b.textContent.length>40;}));
 await page.evaluate(()=>document.querySelector('.hud-selection button:nth-of-type(2)').click());await page.waitForTimeout(800);
 await fixture('part popup opens with a model and only the signals wired to that part',()=>page.evaluate(()=>{
  const d=document.getElementById('detail-dialog');const rows=[...document.querySelectorAll('.hud-component-reading span')].map(e=>e.textContent);
  return d.open&&rows.length>=2&&Boolean(document.querySelector('.hud-part-model canvas'))&&!rows.includes('CONTROLLER FAN');}));
 await shot(page,'part-popup.png');await page.keyboard.press('Escape');await page.waitForTimeout(300);
 await page.evaluate(()=>document.querySelector('.hud-actions .hud-primary').click());await page.waitForTimeout(1200);
 check('prepare modal lists the routine and every calibration',await page.evaluate(()=>document.getElementById('hud-prepare').open&&document.querySelectorAll('#hud-prepare .hud-checklist li').length>=5&&document.querySelectorAll('#hud-prepare .hud-calibrations button').length===11));
 await fixture('jogging unhomed asks to home rather than moving',()=>page.evaluate(async()=>{const b=[...document.querySelectorAll('.hud-jogrow button')].find(b=>(b.getAttribute('aria-label')||b.textContent)==='X+');b.click();await new Promise(r=>setTimeout(r,400));return /not homed/i.test(document.querySelector('.hud-modal[open] .hud-command-status').textContent);}));
 await page.keyboard.press('Escape');await page.waitForTimeout(300);
 await page.evaluate(()=>[...document.querySelectorAll('.hud-header-tools button')].find(b=>b.textContent==='Pages')?.click());await page.waitForTimeout(800);
 await page.evaluate(()=>[...document.querySelectorAll('#hud-pages button')].find(b=>/Signals/.test(b.textContent))?.click());await page.waitForTimeout(1200);
 check('a page opens in the drawer over the printer and keeps it mounted (the Pages list was called Tools when this check was written)',await page.evaluate(()=>document.getElementById('hud-drawer').open&&!document.getElementById('page-machine').hidden&&document.querySelectorAll('.hud-drawer-nav button').length>=9));
 await shot(page,'drawer.png');
 await page.evaluate(()=>window.__stellaScene.focus('stella-rapid-bltouch-cleaned:root:0'));await page.evaluate(()=>window.__stellaScene.view('front'));await page.evaluate(()=>window.__stellaScene.zoom(1.5));await page.waitForTimeout(800);await shot(page,'toolhead-front.png');
 const liveFrames=await page.evaluate(async()=>{const s=window.__stellaScene,live=s.live,wait=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))),base={nozzle:{temperature:61.234},bed:{temperature:41.234},fans:{fan:{speed:0},hotend_fan:{speed:0}}};s.live=()=>{};try{await wait();s.metrics.frames.length=0;live(base);await wait();const first=s.metrics.frames.length;live(structuredClone(base));await wait();const same=s.metrics.frames.length;const changed={...base,nozzle:{temperature:62.345}};live(changed);await wait();const next=s.metrics.frames.length,current=s.selected?.id||null;s.select(current?null:s.entries[0].id);await wait();const selected=s.metrics.frames.length;live(changed);await wait();const restored=s.metrics.frames.length;s.select(current);return {first,same,next,selected,restored};}finally{s.live=live;}});
 // a demo head never stands still, so it draws a frame of its own between two of these counts: the check needs a recording that holds still
 await fixture('identical live visuals skip frames; changes and selection restore redraw',async()=>liveFrames.first===1&&liveFrames.same===1&&liveFrames.next===2&&liveFrames.restored===liveFrames.selected+1);
 check('no script errors',errors.length===0);
 check('only local requests and no POST left the page',requests.every(r=>r.url.startsWith(base))&&!requests.some(r=>r.method!=='GET'));
 await fs.writeFile(path.join(out,'report.json'),JSON.stringify({base,checks,errors,requests:requests.length,at:new Date().toISOString()},null,1));
 console.log(`${checks.length} checks passed`+(failed.length?`, ${failed.length} failed: ${failed.join(' | ')}`:''));if(failed.length)process.exitCode=1;
}finally{await browser.close();}
