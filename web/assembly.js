import * as THREE from './vendor/three.module.js';
import {OrbitControls} from './vendor/OrbitControls.js';

const el=(tag,cls='',text)=>{const e=document.createElement(tag);e.className=cls;if(text!==undefined)e.textContent=text;return e;};
const button=(label,action)=>{const b=el('button','',label);b.type='button';b.addEventListener('click',action);return b;};
const label=(name,input)=>{const e=el('label','',name);e.append(input);return e;};
// What a control is (title) and what clicking it does (data-tip-do), for the HUD's one tooltip.
// Outside the HUD the title is the browser's own tooltip.
const tipped=(e,what,does='')=>{e.title=what;if(does)e.dataset.tipDo=does;return e;};
const views={Isometric:[-1,-1,.8],Front:[0,-1,0],Rear:[0,1,0],Left:[-1,0,0],Right:[1,0,0],Top:[0,0,1]};
const compact=n=>new Intl.NumberFormat(undefined,{notation:'compact',maximumFractionDigits:1}).format(n);
const field=(parent,key,value)=>{const row=el('div','assembly-field');row.append(el('dt','',key),el('dd','',value||'Not recorded'));parent.append(row);};
async function json(url){const r=await fetch(url);if(!r.ok){let message=`Request failed (${r.status})`;try{message=(await r.json()).error||message;}catch{}throw Error(message);}return r.json();}

export function mountAssembly(host,{openPart,timeState,transport,canTransport}){
 const workspace=el('div','assembly-workspace');host.append(workspace);
 const heading=el('div','assembly-heading');heading.append(el('div','', 'SOURCE-BACKED ASSEMBLY'),el('span','assembly-proof','Visual review · RP-01'));
 const introduction=el('p','assembly-intro','Your hardware, with its source files. Inspect an assembly or a single component; the source revision stays attached to every selection.');
 const controls=el('div','assembly-controls'),assetSelect=el('select');assetSelect.setAttribute('aria-label','Source assembly');tipped(assetSelect,'Source assembly','Picks which imported assembly is shown.');
 const reload=tipped(button('Reload library',loadLibrary),'Reload library','Reads the managed file library again.'),importHelp=tipped(button('Import files',showImport),'Import files','Shows how source files are imported on the recorder computer.'),exportLibrary=tipped(button('Export manifest',()=>download('Stella_Asset_Manifest.json',JSON.stringify(manifest,null,2))),'Export manifest','Downloads the library manifest as a JSON file.');
 controls.append(label('Source',assetSelect),reload,importHelp,exportLibrary);
 const status=el('p','assembly-status','Reading managed file library…');status.setAttribute('role','status');
 const grid=el('div','assembly-grid'),navigation=el('aside','assembly-navigation'),tree=el('div','assembly-tree');
 const search=el('input');search.type='search';search.placeholder='Find a component…';search.setAttribute('aria-label','Find assembly component');tipped(search,'Find a component','Type part of a name to filter the assembly tree.');
 navigation.append(el('h2','','Assembly'),search,tree);
 const stage=el('div','assembly-stage'),toolbar=el('div','assembly-camera'),canvas=el('canvas');canvas.tabIndex=0;canvas.setAttribute('aria-label','Imported source geometry. Drag to orbit; right drag to pan; Control plus wheel to zoom. Use assembly navigation to select hidden components.');tipped(canvas,'Assembly view','Drag to orbit, right drag to pan, Ctrl and wheel to zoom, double-click to focus.');canvas.dataset.tipFocusOnly='1';
 const viewport=el('div','assembly-viewport'),caption=el('div','assembly-caption'),hover=el('div','assembly-hover');hover.hidden=true;
 const title=el('h2','','No imported geometry'),basis=el('p','','No source selected');caption.append(title,basis);viewport.append(canvas,caption,hover);
 const viewTools=el('div','assembly-view-tools'),explode=el('input');explode.type='range';explode.min=0;explode.max=100;explode.value=0;explode.setAttribute('aria-label','Explode source components');tipped(explode,'Separate','Drag to pull the components apart.');
 const transparent=el('input');transparent.type='checkbox';tipped(transparent,'X-ray','Makes the parts see-through so hidden components show.');
 const isolate=tipped(button('Isolate',()=>{if(selected){meshes.forEach(m=>m.visible=selected.objects.includes(m));schedule();}}),'Isolate','Shows only the selected component.');
 const hide=tipped(button('Hide',()=>{if(selected){selected.objects.forEach(m=>m.visible=false);schedule();}}),'Hide','Hides the selected component.');
 const showAll=tipped(button('Show all',()=>{meshes.forEach(m=>m.visible=true);schedule();}),'Show all','Shows every component again.');
 viewTools.append(tipped(button('Fit visible',()=>fit()),'Fit visible','Frames the camera on the visible components.'),isolate,hide,showAll,label('Separate',explode),label('X-ray',transparent));
 const time=el('div','assembly-time'),timeText=el('span');time.append(el('span','assembly-static','STATIC SOURCE REVISION'),timeText);
 const timeButtons=el('div','assembly-time-buttons');for(const [name,id]of [['Live','live'],['Previous','prev'],['Play / pause','play'],['Next','next']]){const b=tipped(button(name,()=>transport(id)),name,{live:'Jumps to the newest reading.',prev:'Steps the recording cursor back one sample.',play:'Plays or pauses the recording.',next:'Steps the recording cursor forward one sample.'}[id]);b.dataset.transport=id;timeButtons.append(b);}time.append(timeButtons);
 stage.append(toolbar,viewport,viewTools,el('p','assembly-gestures','Drag to orbit · Right drag to pan · Ctrl + wheel to zoom · Double-click to focus · Plain scroll moves the page'),time);
 const inspector=el('aside','assembly-inspector');grid.append(navigation,stage,inspector);
 workspace.append(heading,introduction,controls,status,grid);
 const dialog=el('dialog','assembly-dialog');workspace.append(dialog);
 let manifest={schema:'stella-scope.assets.v1',assets:[]},asset=null,selected=null,meshes=[],entries=[],loadSeq=0,renderer=null,frame=0,treeButtons=new Map();
 let scene,camera,orbit,group,outline,modelBox,modelSize=100,ground;
 const metrics={loads:[],frames:[],renderer:null};
 // Exposes measured timings and stable selection IDs for local validation, never printer state.
 const inspection={metrics,get selected(){return selected?.id;},get entries(){return entries.map(e=>({id:e.id,name:e.name}));},get renderer(){return renderer;}};
 window.__stellaAssembly=inspection;
 const report=(message,failed=false)=>{status.textContent=message;status.classList.toggle('failed',failed);};
 function initRenderer(){
  if(renderer)return;
  renderer=new THREE.WebGLRenderer({canvas,antialias:true,alpha:false});renderer.setPixelRatio(Math.min(devicePixelRatio,1.5));
  renderer.setClearColor(0x1c2022);renderer.outputColorSpace=THREE.SRGBColorSpace;renderer.toneMapping=THREE.ACESFilmicToneMapping;renderer.toneMappingExposure=1.2;
  scene=new THREE.Scene();camera=new THREE.PerspectiveCamera(34,1,.1,100000);camera.up.set(0,0,1);
  scene.add(new THREE.HemisphereLight(0xf2f5ed,0x525b61,2.6));
  const key=new THREE.DirectionalLight(0xfff1dd,3.4);key.position.set(-500,-600,900);scene.add(key);
  const fill=new THREE.DirectionalLight(0xd3e8f5,2);fill.position.set(600,300,300);scene.add(fill);
  group=new THREE.Group();scene.add(group);orbit=new OrbitControls(camera,canvas);orbit.enableDamping=false;orbit.enableZoom=false;orbit.addEventListener('change',schedule);
  canvas.addEventListener('wheel',e=>{if(!e.ctrlKey&&!e.metaKey&&!e.shiftKey)return;e.preventDefault();const v=camera.position.clone().sub(orbit.target);v.multiplyScalar(Math.exp(e.deltaY*.001));v.clampLength(modelSize*.05,modelSize*15);camera.position.copy(orbit.target).add(v);orbit.update();schedule();},{passive:false});
  let down=null,hoverAt=0;
  canvas.addEventListener('pointerdown',e=>{down=[e.clientX,e.clientY];hover.hidden=true;});
  canvas.addEventListener('pointerup',e=>{if(down&&Math.hypot(e.clientX-down[0],e.clientY-down[1])<4){const m=pick(e);if(m)selectEntry(m.userData.entry);}down=null;});
  canvas.addEventListener('pointermove',e=>{if(e.buttons||performance.now()-hoverAt<80)return;hoverAt=performance.now();const m=pick(e);hover.hidden=!m;canvas.style.cursor=m?'pointer':'grab';if(m){const r=viewport.getBoundingClientRect();hover.textContent=m.userData.entry.name;hover.style.left=`${Math.max(8,Math.min(r.width-210,e.clientX-r.left+12))}px`;hover.style.top=`${Math.max(8,e.clientY-r.top-34)}px`;}});
  canvas.addEventListener('pointerleave',()=>{hover.hidden=true;});canvas.addEventListener('dblclick',()=>fit(selected?.objects));
  canvas.addEventListener('keydown',e=>{if(e.key==='0'){fit();e.preventDefault();}if(e.key==='+'||e.key==='-'||e.key==='='){camera.position.sub(orbit.target).multiplyScalar(e.key==='-'?1.15:1/1.15).add(orbit.target);orbit.update();schedule();e.preventDefault();}});
  new ResizeObserver(resize).observe(viewport);metrics.renderer=renderer.getContext().getParameter(renderer.getContext().RENDERER);resize();
 }
 function resize(){if(!renderer)return;const w=viewport.clientWidth,h=viewport.clientHeight;if(!w||!h)return;renderer.setSize(w,h,false);camera.aspect=w/h;camera.updateProjectionMatrix();schedule();}
 function schedule(){if(!renderer||frame||host.hidden)return;frame=requestAnimationFrame(()=>{frame=0;const t=performance.now();renderer.render(scene,camera);metrics.frames.push(performance.now()-t);if(metrics.frames.length>600)metrics.frames.shift();});}
 function pick(event){const r=canvas.getBoundingClientRect(),ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2((event.clientX-r.left)/r.width*2-1,1-(event.clientY-r.top)/r.height*2),camera);return ray.intersectObjects(meshes.filter(m=>m.visible),false)[0]?.object;}
 function fit(objects,view){if(!meshes.length)return;const box=new THREE.Box3();(objects||meshes.filter(m=>m.visible)).forEach(m=>box.expandByObject(m));if(box.isEmpty())return;const center=box.getCenter(new THREE.Vector3()),size=box.getSize(new THREE.Vector3());const direction=view?new THREE.Vector3(...views[view]).normalize():camera.position.clone().sub(orbit.target).normalize();if(!direction.length())direction.set(1,-1,.8).normalize();const distance=size.length()*.62/Math.tan(THREE.MathUtils.degToRad(camera.fov/2))/Math.min(1,camera.aspect);orbit.target.copy(center);camera.position.copy(center).addScaledVector(direction,Math.max(distance,1));camera.near=Math.max(.01,size.length()/10000);camera.far=Math.max(10000,distance*30);camera.updateProjectionMatrix();orbit.update();schedule();}
 for(const name of Object.keys(views))toolbar.append(tipped(button(name,()=>fit(null,name)),name+' view','Turns the camera to the '+name.toLowerCase()+' view.'));
 toolbar.append(tipped(button('Full screen',async()=>{try{if(document.fullscreenElement)await document.exitFullscreen();else await workspace.requestFullscreen();resize();}catch(e){report(e.message,true);}}),'Full screen','Fills the screen with the assembly; Escape leaves.'));
 explode.addEventListener('input',()=>{if(!modelBox)return;const center=modelBox.getCenter(new THREE.Vector3()),amount=Number(explode.value)/100;meshes.forEach((m,i)=>{const v=m.userData.center.clone().sub(center);if(v.length()<.001)v.set(0,0,modelSize*.1*(i+1));m.position.copy(v.multiplyScalar(amount*.7));});if(outline&&selected)outline.box.setFromObject(selected.objects[0]);schedule();});
 transparent.addEventListener('change',()=>{meshes.forEach(m=>{m.material.transparent=transparent.checked;m.material.opacity=transparent.checked?.28:1;m.material.depthWrite=!transparent.checked;});schedule();});
 search.addEventListener('input',renderTree);assetSelect.addEventListener('change',()=>loadAsset(assetSelect.value));
 function clearGeometry(){if(!group)return;for(const m of meshes){m.geometry.dispose();m.material.dispose();}group.clear();meshes=[];entries=[];selected=null;if(outline){scene.remove(outline);outline.dispose();outline=null;}if(ground){scene.remove(ground);ground.geometry.dispose();ground.material.dispose();ground=null;}explode.value=0;transparent.checked=false;schedule();}
 async function loadLibrary(){
  reload.disabled=true;
  try{manifest=await json('/api/assets');if(manifest.schema!=='stella-scope.assets.v1'||!Array.isArray(manifest.assets))throw Error('Unsupported asset library. Preserve it and check the importer version.');for(const a of manifest.assets){if(typeof a.id!=='string'||!/^[a-z0-9_-]{1,64}$/.test(a.id)||typeof a.name!=='string'||!['stl','step'].includes(a.format)||a.geometry_url!==`/api/assets/${a.id}/geometry`||a.original_url!==`/api/assets/${a.id}/original`)throw Error('Invalid asset identity or managed-file URL. Preserve the manifest and inspect the failed import.');}assetSelect.replaceChildren();for(const a of manifest.assets){const o=el('option','',a.name);o.value=a.id;assetSelect.append(o);}exportLibrary.disabled=!manifest.assets.length;
   if(manifest.assets.length){const id=manifest.assets.find(a=>a.id===asset?.id)?.id||manifest.assets[0].id;assetSelect.value=id;await loadAsset(id);}else{++loadSeq;clearGeometry();asset=null;tree.replaceChildren();inspector.replaceChildren(el('h2','','Start with a real file'),el('p','','Import a STEP assembly or STL using the local import utility. Originals remain unchanged.'));title.textContent='No source files yet';basis.textContent='No geometry has been substituted.';report('Your managed library is empty. Import files to begin.');schedule();}
  }catch(e){report(`Library unavailable: ${e.message}`,true);}finally{reload.disabled=false;}
 }
 async function loadAsset(id){
  const seq=++loadSeq,next=manifest.assets.find(a=>a.id===id);if(!next)return;asset=next;report(`Loading ${asset.name}…`);title.textContent=asset.name;basis.textContent=`${asset.status} · ${asset.format.toUpperCase()} source · millimetres`;tree.replaceChildren();inspector.replaceChildren();clearGeometry();
  try{initRenderer();const start=performance.now(),data=await json(asset.geometry_url);if(seq!==loadSeq)return;
   if(!data.root||!Array.isArray(data.meshes)||!data.meshes.length)throw Error('Source contains no supported geometry.');
   const geometries=data.meshes.map(raw=>{const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(raw.attributes.position.array,3));g.setIndex(raw.index.array);if(raw.attributes.normal?.array?.length===raw.attributes.position.array.length)g.setAttribute('normal',new THREE.Float32BufferAttribute(raw.attributes.normal.array,3));else g.computeVertexNormals();g.computeBoundingBox();return g;});
   const walk=(node,path,depth)=>{const own=[];for(const [i,index]of (node.meshes||[]).entries()){const raw=data.meshes[index],geometry=geometries[index];if(!geometry)throw Error('Assembly references missing mesh.');const color=raw.color?new THREE.Color().setRGB(...raw.color,THREE.SRGBColorSpace):new THREE.Color(0xb9bec1);const material=new THREE.MeshStandardMaterial({color,roughness:.48,metalness:.22,side:THREE.DoubleSide});const object=new THREE.Mesh(geometry,material);const entry={id:node.instance_ids?.[i]||`${asset.id}:${path}:${index}`,name:raw.name||node.name||`Component ${index+1}`,path,depth,objects:[object],index};object.userData.entry=entry;object.userData.center=geometry.boundingBox.getCenter(new THREE.Vector3());group.add(object);meshes.push(object);entries.push(entry);own.push(object);}
    const children=[];for(const [i,child]of (node.children||[]).entries())children.push(...walk(child,`${path}/${i}`,depth+1));const objects=[...own,...children];if(objects.length>1)entries.push({id:`${asset.id}:${path}`,name:node.name||'Assembly',path,depth,objects,index:null});return objects;};
   walk(data.root,'0',0);entries.sort((a,b)=>a.path.localeCompare(b.path,undefined,{numeric:true})||(a.index===null?-1:1));modelBox=new THREE.Box3().setFromObject(group);modelSize=modelBox.getSize(new THREE.Vector3()).length();
   const size=modelBox.getSize(new THREE.Vector3()),center=modelBox.getCenter(new THREE.Vector3());ground=new THREE.GridHelper(Math.max(size.x,size.y,50)*2,20,0x424b4f,0x30383c);ground.rotation.x=Math.PI/2;ground.position.set(center.x,center.y,modelBox.min.z-modelSize*.025);scene.add(ground);
   renderTree();fit(null,'Isometric');selectEntry(entries.find(e=>e.index===null)||entries[0]);
   const elapsed=performance.now()-start;metrics.loads.push({id,ms:elapsed,meshes:meshes.length,triangles:asset.triangle_count,bounds_mm:size.toArray()});
   report(`${compact(meshes.length)} selectable mesh instances · ${compact(asset.triangle_count)} triangles · loaded ${(elapsed/1000).toFixed(2)} s · source revision, not live motion`);
  }catch(e){if(seq===loadSeq){clearGeometry();tree.replaceChildren();inspector.replaceChildren(el('h2','','Source could not be displayed'),el('p','',e.message));report(`Import unavailable: ${e.message}`,true);schedule();}}
 }
 function renderTree(){tree.replaceChildren();treeButtons=new Map();const q=search.value.trim().toLowerCase();for(const entry of entries){if(q&&!`${entry.name} ${entry.id}`.toLowerCase().includes(q))continue;const b=button(entry.name,()=>selectEntry(entry));b.className=entry.index===null?'assembly-node assembly-parent':'assembly-node';b.style.setProperty('--depth',Math.min(entry.depth,5));tipped(b,entry.name,`Selects it and shows its source details (${entry.id}).`);b.setAttribute('aria-pressed',String(selected?.id===entry.id));treeButtons.set(entry.id,b);tree.append(b);}if(!tree.childElementCount)tree.append(el('p','','No matching components.'));}
 function selectEntry(entry){if(!entry)return;selected=entry;for(const [id,b]of treeButtons)b.setAttribute('aria-pressed',String(id===entry.id));const whole=entry.objects.length===meshes.length;for(const m of meshes)m.material.emissive.setHex(!whole&&entry.objects.includes(m)?0x173d36:0);if(outline){scene.remove(outline);outline.dispose();outline=null;}if(!whole){const box=new THREE.Box3();entry.objects.forEach(m=>box.expandByObject(m));outline=new THREE.Box3Helper(box,0x82b9a8);scene.add(outline);}detail();schedule();}
 function detail(){inspector.replaceChildren();inspector.append(el('span','assembly-kicker',selected.index===null?'ASSEMBLY INSTANCE':'COMPONENT INSTANCE'),el('h2','',selected.objects.length===meshes.length?asset.name:selected.name),el('span',`assembly-state ${asset.status}`,asset.status));
  const list=el('dl');field(list,'Source revision',asset.name);field(list,'Component',selected.index===null?`${selected.objects.length} meshes`:selected.id);field(list,'Inventory location',selected.objects.length===meshes.length?asset.slot_id:'This source instance is not mapped yet');field(list,'Manufacturer / model',[asset.manufacturer,asset.model].filter(Boolean).join(' / '));field(list,'Revision',asset.revision);field(list,'Placement',asset.placement?.basis||'Unconfirmed source placement');
  const actions=el('div','assembly-actions');actions.append(tipped(button('Focus component',()=>fit(selected.objects)),'Focus component','Frames the camera on the selected component.'),tipped(button('File & provenance',showProvenance),'File & provenance','Opens the source file record: hashes, licence and placement.'));if(asset.slot_id&&selected.objects.length===meshes.length)actions.append(tipped(button('Inventory & signals',()=>openPart(asset.slot_id)),'Inventory & signals','Opens this location\'s inventory and signals.'));inspector.append(actions,list,el('p','assembly-note',asset.notes||'Source geometry is not proof that this revision is fitted.'));
  inspector.append(el('p','assembly-limit','Static geometry inspection. Recorded motion remains in Machine; this source is not silently substituted into historical recordings.'));
 }
 function openDialog(titleText){dialog.replaceChildren();const top=el('div','assembly-dialog-heading');top.append(el('h2','',titleText),tipped(button('Close',()=>dialog.close()),'Close','Closes this window.'));dialog.append(top);dialog.showModal();}
 function showProvenance(){if(!asset)return;openDialog('Source file & provenance');const list=el('dl');for(const [name,value]of [['File',asset.source_name],['Storage','Managed copy · original left in place'],['Original location',asset.source_path],['Source SHA-256',asset.source_sha256],['Display SHA-256',asset.derived_sha256],['Imported',asset.imported_at],['License',asset.license],['Source units',asset.units],['Displayed coordinates','Millimetres; declared placement baked into display mesh'],['Placement basis',asset.placement?.basis],['Revision',asset.revision]])field(list,name,value);dialog.append(list);
  const a=el('a','assembly-file','Download original file');a.href=asset.original_url;a.download=asset.source_name;if(asset.availability?.original==='missing'){a.removeAttribute('href');a.textContent='Managed original is missing';}dialog.append(a);
  if(asset.source_url){try{const url=new URL(asset.source_url);if(['https:','http:'].includes(url.protocol)){const source=el('a','assembly-file','View source reference');source.href=url.href;source.target='_blank';source.rel='noopener noreferrer';source.referrerPolicy='no-referrer';dialog.append(source);}}catch{}}
  dialog.append(el('p','assembly-note','The location is on the recorder computer. A browser cannot open that computer’s folder. Use the download above; desktop Open file / Show in folder is not implemented in this slice.'),el('p','assembly-note',asset.format==='stl'?'This is the original STL mesh, including its probe attachment. No editable parametric STEP is claimed.':'This display was tessellated from STEP. Source hierarchy is retained where supplied; editable joints are not reconstructed.'));
 }
 function showImport(){openDialog('Import real source geometry');dialog.append(el('p','','This first slice uses the local import utility on the recorder computer. Choose an exact STL or STEP file in its import specification, confirm units and placement, and retain its source and revision. Importing never marks a part installed.'),el('pre','','node tools/asset-import/import.mjs --database /path/recording.sqlite3 --spec /path/part.json'),el('p','assembly-note','The utility creates a managed copy and a separate display mesh beside the recording. See tools/asset-import/README.md for the specification. Reload library after importing. Guided file/folder selection, 3MF and desktop file-opening follow in RP-02.'));
 }
 function download(name,data){const url=URL.createObjectURL(new Blob([data],{type:'application/json'})),a=el('a');a.href=url;a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
 function updateTime(){timeText.textContent=timeState();for(const b of timeButtons.children)b.disabled=!canTransport(b.dataset.transport);}
 const timer=setInterval(()=>{if(!host.hidden)updateTime();},1000);updateTime();loadLibrary();
 return {refresh(){resize();updateTime();},dispose(){clearInterval(timer);renderer?.dispose();orbit?.dispose();}};
}
