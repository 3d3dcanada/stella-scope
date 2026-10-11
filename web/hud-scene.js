import * as THREE from './vendor/three.module.js';
import {OrbitControls} from './vendor/OrbitControls.js';
import {STLLoader} from './vendor/STLLoader.js';
import {jobDisplay,instanceRecordId,insideTravel} from './hud-core.js';

export const CAMERA_VIEWS={iso:[1,-1,.65],front:[0,-1,0],rear:[0,1,0],left:[-1,0,0],right:[1,0,0],top:[0,0,1],bottom:[0,0,-1]};
const empty=new THREE.Box3();
export function createScene(canvas,onSelect,opts={}){
 // NEXT: opts.stage==='next' is opt-in staging for the rebuilt HUD.
 // Without it every line below behaves exactly as before, so the old HUD at / is unchanged.
 const nextStage=opts?.stage==='next';
 const renderer=new THREE.WebGLRenderer({canvas,antialias:true,alpha:nextStage});renderer.setPixelRatio(Math.min(devicePixelRatio,1.5));renderer.setClearColor(0x101314,nextStage?0:1);renderer.outputColorSpace=THREE.SRGBColorSpace;renderer.toneMapping=THREE.ACESFilmicToneMapping;renderer.toneMappingExposure=1.35;renderer.localClippingEnabled=true;
 const world=new THREE.Scene(),camera=new THREE.PerspectiveCamera(32,1,.1,20000);camera.up.set(0,0,1);camera.position.set(1000,-1400,1000);
 world.add(new THREE.HemisphereLight(0xc3e5ff,0x182028,2.4));
 // A warm key from the front left and a cool rim from behind, so the machine separates from the
 // dark room instead of being lit evenly from everywhere.
 if(nextStage){const key=new THREE.DirectionalLight(0xffe9d2,1.6);key.userData.stageLight=true;key.position.set(-900,-1100,1300);world.add(key);
  const rim=new THREE.DirectionalLight(0x6fd6ff,2.6);rim.userData.stageLight=true;rim.position.set(-400,1600,900);world.add(rim);
  const rim2=new THREE.DirectionalLight(0xb48cff,1.2);rim2.userData.stageLight=true;rim2.position.set(1500,900,500);world.add(rim2);}
 for(const [color,power,pos]of [[0xf4f8ff,3.5,[600,-900,1200]],[0x04d9c4,1.8,[-800,200,650]],[0x808eff,2,[400,800,800]]]){const light=new THREE.DirectionalLight(color,power);light.position.set(...pos);world.add(light);}
 // A studio environment, generated once. MeshStandardMaterial metals reflect nothing without an
 // environment map, which is the whole reason the machine read as flat grey plastic (measured
 // 2026-09-22 on the build: machine region avg luminance 25/255, color spread 7.9/255). Softbox
 // panels in a dark room; no brand neons in the light itself — the neons stay in the UI chrome
 // where the brand puts them.
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
 const root=new THREE.Group();world.add(root);const orbit=new OrbitControls(camera,canvas);orbit.enableDamping=false;orbit.enableZoom=true;orbit.zoomToCursor=true;orbit.minDistance=2;orbit.maxDistance=8000;orbit.addEventListener('change',()=>{updateCut();schedule();});
 const meshes=[],entries=new Map(),hidden=new Set(['covers']),hiddenParts=new Set(),variants=new Map();let frame=0,asset=null,sourceAssets=[],bounds=null,selected=null,outline=null,grid=null,explode=0,jobModel=null,jobName='',jobState=null,sourceData=null,lastPose=null,lastMappedPose=null,lastMappedTs=null,lastSet=null,homedSeen='',poseSkips=0,lastJobDisplay=null,lastLiveVisual=null,mini=null;
 const tip=document.getElementById('scene-tip'),caption=document.querySelector('.scene-key');
 const metrics={loads:[],frames:[]};let colors={all:null,groups:{},parts:{}},colorKey='';
 // The build plate starts as dark textured PEI; heat lerps it toward HOT_PLATE and lights it, so a
 // warm glow still reads as heat. Users can pick their own plate colour in the colour panel.
 const PLATE_COLOR='#2b2a28';
 const DEFAULT_GROUP_COLORS={bed:PLATE_COLOR,printed:'#8b9096'};
 const colorNames={printed:'Printed accents',frame:'Frame & rails',toolhead:'Toolhead',bed:'Build plate',motors:'Motors',electronics:'Electronics',covers:'Panels',fasteners:'Fasteners',belts:'Belts'};
 function paintGroup(m){const d=m.userData,n=d.name.toLowerCase(),c=m.material.color,hsl={};c.getHSL(hsl);if(/^bed\.(sheet|plate)$/.test(d.slot_id||''))return 'bed';return /cowl|afterburner|stealthburner/.test(n)?'toolhead':/nema|stepper/.test(n)?'motors':/belt(?!.*drive)|gt2/.test(n)?'belts':/shcs|bhcs|fhcs|washer|nut|screw|bolt/.test(n)?'fasteners':['bed','covers','electronics'].includes(d.layer)?d.layer:hsl.s>.35?'printed':d.layer==='toolhead'?'toolhead':'frame';}
 // The colour a part sits at when it is cold and its fan is off. live() lerps away from it for heat
// And for fan speed, so it has to be recorded here, where the palette is decided, and the live visual
// has to be re-applied afterwards or a repaint would leave a hot plate looking cold. The hotend's own
// materials : brass nozzle and core top, white
 // Silicone sock. They sit ahead of the group defaults so the toolhead paint does not whiten the brass;
 // a colour the user picks for the part or its group still wins.
 const SLOT_FINISH={'head.nozzle':['#c9a24b',.9,.32],'head.core':['#b8913e',.85,.38],'head.heatbreak':['#b8913e',.85,.38],'head.sock':['#e6e0d3',0,.88]};
 function applyColors(){for(const m of meshes){const d=m.userData,fin=SLOT_FINISH[d.slot_id];if(fin&&!colors.parts[d.id]&&!colors.groups[d.colorGroup]){m.material.metalness=fin[1];m.material.roughness=fin[2];}
  m.material.color.set(colors.parts[d.id]||colors.groups[d.colorGroup]||(fin&&fin[0])||DEFAULT_GROUP_COLORS[d.colorGroup]||colors.all||d.sourceColor);d.baseColor=m.material.color.getHex();}lastLiveVisual=null;mini?.refreshColor();schedule();}
 function initColors(){colorKey='stella-colors:v1:'+asset.source_sha256;for(const m of meshes){m.userData.colorGroup=paintGroup(m);m.userData.sourceColor='#'+m.material.color.getHexString();}try{const saved=JSON.parse(localStorage.getItem(colorKey));const valid=v=>typeof v==='string'&&/^#[0-9a-f]{6}$/i.test(v);if(saved&&typeof saved==='object'){colors.all=valid(saved.all)?saved.all:null;for(const [k,v]of Object.entries(saved.groups||{}))if(Object.hasOwn(colorNames,k)&&valid(v))colors.groups[k]=v;for(const [k,v]of Object.entries(saved.parts||{}))if(entries.has(k)&&valid(v))colors.parts[k]=v;}}catch{}applyColors();}
 function setColor(scope,color){if(!/^#[0-9a-f]{6}$/i.test(color))throw Error('Choose a valid color.');const next=structuredClone(colors);if(scope==='all'){colors={all:color,groups:{},parts:{}};}else if(scope.startsWith('group:')){const group=scope.slice(6);if(!Object.hasOwn(colorNames,group))throw Error('Unknown color group.');colors.groups[group]=color;for(const m of meshes)if(m.userData.colorGroup===group)delete colors.parts[m.userData.id];}else if(entries.has(scope))colors.parts[scope]=color;else throw Error('Select a component first.');try{localStorage.setItem(colorKey,JSON.stringify(colors));}catch{colors=next;throw Error('Browser storage unavailable; color was not saved.');}applyColors();}
 function resetColors(){localStorage.removeItem(colorKey);colors={all:null,groups:{},parts:{}};applyColors();}
 function note(s){if(caption)caption.textContent=s;}
 function resize(){const w=canvas.clientWidth,h=canvas.clientHeight;if(!w||!h)return;renderer.setSize(w,h,false);const aspect=w/h;if(bounds)camera.position.sub(orbit.target).multiplyScalar(Math.min(1,camera.aspect)/Math.min(1,aspect)).add(orbit.target);camera.aspect=aspect;camera.updateProjectionMatrix();schedule();}
 new ResizeObserver(resize).observe(canvas);resize();
 function visible(m){const d=m.userData;return !d.notFitted&&!d.focusHidden&&!hidden.has(d.layer)&&!hiddenParts.has(d.id)&&(!d.variantGroup||variants.get(d.variantGroup)===d.variantName);}
 function schedule(){if(frame)return;frame=requestAnimationFrame(()=>{frame=0;if(!canvas.clientWidth||!canvas.clientHeight)return;for(const m of meshes){m.visible=visible(m);const spread=m.userData.center.clone().sub(bounds?.getCenter(new THREE.Vector3())||new THREE.Vector3()).multiplyScalar(explode/500);m.position.copy(m.userData.motionOffset||new THREE.Vector3()).add(spread);}if(outline&&selected)outline.box.setFromObject(selected);const start=performance.now();renderer.render(world,camera);metrics.frames.push(performance.now()-start);if(metrics.frames.length>300)metrics.frames.shift();document.dispatchEvent(new CustomEvent('stella-camera-change'));
  const anchor=heatBlock.find(m=>m.userData.slot_id==='head.nozzle')||heatBlock[0];
  if(anchor){const p=anchor.geometry.boundingBox.getCenter(new THREE.Vector3()).add(anchor.position).project(camera);
   const r=canvas.getBoundingClientRect();
   document.dispatchEvent(new CustomEvent('stella-track',{detail:{x:(p.x*.5+.5)*r.width,y:(-p.y*.5+.5)*r.height,on:p.z<1&&Math.abs(p.x)<1.1&&Math.abs(p.y)<1.1}}));}
  });}
 function fit(objects=meshes,view=null){const box=empty.clone();objects.filter(m=>m.visible).forEach(m=>box.expandByObject(m));if(box.isEmpty())return;const center=box.getCenter(new THREE.Vector3()),size=box.getSize(new THREE.Vector3());const dir=view?new THREE.Vector3(...(nextStage&&view==='iso'?[.85,-1,.42]:CAMERA_VIEWS[view])):camera.position.clone().sub(orbit.target);dir.normalize();const distance=size.length()*(nextStage?.5*Math.max(1,Math.sqrt(1000/Math.max(canvas.clientHeight,1))):.44)/Math.tan(camera.fov*Math.PI/360)/Math.min(1,camera.aspect);// NEXT: aim a little below the middle so the machine sits above the dock instead of behind it.
 if(nextStage&&objects===meshes)center.z-=size.z*(camera.aspect>1?.16:.04);orbit.target.copy(center);camera.position.copy(center).addScaledVector(dir,distance);camera.far=distance*20;camera.updateProjectionMatrix();orbit.update();schedule();}
 function pick(e){const r=canvas.getBoundingClientRect(),ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2((e.clientX-r.left)/r.width*2-1,1-(e.clientY-r.top)/r.height*2),camera);return ray.intersectObjects(meshes.filter(m=>m.visible&&m.material.opacity>.1&&!(nextStage&&INERT.has(m.userData.colorGroup))),false)[0]?.object;}
 // Only parts worth opening answer a click or hover; extrusions, screws, belts and panels are
 // scenery.
 const INERT=new Set(['fasteners','frame','belts','covers']);
 const cutPlane=new THREE.Plane(new THREE.Vector3(0,0,-1),1e6);let cutTarget=null;
 function clearFocus(){for(const m of meshes){m.userData.focusHidden=false;m.material.transparent=Boolean(m.userData.panelTransparent);m.material.opacity=m.userData.panelTransparent?.15:1;m.material.depthWrite=!m.userData.panelTransparent;}
  cutTarget=null;renderer.clippingPlanes=[];canvas.dataset.inspection='whole';document.dispatchEvent(new CustomEvent('stella-inspection',{detail:{mode:'whole'}}));}
 // While focused, everything between the camera and the part is cut away, so zooming in opens the
 // machine up instead of putting the camera inside it.
 function updateCut(){if(!cutTarget)return;const n=camera.position.clone().sub(cutTarget.centre).normalize();
  cutPlane.normal.copy(n.clone().negate());cutPlane.constant=-cutPlane.normal.dot(cutTarget.centre)+cutTarget.radius;renderer.clippingPlanes=[cutPlane];}
 function focusPart(m){clearFocus();hidden.delete(m.userData.layer);hiddenParts.delete(m.userData.id);if(m.userData.variantGroup)variants.set(m.userData.variantGroup,m.userData.variantName);m.visible=visible(m);const center=m.geometry.boundingBox.getCenter(new THREE.Vector3()),radius=Math.max(100,m.geometry.boundingBox.getSize(new THREE.Vector3()).length()*2);for(const other of meshes)if(other!==m){const near=other.userData.center.distanceTo(center)<=radius;other.userData.focusHidden=!near;other.material.transparent=true;other.material.opacity=near?.72:.035;other.material.depthWrite=near;}canvas.dataset.inspection='focus';cutTarget={centre:center.clone(),radius:Math.max(12,m.geometry.boundingBox.getSize(new THREE.Vector3()).length()*.62)};updateCut();
  document.dispatchEvent(new CustomEvent('stella-inspection',{detail:{mode:'focus',name:m.userData.name}}));fit([m],'iso');}
 let down=null,hoverTime=0;
 canvas.addEventListener('pointerdown',e=>{down=e.button===0?[e.clientX,e.clientY]:null;if(tip)tip.hidden=true;});
 canvas.addEventListener('pointerup',e=>{if(e.button===0&&down&&Math.hypot(e.clientX-down[0],e.clientY-down[1])<4){const m=pick(e);if(m){select(m.userData.id);onSelect(m.userData.id);}}down=null;});
 canvas.addEventListener('pointermove',e=>{if(e.buttons||performance.now()-hoverTime<90)return;hoverTime=performance.now();const m=pick(e);canvas.style.cursor=m?'pointer':'grab';if(tip){tip.hidden=!m;if(m){const r=canvas.getBoundingClientRect();tip.textContent=m.userData.name;tip.style.left=Math.min(r.width-220,Math.max(10,e.clientX-r.left+14))+'px';tip.style.top=Math.max(10,e.clientY-r.top-36)+'px';}}});
 canvas.addEventListener('pointerleave',()=>{if(tip)tip.hidden=true;});
 canvas.addEventListener('keydown',e=>{if(e.key==='0'||e.key==='Escape')api.reset();else if(['+','=','-'].includes(e.key)){camera.position.sub(orbit.target).multiplyScalar(e.key==='-'?1.15:1/1.15).add(orbit.target);orbit.update();schedule();}else return;e.preventDefault();});
 canvas.addEventListener('dblclick',()=>fit(selected?[selected]:meshes));
 function zoom(scale){const delta=camera.position.clone().sub(orbit.target),distance=THREE.MathUtils.clamp(delta.length()*scale,orbit.minDistance,orbit.maxDistance);camera.position.copy(orbit.target).add(delta.setLength(distance));orbit.update();schedule();}
 function orbitBy(dx,dy){const delta=camera.position.clone().sub(orbit.target);delta.applyAxisAngle(new THREE.Vector3(0,0,1),-dx*.008);const right=new THREE.Vector3().crossVectors(delta,camera.up).normalize(),next=delta.clone().applyAxisAngle(right,-dy*.008);if(Math.abs(next.clone().normalize().dot(camera.up))<.995)delta.copy(next);camera.position.copy(orbit.target).add(delta);orbit.update();schedule();}
 // The word FRONT showed on the left and right of the view cube. Labels were upright screen text
 // at their face's centre, so a face turned toward edge-on had a label wider than itself spilling
 // onto its neighbours (up to 10.6 px outside its own face, measured). Each label now lies in its
 // face's plane: `label` is the SVG matrix taking the text's right and down onto that face (the
 // directions someone looking square at it reads), so it foreshortens with the face and stays on
 // it, and `facing` lets the HUD fade it as the face turns away.
 const CUBE_TEXT={right:[[0,1,0],[0,0,-1]],left:[[0,-1,0],[0,0,-1]],rear:[[-1,0,0],[0,0,-1]],front:[[1,0,0],[0,0,-1]],top:[[1,0,0],[0,-1,0]],bottom:[[-1,0,0],[0,-1,0]]};
 function orientation(){const inverse=camera.quaternion.clone().invert(),toward=camera.position.clone().sub(orbit.target).normalize();return Object.entries({right:[1,0,0],left:[-1,0,0],rear:[0,1,0],front:[0,-1,0],top:[0,0,1],bottom:[0,0,-1]}).map(([name,normal])=>{const n=new THREE.Vector3(...normal),u=new THREE.Vector3(normal[2]?1:0,normal[0]?1:0,normal[1]?1:0),v=new THREE.Vector3().crossVectors(n,u);const points=[[-1,-1],[1,-1],[1,1],[-1,1]].map(([a,b])=>n.clone().addScaledVector(u,a).addScaledVector(v,b).multiplyScalar(27).applyQuaternion(inverse)).map(p=>[64+p.x,60-p.y]);const center=n.clone().multiplyScalar(27).applyQuaternion(inverse);const [r,d]=CUBE_TEXT[name].map(a=>new THREE.Vector3(...a).applyQuaternion(inverse));return {name,visible:n.dot(toward)>.005,facing:n.dot(toward),points,center:[64+center.x,60-center.y],label:[r.x,-r.y,d.x,-d.y,64+center.x,60-center.y]};});}
 function select(id){const next=(id==='head'?meshes.find(m=>m.userData.slot_id==='head.nozzle'):null)||entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id)||null;if(selected===next)return;clearFocus();selected=next;for(const m of meshes)m.material.emissive.setHex(0);lastLiveVisual=null;if(outline){world.remove(outline);outline.dispose();outline=null;}if(selected){outline=new THREE.Box3Helper(new THREE.Box3().setFromObject(selected),0x04d9c4);world.add(outline);}document.dispatchEvent(new CustomEvent('stella-selection-change'));schedule();}
 function classification(name){const s=name.toLowerCase();return /deck panel/.test(s)?'deck':/panel|door|acrylic/.test(s)?'covers':/z motor|z.drive|z_drive/.test(s)?'z-drives':/bed|build.plate|heater/.test(s)?'bed':/extruder|toolhead|afterburner|hotend/.test(s)?'toolhead':/gantry|x.axis|y.axis/.test(s)?'gantry':/electronics|controller|power.supply|raspberry/.test(s)?'electronics':/deck/.test(s)?'deck':'frame';}
 async function load(){
  note('Loading genuine printer assembly…');const start=performance.now();
  try{const r=await fetch('/api/assets');if(!r.ok)throw Error('Source library unavailable');const library=await r.json();sourceAssets=library.assets;asset=library.assets.find(a=>a.id==='stella-printer')||library.assets.find(a=>/^hartk-voron-24r2-\d{3}-reference$/.test(a.id))||library.assets.find(a=>/^voron-24r2-\d{3}-basic$/.test(a.id))||library.assets.find(a=>a.id==='voron-r2-master-reference');if(!asset)throw Error('Full printer CAD is not imported yet. No substitute geometry is shown.');const response=await fetch(asset.geometry_url);if(!response.ok)throw Error('Printer geometry unavailable');const data=await response.json();sourceData=data;
   const geometry=data.meshes.map(raw=>{const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(raw.attributes.position.array,3));g.setIndex(raw.index.array);if(raw.attributes.normal?.array?.length===raw.attributes.position.array.length)g.setAttribute('normal',new THREE.Float32BufferAttribute(raw.attributes.normal.array,3));else g.computeVertexNormals();g.computeBoundingBox();return g;});
   function walk(node,path,names){for(const [i,index]of (node.meshes||[]).entries()){const raw=data.meshes[index],g=geometry[index];if(!g)throw Error('Missing source mesh');const rawName=raw.name||node.name||'Component',name=/^=>|^\[/.test(rawName)?[...names,node.name].filter(n=>n&&!/^=>|^\[/.test(n)).at(-1)||'Source component':rawName,full=[...names,node.name,name].filter(Boolean).join(' / '),map=raw.mapping||{},id=node.instance_ids?.[i]||`${asset.id}:${path}:${index}`;const color=raw.color?new THREE.Color().setRGB(...raw.color,THREE.SRGBColorSpace):new THREE.Color(0x667380);const material=new THREE.MeshStandardMaterial({color,metalness:.35,roughness:.5,side:THREE.DoubleSide});const m=new THREE.Mesh(g,material);const ancestry=[...names,node.name];let variantGroup=null,variantName=null;for(const group of ['Toolheads','Controllers']){const at=ancestry.findIndex(n=>n?.startsWith(group));if(at>=0&&ancestry[at+1]){variantGroup=group;variantName=ancestry[at+1];if(!variants.has(group))variants.set(group,variantName);}}m.userData={id,record_id:instanceRecordId(id),name,full,asset,variantGroup,variantName,slot_id:map.slot_id||null,layer:['covers','z-drives','bed','toolhead','gantry','electronics','deck','frame'].includes(map.layer)?map.layer:classification(full),motion:map.motion||'fixed',center:g.boundingBox.getCenter(new THREE.Vector3()),motionOffset:new THREE.Vector3()};root.add(m);meshes.push(m);entries.set(id,m);}for(const [i,c]of (node.children||[]).entries())walk(c,path+'/'+i,[...names,node.name]);}
       walk(data.root,'root',[]);
    // Material pass: same colours, honest surfaces. Without this every part shares one mid-grey
    // metallic-plastic finish (.35/.5); with it, aluminium extrusions get their sheen, rails
    // read as chrome, belts as rubber, panels stay matte. It touches only metalness/roughness;
    // colour stays exactly where the palette puts it.
    const familyFinish=d=>{const n=(d.name||'').toLowerCase(),full=(d.full||'').toLowerCase();
     if(/hfsb|extrusion/.test(n))return [.85,.38];
     if(/mgn\d|linear rail|rod|shaft/.test(n))return [1,.18];
     if(/belt(?!.*drive)/.test(n))return [0,.95];
     if(/shcs|bhcs|fhcs|screw|bolt|washer|nut/.test(n))return [.9,.25];
     if(/panel|door|acrylic|cover/.test(n)||d.layer==='covers')return [.05,.8];
     if(/^bed\.(sheet|plate)$/.test(d.slot_id||''))return [.55,.35];
     return null;};
    for(const m of meshes){const f=familyFinish(m.userData);if(f){m.material.metalness=f[0];m.material.roughness=f[1];}}
    stockToolhead();establishMotion();initColors();const sheet=meshes.find(m=>m.userData.name==='Flex Sheet:1');if(sheet&&sourceData.printer_frame?.bed_center){sourceData.printer_frame.bed_center[2]=sheet.geometry.boundingBox.max.z;sourceData.printer_frame.build_surface_basis='Top bound of named source Flex Sheet:1; plate XY center retained, front sheet tab excluded';}bounds=new THREE.Box3().setFromObject(root);const size=bounds.getSize(new THREE.Vector3()),center=bounds.getCenter(new THREE.Vector3());   grid=new THREE.GridHelper(Math.max(size.x,size.y)*1.15,Math.max(20,Math.round(Math.max(size.x,size.y)*1.15/10)),0x174448,0x152028);grid.rotation.x=Math.PI/2;grid.position.set(center.x,center.y,bounds.min.z-4);world.add(grid);
    if(nextStage){// a soft contact shadow under the frame and an accent pool of light around it
     const disc=(stops,r,z)=>{const c=document.createElement('canvas');c.width=c.height=256;const g=c.getContext('2d'),gr=g.createRadialGradient(128,128,0,128,128,128);for(const [o,col]of stops)gr.addColorStop(o,col);g.fillStyle=gr;g.fillRect(0,0,256,256);
      const t=new THREE.CanvasTexture(c);t.colorSpace=THREE.SRGBColorSpace;const m=new THREE.Mesh(new THREE.PlaneGeometry(r*2,r*2),new THREE.MeshBasicMaterial({map:t,transparent:true,depthWrite:false,toneMapped:false}));m.position.set(center.x,center.y,z);m.renderOrder=-1;m.raycast=()=>{};world.add(m);return m;};
     const R=Math.max(size.x,size.y);disc([[0,'rgba(4,217,196,.42)'],[.35,'rgba(4,217,196,.16)'],[.7,'rgba(90,120,255,.05)'],[1,'rgba(4,217,196,0)']],R*1.35,bounds.min.z-5);
     disc([[0,'rgba(0,0,0,.72)'],[.55,'rgba(0,0,0,.45)'],[1,'rgba(0,0,0,0)']],R*.62,bounds.min.z-3);
     }theme(document.body.dataset.theme||'dark');for(const m of meshes)m.visible=visible(m);fit(meshes,'iso');metrics.loads.push({id:asset.id,ms:performance.now()-start,meshes:meshes.length,triangles:meshes.reduce((total,m)=>total+m.geometry.index.count/3,0),bounds:size.toArray()});canvas.dataset.renderer='source-cad';canvas.dataset.source=asset.source_sha256;note(`${meshes.length.toLocaleString()} CAD components · stock toolhead`);document.dispatchEvent(new CustomEvent('stella-geometry-ready'));
    // Finding 2 of a replay or rewind link opened cold set the pose before the model and its motion
    // datum existed, so the head sat at its rest pose until the cursor moved. The last pose asked for
    // is applied again now that the model can take it.
    if(lastSet)set(...lastSet);
  }catch(e){note(e.message);canvas.dataset.renderer='source-unavailable';canvas.dataset.error=e.message;}
 }
 // An axis that is not in homed_axes carries no information, so it holds its last drawn value
 // rather than taking the reported one; before anything has been drawn it holds the CAD rest pose.
 function set(snapshot,id){lastSet=[snapshot,id];select(id);const raw=snapshot?.point?.position,datum=sourceData?.printer_frame?.source_pose,homedAxes=snapshot?.point?.homed||'';const pos=raw&&datum?raw.map((v,i)=>homedAxes.includes('xyz'[i])?v:(lastMappedPose?lastMappedPose[i]:datum[i])):raw;lastPose=pos;canvas.dataset.sampleTs=String(snapshot?.point?.ts??'');
  // The spike is in the data, not the render. The jog is audited at 1789999549.76 intent /
  // 1789999550.00 result as G91 / G1 X-10.00 F6000 / G90, and the recorded sample at 1789999550.244
  // carries motion_report.live_position [0,0,0] for exactly one 253 ms tick, between 330.000 and
  // 320.000, with nozzle and bed continuous either side — so it is Klipper's live position and not
  // an empty frame. Drawn verbatim it threw the head 330 mm to the origin and back. A pose needing
  // more travel than max_velocity allows in the elapsed sample time did not happen on the machine,
  // so it is held rather than drawn. Scrubbing the recording has a large dt, so a deliberate seek
  // still lands.
  const ts=snapshot?.point?.ts,vmax=snapshot?.point?.max_velocity,vzmax=snapshot?.point?.max_z_velocity,limits=snapshot?.point?.limits;
  // An axis that has just become homed is reported in a coordinate frame the drawn pose knows
  // nothing about (G28 sets Z to 0 and hops, then force-sets 464), so the reach test below cannot
  // apply to it. The first pose that is inside the machine after the axis is gained re-anchors that
  // axis; the flag survives every refusal, so it is the first *drawable* sample that lands, not the
  // force position. The recorded sample at 1790021428.895 reports homed_axes "xyz" and Z 464.0195 on
  // BOTH motion_report.live_position and toolhead.position, against axis_maximum 310 — Klipper
  // force-sets the kinematic position to position_endstop + 1.5 * (position_max - position_endstop)
  // for a homing move and then drives down until the probe triggers. So this is not an unhomed axis;
  // it is a homed axis reported outside the machine. The head never went 464 mm up, so the sample is
  // held exactly like the [0,0,0] spike below. 1 mm of slack keeps a legitimate move to the very end
  // of travel drawable.
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
   // A refused pose must not leave the head frozen. Drawn every animation frame, the reach is a
   // few mm, so once one frame was refused every later frame was too far from the stuck pose and
   // the head stayed put until it jumped. A single spike lasts one tick (253 ms); a position
   // that stays refused for 0.75 s of sample time is where the machine really is.
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
 // The motion is "not far behind, but a little janky". The recorder ticks every 250 ms and the model
 // used to jump a whole tick at a time. The accepted pose is now a target and every animation frame
 // walks the drawn pose toward it over the gap between the two samples, so the head glides the way
 // the machine does. Linear, because between two samples the machine is holding a velocity — easing
 // would put a stop in the middle of a straight line. Both guards above run first, so a pose that
 // never happened is never a tween target, and a scrub (huge gap) lands at once rather than crawling
 // there.
 let chainPath=null,refusedSince=null;const X_AXIS=new THREE.Vector3(1,0,0),Y_AXIS=new THREE.Vector3(0,1,0);
 // The rear IGUS Z chain: it stands in the X-Z plane, its left run bolted to the frame at the
 // bottom, its right run's end riding on the gantry. Arc position s runs up the fixed run, over the
 // bend (centre x wc, height uc, radius R) and down the moving run; the bend rises half as far as
 // the gantry. Returns [x, z, angle].
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
  canvas.dataset.drawnPose=JSON.stringify(pos.map(v=>Number(v.toFixed(3))));schedule();}
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
 // printed colour, `todo` the rest, and progress only moves the draw ranges. Travel moves are not
 // drawn at all now — only extruding segments reach the buffer — and "I'd like for the color to
 // change progressively so that we know what's beginning and what's complete", plus, on the same
 // look, he could not tell the old done-teal from the ghost while printing. So the printed portion
 // is a hot ramp: the last HOT_RUN segments run bright orange back to ember, everything printed
 // before that stays ember, and what is still to come is a dim ghost.
 let toolpath=null;
 // The ramp is a fixed share of the file rather than a fixed segment count, so it reads the same on
 // a 40,000-segment flow plate as on a 90,000-segment cube; the floor and cap keep the per-tick
 // repaint bounded either way. the ghost at 0x3d4d55 /.34 was "way too dark now, it kind of looks
 // like shit" — the part still to come has to read as the whole part at a glance. Mid grey-blue
 // at.6, which is still nothing like the hot ramp beside it.
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
   // EXCLUDE_OBJECT ranges are global indices over the parser's own segment numbering (extrudes
   // and travels both counted, spanning layers), while the drawn array packs extrudes only. One
   // pass builds the map from a global index to its packed index (-1 for a travel), so
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
 // The done boundary is now the byte the printer is reading (virtual_sdcard.file_position),
 // binary-searched against each drawn segment's own byte offset, instead of a layer plus an
 // evenly-split fraction of that layer's bytes. Layer and fraction still work for a scrub with no
 // file position.
 function toolpathProgress({layer=0,fraction=0,filePosition=null,live=null}={}){if(!toolpath)return;const starts=toolpath.layerStarts;
  let at;
  if(Number.isFinite(filePosition)&&toolpath.byteAt?.length){const b=toolpath.byteAt;let lo=0,hi=b.length;
   while(lo<hi){const mid=(lo+hi)>>1;if(b[mid]<=filePosition)lo=mid+1;else hi=mid;}at=lo;}
  else at=Math.min(toolpath.segments,Math.round((starts[layer]??0)+((starts[layer+1]??toolpath.segments)-(starts[layer]??0))*Math.max(0,Math.min(1,fraction))));
  // File_position is the byte Klipper has READ, and it runs ahead of the nozzle by whatever is in
  // the lookahead queue — measured against the cube, 7 to 27 mm of path, which on a 30 mm cube is
  // most of a wall drawn as done before the head gets there. The queue is bounded, so walking back
  // from the read pointer for the drawn segment that actually ends under the reported position puts
  // the boundary on the move the machine is making. Only taken when something that close exists;
  // otherwise the byte position stands.
  if(Array.isArray(live)&&live.every(Number.isFinite)&&at>0){const p=toolpath.geometry.attributes.position.array;
   let best=-1,bd=Infinity;
   for(let i=Math.max(0,at-QUEUE_SEGMENTS);i<at;i++){const o=i*6+3;
    const d=(p[o]-live[0])**2+(p[o+1]-live[1])**2+(p[o+2]-live[2])**2;if(d<bd){bd=d;best=i;}}
   if(best>=0&&bd<=9)at=best+1;}
  if(at!==toolpath.painted)paintToolpath(at);
  toolpath.done.geometry.setDrawRange(0,at*2);toolpath.todo.geometry.setDrawRange(at*2,Infinity);schedule();}
  // The bed mesh draped over the plate, and each probe landing as it happens. Printer X/Y map to the
  // scene like the toolpath (bed centre at 175, 175); heights are exaggerated so a tenth of a
  // millimetre can be seen, and the colour comes from the caller.
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
  // Probe landings on the plate: each point is a square tile of the mesh on the flat bed, coloured by
  // how far it sits from level, with a bull's-eye over it that pulses outer ring, middle ring,
  // centre, and loops, in the same colour. The newest landing is drawn larger.
  let probeRaf=0;
  function probeMarks(points,{rgb=null,fresh=10}={}){dropLayer(probeLayer);probeLayer=null;cancelAnimationFrame(probeRaf);probeRaf=0;const bed=sourceData?.printer_frame?.bed_center;
   if(!Array.isArray(points)||!points.length||!bed){schedule();return;}
   // one mark per spot (the probe samples each point twice): the newest reading wins
   const byXY=new Map();for(const p of points)if(Number.isFinite(p.x)&&Number.isFinite(p.y))byXY.set(Math.round(p.x)+':'+Math.round(p.y),p);const pts=[...byXY.values()];
   const zs=pts.map(p=>p.z).filter(Number.isFinite),mean=zs.reduce((a,b)=>a+b,0)/Math.max(1,zs.length),span=Math.max(.02,...zs.map(z=>Math.abs(z-mean)));
   const xs=[...new Set(pts.map(p=>Math.round(p.x)))].sort((a,b)=>a-b),gaps=xs.slice(1).map((v,i)=>v-xs[i]).filter(d=>d>4).sort((a,b)=>a-b);
   const cell=Math.min(40,gaps.length?gaps[Math.floor(gaps.length/2)]:24);
   const colOf=p=>{const rc=rgb?rgb((p.z-mean)/span):[4,217,196];return new THREE.Color(`rgb(${rc[0]},${rc[1]},${rc[2]})`);};
   const newest=pts.reduce((a,p)=>!a||(p.ts||0)>(a.ts||0)?p:a,null);
   // every copy of one shape, placed at every point, merged into a single mesh with vertex colours
   const merged=(shape,z,scale=p=>1)=>{const sp=shape.attributes.position.array,si=shape.index.array,n=sp.length/3,pos=new Float32Array(pts.length*sp.length),col=new Float32Array(pts.length*sp.length),idx=[];
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
  // BLTouch LED. Klipper never reports the LED, so the caller derives it from what the probe was told
  // and said: on steady with the pin up, off while the pin is down, a flash each time it touches, and
  // fast flashing in alarm. Drawn on the front of the probe body with a glow, riding with the
  // toolhead.
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
  // One printed object, highlighted straight off the drawn toolpath's packed extrude array —
  // the same coordinates the plate already shows, so the highlight can never disagree with the
  // print. Used by the Exclude objects modal so "which one is this" is answered by the scene
  // instead of by a name.
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
 const installed=[];const spinners=[];let heatBlock=[],bedSheet=[];
 // The basic model is the stock reference assembly. Its hotend options form a variant group, so one
 // shows at a time; the two probe options are siblings, so the alternative probe and its bracket are
 // hidden and the inductive probe stays.
 const ALTERNATIVE_PROBE=/\/ Panasonic GX-H15A:1( \/|$)|probe_bracket_9mm/;
 function stockToolhead(){let hidden=0;
  for(const m of meshes){const d=m.userData;if(ALTERNATIVE_PROBE.test(d.full||'')||ALTERNATIVE_PROBE.test(d.name||'')){d.notFitted=true;m.visible=false;hidden++;}}
  heatBlock=meshes.filter(m=>/^head\.(block|nozzle|heatbreak|cartridge|core|sock)$/.test(m.userData.slot_id||'')&&!m.userData.notFitted);
  canvas.dataset.suppressed=String(hidden);}
 // Reported position drives the meshes. Datum: the reference nozzle tip in the CAD rest pose, read from
 // the loaded geometry, with the bed centre at (127,127) and the sheet top at Z≈0. Approximate by
 // construction (reference CAD, not a measured printer); unhomed axes stay at rest.
 function establishMotion(){const nozzle=meshes.find(m=>m.userData.slot_id==='head.nozzle');const sheet=meshes.find(m=>m.userData.name==='Flex Sheet:1');const bed=sourceData?.printer_frame?.bed_center;if(!nozzle||!bed)return;
  const tip=nozzle.geometry.boundingBox;const z0=sheet?sheet.geometry.boundingBox.max.z:bed[2];sourceData.printer_frame.source_pose=[175+(tip.min.x+tip.max.x)/2-bed[0],175+(tip.min.y+tip.max.y)/2-bed[1],tip.min.z-z0];sourceData.printer_frame.motion_basis='Nozzle tip in reference CAD mapped to Klipper bed centre 175,175 and sheet top Z=0; approximate';
  for(const m of meshes){const d=m.userData;if(d.motion&&d.motion!=='fixed')continue;const s=d.slot_id||'';d.motion=/^head(\.|$)/.test(s)?'toolhead':s==='x_rail'?'x_axis':(d.layer==='gantry'&&/^(gantry|drive_a|drive_b)$/.test(s))?'gantry':'fixed';}
  // The Y rails slid along in Y with the X beam, and the Z carriages on the frame verticals stayed put
  // while the gantry rose. Both classes come from the stored asset mapping rather than from here, so
  // they are corrected against the real machine: "MGN9 - 300mm (6)/(7)" under the XY joints ARE the
  // gantry's Y rails. The joints slide along them, so the rails themselves only ever travel in Z. They
  // were 'x_axis', which is what dragged them along in Y with the toolhead. The MGN9H carriages on the
  // frame verticals are bolted to the gantry's Z belt clamps, so they ride up and down with it. They
  // were 'fixed', so they stayed behind as the gantry moved. The vertical rails they run on stay
  // 'fixed', which is why the path is matched and not just the name.
  for(const m of meshes){const d=m.userData,n=d.name||'',f=d.full||'';
   // The rail-to-extrusion M3 screws in the MGN9 Y rails' own subtree stayed 'x_axis' and slid along Y
   // with the beam; they stay with the rail.
   if((/^MGN9 - 300mm/.test(n)||(/^M3x\d+ SHCS/.test(n)&&/\/ MGN9 - 300mm/.test(f)))&&/XY Joint - (Left|Right)/.test(f))d.motion='gantry';
   else if(/^MGN9H/.test(n)&&/Verticals Extrusions/.test(f))d.motion='gantry';
   // Measured in the loaded scene, the MGN12H block is the nine generic "PRODUCT_NAME_n" meshes
   // under the rail, X 104.3..149.7, Y 160..170, Z 64.2..91.2 — a 45 x 10 x 27 block wrapping the
   // rail, i.e. the part the toolhead is bolted to. They were 'x_axis', so they stayed on the beam
   // while the carriage plates rode off with the head. The six M3x8 SHCS in the same subtree sit at
   // X -13, 37, 87, 162, 212, 262 and reach up into the beam: those are the rail-to-beam screws and
   // stay 'x_axis'.
   else if(/^PRODUCT_NAME_/.test(n)&&/X Axis \(MGN12\)/.test(f)&&/MGN12 rail/.test(f))d.motion='toolhead';}
  // The X beam is not in the gantry group at all — "Frame:1 / Gantry Extrusions:1" holds the whole
  // gantry rectangle and all 59 of its meshes came in 'fixed'. HFSB5-20-330 (X -88..342, Y 173..193,
  // Z 67.7..87.7) is the X beam and must travel in Y and Z with the MGN12 rail bolted to its front
  // face; the other three (HFSB5-2020-240 at the rear, two HFSB5-2020-350 down the sides) are the
  // gantry frame and only rise in Z. The 55 T-nuts and screws belong to whichever member holds them,
  // so the split is by bounding box rather than by name: 14 travel with the beam, 45 with the frame.
  const extrusions=meshes.filter(m=>/Gantry Extrusions/.test(m.userData.full||''));
  const beam=extrusions.map(m=>m.geometry.boundingBox).reduce((a,b)=>!a||b.max.x-b.min.x>a.max.x-a.min.x?b:a,null);
  if(beam)for(const m of extrusions){const b=m.geometry.boundingBox,pad=8;
   m.userData.motion=['x','y','z'].every(k=>b.min[k]>=beam.min[k]-pad&&b.max[k]<=beam.max[k]+pad)?'x_axis':'gantry';}
  // A Belt:1 and B Belt:1 are each a single mesh carrying a whole CoreXY loop — X -107..361, Y
  // -65..420 — arriving layer:'gantry' with no slot_id, so the rule above left them 'fixed' and the
  // entire loop stood still while the beam slid out from under it. One loop cannot be one joint:
  // the runs across the gantry travel in Y with the beam, while the runs down the side rails only
  // change length, which a rigid mesh cannot show, and rise in Z with the gantry like everything
  // else on it. So each loop is split in two by triangle centroid Y against the gantry band, the
  // way the Gantry Extrusions were split by bounding box. The band is the MGN12 rail's own Y span
  // opened out to reach the belt clamp on the carriage (30 mm in front, 20 mm behind), measured
  // rather than typed. The gantry run becomes its own mesh on 'x_axis'; what is left keeps the
  // original mesh on 'gantry'.
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
   // The belt loops at the toolhead sat still in the middle of the beam. The two strands along
   // the beam end at the carriage clamps, so they stretch and shorten as the head moves and the
   // clamp loops ride with it (strandBelts, every pose).
   m.userData.strand={orig:Float32Array.from(run.attributes.position.array),x0:run.boundingBox.min.x,x1:run.boundingBox.max.x,y0:railBox.min.y-25,y1:railBox.max.y+15};m.frustumCulled=false;
   root.add(m);meshes.push(m);entries.set(id,m);
   beltSplit.push({belt:before.name,was:before.motion,band:[Number(lo.toFixed(2)),Number(hi.toFixed(2))],
    beam_tris:on.length/3,side_tris:off.length/3,
    beam_bbox:[run.boundingBox.min.toArray(),run.boundingBox.max.toArray()].map(a=>a.map(v=>Number(v.toFixed(2)))),
    side_bbox:[g.boundingBox.min.toArray(),g.boundingBox.max.toArray()].map(a=>a.map(v=>Number(v.toFixed(2))))});}
  canvas.dataset.beltSplit=JSON.stringify(beltSplit);
  // The X chain is suppressed (an umbilical feeds the toolhead), so what is shown is the Y
  // chain, and all eighteen of its links arrived 'gantry' — Z only — so the whole chain rode up and
  // down and never followed the beam in Y. It is not one mesh: each link is its own, which is what
  // makes the split honest. The chain is folded in the CAD with two runs one above the other; the
  // upper run carries the moving end bolted to the right XY joint's chain bridge (screws at Y
  // 178..192, Z 131..137) and travels with the beam, the lower run lies on the frame rail and does
  // not move at all. Split on the fold: a link whose box starts above the fold plane is on the
  // moving run. The U-turn between them is drawn rigid and will open a gap as the gantry travels,
  // because a link that rolls cannot be shown by a rigid mesh; the user judges whether that is
  // better than a chain that never moves. The split above drew the lower run 'fixed', so it stayed
  // behind when the gantry rose, and the U-turn was rigid. A drag chain rolls: every link keeps its
  // place along the chain, the lower run lies still on the gantry, the moving end travels with the
  // XY joint and the bend travels at half that speed. So each link now gets its arc position s along
  // the chain's centre line (lower run, half circle, upper run in the Y-Z plane) and applyPose
  // places it at the same s on the bent path for the current Y, turned by how far round the bend it
  // has rolled, and lifted with the gantry in Z.
  const chain=meshes.filter(m=>/\/ Y Cable Chain:/.test(m.userData.full||''));
  const chainSplit=[];
  if(chain.length){const cs=chain.map(m=>m.geometry.boundingBox.getCenter(new THREE.Vector3()));
   const links=cs.filter((c,i)=>!/FHCS|screw/i.test(chain[i].userData.name||'')),zb=Math.min(...links.map(c=>c.z)),zt=Math.max(...links.map(c=>c.z)),zc=(zb+zt)/2,R=(zt-zb)/2;
   const bend=links.filter(c=>c.z>zb+1&&c.z<zt-1),y0=Math.min(...links.map(c=>c.y));let yc=y0,best=Infinity;
   for(let y=y0;y<y0+2*R;y+=0.1){const d=bend.map(c=>Math.hypot(c.y-y,c.z-zc)),m=d.reduce((a,b)=>a+b,0)/Math.max(1,d.length),v=d.reduce((a,b)=>a+(b-m)**2,0);if(v<best){best=v;yc=y;}}
   const top=links.filter(c=>c.z>=zt-1),yEnd=Math.max(...top.map(c=>c.y));
   // The reference chain is four links too short for 350 mm of travel (at Y 0 the bend would
   // have to sit behind the joint it feeds), so the drawn chain is lengthened by whole links
   // copied from its lower run, inserted where the lower run meets the bend. The top run then
   // never runs out, and the bend sits that much further forward.
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
   if(zc.length){const cs=zc.map(m=>m.geometry.boundingBox.getCenter(new THREE.Vector3())),links=cs.filter((c,i)=>/^0 \(/.test(zc[i].userData.name||''));
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
 // A fan now turns: each gets a rotor on its intake (the fan STLs are one solid, so the blades are
 // drawn), the skirt's 6020s turn their own impellers, all at a visible fraction of the commanded
 // speed, about 30 frames a second while any runs. The flash is gone; a fan keeps a light cool tint
 // while it runs.
 let spinFrame=0,spinAt=0;const X_UP=new THREE.Vector3();
 function spin(now){spinFrame=0;const dt=Math.min(.1,(now-(spinAt||now))/1000);if(now-spinAt<33){spinFrame=requestAnimationFrame(spin);return;}spinAt=now;let any=false;
  for(const s of spinners){if(!(s.speed>0))continue;any=true;s.angle=((s.angle||0)+dt*s.speed*2*Math.PI*2.2)%(2*Math.PI);
   if(s.rotor){s.rotor.rotation.z=s.angle;}
   if(s.turn){const q=new THREE.Quaternion().setFromAxisAngle(s.turn.axis,s.angle),c=s.turn.centre;s.mesh.quaternion.copy(q);s.mesh.userData.motionOffset.copy(c).sub(c.clone().applyQuaternion(q));}}
  if(any){schedule();spinFrame=requestAnimationFrame(spin);}}
 // A drawn rotor: hub and seven swept blades in the XY plane, turned to face along `axis`.
 function addRotor(mesh,centre,axis,radius){const g=new THREE.Group(),mat=new THREE.MeshStandardMaterial({color:0x16191c,metalness:.25,roughness:.6});
  // Thin swept blades set inside the fan's opening, dark like the fan body.
  const hub=new THREE.Mesh(new THREE.CylinderGeometry(radius*.24,radius*.24,1.4,24),mat);hub.rotation.x=Math.PI/2;g.add(hub);
  for(let i=0;i<11;i++){const b=new THREE.Mesh(new THREE.BoxGeometry(radius*.46,radius*.2,.3),mat);const a=i/11*Math.PI*2;b.position.set(Math.cos(a)*radius*.74,Math.sin(a)*radius*.74,0);b.rotation.set(.65,0,a+.5);g.add(b);}
  g.position.copy(centre);g.quaternion.setFromUnitVectors(new THREE.Vector3(0,0,1),axis.clone().normalize());
  const inner=new THREE.Group();while(g.children.length)inner.add(g.children[0]);g.add(inner);g.traverse(o=>{o.raycast=()=>{};});mesh.add(g);return inner;}
 // Heat moves a part's own colour toward hot metal as well as lighting it, over a range that
 // matches the machine: the
 // nozzle and its sock from 150 to 250 °C, the plate from 40 to 80 °C. Cold parts are untouched.
 // The blowers and the hotend fan run the same lerp the other way, toward cool blue with fan speed,
 // because the emissive pulse alone was invisible on a near-black 4010.
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
 // Source meshes to leave out of an inspected CAD file, by asset id (name pattern). Empty by default.
 const SOURCE_SKIP={};
 async function inspectSource(assetId){const source=sourceAssets.find(a=>a.id===assetId);if(!source)throw Error('Source file is not in the managed library.');const id=source.id+':root:0';if(entries.has(id))return id;const response=await fetch(source.geometry_url);if(!response.ok)throw Error('Component source geometry is unavailable.');const data=await response.json();
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

 function preview(id,target){mini?.dispose();mini=null;target.replaceChildren();const original=entries.get(id);if(!original)return;const cv=document.createElement('canvas');cv.setAttribute('aria-label','Selected component source model');target.append(cv);// a black part on a near-black well could not be read. The preview is
  // its own studio now: it clears to transparent over a lighter cyclorama well (hud.css), tone maps
  // like the scene, and lights the part with a key, a fill and a rim from behind that edges dark
  // parts against the well. The part's own colour is never changed.
  const r=new THREE.WebGLRenderer({canvas:cv,antialias:true,alpha:true});r.setPixelRatio(Math.min(devicePixelRatio,1.5));r.setClearColor(0x000000,0);r.outputColorSpace=THREE.SRGBColorSpace;r.toneMapping=THREE.ACESFilmicToneMapping;r.toneMappingExposure=1.5;const s=new THREE.Scene(),c=new THREE.PerspectiveCamera(36,1,.01,100000);c.up.set(0,0,1);s.add(new THREE.HemisphereLight(0xffffff,0x3a4448,2.2));
  for(const [color,intensity,x,y,z]of [[0xffffff,3.2,1,-1.2,2],[0xdfeaf0,1.3,-1.5,-.6,.8],[0xffffff,3,-.8,1.6,1.4]]){const light=new THREE.DirectionalLight(color,intensity);light.position.set(x,y,z);s.add(light);}const mesh=new THREE.Mesh(original.geometry,original.material.clone());mesh.material.emissive.setHex(0);s.add(mesh);const box=new THREE.Box3().setFromObject(mesh),center=box.getCenter(new THREE.Vector3()),diam=box.getSize(new THREE.Vector3()).length();c.position.copy(center).addScaledVector(new THREE.Vector3(1,-1,.7).normalize(),diam*1.9);const controls=new OrbitControls(c,cv);controls.target.copy(center);controls.update();const draw=()=>{const w=target.clientWidth,h=target.clientHeight;if(w&&h){r.setSize(w,h,false);c.aspect=w/h;c.updateProjectionMatrix();r.render(s,c);}};controls.addEventListener('change',draw);const observer=new ResizeObserver(draw);observer.observe(target);draw();const initialPosition=c.position.clone();mini={theme(){draw();},refreshColor(){mesh.material.color.copy(original.material.color);draw();},get distance(){return c.position.distanceTo(controls.target);},zoom(scale){c.position.sub(controls.target).multiplyScalar(scale).add(controls.target);controls.update();draw();},reset(){controls.target.copy(center);c.position.copy(initialPosition);controls.update();draw();},dispose(){observer.disconnect();controls.dispose();mesh.material.dispose();r.dispose();}};}
  // The scene follows the interface theme: a lit studio ground in light, neon-lit dark otherwise.
  // state staging: the key light leans with what the machine is doing — warm while heating,
  // cool-bright while printing, magenta-washed on an error, calm when idle. Subtle on purpose: the
  // brand neons stay accents in the chrome, never a flood on the model. theme re-applies the
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
 const api={hidden,live,theme,stage,get world(){return world;},highlightObject,get stageLight(){return world.children.find(c=>c.isDirectionalLight&&!c.userData.stageLight)?.color.getHexString();},get bedCenter(){return sourceData?.printer_frame?.bed_center||null;},loadToolpath,toolpathProgress,clearToolpath,meshOverlay,probeMarks,probeLed,get toolpath(){return toolpath?{name:toolpath.name,segments:toolpath.segments,layers:toolpath.parsed.layers.length}:null;},look(position,target){camera.position.set(...position);orbit.target.set(...target);orbit.update();updateCut();schedule();},geometryBounds(id){const b=entries.get(id)?.geometry.boundingBox;return b?{min:b.min.toArray(),max:b.max.toArray(),size:b.getSize(new THREE.Vector3()).toArray()}:null;},setColor,resetColors,color(id){const m=entries.get(id);return m?'#'+m.material.color.getHexString():null;},get colorGroups(){return Object.entries(colorNames).map(([id,name])=>({id,name,count:meshes.filter(m=>m.userData.colorGroup===id&&!m.userData.notFitted).length,color:colors.groups[id]||colors.all||meshes.find(m=>m.userData.colorGroup===id)?.userData.sourceColor||'#697985'}));},zoom,orbitBy,orientation,reset(){clearFocus();hiddenParts.clear();hidden.clear();hidden.add('covers');explode=0;for(const m of meshes){m.visible=visible(m);m.position.copy(m.userData.motionOffset);}fit(meshes,'iso');},previewZoom(scale){mini?.zoom(scale);},previewReset(){mini?.reset();},get previewDistance(){return mini?.distance;},get visibleParts(){return meshes.filter(m=>visible(m)).map(m=>m.userData.id);},get cameraState(){return {position:camera.position.toArray(),target:orbit.target.toArray()};},inspectSource,get sourceAssets(){return sourceAssets;},get variants(){return [...variants].map(([group,selected])=>({group,selected,options:[...new Set(meshes.filter(m=>m.userData.variantGroup===group).map(m=>m.userData.variantName))]}));},variant(group,name){if(meshes.some(m=>m.userData.variantGroup===group&&m.userData.variantName===name)){variants.set(group,name);schedule();}},hideSelected(){if(selected)hiddenParts.add(selected.userData.id);schedule();},isolate(){if(selected){clearFocus();hidden.clear();for(const m of meshes)if(m!==selected)hiddenParts.add(m.userData.id);hiddenParts.delete(selected.userData.id);schedule();fit([selected]);}},showAll(){clearFocus();hidden.clear();hiddenParts.clear();schedule();},panels(value){for(const m of meshes.filter(m=>m.userData.layer==='covers')){m.userData.panelTransparent=value;m.material.transparent=value;m.material.opacity=value?.15:1;m.material.depthWrite=!value;}hidden.delete('covers');schedule();},metrics,renderer,load,jobUpdate,loadJob,preview,closePreview(){mini?.dispose();mini=null;},associateJob(name){jobName=name;return jobUpdate(jobState);},set,schedule,view(v){const distance=camera.position.distanceTo(orbit.target);camera.position.copy(orbit.target).addScaledVector(new THREE.Vector3(...CAMERA_VIEWS[v]).normalize(),distance);orbit.update();schedule();},focus(id){const m=entries.get(id)||meshes.find(m=>m.userData.record_id===id||m.userData.slot_id===id);if(m)focusPart(m);},select,get selected(){return selected?.userData;},get entries(){return [...entries.values()].map(m=>m.userData);},get explode(){return explode;},set explode(v){explode=v;},get jobPreview(){return jobModel?{position:jobModel.position.toArray(),progress:jobDisplay(jobState,jobName),clip:jobModel.children[1].material.clippingPlanes[0].constant}:null;},get source(){return asset;}};
 window.__stellaScene=api;load();return api;
}
