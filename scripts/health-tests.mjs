// The Health page's verdicts (web/next-health.js): belts that answer alike, a resonance that falls, a heater that slows.
import assert from 'node:assert/strict';
globalThis.document ??= {addEventListener(){},documentElement:{dataset:{}},getElementById:()=>null};
globalThis.window ??= globalThis;globalThis.matchMedia ??= ()=>({matches:false});
const {beltWord,peakWord,heaterWord}=await import('../web/next-health.js');
let n=0;const is=(a,b,m)=>{n++;assert.deepEqual(a,b,m);};
is(beltWord(0.97)[1],'ok','belts alike');is(beltWord(0.9)[1],'warn','a little apart');is(beltWord(0.7)[1],'alarm','apart');is(beltWord(NaN)[0],'—','no measurement');
is(peakWord(50,49)[1],'ok','within 5 %');is(peakWord(50,45)[1],'warn','down 10 %: a belt going slack');is(peakWord(NaN,45)[0],'—','one end unknown');
is(heaterWord(-5)[1],'ok','as usual');is(heaterWord(-30)[1],'warn','much slower than its own usual');
console.log(`health: ${n} checks pass`);
