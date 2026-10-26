// Reading a printer's real CAD from a GLB file (farm board WO-10). The models from 3D Print Rig are flat:
// one node and one mesh per part, float32 positions and normals, uint32 indices, plain colour materials,
// no extensions, no textures. This reads exactly that and nothing else, and says so when a file is
// something else, rather than drawing a guess. No three.js here: it runs in the page and under node.

const CHUNK_JSON = 0x4e4f534a;
const CHUNK_BIN = 0x004e4942;
const FLOAT = 5126, UINT = 5125, USHORT = 5123, UBYTE = 5121;

export function parseGlb(buffer){
  const dv=new DataView(buffer);
  if(buffer.byteLength<20||dv.getUint32(0,true)!==0x46546c67)throw Error('This is not a GLB model file.');
  if(dv.getUint32(4,true)!==2)throw Error('Only GLB version 2 files are supported.');
  let json=null,bin=null,off=12;
  while(off+8<=buffer.byteLength){
    const len=dv.getUint32(off,true),type=dv.getUint32(off+4,true),start=off+8;
    if(start+len>buffer.byteLength)throw Error('The model file is cut short.');
    if(type===CHUNK_JSON)json=JSON.parse(new TextDecoder().decode(new Uint8Array(buffer,start,len)));
    else if(type===CHUNK_BIN&&!bin)bin={start,len};
    off=start+((len+3)&~3);
  }
  if(!json||!bin)throw Error('The model file has no geometry in it.');
  const need=(json.extensionsRequired||[]);
  if(need.length)throw Error('This model needs a GLB extension ('+need.join(', ')+') that is not supported, so it is not drawn.');
  return {json,buffer,binStart:bin.start};
}

function view(g,accessor,types){
  const a=g.json.accessors[accessor];
  if(!a||a.sparse)throw Error('The model uses a layout that is not supported.');
  const bv=g.json.bufferViews[a.bufferView],comps={SCALAR:1,VEC2:2,VEC3:3,VEC4:4}[a.type];
  if(!comps||!types.includes(a.componentType))throw Error('The model uses a number format that is not supported.');
  const T={[FLOAT]:Float32Array,[UINT]:Uint32Array,[USHORT]:Uint16Array,[UBYTE]:Uint8Array}[a.componentType];
  if(bv.byteStride&&bv.byteStride!==comps*T.BYTES_PER_ELEMENT)throw Error('The model uses interleaved data that is not supported.');
  const offset=g.binStart+(bv.byteOffset||0)+(a.byteOffset||0);
  if(offset%T.BYTES_PER_ELEMENT!==0)throw Error('The model data is not aligned.');
  return {array:new T(g.buffer,offset,a.count*comps),count:a.count,comps,min:a.min,max:a.max};
}

// F03 (2026-10-19): a node may place its part with a 4 x 4 matrix (the Annex K1, K2 and K3: 1,500 to 1,800 nodes, 200 to 470 meshes shared between them, every
// node a matrix, in the file's column-major order) or with a rotation and a scale. Before this a part placed that way was drawn at the origin in its own
// frame, a heap of panels. A part with no `part_key` in the file is named by its node (`p00634__M3 heat set` is part `p00634`), as the project does.
function nodeMatrix(node){
  if(Array.isArray(node.matrix)&&node.matrix.length===16)return node.matrix.slice();
  if(!node.rotation&&!node.scale)return null;
  const [x,y,z,w]=node.rotation||[0,0,0,1],[sx,sy,sz]=node.scale||[1,1,1],[tx,ty,tz]=node.translation||[0,0,0];
  const xx=x*x,yy=y*y,zz=z*z,xy=x*y,xz=x*z,yz=y*z,wx=w*x,wy=w*y,wz=w*z;
  return [(1-2*(yy+zz))*sx,2*(xy+wz)*sx,2*(xz-wy)*sx,0, 2*(xy-wz)*sy,(1-2*(xx+zz))*sy,2*(yz+wx)*sy,0, 2*(xz+wy)*sz,2*(yz-wx)*sz,(1-2*(xx+yy))*sz,0, tx,ty,tz,1];
}
/** The corners of a local box through a column-major 4 x 4: the box that holds them. */
function throughMatrix(m,min,max){
  const lo=[Infinity,Infinity,Infinity],hi=[-Infinity,-Infinity,-Infinity];
  for(const x of [min[0],max[0]])for(const y of [min[1],max[1]])for(const z of [min[2],max[2]])
    for(let r=0;r<3;r++){const v=m[r]*x+m[4+r]*y+m[8+r]*z+m[12+r];if(v<lo[r])lo[r]=v;if(v>hi[r])hi[r]=v;}
  return [lo,hi];
}

/** The parts of the model: name, key (the project's part id), colour, geometry views and bounding box.
 *  Geometry is a view on the file's own bytes, not a copy. */
export function readParts(g){
  const {json}=g,out=[];
  const matOf=i=>{
    const m=(json.materials||[])[i]||{},p=m.pbrMetallicRoughness||{},c=p.baseColorFactor||[0.7,0.7,0.7,1];
    return {color:c.slice(0,3),alpha:c[3]??1,metal:p.metallicFactor??0.1,rough:p.roughnessFactor??0.6,double:!!m.doubleSided,name:m.name||''};
  };
  (json.nodes||[]).forEach((node,ni)=>{
    if(node.mesh===undefined)return;
    const mesh=json.meshes[node.mesh];
    for(const prim of mesh.primitives||[]){
      if(prim.mode!==undefined&&prim.mode!==4)continue;
      const pos=view(g,prim.attributes.POSITION,[FLOAT]);
      const nor=prim.attributes.NORMAL!==undefined?view(g,prim.attributes.NORMAL,[FLOAT]):null;
      const idx=prim.indices!==undefined?view(g,prim.indices,[UINT,USHORT]):null;
      const tris=Math.floor((idx?idx.count:pos.count)/3);
      let min=pos.min,max=pos.max;
      if(!min||!max){min=[Infinity,Infinity,Infinity];max=[-Infinity,-Infinity,-Infinity];
        for(let i=0;i<pos.array.length;i+=3)for(let k=0;k<3;k++){const v=pos.array[i+k];if(v<min[k])min[k]=v;if(v>max[k])max[k]=v;}}
      const e=mesh.extras||{},m=nodeMatrix(node),named=String(node.name||'').match(/^(p\d+)__/);
      const [wmin,wmax]=m?throughMatrix(m,min,max):[null,null];
      out.push({node:ni,name:node.name||mesh.name||('part_'+ni),key:String(e.part_key??node.extras?.part_key??named?.[1]??''),
        role:e.appearance_role||'',group:e.group||'',material:matOf(prim.material),pos:pos.array,nor:nor?.array||null,idx:idx?.array||null,tris,min,max,
        // 2026-10-18 (polish WO-02): a node may carry a translation (metres) the vertices do not; the V2.4 350 and the Trident have some.
        t:!m&&Array.isArray(node.translation)&&node.translation.some(v=>v)?node.translation:null,
        // F03: the node's own transform (column-major 4 x 4, metres) and the box the part occupies once it is applied; `min` and `max` stay the mesh's own, which is what its geometry's box is.
        m,wmin,wmax});
    }
  });
  return out;
}

export function sizeOf(p){return Math.hypot(p.max[0]-p.min[0],p.max[1]-p.min[1],p.max[2]-p.min[2]);}
export function triangles(parts){return parts.reduce((n,p)=>n+p.tris,0);}

/** part key -> motion group name, from the registered profile (`groups: {name: [keys]}`). Parts the
 *  registration does not name stay with the frame. */
export function groupIndex(profile){
  const m=new Map();
  for(const [name,keys] of Object.entries(profile?.groups||{}))for(const k of keys)m.set(String(k),name);
  return m;
}

/** Where a placement in the machine's CAD frame (millimetres) lands in the model file's frame (metres), through the machine's
 *  basis rows: the rig's own `point()` for every option file (polish WO-01). */
export function offsetOf(basis,t){const B=basis||[[1,0,0],[0,0,1],[0,-1,0]];return [0,1,2].map(r=>(B[r][0]*t[0]+B[r][1]*t[1]+B[r][2]*t[2])/1000);}

/** The parts worth drawing in the simple view: everything that is not a fastener-sized detail (the file is in
 *  metres, so 0.012 is 12 mm). */
export function simpleSet(parts,smallerThan=0.012){
  const keep=new Set();parts.forEach((p,i)=>{if(sizeOf(p)>=smallerThan)keep.add(i);});return keep;
}

/** Bounding box of all parts, optionally through a 3x3 basis (rows): [[x,y,z],[x,y,z]]. */
export function boundsAfter(parts,basis){
  const B=basis||[[1,0,0],[0,1,0],[0,0,1]],lo=[Infinity,Infinity,Infinity],hi=[-Infinity,-Infinity,-Infinity];
  for(const p of parts)for(const x of [(p.wmin||p.min)[0],(p.wmax||p.max)[0]])for(const y of [(p.wmin||p.min)[1],(p.wmax||p.max)[1]])for(const z of [(p.wmin||p.min)[2],(p.wmax||p.max)[2]])
    for(let r=0;r<3;r++){const v=B[r][0]*x+B[r][1]*y+B[r][2]*z;if(v<lo[r])lo[r]=v;if(v>hi[r])hi[r]=v;}
  return [lo,hi];
}
