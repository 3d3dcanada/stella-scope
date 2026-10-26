import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {plainLabel,matches,needs,selection,dockTools,drawable} from '../web/model-pick.js';

// Rows shaped like GET /api/printer-models (a machine) and /api/printer-models/<id>/options (an option).
const M=(o={})=>({id:'voron_trident_300',label:'VORON Trident 300 · standard printed assembly',bundled:true,installed:null,has_file:true,licence:{class:'gpl',text:'GPL-3.0'},...o});
const O=(id,o={})=>({id,label:id,kind:'addon',drawable:true,bundled:true,installed:false,bytes:1e6,licence:{class:'gpl',text:'GPL-3.0'},...o});

test('plainLabel: the one Japanese label in the catalogue reads in English, nothing else changes',()=>{
 assert.equal(plainLabel('INDX · 原本ノズル参照'),'INDX · original nozzle, reference');
 assert.equal(plainLabel('INDX CHT · 0.4 mm'),'INDX CHT · 0.4 mm');
 assert.equal(plainLabel('Xol / Sherpa Mini · r16 / Rapido 2 UHF'),'Xol / Sherpa Mini · r16 / Rapido 2 UHF');
 assert.equal(plainLabel('Head · 日本語'),'Head');
 assert.equal(plainLabel(null),'');
 // every label in the shipped catalogue comes out with no CJK
 const cat=JSON.parse(readFileSync(new URL('../web/printer-models.json',import.meta.url),'utf8'));
 const bad=s=>[...String(s||'')].some(c=>c.charCodeAt(0)>0x2e7f);
 for(const o of cat.options){assert.ok(!bad(plainLabel(o.label)),o.id);for(const t of Object.values(o.attach?.tools||{}))assert.ok(!bad(plainLabel(t.label)),o.id+'/'+t.label);}
});

test('matches: every word must be there, in any order',()=>{
 assert.ok(matches('trident 300','VORON Trident 300 · standard printed assembly'));
 assert.ok(matches('ender 3','Ender-3 · Creality / Original'));
 assert.ok(matches('300 trident','VORON Trident 300'));
 assert.ok(!matches('trident 350','VORON Trident 300'));
 assert.ok(matches('','anything')&&matches('   ','anything'));
});

test('needs: the machine and each chosen option that is not here yet, the size, and whether a licence must be read',()=>{
 const opts=[O('indx_bank',{kind:'toolchanger',bytes:3950310}),O('sturdy_handles_m5',{bytes:127590}),O('trident_ab_stepper_fan_491404_v2',{bytes:11326380,bundled:false,licence:{class:'nc',text:'CC BY-NC'}})];
 let n=needs(M(),opts,[{option:'indx_bank'}]);
 assert.deepEqual(n.items.map(i=>i.id),['voron_trident_300','indx_bank']);
 assert.deepEqual(n.todo.map(i=>i.id),['voron_trident_300','indx_bank']);
 assert.equal(n.bytes,3950310,'a machine of unknown size adds nothing');assert.equal(n.tick,false,'both are open');
 n=needs(M({installed:{bytes:9}}),opts,[{option:'indx_bank'},{option:'trident_ab_stepper_fan_491404_v2'}]);
 assert.deepEqual(n.todo.map(i=>i.id),['indx_bank','trident_ab_stepper_fan_491404_v2'],'an installed machine is not fetched again');
 assert.equal(n.tick,true,'a non-commercial file must be read first');assert.equal(n.bytes,3950310+11326380);
 n=needs(M({installed:{bytes:9}}),[O('indx_bank',{installed:true})],[{option:'indx_bank'}]);
 assert.equal(n.todo.length,0,'everything here: nothing to get');
 n=needs(M({has_file:false}),[],[]);assert.equal(n.noFile.id,'voron_trident_300','a machine with no published file is named');assert.equal(n.todo.length,0);
 n=needs(null,opts,[{option:'nope'}]);assert.deepEqual(n.items,[],'an option the machine does not list is ignored');
});

test('selection: one entry per ticked row, in order',()=>{
 const rows=[{on:true,value:()=>({option:'a'})},{on:false,value:()=>({option:'b'})},{on:true,value:()=>({option:'c',tools:['x'],active:0})}];
 assert.deepEqual(selection(rows),[{option:'a'},{option:'c',tools:['x'],active:0}]);
 assert.deepEqual(selection([]),[]);
});

test('dockTools: n docks, keeping what was chosen where it still fits, else cycling the tools',()=>{
 const tools=[{id:'a'},{id:'b'},{id:'c'}];
 assert.deepEqual(dockTools(tools,2),['a','b']);
 assert.deepEqual(dockTools(tools,5),['a','b','c','a','b']);
 assert.deepEqual(dockTools(tools,3,['c','zzz']),['c','b','c'],'an unknown id is replaced');
 assert.deepEqual(dockTools(tools,1,['b','c']),['b']);
});

test('drawable: only options Stella Scope can draw',()=>{
 assert.deepEqual(drawable([O('a'),O('b',{drawable:false})]).map(o=>o.id),['a']);
 assert.deepEqual(drawable(null),[]);
});
