import * as THREE from './vendor/three.module.js';
import {OrbitControls} from './vendor/OrbitControls.js';
import {STLLoader} from './vendor/STLLoader.js';
import {jobDisplay,instanceRecordId,insideTravel} from './hud-core.js';

export const CAMERA_VIEWS={iso:[1,-1,.65],front:[0,-1,0],rear:[0,1,0],left:[-1,0,0],right:[1,0,0],top:[0,0,1],bottom:[0,0,-1]};
const empty=new THREE.Box3();
export function createScene(canvas,onSelect,opts={}){
 // NEXT (docs/next-ui/BLUEPRINT.md section 9): opts.stage==='next' is opt-in staging for the rebuilt HUD.
 // Without it every line below behaves exactly as before, so the old HUD at / is unchanged.
 // WO-14 item 1: opts.tile draws the same scene inside a farm-wall tile: no picking or orbiting
 // (the tile's own click opens the printer), no global scene handle, and fetches go through
 // opts.apiPrefix ('/api/p/<id>') so the tile talks to its own printer.
 const nextStage=opts?.stage==='next',tileMode=opts?.tile===true;
 const _bc=new THREE.Vector3(),_sp=new THREE.Vector3(),_sz=new THREE.Vector3();let lastTile=0,tileTimer=0,tileMin=-1;   // (declared up here: resize() below can ask for a frame before the code that draws it is reached)
 const apiPrefix=typeof opts?.apiPrefix==='string'?opts.apiPrefix:'';
 const apiUrl=u=>apiPrefix&&u.startsWith('/api/')?apiPrefix+u.slice(4):u;
 const renderer=new THREE.WebGLRenderer({canvas,antialias:true,alpha:nextStage});renderer.setPixelRatio(Math.min(devicePixelRatio,1.5));renderer.setClearColor(0x101314,nextStage?0:1);renderer.outputColorSpace=THREE.SRGBColorSpace;renderer.toneMapping=THREE.ACESFilmicToneMapping;renderer.toneMappingExposure=1.35;renderer.localClippingEnabled=true;
 const world=new THREE.Scene(),camera=new THREE.PerspectiveCamera(32,1,.1,20000);camera.up.set(0,0,1);camera.position.set(1000,-1400,1000);
 world.add(new THREE.HemisphereLight(0xc3e5ff,0x182028,2.4));
 // NEXT review 2026-09-25 (the owner: "alive, real depth ... car dashboard"): a warm key from the front
 // left and a cool rim from behind, so the machine separates from the dark room instead of being
 // lit evenly from everywhere.
 if(nextStage){const key=new THREE.DirectionalLight(0xffe9d2,1.6);key.userData.stageLight=true;key.position.set(-900,-1100,1300);world.add(key);
  const rim=new THREE.DirectionalLight(0x6fd6ff,2.6);rim.userData.stageLight=true;rim.position.set(-400,1600,900);world.add(rim);
  const rim2=new THREE.DirectionalLight(0xb48cff,1.2);rim2.userData.stageLight=true;rim2.position.set(1500,900,500);world.add(rim2);}
 for(const [color,power,pos]of [[0xf4f8ff,3.5,[600,-900,1200]],[0x04d9c4,1.8,[-800,200,650]],[0x808eff,2,[400,800,800]]]){const light=new THREE.DirectionalLight(color,power);light.position.set(...pos);world.add(light);}
 // WO-12: a studio environment, generated once. MeshStandardMaterial metals reflect nothing
 // without an environment map, which is the whole reason the machine read as flat grey plastic
 // (measured 2026-09-22 on the WO-11 build: machine region avg luminance 25/255, color spread
 // 7.9/255). Softbox panels in a dark room; no brand neons in the light itself — the neons stay
 // in the UI chrome where the brand puts them.
 const studioEnv=new THREE.Scene();studioEnv.background=new THREE.Color(0x1a1f24);
 // DoubleSide: the PMREM cube camera at the origin sees whichever way each plane happens to face,
 // and a single-sided plane facing away is culled from the env entirely (measured 2026-09-22: the
 // key softbox above the machine was invisible to fromScene). Panel sizes matter more than
 // intensity: a small panel at 1500 mm covers ~1% of the hemisphere, which is a point light, not
 // a softbox. These cover large solid angles so metals get real sheen AND parts keep diffuse light.
 const softbox=(color,intensity,w,h,x,y,z,rx,ry)=>{const p=new THREE.Mesh(new THREE.PlaneGeometry(w,h),new THREE.MeshBasicMaterial({color:new THREE.Color(color).multiplyScalar(intensity),side:THREE.DoubleSide}));p.position.set(x,y,z);p.rotation.set(rx,ry,0);studioEnv.add(p);};
 softbox(0xfff2df,8,4000,3000,0,-1600,1400,Math.PI/2,0);
 softbox(0xdfe8ee,2.5,3000,2000,2200,0,600,0,-Math.PI/2);
 softbox(0xcfd8dc,1.8,3000,2000,-2200,0,600,0,Math.PI/2);
 softbox(0x9fb4c0,1.2,2400,1200,0,2200,700,Math.PI,0);
 const pmrem=new THREE.PMREMGenerator(renderer);
 // near/far must cover the softbox distances (fromScene defaults to far=100, which would clip
 // every panel and bake a black room — measured 2026-09-22: scene luminance DROPPED to 22.6/255
 // with the default until the far plane covered the panels).
 world.environment=pmrem.fromScene(studioEnv,.04,1,5000).texture;
 const root=new THREE.Group();world.add(root);const orbit=new OrbitControls(camera,canvas);orbit.enableDamping=false;orbit.enableZoom=true;orbit.zoomToCursor=true;orbit.minDistance=2;orbit.maxDistance=8000;if(tileMode)orbit.enabled=false;orbit.addEventListener('change',()=>{updateCut();schedule();});
 const meshes=[],entries=new Map(),hidden=new Set(['covers']),hiddenParts=new Set(),variants=new Map();let frame=0,asset=null,sourceAssets=[],bounds=null,selected=null,outline=null,grid=null,explode=0,jobModel=null,jobName='',jobState=null,sourceData=null,lastMappedPose=null,lastMappedTs=null,lastSet=null,homedSeen='',poseSkips=0,lastJobDisplay=null,lastLiveVisual=null,mini=null;
 const tip=document.getElementById('scene-tip'),caption=document.querySelector('.scene-key');
 const metrics={loads:[],frames:[]};let colors={all:null,groups:{},parts:{}},colorKey='';
 // 2026-09-21: "my print bed is now a different colour." It did not change between the two
 // builds — the purple was already in the 2026-09-20 binary and the WO-09 evidence screenshot shows
 // the plate the same pink — what he is seeing is the heat glow washing a purple plate at 80 °C,
 // and no warm glow can read as heat over purple: orange on purple is pink at every temperature.
 // So the plate starts where his really is, dark textured PEI, and the glow takes it to red.
 // One constant: put '#7b4fd6' back here if he wants the purple.
 // WO-17 (2026-09-23), the owner: "my build plate's not purple anymore." That default is the whole
 // cause: with the printer off live() adds no heat tint, the idle key light is near white, and the
 // WO-12 sheet finish renders this same purple at hue 274 (measured on the WO-16 build with the
 // colour set by hand). His plate is purple again from Friday, so it is back, one named constant.
 // Heat still lerps it toward HOT_PLATE and lights it: the plate crop reads hue 274 idle and 319
 // (pink-magenta) at 80 °C, measured. The 0.45 plate share in live() is the knob for a redder glow.
 const PLATE_COLOR='#7b4fd6';
 const DEFAULT_GROUP_COLORS={bed:PLATE_COLOR,printed:'#8b9096'};
 const colorNames={printed:'Printed accents',frame:'Frame & rails',toolhead:'Toolhead',bed:'Build plate',motors:'Motors',electronics:'Electronics',covers:'Panels',fasteners:'Fasteners',belts:'Belts'};
 function paintGroup(m){const d=m.userData,n=d.name.toLowerCase(),c=m.material.color,hsl={};c.getHSL(hsl);if(/^bed\.(sheet|plate)$/.test(d.slot_id||''))return 'bed';return /rapid burner/.test(n)?'toolhead':/nema|stepper/.test(n)?'motors':/belt(?!.*drive)|gt2/.test(n)?'belts':/shcs|bhcs|fhcs|washer|nut|screw|bolt/.test(n)?'fasteners':['bed','covers','electronics'].includes(d.layer)?d.layer:hsl.s>.35?'printed':d.layer==='toolhead'?'toolhead':'frame';}
 // The colour a part sits at when it is cold and its fan is off. live() lerps away from it for heat
// and for fan speed, so it has to be recorded here, where the palette is decided, and the live
// visual has to be re-applied afterwards or a repaint would leave a hot plate looking cold.
// The hotend's own materials (NEXT review 2026-09-24, photo 94ce96d4): brass nozzle and core top, white
 // silicone sock. They sit ahead of the group defaults so the toolhead paint does not whiten the brass;
 // a colour the owner picks for the part or its group still wins.
 const SLOT_FINISH={'head.nozzle':['#c9a24b',.9,.32],'head.core':['#b8913e',.85,.38],'head.heatbreak':['#b8913e',.85,.38],'head.sock':['#e6e0d3',0,.88]};
 function applyColors(){for(const m of meshes){const d=m.userData,fin=SLOT_FINISH[d.slot_id];if(fin&&!colors.parts[d.id]&&!colors.groups[d.colorGroup]){m.material.metalness=fin[1];m.material.roughness=fin[2];}
  m.material.color.set(colors.parts[d.id]||colors.groups[d.colorGroup]||(fin&&fin[0])||DEFAULT_GROUP_COLORS[d.colorGroup]||colors.all||d.sourceColor);d.baseColor=m.material.color.getHex();}lastLiveVisual=null;mini?.refreshColor();schedule();}
 function initColors(){colorKey='stella-colors:v1:'+asset.source_sha256;for(const m of meshes){m.userData.colorGroup=paintGroup(m);m.userData.sourceColor='#'+m.material.color.getHexString();}try{const saved=JSON.parse(localStorage.getItem(colorKey));const valid=v=>typeof v==='string'&&/^#[0-9a-f]{6}$/i.test(v);if(saved&&typeof saved==='object'){colors.all=valid(saved.all)?saved.all:null;for(const [k,v]of Object.entries(saved.groups||{}))if(Object.hasOwn(colorNames,k)&&valid(v))colors.groups[k]=v;for(const [k,v]of Object.entries(saved.parts||{}))if(entries.has(k)&&valid(v))colors.parts[k]=v;}}catch{}applyColors();}
 function setColor(scope,color){if(!/^#[0-9a-f]{6}$/i.test(color))throw Error('Choose a valid color.');const next=structuredClone(colors);if(scope==='all'){colors={all:color,groups:{},parts:{}};}else if(scope.startsWith('group:')){const group=scope.slice(6);if(!Object.hasOwn(colorNames,group))throw Error('Unknown color group.');colors.groups[group]=color;for(const m of meshes)if(m.userData.colorGroup===group)delete colors.parts[m.userData.id];}else if(entries.has(scope))colors.parts[scope]=color;else throw Error('Select a component first.');try{localStorage.setItem(colorKey,JSON.stringify(colors));}catch{colors=next;throw Error('Browser storage unavailable; color was not saved.');}applyColors();}
 function resetColors(){localStorage.removeItem(colorKey);colors={all:null,groups:{},parts:{}};applyColors();}
 function note(s){if(caption)caption.textContent=s;}
 function resize(){const w=canvas.clientWidth,h=canvas.clientHeight;if(!w||!h)return;renderer.setSize(w,h,false);const aspect=w/h;if(bounds)camera.position.sub(orbit.target).multiplyScalar(Math.min(1,camera.aspect)/Math.min(1,aspect)).add(orbit.target);camera.aspect=aspect;camera.updateProjectionMatrix();schedule();}
 const resizer=new ResizeObserver(resize);resizer.observe(canvas);resize();
 function visible(m){const d=m.userData;return !d.replacedByRapid&&!d.focusHidden&&!hidden.has(d.layer)&&!hiddenParts.has(d.id)&&(!d.variantGroup||variants.get(d.variantGroup)===d.variantName);}
 // F04 (2026-10-19): a wall tile draws this machine (1,340 meshes) at most four times a second, and does not draw the parts smaller than 1.5% of the machine (fasteners: nobody sees them
 // on a tile and they were most of the 1,340 draw calls). Each draw used to allocate two vectors per mesh and was 30 to 50 ms, a visible hitch every half second on a wall of nine printers.
 function schedule(){if(frame||tileTimer)return;   // (tileTimer is -1 once the scene is disposed: nothing is drawn after that)
  if(tileMode){const wait=250-(performance.now()-lastTile);if(wait>0){tileTimer=setTimeout(()=>{tileTimer=0;schedule();},wait);return;}}
  frame=requestAnimationFrame(()=>{frame=0;if(!canvas.clientWidth||!canvas.clientHeight)return;
   const bc=bounds?bounds.getCenter(_bc):_bc.set(0,0,0),k=explode/500;if(tileMode&&tileMin<0)tileMin=(bounds?bounds.getSize(_sz).length():800)*0.015;
   for(const m of meshes){const d=m.userData;m.visible=visible(m);
    if(tileMode&&m.visible){if(d.tiny===undefined){const bb=m.geometry.boundingBox;d.tiny=bb?bb.getSize(_sz).length()<tileMin:false;}if(d.tiny)m.visible=false;}
    if(k){_sp.copy(d.center).sub(bc).multiplyScalar(k);if(d.motionOffset)m.position.copy(d.motionOffset).add(_sp);else m.position.copy(_sp);}else if(d.motionOffset)m.position.copy(d.motionOffset);else m.position.set(0,0,0);}if(outline&&selected)outline.box.setFromObject(selected);const start=performance.now();renderer.render(world,camera);if(tileMode)lastTile=performance.now();metrics.frames.push(performance.now()-start);if(metrics.frames.length>300)metrics.frames.shift();document.dispatchEvent(new CustomEvent('stella-camera-change'));
  const anchor=heatBlock.find(m=>m.userData.slot_id==='head.nozzle')||heatBlock[0];
  if(anchor){const p=anchor.geometry.boundingBox.getCenter(new THREE.Vector3()).add(anchor.position).project(camera);
   const r=canvas.getBoundingClientRect();
   document.dispatchEvent(new CustomEvent('stella-track',{detail:{x:(p.x*.5+.5)*r.width,y:(-p.y*.5+.5)*r.height,on:p.z<1&&Math.abs(p.x)<1.1&&Math.abs(p.y)<1.1}}));}
  });}
 function fit(objects=meshes,view=null){const box=empty.clone();objects.filter(m=>m.visible).forEach(m=>box.expandByObject(m));if(box.isEmpty())return;const center=box.getCenter(new THREE.Vector3()),size=box.getSize(new THREE.Vector3());const dir=view?new THREE.Vector3(...(nextStage&&view==='iso'?[.85,-1,.42]:CAMERA_VIEWS[view])):camera.position.clone().sub(orbit.target);dir.normalize();const distance=size.length()*(tileMode?.43:nextStage?.5*Math.max(1,Math.sqrt(1000/Math.max(canvas.clientHeight,1))):.44)/Math.tan(camera.fov*Math.PI/360)/Math.min(1,camera.aspect);// NEXT: aim a little below the middle so the machine sits above the dock instead of behind it.
 if(nextStage&&!tileMode&&objects===meshes)center.z-=size.z*(camera.aspect>1?.16:.04);orbit.target.copy(center);camera.position.copy(center).addScaledVector(dir,distance);camera.far=distance*20;camera.updateProjectionMatrix();orbit.update();schedule();}
 function pick(e){const r=canvas.getBoundingClientRect(),ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2((e.clientX-r.left)/r.width*2-1,1-(e.clientY-r.top)/r.height*2),camera);return ray.intersectObjects(meshes.filter(m=>m.visible&&m.material.opacity>.1&&!(nextStage&&INERT.has(m.userData.colorGroup))),false)[0]?.object;}
 // NEXT (2026-09-25): only parts worth opening answer a click or hover; extrusions, screws,
 // belts and panels are scenery.
 const INERT=new Set(['fasteners','frame','belts','covers']);
 const cutPlane=new THREE.Plane(new THREE.Vector3(0,0,-1),1e6);let cutTarget=null;
 let spotMesh=null;
 function clearFocus(){for(const m of meshes){m.userData.focusHidden=false;m.material.transparent=Boolean(m.userData.panelTransparent);m.material.opacity=m.userData.panelTransparent?.15:1;m.material.depthWrite=!m.userData.panelTransparent;}
  for(const m of meshes){const d=m.userData;if(d.dimmed){m.material.color.setHex(d.baseColor);d.dimmed=false;}}
  if(spotMesh){spotMesh.material.emissive.setHex(0);spotMesh.material.emissiveIntensity=1;spotMesh=null;}
  cutTarget=null;renderer.clippingPlanes=[];canvas.dataset.inspection='whole';document.dispatchEvent(new CustomEvent('stella-inspection',{detail:{mode:'whole'}}));}
 // While focused, everything between the camera and the part is cut away, so zooming in opens the
 // machine up instead of putting the camera inside it.
 function updateCut(){if(!cutTarget)return;const n=camera.position.clone().sub(cutTarget.centre).normalize();
  cutPlane.normal.copy(n.clone().negate());cutPlane.constant=-cutPlane.normal.dot(cutTarget.centre)+cutTarget.radius;renderer.clippingPlanes=[cutPlane];}
 function focusPart(m){clearFocus();hidden.delete(m.userData.layer);hiddenParts.delete(m.userData.id);if(m.userData.variantGroup)variants.set(m.userData.variantGroup,m.userData.variantName);m.visible=visible(m);const center=m.geometry.boundingBox.getCenter(new THREE.Vector3()),radius=Math.max(100,m.geometry.boundingBox.getSize(new THREE.Vector3()).length()*2);for(const other of meshes)if(other!==m){const near=other.userData.center.distanceTo(center)<=radius;other.userData.focusHidden=!near;other.material.transparent=true;other.material.opacity=near?.72:.035;other.material.depthWrite=near;}canvas.dataset.inspection='focus';cutTarget={centre:center.clone(),radius:Math.max(12,m.geometry.boundingBox.getSize(new THREE.Vector3()).length()*.62)};updateCut();
  document.dispatchEvent(new CustomEvent('stella-inspection',{detail:{mode:'focus',name:m.userData.name}}));fit([m],'iso');}
 // Highlight in place (2026-09-28: "Isolate showed the motor alone with no sense of where it sits",
 // "Show the motor does not show the motor"). The part is lit and framed with the machine around it and
 // every other part is dimmed to near black: opaque, so there is no haze (a first try faded the rest to
 // 7% opacity; a few hundred overlapping parts stacked into a grey wall). Nothing is hidden, nothing
 // moves, and parts between the camera and the part are cut away as in focus mode, so a part inside the
 // frame is never behind something. Show all or Reset restores the default view. `flash` pulses it three
 // times. The case panels stay out: they are off in this program unless View turns them on.
 function spotlightMesh(m,{flash=true,minSpan=520}={}){
  select(m.userData.id);clearFocus();
  hiddenParts.delete(m.userData.id);hidden.delete(m.userData.layer);m.visible=visible(m);
  for(const o of meshes){const d=o.userData;if(o===m)continue;if(d.layer==='covers'){d.focusHidden=true;continue;}
   o.material.color.setHex(d.baseColor??o.material.color.getHex()).multiplyScalar(.13);d.dimmed=true;}
  m.material.emissive.setHex(0x04d9c4);m.material.emissiveIntensity=.38;spotMesh=m;lastLiveVisual=null;
  canvas.dataset.inspection='spot';document.dispatchEvent(new CustomEvent('stella-inspection',{detail:{mode:'spot',name:m.userData.name}}));
  const box=new THREE.Box3().setFromObject(m),c=box.getCenter(new THREE.Vector3()),size=box.getSize(new THREE.Vector3()),span=Math.max(size.length(),minSpan);
  const dir=new THREE.Vector3(...(nextStage?[.85,-1,.42]:CAMERA_VIEWS.iso)).normalize();
  const distance=span*(tileMode?.43:nextStage?.5*Math.max(1,Math.sqrt(1000/Math.max(canvas.clientHeight,1))):.44)/Math.tan(camera.fov*Math.PI/360)/Math.min(1,camera.aspect);
  orbit.target.copy(c);camera.position.copy(c).addScaledVector(dir,distance);camera.far=Math.max(camera.far,distance*20);camera.updateProjectionMatrix();orbit.update();
  cutTarget={centre:c.clone(),radius:Math.max(150,size.length()*.62)};updateCut();schedule();
  if(flash){const mat=m.material,t0=performance.now();
   const tick=now=>{const t=(now-t0)/1800;if(t>=1||spotMesh!==m){if(spotMesh===m)mat.emissiveIntensity=.38;schedule();return;}
    mat.emissiveIntensity=.38+.9*Math.max(0,Math.sin(t*Math.PI*6));schedule();requestAnimationFrame(tick);};requestAnimationFrame(tick);}
 }
 let down=null,hoverTime=0;
 canvas.addEventListener('pointerdown',e=>{down=e.button===0?[e.clientX,e.clientY]:null;if(tip)tip.hidden=true;});
 canvas.addEventListener('pointerup',e=>{if(e.button===0&&down&&Math.hypot(e.clientX-down[0],e.clientY-down[1])<4){const m=pick(e);if(m&&onSelect){select(m.userData.id);onSelect(m.userData.id);}}down=null;});
 canvas.addEventListener('pointermove',e=>{if(e.buttons||performance.now()-hoverTime<90)return;hoverTime=performance.now();const m=pick(e);canvas.style.cursor=m?'pointer':'grab';if(tip){tip.hidden=!m;if(m){const r=canvas.getBoundingClientRect();tip.textContent=m.userData.name;tip.style.left=Math.min(r.width-220,Math.max(10,e.clientX-r.left+14))+'px';tip.style.top=Math.max(10,e.clientY-r.top-36)+'px';}}});
 canvas.addEventListener('pointerleave',()=>{if(tip)tip.hidden=true;});
 canvas.addEventListener('keydown',e=>{if(e.key==='0'||e.key==='Escape')api.reset();else if(['+','=','-'].includes(e.key)){camera.position.sub(orbit.target).multiplyScalar(e.key==='-'?1.15:1/1.15).add(orbit.target);orbit.update();schedule();}else return;e.preventDefault();});
 canvas.addEventListener('dblclick',()=>fit(selected?[selected]:meshes));
 function zoom(scale){const delta=camera.position.clone().sub(orbit.target),distance=THREE.MathUtils.clamp(delta.length()*scale,orbit.minDistance,orbit.maxDistance);camera.position.copy(orbit.target).add(delta.setLength(distance));orbit.update();schedule();}
 function orbitBy(dx,dy){const delta=camera.position.clone().sub(orbit.target);delta.applyAxisAngle(new THREE.Vector3(0,0,1),-dx*.008);const right=new THREE.Vector3().crossVectors(delta,camera.up).normalize(),next=delta.clone().applyAxisAngle(right,-dy*.008);if(Math.abs(next.clone().normalize().dot(camera.up))<.995)delta.copy(next);camera.position.copy(orbit.target).add(delta);orbit.update();schedule();}
 // WO-17 (2026-09-23), the owner: the word FRONT showed on the left and right of the view cube. Labels
 // were upright screen text at their face's centre, so a face turned toward edge-on had a label
 // wider than itself spilling onto its neighbours (up to 10.6 px outside its own face, measured).
 // Each label now lies in its face's plane: `label` is the SVG matrix taking the text's right and
 // down onto that face (the directions someone looking square at it reads), so it foreshortens
 // with the face and stays on it, and `facing` lets the HUD fade it as the face turns away.
 const CUBE_TEXT={right:[[0,1,0],[0,0,-1]],left:[[0,-1,0],[0,0,-1]],rear:[[-1,0,0],[0,0,-1]],front:[[1,0,0],[0,0,-1]],top:[[1,0,0],[0,-1,0]],bottom:[[-1,0,0],[0,-1,0]]};
 function orientation(){const inverse=camera.quaternion.clone().invert(),toward=camera.position.clone().sub(orbit.target).normalize();return Object.entries({right:[1,0,0],left:[-1,0,0],rear:[0,1,0],front:[0,-1,0],top:[0,0,1],bottom:[0,0,-1]}).map(([name,normal])=>{const n=new THREE.Vector3(...normal),u=new THREE.Vector3(normal[2]?1:0,normal[0]?1:0,normal[1]?1:0),v=new THREE.Vector3().crossVectors(n,u);const points=[[-1,-1],[1,-1],[1,1],[-1,1]].map(([a,b])=>n.clone().addScaledVector(u,a).addScaledVector(v,b).multiplyScalar(27).applyQuaternion(inverse)).map(p=>[64+p.x,60-p.y]);const center=n.clone().multiplyScalar(27).applyQuaternion(inverse);const [r,d]=CUBE_TEXT[name].map(a=>new THREE.Vector3(...a).applyQuaternion(inverse));return {name,visible:n.dot(toward)>.005,facing:n.dot(toward),points,center:[64+center.x,60-center.y],label:[r.x,-r.y,d.x,-d.y,64+center.x,60-center.y]};});}
 // NEXT: select(null) is nothing selected. It used to match the first mesh whose slot_id is null (a screw)
 // and, because set() calls it every tick, replaced whatever was selected and cleared its focus within
 // 32 ms: why "Show the motor" and Isolate never stuck (found 2026-10-12). The classic HUD keeps its behaviour.
 function select(id){const next=nextStage&&id==null?null:(id==='head'?entries.get('stella-rapid-bltouch-cleaned:root:0'):null)||entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id)||null;if(selected===next)return;clearFocus();selected=next;for(const m of meshes)m.material.emissive.setHex(0);lastLiveVisual=null;if(outline){world.remove(outline);outline.dispose();outline=null;}if(selected){outline=new THREE.Box3Helper(new THREE.Box3().setFromObject(selected),0x04d9c4);world.add(outline);}document.dispatchEvent(new CustomEvent('stella-selection-change'));schedule();}
 function classification(name){const s=name.toLowerCase();return /deck panel/.test(s)?'deck':/panel|door|acrylic/.test(s)?'covers':/z motor|z.drive|z_drive/.test(s)?'z-drives':/bed|build.plate|heater/.test(s)?'bed':/extruder|toolhead|afterburner|hotend/.test(s)?'toolhead':/gantry|x.axis|y.axis/.test(s)?'gantry':/electronics|controller|power.supply|raspberry/.test(s)?'electronics':/deck/.test(s)?'deck':'frame';}
 async function load(){
  note('Loading genuine printer assembly…');const start=performance.now();
  try{const r=await fetch(apiUrl('/api/assets'));if(!r.ok)throw Error('Source library unavailable');const library=await r.json();sourceAssets=library.assets;asset=library.assets.find(a=>a.id==='stella-printer')||library.assets.find(a=>/^hartk-voron-24r2-\d{3}-reference$/.test(a.id))||library.assets.find(a=>a.id==='voron-r2-master-reference');if(!asset)throw Error('Full printer CAD is not imported yet. No substitute geometry is shown.');
   // the same machine mountRapidBurner dresses (its two hashes): its parts download now, with the frame
   if(asset.source_sha256==='abeef1bb9bcae90a9ed579e901423d2f564143eae1e39c2ee5b96b2a74ec467c'&&sourceAssets.find(a=>a.id==='stella-rapid-bltouch-cleaned')?.source_sha256==='0172e800f8606e99cf5fc1555457de8931f339ae3f6547c1630fa3b1b49a0329')for(const id of TOOLHEAD_PARTS){const a=sourceAssets.find(x=>x.id===id);if(a)partData(a).catch(()=>{});}
   const data=await assetJson(asset,'Printer geometry unavailable');sourceData=data;
   const geometry=data.meshes.map(raw=>{const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(raw.attributes.position.array,3));g.setIndex(raw.index.array);if(raw.attributes.normal?.array?.length===raw.attributes.position.array.length)g.setAttribute('normal',new THREE.Float32BufferAttribute(raw.attributes.normal.array,3));else g.computeVertexNormals();g.computeBoundingBox();return g;});
   function walk(node,path,names){for(const [i,index]of (node.meshes||[]).entries()){const raw=data.meshes[index],g=geometry[index];if(!g)throw Error('Missing source mesh');const rawName=raw.name||node.name||'Component',name=/^=>|^\[/.test(rawName)?[...names,node.name].filter(n=>n&&!/^=>|^\[/.test(n)).at(-1)||'Source component':rawName,full=[...names,node.name,name].filter(Boolean).join(' / '),map=raw.mapping||{},id=node.instance_ids?.[i]||`${asset.id}:${path}:${index}`;const color=raw.color?new THREE.Color().setRGB(...raw.color,THREE.SRGBColorSpace):new THREE.Color(0x667380);const material=new THREE.MeshStandardMaterial({color,metalness:.35,roughness:.5,side:THREE.DoubleSide});const m=new THREE.Mesh(g,material);const ancestry=[...names,node.name];let variantGroup=null,variantName=null;for(const group of ['Toolheads','Controllers']){const at=ancestry.findIndex(n=>n?.startsWith(group));if(at>=0&&ancestry[at+1]){variantGroup=group;variantName=ancestry[at+1];if(!variants.has(group))variants.set(group,variantName);}}m.userData={id,record_id:instanceRecordId(id),name,full,asset,variantGroup,variantName,slot_id:map.slot_id||null,layer:['covers','z-drives','bed','toolhead','gantry','electronics','deck','frame'].includes(map.layer)?map.layer:classification(full),motion:map.motion||'fixed',center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3()};root.add(m);meshes.push(m);entries.set(id,m);}for(const [i,c]of (node.children||[]).entries())walk(c,path+'/'+i,[...names,node.name]);}
       walk(data.root,'root',[]);
    // WO-12 material pass: same colours, honest surfaces. Without this every part shares one
    // mid-grey metallic-plastic finish (.35/.5); with it, aluminium extrusions get their sheen,
    // rails read as chrome, belts as rubber, panels stay matte. It runs BEFORE mountRapidBurner,
    // so that function's own explicit paints (heatsink, screws, black printed parts, blowers)
    // still win, and it touches only metalness/roughness — colour stays exactly where the
    // palette (and our own recolouring) puts it.
    const familyFinish=d=>{const n=(d.name||'').toLowerCase();
     if(/hfsb|extrusion/.test(n))return [.85,.38];
     if(/mgn\d|linear rail|rod|shaft/.test(n))return [1,.18];
     if(/belt(?!.*drive)/.test(n))return [0,.95];
     if(/shcs|bhcs|fhcs|screw|bolt|washer|nut/.test(n))return [.9,.25];
     if(/panel|door|acrylic|cover/.test(n)||d.layer==='covers')return [.05,.8];
     if(/^bed\.(sheet|plate)$/.test(d.slot_id||''))return [.55,.35];
     return null;};
    for(const m of meshes){const f=familyFinish(m.userData);if(f){m.material.metalness=f[0];m.material.roughness=f[1];}}
    await mountRapidBurner();establishMotion();initColors();const sheet=meshes.find(m=>m.userData.name==='Flex Sheet:1');if(sheet&&sourceData.printer_frame?.bed_center){sourceData.printer_frame.bed_center[2]=sheet.geometry.boundingBox.max.z;sourceData.printer_frame.build_surface_basis='Top bound of named source Flex Sheet:1; plate XY center retained, front sheet tab excluded';}bounds=new THREE.Box3().setFromObject(root);drawUmbilical();const size=bounds.getSize(new THREE.Vector3()),center=bounds.getCenter(new THREE.Vector3());   grid=new THREE.GridHelper(Math.max(size.x,size.y)*1.15,Math.max(20,Math.round(Math.max(size.x,size.y)*1.15/10)),0x174448,0x152028);grid.rotation.x=Math.PI/2;grid.position.set(center.x,center.y,bounds.min.z-4);world.add(grid);
    if(nextStage){// a soft contact shadow under the frame and an accent pool of light around it
     const disc=(stops,r,z)=>{const c=document.createElement('canvas');c.width=c.height=256;const g=c.getContext('2d'),gr=g.createRadialGradient(128,128,0,128,128,128);for(const [o,col]of stops)gr.addColorStop(o,col);g.fillStyle=gr;g.fillRect(0,0,256,256);
      const t=new THREE.CanvasTexture(c);t.colorSpace=THREE.SRGBColorSpace;const m=new THREE.Mesh(new THREE.PlaneGeometry(r*2,r*2),new THREE.MeshBasicMaterial({map:t,transparent:true,depthWrite:false,toneMapped:false}));m.position.set(center.x,center.y,z);m.renderOrder=-1;m.raycast=()=>{};world.add(m);return m;};
     const R=Math.max(size.x,size.y);disc([[0,'rgba(4,217,196,.42)'],[.35,'rgba(4,217,196,.16)'],[.7,'rgba(90,120,255,.05)'],[1,'rgba(4,217,196,0)']],R*1.35,bounds.min.z-5);
     disc([[0,'rgba(0,0,0,.72)'],[.55,'rgba(0,0,0,.45)'],[1,'rgba(0,0,0,0)']],R*.62,bounds.min.z-3);
     }theme(document.body.dataset.theme||'dark');for(const m of meshes)m.visible=visible(m);fit(meshes,'iso');metrics.loads.push({id:asset.id,ms:performance.now()-start,meshes:meshes.length,triangles:meshes.reduce((total,m)=>total+m.geometry.index.count/3,0),bounds:size.toArray()});canvas.dataset.renderer='source-cad';canvas.dataset.source=asset.source_sha256;note(`${meshes.length.toLocaleString()} CAD components · ${canvas.dataset.customToolhead?'toolhead as installed (placement approximate)':'reference toolhead'}${missingParts.length?` · MISSING INSTALLED PART${missingParts.length>1?'S':''}: ${missingParts.join(', ')} — not in the managed library`:''}`);document.dispatchEvent(new CustomEvent('stella-geometry-ready'));
    // WO-20 (2026-09-23), finding 2 of WO-19: a replay or rewind link opened cold set the pose before
    // the model and its motion datum existed, so the head sat at its rest pose until the cursor moved.
    // The last pose asked for is applied again now that the model can take it.
    if(lastSet)set(...lastSet);
  }catch(e){note(e.message);canvas.dataset.renderer='source-unavailable';canvas.dataset.error=e.message;}
 }
 // 2026-09-21: an axis that is not in homed_axes carries no information, so it holds its last
 // drawn value rather than taking the reported one; before anything has been drawn it holds the CAD
 // rest pose.
 function set(snapshot,id){lastSet=[snapshot,id];if(!(nextStage&&id==null))select(id);const raw=snapshot?.point?.position,datum=sourceData?.printer_frame?.source_pose,homedAxes=snapshot?.point?.homed||'';const pos=raw&&datum?raw.map((v,i)=>homedAxes.includes('xyz'[i])?v:(lastMappedPose?lastMappedPose[i]:datum[i])):raw;canvas.dataset.sampleTs=String(snapshot?.point?.ts??'');
  // 2026-09-21: "when I pressed X− it moved the whole print head way to the side really quickly
  // and back." The spike is in the data, not the render. The jog is audited at 1789999549.76 intent
  // / 1789999550.00 result as G91 / G1 X-10.00 F6000 / G90, and the recorded sample at
  // 1789999550.244 carries motion_report.live_position [0,0,0] for exactly one 253 ms tick, between
  // 330.000 and 320.000, with nozzle and bed continuous either side — so it is Klipper's live
  // position and not an empty frame. Drawn verbatim it threw the head 330 mm to the origin and back.
  // A pose needing more travel than max_velocity allows in the elapsed sample time did not happen on
  // the machine, so it is held rather than drawn. Scrubbing the recording has a large dt, so a
  // deliberate seek still lands.
  const ts=snapshot?.point?.ts,vmax=snapshot?.point?.max_velocity,vzmax=snapshot?.point?.max_z_velocity,limits=snapshot?.point?.limits;
  // 2026-09-21, second look: an axis that has just become homed is reported in a coordinate
  // frame the drawn pose knows nothing about (G28 sets Z to 0 and hops, then force-sets 464), so the
  // reach test below cannot apply to it. The first pose that is inside the machine after the axis is
  // gained re-anchors that axis; the flag survives every refusal, so it is the first *drawable*
  // sample that lands, not the force position.
  // 2026-09-21, watching a G28: "the whole gantry was drawn above the printer." The recorded
  // sample at 1790021428.895 reports homed_axes "xyz" and Z 464.0195 on BOTH motion_report
  // .live_position and toolhead.position, against axis_maximum 310 — Klipper force-sets the
  // kinematic position to position_endstop + 1.5 * (position_max - position_endstop) for a homing
  // move and then drives down until the probe triggers. So this is not an unhomed axis; it is a
  // homed axis reported outside the machine. The head never went 464 mm up, so the sample is held
  // exactly like the [0,0,0] spike below. 1 mm of slack keeps a legitimate move to the very end of
  // travel drawable.
  const rebased=snapshot?.point?.rebased||'',gained=[0,1,2].filter(i=>homedAxes.includes('xyz'[i])&&(!homedSeen.includes('xyz'[i])||rebased.includes('xyz'[i])));
  if(pos?.every(Number.isFinite)&&!insideTravel(pos,limits)){
   poseSkips++;canvas.dataset.poseSkips=String(poseSkips);
   canvas.dataset.poseSkipped=JSON.stringify({ts,position:pos,reason:'outside machine travel',min:limits?.min,max:limits?.max});return;}
  // Z has its own speed limit on this machine (max_z_velocity 20 against max_velocity 300), so one
  // reach for all three axes gave Z a 15x free pass: the 441 mm homing jump needed only 0.1 s of XY
  // reach to slip through. Split, and let the clock advance on a refusal too — measuring from the
  // last *accepted* sample widened the window by one tick for every sample refused, which is how a
  // rejected burst ended in a teleport.
  if(pos?.every(Number.isFinite)&&lastMappedPose&&Number.isFinite(ts)&&Number.isFinite(lastMappedTs)){
   const dt=Math.abs(ts-lastMappedTs),reach=dt*(Number.isFinite(vmax)&&vmax>0?vmax:300)*1.5+2,reachZ=dt*(Number.isFinite(vzmax)&&vzmax>0?vzmax:20)*1.5+2;
   const travel=gained.includes(0)||gained.includes(1)?0:Math.hypot(pos[0]-lastMappedPose[0],pos[1]-lastMappedPose[1]);
   const travelZ=gained.includes(2)?0:Math.abs(pos[2]-lastMappedPose[2]);
   // 2026-09-24: a refused pose must not leave the head frozen. Drawn every animation frame,
   // the reach is a few mm, so once one frame was refused every later frame was too far from the
   // stuck pose and the head stayed put until it jumped. A single spike lasts one tick (253 ms);
   // a position that stays refused for 0.75 s of sample time is where the machine really is.
   if(travel<=reach&&travelZ<=reachZ)refusedSince=null;else if(refusedSince===null)refusedSince=ts;
   if((travel>reach||travelZ>reachZ)&&ts-refusedSince<0.75){poseSkips++;canvas.dataset.poseSkips=String(poseSkips);lastMappedTs=ts;
    canvas.dataset.poseSkipped=JSON.stringify({ts,position:pos,travel_mm:Number(travel.toFixed(3)),travel_z_mm:Number(travelZ.toFixed(3)),reach_mm:Number(reach.toFixed(3)),reach_z_mm:Number(reachZ.toFixed(3)),dt_s:Number(dt.toFixed(3))});return;}}
  // The clock advances on every accepted sample, not only on the ones that moved: the head sat at
  // 330 for 30 s before the bad sample, and a reach measured from the last *move* would have been
  // wide enough to let it through.
  const gap=pos?.every(Number.isFinite)&&Number.isFinite(ts)&&Number.isFinite(lastMappedTs)?(ts-lastMappedTs)*1000:0;
  if(pos?.every(Number.isFinite)&&Number.isFinite(ts))lastMappedTs=ts;
  if(pos?.every(Number.isFinite))homedSeen=homedAxes;
  // Motion is allowed only with a documented source datum and explicit per-mesh joints.
  if(datum&&pos?.every(Number.isFinite)&&(!lastMappedPose||pos.some((v,i)=>v!==lastMappedPose[i]))){lastMappedPose=pos.slice();tweenTo(pos.slice(),gap);}
  canvas.dataset.pose=JSON.stringify({position:pos||null,mapped:Boolean(datum),basis:datum?'Mapped source joints':'Static source geometry; motion alignment not established'});
 }
 // 2026-09-21, watching the max-flow tower: the motion is "not far behind, but a little janky".
 // The recorder ticks every 250 ms and the model used to jump a whole tick at a time. The accepted
 // pose is now a target and every animation frame walks the drawn pose toward it over the gap
 // between the two samples, so the head glides the way the machine does. Linear, because between two
 // samples the machine is holding a velocity — easing would put a stop in the middle of a straight
 // line. Both guards above run first, so a pose that never happened is never a tween target, and a
 // scrub (huge gap) lands at once rather than crawling there.
 let chainPath=null,refusedSince=null;const X_AXIS=new THREE.Vector3(1,0,0),Y_AXIS=new THREE.Vector3(0,1,0);
 // The rear IGUS Z chain (2026-09-24, "the cable chain in the back isn't moving"): it stands in
 // the X-Z plane, its left run bolted to the frame at the bottom, its right run's end riding on the
 // gantry. Arc position s runs up the fixed run, over the bend (centre x wc, height uc, radius R)
 // and down the moving run; the bend rises half as far as the gantry. Returns [x, z, angle].
 let zChain=null;function zChainAt(s,uc){const {wc,R}=zChain;if(s<=uc)return [wc-R,s,0];if(s<=uc+Math.PI*R){const f=(s-uc)/R;return [wc-R*Math.cos(f),uc+R*Math.sin(f),f];}return [wc+R,uc-(s-uc-Math.PI*R),Math.PI];}
 // A point on the chain's centre line at arc position s with the bend centred at y=yc, and the
 // rolled angle there (0 on the lower run, pi on the upper).
 function chainAt(s,yc){const {zc,R}=chainPath,bs=-yc;if(s<=bs)return [-s,zc-R,0];if(s<=bs+Math.PI*R){const f=(s-bs)/R;return [yc-R*Math.sin(f),zc-R*Math.cos(f),f];}return [yc+(s-bs-Math.PI*R),zc+R,Math.PI];}
 let posePlayed=null,poseFrom=null,poseTarget=null,poseT0=0,poseDur=0,poseRaf=0;
 // The carriage's belt clamps in source coordinates (the MGN12H block the head is bolted to), widened
 // to take in the clamp loops; everything between moves whole with the head.
 let carriageX=null;
 function strandBelts(dx){const belts=meshes.filter(m=>m.userData.strand);if(!belts.length)return;
  if(!carriageX){const blk=meshes.filter(m=>/^PRODUCT_NAME_/.test(m.userData.name||'')&&/X Axis \(MGN12\)/.test(m.userData.full||''));
   if(blk.length){const b=blk.reduce((a,m)=>a.union(m.geometry.boundingBox.clone()),blk[0].geometry.boundingBox.clone());carriageX=[b.min.x-24,b.max.x+24];}else carriageX=[80,175];}
  const [cL,cR]=carriageX;
  for(const m of belts){const st=m.userData.strand,a=m.geometry.attributes.position,out=a.array,o=st.orig,d=Math.max(st.x0-cL+5,Math.min(st.x1-cR-5,dx));
   for(let i=0;i<o.length;i+=3){const x=o[i],y=o[i+1];if(y<st.y0||y>st.y1){out[i]=x;continue;}
    out[i]=x<cL?st.x0+(x-st.x0)*((cL+d-st.x0)/(cL-st.x0)):x>cR?st.x1-(st.x1-x)*((st.x1-cR-d)/(st.x1-cR)):x+d;}
   a.needsUpdate=true;}}
 function applyPose(pos){const datum=sourceData?.printer_frame?.source_pose;if(!datum)return;posePlayed=pos;
  for(const m of meshes){const motion=m.userData.motion;
   if(m.userData.turning)continue;
   if(motion==='zchain'&&zChain){const dz=pos[2]-datum[2],z=zChain,ucR=z.uc+z.extra/2+dz/2,a=zChainAt(m.userData.chainS0,z.uc),b=zChainAt(m.userData.chainS,ucR);
    m.quaternion.setFromAxisAngle(Y_AXIS,b[2]-a[2]);const c0=new THREE.Vector3(a[0],0,a[1]).applyQuaternion(m.quaternion);m.userData.motionOffset.set(b[0]-c0.x,-c0.y,b[1]-c0.z);continue;}
   if(motion==='chain'&&chainPath){const dy=pos[1]-datum[1],dz=pos[2]-datum[2],shape=Math.max(chainPath.dyMin,dy),slide=dy-shape;
    // dyMin is only a guard now: the drawn chain is long enough for the whole travel.
    const a=chainAt(m.userData.chainS0,chainPath.yc),b=chainAt(m.userData.chainS,chainPath.yc-chainPath.extra/2+shape/2);
    m.quaternion.setFromAxisAngle(X_AXIS,-(b[2]-a[2]));const c0=new THREE.Vector3(0,a[0],a[1]).applyQuaternion(m.quaternion);
    m.userData.motionOffset.set(0,b[0]+slide-c0.y,b[1]+dz-c0.z);continue;}
   m.userData.motionOffset.set(motion==='toolhead'?pos[0]-datum[0]:0,['toolhead','x_axis'].includes(motion)?pos[1]-datum[1]:0,['toolhead','x_axis','gantry'].includes(motion)?pos[2]-datum[2]:0);}
  strandBelts(pos[0]-datum[0]);
  drawUmbilical();canvas.dataset.drawnPose=JSON.stringify(pos.map(v=>Number(v.toFixed(3))));schedule();}
 function poseFrame(now){poseRaf=0;if(!poseTarget||!poseFrom)return;const k=poseDur>0?Math.min(1,(now-poseT0)/poseDur):1;
  applyPose(k>=1?poseTarget.slice():poseFrom.map((v,i)=>v+(poseTarget[i]-v)*k));
  if(k<1)poseRaf=requestAnimationFrame(poseFrame);else{poseFrom=null;poseTarget=null;}}
 function tweenTo(pos,ms){if(!posePlayed||!(ms>16)||ms>1500){poseFrom=null;poseTarget=null;applyPose(pos);return;}
  poseFrom=posePlayed.slice();poseTarget=pos;poseT0=performance.now();poseDur=ms;
  if(!poseRaf)poseRaf=requestAnimationFrame(poseFrame);}
 function jobUpdate(job){jobState=job;const d=jobDisplay(job,jobName),signature=JSON.stringify(d);if(jobModel&&signature!==lastJobDisplay){lastJobDisplay=signature;const [ghost,fill]=jobModel.children;ghost.material.color.setHex(d.color);fill.material.color.setHex(d.color);fill.visible=d.progress!==null;const box=new THREE.Box3().setFromObject(ghost),height=box.max.z-box.min.z;fill.material.clippingPlanes[0].constant=box.min.z+height*(d.progress||0);ghost.material.opacity=d.matched?.12:.55;schedule();}return d;}
 async function loadJob(file,units='mm'){
  if(!file||file.size>32*1024*1024)throw Error('Choose an STL smaller than 32 MB.');if(!/\.stl$/i.test(file.name))throw Error('Choose the source STL for this preview. G-code remains a separate print file.');
  const g=new STLLoader().parse(await file.arrayBuffer());const a=g.attributes.position;if(!a||a.count>1500000||!Array.from(a.array).every(Number.isFinite)){g.dispose();throw Error('STL is invalid or exceeds 500,000 triangles.');}if(units==='inch')g.scale(25.4,25.4,25.4);g.computeBoundingBox();const size=g.boundingBox.getSize(new THREE.Vector3());if(size.x<=0||size.y<=0||size.z<=0||size.x>350||size.y>350||size.z>350){g.dispose();throw Error('Model must fit the 350 × 350 × 350 mm build volume; confirm source units and orientation.');}
  const center=g.boundingBox.getCenter(new THREE.Vector3());g.translate(-center.x,-center.y,-g.boundingBox.min.z);if(jobModel){world.remove(jobModel);jobModel.children[0].geometry.dispose();for(const m of jobModel.children)m.material.dispose();}
  lastJobDisplay=null;jobModel=new THREE.Group();const ghost=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x819ba8,transparent:true,opacity:.55,depthWrite:false,side:THREE.DoubleSide}));const fill=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x04d9c4,roughness:.45,clippingPlanes:[new THREE.Plane(new THREE.Vector3(0,0,-1),0)],side:THREE.DoubleSide}));fill.visible=false;jobModel.add(ghost,fill);const bed=sourceData?.printer_frame?.bed_center;if(!bed){jobModel.children.forEach(m=>m.material.dispose());g.dispose();jobModel=null;throw Error('Printer CAD needs a verified build-plate datum before placing a job.');}jobModel.position.set(...bed);world.add(jobModel);jobName='';jobUpdate(jobState);return {name:file.name,size:size.toArray(),triangles:a.count/3};
 }
 // Real print toolpaths (L6). The parsed G-code is Klipper coordinates, so (175,175) sits on the bed
 // centre datum. Two line objects share one geometry: `done` draws the first N segments with the
 // printed colour, `todo` the rest, and progress only moves the draw ranges.
 // 2026-09-21, watching the YOLO flow plate: "black lines all over the place… we don't need
 // that." Travel moves are not drawn at all now — only extruding segments reach the buffer — and
 // "I'd like for the color to change progressively so that we know what's beginning and what's
 // complete", plus, on the same look, he could not tell the old done-teal from the ghost while
 // printing. So the printed portion is a hot ramp: the last HOT_RUN segments run bright orange back
 // to ember, everything printed before that stays ember, and what is still to come is a dim ghost.
 let toolpath=null;
 // The ramp is a fixed share of the file rather than a fixed segment count, so it reads the same on
 // a 40,000-segment flow plate as on a 90,000-segment cube; the floor and cap keep the per-tick
 // repaint bounded either way.
 // 2026-09-21: the ghost at 0x3d4d55 / .34 was "way too dark now, it kind of looks like shit" —
 // the part still to come has to read as the whole part at a glance. Mid grey-blue at .6, which is
 // still nothing like the hot ramp beside it.
 const PATH_GHOST=new THREE.Color(0x93a8b4),PATH_EMBER=new THREE.Color(0x7a1c00),PATH_FRESH=new THREE.Color(0xffc24a);
 const hotRun=total=>Math.max(600,Math.min(8000,Math.round(total*.06)));
 // Klipper's move queue is a couple of seconds deep; at cube speeds and this file's segment lengths
 // that is a few hundred moves, so the back-search below never has to look further than this.
 const QUEUE_SEGMENTS=400;
  function loadToolpath(parsed,name=''){clearToolpath();const bed=sourceData?.printer_frame?.bed_center;if(!bed||!parsed?.layers?.length)return null;
   let total=0;for(const l of parsed.layers)for(let i=0;i<l.extrude.length;i++)if(l.extrude[i])total++;
   if(!total)return null;
   const position=new Float32Array(total*6),layerStarts=[0],byteAt=new Int32Array(total);let at=0;
   for(const l of parsed.layers){for(let i=0;i<l.extrude.length;i++){if(!l.extrude[i])continue;position.set(l.segments.subarray(i*6,i*6+6),at*6);byteAt[at]=l.offsets?l.offsets[i]:0;at++;}layerStarts.push(at);}
   // WO-13: EXCLUDE_OBJECT ranges are global indices over the parser's own segment numbering
   // (extrudes and travels both counted, spanning layers), while the drawn array packs extrudes
   // only. One pass builds the map from a global index to its packed index (-1 for a travel), so
   // highlighting one object is a subarray copy, never a reparse.
   let objectMap=null;
   if(Array.isArray(parsed.objects)&&parsed.objects.length){let all=0;for(const l of parsed.layers)all+=l.extrude.length;
    const toPacked=new Int32Array(all).fill(-1);let g=0,e=0;
    for(const l of parsed.layers)for(let i=0;i<l.extrude.length;i++,g++)if(l.extrude[i])toPacked[g]=e++;
    objectMap={toPacked,objects:parsed.objects};}
   const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(position,3));
  const colors=new Float32Array(total*6);for(let v=0;v<total*2;v++){colors[v*3]=PATH_GHOST.r;colors[v*3+1]=PATH_GHOST.g;colors[v*3+2]=PATH_GHOST.b;}
  g.setAttribute('color',new THREE.Float32BufferAttribute(colors,3));
  // Draw ranges live on geometries, so the second object gets its own geometry over the same attributes.
  const g2=new THREE.BufferGeometry();g2.setAttribute('position',g.attributes.position);g2.setAttribute('color',g.attributes.color);
  const todo=new THREE.LineSegments(g,new THREE.LineBasicMaterial({vertexColors:true,transparent:true,opacity:.6}));
  const done=new THREE.LineSegments(g2,new THREE.LineBasicMaterial({vertexColors:true,transparent:true,opacity:1}));g2.setDrawRange(0,0);
  const group=new THREE.Group();group.add(todo,done);group.position.set(bed[0]-175,bed[1]-175,bed[2]);world.add(group);
   toolpath={group,geometry:g,todo,done,parsed,name,segments:total,painted:0,hot:hotRun(total),layerStarts,byteAt,objectMap};
  schedule();return {segments:total,layers:parsed.layers.length,bounds:parsed.bounds};}
 // Only what changed is repainted: the hot window behind the head, plus whatever the head has passed
 // since the last call, or given back on a rewind. Bounded by the advance, so a 90,000-segment file
 // costs the same per tick as a small one.
 function paintToolpath(at){const col=toolpath.geometry.attributes.color,a=col.array,c=new THREE.Color(),hot=toolpath.hot;
  const from=Math.max(0,Math.min(at,toolpath.painted)-hot),to=Math.max(at,toolpath.painted);
  for(let i=from;i<to;i++){
   if(i>=at)c.copy(PATH_GHOST);
   else{const k=Math.max(0,Math.min(1,1-(at-1-i)/hot));c.copy(PATH_EMBER).lerp(PATH_FRESH,k*k);}
   for(let v=0;v<2;v++){const o=(i*2+v)*3;a[o]=c.r;a[o+1]=c.g;a[o+2]=c.b;}}
  col.needsUpdate=true;toolpath.painted=at;}
 // 2026-09-21: "it shows pieces finished that aren't." The done boundary is now the byte the
 // printer is reading (virtual_sdcard.file_position), binary-searched against each drawn segment's
 // own byte offset, instead of a layer plus an evenly-split fraction of that layer's bytes. Layer
 // and fraction still work for a scrub with no file position.
 function toolpathProgress({layer=0,fraction=0,filePosition=null,live=null}={}){if(!toolpath)return;const starts=toolpath.layerStarts;
  let at;
  if(Number.isFinite(filePosition)&&toolpath.byteAt?.length){const b=toolpath.byteAt;let lo=0,hi=b.length;
   while(lo<hi){const mid=(lo+hi)>>1;if(b[mid]<=filePosition)lo=mid+1;else hi=mid;}at=lo;}
  else at=Math.min(toolpath.segments,Math.round((starts[layer]??0)+((starts[layer+1]??toolpath.segments)-(starts[layer]??0))*Math.max(0,Math.min(1,fraction))));
  // file_position is the byte Klipper has READ, and it runs ahead of the nozzle by whatever is in
  // the lookahead queue — measured against our cube, 7 to 27 mm of path, which on a 30 mm cube is
  // most of a wall drawn as done before the head gets there. The queue is bounded, so walking back
  // from the read pointer for the drawn segment that actually ends under the reported position puts
  // the boundary on the move the machine is making. Only taken when something that close exists;
  // otherwise the byte position stands. (2026-09-21, "it shows pieces finished that aren't")
  if(Array.isArray(live)&&live.every(Number.isFinite)&&at>0){const p=toolpath.geometry.attributes.position.array;
   let best=-1,bd=Infinity;
   for(let i=Math.max(0,at-QUEUE_SEGMENTS);i<at;i++){const o=i*6+3;
    const d=(p[o]-live[0])**2+(p[o+1]-live[1])**2+(p[o+2]-live[2])**2;if(d<bd){bd=d;best=i;}}
   if(best>=0&&bd<=9)at=best+1;}
  if(at!==toolpath.painted)paintToolpath(at);
  toolpath.done.geometry.setDrawRange(0,at*2);toolpath.todo.geometry.setDrawRange(at*2,Infinity);schedule();}
  // NEXT review 2026-09-24 (the owner): the bed mesh draped over the plate, and each probe landing as
  // it happens. Printer X/Y map to the scene like the toolpath (bed centre at 175, 175); heights are
  // exaggerated so a tenth of a millimetre can be seen, and the colour comes from the caller.
  let meshLayer=null,probeLayer=null;
  function dropLayer(l){if(!l)return;world.remove(l);l.traverse(o=>{o.geometry?.dispose();o.material?.dispose();});}
  function meshOverlay(m,{exag=25,rgb=null}={}){dropLayer(meshLayer);meshLayer=null;delete canvas.dataset.meshOverlay;const bed=sourceData?.printer_frame?.bed_center;
   if(!m?.grid?.length||!m.min||!m.max||!bed||!rgb){schedule();return false;}
   const grid=m.grid,rows=grid.length,cols=grid[0].length,SUB=6,nx=Math.max(2,(cols-1)*SUB+1),ny=Math.max(2,(rows-1)*SUB+1),flat=grid.flat().filter(Number.isFinite);
   const mean=flat.reduce((a,b)=>a+b,0)/flat.length,span=Math.max(.02,...flat.map(v=>Math.abs(v-mean))),[x0,y0]=m.min,[x1,y1]=m.max;
   const at=(u,v)=>{const x=u*(cols-1),y=v*(rows-1),c0=Math.floor(x),r0=Math.floor(y),c1=Math.min(cols-1,c0+1),r1=Math.min(rows-1,r0+1),fx=x-c0,fy=y-r0,g=(r,c)=>Number.isFinite(grid[r][c])?grid[r][c]:mean;return g(r0,c0)*(1-fx)*(1-fy)+g(r0,c1)*fx*(1-fy)+g(r1,c0)*(1-fx)*fy+g(r1,c1)*fx*fy;};
   const pos=new Float32Array(nx*ny*3),col=new Float32Array(nx*ny*3),idx=[];
   for(let j=0;j<ny;j++)for(let i=0;i<nx;i++){const u=i/(nx-1),v=j/(ny-1),z=at(u,v),k=(j*nx+i)*3,rc=rgb((z-mean)/span),c=new THREE.Color(`rgb(${rc[0]},${rc[1]},${rc[2]})`);
    pos[k]=x0+(x1-x0)*u;pos[k+1]=y0+(y1-y0)*v;pos[k+2]=0.6+(span+(z-mean))*exag;col[k]=c.r;col[k+1]=c.g;col[k+2]=c.b;
    if(i<nx-1&&j<ny-1){const a=j*nx+i;idx.push(a,a+1,a+nx,a+1,a+nx+1,a+nx);}}
   const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.BufferAttribute(pos,3));g.setAttribute('color',new THREE.BufferAttribute(col,3));g.setIndex(idx);
   meshLayer=new THREE.Mesh(g,new THREE.MeshBasicMaterial({vertexColors:true,transparent:true,opacity:.94,side:THREE.DoubleSide,depthWrite:false,toneMapped:false}));
   meshLayer.position.set(bed[0]-175,bed[1]-175,bed[2]);meshLayer.renderOrder=3;world.add(meshLayer);canvas.dataset.meshOverlay=`${rows}x${cols}`;schedule();return true;}
  // Probe landings on the plate (2026-09-25): each point is a square tile of the mesh on the flat
  // bed, coloured by how far it sits from level, with a bull's-eye over it that pulses outer ring,
  // middle ring, centre, and loops, in the same colour. The newest landing is drawn larger.
  let probeRaf=0;
  function probeMarks(points,{rgb=null}={}){dropLayer(probeLayer);probeLayer=null;cancelAnimationFrame(probeRaf);probeRaf=0;const bed=sourceData?.printer_frame?.bed_center;
   if(!Array.isArray(points)||!points.length||!bed){schedule();return;}
   // one mark per spot (the probe samples each point twice): the newest reading wins
   const byXY=new Map();for(const p of points)if(Number.isFinite(p.x)&&Number.isFinite(p.y))byXY.set(Math.round(p.x)+':'+Math.round(p.y),p);const pts=[...byXY.values()];
   const zs=pts.map(p=>p.z).filter(Number.isFinite),mean=zs.reduce((a,b)=>a+b,0)/Math.max(1,zs.length),span=Math.max(.02,...zs.map(z=>Math.abs(z-mean)));
   const xs=[...new Set(pts.map(p=>Math.round(p.x)))].sort((a,b)=>a-b),gaps=xs.slice(1).map((v,i)=>v-xs[i]).filter(d=>d>4).sort((a,b)=>a-b);
   const cell=Math.min(40,gaps.length?gaps[Math.floor(gaps.length/2)]:24);
   const colOf=p=>{const rc=rgb?rgb((p.z-mean)/span):[4,217,196];return new THREE.Color(`rgb(${rc[0]},${rc[1]},${rc[2]})`);};
   const newest=pts.reduce((a,p)=>!a||(p.ts||0)>(a.ts||0)?p:a,null);
   // every copy of one shape, placed at every point, merged into a single mesh with vertex colours
   const merged=(shape,z,scale=()=>1)=>{const sp=shape.attributes.position.array,si=shape.index.array,n=sp.length/3,pos=new Float32Array(pts.length*sp.length),col=new Float32Array(pts.length*sp.length),idx=[];
    pts.forEach((p,k)=>{const c=colOf(p),sc=scale(p);for(let v=0;v<n;v++){pos.set([p.x+sp[v*3]*sc,p.y+sp[v*3+1]*sc,z],(k*n+v)*3);col.set([c.r,c.g,c.b],(k*n+v)*3);}for(const t of si)idx.push(t+k*n);});
    const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.BufferAttribute(pos,3));g.setAttribute('color',new THREE.BufferAttribute(col,3));g.setIndex(idx);return g;};
   const mat=(o)=>new THREE.MeshBasicMaterial({vertexColors:true,transparent:true,opacity:o,depthWrite:false,depthTest:false,side:THREE.DoubleSide,toneMapped:false});
   const big=p=>p===newest?1.35:1,group=new THREE.Group();
   const tiles=new THREE.Mesh(merged(new THREE.PlaneGeometry(cell*.9,cell*.9),.6),mat(.34));
   const rings=[[.42,.36],[.28,.22],[.13,0]].map(([ro,ri],k)=>{const m=new THREE.Mesh(merged(ri?new THREE.RingGeometry(cell*ri,cell*ro,40):new THREE.CircleGeometry(cell*ro,32),1.2+k*.1,big),mat(.2));m.userData.ring=k;return m;});
   group.add(tiles,...rings);for(const m of group.children){m.renderOrder=4;m.raycast=()=>{};}
   group.position.set(bed[0]-175,bed[1]-175,bed[2]);probeLayer=group;world.add(group);
   // the pulse: outer, middle, centre, then round again (about 1.2 s a cycle)
   const t0=performance.now();const tick=now=>{if(probeLayer!==group)return;const ph=((now-t0)/1200)%1;
    for(const m of rings){const k=m.userData.ring,d=(ph-k/3+1)%1;m.material.opacity=.18+.72*Math.max(0,1-d*3);}schedule();probeRaf=requestAnimationFrame(tick);};
   probeRaf=requestAnimationFrame(tick);schedule();}
  // BLTouch LED (2026-09-24: "the BL Touch is supposed to blink"). Klipper never reports the LED,
  // so the caller derives it from what the probe was told and said: on steady with the pin up, off
  // while the pin is down, a flash each time it touches, and fast flashing in alarm. Drawn on the
  // front of the probe body with a glow, riding with the toolhead.
  let led=null,ledState='',ledRaf=0,ledFlashUntil=0;
  function ensureLed(){if(led)return led;const m=entries.get('bltouch-probe:root:0');if(!m)return null;const b=m.geometry.boundingBox,c=b.getCenter(new THREE.Vector3());
   const core=new THREE.Mesh(new THREE.SphereGeometry(1.3,16,12),new THREE.MeshBasicMaterial({color:0xff2a1a,transparent:true,opacity:.1,toneMapped:false}));
   const halo=new THREE.Mesh(new THREE.SphereGeometry(3.4,20,14),new THREE.MeshBasicMaterial({color:0xff2a1a,transparent:true,opacity:0,toneMapped:false,depthWrite:false,blending:THREE.AdditiveBlending}));
   core.position.set(c.x,c.y-5.9,b.min.z+7);core.add(halo);core.raycast=()=>{};halo.raycast=()=>{};m.add(core);led={core,halo};canvas.dataset.bltouchLed='fitted';return led;}
  function paintLed(on){led.core.material.opacity=on?1:.12;led.halo.material.opacity=on?.32:0;}
  function ledLoop(now){ledRaf=0;const on=ledState==='alarm'?Math.floor(now/220)%2===0:ledState==='probing'?now<ledFlashUntil:ledState==='on';paintLed(on);schedule();
   if(ledState==='alarm'||(ledState==='probing'&&now<ledFlashUntil+50))ledRaf=requestAnimationFrame(ledLoop);}
  function probeLed(state,{flash=false}={}){if(!ensureLed())return false;if(flash)ledFlashUntil=performance.now()+380;canvas.dataset.bltouchLed=state;
   if(state!==ledState||flash){ledState=state;cancelAnimationFrame(ledRaf);ledLoop(performance.now());}return true;}
  function clearToolpath(){if(!toolpath)return;clearObjectHighlight();world.remove(toolpath.group);toolpath.geometry.dispose();toolpath.done.geometry.dispose();toolpath.todo.material.dispose();toolpath.done.material.dispose();toolpath=null;schedule();}
  // WO-13: one printed object, highlighted straight off the drawn toolpath's packed extrude
  // array — the same coordinates the plate already shows, so the highlight can never disagree
  // with the print. Used by the Exclude objects modal so "which one is this" is answered by the
  // scene instead of by a name.
  let objectHighlight=null;
  function clearObjectHighlight(){if(objectHighlight){world.remove(objectHighlight);objectHighlight.geometry.dispose();objectHighlight.material.dispose();objectHighlight=null;}}
 function highlightObject(name){clearObjectHighlight();
  const map=toolpath?.objectMap;
  if(name==null||!map){schedule();return;}
  // Case-insensitive on purpose: the gcode parser reports the name as sliced, Klipper reports it
  // as defined, and the two disagree on case for the same object (measured 2026-09-22: parsed
  // "BODY" vs status "body").
  const o=map.objects.find(o=>String(o.name).toLowerCase()===String(name).toLowerCase());
  if(o){let e0=-1,e1=-1;
   for(let gIdx=o.start;gIdx<o.end;gIdx++){const e=map.toPacked[gIdx];if(e>=0){if(e0<0)e0=e;e1=e;}}
   if(e0>=0){const copy=toolpath.geometry.attributes.position.array.slice(e0*6,(e1+1)*6);
    const hg=new THREE.BufferGeometry();hg.setAttribute('position',new THREE.Float32BufferAttribute(copy,3));
    objectHighlight=new THREE.LineSegments(hg,new THREE.LineBasicMaterial({color:0xff2d92,transparent:true,opacity:.95,linewidth:2}));
    objectHighlight.position.copy(toolpath.group.position);world.add(objectHighlight);}}
  schedule();}
 // Stella's toolhead as installed (2026-09-19, owner facts): Rapid Burner cowl with the BLTouch
 // bracket on the LEFT seen from the front, Sherpa Mini on its printed mount, EBB42 on the chirpy
 // carrier, two 4010 blowers and one 3010 hotend fan. Parts without an STL on the drive are
 // representative primitives, labelled as such. CAD front is -Y; body centreline X=127.
 // 2026-09-20: the cowl needed "180 then mirror". Net effect is a mirror in Y, which keeps the
 // BLTouch bracket at -X (left from the front) and turns the logo face to the front. Mirroring
 // reverses triangle winding, so the winding is reversed with it or every normal points inward.
 // 2026-09-20 21:10, looking at it live: the Y mirror left the cowl upside down. Rendering the
 // STL as-is, X-, Y- and Z-mirrored side by side against his front photo: only the Z mirror puts the
 // duct mouths at the bottom, the logo on the front and the bracket on the left where his BLTouch is.
 // 2026-09-21, third time asking: the Z mirror alone left the cowl's front facing the REAR of
 // the printer. Mirror it in Y as well and the front comes round to the front, with the duct mouths
 // still down and the BLTouch bracket still on the left. The Y mirror is taken about the part's own
 // centre so COWL_TRANSLATION below still lands it where it already sat.
 const COWL_TRANSLATION=[88.0,143.7,105.7];
 // WO-17 item 1, second pass (2026-09-23), the owner: the cowl "clips onto and screws onto" the back piece.
 // With the Rapid Burner's own hotend mount in the chain (X block, chirpy carriage, the mount's rear
 // half, its front clamp, the cowl), the cowl's two front screws land on the clamp's two inserts and
 // the clamp's face meets the cowl's screw bosses only with the cowl 3.40 mm further back and 1.24 mm
 // lower than it sat. Its two rear inserts then meet the carriage base's arm eyes within 0.05 mm, a
 // second check that does not go through the mount. Every point below that was measured off the cowl
 // was measured where it sat before, so onCowl() carries it with the cowl.
 const COWL_MOVE=[0,3.40,-1.24],shift=(d,p)=>p.map((v,i)=>v+d[i]),onCowl=p=>shift(COWL_MOVE,p);
 // A mirror reverses triangle winding, so reverse the index with it or every normal points inward.
 function reverseWinding(g){const ix=g.getIndex();if(ix){const a=ix.array;for(let i=0;i<a.length;i+=3){const t=a[i];a[i]=a[i+2];a[i+2]=t;}ix.needsUpdate=true;}g.computeVertexNormals();return g;}
 function mirrorZ(g){g.scale(1,1,-1);return reverseWinding(g);}
 function mirrorYInPlace(g){g.computeBoundingBox();const cy=(g.boundingBox.min.y+g.boundingBox.max.y)/2;
  g.translate(0,-cy,0);g.scale(1,-1,1);g.translate(0,cy,0);g.computeBoundingBox();return reverseWinding(g);}
 function mirrorXInPlace(g){g.computeBoundingBox();const cx=(g.boundingBox.min.x+g.boundingBox.max.x)/2;
  g.translate(-cx,0,0);g.scale(-1,1,1);g.translate(cx,0,0);g.computeBoundingBox();return reverseWinding(g);}
 // A half turn about the part's own vertical axis (a rotation, so the winding stays as it is).
 function turnZInPlace(g){g.computeBoundingBox();const c=g.boundingBox.getCenter(new THREE.Vector3());g.translate(-c.x,-c.y,0);g.rotateZ(Math.PI);g.translate(c.x,c.y,0);g.computeBoundingBox();return g;}
 const stockToolhead=/Extruder Body|Extruder Motor Plate|Latch|Guidler|BMG|Needlebearing|Direct_Drivegear|Motorgear|Shaft-assembly|MR85|NEMA17 Short|Spring Assembly|M3x20 shaft|Connector cover|Filament Hinge|Blower Housing|Hotend Fan Mount|Omron TL_Q5MC2|Panasonic GX-H15A|probe_bracket/i;
 const installed=[];const spinners=[];let heatBlock=[],bedSheet=[];
 const rx=a=>g=>g.rotateX(a),rz=a=>g=>g.rotateZ(a);
 // WO-17 (2026-09-23), the owner looking at the app: "the BLTouch could come forward a little bit" and
 // "the fans on the side, the 4010s, could go back just a pinch." Nudges along world Y (the front
 // of the printer is -Y) on top of the measured placements below. They are his calls by eye, not
 // measurements, so they are the two numbers to tune. BLTOUCH_AT is where printer.cfg's offsets
 // and the bracket plate put the probe before the nudge.
 // BLOWER_BACK_MM (NEXT review 2026-09-24: "the fans are still sticking out, protruding too far
 // forward"): his photo f31de800 shows the cowl's front wall wrapping in front of each 4010, so the
 // blower sits recessed behind it. 2 -> 5 mm; one number to tune against the machine.
 const BLTOUCH_FORWARD_MM=3,BLOWER_BACK_MM=5,BLTOUCH_AT=[94.05,120.4,48.6];
 // 2026-09-23: "About 2.5 mm from the bottom of the cowl to the nozzle tip." A hand measurement,
 // about ±0.5 mm: the cowl cannot come off to measure more closely. The nozzle is placed from it.
 const NOZZLE_BELOW_COWL_MM=2.5;
 // WO-17 item 1, second pass (2026-09-23): the Sherpa stands on the Rapid Burner's extruder mount,
 // which lies on the hotend mount's top face (Z 97.56, 6.90 mm thick, so its top is Z 104.46, level
 // with the cowl's top). The Sherpa's two feet holes then sit on the mount's two holes and the hotend
 // mount's two inserts (X 105.25 / 148.75, Y 130.75) and its filament channel on the V6 axis: 1.69 mm
 // right, 10.05 mm back and 16.14 mm lower than WO-11 stood it. WO-11 had it standing on a tab that
 // hangs 8 mm below its feet; that tab is on one of the two front plates the Annex STEP carries
 // (SOURCE_SKIP), and it would pass through the cowl's 1.92 mm top plate, so its box centre rises 4.00 mm.
 // The EBB42 carrier and board no longer take this move: since our answer (2026-09-23) the carrier
 // stands tower up on the carriage's ear inserts and the Sherpa's frame screws, placed absolutely.
 const SHERPA_MOVE=[1.69,10.05,-16.14];
 // Every fitted part is real published CAD. Placement is rigid and approximate: it aligns each
 // model to the reference carriage and nozzle, it is not a measured mounting transform.
 const FITTED=[
  // Sits ON the mount, not in the carriage: the mount's top face is Z 112.60 and the extruder CAD is
  // 53.6 mm tall in this orientation, so its centre is 112.60 + 26.80 = 139.40. It used to centre at
  // 130, which put its base at 103.20 — 2.2 mm inside the X carriage frame, which tops out at
  // 105.40. That is what the owner saw. (2026-09-21)
  {id:'annex-sherpa-mini',name:'Sherpa Mini extruder',ops:[rx(Math.PI/2)],at:[127,128,139.4],move:shift([0,0,4.00],SHERPA_MOVE),note:'Annex Engineering Sherpa Mini CAD on the Rapid Burner extruder mount: feet on its top at Z 104.46, feet holes on the hotend mount\'s inserts at X 105.25 / 148.75, Y 130.75, filament channel on the V6 axis'},
  // WO-17 (2026-09-23), the owner: "the electronics board is supposed to sit on the back of the piece
  // behind the extruder and it's not; it's embedded in the extruder." It sat between the carrier
  // and the Sherpa (Y 132.6..163.4 against the Sherpa's 104.8..151.2). It now follows the carrier
  // (turned the same 180° about Z, so its components face the rear as in our video 711a9f31)
  // and sits on the carrier's four board bosses: PCB centred on their 31 x 31 mm square at
  // X 124.94 / Z 140.85, its back face on the boss tops at Y 161.80, all measured on both CADs as
  // placed. BTT's CAD also carries four M3x20+6 standoffs for a motor mount; Stella's board is on
  // the carrier's inserts instead, so SOURCE_SKIP leaves them out.
  // 2026-09-23 turned the carrier tower up (below), and the board turns over with it: the same
  // half turn about Y, so its components still face the rear. PCB back face on the four bosses'
  // tops at Y 177.06 and centred on their 31 x 31 mm square (X 111.86 / 142.86, Z 113.92 / 144.92),
  // measured on both CADs as placed; its box centre sits 0.02 mm and 0.66 mm off that square's
  // centre because of the connectors, the same offsets as before, mirrored.
  {id:'btt-ebb42-v12',name:'BTT EBB42 CAN V1.2',ops:[rx(-Math.PI/2)],at:[127.34,181.15,128.76],note:'BIGTREETECH EBB42 V1.2 CAD on the rear of the chirpy carrier, turned over with it when the owner put the tower up (2026-09-23): PCB back face on the four board bosses\' tops at Y 177.06, centred on their 31 x 31 mm square, components facing the rear; BTT\'s four motor standoffs left out'},
  // 2026-09-21: both of these were hand-picked points that no longer matched the cowl once it
  // was mirrored, so they are now measured off the cowl's own geometry. Ray-cast against the same
  // /api/assets/stella-rapid-bltouch-cleaned/geometry the HUD loads, in its placed world position:
  //   front face   one round opening, 27.99 mm across, centred X 126.95 / Z 74.68
  //   front face   min Y 103.70, so a 10 mm deep fan sits centred at Y 108.70
  //   BLTouch tab  a flat 3.00 mm plate, Z 69.85..72.85, reaching out to X 88.00 over Y 106..127
  // The probe keeps its configured XY (nozzle X 127.0 / Y 131.1 plus printer.cfg x_offset -32.95,
  // y_offset -10.718) and now hangs from that measured plate instead of floating 2.4 mm over it.
  // Its source CAD is 42.5 mm tall, so a centre of Z 51.60 puts the body top on the plate.
  // 2026-09-21, second look: "the BLTouch is still not in the right position, it is too high."
  // It was hung with its body top on the plate's TOP face (72.85), so the body passed through the
  // 3 mm plate instead of hanging under it. Measured on the probe CAD in its placed orientation
  // (1 mm Z bands over the mesh): the body is 11.53 wide and the two mounting ears open it out to
  // 25.97 over the top 2.5 mm — that flange IS the top of the body, so seating the flange against
  // the plate's underside is the same as putting the body top at Z 69.85, i.e. 3.00 mm lower.
  // Centre Z 72.85 - 3.00 - 42.5/2 = 48.60. Check by the other rule in the WO: that puts the CAD's
  // deployed pin tip at 69.85 - 42.5 = 27.35, which is 1.35 mm below the nozzle tip at 28.70, and
  // the probe triggers z_offset 1.961 above the bed with the pin pushed up — so the two rules agree
  // to within 0.6 mm. The plate rule is the one used, because the probe hangs off the plate.
  {id:'bltouch-probe',name:'BLTouch probe',ops:[rx(Math.PI/2),rz(Math.PI/2)],at:[BLTOUCH_AT[0],BLTOUCH_AT[1]-BLTOUCH_FORWARD_MM,BLTOUCH_AT[2]],move:COWL_MOVE,note:'Probe body at the configured offset X -32.95 / Y -10.718 from the nozzle, brought '+BLTOUCH_FORWARD_MM+' mm forward at the owner\'s call 2026-09-23, mounting flange seated against the underside of the cowl bracket plate, and carried with the cowl onto the hotend mount'},
  // our photo 151eddb3: the axial fan sits upright in the cowl's front opening, blowing back
  // through the heatsink. Rotated from its flat CAD pose and recessed just inside the front face.
  // 2026-09-21, live: "recessed in just a little bit further, it's still sticking out through the
  // model." Measured flush at the bore ring (WO-09 T4), but the cowl face steps back behind the fan
  // over the square corners of the 30 mm body — 5.1% of the probed cells — so the corners showed
  // through. Centre Y 108.70 -> 111.70 puts the body 3 mm further in, spanning Y 106.70..116.70.
  // His call on the next look; step again by 2 mm if it is still visible.
  {id:'fan-3010-axial',name:'Hotend fan · 3010 axial',ops:[rx(Math.PI/2)],at:[126.95,111.7,74.68],move:COWL_MOVE,fan:'hotend_fan',note:'Rapid Burner BOM 24 V 3010 axial, centred on the cowl\'s measured 27.99 mm front opening (X 126.95 / Z 74.68), recessed 8 mm inside the front face at the owner\'s call 2026-09-21, blowing back through the heatsink'},
 ];
 // The toolhead umbilical: a braided loom from the top of the EBB42 carrier up to the rear of the
 // frame (photo d9f032c6). Stella has no toolhead cable chain. Redrawn whenever the head moves.
 let umbilical=null;
 // 2026-09-23: the carrier's tall tower points up, "that's where the umbilical goes", so the loom
 // starts on the centre of the tower's top face: the box of the carrier's vertices within 1 mm of its
 // highest point (X 120.18, Y 173.24, Z 184.81 as placed), not the middle of the whole part.
 function towerTop(g){const p=g.attributes.position.array,z=g.boundingBox.max.z,lo=[Infinity,Infinity],hi=[-Infinity,-Infinity];for(let i=0;i<p.length;i+=3)if(p[i+2]>z-1)for(const k of [0,1]){lo[k]=Math.min(lo[k],p[i+k]);hi[k]=Math.max(hi[k],p[i+k]);}return new THREE.Vector3((lo[0]+hi[0])/2,(lo[1]+hi[1])/2,z);}
 function drawUmbilical(){if(!bounds)return;const carrier=meshes.find(m=>m.userData.id==='stella-ebb42-carrier:root:0');if(!carrier)return;
  const off=carrier.userData.motionOffset||new THREE.Vector3();carrier.userData.towerTop??=towerTop(carrier.geometry);const start=carrier.userData.towerTop.clone().add(off);
  // 2026-09-24: the umbilical leaves the printer beside the rear cable chain, so it ends on the
  // rear top frame just to the right of the Z chain.
  const zcx=zChain?zChain.wc+zChain.R+26:bounds.min.x+150,end=new THREE.Vector3(zcx,zChain?zChain.y-8:bounds.max.y-60,bounds.max.z-30);
  const curve=new THREE.CatmullRomCurve3([start,start.clone().add(new THREE.Vector3(0,6,90)),new THREE.Vector3(start.x+(end.x-start.x)*.45,start.y+(end.y-start.y)*.5,end.z+35),end.clone().add(new THREE.Vector3(0,-40,10)),end],false,'centripetal');
  const g=new THREE.TubeGeometry(curve,48,4.5,10,false);g.computeBoundingBox();
  if(umbilical){umbilical.geometry.dispose();umbilical.geometry=g;umbilical.userData.center=g.boundingBox.getCenter(new THREE.Vector3());return;}
  umbilical=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x111214,metalness:.05,roughness:.95}));const id='stella:umbilical';
  umbilical.userData={id,record_id:instanceRecordId(id),name:'Toolhead umbilical',full:'Stella / Toolhead (installed) / Umbilical loom',asset:{id,name:'Toolhead umbilical',source_name:'drawn from photo d9f032c6',status:'installed',format:'none',revision:'HUD 2026-09-20',notes:'Braided loom carrying CAN, 24 V and the heater from the EBB42 to the rear of the frame. Stella uses this instead of the reference X cable chain.',source_url:null,license:null,original_url:null},slot_id:'head.umbilical',layer:'toolhead',motion:'fixed',center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),colorGroup:'toolhead',placement:{status:'Path drawn from the carrier to the rear frame; not measured'}};
  root.add(umbilical);meshes.push(umbilical);entries.set(id,umbilical);installed.push(umbilical);}
 // A part the owner has installed that cannot be placed used to vanish without a word (null return,
 // paintBlack(null) swallowing it), so the head could lose a piece and the HUD would look finished.
 // Missing ids are now named in the scene note and on the canvas. (2026-09-21, item 11)
 const missingParts=[];
 async function placeSource(assetId,name,transform,note,extra={}){const source=sourceAssets.find(a=>a.id===assetId);if(!source){missingParts.push(assetId);canvas.dataset.missingParts=missingParts.join(' ');return null;}const id=await inspectSource(source.id),m=entries.get(id);transform(m.geometry);m.geometry.computeBoundingBox();
  Object.assign(m.userData,{sourceOnly:false,name,full:'Stella / Toolhead (installed) / '+name,layer:'toolhead',motion:'toolhead',center:m.geometry.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),placement:{status:note},...extra});root.add(m);meshes.push(m);installed.push(m);return m;}
 // Centre a geometry on a world point after its own rotations, so a source model's arbitrary
 // origin never has to be guessed at. Returns the applied translation for the record.
 function centreOn(g,ops,at){for(const op of ops)op(g);g.computeBoundingBox();const c=g.boundingBox.getCenter(new THREE.Vector3());const t=new THREE.Vector3(...at).sub(c);g.translate(t.x,t.y,t.z);return t.toArray().map(v=>Number(v.toFixed(3)));}
 // our hotend is a Trianglelab CHC V6: V6 heatsink, heatbreak and nozzle (real CAD, already in the
 // assembly) with a ceramic heating core in place of the stock block and cartridge. No public CAD
 // exists for the core, so it is turned from Trianglelab's published dimensions: a 16 mm ceramic
 // cylinder tapering to the nozzle seat, 12 mm tall, on the nozzle axis.
 function fitHotend(){
  const nozzle=meshes.find(m=>m.userData.slot_id==='head.nozzle');if(!nozzle)return;
  const nb=nozzle.geometry.boundingBox,axis=[(nb.min.x+nb.max.x)/2,(nb.min.y+nb.max.y)/2];
  for(const m of meshes)if(/^head\.(block|cartridge)$/.test(m.userData.slot_id||''))m.userData.replacedByRapid=true;
  const profile=[[0,0],[8,0],[8,9],[6.4,12],[3.2,12.6],[3.2,13.4],[0,13.4]].map(([r,z])=>new THREE.Vector2(r,z));
  // 2026-09-21: "the nozzle doesn't look anything like what it's supposed to." THREE revolves a
  // lathe about its own +Y, and the quarter turn that stands it on the nozzle axis was left as
  // rotateX(0) — a no-op — so the ceramic core was drawn lying on its side: 16 mm across X and Z
  // with its 13.4 mm height pointing at the back of the printer. Stood up.
  const g=new THREE.LatheGeometry(profile,48);g.rotateX(Math.PI/2);g.translate(axis[0],axis[1],nb.max.z-1.4);g.computeBoundingBox();
  const m=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0xe6e2d8,metalness:.1,roughness:.65}));
  const id='stella:chc-v6-core';
  m.userData={id,record_id:instanceRecordId(id),name:'Trianglelab CHC V6 · ceramic heating core',
   full:'Stella / Toolhead (installed) / Trianglelab CHC V6 heating core',
   asset:{id,name:'Trianglelab CHC V6 heating core',source_name:'turned from published dimensions',status:'installed',format:'none',revision:'HUD 2026-09-20',
    notes:'Identified from the owner\'s photo 2026-09-14. Trianglelab publishes no CAD, so the ceramic core is turned from their published dimensions on the real V6 nozzle axis. The heatsink, heatbreak and nozzle around it are the reference V6 CAD. 24 V, about 57.6 W, 10 ohm core; max_temp 290 with an EPCOS 100K B57560G104F thermistor.',
    source_url:'https://www.trianglelab.net',license:null,original_url:null},
   slot_id:'head.core',layer:'toolhead',motion:'toolhead',center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),colorGroup:'toolhead',
   placement:{status:'On the measured nozzle axis of the reference V6'}};
  root.add(m);meshes.push(m);entries.set(id,m);installed.push(m);
  // 2026-09-21: "there's also no silicone sock on my nozzle, and the nozzle doesn't look
  // anything like what it's supposed to. It's supposed to be big, it's huge." Scale taken off his
  // photo 94ce96d4 (2026-09-14, the hotend from below with the cowl on) against the one known
  // dimension in the same frame: the brass V6 nozzle hex, 7.0 mm across flats, 8.08 across corners
  // and photographed corner-on, measuring 162.5 px — so 0.0497 mm/px. The ceramic core in that
  // photo measures 260 px across, i.e. 12.9 mm at the plane of the nozzle and about 14 mm once the
  // 15 mm it sits further from the lens is allowed for, against Trianglelab's published 16 mm. So
  // the core's own diameter stands. The sock is NOT in that photo (it is the newest hotend photo on
  // the drive and the core is bare in it), so its wall is the part type's, about 2 mm of silicone
  // around the core, giving a 20 mm outer envelope 15.4 mm tall: dark, matte, rounded, the big
  // black block he describes. The 2 mm wall is the one number here that is not measured off our
  // own machine and he should say if it looks wrong.
  // NEXT review 2026-09-24: "you have images of my nozzle and the sock and have not built it properly with the
  // silicone sock." Photo 94ce96d4 IS the sock: off-white silicone, stained, hugging the core, the brass
  // top of the core showing above it and the brass hex and tip below. 17.2 mm across (core 16 plus a thin
  // skin), 16 mm tall, a rolled lip at the bottom around the nozzle hole. Replaces the guessed black 20 mm pot.
  // Its lip sits on the brass hex, as in the photo: base 6 mm above the tip (V6 tip cone plus hex), top at
  // the core's top edge, so the height follows the nozzle datum instead of leaving bare shank showing.
  const sockBase=nb.min.z+6,sockH=Math.max(12,g.boundingBox.max.z-sockBase);
  const sockProfile=[[4.4,0],[6.9,0],[8.1,.5],[8.6,1.8],[8.6,sockH-1.8],[8.2,sockH-.6],[7.4,sockH],[4.4,sockH]].map(([r,zz])=>new THREE.Vector2(r,zz));
  const sg=new THREE.LatheGeometry(sockProfile,48);sg.rotateX(Math.PI/2);sg.translate(axis[0],axis[1],sockBase);sg.computeBoundingBox();
  const sock=new THREE.Mesh(sg,new THREE.MeshStandardMaterial({color:0xe6e0d3,metalness:0,roughness:.88,side:THREE.DoubleSide}));
  const sid='stella:chc-v6-sock';
  sock.userData={id:sid,record_id:instanceRecordId(sid),name:'Silicone sock · CHC V6',
   full:'Stella / Toolhead (installed) / Trianglelab CHC V6 silicone sock',
   asset:{id:sid,name:'CHC V6 silicone sock',source_name:'sized from the owner\'s photo 94ce96d4 against the 7 mm nozzle hex',status:'installed',format:'none',revision:'HUD 2026-09-21',
    notes:'2026-09-21: the hotend is bigger than the model showed and wears a silicone sock. Outer envelope 20.0 mm across and 15.4 mm tall, taken as the measured ceramic core plus a 2 mm silicone wall. Scale reference: the brass V6 nozzle hex in the same photo, 7.0 mm across flats, 162.5 px, 0.0497 mm/px. The sock itself is not in any photo on the drive, so the wall thickness is the part type\'s, not a measurement of his.',
    source_url:'https://www.trianglelab.net',license:null,original_url:null},
   slot_id:'head.sock',layer:'toolhead',motion:'toolhead',center:sg.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),colorGroup:'toolhead',
   placement:{status:'Concentric on the measured nozzle axis, wrapping the ceramic core; wall thickness from the part type, not measured on Stella'}};
  root.add(sock);meshes.push(sock);entries.set(sid,sock);installed.push(sock);
 }
 // The Rapid Burner's own fasteners: the reference CAD's stock-toolhead screws are suppressed with
 // the rest of that toolhead, so the cowl came out with no visible hardware at all (2026-09-21,
 // "no screws in the cowl"). Real M3 SHCS CAD from the reference assembly, cloned onto the holes
 // measured on the cowl's own front face: the two counterbores at X 110.6 and 143.4, Z 94.4 (WO-09's
 // front-face scan) carry the cowl's mounting screws, and the 3010 sits on the standard 24 mm square
 // about the measured bore centre X 126.95 / Z 74.68, so its four screws are at X 114.95 / 138.95
 // and Z 62.68 / 86.68. The screw axis is world Y, into the face at Y 103.70, heads standing proud.
 // WO-17 (2026-09-23), the owner: "the screws are sticking way out." Measured on the cowl as placed:
 // each mounting hole is a 6.5 mm counterbore from the face to a floor at Y 114.70, then a 3.4 mm
 // hole, so the head belongs on that floor, 8 mm below the face, as in his photo 151eddb3. The
 // donor's head is its -Y end (5.5 mm, 3 mm tall), so the screw now centres at Y 117.20 instead of
 // 106.20, which had the head 3 mm proud of the face and 11 mm out of its seat. The cowl has no
 // hole at the 3010's four corners at any depth (the v8 fan is held screwless), so those four
 // screws belonged to nothing and are no longer made.
 const COWL_SCREW_Y=117.2;
 function fitFasteners(){
  const donor=meshes.find(m=>/^M3x8 SHCS/.test(m.userData.name||'')&&!installed.includes(m));
  if(!donor)return 0;
  const holes=[[110.6,94.4,'cowl mount left'],[143.4,94.4,'cowl mount right']];
  for(const [x,z,where]of holes){
   const g=donor.geometry.clone();centreOn(g,[],onCowl([x,COWL_SCREW_Y,z]));g.computeBoundingBox();
   const m=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x8d949b,metalness:.75,roughness:.3}));
   const id='stella:cowl-screw-'+where.replace(/\s+/g,'-');
   m.userData={id,record_id:instanceRecordId(id),name:'M3 SHCS · '+where,full:'Stella / Toolhead (installed) / Cowl fastener '+where,
    asset:donor.userData.asset,slot_id:'head.fastener',layer:'toolhead',motion:'toolhead',colorGroup:'fasteners',
    center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),
    placement:{status:'Reference M3x8 SHCS CAD in the measured cowl counterbore at X '+x+' / Z '+(z+COWL_MOVE[2]).toFixed(2)+', head on its floor at Y '+(114.70+COWL_MOVE[1]).toFixed(2)+', 8 mm below the front face, into the hotend clamp\'s insert'}};
   root.add(m);meshes.push(m);entries.set(id,m);installed.push(m);}
  return holes.length;
 }
 async function mountRapidBurner(){
  const source=sourceAssets.find(a=>a.id==='stella-rapid-bltouch-cleaned');if(!source||source.source_sha256!=='0172e800f8606e99cf5fc1555457de8931f339ae3f6547c1630fa3b1b49a0329'||asset.source_sha256!=='abeef1bb9bcae90a9ed579e901423d2f564143eae1e39c2ee5b96b2a74ec467c')return;
  const cowl=await placeSource(source.id,'Rapid Burner · custom BLTouch cowl',g=>{mirrorZ(g);mirrorYInPlace(g);g.translate(...onCowl(COWL_TRANSLATION));},'Mirrored in Z then in Y: duct mouths down, front face toward the front of the printer, bracket left; its two front screws on the hotend clamp\'s two inserts and its rear inserts on the carriage base\'s arm eyes, both measured');
  // the owner: the cowl is white (2026-09-20); the photos were shot in the dark.
  cowl.userData.full='Stella / Toolhead (installed) / Rapid Burner v8 cowl, BLTouch bracket left';cowl.material.color.setHex(0xe9e9e6);cowl.material.roughness=.7;cowl.material.metalness=.02;
  // Owner review 2026-09-20: no cable chain runs to the toolhead (umbilical instead), so the X
  // cable chain and its carriage anchor go. The right XY joint keeps its chain bridge: that is the
  // red bracket in photo dd09eba8 that carries the Y chain. Loose channel nuts are not on Stella.
  const notOnStella=d=>/\/ X Cable Chain:/.test(d.full)||/Chain Anchor|M5 Hex Nut/i.test(d.name)||/XY Cable Chain Bridge - 2 Hole/.test(d.name);
  const suppressed=[];
  for(const original of meshes){const d=original.userData;if(installed.includes(original))continue;
   d.replacedByRapid=(/\/ Fan Assembly/.test(d.full)||/\/ Toolheads/.test(d.full)&&/body[ _]*(front|rear)/i.test(d.name)||/X Carriage:1/.test(d.full)&&stockToolhead.test(d.name)||notOnStella(d));
   if(d.replacedByRapid)suppressed.push(d.name);}
  // WO-17 (2026-09-23), the owner: "three little washers that would have gone on the cable chain are just
  // randomly sitting there behind it," and "the screws are sticking way out." Suppressing a printed
  // part left the hardware the CAD nests inside it floating where the part had been: the X chain
  // anchor's three threaded inserts and two M3x20, the Clockwork body's and motor plate's screws
  // through the Sherpa, the stock probe mounts' screws under the carriage. Whatever sits inside the
  // CAD node of a part that is not on Stella now goes with that part.
  const gone=meshes.filter(m=>m.userData.replacedByRapid).map(m=>{const f=(m.userData.full||'').split(' / '),i=f.indexOf(m.userData.name);return i>0?f.slice(0,i+1).join(' / ')+' / ':null;}).filter(Boolean);
  const orphans=[];for(const m of meshes){const d=m.userData;if(d.replacedByRapid||installed.includes(m))continue;if(gone.some(p=>(d.full||'').startsWith(p))){d.replacedByRapid=true;suppressed.push(d.name);orphans.push(d.id);}}
  canvas.dataset.orphans=JSON.stringify(orphans);
  canvas.dataset.suppressed=String(suppressed.length);window.__stellaSuppressed=suppressed;
  // our own printed parts (his STLs, from the portable drive).
  const BLACK=0x1e1f22;const paintBlack=m=>{if(m){m.material.color.setHex(BLACK);m.material.roughness=.8;m.material.metalness=.05;}return m;};
  // WO-17 item 1 (2026-09-23), the owner: "It's a Rapid Burner hotend. You have the front cowl, but not
  // the actual back piece that the cowl clips onto and screws onto. It's just open air there."
  // On a v2.4 the Rapid Burner rides on chirpy2605's two-part MGN12H mount, which takes the place
  // of the Afterburner X carriage frame the reference CAD carries. Rail half: the Lite with PCB
  // lugs, the two ears standing on its top in our photos f31de800 and 151eddb3 (his chirpy PCB
  // carrier's lower lobes are 25.70 mm apart, the ears' insert spacing). Lower half: the Short
  // base, a guess over the Long: photo 94ce96d4 shows a printed part carrying a small board right
  // behind the nozzle, where the Short takes a probe, but no photo shows its profile.
  // Both are placed off measured geometry. A quarter turn about X stands the carriage up; its four
  // 3.4 mm holes then land on the block's 20 x 20 mm pattern (X 117 / 137, Z 67.70 / 87.70) and
  // its seat face on the block's front face at Y 160. The base's two top tabs sit on the carriage's
  // two bottom inserts (X 111.15 / 142.85), and its arm eyes meet the cowl's two rear inserts, with
  // the cowl where the hotend mount below puts it, within 0.04 mm across and 0.05 mm in height.
  const CARRIAGE_AT=[127,142.1,107.56],CARRIAGE_BASE_AT=[127,147.07,70.86];
  const carriage=paintBlack(await placeSource('chirpy-mgn12h-x-carriage-lugs','X carriage · chirpy MGN12H Lite with PCB lugs',g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIAGE_AT);},'Its four 3.4 mm holes on the X block\'s measured 20 x 20 mm pattern, seat face on the block front at Y 160; the Lugs variant from photos f31de800 and 151eddb3'));
  if(carriage)paintBlack(await placeSource('chirpy-carriage-base-short','Carriage base · chirpy short (guess)',g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIAGE_BASE_AT);},'On the carriage\'s two bottom inserts; Short over Long is a guess. Its arm eyes meet the cowl\'s two rear inserts within 0.05 mm'));
  // Second pass (2026-09-23): the part the cowl's two front screws go into, found and downloaded on
  // our go: the Rapid Burner v8 hotend mount for a V6 and a Sherpa Mini (chirpy2605, V0/Rapid_Burner/
  // STLs/v0.2/v6_Volcano). The first pass's V6_Mount_Rear/Front on the drive are the Dragon Burner's.
  // Rear half: a quarter turn about X; its two M3x10 holes (X 114.46 / 139.54, Z 82.00) on the
  // carriage's two toolhead holes (114.52 / 139.48), its back face on the carriage's front at
  // Y 142.10 with its key in the carriage's pocket. Front clamp: its face on the rear half's split
  // face at Y 126.60, its two clamp holes on the rear half's two inserts. The cowl's front screws go
  // into the clamp's two inserts (X 110.60 / 143.40, Z 93.16); that is what COWL_MOVE is.
  const REAR_AT=[-20.715,126.60,-40.49],FRONT_AT=[-20.715,121.10,-51.805];
  paintBlack(await placeSource('chirpy-rapid-burner-v6-mount-rear','Hotend mount · Rapid Burner v8, V6 and Sherpa Mini, rear half',g=>{g.rotateX(-Math.PI/2);g.translate(...REAR_AT);},'Its two M3x10 holes on the carriage\'s two toolhead holes, back face on the carriage front at Y 142.10; V6 bore axis X 127.00 / Y 130.81'));
  paintBlack(await placeSource('chirpy-rapid-burner-v6-mount-front','Hotend clamp · Rapid Burner v8, V6, front half',g=>{g.rotateX(-Math.PI/2);g.translate(...FRONT_AT);},'On the rear half\'s split face at Y 126.60, clamp holes on the rear half\'s inserts; the cowl\'s two front screws go into its two inserts'));
  // The V6 sits in the mount's 15.99 mm bore with the bore's 6.00 mm ridge in the heatsink's groove,
  // so the heatsink sits 5.83 mm higher and 0.29 mm further forward than the reference Afterburner
  // held it. The length of our CHC V6 heat stack below the heatsink is on no drawing or photo, so
  // the heatbreak and nozzle only come onto the bore's axis and keep their height: nozzle tip Z 28.70.
  const V6_SEAT=[0,-0.29,5.83],V6_AXIS=[0,-0.29,0];
  for(const m of meshes){const f=m.userData.full||'',d=/\/ E3D V6 All Metal Hotend:1 \/ Heatsink:1 \/|\/ E3D V6:1 \/ PTFE Tube \(40mm\):1 \//.test(f)?V6_SEAT:/\/ E3D V6 All Metal Hotend:1 \/ (Heatbreak|Nozzle):1 \//.test(f)?V6_AXIS:null;if(d){m.geometry.translate(...d);m.geometry.computeBoundingBox();m.userData.center=m.geometry.boundingBox.getCenter(new THREE.Vector3());}}
  // 2026-09-23 measured it: the tip is NOZZLE_BELOW_COWL_MM under the cowl's lowest point. The tip
  // stays the Z datum (establishMotion maps it to the printer's Z), so the mount chain keeps its place
  // on the gantry and the hotend below the heatsink is what grows: the nozzle comes down, and
  // fitHotend() builds the core and sock on it. As placed: cowl lowest point Z 28.50, tip 28.70 to
  // 26.00, 2.70 mm lower. The stack below the heatsink is then 28.13 mm, and 3.63 mm of heatbreak
  // shows between the core and the heatsink, where 0.93 mm did.
  const nozzleParts=meshes.filter(m=>/\/ E3D V6 All Metal Hotend:1 \/ Nozzle:1 \//.test(m.userData.full||''));
  if(nozzleParts.length){const drop=Math.min(...nozzleParts.map(m=>m.geometry.boundingBox.min.z))-(cowl.geometry.boundingBox.min.z-NOZZLE_BELOW_COWL_MM);
   for(const m of nozzleParts){m.geometry.translate(0,0,-drop);m.geometry.computeBoundingBox();m.userData.center=m.geometry.boundingBox.getCenter(new THREE.Vector3());}canvas.dataset.nozzleDrop=drop.toFixed(2);}
  // With chirpy's mount on the block, the Afterburner frame is not on Stella, and nor is anything
  // the reference CAD bolts to it: both halves, the four block screws, inserts, nuts, the probe
  // magnet, and the KW10 X endstop with its two screws (where our PG6 switch sits is in no photo).
  const frameHidden=[];if(carriage)for(const m of meshes){const d=m.userData;if(!installed.includes(m)&&!d.replacedByRapid&&/\/ X Carriage:1 \/ X Carriage Frame:1 \//.test(d.full||'')){d.replacedByRapid=true;suppressed.push(d.name);frameHidden.push(d.id);}}
  canvas.dataset.suppressed=String(suppressed.length);canvas.dataset.carriageFrame=JSON.stringify(frameHidden);
  // 2026-09-21: "the piece on top of the carriage piece that's missing, that's supposed to be
  // underneath the extruder, is not there, which is putting the extruder and the Sherpa right down
  // into the holder that stays on the rail." The mount was in the scene all along — it was stood on
  // edge. Its STL is a flat plate, 52.48 x 24.09 x 6.90 mm with the thickness on its own Z, and the
  // quarter turn about X put that thickness on world Y, so it read as a 7 mm sliver edge-on and the
  // extruder sat through it into the carriage. No rotation: the plate lies flat, seated on the
  // measured tops of the cowl (105.70) and the X carriage frame (105.40), so it spans Z 105.70 to
  // 112.60 and centres at 109.15.
  // WO-17 item 1, second pass (2026-09-23): it is the Rapid Burner's extruder mount (the green part in
  // chirpy's extrudermount.png), and it lies on the hotend mount's top face, not on the cowl. Turned
  // round in place, its two holes (43.50 mm apart) sit on the hotend mount's two extruder inserts at
  // Y 130.75 and its filament hole on the V6 axis; its front edge is then flush with the clamp's face
  // (121.16 against 121.10) and its back edge with the rear half's key (145.25 against 145.22).
  const MOUNT_MOVE=[0,0.205,-8.14];
  paintBlack(await placeSource('stella-sherpa-mini-mount','Sherpa Mini mount · printed',g=>{centreOn(g,[],[127,133,109.15]);turnZInPlace(g);g.translate(...MOUNT_MOVE);},'Printed 2026-09-13; the Rapid Burner extruder mount, on the hotend mount\'s top face at Z 97.56 with its holes on the hotend mount\'s extruder inserts, 6.90 mm thick'));
  // WO-17 (2026-09-23), the owner: "the electronics holder piece that sits on the back of the Sherpa is
  // backwards; turn it around, back to front, and it's too low." Turned 180° about Z, its board
  // bosses face the rear. Height and X come from the holes it is made for: its 3.4 mm pair 43.85 mm
  // apart sits on the axes of the Sherpa Mini's two frame screws, measured on the Annex CAD as
  // placed at X 103.39 / 147.23, Z 147.95 (43.84 apart). That lifts it 18.76 mm, and its 22 mm hole
  // lands 0.76 mm under the motor axis, clear of the motor's rear boss. Its other 43.9 mm pair is
  // 9.64 mm lower on the plate and would lift it that much more. The plate keeps its 1.61 mm gap
  // behind the motor's rear face (Y 151.20), inside chirpy's 1 to 2 mm; how the owner spaced it without
  // the printed spacers is in no photo.
  // Second pass (2026-09-23): it stays hung on the Sherpa's frame screws, so it moves with the Sherpa
  // onto its real mount (SHERPA_MOVE). This way up, its lower end then runs through the carriage, the
  // X block and the rail. Upside down, its two lower lobes land on the carriage's two ear inserts
  // and its other 43.9 mm pair on the frame screws, as chirpy draws it (pcb_toolhead.png), but no
  // photo of our shows which way up his is, so it is left for him (WO-17 stop report).
  // our answer, 2026-09-23: "The tall tower points up on my printer; that's where the umbilical
  // goes." That overrides item 7's orientation: upside down from it, a half turn about Y, so the
  // bosses still face the rear. Measured as placed: its lower lobes' 3.4 mm holes on the carriage's
  // ear inserts (X 114.13 / 139.83, Z 98.07), its plate on their rear face at Y 168.06, its other
  // 43.90 mm pair at X 105.05 / 148.95, Z 131.96 on the Sherpa's frame screws (X 105.08 / 148.92,
  // Z 131.81), and no overlap with the carriage, the X block, the rail or the Sherpa.
  const CARRIER_AT=[-20.70,168.06,43.35];
  paintBlack(await placeSource('stella-ebb42-carrier','EBB42 carrier · printed',g=>{g.rotateX(-Math.PI/2);g.translate(...CARRIER_AT);},'Printed 2026-09-14, carrier only, no spacers. Tall tower up, where the umbilical goes (2026-09-23): lower lobes on the carriage\'s two ear inserts, plate on their rear face at Y 168.06, its other 43.90 mm hole pair on the Sherpa Mini\'s frame screws'));
  for(const part of FITTED){const m=await placeSource(part.id,part.name,g=>{part.at&&centreOn(g,part.ops||[],part.move?shift(part.move,part.at):part.at);},part.note,part.fan?{fanKey:part.fan}:{});if(m&&/sherpa|ebb42|fan-3010/.test(part.id))paintBlack(m);if(m&&part.id==='btt-ebb42-v12')m.material.color.setHex(0x14231c);if(m&&part.fan)spinners.push({mesh:m,key:part.fan,speed:0,base:m.material.color.getHex()});}
  // The reference CAD paints the V6 heatsink green; our is bare aluminium.
  for(const m of meshes)if(/^head\.(heatsink|heatbreak)$/.test(m.userData.slot_id||'')){m.material.color.setHex(0xb9bec4);m.material.metalness=.6;m.material.roughness=.35;}
  // Part cooling: two real 4010 blowers in the cowl's side windows, both driven by [fan]. the owner
  // supplied the STL on 2026-09-21 (40.00 x 40.00 x 10.00 mm, 8656 facets, measured on import).
  // Until then the HUD built one from the 4010 datasheet out of primitives, which is what he kept
  // seeing as "a random box fan".
  // Position measured off the cowl rather than typed: its side window spans Z 43.0..77.2 and
  // Y 104.2..139.1, so the blowers sit on that window's centre. The old hand-typed Z of 72 put them
  // up at the top edge of the opening, which is the "too high up" he called out.
  // The STL's thin axis is Z, so it gets a quarter turn about Y to lie flat on the side wall.
  const blowerSource=await inspectSource('fan-4010-blower').catch(()=>null);
  if(blowerSource){
   const donor=entries.get(blowerSource),record=sourceAssets.find(a=>a.id==='fan-4010-blower')||null;
   // 2026-09-21, live, looking at the pair: "the outside is inside and inside is outside" — the
   // intake face pointed into the cowl on both. WO-09 had mirrored the right one about its own X to
   // make a hand pair; that was the wrong correction and is gone. Both blowers now turn 180° about
   // world Z in place (a real rotation, no winding to reverse), which puts the intake face outboard
   // on each side. The outlet reads off the mesh as the STL's +Y wall (68% clear of material, clear
   // area centred X 20.0 / Z 0.3); the quarter turn about Y left that at world +Y, so after the half
   // turn about Z it points world -Y, toward the front of the printer. the owner judges on the next look —
   // if the outlet is then the wrong way, the fix is an in-place X mirror instead of this rotation,
   // which flips the face and keeps the outlet.
   // Fable review 2026-09-21: two identical clones turned the same way put both intakes on the same
   // world side, so only the left one came out outboard. The right one is mirrored about its own X
   // after the turn: intake outboard on both, outlet still toward the front on both. Real blowers
   // come as a handed pair, so the mirror is the physical truth, not a fudge.
   // 2026-09-21, second look: "your fans are turned around the right way, but the exhaust of the
   // fan is pointing towards the front instead of down." A part cooling blower has to blow at the
   // nozzle. The outlet was at world -Y after the half turn about Z; a quarter turn about world X
   // carries -Y to -Z without touching the intake face, which stays outboard, and the 40 x 40 face
   // stays square to the cowl's side window so the measured placement still holds.
   for(const [side,x]of [['left',102.4],['right',149.5]]){
    const g=donor.geometry.clone();g.rotateY(Math.PI/2);g.rotateZ(Math.PI);g.rotateX(Math.PI/2);if(side==='right')mirrorXInPlace(g);centreOn(g,[],onCowl([x,121.7+BLOWER_BACK_MM,60.1]));g.computeBoundingBox();
    const m=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x1b1d20,metalness:.2,roughness:.7,side:THREE.DoubleSide}));
    const id='stella:blower-'+side;
    m.userData={id,record_id:instanceRecordId(id),name:`Part cooling blower · ${side} 4010`,full:'Stella / Toolhead (installed) / Part cooling blower '+side,asset:record,slot_id:'head.part_fan',layer:'toolhead',motion:'toolhead',fanKey:'fan',center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),colorGroup:'toolhead',placement:{status:'Centred on the cowl side window measured at Y 121.7 / Z 60.1, then '+BLOWER_BACK_MM+' mm toward the rear at the owner\'s call 2026-09-23, carried with the cowl onto the hotend mount; turned 180° about world Z so the intake face is outboard, then a quarter turn about world X at the owner\'s call 2026-09-21 so the outlet points world -Z, down at the nozzle; mounting fit unverified'}};
    root.add(m);meshes.push(m);entries.set(id,m);installed.push(m);spinners.push({mesh:m,key:'fan',speed:0,base:0x1b1d20});}
  }
  fitHotend();
  canvas.dataset.cowlFasteners=String(fitFasteners());
  // The sock is what you can actually see of the hotend through the cowl, so it is what has to glow;
  // block and cartridge are suppressed by the Rapid Burner and glowed where nobody could see them.
  heatBlock=meshes.filter(m=>/^head\.(block|nozzle|heatbreak|cartridge|core|sock)$/.test(m.userData.slot_id||'')&&!m.userData.replacedByRapid);bedSheet=meshes.filter(m=>/^bed\.(sheet|plate)$/.test(m.userData.slot_id||''));
  canvas.dataset.customToolhead=source.source_sha256;canvas.dataset.installedParts=String(installed.length);
 }
 // Reported position drives the meshes. Datum: nozzle tip in the CAD rest pose sits at (127,130.81,26.0) (on the
 // Rapid Burner mount's bore axis since WO-17 item 1, NOZZLE_BELOW_COWL_MM under the cowl since our
 // measurement on 2026-09-23) with the bed centre at (127,127) and the sheet top at Z≈0, so Klipper
 // (175,178.81,26.1) is the rest pose.
 // Approximate by construction (reference CAD, not a measured Stella); unhomed axes stay at rest.
 function establishMotion(){const nozzle=meshes.find(m=>m.userData.slot_id==='head.nozzle');const sheet=meshes.find(m=>m.userData.name==='Flex Sheet:1');const bed=sourceData?.printer_frame?.bed_center;if(!nozzle||!bed)return;
  const tip=nozzle.geometry.boundingBox;const z0=sheet?sheet.geometry.boundingBox.max.z:bed[2];sourceData.printer_frame.source_pose=[175+(tip.min.x+tip.max.x)/2-bed[0],175+(tip.min.y+tip.max.y)/2-bed[1],tip.min.z-z0];sourceData.printer_frame.motion_basis='Nozzle tip in reference CAD mapped to Klipper bed centre 175,175 and sheet top Z=0; approximate';
  for(const m of meshes){const d=m.userData;if(d.motion&&d.motion!=='fixed')continue;const s=d.slot_id||'';d.motion=/^head(\.|$)/.test(s)?'toolhead':s==='x_rail'?'x_axis':(d.layer==='gantry'&&/^(gantry|drive_a|drive_b)$/.test(s))?'gantry':'fixed';}
  // 2026-09-21, watching it move: the Y rails slid along in Y with the X beam, and the Z
  // carriages on the frame verticals stayed put while the gantry rose. Both classes come from the
  // stored asset mapping rather than from here, so they are corrected against the real machine:
  //   "MGN9 - 300mm (6)/(7)" under the XY joints ARE the gantry's Y rails. The joints slide along
  //   them, so the rails themselves only ever travel in Z. They were 'x_axis', which is what
  //   dragged them along in Y with the toolhead.
  //   The MGN9H carriages on the frame verticals are bolted to the gantry's Z belt clamps, so they
  //   ride up and down with it. They were 'fixed', so they stayed behind as the gantry moved. The
  //   vertical rails they run on stay 'fixed', which is why the path is matched and not just the name.
  for(const m of meshes){const d=m.userData,n=d.name||'',f=d.full||'';
   // 2026-09-25: "two random screws sitting in mid-air." The rail-to-extrusion M3 screws in the
   // MGN9 Y rails' own subtree stayed 'x_axis' and slid along Y with the beam; they stay with the rail.
   if((/^MGN9 - 300mm/.test(n)||(/^M3x\d+ SHCS/.test(n)&&/\/ MGN9 - 300mm/.test(f)))&&/XY Joint - (Left|Right)/.test(f))d.motion='gantry';
   else if(/^MGN9H/.test(n)&&/Verticals Extrusions/.test(f))d.motion='gantry';
   // 2026-09-21: "the print head's not staying on the piece that's on the rail." Measured in the
   // loaded scene, the MGN12H block is the nine generic "PRODUCT_NAME_n" meshes under the rail,
   // X 104.3..149.7, Y 160..170, Z 64.2..91.2 — a 45 x 10 x 27 block wrapping the rail, i.e. the
   // part the toolhead is bolted to. They were 'x_axis', so they stayed on the beam while the
   // carriage plates rode off with the head. The six M3x8 SHCS in the same subtree sit at
   // X -13, 37, 87, 162, 212, 262 and reach up into the beam: those are the rail-to-beam screws
   // and stay 'x_axis'.
   else if(/^PRODUCT_NAME_/.test(n)&&/X Axis \(MGN12\)/.test(f)&&/MGN12 rail/.test(f))d.motion='toolhead';}
  // 2026-09-21: "the rail and the X bar are still not moving together." The X beam is not in the
  // gantry group at all — "Frame:1 / Gantry Extrusions:1" holds the whole gantry rectangle and all
  // 59 of its meshes came in 'fixed'. HFSB5-20-330 (X -88..342, Y 173..193, Z 67.7..87.7) is the X
  // beam and must travel in Y and Z with the MGN12 rail bolted to its front face; the other three
  // (HFSB5-2020-240 at the rear, two HFSB5-2020-350 down the sides) are the gantry frame and only
  // rise in Z. The 55 T-nuts and screws belong to whichever member holds them, so the split is by
  // bounding box rather than by name: 14 travel with the beam, 45 with the frame.
  const extrusions=meshes.filter(m=>/Gantry Extrusions/.test(m.userData.full||''));
  const beam=extrusions.map(m=>m.geometry.boundingBox).reduce((a,b)=>!a||b.max.x-b.min.x>a.max.x-a.min.x?b:a,null);
  if(beam)for(const m of extrusions){const b=m.geometry.boundingBox,pad=8;
   m.userData.motion=['x','y','z'].every(k=>b.min[k]>=beam.min[k]-pad&&b.max[k]<=beam.max[k]+pad)?'x_axis':'gantry';}
  // 2026-09-21, live: "the belts don't move with the X beam." A Belt:1 and B Belt:1 are each a
  // single mesh carrying a whole CoreXY loop — X -107..361, Y -65..420 — arriving layer:'gantry'
  // with no slot_id, so the rule above left them 'fixed' and the entire loop stood still while the
  // beam slid out from under it. One loop cannot be one joint: the runs across the gantry travel in
  // Y with the beam, while the runs down the side rails only change length, which a rigid mesh
  // cannot show, and rise in Z with the gantry like everything else on it. So each loop is split in
  // two by triangle centroid Y against the gantry band, the way the Gantry Extrusions were split by
  // bounding box. The band is the MGN12 rail's own Y span opened out to reach the belt clamp on the
  // carriage (30 mm in front, 20 mm behind), measured rather than typed. The gantry run becomes its
  // own mesh on 'x_axis'; what is left keeps the original mesh on 'gantry'.
  const beltSplit=[];
  const railBox=meshes.filter(m=>/MGN12 rail/.test(m.userData.full||'')).reduce((a,m)=>a?a.union(m.geometry.boundingBox.clone()):m.geometry.boundingBox.clone(),null);
  if(railBox)for(const belt of meshes.filter(m=>/^[AB] Belt:/.test(m.userData.name||''))){
   const g=belt.geometry,ix=g.getIndex().array,pos=g.attributes.position.array,lo=railBox.min.y-30,hi=railBox.max.y+20,on=[],off=[];
   for(let t=0;t<ix.length;t+=3){const cy=(pos[ix[t]*3+1]+pos[ix[t+1]*3+1]+pos[ix[t+2]*3+1])/3;(cy>=lo&&cy<=hi?on:off).push(ix[t],ix[t+1],ix[t+2]);}
   if(!on.length||!off.length)continue;
   // computeBoundingBox() walks the position attribute and ignores the index, and both halves share
   // one position buffer, so each box is taken over its own triangles or they would both read as the
   // whole loop.
   const boxOf=list=>{const b=new THREE.Box3(),v=new THREE.Vector3();for(const i of list)b.expandByPoint(v.set(pos[i*3],pos[i*3+1],pos[i*3+2]));return b;};
   const before={name:belt.userData.name,motion:belt.userData.motion,bbox:g.boundingBox.clone()};
   const run=new THREE.BufferGeometry();run.setAttribute('position',g.attributes.position.clone());if(g.attributes.normal)run.setAttribute('normal',g.attributes.normal);
   run.setIndex(on);run.boundingBox=boxOf(on);g.setIndex(off);g.boundingBox=boxOf(off);
   belt.userData.motion='gantry';belt.userData.center=g.boundingBox.getCenter(new THREE.Vector3());
   const id=belt.userData.id+':gantry-run',m=new THREE.Mesh(run,belt.material.clone());
   m.userData={...belt.userData,id,record_id:instanceRecordId(id),name:belt.userData.name.replace(/:\d+$/,'')+' · gantry run',
    full:belt.userData.full+' / gantry run',motion:'x_axis',center:run.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3(),
    placement:{status:'Split from the belt loop by triangle centroid Y inside the gantry band; the loop changes length in life and this half is drawn rigid'}};
   // 2026-09-25: the belt loops at the toolhead sat still in the middle of the beam. The two
   // strands along the beam end at the carriage clamps, so they stretch and shorten as the head
   // moves and the clamp loops ride with it (strandBelts, every pose).
   m.userData.strand={orig:Float32Array.from(run.attributes.position.array),x0:run.boundingBox.min.x,x1:run.boundingBox.max.x,y0:railBox.min.y-25,y1:railBox.max.y+15};m.frustumCulled=false;
   root.add(m);meshes.push(m);entries.set(id,m);
   beltSplit.push({belt:before.name,was:before.motion,band:[Number(lo.toFixed(2)),Number(hi.toFixed(2))],
    beam_tris:on.length/3,side_tris:off.length/3,
    beam_bbox:[run.boundingBox.min.toArray(),run.boundingBox.max.toArray()].map(a=>a.map(v=>Number(v.toFixed(2)))),
    side_bbox:[g.boundingBox.min.toArray(),g.boundingBox.max.toArray()].map(a=>a.map(v=>Number(v.toFixed(2))))});}
  canvas.dataset.beltSplit=JSON.stringify(beltSplit);
  // 2026-09-21: "the cable chain on the X rail isn't moving at all." The X chain is suppressed
  // (Stella runs an umbilical), so what he is watching is the Y chain, and all eighteen of its links
  // arrived 'gantry' — Z only — so the whole chain rode up and down and never followed the beam in
  // Y. It is not one mesh: each link is its own, which is what makes the split honest. The chain is
  // folded in the CAD with two runs one above the other; the upper run carries the moving end bolted
  // to the right XY joint's chain bridge (screws at Y 178..192, Z 131..137) and travels with the
  // beam, the lower run lies on the frame rail and does not move at all. Split on the fold: a link
  // whose box starts above the fold plane is on the moving run. The U-turn between them is drawn
  // rigid and will open a gap as the gantry travels, because a link that rolls cannot be shown by a
  // rigid mesh; the owner judges whether that is better than a chain that never moves.
  // 2026-09-24: "the cable chain is cut in half and sliding, not moving with the X bar." The
  // split above drew the lower run 'fixed', so it stayed behind when the gantry rose, and the U-turn
  // was rigid. A drag chain rolls: every link keeps its place along the chain, the lower run lies
  // still on the gantry, the moving end travels with the XY joint and the bend travels at half that
  // speed. So each link now gets its arc position s along the chain's centre line (lower run, half
  // circle, upper run in the Y-Z plane) and applyPose places it at the same s on the bent path for
  // the current Y, turned by how far round the bend it has rolled, and lifted with the gantry in Z.
  const chain=meshes.filter(m=>/\/ Y Cable Chain:/.test(m.userData.full||''));
  const chainSplit=[];
  if(chain.length){const cs=chain.map(m=>m.geometry.boundingBox.getCenter(new THREE.Vector3()));
   const links=cs.filter((_,i)=>!/FHCS|screw/i.test(chain[i].userData.name||'')),zb=Math.min(...links.map(c=>c.z)),zt=Math.max(...links.map(c=>c.z)),zc=(zb+zt)/2,R=(zt-zb)/2;
   const bend=links.filter(c=>c.z>zb+1&&c.z<zt-1),y0=Math.min(...links.map(c=>c.y));let yc=y0,best=Infinity;
   for(let y=y0;y<y0+2*R;y+=0.1){const d=bend.map(c=>Math.hypot(c.y-y,c.z-zc)),m=d.reduce((a,b)=>a+b,0)/Math.max(1,d.length),v=d.reduce((a,b)=>a+(b-m)**2,0);if(v<best){best=v;yc=y;}}
   const top=links.filter(c=>c.z>=zt-1),yEnd=Math.max(...top.map(c=>c.y));
   // 2026-09-24, the front 70 mm: the reference chain is four links too short for 350 mm of
   // travel (at Y 0 the bend would have to sit behind the joint it feeds), so the drawn chain is
   // lengthened by whole links copied from its lower run, inserted where the lower run meets the
   // bend. The top run then never runs out, and the bend sits that much further forward.
   const lower=links.filter(c=>c.z<zc&&c.y>yc).map(c=>c.y).sort((a,b)=>a-b),pitch=lower.length>1?Math.abs(lower[1]-lower[0]):20;
   const datumY=sourceData?.printer_frame?.source_pose?.[1]??175,need=2*(8-(yEnd-yc)+datumY/2),extra=need>0?Math.ceil(need/pitch)*pitch:0;
   chainPath={zc,R,yc,extra,dyMin:-2*(yEnd-yc)-extra};
   const sOf=c=>c.y>=yc?(c.z<zc?-c.y:-yc+Math.PI*R+(c.y-yc)):-yc+R*Math.atan2(-(c.y-yc),-(c.z-zc));
   for(const [i,m]of chain.entries()){const c=cs[i],d=m.userData,was=d.motion,s=sOf(c);
    d.motion='chain';d.chainS0=s;d.chainS=s>-yc?s+extra:s;chainSplit.push({link:d.name,was,now:'chain',s:Number(s.toFixed(2))});}
   const srcI=cs.findIndex((c,i)=>!/FHCS|screw/i.test(chain[i].userData.name||'')&&c.z<zc&&Math.abs(c.y-lower[0])<0.5);
   if(srcI>=0)for(let k=0;k<extra/pitch;k++){const src=chain[srcI],m=new THREE.Mesh(src.geometry,src.material),d=src.userData,id=d.id+'#added-'+k;
    m.userData={...d,id,name:d.name+' (drawn extra link)',motionOffset:new THREE.Vector3(),center:d.center.clone(),chainS0:d.chainS0,chainS:-yc+pitch*(k+.5),
     placement:{status:'Drawn: the reference chain is too short for 350 mm of Y travel, so links are added to the lower run'}};
    src.parent.add(m);meshes.push(m);entries.set(id,m);}
   canvas.dataset.chainExtra=String(extra);
   canvas.dataset.chainFold=String(Number(zc.toFixed(2)));}
  canvas.dataset.chainSplit=JSON.stringify(chainSplit);
  {const zc=meshes.filter(m=>/\/ IGUS Z Chain:/.test(m.userData.full||''));
   if(zc.length){const cs=zc.map(m=>m.geometry.boundingBox.getCenter(new THREE.Vector3())),links=cs.filter((_,i)=>/^0 \(/.test(zc[i].userData.name||''));
    const xs=links.map(c=>c.x),xa=Math.min(...xs),xb=Math.max(...xs),wc=(xa+xb)/2,R=(xb-xa)/2;
    const straight=links.filter(c=>Math.abs(c.x-xa)<1||Math.abs(c.x-xb)<1),top=Math.max(...straight.map(c=>c.z)),bend=links.filter(c=>Math.abs(c.x-xa)>=1&&Math.abs(c.x-xb)>=1);
    let uc=top,best=Infinity;for(let u=top-R;u<top+R;u+=.1){const d=bend.map(c=>Math.hypot(c.x-wc,c.z-u)),mn=d.reduce((a,b)=>a+b,0)/Math.max(1,d.length),v=d.reduce((a,b)=>a+(b-mn)**2,0);if(v<best){best=v;uc=u;}}
    const moving=links.filter(c=>Math.abs(c.x-xb)<1).map(c=>c.z),um=Math.min(...moving),pitch=20;
    // The reference chain covers ~180 mm of Z; the gantry travels to Z 310. Whole links are added
    // to the fixed run where it meets the bend, so the moving run never runs out.
    const datumZ=sourceData?.printer_frame?.source_pose?.[2]??30,maxDz=310-datumZ,need=maxDz+2*8-2*(uc-um),extra=need>0?Math.ceil(need/pitch)*pitch:0;
    zChain={wc,R,uc,extra,y:cs.reduce((a,c)=>a+c.y,0)/cs.length};const sOf=c=>c.z<uc?(c.x<wc?c.z:uc+Math.PI*R+(uc-c.z)):uc+R*Math.atan2(c.z-uc,wc-c.x);
    for(const [i,m]of zc.entries()){const s0=sOf(cs[i]);m.userData.motion='zchain';m.userData.chainS0=s0;m.userData.chainS=s0>uc?s0+extra:s0;}
    const src=zc[cs.findIndex((c,i)=>/^0 \(/.test(zc[i].userData.name||'')&&Math.abs(c.x-xa)<1&&Math.abs(c.z-top)<.5)];
    if(src)for(let k=0;k<extra/pitch;k++){const m=new THREE.Mesh(src.geometry,src.material),d=src.userData,id=d.id+'#added-'+k;
     m.userData={...d,id,name:d.name+' (drawn extra link)',motionOffset:new THREE.Vector3(),center:d.center.clone(),chainS0:d.chainS0,chainS:uc+pitch*(k+.5),placement:{status:'Drawn: the reference Z chain is too short for 310 mm of Z travel'}};
     src.parent.add(m);meshes.push(m);entries.set(id,m);}
    canvas.dataset.zChain=JSON.stringify({links:zc.length,extra,uc:Number(uc.toFixed(1)),R:Number(R.toFixed(1))});}}
  // Fan rotors: the hotend 3010 blows back (+Y) through the heatsink, its intake on the front face;
  // the blowers take air in on their outboard faces (world X). The skirt 6020s have impellers.
  for(const sp of spinners){const b=sp.mesh.geometry.boundingBox,c=b.getCenter(new THREE.Vector3()),sz=b.getSize(new THREE.Vector3());
   if(sp.key==='hotend_fan')sp.rotor=addRotor(sp.mesh,new THREE.Vector3(c.x,b.min.y+2.5,c.z),new THREE.Vector3(0,-1,0),Math.min(sz.x,sz.z)*.4);
   else if(sp.key==='fan'){const left=/left/.test(sp.mesh.userData.name||'');sp.rotor=addRotor(sp.mesh,new THREE.Vector3(left?b.min.x+1.6:b.max.x-1.6,c.y,c.z+2),new THREE.Vector3(left?-1:1,0,0),Math.min(sz.y,sz.z)*.28);}}
  for(const m of meshes.filter(m=>/6020 Impeller/.test(m.userData.name||'')&&/\/ Skirt/.test(m.userData.full||''))){const b=m.geometry.boundingBox,sz=b.getSize(new THREE.Vector3()),axis=sz.x<=sz.y&&sz.x<=sz.z?new THREE.Vector3(1,0,0):sz.y<=sz.z?new THREE.Vector3(0,1,0):new THREE.Vector3(0,0,1);
   m.userData.turning=true;spinners.push({mesh:m,key:'controller_fan',speed:0,base:m.userData.baseColor,turn:{axis,centre:b.getCenter(new THREE.Vector3())}});}
  canvas.dataset.fanRotors=String(spinners.filter(s=>s.rotor||s.turn).length);
  canvas.dataset.motion='mapped';}
 // 2026-09-24: "the fans don't move, they just flash." A fan now turns: each gets a rotor on its
 // intake (the fan STLs are one solid, so the blades are drawn), the skirt's 6020s turn their own
 // impellers, all at a visible fraction of the commanded speed, about 30 frames a second while any
 // runs. The flash is gone; a fan keeps a light cool tint while it runs.
 let spinFrame=0,spinAt=0;
 function spin(now){spinFrame=0;const dt=Math.min(.1,(now-(spinAt||now))/1000);if(now-spinAt<33){spinFrame=requestAnimationFrame(spin);return;}spinAt=now;let any=false;
  for(const s of spinners){if(!(s.speed>0))continue;any=true;s.angle=((s.angle||0)+dt*s.speed*2*Math.PI*2.2)%(2*Math.PI);
   if(s.rotor){s.rotor.rotation.z=s.angle;}
   if(s.turn){const q=new THREE.Quaternion().setFromAxisAngle(s.turn.axis,s.angle),c=s.turn.centre;s.mesh.quaternion.copy(q);s.mesh.userData.motionOffset.copy(c).sub(c.clone().applyQuaternion(q));}}
  if(any){schedule();spinFrame=requestAnimationFrame(spin);}}
 // A drawn rotor: hub and seven swept blades in the XY plane, turned to face along `axis`.
 function addRotor(mesh,centre,axis,radius){const g=new THREE.Group(),mat=new THREE.MeshStandardMaterial({color:0x16191c,metalness:.25,roughness:.6});
  // Thin swept blades set inside the fan's opening, dark like the fan body (2026-09-25: the first
  // rotors were chunky blocks sticking out of the cowl).
  const hub=new THREE.Mesh(new THREE.CylinderGeometry(radius*.24,radius*.24,1.4,24),mat);hub.rotation.x=Math.PI/2;g.add(hub);
  for(let i=0;i<11;i++){const b=new THREE.Mesh(new THREE.BoxGeometry(radius*.46,radius*.2,.3),mat);const a=i/11*Math.PI*2;b.position.set(Math.cos(a)*radius*.74,Math.sin(a)*radius*.74,0);b.rotation.set(.65,0,a+.5);g.add(b);}
  g.position.copy(centre);g.quaternion.setFromUnitVectors(new THREE.Vector3(0,0,1),axis.clone().normalize());
  const inner=new THREE.Group();while(g.children.length)inner.add(g.children[0]);g.add(inner);g.traverse(o=>{o.raycast=()=>{};});mesh.add(g);return inner;}
 // 2026-09-21: "if it's heating up, that should be glowing … the hot end should be glowing as
 // well," and separately "my print bed is now a different colour." Both are this function. Nothing
 // repainted the plate between the two builds — the purple build-plate default has been in
 // DEFAULT_GROUP_COLORS since 2026-09-20, before WO-09 shipped — what changed is that the bed is at
 // 80 °C now and was at 49 °C then, and the old glow simply added an orange emissive over the whole
 // plate, which at 80 °C washes purple to pink and reads as a repaint. So heat now moves the part's
 // own colour toward hot metal as well as lighting it, over a range that matches the machine: the
 // nozzle and its sock from 150 to 250 °C, the plate from 40 to 80 °C. Cold parts are untouched.
 // The blowers and the hotend fan run the same lerp the other way, toward cool blue with fan speed,
 // because the emissive pulse alone was invisible on a near-black 4010 (the owner: item 16).
 const HOT_METAL=new THREE.Color(0xff3c10),HOT_PLATE=new THREE.Color(0xd4452a),COOL_AIR=new THREE.Color(0x4fc3ff);
 function live(p){if(!heatBlock.length&&!bedSheet.length)return;const fans=p?.fans||{},speeds=spinners.map(s=>{const f=fans[s.key]?.speed;return typeof f==="number"&&f>0?f:0;}),heat=(t,lo,hi)=>typeof t==='number'&&Number.isFinite(t)?Math.max(0,Math.min(1,(t-lo)/(hi-lo))):0,nozzle=heat(p?.nozzle?.temperature,150,250),bed=heat(p?.bed?.temperature,40,80),visual=JSON.stringify([nozzle,bed,...speeds]);if(visual===lastLiveVisual)return;lastLiveVisual=visual;
  const tint=(m,k,to,lerp,hex,emissive)=>{const base=m.userData.baseColor;if(base!==undefined)m.material.color.setHex(base).lerp(to,k*lerp);m.material.emissive.setHex(hex).multiplyScalar(k*emissive);};
  for(const [i,s]of spinners.entries()){s.speed=speeds[i];if(!s.turn)tint(s.mesh,s.speed,COOL_AIR,.35,0x1f6fb8,.18);}
  for(const m of heatBlock)tint(m,nozzle,HOT_METAL,.85,0xff5a14,1);
  // The plate is the biggest surface in the scene and the toolpath is drawn a fraction of a
  // millimetre above it, so it takes a much gentler share than the hotend: at 80 °C it has to read
  // as hot dark metal and still stay darker than the freshest segment of the ramp (#ffc24a), or the
  // print disappears into the plate. Measured at 80 °C: plate #7d392c, ramp head #ffc24a.
  for(const m of bedSheet)tint(m,bed,HOT_PLATE,.45,0xff2a08,.28);
  if(spinners.some(s=>s.speed>0)){if(!spinFrame)spinFrame=requestAnimationFrame(spin);}else schedule();}
 // WO-17 (2026-09-23): source meshes of a fitted CAD that are not on Stella. BTT's EBB42 CAD ships
 // four M3x20+6 hex standoffs for mounting on a motor; our board sits on the chirpy carrier's
 // inserts, and those standoffs would stick 20 mm forward through the carrier, the upper two into
 // the Sherpa's motor.
 // WO-17 item 1, second pass (2026-09-23): the Annex Sherpa Mini STEP carries its front plate twice,
 // NAUO25 and NAUO26, the second with a tab hanging 8 mm below the feet. With the Sherpa on its
 // mount that tab would pass through the cowl's 1.92 mm top plate, which no real one can, so it is left out.
 const SOURCE_SKIP={'btt-ebb42-v12':/^M3x20\+6$/};SOURCE_SKIP['annex-sherpa-mini']=/^NAUO26$/;
 // U03 a (2026-10-20): the geometry of an asset is the same bytes until the library is imported again, and then its sha changes. The 124 MB frame is too big for the browser's HTTP cache (it keeps
 // nothing over about a sixteenth of its size) and the service sends no-store, so every page open downloaded it again. It is kept in the browser's Cache Storage, under the asset and its sha (one copy
 // for every printer that has it; the older sha of the same asset is dropped), and read from there next time. Where the page is not a secure context (a phone on the LAN address) there is no
 // Cache Storage and it downloads as before. The farm wall's tiles do not keep it (many printers, many copies).
 async function assetJson(a,fail){
  const sha=a.source_sha256,key='/__stella-asset/'+encodeURIComponent(a.id)+'/'+sha;let store=null;
  if(!tileMode&&sha&&typeof caches!=='undefined'){try{store=await caches.open('stella-assets-v1');const hit=await store.match(key);if(hit){canvas.dataset.assetCache=(canvas.dataset.assetCache||'')+' hit:'+a.id;return await hit.json();}}catch{store=null;}}
  const r=await fetch(apiUrl(a.geometry_url));if(!r.ok)throw Error(fail);
  if(store){const copy=r.clone();store.put(key,copy).then(()=>store.keys()).then(ks=>{const mine='/__stella-asset/'+encodeURIComponent(a.id)+'/';for(const k of ks){const p=new URL(k.url).pathname;if(p.startsWith(mine)&&p!==key)store.delete(k);}}).catch(()=>{});}
  return r.json();}
 // U03 a (2026-10-20): the printer page waited on a chain. The 124 MB frame came first, was parsed and built, and only then did the toolhead's twelve parts (33 MB) start, one after the other
 // (measured: all 12 requests started after 2.3 s and the last ended at 4.2 s of a 4.8 s load). They are asked for as soon as the library list is known, beside the frame, and
 // inspectSource takes them from here; a part is kept only until it has been used.
 const partFetch=new Map();
 const partData=a=>{let p=partFetch.get(a.geometry_url);if(!p){p=assetJson(a,'Component source geometry is unavailable.');partFetch.set(a.geometry_url,p);p.catch(()=>partFetch.delete(a.geometry_url));}return p;};
 const TOOLHEAD_PARTS=['stella-rapid-bltouch-cleaned','chirpy-mgn12h-x-carriage-lugs','chirpy-carriage-base-short','chirpy-rapid-burner-v6-mount-rear','chirpy-rapid-burner-v6-mount-front','stella-sherpa-mini-mount','stella-ebb42-carrier','annex-sherpa-mini','btt-ebb42-v12','bltouch-probe','fan-3010-axial','fan-4010-blower'];
 async function inspectSource(assetId){const source=sourceAssets.find(a=>a.id===assetId);if(!source)throw Error('Source file is not in the managed library.');const id=source.id+':root:0';if(entries.has(id))return id;const data=await partData(source);partFetch.delete(source.geometry_url);
  const parts=(data.meshes||[]).filter(m=>!SOURCE_SKIP[assetId]?.test(m.name||''));if(!parts.length)throw Error('That source file has no display geometry.');
  // One fitted component = one selectable mesh, so a 460-part board does not become 460 clickable screws.
  let position,index;
  if(parts.length===1){position=Float32Array.from(parts[0].attributes.position.array);index=Uint32Array.from(parts[0].index.array);}
  else{const total=parts.reduce((n,m)=>n+m.attributes.position.array.length,0),tris=parts.reduce((n,m)=>n+m.index.array.length,0);
   position=new Float32Array(total);index=new Uint32Array(tris);let po=0,io=0,vo=0;
   for(const part of parts){const a=part.attributes.position.array;position.set(a,po);for(let i=0;i<part.index.array.length;i++)index[io+i]=part.index.array[i]+vo;po+=a.length;io+=part.index.array.length;vo+=a.length/3;}}
  const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(position,3));g.setIndex(new THREE.BufferAttribute(index,1));g.computeVertexNormals();g.computeBoundingBox();
  const m=new THREE.Mesh(g,new THREE.MeshStandardMaterial({color:0x82949c,roughness:.48,metalness:.15,side:THREE.DoubleSide}));
  m.userData={id,record_id:instanceRecordId(id),name:source.name,full:source.notes,asset:source,slot_id:source.slot_id,layer:'toolhead',sourceOnly:true};entries.set(id,m);return id;}

 function preview(id,target){mini?.dispose();mini=null;target.replaceChildren();const original=entries.get(id);if(!original)return;const cv=document.createElement('canvas');cv.setAttribute('aria-label','Selected component source model');target.append(cv);// WO-19 (2026-09-23), reviewer: a black part on a near-black well could not be read. The preview is
  // its own studio now: it clears to transparent over a lighter cyclorama well (hud.css), tone maps
  // like the scene, and lights the part with a key, a fill and a rim from behind that edges dark
  // parts against the well. The part's own colour is never changed.
  const r=new THREE.WebGLRenderer({canvas:cv,antialias:true,alpha:true});r.setPixelRatio(Math.min(devicePixelRatio,1.5));r.setClearColor(0x000000,0);r.outputColorSpace=THREE.SRGBColorSpace;r.toneMapping=THREE.ACESFilmicToneMapping;r.toneMappingExposure=1.5;const s=new THREE.Scene(),c=new THREE.PerspectiveCamera(36,1,.01,100000);c.up.set(0,0,1);s.add(new THREE.HemisphereLight(0xffffff,0x3a4448,2.2));
  for(const [color,intensity,x,y,z]of [[0xffffff,3.2,1,-1.2,2],[0xdfeaf0,1.3,-1.5,-.6,.8],[0xffffff,3,-.8,1.6,1.4]]){const light=new THREE.DirectionalLight(color,intensity);light.position.set(x,y,z);s.add(light);}const mesh=new THREE.Mesh(original.geometry,original.material.clone());mesh.material.emissive.setHex(0);s.add(mesh);const box=new THREE.Box3().setFromObject(mesh),center=box.getCenter(new THREE.Vector3()),diam=box.getSize(new THREE.Vector3()).length();c.position.copy(center).addScaledVector(new THREE.Vector3(1,-1,.7).normalize(),diam*1.9);const controls=new OrbitControls(c,cv);controls.target.copy(center);controls.update();const draw=()=>{const w=target.clientWidth,h=target.clientHeight;if(w&&h){r.setSize(w,h,false);c.aspect=w/h;c.updateProjectionMatrix();r.render(s,c);}};controls.addEventListener('change',draw);const observer=new ResizeObserver(draw);observer.observe(target);draw();const initialPosition=c.position.clone();mini={theme(){draw();},refreshColor(){mesh.material.color.copy(original.material.color);draw();},get distance(){return c.position.distanceTo(controls.target);},zoom(scale){c.position.sub(controls.target).multiplyScalar(scale).add(controls.target);controls.update();draw();},reset(){controls.target.copy(center);c.position.copy(initialPosition);controls.update();draw();},dispose(){observer.disconnect();controls.dispose();mesh.material.dispose();r.dispose();}};}
  // The scene follows the interface theme: a lit studio ground in light, neon-lit dark otherwise.
  // WO-12 state staging: the key light leans with what the machine is doing — warm while heating,
  // cool-bright while printing, magenta-washed on an error, calm when idle. Subtle on purpose: the
  // brand neons stay accents in the chrome, never a flood on the model. theme() re-applies the
  // current stage after it rewrites the light colours, so the two never fight.
  let staged='idle';
  function applyStage(){const key=world.children.find(c=>c.isDirectionalLight&&!c.userData.stageLight);if(!key)return;
   const light=document.body.dataset.theme==='light';
   if(staged==='heating')key.color.set(light?0xfff4e6:0xffeddc);
   else if(staged==='error')key.color.set(0xff2d92);
   else key.color.set(light?0xffffff:0xf4f8ff);
   key.intensity=staged==='printing'?3.9:staged==='error'?2.2:3.5;
   schedule();}
  function stage(state){if(state===staged)return;staged=state;applyStage();}
  function theme(mode){const light=mode==='light';renderer.setClearColor(light?0xedf1f0:0x101314,nextStage?0:1);
   if(grid){grid.material.vertexColors=false;grid.material.color.set(light?0x8ba39a:nextStage?0x2c6f6a:0x35534b);grid.material.transparent=true;grid.material.opacity=light?.28:nextStage?.22:.5;grid.material.needsUpdate=true;}
   world.children.filter(c=>c.isHemisphereLight).forEach(l=>{l.intensity=light?3:nextStage?2.1:2.8;l.color.set(light?0xffffff:0xe1f2ee);l.groundColor.set(light?0xcbd7d1:0x333b37);});
   world.children.filter(c=>c.isDirectionalLight&&!c.userData.stageLight).forEach((l,i)=>{l.color.set((light?[0xffffff,0xe0eee6,0xfff4e7]:[0xf4f8f6,0x04d9c4,0xff2d92])[i]);l.intensity=(light?[3.2,1.5,1.4]:[3.5,1.2,.55])[i];});
   renderer.toneMappingExposure=light?1.15:1.35;mini?.theme(light);
   applyStage();}
 const api={hidden,live,theme,stage,get world(){return world;},highlightObject,get stageLight(){return world.children.find(c=>c.isDirectionalLight&&!c.userData.stageLight)?.color.getHexString();},get bedCenter(){return sourceData?.printer_frame?.bed_center||null;},loadToolpath,toolpathProgress,clearToolpath,meshOverlay,probeMarks,probeLed,get toolpath(){return toolpath?{name:toolpath.name,segments:toolpath.segments,layers:toolpath.parsed.layers.length}:null;},look(position,target){camera.position.set(...position);orbit.target.set(...target);orbit.update();updateCut();schedule();},geometryBounds(id){const b=entries.get(id)?.geometry.boundingBox;return b?{min:b.min.toArray(),max:b.max.toArray(),size:b.getSize(new THREE.Vector3()).toArray()}:null;},setColor,resetColors,color(id){const m=entries.get(id);return m?'#'+m.material.color.getHexString():null;},get colorGroups(){return Object.entries(colorNames).map(([id,name])=>({id,name,count:meshes.filter(m=>m.userData.colorGroup===id&&!m.userData.replacedByRapid).length,color:colors.groups[id]||colors.all||meshes.find(m=>m.userData.colorGroup===id)?.userData.sourceColor||'#697985'}));},zoom,orbitBy,orientation,reset(){clearFocus();hiddenParts.clear();hidden.clear();hidden.add('covers');explode=0;for(const m of meshes){m.visible=visible(m);m.position.copy(m.userData.motionOffset);}fit(meshes,'iso');},previewZoom(scale){mini?.zoom(scale);},previewReset(){mini?.reset();},get previewDistance(){return mini?.distance;},get visibleParts(){return meshes.filter(m=>visible(m)).map(m=>m.userData.id);},get cameraState(){return {position:camera.position.toArray(),target:orbit.target.toArray()};},inspectSource,get sourceAssets(){return sourceAssets;},get variants(){return [...variants].map(([group,selected])=>({group,selected,options:[...new Set(meshes.filter(m=>m.userData.variantGroup===group).map(m=>m.userData.variantName))]}));},variant(group,name){if(meshes.some(m=>m.userData.variantGroup===group&&m.userData.variantName===name)){variants.set(group,name);schedule();}},hideSelected(){if(selected)hiddenParts.add(selected.userData.id);schedule();},isolate(){if(selected)spotlightMesh(selected,{flash:false});},showAll(){api.reset();},spotlight(id,opts){const m=entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id);if(!m)return false;spotlightMesh(m,opts);return true;},project(id){const m=entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id);if(!m||!m.geometry.boundingBox)return null;const b=m.geometry.boundingBox,c=b.getCenter(new THREE.Vector3()).add(m.position),r=Math.max(8,b.getSize(new THREE.Vector3()).length()/2),p=c.clone().project(camera),right=new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld,0).multiplyScalar(r),q=c.clone().add(right).project(camera),rect=canvas.getBoundingClientRect();return {x:(p.x*.5+.5)*rect.width,y:(-p.y*.5+.5)*rect.height,r:Math.hypot((q.x-p.x)*.5*rect.width,(q.y-p.y)*.5*rect.height),on:p.z<1&&Math.abs(p.x)<1.2&&Math.abs(p.y)<1.2};},panels(value){for(const m of meshes.filter(m=>m.userData.layer==='covers')){m.userData.panelTransparent=value;m.material.transparent=value;m.material.opacity=value?.15:1;m.material.depthWrite=!value;}hidden.delete('covers');schedule();},metrics,renderer,load,jobUpdate,loadJob,preview,closePreview(){mini?.dispose();mini=null;},associateJob(name){jobName=name;return jobUpdate(jobState);},set,schedule,view(v){const distance=camera.position.distanceTo(orbit.target);camera.position.copy(orbit.target).addScaledVector(new THREE.Vector3(...CAMERA_VIEWS[v]).normalize(),distance);orbit.update();schedule();},focus(id){const m=entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id);if(m)focusPart(m);},select,get selected(){return selected?.userData;},get entries(){return [...entries.values()].map(m=>m.userData);},get explode(){return explode;},set explode(v){explode=v;},get jobPreview(){return jobModel?{position:jobModel.position.toArray(),progress:jobDisplay(jobState,jobName),clip:jobModel.children[1].material.clippingPlanes[0].constant}:null;},get source(){return asset;}};
 // Tile mode (farm wall, WO-14) disposes the GL context when the tile goes away; the full view lives
 // for the page's life and stays on window for the console, as before.
 api.dispose=()=>{resizer.disconnect();clearTimeout(tileTimer);tileTimer=-1;if(frame){cancelAnimationFrame(frame);frame=0;}orbit.dispose();pmrem.dispose();renderer.dispose();};
 if(!tileMode)window.__stellaScene=api;load();return api;
}
