import fs from 'node:fs/promises';
import {openSync,closeSync,readFileSync,writeFileSync,existsSync} from 'node:fs';
import {spawn,execFileSync} from 'node:child_process';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {join,resolve,dirname} from 'node:path';
import {homedir} from 'node:os';
import net from 'node:net';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const lockPath=join(root,'docs/next-ui/.heavy.lock'),logPath=join(root,'docs/next-ui/LOG.md');
const sourceAssets=join(homedir(),'.local/share/stella-scope/stella.sqlite3.assets');
const sourceConfig=join(homedir(),'.config/stella-scope/stella-scope.json');
const binary=join(root,'target/release/stella-scope');
const allStates=['off','unhomed','idle','heating','printing','paused','complete','halted','error_hot'];
const classes={
 'desktop-xl':[2087,1000,false,1],desktop:[1568,751,false,1],
 'tablet-land':[1180,820,true,1],'tablet-port':[820,1180,true,1],
 kiosk:[800,480,true,1],'kiosk-wide':[1024,600,true,1],
 phone:[390,844,true,2],'phone-land':[844,390,true,2]
};
const stamp=()=>new Date().toISOString();
const append=(phase,message)=>fs.appendFile(logPath,`${stamp()} codex C3 ${phase} ${message}\n`);
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const args=process.argv.slice(2),options={states:allStates,classes:Object.keys(classes),paths:['/next'],out:join(root,'docs/next-ui/matrix',stamp().slice(0,16).replaceAll('-','').replace('T','-').replace(':',''))};
for(let i=0;i<args.length;i+=2){
 const key=args[i],value=args[i+1];
 if(!['--states','--classes','--paths','--out'].includes(key)||!value)throw Error(`Unknown or incomplete option: ${key}`);
 options[key.slice(2)]=key==='--out'?resolve(value):value.split(',');
}
for(const s of options.states)if(!allStates.includes(s))throw Error(`Unknown state: ${s}`);
for(const c of options.classes)if(!Object.hasOwn(classes,c))throw Error(`Unknown class: ${c}`);
for(const p of options.paths)if(!['/','/next'].includes(p))throw Error(`Path must be / or /next: ${p}`);
if(!options.states.length||!options.classes.length||!options.paths.length)throw Error('Choose at least one state, class and path');

let aborted=false,activeChild=null,fake=null,app=null,browser=null,browserServer=null,browserPid=null,held=false,scratch=null;
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{
 aborted=true;
 for(const child of [activeChild,fake,app])if(child?.pid&&child.exitCode===null)child.kill('SIGTERM');
 browserServer?.close().catch(()=>{});
});
const checkAbort=()=>{if(aborted)throw Error('interrupted');};
function availableMb(){
 const output=execFileSync('free',['-m'],{encoding:'utf8'});
 const row=output.split('\n').find(line=>line.startsWith('Mem:'));
 const available=Number(row?.trim().split(/\s+/)[6]);
 if(!Number.isFinite(available))throw Error(`Cannot read available RAM from free -m: ${output}`);
 console.log(`free -m: ${available} MB available`);
 return available;
}
async function waitForRam(){
 const until=Date.now()+15*60_000;
 while(availableMb()<2500){
  checkAbort();if(Date.now()>=until)throw Error('RAM stayed below 2500 MB for 15 minutes');
  await wait(60_000);
 }
}
async function checkRamNow(){await waitForRam();}
async function takeLock(what){
 const until=Date.now()+15*60_000;
 while(true){
  checkAbort();
  try{
   const fd=openSync(lockPath,'wx');
   const line=`codex ${process.pid} ${stamp()} ${what}\n`;
   writeFileSync(fd,line);closeSync(fd);
   if(readFileSync(lockPath,'utf8')!==line)throw Error('Heavy lock content changed after taking it');
   held=true;return;
  }catch(e){
   if(e.code!=='EEXIST')throw e;
   const line=await fs.readFile(lockPath,'utf8').catch(()=>null);
   if(line===null)continue;
   const [,pidText,time]=line.trim().split(/\s+/),pid=Number(pidText);
   let alive=false;try{process.kill(pid,0);alive=true;}catch(e){alive=e.code==='EPERM';}
   if(!alive||!Number.isFinite(Date.parse(time))||Date.now()-Date.parse(time)>30*60_000){
    await append('blocked',`stale heavy lock removed: ${line.trim()}`);
    await fs.unlink(lockPath).catch(e=>{if(e.code!=='ENOENT')throw e;});continue;
   }
   if(Date.now()>=until)throw Error(`Heavy lock remained held for 15 minutes: ${line.trim()}`);
   await wait(60_000);
  }
 }
}
async function releaseLock(){
 if(!held)return;
 const line=await fs.readFile(lockPath,'utf8').catch(()=>null);
 if(line?.startsWith(`codex ${process.pid} `))await fs.unlink(lockPath);
 held=false;
}
async function heavy(label,fn){
 await waitForRam();await takeLock(label);await append('started',`heavy ${label} pid ${process.pid}`);
 try{const result=await fn();await append('done',`heavy ${label}`);return result;}
 catch(e){await append('blocked',`heavy ${label}: ${e.message}`);throw e;}
 finally{await releaseLock();}
}
async function command(bin,args,logFile){
 const fd=openSync(logFile,'w');
 const child=spawn(bin,args,{cwd:root,stdio:['ignore',fd,fd]});closeSync(fd);activeChild=child;
 console.log(`${bin} pid ${child.pid}`);
 try{
  const code=await new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',resolve);});
  if(code!==0)throw Error(`${bin} exited ${code}; see ${logFile}`);
 }finally{activeChild=null;}
}
async function stopChild(child){
 if(!child?.pid||child.exitCode!==null||child.signalCode!==null)return;
 const pid=child.pid;child.kill('SIGTERM');
 await Promise.race([new Promise(r=>child.once('exit',r)),wait(5000)]);
 if(child.exitCode===null&&child.signalCode===null){
  try{process.kill(pid,'SIGKILL');}catch(e){if(e.code!=='ESRCH')throw e;}
  await Promise.race([new Promise(r=>child.once('exit',r)),wait(2000)]);
 }
 console.log(`stopped pid ${pid}`);
}
function startProcess(bin,args,env,logFile){
 const fd=openSync(logFile,'w');
 const child=spawn(bin,args,{cwd:root,env:{...process.env,...env},stdio:['ignore',fd,fd]});closeSync(fd);
 if(!child.pid)throw Error(`Could not start ${bin}`);
 console.log(`${bin} pid ${child.pid}`);return child;
}
async function portFree(port){
 return new Promise(resolve=>{
  const socket=net.connect({host:'127.0.0.1',port});
  socket.once('connect',()=>{socket.destroy();resolve(false);});
  socket.once('error',()=>resolve(true));
 });
}
async function waitUrl(url,seconds,child){
 const until=Date.now()+seconds*1000;
 while(Date.now()<until){
  checkAbort();if(child?.exitCode!==null)throw Error(`Process ${child.pid} exited before ${url} answered`);
  try{const r=await fetch(url);if(r.ok)return await r.json();}catch{}
  await wait(500);
 }
 throw Error(`${url} did not answer in ${seconds}s`);
}
async function waitTick(expected){
 const until=Date.now()+60_000;let last='unavailable';
 while(Date.now()<until){
  checkAbort();if(app?.exitCode!==null)throw Error(`Stella Scope pid ${app.pid} exited before tick`);
  try{
   const r=await fetch('http://127.0.0.1:7421/api/tick');
   if(r.ok){const tick=await r.json();last=tick?.observer?.printer_state??'missing';if(last===expected)return tick;}
  }catch(e){last=e.message;}
  await wait(500);
 }
 throw Error(`Expected observer.printer_state=${expected}; last ${last} after 60s`);
}
async function findPlaywright(){
 const npx=join(homedir(),'.npm/_npx');let found=[];
 for(const dir of await fs.readdir(npx)){
  const file=join(npx,dir,'node_modules/playwright-core/index.js');
  try{found.push({file,mtime:(await fs.stat(file)).mtimeMs});}catch{}
 }
 found.sort((a,b)=>b.mtime-a.mtime);
 if(!found.length)throw Error('playwright-core not found under ~/.npm/_npx/*/node_modules/playwright-core/index.js');
 const cache=join(homedir(),'.cache/ms-playwright');let chrome=[];
 for(const dir of await fs.readdir(cache))if(dir.startsWith('chromium-')){
  const file=join(cache,dir,'chrome-linux64/chrome');
  try{chrome.push({file,mtime:(await fs.stat(file)).mtimeMs});}catch{}
 }
 chrome.sort((a,b)=>b.mtime-a.mtime);
 if(!chrome.length)throw Error('Bundled Chromium not found under ~/.cache/ms-playwright/chromium-*');
 const module=await import(pathToFileURL(found[0].file).href);
 const chromium=module.chromium??module.default?.chromium;
 if(!chromium)throw Error(`playwright-core at ${found[0].file} has no Chromium launcher`);
 return {chromium,executablePath:chrome[0].file};
}
async function closeBrowser(){
 try{await browser?.close();}catch{}
 browser=null;
 try{await browserServer?.close();}catch{}
 browserServer=null;
 if(browserPid){try{process.kill(browserPid,0);process.kill(browserPid,'SIGTERM');}catch(e){if(e.code!=='ESRCH')throw e;}console.log(`closed browser pid ${browserPid}`);browserPid=null;}
}
const safeName=p=>p==='/'?'_root':'_next';
async function capture(state,cls,path,out,chromium,executablePath){
 const [width,height,hasTouch,deviceScaleFactor]=classes[cls];
 const context=await browser.newContext({viewport:{width,height},hasTouch,deviceScaleFactor});
 const page=await context.newPage(),errors=[];
 page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
 page.on('pageerror',e=>errors.push(e.message));
 const started=Date.now(),url=`http://127.0.0.1:7421${path}`;
 try{
  await page.goto(url,{waitUntil:'domcontentloaded',timeout:45_000});
  await page.waitForLoadState('networkidle',{timeout:45_000}).catch(()=>{});
  let sceneReady=true;
  try{await page.waitForFunction(()=>window.__stellaScene?.entries?.length>1000,null,{timeout:90_000});}
  catch{sceneReady=false;}
  const loadMs=Date.now()-started;
  await page.waitForTimeout(1500);
  const name=cls+safeName(path),dir=join(out,state);
  await fs.mkdir(dir,{recursive:true});
  await page.screenshot({path:join(dir,`${name}.png`),timeout:120_000});
  const metrics=await page.evaluate(()=>{
   const controls=[...document.querySelectorAll('button,[role=button]')].filter(el=>{
    const r=el.getBoundingClientRect(),s=getComputedStyle(el);
    return r.width>0&&r.height>0&&s.display!=='none'&&s.visibility!=='hidden';
   });
   const small=controls.map(el=>{const r=el.getBoundingClientRect();return {id:el.id,class:String(el.className),text:el.textContent?.trim().slice(0,80)||el.getAttribute('aria-label')||'',width:r.width,height:r.height};}).filter(r=>r.width<44||r.height<44);
   return {visibleButtons:controls.length,smallButtons:small,bodyDataset:{...document.body.dataset}};
  });
  const result={state,class:cls,path,viewport:{width,height,deviceScaleFactor,hasTouch},loadMs,sceneReady,consoleErrors:errors,...metrics};
  await fs.writeFile(join(dir,`${name}.json`),JSON.stringify(result,null,2)+'\n');
  console.log(`${state} ${name}: ${loadMs} ms; scene ${sceneReady?'ready':'timeout'}; ${errors.length} console errors`);
  return result;
 }finally{await page.close();await context.close();}
}
const html=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
async function contactSheet(out,results){
 const cells=options.states.map(state=>`<section><h2>${html(state)}</h2><div class="grid">${options.classes.flatMap(cls=>options.paths.map(path=>{
  const name=cls+safeName(path),exists=existsSync(join(out,state,`${name}.png`));
  return `<figure><figcaption>${html(cls)} ${html(path)}</figcaption>${exists?`<a href="${html(state+'/'+name+'.png')}"><img src="${html(state+'/'+name+'.png')}" loading="lazy" alt="${html(state+' '+cls+' '+path)}"></a>`:'<p>Missing screenshot</p>'}</figure>`;
 })).join('')}</div></section>`).join('\n');
 await fs.writeFile(join(out,'matrix.css'),'body{margin:0;padding:24px;background:#10151b;color:#ecf1f4;font:14px system-ui}h1,h2{margin:0 0 16px}section{margin:24px 0}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:16px}figure{margin:0;background:#1b222b;padding:8px;border-radius:8px}figcaption{padding:4px 0 8px}img{display:block;width:100%;height:auto}a{color:inherit}\n');
 await fs.writeFile(join(out,'index.html'),`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="matrix.css"><title>Stella Scope NEXT state matrix</title><h1>Stella Scope NEXT state matrix</h1>${cells}</html>\n`);
 await fs.writeFile(join(out,'results.json'),JSON.stringify(results,null,2)+'\n');
}

async function main(){
 const out=options.out;
 if(existsSync(out))throw Error(`Output already exists: ${out}`);
 if(!await portFree(7421)||!await portFree(7129))throw Error('Port 7421 or 7129 is already in use; refusing to touch its owner');
 if(!existsSync(sourceAssets))throw Error(`Asset library missing: ${sourceAssets}`);
 if(!existsSync(binary)){
  await heavy('cargo build --release',()=>command('cargo',['build','--release'],join(root,'tools/next-matrix/cargo-build.log')));
 }
 scratch=join(process.env.TMPDIR||'/tmp',`next-matrix-${process.pid}`);
 await fs.mkdir(scratch);await fs.mkdir(out,{recursive:true});
 const started=Date.now(),results=[];
 try{
  await heavy('rustc fake Moonraker',()=>command('rustc',['-O','--edition','2021',join(root,'tools/next-matrix/fake-moonraker.rs'),'-o',join(scratch,'fake-moonraker')],join(out,'rustc.log')));
  const {chromium,executablePath}=await findPlaywright();
  for(const state of options.states){
   checkAbort();
   try{
    const result=await heavy(`state ${state}`,async()=>{
     const db=join(scratch,`${state}.sqlite3`),cfg=JSON.parse(await fs.readFile(sourceConfig,'utf8'));
     cfg.port=7421;cfg.database=db;cfg.moonraker_url='http://127.0.0.1:7129';cfg.desktop_notifications=false;cfg.controls_enabled=true;
     await command('cp',['-al',sourceAssets,`${db}.assets`],join(out,`${state}-assets.log`));
     await fs.writeFile(join(scratch,`${state}.json`),JSON.stringify(cfg,null,2)+'\n');
     try{
      if(state!=='off'){
       await checkRamNow();fake=startProcess(join(scratch,'fake-moonraker'),[],{FAKE_STATE:state,FAKE_PORT:'7129'},join(out,`${state}-fake.log`));
       await waitUrl('http://127.0.0.1:7129/printer/info',10,fake);
      }
      await checkRamNow();app=startProcess(binary,['--config',join(scratch,`${state}.json`),'--port','7421'],{},join(out,`${state}-stella.log`));
      const expected=state==='off'?'unreachable':state==='halted'?'not_ready':'ready';
      await waitTick(expected);
      await checkRamNow();browserServer=await chromium.launchServer({executablePath,headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=swiftshader','--enable-unsafe-swiftshader','--ignore-gpu-blocklist']});
      browserPid=browserServer.process().pid;console.log(`browser pid ${browserPid}`);
      browser=await chromium.connect(browserServer.wsEndpoint());
      const captures=[];
      for(const cls of options.classes)for(const path of options.paths)captures.push(await capture(state,cls,path,out,chromium,executablePath));
      return {state,expected,reached:true,captures};
     }finally{await closeBrowser();await stopChild(app);await stopChild(fake);app=null;fake=null;}
    });
    results.push(result);
   }catch(e){console.error(`${state}: ${e.message}`);results.push({state,reached:false,error:e.message});await append('blocked',`state ${state}: ${e.message}`);}
  }
 }finally{
  await closeBrowser();await stopChild(app);await stopChild(fake);
  await contactSheet(out,results);
  await fs.rm(scratch,{recursive:true,force:true});scratch=null;
  console.log(`index: ${join(out,'index.html')}`);
  console.log(`total: ${((Date.now()-started)/1000).toFixed(1)} s`);
 }
 const ok=results.every(r=>r.reached&&r.captures?.length===options.classes.length*options.paths.length&&r.captures.every(c=>c.sceneReady));
 if(!ok)process.exitCode=1;
}
try{await main();}catch(e){console.error(e.stack||e);process.exitCode=1;}
finally{await closeBrowser();await stopChild(app);await stopChild(fake);await releaseLock();if(scratch)await fs.rm(scratch,{recursive:true,force:true});}
