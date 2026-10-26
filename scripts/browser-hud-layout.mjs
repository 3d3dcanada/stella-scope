// Six-layout visual evidence against a local replay. One browser/page at a time.
// Polish WO-06 (2026-10-18): the default run (six layouts captured, no failure) works against any replay. HUD_AUDIT=1 was written for the 2026-09-20
// layout: its first checks pass (page overflow, readout colours, header, transport, collisions between instruments, where a phone's job strip used to run
// into the heading: fixed in hud.css), then it opens popups the classic HUD no longer has (the controller-fan gauge, a JOG+ button in the jog row, a Tools button
// that is now Pages). Those checks are not brought up to date.
import fs from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const base=process.env.HUD_URL;
if(!/^http:\/\/127\.0\.0\.1:\d+$/.test(base||''))throw Error('HUD_URL must be a loopback replay instance');
const out=path.resolve(process.env.VALIDATION_DIR||'docs/work-orders/hud-2026-09-19/evidence/codex-2026-09-20/before');
await fs.mkdir(out,{recursive:true});
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=gl','--ignore-gpu-blocklist']});
const results=[],errors=[],requests=[];
const audit=process.env.HUD_AUDIT==='1',checks=[];
const check=(name,ok,details='')=>{assert.ok(ok,name+' '+details);checks.push(name);console.log('PASS '+name);};
try{
 for(const [width,height,tag]of [[390,844,'phone'],[1366,768,'laptop'],[1600,1080,'desktop']]){
  const page=await browser.newPage({viewport:{width,height}});
  page.on('pageerror',e=>{errors.push(e.message);console.error(e.message);});page.on('request',r=>requests.push({url:r.url(),method:r.method()}));
  await page.goto(base+'/service/machine');
  await page.waitForTimeout(300);
  if(errors.length)throw Error(errors.join('\n'));
  await page.waitForFunction(()=>window.__stellaScene?.metrics.loads.length||document.querySelector('#machine-view')?.dataset.error,null,{timeout:180000});
  const sceneError=await page.locator('#machine-view').getAttribute('data-error');if(sceneError)throw Error(sceneError);
  await page.waitForFunction(()=>document.querySelector('#capture-status')?.textContent.startsWith('Recorded '));
  await page.evaluate(()=>document.fonts.ready);
  for(const theme of ['dark','light']){
   if(theme==='light')await page.locator('.hud-header-tools button').filter({hasText:/^Light$/}).click();
   await page.waitForTimeout(700);
   await page.screenshot({path:path.join(out,`${tag}-${theme}.png`),timeout:120000});
   const layout=await page.evaluate(()=>{
    const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};
    const visible=e=>e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden';
    const selectors=['header','.hud-heading','.hud-jobstrip','.hud-viewbox','.hud-offline','.hud-rail.left','.hud-rail.right','.hud-bottom','.hud-jogrow','.hud-actions','.transport'];
    const overlays=selectors.map(s=>({selector:s,element:document.querySelector(s)})).filter(x=>x.element&&visible(x.element)).map(x=>({selector:x.selector,...rect(x.element)}));
    const controls=[...document.querySelectorAll('.hud button,.hud-header-tools button,.transport button')].filter(visible).map(e=>({text:e.textContent,...rect(e)}));
    const readouts=[...document.querySelectorAll('.hud-gauge-value')].map(e=>{const s=getComputedStyle(e);return {text:e.textContent,color:s.color,fill:s.webkitTextFillColor,opacity:s.opacity,font:s.fontSize,background:s.backgroundColor,ancestors:[e.parentElement,e.parentElement.parentElement,e.closest('.viewer')].map(p=>({class:p.className,filter:getComputedStyle(p).filter,backdrop:getComputedStyle(p).backdropFilter,opacity:getComputedStyle(p).opacity})),...rect(e)};});
    return {overlays,controls,readouts,scroll:{width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight},canvas:rect(document.querySelector('#machine-view'))};
   });
   results.push({tag,theme,width,height,...layout});console.log(`${tag}-${theme}: captured`);
   if(audit){
    const prefix=`${tag}-${theme}`;
    check(prefix+': no page overflow',layout.scroll.width<=width&&layout.scroll.height<=height,JSON.stringify(layout.scroll));
    // Polish WO-06 (2026-10-18): heat, bed, part-fan and hotend-fan readouts are drawn in their accent colour since this check was written;
    // speed and flow still use the theme's ink.
    const accents=['rgb(255, 106, 0)','rgb(245, 179, 45)','rgb(4, 217, 196)','rgb(109, 184, 255)'];
    check(prefix+': readouts use theme ink or their accent',layout.readouts.every(r=>r.color===(theme==='light'?'rgb(20, 35, 34)':'rgb(237, 244, 242)')||accents.includes(r.color)));
    const header=await page.locator('header').evaluate(e=>({width:e.scrollWidth,visible:e.clientWidth}));
    check(prefix+': header controls fit',header.width<=header.visible);
    check(prefix+': header button labels fit',await page.locator('header button').evaluateAll(elements=>elements.every(e=>e.scrollWidth<=e.clientWidth)));
    const transport=await page.locator('.transport').evaluate(d=>[...d.querySelectorAll('button,select,input')].filter(e=>e.getClientRects().length).map(e=>{
     e.scrollIntoView({block:'center',inline:'center'});const r=e.getBoundingClientRect(),at=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
     return {text:e.textContent||e.id,ok:r.top>=0&&r.bottom<=innerHeight&&r.left>=0&&r.right<=innerWidth&&(at===e||e.contains(at))};
    }));
    for(const item of transport)check(prefix+': transport reachable '+item.text,item.ok);
    await page.locator('.transport-row').evaluate(e=>e.scrollLeft=0);
    const regions=layout.overlays.filter(r=>['.hud-heading','.hud-jobstrip','.hud-viewbox','.hud-offline','.hud-rail.left','.hud-rail.right','.hud-bottom','.transport'].includes(r.selector));
    const overlaps=[];for(let i=0;i<regions.length;i++)for(let j=i+1;j<regions.length;j++){const a=regions[i],b=regions[j];if(Math.min(a.right,b.right)-Math.max(a.x,b.x)>2&&Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y)>2&&!(width<=760&&[a.selector,b.selector].includes('.transport'))&&!(width<=760&&[a.selector,b.selector].sort().join()==='.hud-heading,.hud-viewbox'))overlaps.push(a.selector+' / '+b.selector);}
    check(prefix+': instruments do not collide',!overlaps.length,overlaps.join(', '));
    if(width<=760){
     await page.locator('.hud-actions').scrollIntoViewIfNeeded();await page.screenshot({path:path.join(out,`${prefix}-controls.png`)});
     results.at(-1).controlsReadouts=await page.locator('.hud-gauge-value').evaluateAll(elements=>elements.map(e=>{const r=e.getBoundingClientRect();return {text:e.textContent,x:r.x,y:r.y,right:r.right,bottom:r.bottom};}));
    }
    if(theme==='light')for(let i=0;i<5;i++)await page.locator('.hud-gauge-value').nth(i).screenshot({path:path.join(out,`${prefix}-readout-${i}.png`)});
    const closeAll=async()=>{for(let i=0;i<8&&await page.locator('dialog[open]').count();i++)await page.keyboard.press('Escape');};
    const inspect=async name=>{
     const dialog=page.locator('dialog[open]').last();await dialog.waitFor();
     if(name==='history')await page.waitForFunction(()=>!document.querySelector('.hud-modal[open] .hud-modal-note')?.textContent.startsWith('Loading captured samples'));
     await page.waitForTimeout(120);
     const m=await dialog.evaluate(d=>{const panel=d.querySelector('.hud-modal-inner')||d,r=panel.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:panel.clientWidth,scroll:panel.scrollWidth,body:[...d.querySelectorAll('.hud-modal-body,.hud-drawer-body')].map(e=>({w:e.clientWidth,sw:e.scrollWidth}))};});
     check(prefix+': '+name+' fits viewport',m.x>=-1&&m.y>=-1&&m.right<=width+1&&m.bottom<=height+1,JSON.stringify(m));
     check(prefix+': '+name+' has no horizontal clipping',m.scroll<=m.width+1&&m.body.every(b=>b.sw<=b.w+1),JSON.stringify(m));
     await page.screenshot({path:path.join(out,`${prefix}-${name}.png`),timeout:120000});
     const hits=await dialog.evaluate(d=>[...d.querySelectorAll('button')].filter(e=>e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden').map(e=>{
      e.scrollIntoView({block:'center',inline:'nearest'});
      const r=e.getBoundingClientRect(),x=(r.left+r.right)/2,y=(r.top+r.bottom)/2,at=document.elementFromPoint(x,y);
      return {text:e.textContent,rect:{x:r.x,y:r.y,right:r.right,bottom:r.bottom},hit:at?.tagName+'.'+at?.className,ok:r.left>=-1&&r.right<=innerWidth+1&&y>=0&&y<=innerHeight&&(e===at||e.contains(at))};
     }));
     for(const hit of hits)check(prefix+': '+name+' reachable '+hit.text,hit.ok,hit.ok?'':JSON.stringify(hit));
    };
    const gaugeOpeners=[['nozzle','.hud-rail.left .hud-gauge-value',0],['bed','.hud-rail.left .hud-gauge-value',1],['part-fan','.hud-rail.right .hud-gauge-value',0],['hotend-fan','.hud-gauge[data-fan=hotend] .hud-gauge-value',0],['controller-fan','.hud-gauge[data-fan=controller] .hud-gauge-value',0],['speed','.hud-bottom .hud-gauge-value',0],['flow','.hud-bottom .hud-gauge-value',1]];
    for(const [name,selector,index]of gaugeOpeners){await page.locator(selector).nth(index).click();await inspect(name);await closeAll();}
    for(const [name,label]of [['motion','JOG+'],['prepare','Prepare'],['files','Files'],['console','Console'],['more','More']]){await page.locator('.hud-jogrow button').filter({hasText:new RegExp('^'+label.replace('+','\\+')+'$')}).click();await inspect(name);await closeAll();}
    await page.locator('.hud-header-tools button').filter({hasText:/^Colors$/}).click();await inspect('colors');await closeAll();
    await page.locator('.hud-selection button').filter({hasText:/^Details$/}).click();await inspect('part-detail');await closeAll();
    for(const label of ['Filament','Tune','Macros','Bed mesh','History','Reports','Restart']){
     await page.locator('.hud-actions button').filter({hasText:/^More$/}).click();
     await page.locator('.hud-more').filter({has:page.locator('strong',{hasText:new RegExp('^'+label+'$')})}).click();
     await inspect(label.toLowerCase().replaceAll(' ','-'));await closeAll();
    }
    await page.locator('.hud-header-tools button').filter({hasText:/^Tools$/}).click();await inspect('tools-signals');
    const toolPages=await page.locator('.hud-drawer-nav button').evaluateAll(elements=>elements.map(e=>e.dataset.page));
    assert.deepEqual([...toolPages].sort(),['assembly','signals','electronics','parts','diagnostics','maintenance','timeline','thermal','leveling'].sort(),'All nine Tools pages must be present');
    for(const id of toolPages.filter(id=>id!=='signals')){
     await page.locator(`.hud-drawer-nav button[data-page="${id}"]`).click();
     await page.locator('.hud-drawer-body').evaluate(e=>e.scrollTop=0);await inspect('tools-'+id);
    }
    await closeAll();
    // Restore the dashboard before changing theme; all phone instruments remain in one scroll surface.
    await page.locator('main').evaluate(e=>e.scrollTop=0);
   }
  }
  await page.close();
 }
}finally{await browser.close();await fs.writeFile(path.join(out,'layout.json'),JSON.stringify({results,checks,errors,requests},null,2));}
if(errors.length)throw Error(errors.join('\n'));
assert.equal(results.length,6,'Every requested viewport must finish in both themes');
if(requests.some(r=>!r.url.startsWith(base)||r.method!=='GET'))throw Error('Unexpected external or mutating request');
