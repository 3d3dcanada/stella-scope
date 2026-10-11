import {test} from 'node:test';
import assert from 'node:assert/strict';
import {diffMesh,meshRgb} from '../web/next-pages.js';

test('diffMesh compares two meshes over the area both cover',()=>{
 const a={grid:[[0,0],[0,0]],min:[0,0],max:[10,10],rows:2,cols:2,ts:1},b={grid:[[0.1,0.1],[0.2,0.2]],min:[5,0],max:[15,10],rows:2,cols:2,ts:2};
 const d=diffMesh(a,b);assert.deepEqual(d.min,[5,0]);assert.deepEqual(d.max,[10,10]);assert.ok(d.diff);
 assert.ok(Math.abs(d.grid[0][0]-0.1)<1e-9&&Math.abs(d.grid.at(-1)[0]-0.2)<1e-9,'newer minus older, front row first');
 assert.ok(Math.abs(d.range_mm-0.1)<1e-9);
 assert.match(diffMesh(a,{...b,min:[20,20],max:[30,30]}).error,/do not cover/);
 assert.match(diffMesh(a,{...b,grid:null}).error,/without its grid/);});
test('meshRgb: low blue, level green, high red, clamped',()=>{
 assert.deepEqual(meshRgb(0),[24,227,107]);assert.deepEqual(meshRgb(-1),[47,107,255]);assert.deepEqual(meshRgb(1),[255,45,72]);assert.deepEqual(meshRgb(9),meshRgb(1));});
