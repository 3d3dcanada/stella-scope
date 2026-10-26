import {layout} from './workspace-scene.js';
import {slotById,SLOTS} from './service-core.js';
export const CAMERA_VIEWS={iso:[-.62,.46],front:[0,.02],back:[Math.PI,.02],right:[Math.PI/2,.02],left:[-Math.PI/2,.02],top:[0,1.56],bottom:[0,-1.56]};
const col={board:'#1c2026',chip:'#2a3138',metal:'#ccd5d9',steel:'#aab4b9',gold:'#e6b05f',red:'#c7332c',dark:'#2a3138',wire:'#454e57',selected:'#04D9C4',warn:'#FF6A00',edge:[0.08,0.55,0.5]};
const box=(id,pos,size,color,layer='electronics',parent=id)=>({id,parent,pos,size,color,kind:'box',layer});
const cyl=(id,pos,r,h,color,layer='electronics',parent=id)=>({id,parent,pos,r,h,color,kind:'cylinder',layer});
export function serviceLayout(snapshot,explode=0){
 const base=layout(snapshot),w=base.pose.bed[0],d=base.pose.bed[1],a=[];
 for(const p of base.parts){let layer=p.id==='frame'?'frame':p.id==='heater_bed'?'bed':p.id.startsWith('stepper_z')?'z-drives':['gantry','x_rail','y_rails','stepper_x','stepper_y'].includes(p.id)?'gantry':p.id==='controller_fan'?'electronics':'toolhead';let id=p.id;
  if(p.id==='frame'&&p.size?.[2]===4) {id='deck';layer='deck';}
  if(p.id==='controller_fan')continue;
  const q={...p,id,parent:p.id,layer,pos:[...p.pos]};if(layer==='toolhead')q.pos[2]+=explode*.4;if(layer==='bed')q.pos[2]+=explode*.15;if(layer==='covers')q.pos[2]+=explode*.65;a.push(q);
 }
 // Electronics are recognizable location envelopes. No board pad/connector placement is claimed measured.
 const z=-29-explode*.7;
 for(const y of [d*.27,d*.73])a.push(box('bay.rails',[w/2,y,z-10],[w-25,14,7],col.metal));
 const cx=w*.29,cy=d*.58;
 // BTT Octopus envelope: green board, MCU chip, 2x4 driver modules with gold heatsink fins, edge connectors.
 a.push(box('controller',[cx,cy,z],[133,96,3],col.board));
 a.push(box('controller',[cx,cy,z+1.5],[133,96,1],col.chip));
 a.push(box('controller',[cx-24,cy-24,z+4],[16,16,6],col.chip));
 for(let r=0;r<2;r++)for(let c=0;c<4;c++){const dx=cx-39+c*26,dy=cy-14+r*28;
  a.push(box('controller',[dx,dy,z+6],[20,22,9],col.dark));
  a.push(box('controller',[dx,dy,z+12],[16,18,3],col.gold));}
 for(const edge of [-1,1])for(let i=0;i<6;i++)a.push(box('controller',[cx-45+i*18,cy+edge*44,z+5],[11,8,6],i<2?col.gold:col.dark));
 // Wire runs between bays — illustrative routing, not measured harness paths.
 a.push(box('distribution',[cx+67,cy+2,z+5],[26,3,3],col.wire));
 a.push(box('distribution',[w*.6,cy+2,z+5],[w*.26,3,3],col.wire));
 a.push(box('inlet',[w*.9,d*.5,z+5],[3,d*.28,3],col.wire));
 const hx=w*.76,hy=d*.73;
 // Pi host envelope: board, case, MCU, GPIO stripe, USB LAN block.
 a.push(box('host',[hx,hy,z+3],[86,57,3],col.board));
 a.push(box('host',[hx,hy,z+8],[78,50,9],col.dark));
 a.push(box('host',[hx-24,hy-8,z+14],[14,14,4],col.chip));
 a.push(box('host',[hx-38,hy,z+13],[10,44,3],col.gold));
 for(let i=0;i<3;i++)a.push(box('host',[hx+30,hy-17+i*17,z+14],[18,14,8],col.metal));
 const px=w*.73,py=d*.29;
 a.push(box('psu',[px,py,z-9],[101,154,24],col.metal));
 a.push(box('psu',[px,py,z+4],[101,26,1],col.dark));
 for(const y2 of [-55,55])a.push(cyl('psu',[px,py+y2,z+4],5,3,col.gold));
 a.push(box('ssr',[w*.24,d*.19,z+4],[46,59,24],col.metal));a.push(box('ssr',[w*.24,d*.19,z+17],[36,29,2],col.dark));
 a.push(box('distribution',[w*.47,d*.15,z+6],[32,62,17],col.dark));for(let i=0;i<6;i++)a.push(cyl('distribution',[w*.47,d*.15-23+i*9,z+16],2.5,2,col.gold));
 a.push(box('inlet',[w+43,d*.65,-18],[15,44,34],col.dark));
  a.push(box('controller_fan',[w*.38,-8,z+3],[54,54,10],col.dark));
  a.push(cyl('controller_fan',[w*.38,-8,z+10],21,3,col.metal));a.push(cyl('controller_fan',[w*.38,-8,z+12],7,3,col.dark));
  for(let i=0;i<6;i++){const t=i*Math.PI/3;a.push(box('controller_fan',[w*.38+12*Math.cos(t),-8+12*Math.sin(t),z+12],[9,5,2],col.metal));}
  // Skinning panels (2026-09-18 build-out): the machine inspects open by default;
  // the Covers toggle in the viewer reveals the skinned, panelled printer.
  const pz=base.top-30;
  for(const sx of [-66,w+66])a.push({...box('panels',[sx,d/2,pz/2+12],[7,d+92,pz],col.dark),layer:'covers'});
  a.push({...box('panels',[w/2,d+66,pz/2+12],[w+92,7,pz],col.dark),layer:'covers'});
  a.push({...box('panels',[w/2,-66,pz/2+12],[w+92,7,pz],col.dark),layer:'covers'});
  a.push({...box('panels',[w/2,d/2,base.top+1],[w+92,d+92,5],col.dark),layer:'covers'});
 // BLTouch body and pin remain stationary relative to the head; deployment is explicitly unobserved.
 const p=base.pose,bz=base.bedZ,tx=p.x+p.probe[0],ty=p.y+p.probe[1],tz=bz+p.z+20+explode*.4;
 a.splice(0,a.length,...a.filter(x=>x.id!=='probe'));
  for(const q of [box('probe',[tx,ty,tz],[13,15,26],'#202b33','toolhead'),box('probe',[tx,ty,tz+2],[13.5,15.5,4],col.board,'toolhead'),box('probe',[tx,ty-8,tz],[15,4,20],col.steel,'toolhead'),cyl('probe',[tx,ty,tz-15],2.5,8,col.red,'toolhead')]){q.dyn=true;a.push(q);}
  // Recorded BLTouch landings on the bed (board item 3): the newest points glow mint. Bed-fixed
  // positions, so they ride the static cache; the cache key carries their count and newest ts.
  for(const [i,pp] of (snapshot?.point?.probe_points||[]).entries()){const fresh=i>=Math.max(0,(snapshot?.point?.probe_points||[]).length-4);a.push(box('probe',[pp.x,pp.y,bz+9],[4.5,4.5,20],fresh?col.selected:col.steel,'toolhead'));}
  return {...base,parts:a,explode};
}
const faces=[[0,1,2,3],[4,7,6,5],[0,4,5,1],[1,5,6,2],[2,6,7,3],[3,7,4,0]];
const vecsub=(a,b)=>a.map((x,i)=>x-b[i]);const cross=(a,b)=>[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];const norm=a=>{const m=Math.hypot(...a)||1;return a.map(x=>x/m);};
export function vertices(p){if(p.kind==='axle'){const k=16,lo=[],hi=[];for(let i=0;i<k;i++){const t=i*2*Math.PI/k;lo.push([p.pos[0]+p.r*Math.cos(t),p.pos[1]-p.len/2,p.pos[2]+p.r*Math.sin(t)]);hi.push([p.pos[0]+p.r*Math.cos(t),p.pos[1]+p.len/2,p.pos[2]+p.r*Math.sin(t)]);}return [...lo.map((_,i)=>[lo[i],lo[(i+1)%k],hi[(i+1)%k],hi[i]]),hi,lo.slice().reverse()];}
 if(p.kind==='box'){const [a,b,c]=p.size.map(x=>x/2);const v=[[-a,-b,-c],[a,-b,-c],[a,b,-c],[-a,b,-c],[-a,-b,c],[a,-b,c],[a,b,c],[-a,b,c]].map(q=>q.map((x,i)=>x+p.pos[i]));return faces.map(ids=>ids.map(i=>v[i]));}const lo=[],hi=[],k=24;for(let i=0;i<k;i++){const t=i*2*Math.PI/k;lo.push([p.pos[0]+p.r*Math.cos(t),p.pos[1]+p.r*Math.sin(t),p.pos[2]-p.h/2]);hi.push([p.pos[0]+p.r*Math.cos(t),p.pos[1]+p.r*Math.sin(t),p.pos[2]+p.h/2]);}return [...lo.map((_,i)=>[lo[i],lo[(i+1)%k],hi[(i+1)%k],hi[i]]),hi,lo.reverse()];}
// Box edges (shrunk 1% toward the part centre) for the technical-illustration outline pass.
export function edges(p){if(p.kind!=='box')return [];const [a,b,c]=p.size.map(x=>x/2*.99),v=[[-a,-b,-c],[a,-b,-c],[a,b,-c],[-a,b,-c],[-a,-b,c],[a,-b,c],[a,b,c],[-a,b,c]].map(q=>q.map((x,i)=>x+p.pos[i]));return [[0,1],[1,2],[2,3],[3,0],[4,5],[5,6],[6,7],[7,4],[0,4],[1,5],[2,6],[3,7]].map(([i,j])=>[v[i],v[j]]);}
export function matrix(w,h,model,yaw,pitch,zoom,pan=[0,0]){const bw=model.pose.bed[0],bd=model.pose.bed[1],extent=Math.max(bw+180,bd+180,model.top+170+model.explode),scale=1.65/extent*zoom,aspect=w/h,ca=Math.cos(yaw),sa=Math.sin(yaw),cp=Math.cos(pitch),sp=Math.sin(pitch),center=[bw/2,bd/2,(model.top-65-model.explode*.7)/2];const r=[ca,-sa,0],u=[sa*sp,ca*sp,cp],depth=[sa*cp,ca*cp,-sp];const sx=scale/aspect,sy=scale,sz=1/(extent*3);return new Float32Array([r[0]*sx,u[0]*sy,depth[0]*sz,0,r[1]*sx,u[1]*sy,depth[1]*sz,0,r[2]*sx,u[2]*sy,depth[2]*sz,0,-r.reduce((a,x,i)=>a+x*center[i],0)*sx+pan[0],-u.reduce((a,x,i)=>a+x*center[i],0)*sy+pan[1],-depth.reduce((a,x,i)=>a+x*center[i],0)*sz,1]);}
export function project(v,m,w,h){return {x:(m[0]*v[0]+m[4]*v[1]+m[8]*v[2]+m[12]+1)*w/2,y:(1-(m[1]*v[0]+m[5]*v[1]+m[9]*v[2]+m[13]))*h/2,z:m[2]*v[0]+m[6]*v[1]+m[10]*v[2]+m[14]};}
 export class ServiceScene{
 constructor(canvas,select){this.canvas=canvas;this.select=select;this.snapshot=null;this.selected='head';this.yaw=-.62;this.pitch=.46;this.zoom=1.3;this.pan=[0,0];this.explode=0;this.hidden=new Set(['covers']);this.pointer=new Map();this.moved=0;this.frame=0;this.drawMs=0;window.__stellaScene=this;
 const gl=canvas.getContext('webgl',{antialias:true,alpha:false,preserveDrawingBuffer:true});if(!gl)throw Error('WebGL is unavailable. Enable browser graphics support or use the classic workspace; no synthetic fallback is labelled as the new 3D view.');this.gl=gl;
  const vs='attribute vec3 p;attribute vec3 normal;attribute vec3 color;attribute vec3 pick;uniform mat4 m;uniform bool picking;varying vec3 c;void main(){gl_Position=m*vec4(p,1.);float light=.42+.48*abs(dot(normal,normalize(vec3(.42,-.58,.7))))+.18*abs(normal.z);c=picking?pick:color*light+vec3(.005,.024,.022);}';
 const fs='precision mediump float;varying vec3 c;void main(){gl_FragColor=vec4(c,1.);}';
 const shader=(type,src)=>{const s=gl.createShader(type);gl.shaderSource(s,src);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
 const prog=gl.createProgram();gl.attachShader(prog,shader(gl.VERTEX_SHADER,vs));gl.attachShader(prog,shader(gl.FRAGMENT_SHADER,fs));gl.linkProgram(prog);if(!gl.getProgramParameter(prog,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(prog));this.program=prog;this.buffer=gl.createBuffer();this.mu=gl.getUniformLocation(prog,'m');this.pu=gl.getUniformLocation(prog,'picking');
  for(const name of ['p','normal','color','pick']){const a=gl.getAttribLocation(prog,name);gl.enableVertexAttribArray(a);this.attribs=(this.attribs||[]).concat(a);}
  gl.enable(gl.DEPTH_TEST);gl.disable(gl.DITHER);
  this.edgeBuffer=gl.createBuffer();this.dynBuffer=gl.createBuffer();this.dynEdgeBuffer=gl.createBuffer();
 this.tip=document.getElementById('scene-tip');this.hoverAt=0;this.hoverId=null;
 canvas.addEventListener('contextmenu',e=>e.preventDefault());
 canvas.addEventListener('pointerdown',e=>{canvas.setPointerCapture(e.pointerId);this.pointer.set(e.pointerId,{x:e.clientX,y:e.clientY});this.moved=0;if(this.tip)this.tip.hidden=true;});
 canvas.addEventListener('pointermove',e=>{const prev=this.pointer.get(e.pointerId);if(!prev){
  // Hover pick, throttled: shows the part name tooltip; never blocks orbiting or the page.
  if(this.tip){const now=performance.now();if(now-this.hoverAt>90){this.hoverAt=now;const rect=canvas.getBoundingClientRect();const id=this.pick(e.clientX-rect.left,e.clientY-rect.top);this.hoverId=id;
   if(id){const s=slotById(id)||SLOTS.find(x=>x.id.split('.').pop()===id);this.tip.textContent=s?`${s.name} — ${s.location}`:id.charAt(0).toUpperCase()+id.slice(1).replace(/_/g,' ');this.tip.style.left=Math.min(canvas.clientWidth-180,e.clientX-rect.left+14)+'px';this.tip.style.top=Math.max(6,e.clientY-rect.top-32)+'px';this.tip.hidden=false;canvas.style.cursor='pointer';}else{this.tip.hidden=true;canvas.style.cursor='grab';}}}
  return;}
 const before=[...this.pointer.values()],next={x:e.clientX,y:e.clientY},dx=next.x-prev.x,dy=next.y-prev.y;this.pointer.set(e.pointerId,next);this.moved+=Math.hypot(dx,dy);
  if(this.pointer.size===2){const after=[...this.pointer.values()],a=Math.hypot(before[0].x-before[1].x,before[0].y-before[1].y),b=Math.hypot(after[0].x-after[1].x,after[0].y-after[1].y);if(a>5)this.zoom=Math.min(5,Math.max(.3,this.zoom*b/a));this.pan[0]+=dx/this.canvas.clientWidth;this.pan[1]-=dy/this.canvas.clientHeight;}
  else if(e.shiftKey||e.buttons===2||e.buttons===4){this.pan[0]+=2*dx/this.canvas.clientWidth;this.pan[1]-=2*dy/this.canvas.clientHeight;}
  else{this.yaw+=dx*.007;this.pitch=Math.max(-1.565,Math.min(1.565,this.pitch+dy*.006));}this.schedule();});
 canvas.addEventListener('pointerup',e=>{if(this.pointer.size===1&&this.moved<5){const rect=canvas.getBoundingClientRect(),id=this.pick(e.clientX-rect.left,e.clientY-rect.top);if(id)this.select(id);}this.pointer.delete(e.pointerId);});
 canvas.addEventListener('pointercancel',e=>this.pointer.delete(e.pointerId));
 canvas.addEventListener('pointerleave',()=>{if(this.tip)this.tip.hidden=true;});
 // Plain wheel scrolls the page; only a held modifier zooms the model (trackpad pinch arrives as ctrl+wheel).
 canvas.addEventListener('wheel',e=>{if(!e.ctrlKey&&!e.metaKey&&!e.shiftKey)return;e.preventDefault();this.zoom=Math.min(5,Math.max(.3,this.zoom*Math.exp(-e.deltaY*.001)));this.schedule();},{passive:false});
 canvas.addEventListener('dblclick',()=>this.focus(this.selected));
 canvas.addEventListener('keydown',e=>{const d=.08;if(e.key.startsWith('Arrow')){e.preventDefault();if(e.shiftKey){this.pan[0]+=e.key==='ArrowLeft'?-d:e.key==='ArrowRight'?d:0;this.pan[1]+=e.key==='ArrowUp'?d:e.key==='ArrowDown'?-d:0;}else{this.yaw+=e.key==='ArrowLeft'?-d:e.key==='ArrowRight'?d:0;this.pitch=Math.max(-1.565,Math.min(1.565,this.pitch+(e.key==='ArrowUp'?d:e.key==='ArrowDown'?-d:0)));}}else if(e.key==='+'||e.key==='=')this.zoom=Math.min(5,this.zoom*1.1);else if(e.key==='-')this.zoom=Math.max(.3,this.zoom/1.1);else if(e.key==='0')this.view('iso');else return;this.schedule();});
 new ResizeObserver(()=>this.schedule()).observe(canvas);
 }
 view(v){[this.yaw,this.pitch]=CAMERA_VIEWS[v]||CAMERA_VIEWS.iso;this.pan=[0,0];this.zoom=1;this.schedule();}
 // Chromium regression exposed one frame of new readings paired with old geometry.
 // Commit snapshot changes synchronously; only camera gestures are frame-coalesced.
 set(snapshot,selected){this.snapshot=snapshot;this.selected=selected;if(this.frame){cancelAnimationFrame(this.frame);this.frame=0;}this.draw();}
 schedule(){if(!this.frame)this.frame=requestAnimationFrame(()=>{this.frame=0;this.draw();});}
 focus(id){const model=serviceLayout(this.snapshot,this.explode),slot=slotById(id),parent=slot?.parent||id,parts=model.parts.filter(p=>p.id===id||p.parent===parent||p.id===parent);if(!parts.length)return;this.pan=[0,0];this.zoom=2;const c=[0,1,2].map(i=>parts.reduce((s,p)=>s+p.pos[i],0)/parts.length);const m=matrix(this.canvas.clientWidth,this.canvas.clientHeight,model,this.yaw,this.pitch,this.zoom,this.pan),p=project(c,m,this.canvas.clientWidth,this.canvas.clientHeight);this.pan=[1-2*p.x/this.canvas.clientWidth,2*p.y/this.canvas.clientHeight-1];this.schedule();}
 geometry(){const model=serviceLayout(this.snapshot,this.explode),slot=slotById(this.selected),target=slot?.parent||this.selected;
  // Static parts (frame, bed, electronics, Z drives) cache; only pose-dependent gantry parts rebuild per frame.
  const flags=Object.entries(this.snapshot?.point?.motors||{}).filter(([,v])=>v.flags?.length).map(([k])=>k).join(',');
  // Geometry rides the key too: switching to a recording with a different bed or Z height must
  // invalidate every cached static part, or the old frame floats under the new data. The newest
  // probe landings ride it as well so fresh BLTouch points appear without a full refetch.
  const pps=this.snapshot?.point?.probe_points||[];
  const key=this.snapshot&&`${this.selected}|${this.explode}|${[...this.hidden].sort().join(',')}|${flags}|${model.pose.bed[0]}x${model.pose.bed[1]}x${model.pose.zmax}|${pps.length}@${pps.at(-1)?.ts??''}`;
  if(this._key!==key){
   const v=[],e=[],ids=[],index=new Map();
   for(const p of model.parts){if(p.dyn)continue;if(this.hidden.has(p.layer))continue;
    let color=p.color;if(p.id===this.selected||p.id===target||p.parent===target)color=col.selected;
    if(this.snapshot?.point?.motors?.[p.parent]?.flags?.length)color=col.warn;
    let idx=index.get(p.id);if(idx===undefined){ids.push(p.id);idx=ids.length-1;index.set(p.id,idx);}idx+=1;
    this._pushPart(p,color,idx,v,e);}
   this._staticV=new Float32Array(v);this._staticE=new Float32Array(e);this._staticIds=ids;this._key=key;
  }
  const v=[],e=[],ids=[...this._staticIds],index=new Map(ids.map((x,i)=>[x,i]));
  for(const p of model.parts){if(!p.dyn)continue;if(this.hidden.has(p.layer))continue;
   let color=p.color;if(p.id===this.selected||p.id===target||p.parent===target)color=col.selected;
   if(this.snapshot?.point?.motors?.[p.parent]?.flags?.length)color=col.warn;
   let idx=index.get(p.id);if(idx===undefined){ids.push(p.id);idx=ids.length-1;index.set(p.id,idx);}idx+=1;
   this._pushPart(p,color,idx,v,e);}
  // Reuse the typed-array capacity across frames: per-frame Float32Array allocation caused GC stalls in live motion.
  if(!this._dv||this._dv.length<v.length)this._dv=new Float32Array(Math.ceil(v.length*1.3/1024)*1024);
  if(!this._de||this._de.length<e.length)this._de=new Float32Array(Math.ceil(e.length*1.3/1024)*1024);
  this._dv.set(v);this._de.set(e);
  return {model,ids,sv:this._staticV,se:this._staticE,dv:this._dv.subarray(0,v.length),de:this._de.subarray(0,e.length),_key:this._key};
 }
 _pushPart(p,color,idx,v,e){const pick=[(idx&255)/255,((idx>>8)&255)/255,((idx>>16)&255)/255],rgb=[1,3,5].map(i=>parseInt(color.slice(i,i+2),16)/255);
  for(const poly of vertices(p)){const normal=norm(cross(vecsub(poly[1],poly[0]),vecsub(poly[2],poly[0])));for(let i=1;i<poly.length-1;i++)for(const pt of [poly[0],poly[i],poly[i+1]])v.push(...pt,...normal,...rgb,...pick);}
  for(const [a2,b2] of edges(p)){e.push(...a2,0,0,1,...col.edge,0,0,0,...b2,0,0,1,...col.edge,0,0,0);}}
  // WebGL captures the attribute→buffer binding at vertexAttribPointer time. The
  // original code bound the pointers once to the static buffer, so every "dynamic"
  // draw re-rendered the static geometry instead — the gantry, X beam, carriage,
  // toolhead and belts never drew at all (fixed 2026-09-18). Re-bind per buffer.
  bindAttribs(buffer){const gl=this.gl;gl.bindBuffer(gl.ARRAY_BUFFER,buffer);this.attribs.forEach((a,i)=>gl.vertexAttribPointer(a,3,gl.FLOAT,false,48,i*12));}
 draw(picking=false){const t0=performance.now();const gl=this.gl,w=this.canvas.clientWidth,h=this.canvas.clientHeight;if(!w||!h)return;const d=Math.min(devicePixelRatio||1,2);if(this.canvas.width!==Math.round(w*d)||this.canvas.height!==Math.round(h*d)){this.canvas.width=Math.round(w*d);this.canvas.height=Math.round(h*d);}gl.viewport(0,0,this.canvas.width,this.canvas.height);gl.clearColor(picking?0:.026,picking?0:.031,picking?0:.033,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);const g=this.geometry();this.ids=g.ids;this.currentMatrix=matrix(w,h,g.model,this.yaw,this.pitch,this.zoom,this.pan);this.currentModel=g.model;gl.useProgram(this.program);gl.uniformMatrix4fv(this.mu,false,this.currentMatrix);gl.uniform1i(this.pu,picking?1:0);
 if(!picking){ // static geometry uploads only when selection/layers/flags changed; dynamics stream every frame
   this.bindAttribs(this.buffer);
   if(this._uploaded!==g._key){gl.bufferData(gl.ARRAY_BUFFER,g.sv,gl.DYNAMIC_DRAW);this._uploaded=g._key;}
   gl.drawArrays(gl.TRIANGLES,0,g.sv.length/12);
   this.bindAttribs(this.dynBuffer);gl.bufferData(gl.ARRAY_BUFFER,g.dv,gl.DYNAMIC_DRAW);gl.drawArrays(gl.TRIANGLES,0,g.dv.length/12);
   if(g.se.length){this.bindAttribs(this.edgeBuffer);
    if(this._edgesUploaded!==g._key){gl.bufferData(gl.ARRAY_BUFFER,g.se,gl.DYNAMIC_DRAW);this._edgesUploaded=g._key;}
    gl.drawArrays(gl.LINES,0,g.se.length/12);}
   if(g.de.length){this.bindAttribs(this.dynEdgeBuffer);gl.bufferData(gl.ARRAY_BUFFER,g.de,gl.DYNAMIC_DRAW);gl.drawArrays(gl.LINES,0,g.de.length/12);}
  }
  if(picking){this.bindAttribs(this.buffer);gl.bufferData(gl.ARRAY_BUFFER,g.sv,gl.DYNAMIC_DRAW);gl.drawArrays(gl.TRIANGLES,0,g.sv.length/12);
   this.bindAttribs(this.dynBuffer);gl.bufferData(gl.ARRAY_BUFFER,g.dv,gl.DYNAMIC_DRAW);gl.drawArrays(gl.TRIANGLES,0,g.dv.length/12);}
 if(!picking)this.drawMs=performance.now()-t0;
 this.canvas.dataset.camera=JSON.stringify({yaw:this.yaw,pitch:this.pitch,zoom:this.zoom,pan:this.pan});this.canvas.dataset.pose=JSON.stringify(g.model.pose);this.canvas.dataset.sampleTs=this.snapshot?.point?.ts===undefined?'':String(this.snapshot.point.ts);this.canvas.dataset.renderer='webgl-depth';}
 pick(x,y){this.draw(true);const gl=this.gl,p=new Uint8Array(4),d=this.canvas.width/this.canvas.clientWidth;gl.readPixels(Math.max(0,Math.min(this.canvas.width-1,Math.floor(x*d))),Math.max(0,Math.min(this.canvas.height-1,Math.floor((this.canvas.clientHeight-y)*d))),1,1,gl.RGBA,gl.UNSIGNED_BYTE,p);const idx=p[0]+p[1]*256+p[2]*65536;const result=this.ids[idx-1]||null;this.draw(false);return result;}
}

// ===== Canvas-2D fallback (2026-09-18 restyle). The machine view must never be
// blank: if WebGL is unavailable (Brave can disable it), the same geometry,
// camera math and interaction model are painted with the 2D context — painter
// sorted, edge-lit teal, same picking. Same public surface as ServiceScene. =====
const hex2rgb=h=>{const m=/^#?([0-9a-f]{6})$/i.exec(h||'');return m?[parseInt(m[1].slice(0,2),16)/255,parseInt(m[1].slice(2,4),16)/255,parseInt(m[1].slice(4,6),16)/255]:[.2,.2,.2];};
const LIGHT=[.42,-.58,.7];
const shade2d=(rgb,normal)=>{const l=.42+.48*Math.abs(normal[0]*LIGHT[0]+normal[1]*LIGHT[1]+normal[2]*LIGHT[2])+.18*Math.abs(normal[2]);return `rgb(${Math.min(255,Math.round((rgb[0]*l+.004)*255))},${Math.min(255,Math.round((rgb[1]*l+.020)*255))},${Math.min(255,Math.round((rgb[2]*l+.019)*255))})`;};
const inside2d=(x,y,pts)=>{let hit=false;for(let i=0,j=pts.length-1;i<pts.length;j=i++){const xi=pts[i][0],yi=pts[i][1],xj=pts[j][0],yj=pts[j][1];if((yi>y)!==(yj>y)&&x<(xj-xi)*(y-yi)/(yj-yi)+xi)hit=!hit;}return hit;};
export class ServiceScene2D{
 constructor(canvas,select){this.canvas=canvas;this.select=select;this.snapshot=null;this.selected='head';this.yaw=-.62;this.pitch=.46;this.zoom=1.3;this.pan=[0,0];this.explode=0;this.hidden=new Set(['covers']);this.pointer=new Map();this.moved=0;this.frame=0;this.drawMs=0;this.faces=[];window.__stellaScene=this;this.ctx=canvas.getContext('2d');
  this.tip=document.getElementById('scene-tip');this.hoverAt=0;this.hoverId=null;
  canvas.addEventListener('contextmenu',e=>e.preventDefault());
  canvas.addEventListener('pointerdown',e=>{canvas.setPointerCapture(e.pointerId);this.pointer.set(e.pointerId,{x:e.clientX,y:e.clientY});this.moved=0;if(this.tip)this.tip.hidden=true;});
  canvas.addEventListener('pointermove',e=>{const prev=this.pointer.get(e.pointerId);if(!prev){
   if(this.tip){const now=performance.now();if(now-this.hoverAt>90){this.hoverAt=now;const rect=canvas.getBoundingClientRect();const id=this.pick(e.clientX-rect.left,e.clientY-rect.top);this.hoverId=id;
    if(id){const s=slotById(id)||SLOTS.find(x=>x.id.split('.').pop()===id);this.tip.textContent=s?`${s.name} — ${s.location}`:id.charAt(0).toUpperCase()+id.slice(1).replace(/_/g,' ');this.tip.style.left=Math.min(canvas.clientWidth-180,e.clientX-rect.left+14)+'px';this.tip.style.top=Math.max(6,e.clientY-rect.top-32)+'px';this.tip.hidden=false;canvas.style.cursor='pointer';}else{this.tip.hidden=true;canvas.style.cursor='grab';}}}
   return;}
  const before=[...this.pointer.values()],next={x:e.clientX,y:e.clientY},dx=next.x-prev.x,dy=next.y-prev.y;this.pointer.set(e.pointerId,next);this.moved+=Math.hypot(dx,dy);
  if(this.pointer.size===2){const after=[...this.pointer.values()],a=Math.hypot(before[0].x-before[1].x,before[0].y-before[1].y),b=Math.hypot(after[0].x-after[1].x,after[0].y-after[1].y);if(a>5)this.zoom=Math.min(5,Math.max(.3,this.zoom*b/a));this.pan[0]+=dx/this.canvas.clientWidth;this.pan[1]-=dy/this.canvas.clientHeight;}
  else if(e.shiftKey||e.buttons===2||e.buttons===4){this.pan[0]+=2*dx/this.canvas.clientWidth;this.pan[1]-=2*dy/this.canvas.clientHeight;}
  else{this.yaw+=dx*.007;this.pitch=Math.max(-1.565,Math.min(1.565,this.pitch+dy*.006));}this.schedule();});
  canvas.addEventListener('pointerup',e=>{if(this.pointer.size===1&&this.moved<5){const rect=canvas.getBoundingClientRect(),id=this.pick(e.clientX-rect.left,e.clientY-rect.top);if(id)this.select(id);}this.pointer.delete(e.pointerId);});
  canvas.addEventListener('pointercancel',e=>this.pointer.delete(e.pointerId));
  canvas.addEventListener('pointerleave',()=>{if(this.tip)this.tip.hidden=true;});
  canvas.addEventListener('wheel',e=>{if(!e.ctrlKey&&!e.metaKey&&!e.shiftKey)return;e.preventDefault();this.zoom=Math.min(5,Math.max(.3,this.zoom*Math.exp(-e.deltaY*.001)));this.schedule();},{passive:false});
  canvas.addEventListener('dblclick',()=>this.focus(this.selected));
  canvas.addEventListener('keydown',e=>{const d=.08;if(e.key.startsWith('Arrow')){e.preventDefault();if(e.shiftKey){this.pan[0]+=e.key==='ArrowLeft'?-d:e.key==='ArrowRight'?d:0;this.pan[1]+=e.key==='ArrowUp'?d:e.key==='ArrowDown'?-d:0;}else{this.yaw+=e.key==='ArrowLeft'?-d:e.key==='ArrowRight'?d:0;this.pitch=Math.max(-1.565,Math.min(1.565,this.pitch+(e.key==='ArrowUp'?d:e.key==='ArrowDown'?-d:0)));}}else if(e.key==='+'||e.key==='=')this.zoom=Math.min(5,this.zoom*1.1);else if(e.key==='-')this.zoom=Math.max(.3,this.zoom/1.1);else if(e.key==='0')this.view('iso');else return;this.schedule();});
  new ResizeObserver(()=>this.schedule()).observe(canvas);
 }
 view(v){[this.yaw,this.pitch]=CAMERA_VIEWS[v]||CAMERA_VIEWS.iso;this.pan=[0,0];this.zoom=1;this.schedule();}
 set(snapshot,selected){this.snapshot=snapshot;this.selected=selected;if(this.frame){cancelAnimationFrame(this.frame);this.frame=0;}this.draw();}
 schedule(){if(!this.frame)this.frame=requestAnimationFrame(()=>{this.frame=0;this.draw();});}
 focus(id){const model=serviceLayout(this.snapshot,this.explode),slot=slotById(id),parent=slot?.parent||id,parts=model.parts.filter(p=>p.id===id||p.parent===parent||p.id===parent);if(!parts.length)return;this.pan=[0,0];this.zoom=2;const c=[0,1,2].map(i=>parts.reduce((s,p)=>s+p.pos[i],0)/parts.length);const m=matrix(this.canvas.clientWidth,this.canvas.clientHeight,model,this.yaw,this.pitch,this.zoom,this.pan),p=project(c,m,this.canvas.clientWidth,this.canvas.clientHeight);this.pan=[1-2*p.x/this.canvas.clientWidth,2*p.y/this.canvas.clientHeight-1];this.schedule();}
 colorFor(p){const slot=slotById(this.selected),target=slot?.parent||this.selected;if(p.id===this.selected||p.id===target||p.parent===target)return col.selected;if(this.snapshot?.point?.motors?.[p.parent]?.flags?.length)return col.warn;return p.color;}
 draw(){const t0=performance.now();const w=this.canvas.clientWidth,h=this.canvas.clientHeight;if(!w||!h)return;const d=Math.min(devicePixelRatio||1,2);if(this.canvas.width!==Math.round(w*d)||this.canvas.height!==Math.round(h*d)){this.canvas.width=Math.round(w*d);this.canvas.height=Math.round(h*d);}const ctx=this.ctx;ctx.setTransform(d,0,0,d,0,0);ctx.clearRect(0,0,w,h);
  const model=serviceLayout(this.snapshot,this.explode);
  const m=matrix(w,h,model,this.yaw,this.pitch,this.zoom,this.pan);this.currentMatrix=m;this.currentModel=model;
  if(!this.snapshot){ctx.fillStyle='#879396';ctx.font='500 11px "Geist Mono", monospace';ctx.textAlign='center';ctx.fillText('Waiting for recorded data',w/2,h/2);this.canvas.dataset.renderer='canvas2d';return;}
  const faces=[];
  for(const p of model.parts){if(this.hidden.has(p.layer))continue;const rgb=hex2rgb(this.colorFor(p));
   for(const poly of vertices(p)){const normal=norm(cross(vecsub(poly[1],poly[0]),vecsub(poly[2],poly[0])));const pts=[];let z=0;for(const v of poly){const q=project(v,m,w,h);pts.push([q.x,q.y]);z+=q.z;}faces.push({id:p.id,pts,z:z/poly.length,fill:shade2d(rgb,normal),glow:this.colorFor(p)===col.selected&&p.dyn});}
   for(const e2 of edges(p))faces.push({edge:true,id:p.id,seg:e2});
  }
  for(const f of faces)if(f.edge){const q1=project(f.seg[0],m,w,h),q2=project(f.seg[1],m,w,h);f.pts=[[q1.x,q1.y],[q2.x,q2.y]];f.z=(q1.z+q2.z)/2+.001;}
  // Painter order: farthest (largest depth) first, so nearer faces overpaint.
  faces.sort((a,b)=>b.z-a.z);
  this.faces=faces;
  ctx.lineJoin='round';
  for(const f of faces){
   if(f.edge){ctx.strokeStyle='rgba(4,217,196,.28)';ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(f.pts[0][0],f.pts[0][1]);ctx.lineTo(f.pts[1][0],f.pts[1][1]);ctx.stroke();continue;}
   ctx.beginPath();ctx.moveTo(f.pts[0][0],f.pts[0][1]);for(let i=1;i<f.pts.length;i++)ctx.lineTo(f.pts[i][0],f.pts[i][1]);ctx.closePath();
   if(f.glow){ctx.shadowColor='rgba(4,217,196,.8)';ctx.shadowBlur=12;}
   ctx.fillStyle=f.fill;ctx.fill();
   if(f.glow){ctx.shadowBlur=0;ctx.shadowColor='transparent';}
  }
  this.drawMs=performance.now()-t0;
  this.canvas.dataset.camera=JSON.stringify({yaw:this.yaw,pitch:this.pitch,zoom:this.zoom,pan:this.pan});this.canvas.dataset.pose=JSON.stringify(model.pose);this.canvas.dataset.sampleTs=this.snapshot?.point?.ts===undefined?'':String(this.snapshot.point.ts);this.canvas.dataset.renderer='canvas2d';
 }
 pick(x,y){for(let i=this.faces.length-1;i>=0;i--){const f=this.faces[i];if(f.edge)continue;if(inside2d(x,y,f.pts))return f.id;}return null;}
}
export function createScene(canvas,select){try{return new ServiceScene(canvas,select);}catch(e){console.warn('Stella Scope: WebGL unavailable, painting the machine view with canvas 2D.',e);return new ServiceScene2D(canvas,select);}}
