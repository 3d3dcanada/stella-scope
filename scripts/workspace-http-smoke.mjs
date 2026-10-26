import http from 'node:http';import net from 'node:net';import {spawn} from 'node:child_process';import {readFileSync,writeFileSync,mkdirSync,mkdtempSync,rmSync} from 'node:fs';import {tmpdir} from 'node:os';import path from 'node:path';import {createHash} from 'node:crypto';import assert from 'node:assert/strict';
const out=path.resolve(process.env.VALIDATION_DIR||'validation/workspace-http'),bin=path.resolve(process.env.STELLA_BIN||'target/release/stella-scope');mkdirSync(out,{recursive:true});
const work=mkdtempSync(path.join(tmpdir(),'stella-workspace-http-')),db=path.join(work,'recording.sqlite3'),checks=[],traffic=[];let child,phase='normal',offline=false,seq=0,peak=0;
const ids=['stepper_x','stepper_y','stepper_z','stepper_z1','stepper_z2','stepper_z3','extruder'],consoleRows=[];
const wait=ms=>new Promise(r=>setTimeout(r,ms));
function status(){seq++;const missing=phase==='missing';const data={motion_report:{live_position:missing?[null,null,null,0]:[100+seq,160,33,0],live_velocity:25},toolhead:{position:[200,200,45,0],axis_maximum:[350,350,305,0],homed_axes:'xyz'},extruder:{temperature:phase==='second'?230:205,target:210,power:.4},heater_bed:{temperature:60,target:60,power:.3},fan:missing?{}:{speed:.65},'heater_fan hotend_fan':{speed:1},'controller_fan controller_fan':{speed:.5},probe:{last_query:false,last_z_result:3.2},bed_mesh:{profile_name:'synthetic',probed_matrix:[[.1,0],[-.1,null]],mesh_min:[20,20],mesh_max:[330,330]},quad_gantry_level:{applied:true},gcode_move:{homing_origin:[0,0,-.035]},stepper_enable:{steppers:{}},print_stats:{filename:'TEST <img src=x>.gcode',state:'printing',print_duration:12,filament_used:42},virtual_sdcard:{progress:.1},configfile:{settings:{printer:{kinematics:'corexy'},bltouch:{x_offset:-12,y_offset:18},stepper_z:{position_max:305},private_secret:'NEVER_EXPORT_CONFIG_SECRET'}}};
 for(const [i,id] of ids.entries()){data.stepper_enable.steppers[id]=true;data['tmc2209 '+id]={run_current:i<2?1:.8,hold_current:.4,drv_status:missing?null:phase==='second'&&id==='stepper_z1'?{cs_actual:25,otpw:true}:{cs_actual:8,stst:1}};}return data;}
const mock=http.createServer((req,res)=>{traffic.push({method:req.method,url:req.url});if(offline){res.writeHead(503);res.end('simulated printer unavailable');return;}const u=new URL(req.url,'http://localhost');let result;if(u.pathname==='/printer/objects/list')result={objects:Object.keys(status())};else if(u.pathname==='/printer/objects/query')result={status:status(),eventtime:42};else if(u.pathname==='/server/gcode_store')result={gcode_store:consoleRows};else{res.writeHead(404);res.end();return;}res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({result}));});await new Promise(r=>mock.listen(0,'127.0.0.1',r));
const port=await new Promise(r=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p));});}),base=`http://127.0.0.1:${port}`;
const cfg={moonraker_url:`http://127.0.0.1:${mock.address().port}`,database:db,bind:'127.0.0.1',port,poll_interval_ms:50,console_interval_ms:100,request_timeout_ms:100,offline_retry_ms:250};writeFileSync(path.join(work,'config.json'),JSON.stringify(cfg));
function check(label,v){assert.ok(v,label);checks.push(label);}
async function call(url,options={}){const r=await fetch(base+url,{...options,signal:AbortSignal.timeout(15000)});const text=await r.text();let json;try{json=JSON.parse(text);}catch{}return{status:r.status,json,text,headers:r.headers};}
async function until(fn,label){for(let n=0;n<200;n++){try{if(await fn())return;}catch{}if(child?.exitCode!==null&&child?.exitCode!==undefined)throw Error('Recorder stopped: '+label);await wait(50);}throw Error('Timeout '+label);}
function rss(){try{const n=Number(readFileSync(`/proc/${child.pid}/status`,'utf8').match(/^VmRSS:\s+(\d+)/m)?.[1]||0);peak=Math.max(peak,n);}catch{}}
function launch(extra=[]){child=spawn(bin,['--config',path.join(work,'config.json'),...extra],{stdio:['ignore','pipe','pipe']});let log='';child.stdout.on('data',b=>log+=b);child.stderr.on('data',b=>log+=b);child.on('close',()=>writeFileSync(path.join(out,extra.length?'replay.log':'recorder.log'),log));child.on('error',e=>{console.error(e);});}
async function stop(){if(!child)return;const p=child;if(p.exitCode===null){p.kill('SIGTERM');await Promise.race([new Promise(r=>p.once('close',r)),wait(1500)]);if(p.exitCode===null)p.kill('SIGKILL');}child=null;}
try{
 launch();await until(async()=>((await call('/api/meta')).json?.collector?.samples||0)>6,'first recording');
 const early=(await call('/api/workspace')).json;const then=early.point.ts;
 check('workspace schema and read-only boundary',early.schema==='stella-scope.workspace.v1'&&early.read_only===true);
 check('snapshot position is reported live position, not commanded',early.point.position[2]===33&&early.point.commanded[2]===45);
 check('recorded nozzle/bed and power supplied',early.point.nozzle.temperature===205&&early.point.bed.temperature===60&&early.point.nozzle.power===.4);
 check('all seven motors projected',Object.keys(early.point.motors).length===7);
 check('fan PWM is distinct from absent RPM',early.point.fans.fan.speed===.65&&early.point.fans.fan.rpm===null);
 check('probe false is retained',early.point.probe.triggered===false);
 check('probe offsets use recorded modified-toolhead configuration',early.geometry.probe_offset_mm[0]===-12&&early.geometry.probe_offset_mm[1]===18);
 check('Z envelope uses recorded travel limit',early.geometry.z_max_mm===305);
 check('no independent corner-height invention',early.geometry.independent_corner_positions===false);
 check('actual mesh matrix keeps unknown cells',early.point.mesh.grid[1][1]===null&&early.point.mesh.grid[0][0]===.1);
 check('job metadata projected without markup execution',early.point.job.name==='TEST <img src=x>.gcode'&&early.point.job.progress===.1);
 check('arbitrary config excluded',!JSON.stringify(early).includes('NEVER_EXPORT_CONFIG_SECRET'));
 phase='second';const eventTime=Date.now()/1000;consoleRows.push({time:eventTime,type:'response',message:'Making the following Z adjustments:\nstepper_z = 0.010297\nstepper_z1 = -0.035458\nstepper_z2 = -0.002368\nstepper_z3 = 0.027528\nRetries: 0/5 Probed points range: 0.040000 tolerance: 0.020000'}, {time:eventTime+.001,type:'response',message:"Stepper 'stepper_z1' position skew after probe: pos 4 now 6"});
 await until(async()=>{const s=(await call('/api/workspace')).json;return s?.point?.nozzle.temperature===230&&s.rounds.length>0;},'later round');
 const later=(await call('/api/workspace?points=12')).json;
 check('later point has changed temperature',later.point.nozzle.temperature===230);
 check('boolean warning flag retained',later.point.motors.stepper_z1.flags.includes('otpw'));
 check('levelling and events captured',later.rounds[0].z1===-.035458&&later.events.some(e=>e.kind==='position_warning'));
 check('representative history has hard point limit',later.history.length<=12&&later.history.every((p,i)=>!i||p.ts>later.history[i-1].ts));
 const past=(await call('/api/workspace?'+new URLSearchParams({ts:then,points:20}))).json;
 check('cursor point is at or before cursor',past.point.ts<=then&&past.cursor===then);
 check('heat rewinds instead of leaking current value',past.point.nozzle.temperature===205);
 check('motor flags rewind with cursor',past.point.motors.stepper_z1.flags.length===0);
 check('future levelling rounds excluded',past.rounds.every(r=>r.ts<=then)&&past.rounds.length===0);
 check('future events excluded',past.events.every(e=>e.ts<=then)&&!past.events.some(e=>e.kind==='position_warning'));
 check('all history remains at or before cursor',past.history.every(p=>p.ts<=then));
 check('observer status uses explicitly separate present-time namespace',past.observer_now.samples>early.observer_now.samples&&'recorded_health' in past);
 const previous=(await call('/api/workspace?'+new URLSearchParams({ts:later.point.ts,step:'previous'}))).json;
 check('previous sample navigation',previous.cursor<later.point.ts&&previous.point.ts===previous.cursor);
 const next=(await call('/api/workspace?'+new URLSearchParams({ts:previous.cursor,step:'next'}))).json;
 check('next sample navigation',next.cursor===later.point.ts);
 for(const route of ['/','/workspace','/workspace/printer','/workspace/motion','/workspace/motors','/workspace/thermal','/workspace/leveling','/workspace/events','/workspace/recording','/workspace.js','/workspace-core.js','/workspace-scene.js','/workspace.css','/classic','/investigate']){const r=await call(route);check('embedded route '+route,r.status===200&&r.text.length>100&&r.headers.get('content-security-policy')?.includes("script-src 'self'"));}
 for(const query of ['ts=NaN','ts=1&ts=2','ts=-1','points=241','window=7201','step=next','ts=1&step=move','other=x'])check('invalid query '+query,(await call('/api/workspace?'+query)).status===400);
 for(const method of ['POST','PUT','DELETE'])check(method+' rejected',(await call('/api/workspace',{method})).status===405);
 check('printer movement route does not exist',(await call('/api/move')).status===404);
 phase='missing';await until(async()=>(await call('/api/workspace')).json?.point?.motors?.stepper_z1?.flags===null,'missing status');
 const missing=(await call('/api/workspace')).json;check('missing position is not substituted from commanded position',missing.point.position.every(p=>p===null));check('unreported fan remains unknown',missing.point.fans.fan.speed===null);
 offline=true;await until(async()=>(await call('/api/meta')).json?.collector?.connected===false,'offline');await wait(450);const stale=(await call('/api/workspace')).json;check('offline retains last sample with explicit age',stale.point!==null&&stale.age_seconds>.2);check('recorded offline health is distinct from current observer',stale.recorded_health?.connected===false&&stale.observer_now.connected===false);rss();
 const requests=await Promise.all(Array.from({length:4},()=>call('/api/workspace?points=80')));check('four concurrent bounded readers return coherent snapshots',requests.every(r=>r.status===200&&r.json.history.every(p=>p.ts<=r.json.cursor)));rss();
 const earliest=(await call('/api/workspace?ts=1')).json;check('pre-recording time has no fictitious readings',earliest.point===null&&earliest.history.length===0);
 await stop();launch(['--replay',db]);await until(async()=>(await call('/api/workspace')).json?.observer_now?.mode==='replay','replay');const hash=()=>createHash('sha256').update(readFileSync(db)).digest('hex'),before=hash();const replay=(await call('/api/workspace')).json;check('replay defaults to final recorded sample',replay.cursor===replay.last_ts&&replay.age_seconds===0);await call('/api/workspace?'+new URLSearchParams({ts:then}));check('replay workspace never changes recording file',before===hash());rss();await stop();
 check('all collector network traffic stays GET-only',traffic.every(r=>r.method==='GET'));check('collector contacted only approved Moonraker read endpoints',traffic.every(r=>/^\/(printer\/objects\/(list|query)|server\/(gcode_store|files\/metadata)|websocket)/.test(r.url)));
 const report={passed:true,assertions:checks.length,checks,host:process.arch,observed_peak_rss_kib:peak,scope:'Compiled 0.3 executable over real HTTP with simulated Moonraker; short sample, four-reader exercise and replay. Not physical Voron/Pi acceptance, long soak or a hard memory guarantee.'};writeFileSync(path.join(out,'report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report,null,2));
}finally{await stop();mock.closeAllConnections();mock.close();rmSync(work,{recursive:true,force:true});}
