// WO-10: the GLB reader the printer-model view uses. A synthetic two-part file proves the reading and
// the refusals; a real published model is read too when one is at $STELLA_TEST_GLB (skipped otherwise).
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
import {parseGlb,readParts,groupIndex,simpleSet,boundsAfter,triangles,offsetOf} from '../web/model-glb.js';

function glb(json,bin){
  const j=Buffer.from(JSON.stringify(json));const jp=Buffer.concat([j,Buffer.alloc((4-j.length%4)%4,0x20)]);
  const bp=Buffer.concat([bin,Buffer.alloc((4-bin.length%4)%4)]);
  const h=Buffer.alloc(12);h.writeUInt32LE(0x46546c67,0);h.writeUInt32LE(2,4);h.writeUInt32LE(12+8+jp.length+8+bp.length,8);
  const c1=Buffer.alloc(8);c1.writeUInt32LE(jp.length,0);c1.writeUInt32LE(0x4e4f534a,4);
  const c2=Buffer.alloc(8);c2.writeUInt32LE(bp.length,0);c2.writeUInt32LE(0x004e4942,4);
  const all=Buffer.concat([h,c1,jp,c2,bp]);return all.buffer.slice(all.byteOffset,all.byteOffset+all.length);
}
// Two triangles' worth of vertices: a 10 mm part and a 2 mm part (metres in the file).
const pos=new Float32Array([0,0,0, .01,0,0, 0,.01,0,  .1,.1,.1, .102,.1,.1, .1,.102,.1]);
const idx=new Uint32Array([0,1,2]);
const bin=Buffer.concat([Buffer.from(pos.buffer),Buffer.from(idx.buffer)]);
const json={asset:{version:'2.0'},scene:0,scenes:[{nodes:[0,1]}],
  nodes:[{name:'part_0',mesh:0},{name:'part_1',mesh:1}],
  meshes:[{name:'part_0',extras:{part_key:'0',group:'fixed'},primitives:[{attributes:{POSITION:0},indices:2,material:0}]},
          {name:'part_1',extras:{part_key:'1'},primitives:[{attributes:{POSITION:1},indices:2,material:0}]}],
  materials:[{pbrMetallicRoughness:{baseColorFactor:[1,0,0,1],metallicFactor:0,roughnessFactor:.5},doubleSided:true}],
  accessors:[{bufferView:0,byteOffset:0,componentType:5126,count:3,type:'VEC3',min:[0,0,0],max:[.01,.01,0]},
             {bufferView:0,byteOffset:36,componentType:5126,count:3,type:'VEC3',min:[.1,.1,.1],max:[.102,.102,.1]},
             {bufferView:1,componentType:5125,count:3,type:'SCALAR'}],
  bufferViews:[{buffer:0,byteOffset:0,byteLength:72},{buffer:0,byteOffset:72,byteLength:12}],buffers:[{byteLength:84}]};

const g=parseGlb(glb(json,bin));
const parts=readParts(g);
assert.equal(parts.length,2);
assert.equal(parts[0].key,'0');assert.equal(parts[1].key,'1');
assert.equal(parts[0].group,'fixed');
assert.deepEqual([...parts[0].material.color],[1,0,0]);assert.equal(parts[0].material.double,true);
assert.equal(triangles(parts),2,'one triangle each');
assert.equal(parts[0].pos.length,9,'positions are a view of the file, three floats a vertex');
assert.ok(Math.abs(parts[0].pos[3]-0.01)<1e-7);
// A node's own translation (metres) is kept for the view to apply; a node with none, or a zero one, has none
// (polish WO-02: the V2.4 350 and the Trident carry some, and a part drawn without it sits in the wrong place).
assert.equal(parts[0].t,null);
const moved=structuredClone(json);moved.nodes[1].translation=[0.05,0,-0.025];moved.nodes[0].translation=[0,0,0];
const mp=readParts(parseGlb(glb(moved,bin)));
assert.deepEqual(mp[1].t,[0.05,0,-0.025]);assert.equal(mp[0].t,null,'a zero translation is no translation');
// Which motion group a part is in comes from the registration, by the project's part key.
const groups=groupIndex({groups:{head:['1'],bed:['7']}});
assert.equal(groups.get('1'),'head');assert.equal(groups.get('0'),undefined);
// The simple view drops what is smaller than a fastener (12 mm = 0.012 in the file's metres).
assert.deepEqual([...simpleSet(parts,0.012)],[0],'the 14 mm part stays, the 3 mm part is dropped');

// Bounds are what the camera is framed on.
const [lo,hi]=boundsAfter(parts,null);
assert.ok(Math.abs(lo[0])<1e-9&&Math.abs(hi[0]-0.102)<1e-6);
// What it will not read is said, not guessed at.
assert.throws(()=>parseGlb(new Uint8Array(40).buffer),/not a GLB/);
assert.throws(()=>parseGlb(glb({...json,extensionsRequired:['KHR_draco_mesh_compression']},bin)),/extension/);
const sparse=structuredClone(json);sparse.accessors[0].sparse={count:1};
assert.throws(()=>readParts(parseGlb(glb(sparse,bin))),/not supported/);
const bad=structuredClone(json);bad.accessors[0].componentType=5122;
assert.throws(()=>readParts(parseGlb(glb(bad,bin))),/number format/);

// Polish WO-01: an option file is placed in the machine's CAD frame (millimetres) and drawn in the file's (metres, Y up).
// The first dock of a Voron 2.4 300 bank sits at the rig's (-134.99991944981247, -214.99992383691756, 279.9990417204351).
const o=offsetOf([[1,0,0],[0,0,1],[0,-1,0]],[-134.99991944981247,-214.99992383691756,279.9990417204351]);
assert.ok(Math.abs(o[0]+0.13499991944981247)<1e-12&&Math.abs(o[1]-0.2799990417204351)<1e-12&&Math.abs(o[2]-0.21499992383691756)<1e-12,'X stays, CAD Z becomes up, CAD Y becomes back');
assert.deepEqual(offsetOf(null,[1000,0,0]),[1,0,0],'no basis given: the Z-up default');
assert.deepEqual(offsetOf([[1,0,0],[0,1,0],[0,0,1]],[0,2000,0]),[0,2,0],'a model already Y up keeps its axes');


// F03 (2026-10-19): a node placed by a 4 x 4 matrix (the Annex K1, K2 and K3), a part named by its node when the file has no part key, and the box a part occupies
// once placed. Here: the 10 mm part (0,0,0)-(.01,.01,0) turned a quarter about X (local y becomes z, local z becomes -y) and moved .05 m along X.
const placed=structuredClone(json);
placed.nodes=[{name:'p00007__M3 heat set',mesh:0,matrix:[1,0,0,0, 0,0,1,0, 0,-1,0,0, 0.05,0,0,1]},{name:'part_1',mesh:1,rotation:[Math.SQRT1_2,0,0,Math.SQRT1_2],scale:[2,2,2],translation:[0,0.1,0]}];
placed.meshes[0].extras={};
const pp=readParts(parseGlb(glb(placed,bin)));
assert.equal(pp[0].key,'p00007','a part with no part key in the file is named by its node');
assert.equal(pp[0].t,null,'a matrix replaces the node translation');
assert.deepEqual(pp[0].m,[1,0,0,0, 0,0,1,0, 0,-1,0,0, 0.05,0,0,1]);
const near=(a,b)=>a.every((v,i)=>Math.abs(v-b[i])<1e-9);
assert.ok(near(pp[0].wmin,[0.05,0,0])&&near(pp[0].wmax,[0.06,0,0.01]),'the box is the mesh box through the matrix: x .05 to .06, z 0 to .01: '+pp[0].wmin+' '+pp[0].wmax);
assert.equal(pp[0].min[0],0,'the mesh keeps its own box (its geometry\'s)');
// a rotation and a scale compose into the same kind of matrix: a quarter about X, doubled, moved .1 up
assert.ok(near(pp[1].m,[2,0,0,0, 0,0,2,0, 0,-2,0,0, 0,0.1,0,1]),'rotation and scale become a matrix: '+pp[1].m.map(v=>Math.round(v*1e9)/1e9));
// framing a machine uses where the parts are, not where their meshes are
const [fl,fh]=boundsAfter(pp,null);
assert.ok(fl[0]<=0.05+1e-9&&fh[0]>=0.204-1e-9&&fl[1]<=-0.1+1e-9,'bounds follow the placed parts (the second goes to x .2, y -.1, z .2): '+fl+' '+fh);
// an orientation (three rows, display = orient * file) carries them to the display's frame
const turned=boundsAfter(pp.slice(0,1),[[1,0,0],[0,0,-1],[0,1,0]]);
assert.ok(near(turned[0],[0.05,-0.01,0])&&near(turned[1],[0.06,0,0]),'the placed part, turned (y is minus z, z is y): '+turned);

// A real published model, when one is at hand: the reader agrees with the file's own counts.
const real=process.env.STELLA_TEST_GLB||'';
if(real&&existsSync(real)){
  const b=readFileSync(real);const rg=parseGlb(b.buffer.slice(b.byteOffset,b.byteOffset+b.length));
  const rp=readParts(rg);
  assert.equal(rp.length,rg.json.nodes.length);
  assert.ok(rp.every(p=>p.key!==''),'every part carries the project\'s part key');
  const [l,h]=boundsAfter(rp,null);
  assert.ok(h[1]-l[1]>0.1&&h[1]-l[1]<1,'about a machine tall, in metres');
  console.log('real model:',rp.length,'parts',triangles(rp),'triangles');
}else console.log('real model: skipped (no file at',real+')');
console.log('model-glb tests passed');
