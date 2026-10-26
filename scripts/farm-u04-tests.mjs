// node scripts/farm-u04-tests.mjs: U04 (2026-10-20) the farm wall's one tile size, its gauges and keys, checked in the files themselves.
// The behaviour (grow, settle, hold a gauge, one command, offline and back) is proven in a real browser: docs/work-orders/ui-2026-10-20/U04-evidence/scripts/u04-state.mjs.
import {test} from 'node:test';import assert from 'node:assert/strict';import {readFileSync} from 'node:fs';
const web=f=>readFileSync(new URL('../web/'+f,import.meta.url),'utf8'),js=web('farm.js'),css=web('farm.css'),html=web('farm.html');
test('there is one tile size: no S/M/L switch, no size rule, no stored size',()=>{
 assert.ok(!/SIZES|defaultSize|validSize|fm-size|view\.size|data-size/.test(js),'farm.js');assert.ok(!/data-size|fm-size|fm-sizepick/.test(css+html),'farm.css and farm.html');
 assert.ok(!/store\('fm-size'/.test(js),'nothing keeps a size');
});
// 2026-10-24: "it expands the tab for no reason ... it could just be an overlay or a card over top of it." A press opens the printer as a card over the wall.
test('a press opens the printer as a card over the wall; its cell is kept, nothing on the wall moves',()=>{const hud=web('next-hud.css');
 assert.match(hud,/\.fm-tile\[data-open\]\{position:fixed!important;z-index:60/,'the card stands over the wall');assert.match(hud,/\.fm-veil\{position:fixed;inset:0;z-index:55/,'a veil under it');
 assert.match(js,/el\('div','fm-tile fm-ghost'\)/,'an empty frame keeps its cell');assert.match(js,/t\.root\.parentElement\.append\(v\)/,'the veil beside the card');
 assert.match(js,/if\(e\.key==='Escape'&&openId/,'Escape closes it');
 assert.match(js,/r\.animate\(\[\{transformOrigin:'0 0',transform:`translate/,'it flies from its tile: FLIP on the spring');assert.match(js,/const SPRING='cubic-bezier\(\.34,1\.56,\.64,1\)'/);
});
test('the tile uses the kit: LED gauges and square keys, no pill buttons and no menu of dots',()=>{
 assert.match(js,/import \{control,key,CONTROLS\} from '\.\/next-control\.js'/);assert.match(html,/href="\/next-control\.css"/);
 for(const k of ["sq('nav-hud','Open'","sq('pause','Pause'","sq('play','Resume'","sq('stop-square','Cancel print'","sq('power','Power'","sq('check','Plate cleared'","sq('play','Start next'"])assert.ok(js.includes(k),k);
 // the pill buttons that are left are the ones inside a dialog or a card (U05 turns those into things that grow), none on a tile and none in the header
 const pills=[...js.replace(/function confirmStop[\s\S]*?\n\/\/ Power/,'').replace(/async function confirmPower[\s\S]*?\n\}\n/,'').matchAll(/button\(\{icon:'i-[a-z-]+',label:'([^']+)'/g)].map(m=>m[1]).filter(l=>!/^(Show every printer|Add a printer|Not now|Cancel)$/.test(l));
 assert.deepEqual(pills,['Send once','Enter licence','Try the demo farm'],'only the review, the licence note and the demo card keep a pill button');
 assert.ok(!/openMenu|bindLongPress|fm-menu/.test(js+css),'no menu of dots');
 assert.match(js,/\.\.\.CONTROLS\[0\]/);assert.match(js,/\.\.\.CONTROLS\[1\]/,'the nozzle and the bed take the kit\'s own steps');
});
test('a heat gauge sends once, by the farm\'s command path with its preview, and never more than the service takes',()=>{
 assert.match(js,/onSet:v=>\{t\.holdN=Date\.now\(\)\+6000;heatDone\(t,t\.gN,'extruder',v\);\}/);assert.match(js,/\/command-preview/);assert.match(js,/pv\.tier==='T3'&&pv\.review!==false/);
 assert.match(js,/const SET_MAX=\{extruder:290,heater_bed:120\}/);assert.ok(!/onChange/.test(js),'nothing is sent while the hand turns');
});
test('the wall asks for its printers with a time limit and one at a time',()=>{
 assert.match(js,/AbortSignal\.timeout\(8000\)/);assert.match(js,/if\(refreshAt&&Date\.now\(\)-refreshAt<9000\)return/);
});
