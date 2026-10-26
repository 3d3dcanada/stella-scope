// The Calibrate page reads the figures Klipper prints when a calibration ends (web/next-calibrate.js READ), in Klipper's own words.
import assert from 'node:assert/strict';
globalThis.document ??= {addEventListener(){},documentElement:{dataset:{}},getElementById:()=>null};
globalThis.window ??= globalThis;globalThis.matchMedia ??= ()=>({matches:false});
const {READ,STEPS,PRUSA}=await import('../web/next-calibrate.js');
let n=0;const is=(a,b,m)=>{n++;assert.deepEqual(a,b,m);};
is(READ.pid('// PID parameters: pid_Kp=22.865 pid_Ki=1.292 pid_Kd=101.178'),{kp:22.865,ki:1.292,kd:101.178},'PID_CALIBRATE result');
is(READ.pid('// Heater extruder approved'),null,'a line with no PID values');
is(READ.probeZ('// probe: z_offset: 1.985\n// The SAVE_CONFIG command will update the printer config file'),{z_offset:1.985},'PROBE_CALIBRATE result');
is(READ.endstopZ('// stepper_z: position_endstop: -0.425'),{position_endstop:-0.425},'Z_ENDSTOP_CALIBRATE result, below zero');
is(READ.testZ('// Z position: 0.4000 --> 0.3500 <-- 0.3000'),0.35,'the paper test position between two tries');
is(READ.testZ('// Z position: ?????? --> 5.000 <-- ??????'),5,'the first position');
is(READ.qgl('// Retries: 1/5 Probed points range: 0.004120 tolerance: 0.007500'),{range:0.00412},'gantry level range');
is(STEPS.map(s=>s.id),['pid_nozzle','pid_bed','z_offset','gantry','mesh','shaper','rotation','pressure_advance','flow'],'the setup order');
is(PRUSA[0].id,'prusa_selftest','a Prusa starts with its selftest');
for(const s of [...STEPS,...PRUSA]){n++;assert.match(s.id,/^[a-z0-9_]{1,40}$/,`${s.id} is a record name the service accepts`);}
console.log(`calibrate: ${n} checks pass`);
