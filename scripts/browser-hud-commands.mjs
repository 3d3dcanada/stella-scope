// Replay-only command/modal regression. Exact previews and one intercepted synthetic STOP; no real command dispatch.
// HUD_URL=http://127.0.0.1:17422 PLAYWRIGHT_MODULE=/path/to/playwright node scripts/browser-hud-commands.mjs
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';

const require=createRequire(import.meta.url),{chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const base=process.env.HUD_URL;
if(!base||!/^http:\/\/127\.0\.0\.1:\d+$/.test(base))throw Error('HUD_URL must be a loopback replay instance.');
const checks=[],requests=[],errors=[];
const check=(name,value)=>{assert.ok(value,name);checks.push(name);console.log('PASS '+name);};
const browser=await chromium.launch({executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--use-gl=angle','--use-angle=gl','--ignore-gpu-blocklist']});
try{
 // HUD_PART=live runs only the second half (below); the first half needs a replay as HUD_URL.
 if(process.env.HUD_PART!=='live'){
 const page=await browser.newPage({viewport:{width:1366,height:768}});
 page.on('request',r=>requests.push({url:r.url(),method:r.method()}));page.on('pageerror',e=>errors.push(e.message));
 const workspaceResponse=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/workspace'&&r.status()===200);
 await page.goto(base+'/service/machine');const workspace=await (await workspaceResponse).json();await page.waitForFunction(()=>window.__stellaScene?.metrics.loads.length,{},{timeout:180000});
 const nozzleTrack=page.locator('.hud-rail.left .hud-gauge-track').first();
 check('gauge track is a named keyboard button',await nozzleTrack.getAttribute('role')==='button'&&await nozzleTrack.getAttribute('aria-label')==='NOZZLE control'&&await nozzleTrack.getAttribute('tabindex')==='0');
 await nozzleTrack.focus();await page.keyboard.press('Enter');
 // Polish WO-06 (2026-10-18): this clicked Set at once and read the refusal toast. Since the printer-state locks of 2026-09-23/24 the button is
 // locked as soon as the page knows the state (a replay), and whether the click came before that was a race. The check is the lock itself.
 const target=page.locator('.hud-modal[open] .hud-num');await target.fill('222');
 await page.waitForFunction(()=>[...document.querySelectorAll('.hud-modal[open] button')].some(b=>b.textContent==='Set'&&b.dataset.locked==='1'&&(b.disabled||b.getAttribute('aria-disabled')==='true')),{},{timeout:15000});
 check('heater set is locked in a replay (the button is disabled)',await page.getByRole('button',{name:'Set',exact:true}).isDisabled());
 check('heater set skips the review modal',await page.getByRole('heading',{name:'Review printer command'}).count()===0);
 check('replay refuses the heater set before any network dispatch',requests.filter(r=>r.method==='POST'&&r.url===base+'/api/commands').length===0);
 await page.keyboard.press('Escape');
 check('Escape closes the heater modal',await page.locator('.hud-modal[open]').count()===0);
 // Polish WO-06 (2026-10-18): since the printer-state locks of 2026-09-23/24 a replay does not offer a motion command at all: the button
 // is locked, with its reason, and nothing opens. (This used to open the exact G28 review, disabled; that review is covered by the Rust
 // command tests and by NEXT's review dialog.)
 const home=page.getByRole('button',{name:'HOME',exact:true});
 check('motion is locked in a replay, with a reason, and opens no review',await home.isDisabled()&&await home.getAttribute('data-locked')==='1'&&await page.getByRole('heading',{name:'Review printer command'}).count()===0);
 await page.mouse.click(1,1);
 check('outside click leaves no modal open',await page.locator('.hud-modal[open]').count()===0);

 // The tuning, History and missing-value checks read the Klipper telemetry of our September recording; a demo has none of it.
 if(process.env.HUD_FIXTURE==='replay-2026-09'){
 await page.getByRole('button',{name:'More',exact:true}).click();await page.getByText('Tune',{exact:true}).click();
 const tuning=page.locator('.hud-modal[open]');
 const signal=(object,field)=>workspace.point.telemetry.signals.find(s=>s.object===object&&s.field===field)?.value,reported=(direct,object,field)=>Number.isFinite(direct)?direct:signal(object,field);
 const expected={velocity:reported(workspace.point.max_velocity,'toolhead','max_velocity'),accel:reported(workspace.point.max_accel,'toolhead','max_accel'),scv:reported(workspace.point.square_corner_velocity,'toolhead','square_corner_velocity'),pa:reported(workspace.point.pressure_advance,'extruder','pressure_advance')};
 check('selected replay exposes current tuning telemetry',Object.values(expected).every(Number.isFinite));
 check('tuning uses selected recorded values',await tuning.locator('.hud-row').filter({hasText:'Velocity mm/s'}).locator('input').inputValue()===String(expected.velocity)&&await tuning.locator('.hud-row').filter({hasText:'Acceleration'}).locator('input').inputValue()===String(expected.accel)&&await tuning.locator('.hud-row').filter({hasText:'Square corner'}).locator('input').inputValue()===String(expected.scv)&&await tuning.locator('.hud-row').filter({hasText:'Seconds'}).locator('input').inputValue()===String(expected.pa));
 while(await page.locator('.hud-modal[open]').count())await page.keyboard.press('Escape');
 const historyResponse=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/series');
 await page.getByRole('button',{name:'More',exact:true}).click();await page.getByText('History',{exact:true}).click();check('recorded History request succeeds',(await historyResponse).ok());
 const historyNote=page.locator('.hud-modal[open] .hud-modal-note');await historyNote.waitFor();await page.waitForFunction(()=>!/Loading captured samples/.test(document.querySelector('.hud-modal[open] .hud-modal-note')?.textContent||''));
 check('recorded History is drawn from the selected recording',/Nozzle in orange, bed in amber/.test(await historyNote.textContent()));
 while(await page.locator('.hud-modal[open]').count())await page.keyboard.press('Escape');
 const missing=structuredClone(workspace);delete missing.point.max_velocity;delete missing.point.max_accel;delete missing.point.square_corner_velocity;delete missing.point.pressure_advance;missing.point.telemetry.signals=missing.point.telemetry.signals.filter(s=>!((s.object==='toolhead'&&['max_velocity','max_accel','square_corner_velocity'].includes(s.field))||(s.object==='extruder'&&s.field==='pressure_advance')));
 const workspaceRoute=/\/api\/workspace\?/;await page.route(workspaceRoute,route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify(missing)}));
 await page.getByRole('button',{name:'Previous sample'}).click();await page.waitForFunction(()=>!document.querySelector('.hud-velocity')?.textContent.includes('max'));
 await page.getByRole('button',{name:'More',exact:true}).click();await page.getByText('Tune',{exact:true}).click();
 check('genuinely missing tuning values stay blank',(await page.locator('.hud-modal[open] .hud-num').evaluateAll(inputs=>inputs.every(input=>input.value===''))));
 while(await page.locator('.hud-modal[open]').count())await page.keyboard.press('Escape');await page.unroute(workspaceRoute);
 }else console.log('SKIP (needs the 2026-09 replay recording) tuning telemetry, History and missing values');
 // Polish WO-06 (2026-10-18): the controller-fan gauge was folded away after this check was written; the hotend fan keeps the same kind of truthful popup.
 check('there is no controller-fan gauge any more',await page.locator('.hud-gauge[data-fan=controller]').count()===0);
 await page.locator('.hud-gauge[data-fan=hotend] .hud-gauge-value').click();
 check('hotend fan has its own truthful popup, driven automatically and with no tachometer',await page.getByRole('heading',{name:'Hotend fan'}).isVisible()&&/automatically/i.test(await page.locator('.hud-modal[open]').textContent())&&/no tachometer/i.test(await page.locator('.hud-modal[open]').textContent()));
 await page.keyboard.press('Escape');
 check('no script errors',errors.length===0);
 check('no command was dispatched (a preview is not sent either, since a replay opens no review)',!requests.some(r=>r.method==='POST'&&(r.url===base+'/api/commands'||r.url===base+'/api/command-preview')));
 await page.close();
 }

 // Polish WO-06 (2026-10-18): rewinding with the emergency stop still usable is a property of a LIVE instance (a replay says "saved recording, read only"),
 // so this half runs against HUD_LIVE_URL, a scratch instance of demo printers, and is skipped without it.
 const liveBase=process.env.HUD_LIVE_URL;
 if(!liveBase)console.log('SKIP (needs HUD_LIVE_URL, a live scratch instance) the synthetic rewind STOP');
 else{
 // Synthetic boundary: only these two loopback API routes are intercepted. No request reaches a
 // real command handler or printer; this proves the frontend keeps emergency stop usable in rewind.
 const synthetic=[],syntheticNetwork=[];
 const emergencyPage=await browser.newPage({viewport:{width:1366,height:768}});
 emergencyPage.on('request',r=>syntheticNetwork.push({url:r.url(),method:r.method()}));emergencyPage.on('pageerror',e=>errors.push(e.message));
 await emergencyPage.route(liveBase+'/api/controls',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({
  enabled:true,reason:'synthetic live controls',token:'synthetic-token',commands:[{id:'emergency_stop',allowed:true,reason:'synthetic emergency available',tier:'T3',params:'fixed'}]
 })}));
 await emergencyPage.route(liveBase+'/api/commands',async route=>{const request=route.request();synthetic.push({method:request.method(),headers:request.headers(),body:request.postDataJSON()});await route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({status:'completed',message:'Synthetic Moonraker response only; no printer contacted.',audit_id:null})});});
 await emergencyPage.goto(liveBase+'/service/machine');await emergencyPage.waitForFunction(()=>window.__stellaScene?.metrics.loads.length,{},{timeout:180000});
 await emergencyPage.getByRole('button',{name:'Previous sample'}).click();
 await emergencyPage.waitForFunction(()=>/paused while rewound/i.test(document.querySelector('.hud-readiness')?.textContent||''));
 check('synthetic boundary enters rewind with STOP enabled',!await emergencyPage.locator('.hud-emergency').isDisabled());
 const dispatched=emergencyPage.waitForResponse(liveBase+'/api/commands',{timeout:15000});
 await emergencyPage.locator('.hud-emergency').click();await dispatched;
 check('synthetic rewind STOP dispatches exactly one mocked emergency request',synthetic.length===1&&synthetic[0].method==='POST'&&synthetic[0].body.id==='emergency_stop'&&synthetic[0].body.confirm===false&&synthetic[0].headers['x-stella-control-token']==='synthetic-token');
 check('synthetic boundary intercepted every mutating request',syntheticNetwork.filter(r=>r.method!=='GET').length===1&&syntheticNetwork.find(r=>r.method!=='GET')?.url===liveBase+'/api/commands');
 check('synthetic boundary has no script errors',errors.length===0);
 await emergencyPage.close();
 }
 console.log(`${checks.length} checks passed`);
}finally{await browser.close();}
