// Local headless source-build verification; never connects to or operates a printer.
// Requires an explicitly approved replay server, populated asset library, and Playwright.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const url=process.env.ASSEMBLY_URL;
if(!url||!/^http:\/\/127\.0\.0\.1:\d+$/.test(url))throw Error('Set ASSEMBLY_URL to the approved loopback replay server.');
const out=path.resolve(process.env.VALIDATION_DIR||'validation/assembly');await fs.mkdir(out,{recursive:true});
const checks=[],errors=[],requests=[];
const cowlOnly=process.argv.includes('--cowl-only');
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
let page;
const check=(name,condition)=>{assert.ok(condition,name);checks.push(name);};
async function ready(){await page.waitForFunction(()=>window.__stellaAssembly?.metrics.loads.length>0&&!document.querySelector('.assembly-status')?.textContent.startsWith('Loading'),{},{timeout:120000});}
async function click(text){await page.locator('#page-assembly').getByRole('button',{name:text,exact:true}).click();}
async function shot(name){await page.screenshot({path:path.join(out,name),fullPage:true});}
try{
 page=await browser.newPage({viewport:{width:1600,height:1080},deviceScaleFactor:1});
 page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>requests.push(r.url()));
 await page.goto(url+'/service/assembly');await ready();await page.evaluate(()=>document.fonts.ready);
 check('assembly route is in existing app',await page.locator('#pages [data-page=assembly]').count()===1);
 check('real cowl imported with stable identity',(await page.evaluate(()=>window.__stellaAssembly.selected)).startsWith('stella-rapid-bltouch-cleaned:'));
 check('source is explicitly static',await page.locator('.assembly-static').textContent()==='STATIC SOURCE REVISION');
 await click('Front');await shot('01-custom-cowl-front.png');await click('Isometric');await shot('02-custom-cowl-iso.png');
 await click('File & provenance');check('source provenance is linked to selected cowl',(await page.locator('.assembly-dialog').textContent()).includes('0172e800f8606e99cf5fc1555457de8931f339ae3f6547c1630fa3b1b49a0329'));
 check('browser file-location limitation is explicit',(await page.locator('.assembly-dialog').textContent()).includes('browser cannot open'));
 const [download]=await Promise.all([page.waitForEvent('download'),page.locator('.assembly-dialog').getByRole('link',{name:'Download original file',exact:true}).click()]);
 await download.saveAs(path.join(out,'downloaded-original.stl'));check('original download has expected filename',download.suggestedFilename()==='Rapid_Burner_Cowl_BLTouch.stl');
 check('download preserves original bytes',createHash('sha256').update(await fs.readFile(path.join(out,'downloaded-original.stl'))).digest('hex')==='0172e800f8606e99cf5fc1555457de8931f339ae3f6547c1630fa3b1b49a0329');
 await shot('03-provenance.png');await page.locator('.assembly-dialog').getByRole('button',{name:'Close',exact:true}).click();
 const before=await page.evaluate(()=>window.__stellaAssembly.selected);await page.reload();await ready();check('mapping survives browser reload',await page.evaluate(()=>window.__stellaAssembly.selected)===before);
 if(cowlOnly){
  check('desktop viewer clears shared timeline',await page.evaluate(()=>document.querySelector('.assembly-workspace').getBoundingClientRect().bottom<document.querySelector('.transport').getBoundingClientRect().top));
  await click('Hide');await page.waitForFunction(()=>window.__stellaAssembly.renderer.info.render.triangles===0);
  check('hide removes the source mesh from the actual render',await page.evaluate(()=>window.__stellaAssembly.renderer.info.render.triangles)===0);
  await page.getByLabel('Find assembly component').fill('Rapid_Burner');await page.locator('.assembly-node').click();
  check('hidden component remains selectable through navigation',await page.evaluate(()=>window.__stellaAssembly.selected)===before);
  await click('Show all');await page.waitForFunction(()=>window.__stellaAssembly.renderer.info.render.triangles===9322);
  check('show all restores source triangles',await page.evaluate(()=>window.__stellaAssembly.renderer.info.render.triangles)===9322);
  await page.getByLabel('Find assembly component').fill('');await click('Front');
  const box=await page.locator('.assembly-viewport canvas').boundingBox();let hit=false;
  for(const y of [.35,.45,.55,.65]){for(const x of [.4,.5,.6]){await page.mouse.move(box.x+box.width*x,box.y+box.height*y);await page.waitForTimeout(90);if(await page.locator('.assembly-hover').isVisible()){await page.mouse.click(box.x+box.width*x,box.y+box.height*y);hit=true;break;}}if(hit)break;}
  check('pointer picking reaches actual source geometry',hit&&await page.evaluate(()=>window.__stellaAssembly.selected)===before);
  check('empty replay does not offer source motion',await page.locator('.assembly-time-buttons button:disabled').count()===4);
  await click('Isometric');
 }
 if(!cowlOnly){
 await page.getByLabel('Source assembly',{exact:true}).selectOption('voron-r2-master-reference');
 await page.waitForFunction(()=>window.__stellaAssembly?.metrics.loads.some(x=>x.id==='voron-r2-master-reference'),{},{timeout:120000});
 check('master has real selectable mesh instances',await page.evaluate(()=>window.__stellaAssembly.entries.length>100));
 await shot('04-master-isometric.png');
 await page.getByLabel('Find assembly component').fill('Z Motor Unit');
 const matches=page.locator('.assembly-node');check('Z drive is reachable through assembly navigation',await matches.count()>0);
 await matches.first().click();await click('Focus component');await click('Isolate');await shot('05-z-drive.png');
 await click('Show all');await page.getByLabel('Find assembly component').fill('');await click('Fit visible');
 }
 const rect=await page.locator('.assembly-viewport canvas').boundingBox();
 await page.mouse.move(rect.x+rect.width/2,rect.y+rect.height/2);await page.mouse.down();
 const tracking=page.evaluate(async()=>{
  const frames=[];let last=performance.now();
  for(let i=0;i<120;i++){await new Promise(requestAnimationFrame);const now=performance.now();frames.push(now-last);last=now;}
  const sorted=frames.slice(5).sort((a,b)=>a-b),gl=window.__stellaAssembly.renderer.getContext(),ext=gl.getExtension('WEBGL_debug_renderer_info');
  return {frame_median_ms:sorted[Math.floor(sorted.length/2)],frame_p95_ms:sorted[Math.floor(sorted.length*.95)],gl_renderer:ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):gl.getParameter(gl.RENDERER),...window.__stellaAssembly.metrics};
 });
 await page.mouse.move(rect.x+rect.width/2+180,rect.y+rect.height/2+40,{steps:120});await page.mouse.up();
 const performanceReport=await tracking;
 await fs.writeFile(path.join(out,'performance.json'),JSON.stringify(performanceReport,null,2));
 await click('Full screen');await page.waitForFunction(()=>document.fullscreenElement?.classList.contains('assembly-workspace'));
 await click('Isometric');
 check('fullscreen retains static-source and time controls',await page.locator('.assembly-workspace .assembly-time-buttons button').count()===4&&await page.locator('.assembly-static').isVisible());
 check('fullscreen controls fit inside the screen',await page.evaluate(()=>document.querySelector('.assembly-time').getBoundingClientRect().bottom<=innerHeight+1));
 await shot('07-fullscreen.png');await click('Full screen');await page.waitForFunction(()=>!document.fullscreenElement);
 await page.setViewportSize({width:390,height:844});await shot('06-mobile.png');
 check('mobile has no horizontal page overflow',await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
 check('no browser script errors',errors.length===0);
 check('runtime assets stay local',requests.every(r=>r.startsWith(url+'/')||r.startsWith('data:')||r.startsWith('blob:')));
 await fs.writeFile(path.join(out,'report.json'),JSON.stringify({checks,errors,requests,scope:cowlOnly?'Partial cowl-only visual check. Master assembly and Z drive remain blocked. Not installed/live/owner acceptance.':'Headless source build with real imported files; not installed/live/owner acceptance.',performance:performanceReport},null,2));
 console.log(JSON.stringify({passed:checks.length,performance:performanceReport.frame_median_ms,errors},null,2));
}catch(e){if(page)await shot('failure.png').catch(()=>{});await fs.writeFile(path.join(out,'failure.json'),JSON.stringify({error:e.stack,checks,errors,requests},null,2));throw e;}
finally{await browser.close();}
