import http from 'node:http';
import net from 'node:net';
import {spawn} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const out=path.resolve(process.env.VALIDATION_DIR||'validation'),bin=path.resolve(process.env.STELLA_BIN||'target/release/stella-scope');mkdirSync(out,{recursive:true});
const work=mkdtempSync(path.join(tmpdir(),'stella-investigation-')),db=path.join(work,'recording.sqlite3'),checks=[],requests=[];
let child,offline=false,phase='normal',sampleRequests=0,peak=0;
const motors=['stepper_x','stepper_y','stepper_z','stepper_z1','stepper_z2','stepper_z3','extruder'];
const start=Date.now()/1000,consoleRows=[{time:start,type:'response',message:'Gantry-relative probe points:\n0: 15.474970 1: 15.504345 2: 15.487470 3: 15.464345\nMaking the following Z adjustments:\nstepper_z = 0.010297\nstepper_z1 = -0.035458\nstepper_z2 = -0.002368\nstepper_z3 = 0.027528\nRetries: 0/5 Probed points range: 0.040000 tolerance: 0.020000'},{time:start+.001,type:'response',message:'PRIVATE_CONSOLE_SECRET <img src=x onerror=alert(1)>'}];
function data(){sampleRequests++;const d={motion_report:{live_position:[sampleRequests,100,5,0],live_velocity:40},toolhead:{position:[sampleRequests,100,5,0],homed_axes:'xyz'},stepper_enable:{steppers:{}},extruder:{temperature:210,target:210},heater_bed:{temperature:60,target:60},fan:{speed:.5},print_stats:{filename:'PRIVATE_FILENAME.gcode',state:'printing'},configfile:{settings:{secret:'PRIVATE_CONFIGURATION_SECRET'}},quad_gantry_level:{applied:false}};motors.forEach((m,i)=>{d.stepper_enable.steppers[m]=true;d['tmc2209 '+m]={run_current:i<2?1:.8,hold_current:.5,drv_status:phase==='unknown'&&m==='stepper_z1'?null:{cs_actual:phase==='warning'&&m==='stepper_z1'?31:8,otpw:phase==='warning'&&m==='stepper_z1'?1:0}};});return d;}
const mock=http.createServer((req,res)=>{requests.push({method:req.method,url:req.url});const u=new URL(req.url,'http://localhost');if(offline){res.writeHead(503);res.end('printer switched off in simulation');return;}let result;if(u.pathname==='/printer/objects/list'){result={objects:[...Object.keys(data()),'gcode_move','idle_timeout','virtual_sdcard','probe','bed_mesh','heater_fan hotend_fan','controller_fan controller_fan']};}else if(u.pathname==='/printer/objects/query'){result={status:data(),eventtime:42};}else if(u.pathname==='/server/gcode_store'){result={gcode_store:consoleRows};}else{res.writeHead(404);res.end();return;}res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({result}));});
await new Promise(r=>mock.listen(0,'127.0.0.1',r));
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const port=await new Promise(r=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p));});});const base=`http://127.0.0.1:${port}`;
function check(name,value){assert.ok(value,name);checks.push(name);}
async function call(url,options){const r=await fetch(base+url,{...options,signal:AbortSignal.timeout(12000)});const text=await r.text();let json;try{json=JSON.parse(text);}catch{}return{r,text,json};}
async function until(fn,label){for(let i=0;i<150;i++){if(child&&child.exitCode!==null)throw new Error('Recorder exited before '+label);try{if(await fn())return;}catch{}await wait(50);}throw new Error('Timed out: '+label);}
function launch(extra=[]){child=spawn(bin,['--config',path.join(work,'config.json'),...extra],{stdio:['ignore','pipe','pipe']});let log='';child.stdout.on('data',b=>log+=b);child.stderr.on('data',b=>log+=b);child.on('close',()=>writeFileSync(path.join(out,extra.includes('--replay')?'replay.log':'collector.log'),log));child.on('error',e=>{console.error(e);});return child;}
async function stop(){if(!child||child.exitCode!==null)return;const p=child;p.kill('SIGTERM');await Promise.race([new Promise(r=>p.once('close',r)),wait(2000)]);if(p.exitCode===null)p.kill('SIGKILL');child=null;}
function rss(){try{const s=readFileSync(`/proc/${child.pid}/status`,'utf8');const n=Number(s.match(/^VmRSS:\s+(\d+)/m)?.[1]||0);peak=Math.max(peak,n);return n;}catch{return null;}}
const cfg={moonraker_url:`http://127.0.0.1:${mock.address().port}`,database:db,bind:'127.0.0.1',port,poll_interval_ms:50,console_interval_ms:100,request_timeout_ms:100,offline_retry_ms:250,investigation:{max_samples:12000,gap_multiple:3}};
writeFileSync(path.join(work,'config.json'),JSON.stringify(cfg));
try{
 launch();await until(async()=>((await call('/api/meta')).json?.collector?.samples||0)>=8,'initial samples');rss();
 phase='warning';await wait(400);phase='unknown';await wait(250);phase='normal';await wait(250);
 const before=(await call('/api/meta')).json;offline=true;await until(async()=>(await call('/api/meta')).json?.collector?.connected===false,'offline state');await wait(450);
 check('disconnection keeps recorded samples',(await call('/api/meta')).json.collector.samples>=before.collector.samples);
 offline=false;await until(async()=>(await call('/api/meta')).json?.collector?.connected===true,'reconnection');await wait(300);
 const meta=(await call('/api/meta')).json,from=Math.min(start,meta.first_ts),to=meta.last_ts,q=new URLSearchParams({from,to,points:100,messages:1});
 const result=await call('/api/investigation?'+q),r=result.json;rss();check('investigation responds over real HTTP',result.r.status===200&&r.schema==='stella-scope.investigation.v1');
 check('analysis complete and sample count matches the selected recording',r.complete&&r.samples_scanned===meta.collector.samples);
 check('all seven motor statistics present',r.motors.length===7);
 check('back-left current-scale peak retained',r.motors[3].drive_level.max===31&&r.points.some(p=>p.motors[3].drive_level===31));
 check('missing driver status recorded as unknown',r.motors[3].status_unknown>0);
 check('warning and capture-gap incidents present',r.incidents.some(i=>i.category==='driver')&&r.incidents.some(i=>i.category==='capture'));
 check('exact levelling corrections retained',r.rounds.some(x=>x.z0===.010297&&x.z1===-.035458&&x.z2===-.002368&&x.z3===.027528));
 check('chart points bounded and ordered',r.points.length<=100&&r.points.every((p,i)=>!i||p.ts>r.points[i-1].ts));
 const secretFree=await call('/api/export?'+new URLSearchParams({from,to,points:100}));check('default export succeeds and omits all private input',secretFree.r.status===200&&!secretFree.text.includes('PRIVATE_'));
 const opted=await call('/api/export?'+q);check('console text included only by explicit opt-in',opted.r.status===200&&opted.text.includes('PRIVATE_CONSOLE_SECRET')&&!opted.text.includes('PRIVATE_CONFIGURATION_SECRET')&&!opted.text.includes('PRIVATE_FILENAME'));
 const csv=await call('/api/export?'+new URLSearchParams({from,to,points:100,format:'csv'}));check('CSV exports only bounded typed chart rows',csv.r.status===200&&csv.text.startsWith('timestamp,x_mm')&&csv.text.trim().split('\n').length<=101&&!csv.text.includes('PRIVATE_')&&!csv.text.includes('<img'));
 for(const url of ['/investigate','/investigate.js','/investigate.css','/','/app.js','/style.css']){const a=await call(url);check('embedded asset '+url,a.r.status===200&&a.text.length>100);check('CSP on '+url,a.r.headers.get('content-security-policy')?.includes("script-src 'self'"));}
 for(const url of ['/api/meta','/api/live','/api/at?ts='+to,'/api/series?from='+from+'&to='+to+'&points=30','/api/events?from='+from+'&to='+to,'/api/qgl'])check('original route retained '+url.split('?')[0],(await call(url)).r.status===200);
 check('POST investigation rejected',(await call('/api/investigation?'+q,{method:'POST'})).r.status===405);
 check('POST export rejected',(await call('/api/export?'+q,{method:'POST'})).r.status===405);
 for(const query of ['from=NaN&to=1','from=2&to=1','from=0&to=9999999999','from=1&to=2&points=1','from=1&to=2&messages=true','from=1&to=2&format=xml'])check('invalid query rejected '+query,(await call('/api/investigation?'+query)).r.status===400);
 check('empty export refused',(await call('/api/export?from=1&to=2')).r.status===400);
 const midpoint=r.points[Math.floor(r.points.length/2)].ts,rew=(await call('/api/investigation?'+new URLSearchParams({from,to:midpoint,points:100}))).json;check('investigation rewind excludes future samples and events',rew.points.every(p=>p.ts<=midpoint)&&rew.events.every(e=>e.ts<=midpoint)&&rew.rounds.every(e=>e.ts<=midpoint));
 const at=(await call('/api/at?ts='+midpoint)).json;check('cursor sample at or before requested time',at.sample.ts<=midpoint);rss();
 await stop();cfg.investigation.max_samples=2;writeFileSync(path.join(work,'config.json'),JSON.stringify(cfg));launch(['--replay',db]);await until(async()=>(await call('/api/meta')).json?.collector?.mode==='replay','replay ready');const hash=()=>createHash('sha256').update(readFileSync(db)).digest('hex'),original=hash();
 const partial=(await call('/api/investigation?'+q)).json;check('sample cap is visible with continuation',partial.complete===false&&partial.samples_scanned===2&&Number.isFinite(partial.next_from));check('partial export refused',(await call('/api/export?'+q)).r.status===400);check('replay does not edit recording file',hash()===original);rss();await stop();
 check('only read-only Moonraker endpoints were requested',requests.every(r=>r.method==='GET'&&/^\/(printer\/objects\/(list|query)|server\/gcode_store|server\/files\/metadata|websocket(\?|$))/.test(r.url)));
 check('Klipper object names encode spaces as percent20',requests.some(r=>r.url.includes('tmc2209%20stepper_z1'))&&requests.every(r=>!r.url.includes('+')));
 const report={passed:true,assertions:checks.length,checks,host:process.arch,observed_peak_rss_kib:peak,under_20_mib:peak>0&&peak<20480,scope:'Compiled Rust executable over HTTP, simulated Moonraker. Short synthetic workload, not physical Raspberry Pi, SD storage, or real-printer acceptance.'};writeFileSync(path.join(out,'integration-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report,null,2));
}finally{await stop();mock.closeAllConnections();mock.close();rmSync(work,{recursive:true,force:true});}
