import assert from 'node:assert/strict';
import {jobDisplay,preflight,installationAge,instanceRecordId,printerLink,controlsSummary,insideTravel} from '../web/hud-core.js';
// finish-2 F02: a PrusaLink that reports only Z while moving still has a position; nothing reported, or a known axis out of travel, has not
{const L={min:[0,-5,0],max:[300,330,330]};assert.equal(insideTravel([null,null,98.1],L),true);assert.equal(insideTravel([null,null,null],L),false);assert.equal(insideTravel([null,null,500],L),false);assert.equal(insideTravel([1,2,'x'],L),false);assert.equal(insideTravel([1,2,3],L),true);}
assert.equal(jobDisplay({name:'a.gcode',state:'printing',progress:.5},'b.gcode').progress,null);
assert.equal(jobDisplay({name:'a.gcode',state:'printing',progress:.5},'a.gcode').progress,.5);
assert.equal(jobDisplay({name:'a.gcode',state:'paused',progress:.5},'a.gcode').state,'paused');
assert.equal(jobDisplay({name:'a.gcode',state:'complete',progress:null},'a.gcode').progress,null);
assert.equal(preflight(null)[0].state,'unknown');
assert.equal(preflight({point:{homed:'xy',mesh:{grid:[[null]]}}})[1].state,'review');
assert.equal(preflight({point:{homed:'xyz',mesh:{grid:[[null]]}}})[3].state,'unknown');
assert.equal(preflight({point:{homed:'xyz',mesh:{grid:[[0,.1],[.1,.2]]}}})[3].state,'observed');
assert.equal(preflight({point:{},age_seconds:20})[0].state,'review');
assert.equal(installationAge({state:'candidate',effective_at:0},86400),null);
assert.equal(installationAge({state:'installed',effective_at:0},86400),1);
assert.equal(installationAge({state:'installed',effective_at:86400},0),null);
assert.equal(instanceRecordId('part:root/0/2:3'),'cad:part:root.0.2:3');
// WO-13 (2026-10-12): a PrusaLink/FlashForge printer never reads "Klipper" in its state text.
{
 const off={printer_state:'unreachable'},nr={printer_state:'not_ready',printer_message:'PrusaLink refused the password'};
 for(const [obs,opts]of [
  [nr,{klipper:false,brand:'PrusaLink'}],[off,{klipper:false,brand:'FlashForge'}],
  [nr,{}],[off,{}],[{printer_state:'ready'},{}],[{printer_state:'service'},{}]]){
  const l=printerLink(obs,opts),cs=controlsSummary(l,{live:true,enabled:true});
  const all=JSON.stringify([l.label,l.reason,cs.badge,cs.line]);
  if(opts.klipper===false)assert.ok(!/Klipper|Moonraker|FIRMWARE/i.test(all),'non-Klipper state must not name Klipper: '+all);
  if(!('klipper' in opts))assert.ok(/Klipper|Moonraker/.test(all)||l.state==='service'||l.state==='ready'||l.state==='off','Klipper words stay for Klipper: '+all); // 2026-10-25: an off printer is described in plain words, with no Moonraker
 }
 const l=printerLink(nr,{klipper:false,brand:'PrusaLink'});
 assert.equal(l.label,'PRINTER NOT READY');assert.equal(l.recover,false);assert.ok(l.reason.includes('PrusaLink refused the password'));
 assert.equal(controlsSummary(l).badge,'NOT READY · LOCKED');
 assert.equal(printerLink(nr,{}).label,'KLIPPER NOT READY');
}
console.log('HUD evidence rules: 13 + 12 assertions passed.');
