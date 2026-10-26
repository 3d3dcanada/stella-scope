// Finish-2 F07 (2026-10-19): the words on a farm tile. One name for ending a print (Cancel print, the printer's page said so; the wall said Stop, and the red STOP is
// the emergency stop), a cancelled print is CANCELLED everywhere on a tile, an error is ERROR, and an unknown spool weight is left out, never 0 g.
//   node --test scripts/farm-words-tests.mjs
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {ringWord,bigWord,spoolLine,WORD,CHIP} from '../web/farm-wall.js';

test('a cancelled print is CANCELLED on the ring and in the big word, an error is ERROR, nothing says STOPPED',()=>{
 for(const look of ['printing','heating','paused','clear','done','idle','error','offline']){
  for(const cancelled of [false,true])for(const state of ['finished','printing','error']){
   for(const w of [ringWord(look,cancelled,state),bigWord(look,cancelled)])assert.ok(!/STOPPED/i.test(w),`${look}/${cancelled}: ${w}`);}}
 assert.equal(ringWord('done',true,'finished'),'CANCELLED');assert.equal(ringWord('clear',true,'finished'),'CANCELLED');
 assert.equal(ringWord('done',false,'finished'),'DONE');assert.equal(ringWord('clear',false,'finished'),'DONE');assert.equal(ringWord('clear',false,'idle'),'CLEAR');
 assert.equal(ringWord('error',false,'error'),'ERROR');assert.equal(bigWord('error',false),'ERROR');
 assert.equal(bigWord('done',true),'CANCELLED');assert.equal(bigWord('done',false),'DONE');assert.equal(bigWord('clear',false),'CLEAR PLATE');assert.equal(bigWord('offline',false),'OFF');
 assert.equal(ringWord('printing',false,'printing'),'PRINTED');assert.equal(ringWord('offline',false,'offline'),'NO LINK');
 for(const l of Object.keys(WORD))assert.ok(WORD[l]&&CHIP[l]);
});

test('an unknown spool weight is left out, never 0 g',()=>{
 assert.equal(spoolLine({brand:'Polymaker',material:'PLA',grams_left:812.4}),'Polymaker PLA · 812 g');
 assert.equal(spoolLine({brand:'Polymaker',material:'PLA',grams_left:null}),'Polymaker PLA');
 assert.equal(spoolLine({material:'PETG'}),'PETG');
 assert.equal(spoolLine({material:'PETG',grams_left:0}),'PETG · 0 g','a spool that really is empty says so');
 assert.equal(spoolLine({grams_left:NaN}),'Spool');
 assert.equal(spoolLine({}),'Spool');
 assert.equal(spoolLine(undefined),'Spool');
});

// One name for ending a print on every page that offers it: the wall's button, its menu and its question say Cancel print, as the printer's page does.
test('ending a print is called Cancel print on the wall as on the printer page',()=>{
 const farm=fs.readFileSync(new URL('../web/farm.js',import.meta.url),'utf8');
 assert.ok(!/'Stop the print'|title:'Stop'|label:'Stop'|`Stop \$\{p\.name\}\?`/.test(farm),'no "Stop" for ending a print on the wall (the red STOP is the emergency stop)');
 // U04 (2026-10-20): the tile's Cancel print is a square key now (the menu that held a second one is gone), so the two old patterns for the menu row and the icon button became the one key; the confirm dialog's button is unchanged.
 assert.match(farm,/label:'Cancel print',kind:'danger',size:'row',title:'Cancel print'/);
 assert.match(farm,/sq\('stop-square','Cancel print','Cancel print'/);
 assert.match(farm,/Cancel the print on \$\{p\.name\}\?/);
 const next=fs.readFileSync(new URL('../web/next.js',import.meta.url),'utf8');assert.match(next,/cancel:\{title:'Cancel print'/);
 // and the farm wall's bell, its sheet and its settings are all Alerts
 const fleet=fs.readFileSync(new URL('../web/next-fleet.js',import.meta.url),'utf8');
 assert.match(fleet,/label:'Alerts',size:'row',title:'Alerts'/);assert.match(fleet,/openSheet\(\{title:'Alerts'/);assert.ok(!/title:'Alarms'/.test(fleet));
 // and the settings are not also called Alerts: the list and its settings have two names
 const notify=fs.readFileSync(new URL('../web/next-notify.js',import.meta.url),'utf8');assert.match(notify,/openSheet\(\{title:'Alert settings'/);assert.ok(!/openSheet\(\{title:'Alerts'/.test(notify));
});
