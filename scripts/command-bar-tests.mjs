// Finish-2 F07 (2026-10-19): the command bar finds every control by its plain name; a button's accessible name holds the words on it; the chip says
// "6 days ago", not "REWOUND 6 days"; no developer word names the state chip.   node --test scripts/command-bar-tests.mjs
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {CONTROLS,controlItems,controlsLacking,paletteItems,dockFor,SUB_MENUS,accessibleName,plainState,sinceText} from '../web/next-core.js';

const all=()=>true;
// what next.js builds from controlItems for the search
const bar=(o,extra=[])=>({actions:[...controlItems(o).map(c=>({label:c.label,help:c.help,words:c.words,icon:c.icon,go(){}})),...extra],history:[],commands:[]});

// The lead's findings: it found nothing for failure watch, view, speed, flow, move or motors.
test('the command bar finds every control by its plain name',()=>{
 const idle=bar({has:all,mode:'idle'});
 for(const [typed,want] of [['failure watch','Failure watch'],['failure','Failure watch'],['view','View'],['speed','Speed'],['flow','Flow'],['move','Move'],['motors','Motors off'],['motors off','Motors off'],
  ['heat','Heat'],['fan','Fans'],['camera','Camera'],['queue','Queue'],['recording','Recording'],['home','Home all axes'],['things to print','Things to print'],['console','Console'],['macro','Macros'],['pages','Pages'],['help','Help'],['guide','Help'],['pair','Pair phone'],['filament','Load filament'],['mesh','Bed mesh'],['gantry','Quad gantry level'],['restart','Firmware restart']]){
  const items=paletteItems(idle,typed);
  assert.ok(items.some(i=>i.label===want&&i.kind==='action'),`"${typed}" finds ${want}; it found: ${items.map(i=>i.label).join(', ')||'nothing'}`);
 }
 // and by the other names people say for it
 for(const [typed,want] of [['spaghetti','Failure watch'],['faster','Speed'],['jog','Move'],['steppers','Motors off'],['extrusion','Flow'],['nozzle','Heat'],['webcam','Camera'],['whatsapp','Pair phone'],['rewind','Recording'],['kiosk','View']]){
  assert.ok(paletteItems(idle,typed).some(i=>i.label===want),`"${typed}" finds ${want}`);
 }
 // the name typed whole comes first
 assert.equal(paletteItems(idle,'speed')[0].label,'Speed');
 assert.equal(paletteItems(idle,'failure watch')[0].label,'Failure watch');
 // nothing typed: the first nine, the most used first
 const first=paletteItems(idle,'').map(i=>i.label);
 assert.equal(first.length,9);assert.deepEqual(first.slice(0,4),['Home all axes','Heat','Move','Things to print']);
});

test('a printer that cannot do a thing is not offered it, and print controls come only while they mean something',()=>{
 const noMotors=bar({has:f=>f!=='motors_off',mode:'idle'});
 assert.ok(!paletteItems(noMotors,'motors').some(i=>i.label==='Motors off'),'a printer with no motor release is not offered one');
 const prusa=controlItems({has:f=>!['klipper','level','mesh','motors_off','restart','macros','console','speed','flow','fan','jog'].includes(f),mode:'idle'}).map(c=>c.label);
 for(const l of ['Quad gantry level','Bed mesh','Motors off','Firmware restart','Macros','Console','Speed','Flow','Fans','Move','Exclude objects'])assert.ok(!prusa.includes(l),`${l} is not offered on a printer that cannot`);
 for(const l of ['Heat','Things to print','Queue','Camera','View','Failure watch','Pair phone','Recording','Home all axes'])assert.ok(prusa.includes(l),`${l} is offered everywhere`);
 const names=m=>controlItems({has:all,mode:m}).map(c=>c.label);
 assert.ok(names('printing').includes('Pause print')&&names('printing').includes('Cancel print')&&!names('printing').includes('Resume print')&&!names('printing').includes('Print again'));
 assert.ok(names('paused').includes('Resume print')&&!names('paused').includes('Pause print'));
 assert.ok(names('finished').includes('Print again')&&!names('finished').includes('Cancel print'));
 assert.ok(names('rewound').includes('Go live')&&!names('idle').includes('Go live'));
 assert.ok(!names('idle').includes('Cancel print'),'nothing to cancel when nothing prints');
});

// A printer that cannot do a thing says so when its name is typed: "nothing matches" was all a Prusa said for motors.
test('a control the printer lacks is named, not missing',()=>{
 const has=f=>!['motors_off','level','mesh','klipper','macros','console','restart','speed','flow','fan','jog'].includes(f);
 const prusa={...bar({has,mode:'idle'}),lacking:controlsLacking({has,mode:'idle'}).map(c=>({label:c.label,words:c.words}))};
 for(const [typed,what] of [['motors','Motors off'],['steppers','Motors off'],['speed','Speed'],['move','Move'],['flow','Flow'],['mesh','Bed mesh'],['gantry','Quad gantry level']]){
  const items=paletteItems(prusa,typed,{console:false});
  const row=items.find(i=>i.label===what+': not on this printer');
  assert.ok(row,`"${typed}" says ${what} is not on this printer; it said: ${items.map(i=>i.label).join(', ')}`);
  assert.equal(row.kind,'none');assert.equal(row.go,undefined,'nothing to run');
 }
 assert.ok(!paletteItems(prusa,'heat').some(i=>/not on this printer/.test(i.label)),'a control the printer has is found, not refused');
 assert.ok(paletteItems(prusa,'zzz',{console:false}).some(i=>i.kind==='none'&&/Nothing matches/.test(i.label)),'a word that names no control still says nothing matches');
 // a printer that can does not get the line
 assert.ok(!paletteItems(bar({has:all,mode:'idle'}),'motors').some(i=>/not on this printer/.test(i.label)));
 // printing: no Cancel to look for when nothing prints is not a "lack"
 assert.ok(!controlsLacking({has:()=>true,mode:'idle'}).length);
});

// The other names are whole words: "move" is Move, never Unload filament (whose other name is "remove").
test('the other names count as whole words',()=>{
 const idle=bar({has:all,mode:'idle'});
 assert.ok(!paletteItems(idle,'move').some(i=>i.label==='Unload filament'),'move is not remove');
 assert.ok(paletteItems(idle,'remove').some(i=>i.label==='Unload filament'));
 assert.ok(paletteItems(idle,'pull out').some(i=>i.label==='Unload filament'),'every typed word begins one of the names');
 const noFil=f=>f!=='filament',lacking={...bar({has:noFil,mode:'idle'}),lacking:controlsLacking({has:noFil,mode:'idle'}).map(c=>({label:c.label,words:c.words}))};
 assert.ok(!paletteItems(lacking,'move',{console:false}).some(i=>/not on this printer/.test(i.label)),'a control the printer lacks is not named for a word it only contains');
 assert.ok(paletteItems(lacking,'retract',{console:false}).some(i=>i.label==='Unload filament: not on this printer'));
 assert.ok(paletteItems(lacking,'unload',{console:false}).every(i=>i.help===undefined||i.help===''||i.kind!=='none'),'the line that says it is not on this printer is not given a second line that would be cut');
});

// Every control says how it is done, and the way exists: a sheet next.js can open, a command the printer's own buttons send, a page of the app.
test('every control names a way that exists',()=>{
 assert.equal(new Set(CONTROLS.map(c=>c.label.toLowerCase())).size,CONTROLS.length,'one name per control');
 const app=fs.readFileSync(new URL('../web/next.js',import.meta.url),'utf8'),table=app.slice(app.indexOf('function openNamedSheet'),app.indexOf('function recAction'));
 const cmds=new Set();for(const m of ['idle','printing','paused','finished','unhomed','off','klipper','replay','rewound'])for(const s of dockFor(m))if(s.cmd)cmds.add(s.cmd);
 for(const menu of Object.values(SUB_MENUS))for(const it of menu)if(it.cmd)cmds.add(it.cmd);
 for(const c of CONTROLS){
  assert.match(c.how,/^(sheet:[a-z]+|cmd:[a-z_]+|href:\/[a-z/?=&-]*|reprint|rec:live)$/,c.label);
  assert.match(c.icon,/^i-/,c.label);assert.ok(c.help&&c.words!==undefined,c.label);
  if(c.how.startsWith('sheet:'))assert.ok(new RegExp('[{,\\s]'+c.how.slice(6)+':').test(table),`${c.label}: next.js has no sheet called ${c.how.slice(6)}`);
  if(c.how.startsWith('cmd:'))assert.ok(cmds.has(c.how.slice(4)),`${c.label}: the dock sends no command called ${c.how.slice(4)}`);
 }
 // and the sheets are opened by the bar itself
 assert.match(app,/controlItems\(\{has,mode:state\.mode\}\)/);
});

// "the plate-check button's accessible name does not match its visible label": the name holds the words shown.
test('a button\'s accessible name holds the words on it',()=>{
 assert.equal(accessibleName('Set up the plate check','Plate check'),'Set up the plate check','the label is not in the title: the label is the name');
 assert.equal(accessibleName('Change the plate check','Plate check'),'Change the plate check');
 assert.equal(accessibleName('PRINT','Things to print'),'Things to print','the title holds the label: the longer title is the name');
 assert.equal(accessibleName('Home all','Home all axes'),'Home all axes');
 assert.equal(accessibleName('','Close'),'Close','an icon-only button is named by its title');
 assert.equal(accessibleName('Save',''),'Save');
 assert.equal(accessibleName('Level gantry','Quad gantry level'),'Level gantry');
 // and next-ui.js applies it to every button it makes
 const ui=fs.readFileSync(new URL('../web/next-ui.js',import.meta.url),'utf8');
 assert.match(ui,/setAttribute\('aria-label',accessibleName\(size!=='icon'\?label:'',title\|\|label\)\)/);
});

// "REWOUND 6 days": the time since the printer last answered is "6 days ago", only on the live page.
test('the chip says "6 days ago", never "REWOUND 6 days"',()=>{
 assert.equal(sinceText({state:'off',since:'6 days'},true),'6 days ago');
 assert.equal(sinceText({state:'off',since:'45 s'},true),'45 s ago');
 assert.equal(sinceText({state:'service',since:'12 s'},true),'12 s ago');
 assert.equal(sinceText({state:'off',since:'6 days'},false),'','a rewound page shows no "since": the printer being off is not part of a past moment');
 assert.equal(sinceText({state:'ready',since:'3 s'},true),'');
 assert.equal(sinceText({state:'off',since:''},true),'');
 assert.equal(sinceText(null),'');
 assert.equal(plainState('SERVICE OFF'),'STELLA SCOPE OFF');assert.equal(plainState('SERVICE OFF · LOCKED'),'STELLA SCOPE OFF · LOCKED');assert.equal(plainState('PRINTER OFF'),'PRINTER OFF');assert.match(plainState('Stella Scope service is not running: this page cannot reach it, so nothing new arrives and no command can go. The page tries again by itself and picks up at once when it answers.'),/^Stella Scope is not answering: this page cannot reach it/);assert.ok(!/service/i.test(plainState('Stella Scope service is not running: this page cannot reach it, so nothing new arrives and no command can go. The page tries again by itself and picks up at once when it answers.')));assert.equal(plainState(undefined),'');
 // next.js uses both
 const app=fs.readFileSync(new URL('../web/next.js',import.meta.url),'utf8');
 assert.match(app,/textContent=sinceText\(l,state\.live\)/);assert.match(app,/plainState\(l\.label\)/);
});
