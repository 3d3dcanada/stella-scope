import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {lookOf,modelLine,WORD,CHIP,ICON,RANK,SORTS,sortPrinters,inFilter,matchesQuery,filterOptions,counts,actionsFor,tvLayout,heatOf,budget3d,plan3d,brandOf,headerKey} from '../web/farm-wall.js';

// The capabilities and fields are what GET /api/printers returns for a demo printer and for a Klipper one.
const DEMO_CAPS={camera:false,cancel:true,pause_resume:true,start_print:true,upload:true,heat:true};
const KLIPPER_CAPS={camera:true,cancel:true,pause_resume:true,start_print:true,upload:true,heat:true};
const P=(id,state,o={})=>({id,name:o.name||id,state,driver:'demo',brand:'Voron',model:'Voron 2.4 350',capabilities:DEMO_CAPS,controls:true,job_state:state==='printing'?'printing':state==='finished'?'complete':'',...o});
const looksOf=(list,qi=new Map(),qon=false)=>new Map(list.map(p=>[p.id,lookOf(p,qi.get(p.id),qon)]));

test('lookOf: a finished print needs its plate cleared only while the queue says so',()=>{
 const fin=P('a','finished');
 assert.equal(lookOf(fin,undefined,false),'done','no queue: finished is done');
 assert.equal(lookOf(fin,{released:false},true),'clear','queue on, plate not released');
 assert.equal(lookOf(fin,{released:true},true),'done','queue on, plate released');
 assert.equal(lookOf(P('b','idle'),{awaiting_clear:true},true),'clear','awaiting clear outranks idle');
 assert.equal(lookOf(P('b','idle'),{awaiting_clear:true},false),'idle','no queue, nothing to clear');
 for(const s of ['error','offline','paused','printing'])assert.equal(lookOf(P('c',s),{awaiting_clear:true},true),s,s+' is never hidden behind the plate');
 assert.equal(lookOf(P('d','heating'),undefined,true),'heating');
 assert.equal(lookOf(P('e','idle')),'idle');
 for(const l of ['error','paused','clear','done','heating','printing','idle','offline'])assert.ok(WORD[l]&&CHIP[l]&&/^i-/.test(ICON[l])&&RANK[l]!==undefined,l);
 assert.equal(new Set(Object.values(ICON)).size,8,'every look has its own icon');
 assert.equal(new Set(Object.values(CHIP)).size,8,'every look has its own word');
});

test('the default order puts what needs a person first: error, paused, plate, done, then the rest',()=>{
 const list=[P('idle1','idle'),P('off','offline'),P('pr2','printing',{time_left_s:900}),P('pr1','printing',{time_left_s:60}),P('fin','finished'),P('done','finished'),P('pau','paused'),P('err','error'),P('heat','heating'),P('pr3','printing',{time_left_s:null})];
 const looks=looksOf(list,new Map([['fin',{released:false}],['done',{released:true}]]),true);
 const order=sortPrinters(list,looks,'attention').map(p=>p.id);
 assert.deepEqual(order,['err','pau','fin','done','heat','pr1','pr2','pr3','idle1','off']);
 // shuffling the input never changes the output (ties fall back to the name)
 let seed=7;const rnd=()=>(seed=(seed*1664525+1013904223)>>>0)/2**32;
 for(let k=0;k<20;k++){const sh=[...list].sort(()=>rnd()-.5);assert.deepEqual(sortPrinters(sh,looks,'attention').map(p=>p.id),order);}
});

test('sort by name is natural, by time soonest first with unknowns last, by progress furthest first',()=>{
 const list=[P('p10','printing',{name:'Voron 10',time_left_s:30,progress:.9}),P('p2','printing',{name:'Voron 2',time_left_s:600,progress:.2}),P('p1','idle',{name:'voron 1'}),P('p3','printing',{name:'Voron 3',time_left_s:null,progress:null})];
 const looks=looksOf(list);
 assert.deepEqual(sortPrinters(list,looks,'name').map(p=>p.name),['voron 1','Voron 2','Voron 3','Voron 10']);
 assert.deepEqual(sortPrinters(list,looks,'time').map(p=>p.id),['p10','p2','p1','p3']);
 assert.deepEqual(sortPrinters(list,looks,'progress').map(p=>p.id),['p10','p2','p1','p3']);
 assert.deepEqual(sortPrinters(list,looks,'state').map(p=>p.id),['p2','p3','p10','p1'],'printing first by name (Voron 2, 3, 10), then idle');
 assert.deepEqual(sortPrinters(list,looks,'nonsense').map(p=>p.name),['voron 1','Voron 2','Voron 3','Voron 10'],'an unknown sort falls back to the name');
 const before=JSON.stringify(list);sortPrinters(list,looks,'time');assert.equal(JSON.stringify(list),before,'the input is not changed');
});

test('time left and progress only count where they mean something',()=>{
 // the service reports a time left from the progress for any printer with a job, a stopped one included
 const list=[P('c','finished',{name:'Cancelled',job_state:'cancelled',time_left_s:5400,progress:.08}),P('f','finished',{name:'Finished',time_left_s:0,progress:1}),
  P('p','printing',{name:'Printing',time_left_s:600,progress:.5}),P('e','error',{name:'Error',time_left_s:100,progress:.9}),P('i','idle',{name:'Idle'}),P('z','paused',{name:'Paused',time_left_s:50,progress:.2})];
 const looks=looksOf(list,new Map(),false);
 assert.deepEqual(sortPrinters(list,looks,'time').map(p=>p.id),['f','p','c','e','i','z'],'finished first (nothing left), then by time, the rest by name');
 assert.deepEqual(sortPrinters(list,looks,'progress').map(p=>p.id),['f','p','z','c','e','i'],'furthest first; error and idle have none');
});

test('the model line does not say the brand twice',()=>{
 assert.equal(modelLine({brand:'Voron',model:'Voron 2.4 350'}),'Voron 2.4 350');
 assert.equal(modelLine({brand:'Prusa',model:'CORE One L'}),'Prusa · CORE One L');
 assert.equal(modelLine({brand:'Klipper',model:''}),'Klipper');
 assert.equal(modelLine({driver:'moonraker'}),'moonraker');
 assert.equal(modelLine({}),'');
});

test('filter groups and counts agree with each other',()=>{
 const list=[P('a','printing'),P('b','heating'),P('c','paused'),P('d','finished'),P('e','finished'),P('f','error'),P('g','idle'),P('h','offline',{driver:'moonraker',brand:'Klipper'}),P('i','idle',{driver:'prusalink',brand:'Prusa'})];
 const looks=looksOf(list,new Map([['d',{released:false}],['e',{released:true}]]),true);
 const ids=k=>list.filter(p=>inFilter(k,p,looks.get(p.id))).map(p=>p.id);
 assert.deepEqual(ids('all'),list.map(p=>p.id));
 assert.deepEqual(ids('printing'),['a','b']);
 assert.deepEqual(ids('attention'),['c','d','e','f'],'paused, plate needs clearing, done, error');
 assert.deepEqual(ids('idle'),['g','i']);
 assert.deepEqual(ids('offline'),['h']);
 assert.deepEqual(ids('demo'),['a','b','c','d','e','f','g']);
 assert.deepEqual(ids('brand:Prusa'),['i']);
 assert.deepEqual(ids('brand:Voron').length,7);
 const c=counts(list,looks);
 assert.deepEqual(c,{printing:2,attention:4,idle:2,offline:1});
 assert.equal(c.printing+c.attention+c.idle+c.offline,list.length,'every printer is in exactly one header group');
 const opts=filterOptions(list,looks),by=k=>opts.find(o=>o.key===k);
 assert.equal(by('all').count,9);assert.equal(by('attention').count,4);assert.equal(by('demo').count,7);
 assert.equal(by('attention').label,'Needs attention');assert.equal(by('attention').short,'Attention');assert.equal(by('demo').short,'Demo');assert.equal(by('printing').short,'Printing');
 for(const o of opts)assert.ok(`${o.short} (${o.count})`.length<=16,'a phone\'s menu holds about 16 characters: '+o.short);
 assert.deepEqual(opts.filter(o=>o.key.startsWith('brand:')).map(o=>o.label),['Klipper','Prusa','Voron']);
 // one brand and no demo mix: nothing to tell apart, so no brand or demo entries
 const one=[P('x','idle'),P('y','idle')];
 assert.deepEqual(filterOptions(one,looksOf(one)).map(o=>o.key),['all','printing','attention','idle','offline']);
 assert.equal(brandOf({driver:'moonraker'}),'moonraker');
});

test('search: every word must match something about the printer',()=>{
 const p=P('p03','printing',{name:'Cobalt',model:'CORE One L',brand:'Prusa',file:'3D3D twist bowl.gcode'});
 for(const q of ['','cobalt','COBALT','prusa core','bowl','demo','printing','prusa bowl'])assert.ok(matchesQuery(p,'printing',q),q);
 for(const q of ['voron','cobalt voron','paused','xyz'])assert.ok(!matchesQuery(p,'printing',q),q);
 assert.ok(matchesQuery(P('e','error',{detail:'Klipper state: shutdown'}),'error','shutdown'));
 assert.ok(matchesQuery(P('c','finished'),'clear','needs clearing'));
 assert.ok(!matchesQuery(P('r','idle',{driver:'prusalink'}),'idle','demo'),'only demo printers say demo');
});

test('actionsFor offers only what the printer can do now',()=>{
 const caps=KLIPPER_CAPS;
 assert.deepEqual(Object.entries(actionsFor(P('a','printing',{capabilities:caps}))).filter(([,v])=>v).map(([k])=>k),['pause','stop']);
 assert.deepEqual(Object.entries(actionsFor(P('a','paused',{capabilities:caps}))).filter(([,v])=>v).map(([k])=>k),['resume','stop']);
 assert.deepEqual(Object.entries(actionsFor(P('a','idle',{capabilities:caps}))).filter(([,v])=>v),[],'idle: nothing without a queued file');
 assert.ok(actionsFor(P('a','finished',{capabilities:caps}),{qon:true,qi:{released:false}}).clear);
 assert.ok(!actionsFor(P('a','finished',{capabilities:caps}),{qon:true,qi:{released:true}}).clear);
 assert.ok(actionsFor(P('a','finished',{capabilities:caps}),{qon:true,qi:{released:true,next_job:4}}).next,'start next once the plate is released');
 assert.ok(!actionsFor(P('a','finished',{capabilities:caps}),{qon:false,qi:{released:false}}).clear,'no queue, no plate button');
 const off=actionsFor(P('a','printing',{capabilities:caps,controls:false}));assert.ok(!off.pause&&!off.stop,'controls off: nothing to press');
 assert.ok(!actionsFor(P('a','printing',{capabilities:{...caps,pause_resume:false,cancel:false}})).pause);
 assert.ok(actionsFor(P('a','idle',{plug:{spec:'shelly://x'}}),{canManage:true}).power);
 assert.ok(!actionsFor(P('a','idle',{plug:{spec:'shelly://x'}}),{canManage:false}).power);
 assert.ok(actionsFor(P('a','idle',{camera_url:'/api/p/a/camera/snapshot'})).camera);
 assert.ok(!actionsFor(P('a','offline',{camera_url:'/api/p/a/camera/snapshot'})).camera);
});

test('the sort menu starts with the default and lists every sort the code knows',()=>{
 assert.equal(SORTS[0][0],'attention','needs attention first is the default');
 for(const [,l] of SORTS)assert.ok(l.length<=16,'a phone\'s menu holds about 16 characters: '+l);
 assert.deepEqual(SORTS.map(s=>s[0]),['attention','name','state','time','progress']);
 const list=[P('a','idle')],looks=looksOf(list);
 for(const [k] of SORTS)assert.equal(sortPrinters(list,looks,k).length,1,k);
});

// U04 (2026-10-20): the S/M/L switch and its defaults (defaultSize, validSize, SIZES) are gone: there is one tile size. These two tests replace the one that
// asserted the old sizes: what is left of "how big" is the wall screen's layout, and the heat gauges read the printer's own numbers.
test('the wall screen picks the columns that give each tile the shape of a tile and says when they fill the screen',()=>{
 assert.deepEqual(tvLayout(0,1920,1000),{cols:1,rows:1,fill:false});
 const six=tvLayout(6,1920,990);assert.deepEqual([six.cols,six.rows,six.fill],[3,2,true],'six printers on a 1080p screen: three by two');
 const one=tvLayout(1,1920,990);assert.equal(one.cols,1);assert.equal(one.fill,true);
 const nine=tvLayout(9,1920,990);assert.deepEqual([nine.cols,nine.rows,nine.fill],[5,2,true],'nine: five by two with one empty cell, rather than three rows under 340 px tall');
 assert.equal(tvLayout(10,1366,666).fill,false,'ten printers on a 1366 by 768 screen do not fit at 340 by 250 each: the wall wraps and scrolls instead of squeezing the tiles');
 assert.deepEqual(Object.values(tvLayout(6,1366,700)).slice(0,3),[3,2,true],'six on a 1366 screen with a little more height fill it');
 const ten=tvLayout(10,1920,980);assert.deepEqual([ten.cols,ten.rows,ten.fill],[5,2,true],'ten printers on a 1080p screen: five by two, every tile 340 px tall or more');
 const many=tvLayout(60,1920,990);assert.equal(many.fill,false,'sixty printers do not fill the screen: the wall wraps and scrolls');
 for(const n of [1,2,3,4,5,6,7,8,9,12,20])for(const [w,h] of [[1920,990],[1366,680],[3840,2000]]){const l=tvLayout(n,w,h);assert.ok(l.cols>=1&&l.cols<=n&&l.cols*l.rows>=n,`${n} at ${w}x${h}`);}
});
test('a heat gauge reads the printer\'s own numbers and a heater with none is not drawn',()=>{
 assert.deepEqual(heatOf({temp:24.6,target:0}),{has:true,now:24.6,target:0});
 assert.deepEqual(heatOf({temp:239,target:240}),{has:true,now:239,target:240});
 assert.equal(heatOf(undefined).has,false);assert.equal(heatOf({temp:null,target:null}).has,false);assert.equal(heatOf({temp:NaN}).has,false);
 assert.deepEqual(heatOf({target:60}),{has:true,now:null,target:60},'a target with no reading still draws, with the dot where the gauge is');
});

test('WebGL budget: a phone holds fewest, a big screen most, always under Chrome\'s cap of about 16',()=>{
 assert.equal(budget3d(390),3);assert.equal(budget3d(600,true),3);assert.equal(budget3d(1024),6);assert.equal(budget3d(1366,true),6);assert.equal(budget3d(1920),10);
 for(const w of [320,390,600,601,1024,1100,1101,1920,3840])for(const c of [false,true])assert.ok(budget3d(w,c)<=12);
});

test('plan3d never lets more than the budget run, whatever the tiles do',()=>{
 let seed=11;const rnd=()=>(seed=(seed*1664525+1013904223)>>>0)/2**32;
 for(let trial=0;trial<500;trial++){
  const budget=1+Math.floor(rnd()*12),n=1+Math.floor(rnd()*80);
  let items=Array.from({length:n},(_,i)=>({id:'t'+i,want:false,visible:false,state:'none'}));
  for(let step=0;step<30;step++){
   // the user scrolls: a window of tiles is on screen, the ones just left are still wanted for a moment
   const from=Math.floor(rnd()*n),span=1+Math.floor(rnd()*20);
   items=items.map((t,i)=>{const vis=i>=from&&i<from+span;return {...t,visible:vis,want:vis||(t.state!=='none'&&t.want&&rnd()<.5)};});
   const plan=plan3d(items,budget),startS=new Set(plan.start),stopS=new Set(plan.stop);
   for(const id of plan.start)assert.equal(items.find(t=>t.id===id).state,'none','only idle tiles start');
   for(const id of plan.stop)assert.notEqual(items.find(t=>t.id===id).state,'none','only running tiles stop');
   const after=items.map(t=>({...t,state:stopS.has(t.id)?'none':startS.has(t.id)?'loading':t.state}));
   const running=after.filter(t=>t.state!=='none');
   assert.ok(running.length<=budget,`trial ${trial}: ${running.length} running over budget ${budget}`);
   assert.ok(running.every(t=>t.want),'a tile nobody wants is never left running');
   const visible=after.filter(t=>t.visible&&t.want);
   assert.ok(running.filter(t=>t.visible).length>=Math.min(visible.length,budget)-0||running.length===budget,'on-screen tiles get places before tiles that left');
   items=after.map(t=>t.state==='loading'?{...t,state:'live'}:t);
  }
 }
});

test('plan3d keeps what is on screen running and frees places taken by tiles that left',()=>{
 const mk=(id,state,visible,want=true)=>({id,want,visible,state});
 // 3 places; a and b are on screen and live, c just left and is live, d and e are on screen waiting
 let plan=plan3d([mk('a','live',true),mk('b','live',true),mk('c','live',false),mk('d','none',true),mk('e','none',true)],3);
 assert.deepEqual(plan,{start:['d'],stop:['c']},'c gives its place to the first waiting tile on screen');
 // nothing on screen wants a place: the leavers stay until their grace ends, then they are unwanted and stop
 plan=plan3d([mk('a','live',false),mk('b','live',false,false)],3);assert.deepEqual(plan,{start:[],stop:['b']});
 // the budget shrinks (a resize to a phone): the later tiles stop first
 plan=plan3d([mk('a','live',true),mk('b','live',true),mk('c','live',true),mk('d','live',true)],2);assert.deepEqual(plan,{start:[],stop:['c','d']});
 // a tile still loading counts as running
 plan=plan3d([mk('a','loading',true),mk('b','none',true)],1);assert.deepEqual(plan,{start:[],stop:[]});
 assert.deepEqual(plan3d([],5),{start:[],stop:[]});
 assert.deepEqual(plan3d([mk('a','none',true)],0),{start:[],stop:[]},'a budget of zero starts nothing');
});

test('headerKey: the header is rewritten when something it shows changes, and not when a percentage ticks over',()=>{
 const base=counts([P('a','printing'),P('b','idle')],looksOf([P('a','printing'),P('b','idle')]));
 const k=(c,o)=>headerKey(c,o);
 const o={filter:'all',queued:2,next:{name:'Atlas'},clock:'10:15 AM',phone:false,first:'a'};
 assert.equal(k(base,o),k({...base},{...o}),'the same facts, the same key');
 for(const [what,change] of [['a count',[{...base,printing:base.printing+1},o]],['the filter',[base,{...o,filter:'printing'}]],['the queue',[base,{...o,queued:3}]],['the next to finish',[base,{...o,next:{name:'Bolt'}}]],['its time',[base,{...o,clock:'10:16 AM'}]],['a phone',[base,{...o,phone:true}]],['the first printer',[base,{...o,first:'b'}]]])
  assert.notEqual(k(...change),k(base,o),what+' changes the key');
 assert.equal(k(base,{...o,next:null,clock:''}),k(base,{...o,next:null,clock:'11:00 AM'}),'no printer to finish, no clock in the key');
 assert.equal(k(base,{...o,queued:-1}),k(base,{...o,queued:-1}),'the queue being off is its own value');
});

// The page's scripts are served as they are and there is no build step: a bracket swallowed by a trailing comment is only found by loading the page.
test('the wall\'s scripts parse as modules',()=>{
 for(const f of ['farm.js','farm-wall.js','model-view.js','next-fleet.js']){
  const r=spawnSync(process.execPath,['--input-type=module','--check'],{input:readFileSync(new URL('../web/'+f,import.meta.url))});
  assert.equal(r.status,0,f+': '+String(r.stderr).split('\n').slice(0,4).join(' '));}
});
