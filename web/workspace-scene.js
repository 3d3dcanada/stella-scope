import {pose,partById,severity,finite} from './workspace-core.js';
const C={frame:'#3d444c',rail:'#ccd5d9',dark:'#2a3138',plate:'#cdd6da',printed:'#f4791f',selected:'#04D9C4',belt:'#d4b06a',motor:'#6b757d',gold:'#e6b05f',board:'#1c2026',steel:'#aab4b9'};
const box=(id,pos,size,color=C.frame)=>({id,pos,size,color,kind:'box'});
const cylinder=(id,pos,r,h,color=C.motor)=>({id,pos,r,h,color,kind:'cylinder'});
// Axle = cylinder along Y (horizontal, front/back) for motor shafts, pulleys, hob gears, fan hubs.
const axle=(id,pos,r,len,color=C.steel)=>({id,pos,r,len,color,kind:'axle'});
// Slot hint: a dark inset line running the long axis of an extrusion, seen on the inward face.
const slot=(id,pos,size)=>({id,pos,size,color:'#1a1f24',kind:'box'});
// Proportioned R2 kinematic envelopes, not dimensional CAD or independent corner telemetry.
// Palette and part density raised 2026-09-17 (WO-04): still schematic, still no invented telemetry.
export function layout(snapshot){
 const p=pose(snapshot),[w,d]=p.bed,bz=74,gz=bz+p.z+58,top=bz+p.zmax+112,a=[];
 let dyn=false; // dyn parts depend on the reported pose (whole gantry rides Z); the rest caches in the renderer.
 const b=(...args)=>{const o=box(...args);o.dyn=dyn;a.push(o);},cy=(...args)=>{const o=cylinder(...args);o.dyn=dyn;a.push(o);},ax=(...args)=>{const o=axle(...args);o.dyn=dyn;a.push(o);};
 const slotDyn=(...args)=>{const o=slot(...args);o.dyn=dyn;a.push(o);};
 // NEMA17-style motor composite: body, machined end plate on the shaft face, corner bolts, boss, shaft.
 const motor=(id,x,y,z,dir=1)=>{b(id,[x,y,z],[42,42,42],C.dark);b(id,[x,y+dir*21,z],[44,2.5,44],C.motor);
  for(const dx of [-15,15])for(const dz of [-15,15])ax(id,[x+dx,y+dir*23,z+dz],2.5,3.5,C.gold);
  ax(id,[x,y+dir*26,z],9,4,C.motor);ax(id,[x,y+dir*33,z],4,20,C.steel);};
 // GT2-style pulley: drum with gold flanges and dark tooth bands.
 const pulley=(id,x,y,z,r,w)=>{ax(id,[x,y,z],r,w,C.motor);for(const dy of [-w/2,w/2])b(id,[x,y+dy,z],[r*2+4,2.5,r*2+4],C.gold);
  for(const dy of [-w/4,w/4])b(id,[x,y+dy,z],[r*2-.5,1.2,r*2-.5],C.dark);};
 // Frame: verticals with groove hint, top/bottom rails, corner feet.
 for(const x of [-55,w+55])for(const y of [-55,d+55]){
  b('frame',[x,y,top/2],[20,20,top]);a.push(slot('frame',[x+(x<0?10:-10),y,top/2],[1.6,6,top-14]));
  b('frame',[x,y,-5],[38,38,10],C.dark);
 }
 for(const z of [14,top-10]){
  for(const y of [-55,d+55]){b('frame',[w/2,y,z],[w+110,20,20]);a.push(slot('frame',[w/2,y+(y<0?10:-10),z],[w+96,1.6,3.5]));}
  for(const x of [-55,w+55]){b('frame',[x,d/2,z],[20,d+110,20]);a.push(slot('frame',[x+(x<0?10:-10),d/2,z],[3.5,1.6,d+96]));}
 }
  b('frame',[w/2,d/2,31],[w+90,d+90,4],C.dark);
  // Base skirts (2026-09-18 build-out): a dark band around the feet grounds the
  // silhouette and reads as a machine at one glance. Static, schematic.
  b('frame',[w/2,-64,12],[w+108,14,44],C.dark);
  b('frame',[w/2,d+64,12],[w+108,14,44],C.dark);
  for(const x of [-64,w+64])b('frame',[x,d/2,12],[14,d+108,44],C.dark);
  // Frame gussets (2026-09-18 build-out): machined corner cubes at the eight joints.
  for(const x of [-55,w+55])for(const y of [-55,d+55])for(const z of [14,top-10])b('frame',[x,y,z],[26,26,26],C.dark);
  // Front display (2026-09-18 build-out): bezel and dark screen on the front skirt.
  b('frame',[w*.5,-73,28],[56,10,32],C.dark);
  b('frame',[w*.5,-69,28],[46,3,24],'#10161a');
 // Fixed bed: steel rails, support feet, dark heater plate, PEI sheet, corner tabs.
 for(const x of [w*.27,w*.73])b('heater_bed',[x,d/2,48],[20,d+75,20],C.frame);
 for(const x of [w*.15,w*.85])for(const y of [d*.15,d*.85])cy('heater_bed',[x,y,61],5,17,C.steel);
 b('heater_bed',[w/2,d/2,bz-5],[w+8,d+8,9],C.dark);
 b('heater_bed',[w/2,d/2,bz],[w,d,2],C.plate);
 for(const x of [-8,w+8])for(const y of [-8,d+8])b('heater_bed',[x,y,bz+1],[10,10,1.5],C.gold);
 // Z drives — the real R2 arrangement (owner's 154-slot inventory as the blueprint):
 // sideways motor → motor pulley → belt loop → jackshaft in bearing blocks → reduction pulley →
 // vertical Z belt → top idler. Owner-corrected 2026-09-16: front shafts face front, rear face back.
 const corners=[[-34,-27,'stepper_z'],[-34,d+27,'stepper_z1'],[w+34,d+27,'stepper_z2'],[w+34,-27,'stepper_z3']];
 for(const [x,y,id] of corners){
  b('frame',[x+(x<0?-12:12),y,top/2],[8,13,top-75],C.steel);   // vertical corner rail the drive mounts to
  const out=y<0?-1:1,yj=y+out*30;
  motor(id,x,y,30,out);                                        // sideways motor, shaft faces front/back
  pulley(id,x,y+out*40,30,9,9);                                // motor pulley
  b(id,[x,y+out*40,48],[2.5,7,22],C.belt);                     // belt loop run at the motor pulley
  b(id,[x,yj,48],[2.5,7,22],C.belt);                           // belt loop run at the jackshaft pulley
  b(id,[x,y+out*20,66],[30,44,16],C.printed);                  // drive mount block on the corner rail
  ax(id,[x,yj,66],5,46,C.steel);                               // jackshaft
  for(const dy of [-20,20])b(id,[x,yj+dy,66],[15,8,26],C.printed); // bearing blocks
  pulley(id,x,yj,66,11,11);                                    // reduction / Z-belt pulley
  for(const dx of [-9,9])b(id,[x+dx,yj,(66+top-20)/2],[2.5,7,top-20-66],C.belt); // vertical Z belts
  ax(id,[x,yj,top-20],10,14,C.dark);                            // top idler axle (belt axis, like the jackshaft)
  b(id,[x,yj,top-20],[30,26,20],C.printed);                    // top idler housing
  dyn=true; // gantry joints ride the gantry (gz follows reported Z)
  b(id,[x,y,gz],[30,34,30],C.printed);b(id,[x+(x<0?-8:8),y,gz],[12,25,42],C.dark); // gantry joint + clamp
  dyn=false;
 }
 dyn=true; // from here through the toolhead everything rides the gantry
 // Gantry: side extrusions with grooves, rear crossbar only (owner-corrected), Y rails, carriages.
 for(const x of [-24,w+24]){
  b('gantry',[x,d/2,gz],[20,d+75,20],C.frame);
  slotDyn('gantry',[x+(x<0?10:-10),d/2,gz],[3.5,1.6,d+60]);
  b('y_rails',[x,d/2,gz+12],[11,d+57,5],C.rail);
  b('gantry',[x,p.y,gz+17],[30,39,12],C.printed);
   for(const yy of [-22,d+22]){b('gantry',[x,yy,gz+12],[37,36,22],C.printed);cy('gantry',[x,yy,gz+12],8,7,C.steel);} // corner XY idlers, vertical axle like the belts that wrap them
 }
 b('gantry',[w/2,d+26,gz],[w+70,20,20],C.frame);
 slotDyn('gantry',[w/2,d+16,gz],[w+56,1.6,3.5]);
 // X beam rides the Y rails; carriage and toolhead move with reported position.
 b('x_rail',[w/2,p.y,gz+20],[w+61,20,20],C.frame);
 slotDyn('x_rail',[w/2,p.y-10,gz+20],[w+48,1.6,3.5]);
 b('x_rail',[w/2,p.y-11,gz+20],[w+47,5,12],C.rail); // MGN12 rail line
 b('head',[p.x,p.y,gz+20],[46,26,26],C.printed);     // X carriage block
 // CoreXY belts: side runs plus the moving cross runs (recomputed with the head each frame).
 for(const z of [gz+25,gz+34]){
  b('gantry',[w/2,p.y-16,z],[w+40,2.5,2.5],C.belt);
  for(const x of [-20,w+20])b('gantry',[x,d/2,z],[2.5,d+48,2.5],C.belt);
 }
 // Rear A/B drives: NEMA17 motors with shafts facing back, GT2 pulleys between the belt planes.
 for(const [x,id] of [[-16,'stepper_x'],[w+16,'stepper_y']]){
  b(id,[x,d+16,gz+29],[46,12,52],C.printed);   // mount plate on the rear crossbar
  motor(id,x,d+8,gz+29,1);                     // motor, shaft toward the back
  pulley(id,x,d+34,gz+29,9,10);                // A/B pulley riding both belt planes
 }
 // Toolhead: Rapid Burner v8 envelope — blower with hub and blades, ducts, heatsink fins, heater block.
 b('head',[p.x,p.y-20,gz+19],[42,25,40],C.printed);
 b('head',[p.x,p.y-33,gz+6],[44,27,42],C.dark);
 ax('hotend_fan',[p.x,p.y-14,gz+7],11,8,C.dark);
 for(const t of [0,Math.PI/4])b('hotend_fan',[p.x+Math.cos(t)*8,p.y-14,gz+7+Math.sin(t)*8],[3,13,3],C.steel);
 b('fan',[p.x+23,p.y-22,gz-5],[17,32,25],C.printed);
 b('fan',[p.x+30,p.y-25,gz-5],[5,22,18],C.dark);
 ax('fan',[p.x+30,p.y-22,gz-5],9,4,C.steel);
 for(const y of [-6,6])b('fan',[p.x+33,p.y-22+y*.9,gz-5+y*.4],[9,7,12],C.printed); // part-cooling ducts
 for(let i=0;i<5;i++)b('hotend',[p.x,p.y,gz-2-i*3.4],[14,14,1.6],C.gold);          // heatsink fin stack
  b('extruder',[p.x,p.y-20,gz+52],[26,26,24],C.dark);                              // extruder pancake motor
  b('extruder',[p.x,p.y-20,gz+52],[27,2,27],C.motor);
  for(const dx of [-9,9])for(const dz of [-9,9])ax('extruder',[p.x+dx,p.y-8,gz+52+dz],1.8,3,C.gold); // motor bolts
  b('extruder',[p.x,p.y-20,gz+40],[33,32,10],C.steel);
  ax('extruder',[p.x,p.y-9,gz+44],6,6,C.gold);                                      // hob gear face
 b('extruder',[p.x,p.y-20,gz+66],[3,3,10],C.steel);                                // filament inlet
 b('hotend',[p.x,p.y,bz+p.z+22],[16,13,9],C.gold);                                 // heater block
 b('hotend',[p.x+9,p.y,bz+p.z+22],[3,3,7],'#c7332c');                              // thermistor
 cy('hotend',[p.x,p.y,bz+p.z+3],4,7,C.gold);                                       // nozzle
 // BLTouch: body, blue stripe, pin, metal mount at the recorded probe offset.
 const px=p.x+p.probe[0],py=p.y+p.probe[1];
 b('probe',[px,py,bz+p.z+15],[13,15,26],C.dark);
 b('probe',[px,py,bz+p.z+15],[13.5,15.5,4],C.board);
 b('probe',[px,py-8,bz+p.z+15],[15,4,20],C.steel);
  cy('probe',[px,py,bz+p.z+1],2.5,3,'#c7332c');
  // Umbilical (2026-09-18 build-out): the toolhead's cable run, orthogonal like a real
  // harness — straight up from the head, back along the underside of the top frame,
  // down the rear corner to the bay. Hugs the silhouette; never crosses the build volume.
  {const ux=p.x,uy=p.y-20,zt=top-16,rise=zt-(gz+50),backY=d+10,downX=w*.62;
   b('head',[ux,uy,gz+50+rise/2],[7,7,rise],'#454e57');
   b('head',[ux,(uy+backY)/2,zt],[7,Math.abs(backY-uy)+7,7],'#454e57');
   b('head',[(ux+downX)/2,backY,zt],[Math.abs(downX-ux)+7,7,7],'#454e57');
   b('head',[downX,backY,(40+zt)/2],[7,7,zt-40],'#454e57');}
  dyn=false;
 b('controller_fan',[w*.77,30,44],[45,45,8],C.dark);
 cy('controller_fan',[w*.77,30,49],16,3,C.rail);
 return {parts:a,pose:p,bedZ:bz,gantryZ:gz,top};
}
function shade(hex,n){const h=hex.slice(1),rgb=[0,2,4].map(i=>Math.max(0,Math.min(255,Math.round(parseInt(h.slice(i,i+2),16)*n))));return `rgb(${rgb.join(',')})`;}
function inside(p,polygon){let result=false;for(let i=0,j=polygon.length-1;i<polygon.length;j=i++){
 const a=polygon[i],b=polygon[j];if((a.y>p.y)!==(b.y>p.y)&&p.x<(b.x-a.x)*(p.y-a.y)/(b.y-a.y)+a.x)result=!result;
}return result;}
// The earlier height-only scale clipped the top frame when depth projected into Y.
// Fit the fixed machine envelope so camera scale does not change as the head travels.
export function fitScale(width,height,bed,top,yaw,elevation){
 const a=Math.abs(Math.cos(yaw)),b=Math.abs(Math.sin(yaw));
 const horizontal=a*(bed[0]+180)+b*(bed[1]+180);
 const vertical=Math.abs(Math.cos(elevation))*(top+30)+Math.abs(Math.sin(elevation))*(b*(bed[0]+180)+a*(bed[1]+180));
 return Math.min(Math.max(1,width-48)/horizontal,Math.max(1,height-80)/vertical);
}
export class PrinterScene{
 constructor(canvas,onSelect){this.canvas=canvas;this.onSelect=onSelect;this.snapshot=null;this.selected='head';this.yaw=-.53;this.elevation=.47;this.zoom=1;this.showFrame=true;this.showLabels=true;this.showTrail=true;this.hits=[];this.pointer=null;
 canvas.addEventListener('pointerdown',e=>{this.pointer={id:e.pointerId,x:e.offsetX,y:e.offsetY,startX:e.offsetX,startY:e.offsetY,yaw:this.yaw,elevation:this.elevation};canvas.setPointerCapture(e.pointerId);});
 canvas.addEventListener('pointermove',e=>{if(this.pointer){this.yaw=this.pointer.yaw+(e.offsetX-this.pointer.x)*.008;this.elevation=Math.max(.06,Math.min(1.55,this.pointer.elevation-(e.offsetY-this.pointer.y)*.006));this.draw();}else{canvas.style.cursor=this.pick(e.offsetX,e.offsetY)?'pointer':'grab';}});
 canvas.addEventListener('pointerup',e=>{const p=this.pointer;this.pointer=null;if(p&&Math.hypot(e.offsetX-p.startX,e.offsetY-p.startY)<6){const id=this.pick(e.offsetX,e.offsetY);if(id)this.onSelect(id);}});
 canvas.addEventListener('pointercancel',()=>this.pointer=null);
 // Plain wheel scrolls the page; only a held modifier zooms the model (trackpad pinch arrives as ctrl+wheel).
 canvas.addEventListener('wheel',e=>{if(!e.ctrlKey&&!e.metaKey&&!e.shiftKey)return;e.preventDefault();this.zoom=Math.max(.6,Math.min(1.8,this.zoom*Math.exp(-e.deltaY*.001)));this.draw();},{passive:false});
 canvas.addEventListener('keydown',e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();this.yaw+=e.key==='ArrowLeft'?-.12:.12;this.draw();}if(e.key==='ArrowUp'||e.key==='ArrowDown'){e.preventDefault();this.elevation=Math.max(.06,Math.min(1.55,this.elevation+(e.key==='ArrowUp'?.1:-.1)));this.draw();}});
 this.observer=new ResizeObserver(()=>this.draw());this.observer.observe(canvas);
 }
 set(snapshot,selected){this.snapshot=snapshot;this.selected=selected;this.draw();}
 view(name){this.zoom=1;[this.yaw,this.elevation]=name==='top'?[0,1.55]:name==='front'?[0,.06]:name==='right'?[Math.PI/2,.12]:[-.53,.47];this.draw();}
 pick(x,y){for(let i=this.hits.length-1;i>=0;i--)if(inside({x,y},this.hits[i].poly))return this.hits[i].id;return null;}
 project(p){const {w,h,s,cx,cy,cz}=this.projection;let x=p[0]-cx,y=p[1]-cy,z=p[2]-cz;const ca=Math.cos(this.yaw),sa=Math.sin(this.yaw),ce=Math.cos(this.elevation),se=Math.sin(this.elevation),a=x*ca-y*sa,b=x*sa+y*ca;return {x:w/2+a*s,y:h*.51+(-z*ce-b*se)*s,depth:b*ce-z*se};}
 draw(){
 const rect=this.canvas.getBoundingClientRect();if(!rect.width||!rect.height)return;
 const w=rect.width,h=rect.height,dpr=Math.min(devicePixelRatio||1,2);this.canvas.width=Math.round(w*dpr);this.canvas.height=Math.round(h*dpr);const c=this.canvas.getContext('2d');c.scale(dpr,dpr);
 const model=layout(this.snapshot),{bed}=model.pose;this.projection={w,h,s:fitScale(w,h,bed,model.top,this.yaw,this.elevation)*this.zoom,cx:bed[0]/2,cy:bed[1]/2,cz:model.top/2};
 c.fillStyle='#0b0e0f';c.fillRect(0,0,w,h);
 const worldLine=(points,color,width=1,dash=[])=>{c.strokeStyle=color;c.lineWidth=width;c.setLineDash(dash);c.beginPath();points.forEach((p,i)=>{const q=this.project(p);i?c.lineTo(q.x,q.y):c.moveTo(q.x,q.y);});c.stroke();c.setLineDash([]);};
 for(let x=-100;x<=bed[0]+100;x+=50)worldLine([[x,-100,-12],[x,bed[1]+100,-12]],'#1b2124');
 for(let y=-100;y<=bed[1]+100;y+=50)worldLine([[-100,y,-12],[bed[0]+100,y,-12]],'#1b2124');
 const faces=[];
  for(let item of model.parts){if(item.id==='frame'&&!this.showFrame)continue;const col=item.id===this.selected?C.selected:severity(this.snapshot?.point,item.id)==='warning'?'#FF6A00':item.color;
   if(item.kind==='axle')item={...item,kind:'box',size:[item.r*2,item.len,item.r*2]}; // 2D page degrades axles to blocks
   if(item.kind==='box'){
   const [x,y,z]=item.pos,[a,b,d]=item.size.map(n=>n/2),v=[[-a,-b,-d],[a,-b,-d],[a,b,-d],[-a,b,-d],[-a,-b,d],[a,-b,d],[a,b,d],[-a,b,d]].map(p=>this.project([p[0]+x,p[1]+y,p[2]+z]));
   [[0,1,2,3],[4,7,6,5],[0,4,5,1],[1,5,6,2],[2,6,7,3],[3,7,4,0]].forEach((ids,i)=>{const poly=ids.map(k=>v[k]);faces.push({id:item.id,poly,depth:poly.reduce((n,p)=>n+p.depth,0)/4,color:shade(col,[.48,1.23,.77,.94,.65,.85][i])});});
  }else{
   const n=12,lo=[],hi=[];for(let i=0;i<n;i++){const t=i*Math.PI*2/n;lo.push(this.project([item.pos[0]+item.r*Math.cos(t),item.pos[1]+item.r*Math.sin(t),item.pos[2]-item.h/2]));hi.push(this.project([item.pos[0]+item.r*Math.cos(t),item.pos[1]+item.r*Math.sin(t),item.pos[2]+item.h/2]));}
   for(let i=0;i<n;i++){const j=(i+1)%n,poly=[lo[i],lo[j],hi[j],hi[i]];faces.push({id:item.id,poly,depth:poly.reduce((s,p)=>s+p.depth,0)/4,color:shade(col,.6+.3*(1+Math.cos(i*.52)))});}faces.push({id:item.id,poly:hi,depth:hi.reduce((s,p)=>s+p.depth,0)/n,color:shade(col,1.2)});
  }
 }
 faces.sort((a,b)=>b.depth-a.depth);this.hits=[];for(const face of faces){c.beginPath();face.poly.forEach((p,i)=>i?c.lineTo(p.x,p.y):c.moveTo(p.x,p.y));c.closePath();c.fillStyle=face.color;c.fill();c.strokeStyle='#0d172075';c.lineWidth=.55;c.stroke();this.hits.push(face);}
 const {x,y,z,known,probe}=model.pose;
 // The dotted bed grid and path markers are overlays, explicitly not a mesh surface.
 for(let q=0;q<=bed[0];q+=50)worldLine([[q,0,model.bedZ+1],[q,bed[1],model.bedZ+1]],'#68818b35',.65);
 for(let q=0;q<=bed[1];q+=50)worldLine([[0,q,model.bedZ+1],[bed[0],q,model.bedZ+1]],'#68818b35',.65);
 if(this.showTrail){const path=this.snapshot?.history||[];c.fillStyle='#75dac66b';for(const p of path){if(p.position?.every(v=>finite(v)!==null)){const s=this.project([p.position[0],p.position[1],model.bedZ+p.position[2]+1]);c.beginPath();c.arc(s.x,s.y,1.8,0,Math.PI*2);c.fill();}}}
 if(known){worldLine([[x,y,model.bedZ+z],[x,y,model.bedZ+1]],'#04D9C4',1,[4,5]);worldLine([[x-7,y,model.bedZ+z],[x+7,y,model.bedZ+z]],'#dffff9',1.6);worldLine([[x,y-7,model.bedZ+z],[x,y+7,model.bedZ+z]],'#dffff9',1.6);const q=this.project([x+probe[0],y+probe[1],model.bedZ+z]);c.strokeStyle='#FF2D92';c.beginPath();c.arc(q.x,q.y,5,0,Math.PI*2);c.stroke();}
 if(this.showLabels){
  const anchors=[['stepper_z',[-34,-27,70]],['stepper_z1',[-34,bed[1]+27,70]],['stepper_z2',[bed[0]+34,bed[1]+27,70]],['stepper_z3',[bed[0]+34,-27,70]],['stepper_x',[-16,bed[1]+20,model.gantryZ+25]],['stepper_y',[bed[0]+16,bed[1]+20,model.gantryZ+25]],['head',[x,y-25,model.gantryZ+44]],['heater_bed',[bed[0]*.6,0,model.bedZ]],['probe',[x+probe[0],y+probe[1],model.bedZ+z]]];
  const used=[];for(const [id,pos] of anchors){const q=this.project(pos),label=partById(id).short;c.font='600 10px system-ui';const tw=c.measureText(label).width+16;let lx=Math.max(8,Math.min(w-tw-8,q.x+12)),ly=Math.max(20,Math.min(h-28,q.y-15));for(let attempt=0;attempt<12&&used.some(r=>Math.abs(r.y-ly)<25&&lx<r.x+r.w+6&&lx+tw>r.x-6);attempt++)ly=Math.min(h-27,ly+25);used.push({x:lx,y:ly,w:tw});c.strokeStyle=id===this.selected?C.selected:'#8793968a';c.lineWidth=.8;c.beginPath();c.moveTo(q.x,q.y);c.lineTo(lx,ly);c.stroke();c.fillStyle=id===this.selected?'#0d3330':'#14232beb';c.fillRect(lx,ly-11,tw,22);c.strokeRect(lx,ly-11,tw,22);c.fillStyle=id===this.selected?'#a8fff2':'#e8f6f3';c.fillText(label,lx+8,ly+3);this.hits.push({id,poly:[{x:lx,y:ly-11},{x:lx+tw,y:ly-11},{x:lx+tw,y:ly+11},{x:lx,y:ly+11}]});}
 }
 const front=this.project([bed[0]/2,-84,8]);c.font='600 10px system-ui';c.fillStyle='#879396';c.textAlign='center';c.fillText('FRONT  ·  X →',front.x,front.y);c.textAlign='left';
 c.fillStyle=known?'#879396':'#FF6A00';c.font='11px system-ui';c.fillText(known?'Position-synchronised kinematic model':'NO POSITION DATA · ghost layout only',16,23);
 c.fillStyle='#6f7a7d';c.font='10px system-ui';c.fillText('Drag to orbit · Ctrl+scroll zooms (plain scroll moves the page) · click a component',16,h-14);
 this.canvas.dataset.pose=JSON.stringify({x,y,z,known,gantryZ:model.gantryZ,bedZ:model.bedZ});
 }
}
