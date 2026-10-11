// Charts and the bed-mesh surface. Drawn here rather than shipping Klipper's PNGs, so the numbers
// can be explained in plain words and compared with the last run.
import * as THREE from './vendor/three.module.js';
import {OrbitControls} from './vendor/OrbitControls.js';

const clamp=(v,a,b)=>Math.max(a,Math.min(b,v));
// Blue = low, black = on plane, orange = high. Symmetric so the plate reads as flat when it is.
export function meshColor(v,span){const k=clamp(v/(span||1),-1,1);
 return k<0?new THREE.Color(0.10-0.10*k,0.42-0.18*k,0.85+0.15*k):new THREE.Color(0.95,0.45+0.25*(1-k),0.20*(1-k));}

/** Draw a probed bed mesh as a real surface. grid is rows of millimetres, already probed. */
export function meshSurface(canvas,{grid,min,max,profile=''}){
 if(!Array.isArray(min)||!Array.isArray(max)||[...min,...max].length!==4||[...min,...max].some(v=>!Number.isFinite(v))||max[0]<=min[0]||max[1]<=min[1])throw Error('Mesh X/Y bounds were not recorded; surface position cannot be drawn accurately.');
 const rows=grid.length,cols=grid[0].length;
 const flat=grid.flat().filter(v=>typeof v==='number'&&Number.isFinite(v));
 if(!flat.length)throw Error('That mesh has no probed points.');
 const lo=Math.min(...flat),hi=Math.max(...flat),mid=(lo+hi)/2,span=Math.max(0.02,(hi-lo)/2);
 const w=max[0]-min[0],d=max[1]-min[1];
 const renderer=new THREE.WebGLRenderer({canvas,antialias:true});renderer.setPixelRatio(Math.min(devicePixelRatio,1.5));renderer.setClearColor(document.body.dataset.theme==='light'?0xe3ebe8:0x14191a);
 const scene=new THREE.Scene(),camera=new THREE.PerspectiveCamera(36,1,.1,5000);camera.up.set(0,0,1);
 scene.add(new THREE.HemisphereLight(0xdfefff,0x101820,2.6));
 const key=new THREE.DirectionalLight(0xffffff,2.2);key.position.set(1,-1.4,2);scene.add(key);
 // Exaggerate Z so a 0.1 mm bow is visible without lying about the numbers: the scale is labelled.
 const exaggeration=Math.min(900,Math.max(120,60/Math.max(0.01,hi-lo)));
 const geometry=new THREE.PlaneGeometry(w,d,cols-1,rows-1);
 const position=geometry.attributes.position,colors=[];
 for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){
  const i=r*cols+c,v=grid[r][c];const z=Number.isFinite(v)?(v-mid)*exaggeration:0;
  position.setZ(i,z);const col=meshColor(Number.isFinite(v)?v-mid:0,span);colors.push(col.r,col.g,col.b);}
 geometry.setAttribute('color',new THREE.Float32BufferAttribute(colors,3));
 geometry.computeVertexNormals();
 const surface=new THREE.Mesh(geometry,new THREE.MeshStandardMaterial({vertexColors:true,roughness:.55,metalness:.05,side:THREE.DoubleSide}));
 scene.add(surface);
 const wire=new THREE.Mesh(geometry,new THREE.MeshBasicMaterial({color:0x0a1418,wireframe:true,transparent:true,opacity:.35}));scene.add(wire);
 const plane=new THREE.GridHelper(Math.max(w,d),12,0x1d3b45,0x142a31);plane.rotation.x=Math.PI/2;plane.position.z=-Math.max(6,(hi-lo)*exaggeration);scene.add(plane);
 const radius=Math.max(w,d);
 camera.position.set(radius*.85,-radius*1.05,radius*.75);
 const controls=new OrbitControls(camera,canvas);controls.enableDamping=false;
 const draw=()=>{const cw=canvas.clientWidth,ch=canvas.clientHeight;if(!cw||!ch)return;
  renderer.setSize(cw,ch,false);camera.aspect=cw/ch;camera.updateProjectionMatrix();renderer.render(scene,camera);};
 controls.addEventListener('change',draw);const ro=new ResizeObserver(draw);ro.observe(canvas);draw();
 // Worst corner, measured from the mesh itself rather than guessed.
 let worst={v:0,r:0,c:0};
 for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){const v=grid[r][c];if(Number.isFinite(v)&&Math.abs(v-mid)>Math.abs(worst.v)) worst={v:v-mid,r,c};}
 const corner=`${worst.r<rows/2?'front':'back'} ${worst.c<cols/2?'left':'right'}`;
 return {range:hi-lo,low:lo,high:hi,points:flat.length,rows,cols,profile,exaggeration,
  worst:{delta:worst.v,corner,x:min[0]+worst.c*(w/(cols-1)),y:min[1]+worst.r*(d/(rows-1))},
  dispose(){ro.disconnect();controls.dispose();geometry.dispose();renderer.dispose();}};
}

/** Minimal line chart: several series on a shared time axis, drawn on a 2D canvas. */
export function lineChart(canvas,series,{unit='',height=210}={}){
 const dpr=Math.min(devicePixelRatio,2),w=canvas.clientWidth||600,h=height;
 canvas.width=w*dpr;canvas.height=h*dpr;canvas.style.height=h+'px';
 const g=canvas.getContext('2d');g.scale(dpr,dpr);g.clearRect(0,0,w,h);
 const live=series.filter(s=>s.points.length>1);
 if(!live.length){g.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim()||'#a5b2b2';g.font='12px ui-monospace,monospace';g.fillText('No history captured in this window.',12,h/2);return null;}
 const xs=live.flatMap(s=>s.points.map(p=>p[0])),ys=live.flatMap(s=>s.points.map(p=>p[1]));
 const x0=Math.min(...xs),x1=Math.max(...xs);let y0=Math.min(...ys),y1=Math.max(...ys);
 if(y1-y0<1){y0-=1;y1+=1;}const pad={l:44,r:10,t:10,b:20};
 const px=t=>pad.l+(t-x0)/((x1-x0)||1)*(w-pad.l-pad.r);
 const py=v=>h-pad.b-(v-y0)/((y1-y0)||1)*(h-pad.t-pad.b);
 g.strokeStyle=getComputedStyle(canvas).getPropertyValue('--line').trim()||'#354342';g.fillStyle=getComputedStyle(canvas).getPropertyValue('--muted').trim()||'#a5b2b2';g.font='12px ui-monospace,monospace';
 for(let i=0;i<=4;i++){const v=y0+(y1-y0)*i/4,y=py(v);g.beginPath();g.moveTo(pad.l,y);g.lineTo(w-pad.r,y);g.stroke();
  g.fillText(v.toFixed(v>50?0:1)+unit,4,y+3);}
 for(const s of live){g.strokeStyle=s.color;g.lineWidth=s.width||2;g.setLineDash(s.dashed?[6,4]:[]);g.beginPath();
  s.points.forEach(([t,v],i)=>{const X=px(t),Y=py(v);i?g.lineTo(X,Y):g.moveTo(X,Y);});g.stroke();
  if(s.dashed){g.setLineDash([]);}}
 return {from:x0,to:x1,low:y0,high:y1};
}

/** Power spectral density chart: several traces over 0..200 Hz, peaks marked, optional shaper overlay. */
export function psdChart(canvas,{freq,traces,peaks=[],band=null,height=260,title=''}){
 const dpr=Math.min(devicePixelRatio,2),w=canvas.clientWidth||600,h=height;
 canvas.width=w*dpr;canvas.height=h*dpr;canvas.style.height=h+'px';
 const g=canvas.getContext('2d');g.scale(dpr,dpr);g.clearRect(0,0,w,h);
 const css=k=>getComputedStyle(canvas).getPropertyValue(k).trim();
 const muted=css('--muted')||'#a5b2b2',line=css('--line')||'#354342',ink=css('--ink')||'#e8f1ef';
 const live=traces.filter(t=>t.values&&t.values.length===freq.length);
 g.font='12px ui-monospace,monospace';
 if(!live.length||!freq.length){g.fillStyle=muted;g.fillText('No spectrum captured.',12,h/2);return null;}
 const pad={l:46,r:12,t:title?24:12,b:22};
 const x0=0,x1=Math.min(200,freq[freq.length-1]||200);
 const top=Math.max(1e-9,...live.flatMap(t=>t.values));
 const px=f=>pad.l+(f-x0)/(x1-x0)*(w-pad.l-pad.r),py=v=>h-pad.b-clamp(v/top,0,1)*(h-pad.t-pad.b);
 if(band){g.fillStyle='rgba(46,230,207,.08)';g.fillRect(px(band[0]),pad.t,px(band[1])-px(band[0]),h-pad.t-pad.b);}
 g.strokeStyle=line;g.fillStyle=muted;
 for(let f=0;f<=x1;f+=25){const X=px(f);g.beginPath();g.moveTo(X,pad.t);g.lineTo(X,h-pad.b);g.stroke();g.fillText(f+'',X-8,h-6);}
 for(let i=0;i<=4;i++){const y=pad.t+(h-pad.t-pad.b)*i/4;g.beginPath();g.moveTo(pad.l,y);g.lineTo(w-pad.r,y);g.stroke();}
 g.fillText('Hz',w-pad.r-16,h-6);g.save();g.translate(12,h/2);g.rotate(-Math.PI/2);g.fillText('power',-18,0);g.restore();
 for(const t of live){g.strokeStyle=t.color;g.lineWidth=t.width||1.5;g.setLineDash(t.dashed?[5,4]:[]);g.beginPath();
  for(let i=0;i<freq.length;i++){if(freq[i]>x1)break;const X=px(freq[i]),Y=py(t.values[i]);i?g.lineTo(X,Y):g.moveTo(X,Y);}g.stroke();g.setLineDash([]);}
 g.fillStyle=ink;
 for(const p of peaks){const X=px(p.hz),Y=py(p.power);g.beginPath();g.arc(X,Y,3.5,0,Math.PI*2);g.fill();g.fillText(p.hz.toFixed(1)+' Hz',X+6,Math.max(pad.t+12,Y-6));}
 if(title){g.fillStyle=ink;g.font='600 12px ui-monospace,monospace';g.fillText(title,pad.l,16);}
 let lx=pad.l;g.font='12px ui-monospace,monospace';for(const t of live){g.fillStyle=t.color;g.fillRect(lx,pad.t+4,14,3);g.fillStyle=muted;g.fillText(t.label,lx+18,pad.t+10);lx+=18+g.measureText(t.label).width+14;}
 return {top,x1};
}
