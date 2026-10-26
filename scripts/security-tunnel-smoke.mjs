// Phone path, end to end on this computer: two scratch demo recorders, each reached through a real
// stella-link tunnel (link/examples/local-pair: iroh on 127.0.0.1, relays and lookup off) from one
// Chromium profile, as one phone with two printers. Proves one-scan pairing through the tunnel, one
// browser cookie per printer, no cookie crossing between printers, revocation, and a full NEXT page
// load through the stream caps. 2026-09-27: the link admits only paired phones (or any phone while a
// pairing code shows), and a device key works only from the phone that paired it: a third tunnel is a
// second phone to printer A. Nothing leaves this computer; no printer, no live port.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createRequire} from 'node:module';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const free=()=>new Promise((resolve,reject)=>{const s=net.createServer();s.once('error',reject);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p));});});
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const temp=await fs.mkdtemp(path.join(os.tmpdir(),'stella-tunnel-smoke-'));
const children=[];let browser;
async function recorder(name){
 const desktop=await free(),phone=await free(),dir=path.join(temp,name);await fs.mkdir(dir);
 const config=path.join(dir,'config.json');
 await fs.writeFile(config,JSON.stringify({moonraker_url:'http://127.0.0.1:1',bind:'127.0.0.1',port:desktop,lan_bind:'127.0.0.1',lan_port:phone,remote_link:false,failure_watch:false,controls_enabled:false,desktop_notifications:false,database:path.join(dir,'demo.sqlite3')}));
 const p=spawn(path.join(root,'target/release/stella-scope'),['--config',config,'--demo'],{cwd:dir,stdio:'ignore'});children.push(p);
 const base=`http://127.0.0.1:${desktop}`;
 for(let i=0;i<100;i++){try{if((await fetch(base+'/api/meta')).ok)return {name,base,phone,allow:path.join(dir,'stella-scope-demo.sqlite3.link.allow')/* --demo uses this name in its working directory */};}catch{}await pause(100);}
 throw Error(name+' recorder did not start');}
const desk=(r,p,body)=>fetch(r.base+p,{method:body?'POST':'GET',headers:body?{'Content-Type':'application/json',Origin:r.base}:{},body:body?JSON.stringify(body):undefined}).then(x=>x.json());
try{
 const A=await recorder('a'),B=await recorder('b');
 const link=spawn(path.join(root,'link/target/release/examples/local-pair'),['--forward',`127.0.0.1:${A.phone}=${A.allow}`,'--forward',`127.0.0.1:${B.phone}=${B.allow}`,'--forward',`127.0.0.1:${A.phone}=${A.allow}`],{stdio:['ignore','pipe','inherit']});children.push(link);
 const lines=await new Promise((resolve,reject)=>{let out='';const t=setTimeout(()=>reject(Error('tunnels did not start: '+out)),30000);link.stdout.on('data',b=>{out+=b;const l=out.trim().split('\n');if(l.length>=3){clearTimeout(t);resolve(l);}});});
 for(const [r,line] of [[A,lines[0]],[B,lines[1]]]){const [,url,id]=line.split(' ');r.url=url.replace(/\/$/,'');r.id=id;}
 const A2={};{const [,url,id]=lines[2].split(' ');A2.url=url.replace(/\/$/,'');A2.id=id;}
 console.log(`tunnels up: A ${A.url}, B ${B.url}`);
 browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=swiftshader']});
 const phone=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
 const page=await phone.newPage();const failed=[];page.on('requestfailed',r=>failed.push(r.url()+' '+r.failure()?.errorText));
 // a navigation that the link refuses fails outright; otherwise its HTTP status
 // (its own tab each time: a failed load leaves Chrome's error page committing late in that tab)
 const reach=async(pg,u)=>{const t=await pg.context().newPage();try{return await t.goto(u,{timeout:20000}).then(r=>r.status(),()=>'refused');}finally{await t.close();}};
 // no pairing window, not paired: the link closes the connection before any HTTP
 assert.equal(await reach(page,A.url+'/api/meta'),'refused');console.log('stranger phone, no pairing window: refused at the link');
 // one-scan pairing, as the app does it: open /pair#code= on the tunnel port
 for(const r of [A,B]){
  const {code}=await desk(r,'/api/pair/new',{});
  await page.goto(`${r.url}/pair#code=${code}`);
  await page.waitForURL(u=>u.pathname==='/',{timeout:20000});
  console.log(`${r.name}: paired through the tunnel with code from the desktop`);
 }
 const cookies=await phone.cookies();const names=cookies.map(c=>c.name).sort();
 assert.deepEqual(names,[`stella_device_${A.id}`,`stella_device_${B.id}`].sort(),'one cookie per printer, none under the shared name: '+names);
 assert(cookies.every(c=>c.httpOnly&&c.sameSite==='Strict'),'cookies stay HttpOnly and Strict');
 console.log('cookies: one per printer, HttpOnly, SameSite=Strict');
 const status=r=>reach(page,r.url+'/api/meta');
 assert.equal(await status(A),200);assert.equal(await status(B),200);console.log('both printers answer the paired phone: 200 / 200');
 // a phone-origin page cannot post to the other printer (same site, other origin)
 await page.goto(B.url+'/pair');
 // the pair page may move on by itself once it sees the device is already paired: wait it out
 let cross;
 for(let i=0;i<6&&cross===undefined;i++){
  try{await page.waitForLoadState('load');await pause(400);cross=await page.evaluate(u=>fetch(u+'/api/pair/new',{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'}).then(x=>x.status).catch(()=>'blocked'),A.url);}
  catch(e){if(!/context was destroyed|navigation/i.test(String(e)))throw e;}
 }
 assert(cross==='blocked'||cross===403,'cross-printer post: '+cross);console.log('B page posting to A: '+cross);
 // revoke on B's desktop: B refuses, A still answers
 const dev=(await desk(B,'/api/pair/devices')).devices;assert.equal(dev.length,1);
 await desk(B,'/api/pair/revoke',{id:dev[0].id});
 assert.equal(await status(B),'refused');assert.equal(await status(A),200);console.log('revoked on B: B refused at the link, A still 200');
 // a second phone with phone 1's key for A: refused at the link; with a pairing window open it gets
 // through the link but the key is bound to phone 1, so the recorder answers 401
 const key=(await phone.cookies()).find(c=>c.name===`stella_device_${A.id}`).value;
 const other=await browser.newContext();const op=await other.newPage();
 await other.addCookies([{name:`stella_device_${A2.id}`,value:key,url:A2.url,httpOnly:true,sameSite:'Strict'}]);
 assert.equal(await reach(op,A2.url+'/api/meta'),'refused');
 await desk(A,'/api/pair/new',{});
 assert.equal(await reach(op,A2.url+'/api/meta'),401);
 assert.equal(await status(A),200);
 console.log("second phone with phone 1's key: refused at the link, 401 inside a pairing window; phone 1 still 200");
 await other.close();
 // full NEXT page through the tunnel: every module, sprite and font over the stream caps
 failed.length=0;
 await page.goto(A.url+'/',{waitUntil:'load',timeout:60000});
 await page.waitForFunction(()=>document.body.dataset.scene==='off',null,{timeout:20000});await pause(2000);
 const cards=await page.evaluate(()=>document.querySelectorAll('.nx-stagecard').length);
 const desktopOnly=await page.evaluate(async()=>{const c=await fetch('/api/capabilities').then(x=>x.json());return c;});
 const real=failed.filter(f=>!/api\/camera|tick\/stream/.test(f)&&!/ERR_ABORTED$/.test(f));// ERR_ABORTED = the page cancelled its own superseded request
 assert.deepEqual(real,[],'requests failed through the tunnel: '+real.join('; '));
 assert.equal(cards,1,'one camera card on a phone');
 assert.deepEqual(desktopOnly,{desktop:false,phone:true});
 console.log(`NEXT page through tunnel: loaded, 0 failed requests, ${cards} camera card, capabilities phone-only`);
 // two phone pages at once plus burst of parallel requests: nothing dropped by the caps
 const second=await phone.newPage();await second.goto(A.url+'/',{waitUntil:'load',timeout:60000});
 const burst=await second.evaluate(()=>Promise.all(Array.from({length:24},()=>fetch('/api/meta').then(r=>r.status).catch(()=>0))));
 assert(burst.every(s=>s===200),'burst: '+burst.join(','));console.log('24 parallel requests with two pages open: all 200');
 // A farm through one tunnel (farm board WO-11): a live service with two printers, one pairing for the
 // whole farm. The phone's home is the farm page, the second printer's routes answer on the same
 // cookie, an unknown printer is 404, and nothing needs a second pairing.
 {
  const fakes=[];
  for(const port of [await free(),await free()]){
   const f=spawn(path.join(root,'target/release/examples/fake-printers'),['moonraker',String(port)],{stdio:'ignore'});children.push(f);fakes.push(port);
  }
  await pause(600);
  const desktop=await free(),phonePort=await free(),dir=path.join(temp,'farm');await fs.mkdir(dir);
  const config=path.join(dir,'config.json');
  await fs.writeFile(config,JSON.stringify({moonraker_url:`http://127.0.0.1:${fakes[0]}`,bind:'127.0.0.1',port:desktop,lan_bind:'127.0.0.1',lan_port:phonePort,remote_link:false,failure_watch:false,controls_enabled:true,desktop_notifications:false,
   database:path.join(dir,'farm.sqlite3'),printers:[{id:'stella',name:'Stella',address:`http://127.0.0.1:${fakes[0]}`,database:path.join(dir,'farm.sqlite3')},{id:'bench',name:'Bench',address:`http://127.0.0.1:${fakes[1]}`}]}));
  const proc=spawn(path.join(root,'target/release/stella-scope'),['--config',config],{cwd:dir,stdio:'ignore'});children.push(proc);
  const F={name:'farm',base:`http://127.0.0.1:${desktop}`,phone:phonePort,allow:path.join(dir,'farm.sqlite3.link.allow')};
  for(let i=0;i<100;i++){try{if((await fetch(F.base+'/api/meta')).ok)break;}catch{}await pause(100);}
  for(let i=0;i<100;i++){const d=await fetch(F.base+'/api/printers').then(r=>r.json()).catch(()=>null);if(d?.printers?.length===2)break;await pause(100);}
  const l2=spawn(path.join(root,'link/target/release/examples/local-pair'),['--forward',`127.0.0.1:${F.phone}=${F.allow}`],{stdio:['ignore','pipe','inherit']});children.push(l2);
  const line=await new Promise((resolve,reject)=>{let out='';const t=setTimeout(()=>reject(Error('farm tunnel did not start: '+out)),30000);l2.stdout.on('data',b=>{out+=b;if(out.includes('\n')){clearTimeout(t);resolve(out.trim().split('\n')[0]);}});});
  const [,furl,fid]=line.split(' ');F.url=furl.replace(/\/$/,'');F.id=fid;
  const fphone=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  const fp=await fphone.newPage();const ffailed=[];fp.on('requestfailed',r=>ffailed.push(r.url()+' '+r.failure()?.errorText));
  const {code}=await desk(F,'/api/pair/new',{});
  await fp.goto(`${F.url}/pair#code=${code}`);
  await fp.waitForURL(u=>u.pathname==='/',{timeout:20000});
  // The paired phone's home is the farm: both printers, one tile each.
  console.log('farm phone page:',fp.url(),JSON.stringify(await fp.title()),'printers on the desktop:',(await desk(F,'/api/printers')).printers?.map(p=>p.id).join(','));
  await fp.waitForSelector('.fm-root',{timeout:30000});
  await fp.waitForFunction(()=>document.querySelectorAll('article').length===2,null,{timeout:30000});
  const tiles=await fp.evaluate(()=>[...document.querySelectorAll('article')].map(a=>a.innerText.split('\n')[0]));
  assert.deepEqual(tiles.sort(),['Bench','Stella'].sort(),'the farm page lists both printers: '+tiles);
  console.log('farm through the tunnel: the phone home is the farm page with '+tiles.join(' and '));
  // Both printers' routes answer on the one cookie; an unknown printer does not.
  const get=u=>fp.evaluate(async x=>(await fetch(x)).status,u);
  assert.equal(await get('/api/p/bench/meta'),200);assert.equal(await get('/api/p/stella/meta'),200);
  assert.equal(await get('/api/p/nosuch/meta'),404);
  const names=(await fphone.cookies()).map(c=>c.name);
  assert.deepEqual(names,[`stella_device_${F.id}`],'one pairing, one cookie, for the whole farm: '+names);
  console.log('second printer route through the tunnel: /api/p/bench/meta 200, unknown printer 404, one cookie');
  // The second printer's own page loads as the app does for a tapped alert (stella://p/bench).
  await fp.goto(F.url+'/p/bench',{waitUntil:'load',timeout:60000});
  await fp.waitForFunction(()=>/Bench/.test(document.title),null,{timeout:20000});
  const ctl=await fp.evaluate(async()=>{const r=await fetch('/api/p/bench/controls');return {status:r.status,body:await r.json()};});
  assert.equal(ctl.status,200);assert.equal(typeof ctl.body.enabled,'boolean');
  const realF=ffailed.filter(f=>!/api\/camera|tick\/stream|print\/thumbnail/.test(f)&&!/ERR_ABORTED$/.test(f));
  assert.deepEqual(realF,[],'requests failed through the farm tunnel: '+realF.join('; '));
  console.log("printer page /p/bench through the tunnel: loaded, its controls door answers, 0 failed requests");
 }
 console.log('tunnel smoke: all checks passed');
}finally{
 if(browser)await browser.close();
 for(const c of children){try{c.kill('SIGTERM');}catch{}}
 await pause(300);await fs.rm(temp,{recursive:true,force:true});
}
