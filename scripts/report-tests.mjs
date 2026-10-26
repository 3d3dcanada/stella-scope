// The print report's arithmetic (web/next-report.js): grams from metres of 1.75 mm filament and the material's density, money from the
// price a kilo, what was recorded at a moment, and which timelapse belongs to which print.
import assert from 'node:assert/strict';
globalThis.document ??= {addEventListener(){},documentElement:{dataset:{}},getElementById:()=>null};
globalThis.window ??= globalThis;globalThis.matchMedia ??= ()=>({matches:false});
const {grams,cost,at,matchTimelapse}=await import('../web/next-report.js');
let n=0;const near=(a,b,m,e=0.01)=>{n++;assert.ok(Math.abs(a-b)<=e,`${m}: ${a} vs ${b}`);},is=(a,b,m)=>{n++;assert.deepEqual(a,b,m);};
near(grams(1000,1.24),2.982,'one metre of PLA is about 2.98 g');
near(grams(10000,1.27),30.54,'ten metres of PETG',0.05);
is(grams(1000,null),null,'no density, no grams (never a made-up default)');
is(grams(0,1.24),null,'nothing used');
near(cost(250,24),6,'250 g at 24 a kilo');
is(cost(250,null),null,'no price, no cost');
const log=[{name:'Smooth PEI',at:100},{name:'Textured PEI',at:500}];
is(at(log,300)?.name,'Smooth PEI','the plate on the bed at that time');
is(at(log,1000)?.name,'Textured PEI','the later one after it was swapped');
is(at(log,10),null,'nothing recorded yet');
const tls=[{id:'1000-a',file:'a.gcode',started:1000},{id:'5000-b',file:'b.gcode',started:5000}];
is(matchTimelapse(tls,{start_time:1200,filename:'x.gcode'})?.id,'1000-a','started within ten minutes');
is(matchTimelapse(tls,{start_time:6500,filename:'folder/b.gcode'})?.id,'5000-b','the same file, the same day');
is(matchTimelapse(tls,{start_time:90000,filename:'c.gcode'}),null,'none');
console.log(`report: ${n} checks pass`);
