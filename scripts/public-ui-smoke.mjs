// Local release UI proof. Own demo recorder, scratch ports, software-rendered browser, no printer.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createRequire} from 'node:module';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const out=process.env.VALIDATION_DIR||path.join(root,'docs/work-orders/public-release-2026-09-26/evidence/WO-07/ui');
const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const free=()=>new Promise((resolve,reject)=>{const s=net.createServer();s.once('error',reject);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p));});});
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const temp=await fs.mkdtemp(path.join(os.tmpdir(),'stella-public-ui-'));
await fs.mkdir(out,{recursive:true});
const desktop=await free(),phone=await free();assert.notEqual(desktop,phone);
const config=path.join(temp,'config.json');
await fs.writeFile(config,JSON.stringify({moonraker_url:'http://127.0.0.1:1',bind:'127.0.0.1',port:desktop,lan_bind:'127.0.0.1',lan_port:phone,remote_link:false,failure_watch:false,controls_enabled:false,desktop_notifications:false,database:path.join(temp,'demo.sqlite3')}));
const service=spawn(path.join(root,'target/release/stella-scope'),['--config',config,'--demo'],{cwd:temp,stdio:['ignore','pipe','pipe']});
let log='';for(const stream of [service.stdout,service.stderr])stream.on('data',b=>log+=b.toString());
let browser;
try{
 const base=`http://127.0.0.1:${desktop}`,phoneBase=`http://127.0.0.1:${phone}`;
 let ready=false;for(let i=0;i<100;i++){if(service.exitCode!==null)throw Error('scratch recorder exited: '+log);try{const r=await fetch(base+'/api/meta');if(r.ok){ready=true;break;}}catch{}await pause(100);}assert(ready,'scratch recorder did not start: '+log);
 browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=swiftshader']});
 const errors=[];
 for(const [label,width,height] of [['phone',390,844],['laptop',1366,768],['desktop',1600,900]]){
  const page=await browser.newPage({viewport:{width,height}});page.on('pageerror',e=>errors.push(label+': '+e.message));
  await page.goto(base+'/',{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>document.body.dataset.scene==='off',null,{timeout:15000});
  const result=await page.evaluate(()=>({overflow:document.documentElement.scrollWidth-innerWidth,loading:!!document.querySelector('#nx-loading')&&getComputedStyle(document.querySelector('#nx-loading')).opacity!=='0',fallback:document.querySelector('#nx-scene')?.hidden===true}));
  assert(result.fallback&&!result.loading,`${label} fallback/LOADING: ${JSON.stringify(result)}`);
  assert(result.overflow<=2,`${label} horizontal overflow: ${result.overflow}px`);
  await page.screenshot({path:path.join(out,`${label}-fallback.png`)});
  console.log(`${label}: fallback visible, no LOADING, overflow ${result.overflow}px`);
  await page.close();
 }
 assert.deepEqual(errors,[]);
 for(const [route,label] of [['/classic-hud','classic-hud'],['/service/diagnostics','service-diagnostics'],['/workspace','workspace']]){
  const legacy=await browser.newPage({viewport:{width:1366,height:768}}),routeErrors=[];
  legacy.on('pageerror',e=>routeErrors.push(e.message));
  const response=await legacy.goto(base+route,{waitUntil:'domcontentloaded'});await pause(900);
  const view=await legacy.evaluate(()=>({title:document.title,text:document.body.innerText.length,overflow:document.documentElement.scrollWidth-innerWidth}));
  assert.equal(response.status(),200,`${route} did not load`);
  assert(view.text>40,`${route} was empty`);assert.deepEqual(routeErrors,[],`${route} script errors`);
  await legacy.screenshot({path:path.join(out,`${label}.png`)});
  console.log(`${route}: visible (${view.text} text characters), overflow ${view.overflow}px`);
  await legacy.close();
 }
 const page=await browser.newPage({viewport:{width:1366,height:768}});
 const tiny={root:{name:'Fixture',meshes:[0],children:[]},meshes:[{name:'Fixture triangle',attributes:{position:{array:[0,0,0,30,0,0,0,30,0]}},index:{array:[0,1,2]},color:[0.1,0.8,0.7],mapping:{}}]};
 await page.route('**/api/assets',r=>r.fulfill({contentType:'application/json',body:JSON.stringify({assets:[{id:'stella-printer',geometry_url:'/fixture-small-geometry',source_sha256:'0'.repeat(64)}]})}));
 await page.route('**/fixture-small-geometry',r=>r.fulfill({contentType:'application/json',body:JSON.stringify(tiny)}));
 await page.goto(base+'/',{waitUntil:'domcontentloaded'});
 await page.waitForFunction(()=>['ready','off'].includes(document.body.dataset.scene),null,{timeout:15000});
 const small=await page.evaluate(()=>({scene:document.body.dataset.scene,renderer:document.querySelector('#nx-scene')?.dataset.renderer,error:document.querySelector('#nx-scene')?.dataset.error,entries:window.__stellaScene?.entries?.length}));
 assert.equal(small.scene,'ready',`small CAD did not leave LOADING: ${JSON.stringify(small)}`);assert.equal(small.entries,1);
 await page.screenshot({path:path.join(out,'small-cad.png')});console.log('small CAD: renderer ready with one mesh');
 const presets=await page.evaluate(async()=>await (await fetch('/api/presets')).json());assert.deepEqual(presets.materials,[]);console.log('first run: no material heat targets');
 const pair=await page.evaluate(async()=>await (await fetch('/api/pair/new',{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'})).json());assert.match(pair.code,/^\d{6}$/);
 const paired=await browser.newPage({viewport:{width:390,height:844}});await paired.goto(phoneBase+'/',{waitUntil:'domcontentloaded'});
 const reply=await paired.evaluate(async code=>{const r=await fetch('/api/pair',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({code})});return r.status;},pair.code);assert.equal(reply,200);
 await paired.goto(phoneBase+'/',{waitUntil:'domcontentloaded'});
 // UI rebuild 2026-10-22: Pages is one key of the standing pair (Pages, Find) at the end of the phone's gauges tier; it used to be a spoke of the More radial in the bar. The test asserted that route.
 await paired.locator('.nxkey[data-key=pages]').click();
 await paired.locator('#nx-sheet a').filter({hasText:'Phone & WhatsApp'}).click();
 await paired.getByText('This phone is paired. Pairing and alerts are set up from the Stella Scope window on the computer.').waitFor();
 await paired.screenshot({path:path.join(out,'paired-phone-sheet.png')});console.log('paired phone: desktop-only settings hidden');
 await paired.close();await page.close();
}finally{
 await browser?.close();if(service.exitCode===null){service.kill('SIGTERM');await new Promise(r=>service.once('exit',r));}await fs.rm(temp,{recursive:true,force:true});
}
