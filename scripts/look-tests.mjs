// Look A (2026-10-23): a signal says what it is in plain words first, Klipper's name beside it.
import test from 'node:test';import assert from 'node:assert/strict';
import {plainSignal} from '../web/next-pages.js';
test('Klipper objects and fields read as plain words',()=>{
 assert.deepEqual(plainSignal('extruder','temperature'),{object:'Nozzle heater',field:'Temperature'});
 assert.deepEqual(plainSignal('heater_bed','power'),{object:'Bed heater',field:'Heater power'});
 assert.deepEqual(plainSignal('tmc2209 stepper_z','run_current'),{object:'Motor driver Z (TMC2209)',field:'Run current'});
 assert.deepEqual(plainSignal('tmc5160 extruder','hold_current'),{object:'Motor driver extruder (TMC5160)',field:'Hold current'});
 assert.deepEqual(plainSignal('gcode_move','homing_origin.2'),{object:'Motion as G-code sees it',field:'Homing offset Z'});
 assert.deepEqual(plainSignal('controller_fan controller_fan','rpm'),{object:'Controller fan',field:'Fan RPM'});
 assert.deepEqual(plainSignal('temperature_sensor chamber_temp','temperature'),{object:'Temperature: chamber temp',field:'Temperature'});
});
test('an unknown name is still readable, never empty',()=>{
 assert.deepEqual(plainSignal('neopixel case_lights','color_data'),{object:'Neopixel case lights',field:'Color data'});
 assert.deepEqual(plainSignal('',''),{object:'',field:''});
});
