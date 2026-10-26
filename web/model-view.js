// The 3D view of a printer's real CAD (farm board WO-10). Loaded only when someone opens a model, never
// by the farm wall. It draws one mesh per part, moves the registered motion groups from the pose the
// service computes, and redraws only when something changed (a pose, the camera, the size).
import * as THREE from './vendor/three.module.js';
import {OrbitControls} from './vendor/OrbitControls.js';
import {parseGlb,readParts,groupIndex,simpleSet,boundsAfter,triangles,offsetOf,sizeOf} from './model-glb.js';

const VIEWS={iso:[0.9,0.7,1.15],front:[0,0.15,1.6],top:[0,1.8,0.001],right:[1.6,0.2,0]};

/** Unpack a gzip file with the browser's own decompressor, or take a plain GLB as it is. */
export async function modelBuffer(url,{onProgress}={}){
  const r=await fetch(url,{cache:'no-store'});
  if(!r.ok)throw Error((await r.json().catch(()=>({}))).error||'The model could not be read.');
  const total=Number(r.headers.get('content-length'))||0;
  let seen=0;
  const counted=new TransformStream({transform(chunk,c){seen+=chunk.length;if(onProgress&&total)onProgress(seen/total);c.enqueue(chunk);}});
  const raw=await new Response(r.body.pipeThrough(counted)).arrayBuffer();
  const head=new Uint8Array(raw,0,4);
  if(head[0]===0x1f&&head[1]===0x8b){
    if(typeof DecompressionStream==='undefined')throw Error('This browser cannot unpack the model. Use a current Chrome, Edge, Firefox or Safari.');
    return new Response(new Blob([raw]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
  }
  return raw;
}

// F04 (2026-10-19): `tile` is a picture on the farm wall, built so the wall keeps its frame rate. Almost every part of these files comes with no normals, and working
// them out was 2.2 s of the page's thread over seven tiles (measured: computeVertexNormals 17% of the first 13 s), so a tile shades flat (the faces of CAD parts are flat; at
// tile size nobody sees the facets) and never computes them; and its parts appear a few hundred thousand triangles at a time, a frame between, instead of one 150 to 270 ms
// upload. A tile is a picture, not a model to inspect: the printer page and the model sheets draw as before (smooth, all at once).
export async function createModelView(host,{buffer,basis,profile,simple=false,onInfo,tile=false,onPick=null}){
  // `buffer` is the machine's file; with none, the view holds only the files of a scene (an option looked at on its own).
  const glb=buffer?parseGlb(buffer):null,parts=glb?readParts(glb):[],groups=groupIndex(profile);
  const tris=triangles(parts);
  const renderer=new THREE.WebGLRenderer({antialias:true,alpha:true,powerPreference:'default'});
  renderer.setPixelRatio(Math.min(devicePixelRatio||1,2));
  host.append(renderer.domElement);
  renderer.domElement.style.cssText='width:100%;height:100%;display:block;touch-action:none';
  const scene=new THREE.Scene();
  const camera=new THREE.PerspectiveCamera(32,1,10,60000);
  scene.add(new THREE.HemisphereLight(0xdfe8ff,0x1a1d22,1.35));
  const sun=new THREE.DirectionalLight(0xffffff,1.9);sun.position.set(700,1400,900);scene.add(sun);
  const fill=new THREE.DirectionalLight(0x9fb8ff,0.7);fill.position.set(-900,500,-700);scene.add(fill);

  // The model file is already in the display frame, in metres, shifted to the machine's origin. The
  // motion registration is in the CAD frame, in millimetres: `basis` carries its offsets across.
  const root=new THREE.Group();
  const B=basis||[[1,0,0],[0,0,1],[0,-1,0]];
  scene.add(root);
  const motion=new Map();   // group name -> THREE.Group
  const holder=n=>{if(!motion.has(n)){const g=new THREE.Group();g.name=n;root.add(g);motion.set(n,g);}return motion.get(n);};
  const materials=new Map(),meshes=[];
  const material=(m,flat=false)=>{
    const k=m.color.join(',')+'|'+m.alpha+'|'+m.metal+'|'+m.rough+'|'+m.double+'|'+flat;
    if(!materials.has(k)){
      const mat=new THREE.MeshStandardMaterial({color:new THREE.Color(m.color[0],m.color[1],m.color[2]),flatShading:flat,
        metalness:m.metal,roughness:m.rough,side:m.double?THREE.DoubleSide:THREE.FrontSide,transparent:m.alpha<0.999,opacity:m.alpha});
      materials.set(k,mat);
    }
    return materials.get(k);
  };
  const geometry=p=>{
    const g=new THREE.BufferGeometry();
    g.setAttribute('position',new THREE.BufferAttribute(p.pos,3));
    if(p.nor)g.setAttribute('normal',new THREE.BufferAttribute(p.nor,3));
    if(p.idx)g.setIndex(new THREE.BufferAttribute(p.idx,1));
    if(!p.nor&&!tile)g.computeVertexNormals();
    g.boundingBox=new THREE.Box3(new THREE.Vector3(...p.min),new THREE.Vector3(...p.max));
    g.boundingSphere=g.boundingBox.getBoundingSphere(new THREE.Sphere());
    return g;
  };
  // F03 (2026-10-19): a model whose file is not in the display's frame carries `orient` in its registered profile (three rows, display = orient * file: the Annex
  // K1, K2 and K3 are written with their up along -Y or -Z and are turned upright by the rig); a part placed by a matrix of its own (model-glb.js) is drawn through it.
  // Both apply to the machine's own parts; an option's files (scene placements) are placed by the catalogue in the display's frame and keep to that.
  const O=Array.isArray(profile?.orient)&&profile.orient.length===3?profile.orient:null,m4=new THREE.Matrix4();
  const O4=O?new THREE.Matrix4().set(O[0][0],O[0][1],O[0][2],0,O[1][0],O[1][1],O[1][2],0,O[2][0],O[2][1],O[2][2],0,0,0,0,1):null;
  const addMesh=(p,parent,g=geometry(p),orient=true)=>{const mesh=new THREE.Mesh(g,material(p.material,tile&&!p.nor));
    if(p.m||(orient&&O4)){if(p.m)m4.fromArray(p.m);else m4.makeTranslation(p.t?p.t[0]:0,p.t?p.t[1]:0,p.t?p.t[2]:0);if(orient&&O4)m4.premultiply(O4);mesh.matrix.copy(m4);mesh.matrixAutoUpdate=false;mesh.matrixWorldNeedsUpdate=true;}
    else{if(p.t)mesh.position.set(p.t[0],p.t[1],p.t[2]);mesh.matrixAutoUpdate=false;mesh.updateMatrix();}
    parent.add(mesh);return mesh;};
  // A model whose side panels and doors are solid in the file (the Annex machines' are: a closed grey box) names them in its profile (`see_through`: the words that make a
  // part a panel, and the words that make it something else, such as a hinge): they are drawn nearly clear, as the rig's own viewer offers.
  const seeThrough=profile?.see_through?.match?{m:new RegExp(profile.see_through.match,'i'),x:profile.see_through.except?new RegExp(profile.see_through.except,'i'):null}:null;
  const clear=p=>seeThrough&&seeThrough.m.test(p.name)&&!(seeThrough.x&&seeThrough.x.test(p.name))?{...p,material:{...p.material,alpha:Math.min(p.material.alpha,0.12)}}:p;
  for(const p of parts){const m=addMesh(clear(p),holder(groups.get(p.key)||p.group||'fixed'));m.userData.key=p.key;m.userData.name=p.name;m.userData.group=p.group;meshes.push(m);}
  // 2026-10-24/25: a click on a part of the machine says what it is (finish E17). A press that moves is turning the view, not a click.
  if(onPick&&!tile){const ray=new THREE.Raycaster(),at=new THREE.Vector2();let down=null;
   renderer.domElement.addEventListener('pointerdown',e=>{down=e.button===0?[e.clientX,e.clientY]:null;});
   renderer.domElement.addEventListener('pointerup',e=>{if(!down||Math.hypot(e.clientX-down[0],e.clientY-down[1])>5){down=null;return;}down=null;
    const r=renderer.domElement.getBoundingClientRect();at.set(((e.clientX-r.left)/r.width)*2-1,-((e.clientY-r.top)/r.height)*2+1);ray.setFromCamera(at,camera);
    const hit=ray.intersectObjects(meshes.filter(m=>m.visible),false)[0];if(hit)onPick({name:hit.object.userData.name||'',group:hit.object.userData.group||'',x:e.clientX,y:e.clientY});});}
  for(const g of motion.values()){g.updateMatrixWorld(true);}

  // Frame it.
  const centre=new THREE.Vector3();let radius=0.2;
  const frame=(lo,hi)=>{centre.set((lo[0]+hi[0])/2,(lo[1]+hi[1])/2,(lo[2]+hi[2])/2);radius=Math.max(0.001,Math.hypot(hi[0]-lo[0],hi[1]-lo[1],hi[2]-lo[2])/2);};
  if(parts.length)frame(...boundsAfter(parts,O));
  // Polish WO-06 (2026-10-19): the machine is fitted to its stage, not just aimed at the middle of its box. The old distance allowed only for a narrow stage, so on a wide
  // tile the base of a tall machine met the bottom edge (measured on the CORE One L: 0 px under it, 19 to 28 px of empty space above). The corners of the parts big enough to
  // shape the outline (a node's own translation included) are projected and the camera is moved until they sit in the middle and inside the stage.
  const corners=(lo,hi,t=[0,0,0])=>{const o=[];for(const x of [lo[0],hi[0]])for(const y of [lo[1],hi[1]])for(const z of [lo[2],hi[2]])o.push([x+t[0],y+t[1],z+t[2]]);return o;};
  const cornersOf=p=>{const lo=p.wmin||p.min,hi=p.wmax||p.max,t=p.m?[0,0,0]:(p.t||[0,0,0]),o=[];for(const x of [lo[0],hi[0]])for(const y of [lo[1],hi[1]])for(const z of [lo[2],hi[2]]){const v=[x+t[0],y+t[1],z+t[2]];o.push(O?[0,1,2].map(r=>O[r][0]*v[0]+O[r][1]*v[1]+O[r][2]*v[2]):v);}return o;};
  let outline=[];for(const p of parts)if(sizeOf(p)>=radius*0.04)outline.push(...cornersOf(p));
  const controls=new OrbitControls(camera,renderer.domElement);
  controls.enableDamping=false;controls.screenSpacePanning=true;
  let raf=0,disposed=false,dirty=false,held=tile?()=>{}:null; // polish WO-03: a held view is drawn by whoever holds it (the wall's frame scheduler); a tile draws nothing while it is built (F04)
  // WO-13: frame-time meter, so the farm wall can prove the shared cadence stays under a frame.
  const perf={n:0,last:0,ms:[]};
  const draw=()=>{raf=0;dirty=false;if(disposed||document.hidden)return;const t0=performance.now();renderer.render(scene,camera);const d=performance.now()-t0;perf.n++;perf.last=d;perf.ms.push(d);if(perf.ms.length>120)perf.ms.shift();};
  const want=()=>{dirty=true;if(held){held();return;}if(!raf&&!disposed)raf=requestAnimationFrame(draw);};
  controls.addEventListener('change',want);
  let lastView='iso',touched=false;
  controls.addEventListener('start',()=>{touched=true;});
  // A narrow stage (a phone held upright) needs the camera further back to keep the whole machine in.
  const fit=()=>{if(!outline.length)return;const v=new THREE.Vector3(),right=new THREE.Vector3(),up=new THREE.Vector3(),dir=new THREE.Vector3(),FILL=0.88;
    for(let n=0;n<5;n++){camera.updateMatrixWorld(true);let x0=1e9,x1=-1e9,y0=1e9,y1=-1e9;
      for(const p of outline){v.set(p[0],p[1],p[2]).project(camera);if(v.x<x0)x0=v.x;if(v.x>x1)x1=v.x;if(v.y<y0)y0=v.y;if(v.y>y1)y1=v.y;}
      const dist=camera.position.distanceTo(controls.target),hh=dist*Math.tan(camera.fov*Math.PI/360);
      right.setFromMatrixColumn(camera.matrixWorld,0).multiplyScalar(((x0+x1)/2)*hh*camera.aspect);up.setFromMatrixColumn(camera.matrixWorld,1).multiplyScalar(((y0+y1)/2)*hh);right.add(up);
      camera.position.add(right);controls.target.add(right);dir.subVectors(camera.position,controls.target).normalize();
      camera.position.copy(controls.target).addScaledVector(dir,Math.min(radius*8,Math.max(radius*1.1,dist*Math.max((x1-x0)/2,(y1-y0)/2)/FILL)));}
    camera.updateMatrixWorld(true);};
  const put=(name)=>{lastView=name;touched=false;const v=VIEWS[name]||VIEWS.iso,d=radius*3.1*Math.max(1,1/camera.aspect);
    camera.position.set(centre.x+v[0]*d/1.6,centre.y+v[1]*d/1.6,centre.z+v[2]*d/1.6);controls.target.copy(centre);camera.near=radius/50;camera.far=radius*40;camera.updateProjectionMatrix();controls.update();fit();controls.update();want();};
  const size=()=>{const w=Math.max(120,host.clientWidth),h=Math.max(120,host.clientHeight);renderer.setSize(w,h,false);camera.aspect=w/h;camera.updateProjectionMatrix();if(!touched)put(lastView);else want();};
  const ro=new ResizeObserver(size);ro.observe(host);
  const vis=()=>{if(!document.hidden)want();};document.addEventListener('visibilitychange',vis);
  size();put('iso');

  // Simple view: parts smaller than a fastener are hidden. Big models stay light on a phone.
  const keep=simpleSet(parts,0.012);
  let isSimple=false;
  // A part an option replaces (the machine's own head under a tool bank) stays hidden whatever the view.
  const setSimple=on=>{isSimple=on;meshes.forEach((m,i)=>{m.visible=!m.userData.replaced&&(!on||keep.has(i));});want();};
  setSimple(simple);
  // F04: a tile's parts appear in slices (see the note on `tile` above). The picture is not on screen until the build is done (the wall keeps the ring or the last still over it).
  if(tile&&meshes.length){
    const on=meshes.map(m=>m.visible);meshes.forEach(m=>{m.visible=false;});
    const breath=()=>new Promise(r=>{const t=setTimeout(r,80);requestAnimationFrame(()=>{clearTimeout(t);r();});});   // (a hidden tab gets no frames: the timer lets the build go on)
    let n=0;
    for(let i=0;i<meshes.length;i++){
      if(!on[i])continue;
      meshes[i].visible=true;n+=parts[i].tris;
      if(n>=150000){if(disposed)break;renderer.render(scene,camera);n=0;await breath();}}
    if(!disposed)renderer.render(scene,camera);
    held=null;
  }
  const shown=()=>meshes.filter(m=>m.visible).length;
  // Options (polish WO-01, 2026-10-18): the files of a toolchanger, toolhead or add-on, each at the place the rig
  // registers for it. Placements are in the CAD frame, millimetres; `follows` names a motion group (the tool on the
  // gantry moves with the head). A file is read once however many docks use it.
  let items=[];const extraCache=new Map();
  const clearExtra=()=>{for(const g of items)g.parent?.remove(g);items=[];};
  const setScene=async(sc,{onProgress}={})=>{
    clearExtra();
    const hide=new Set((sc?.hide_machine_keys||[]).map(String));
    meshes.forEach(m=>{m.userData.replaced=hide.has(String(m.userData.key));});setSimple(isSimple);
    const placements=(sc?.placements||[]).filter(p=>p.installed);let drawn=0,done=0;
    for(const p of placements){
      if(!extraCache.has(p.part))extraCache.set(p.part,modelBuffer(p.url).then(b=>readParts(parseGlb(b)).map(r=>({r,g:geometry(r)}))));
      const set=await extraCache.get(p.part);
      const g=new THREE.Group(),t=p.translation_mm,hid=new Set((p.hidden_keys||[]).map(String));
      g.position.set(...offsetOf(B,t));
      g.name=p.part;g.userData={part:p.part,role:p.role,slot:p.slot};
      // F03 (2026-10-19): a placed file whose parts the registered profile names (the Trident's gantry module: its X beam moves with Y, its carriage with X and Y) puts
      // each of those in its motion group at the file's place; a file that follows a group as a whole (a tool) and every part the profile does not name stay together.
      const into=new Map();
      for(const {r,g:geo} of set){
        if(hid.has(String(r.key)))continue;
        const name=p.follows?null:groups.get(r.key);
        if(!name){addMesh(r,g,geo,false);}
        else{let h=into.get(name);if(!h){h=new THREE.Group();h.position.copy(g.position);h.name=p.part+':'+name;h.userData={part:p.part,role:p.role,slot:p.slot};holder(name).add(h);into.set(name,h);items.push(h);}addMesh(r,h,geo,false);}
        drawn++;}
      (p.follows?holder(p.follows):root).add(g);items.push(g);
      if(onProgress)onProgress(++done/placements.length);
    }
    for(const m of motion.values())m.updateMatrixWorld(true);root.updateMatrixWorld(true);
    if(!parts.length&&items.length){const b=new THREE.Box3().setFromObject(root);frame([b.min.x,b.min.y,b.min.z],[b.max.x,b.max.y,b.max.z]);outline=corners([b.min.x,b.min.y,b.min.z],[b.max.x,b.max.y,b.max.z]);put(lastView);}
    want();
    return {placed:placements.length,parts:drawn,replaced:meshes.filter(m=>m.userData.replaced).length};
  };

  const info={parts:parts.length,triangles:tris,groups:[...motion.keys()],simple_parts:keep.size};
  if(onInfo)onInfo(info);

  return {
    info,
    /** What the print layer and the followers need (polish WO-03): the root, a motion group by name, the basis, a redraw request. */
    ext:{root,holder,B,want,
      /** Take over drawing: `want` only marks the view dirty and calls `cb`; the holder calls `drawNow`. `hold(null)` gives it back. */
      hold(cb){held=cb||null;if(!cb&&dirty)want();},
      /** Draw now if something changed; returns the milliseconds it took (0 when nothing was drawn). */
      drawNow(){if(!dirty||disposed||document.hidden)return 0;if(raf){cancelAnimationFrame(raf);raf=0;}const t=performance.now();draw();return performance.now()-t;},
      isDirty:()=>dirty},
    /** `groups` is {name:[dx,dy,dz]} in the CAD frame, millimetres from the CAD pose. */
    setGroups(g){
      for(const [name,v] of Object.entries(g||{})){
        const n=motion.get(name);if(!n)continue;
        const a=v[0]||0,b=v[1]||0,c=v[2]||0;
        n.position.set((B[0][0]*a+B[0][1]*b+B[0][2]*c)/1000,(B[1][0]*a+B[1][1]*b+B[1][2]*c)/1000,(B[2][0]*a+B[2][1]*b+B[2][2]*c)/1000);}
      for(const m of motion.values())m.updateMatrixWorld(true);want();},
    /** 2026-10-25 (live numbers on the machine): where a motion group stands on the screen, in page pixels. `where` is 'bottom' (the nozzle's tip of the
     *  head), 'top' (the bed's surface) or the middle. Null when the group is not in this model or is behind the camera. */
    anchor(name,where='center'){const g=motion.get(name);if(!g||disposed)return null;const box=new THREE.Box3().setFromObject(g);if(box.isEmpty())return null;
      const p=box.getCenter(new THREE.Vector3());if(where==='bottom')p.y=box.min.y;else if(where==='top')p.y=box.max.y;p.project(camera);if(p.z>1||p.z<-1)return null;
      const r=renderer.domElement.getBoundingClientRect();return {x:r.left+(p.x+1)/2*r.width,y:r.top+(1-p.y)/2*r.height};},
    view:put,setSimple,shown,perf,setScene,sceneItems:()=>items.map(g=>({part:g.userData.part,role:g.userData.role,slot:g.userData.slot,follows:g.parent!==root})),
    /** Draw the picture as it is now into a 2D canvas (farm wall, polish WO-04: a tile that goes off screen keeps this as its still; the page's CSP allows no data: or blob: images, so the still is a canvas). Drawn and copied in one task: a WebGL canvas is only readable in the task that drew it. */
    snapshot(into,maxW=640){if(disposed||!renderer.domElement.width)return false;renderer.render(scene,camera);const src=renderer.domElement,w=Math.min(maxW,src.width),h=Math.max(1,Math.round(src.height*w/src.width));if(into.width!==w)into.width=w;if(into.height!==h)into.height=h;const g=into.getContext('2d');g.clearRect(0,0,w,h);g.drawImage(src,0,0,w,h);return true;},
    /** Median frame time of the recent draws, in ms (null before any draw). */
    frameMedian(){if(!perf.ms.length)return null;const s=[...perf.ms].sort((a,b)=>a-b);return s[s.length>>1];},
    dispose(){disposed=true;if(raf)cancelAnimationFrame(raf);ro.disconnect();document.removeEventListener('visibilitychange',vis);controls.dispose();
      for(const m of meshes)m.geometry.dispose();clearExtra();for(const p of extraCache.values())p.then(s=>s.forEach(x=>x.g.dispose())).catch(()=>{});for(const m of materials.values())m.dispose();renderer.dispose();
      // Polish WO-04: let the GL context go now, not whenever the garbage collector gets to it (Chrome keeps about 16 and kills the oldest beyond that).
      try{renderer.forceContextLoss();}catch{}renderer.domElement.remove();},
  };
}
