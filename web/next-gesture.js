const round=v=>{const n=Number(v.toFixed(3));return Object.is(n,-0)?0:n;};
const disabled=el=>el.disabled||el.getAttribute('aria-disabled')==='true';

export function angleOf(dx,dy){
 if(dx===0&&dy===0)return null;
 return round((Math.atan2(dy,dx)*180/Math.PI+360)%360);
}
export function radialLayout(count,radius,{start=270}={}){
 if(count<=0)return [];
 return Array.from({length:count},(_,index)=>{
  const angle=(start+index*360/count)%360,rad=angle*Math.PI/180;
  return {index,angle:round(angle),x:round(radius*Math.cos(rad)),y:round(radius*Math.sin(rad))};
 });
}
export function radialIndex(dx,dy,count,{deadzone=24,start=270}={}){
 if(count<=0||Math.hypot(dx,dy)<deadzone)return -1;
 const step=360/count,angle=angleOf(dx,dy);
 if(angle===null)return -1;
 return Math.floor((((angle-start)%360+360)%360+step/2)/step)%count;
}
export function jogDirection(dx,dy,{deadzone=28}={}){
 if(Math.hypot(dx,dy)<deadzone)return null;
 const angle=angleOf(dx,dy);
 return angle<45||angle>=315?'x+':angle<135?'y-':angle<225?'x-':'y+';
}
export function createHold({ms=900}={}){
 let startT=null;
 return {
  start(t){startT=t;return 0;},
  progress(t){return startT===null?0:Math.max(0,Math.min(1,(t-startT)/ms));},
  release(t){if(startT===null)return 'none';const done=t-startT>=ms;startT=null;return done?'confirm':'cancel';},
  cancel(){startT=null;},
  get active(){return startT!==null;}
 };
}
export function bindHold(el,{ms=900,onProgress,onConfirm,onCancel}={}){
 const hold=createHold({ms});let frame=0,source=null,pointerId=null,completed=null,suppressUntil=0;
 const progress=value=>{el.style.setProperty('--hold',String(value));onProgress?.(value);};
 const stop=()=>{if(frame)cancelAnimationFrame(frame);frame=0;source=null;pointerId=null;hold.cancel();progress(0);};
 const inside=e=>{const r=el.getBoundingClientRect();return e.clientX>=r.left&&e.clientX<=r.right&&e.clientY>=r.top&&e.clientY<=r.bottom;};
 const tick=()=>{
  if(!source)return;
  if(!el.isConnected||disabled(el)){finish(false);return;}
  const value=hold.progress(performance.now());progress(value);
  if(value>=1){completed={source,pointerId};suppressUntil=performance.now()+50;stop();onConfirm?.();return;}
  frame=requestAnimationFrame(tick);
 };
 const begin=(kind,e)=>{
  if(source||disabled(el))return;
  completed=null;
  source=kind;pointerId=kind==='pointer'?e.pointerId:null;
  hold.start(performance.now());progress(0);frame=requestAnimationFrame(tick);
 };
 const finish=confirm=>{
  if(!source)return;
  const result=confirm?hold.release(performance.now()):'cancel';
  suppressUntil=performance.now()+50;stop();
  if(result==='confirm')onConfirm?.();else onCancel?.();
 };
 const down=e=>{if(e.pointerType==='mouse'&&e.button!==0)return;begin('pointer',e);if(source==="pointer")try{el.setPointerCapture?.(e.pointerId);}catch{}};
 const move=e=>{if(source==='pointer'&&e.pointerId===pointerId&&!inside(e))finish(false);};
 const up=e=>{if(source==='pointer'&&e.pointerId===pointerId)finish(inside(e));else if(completed?.source==='pointer'&&e.pointerId===completed.pointerId){suppressUntil=performance.now()+50;completed=null;}};
 const leave=e=>{if(source==='pointer'&&e.pointerId===pointerId)finish(false);};
 const keydown=e=>{if((e.key==='Enter'||e.key===' ')&&!e.repeat){e.preventDefault();begin('key',e);}};
 const keyup=e=>{if(e.key==='Enter'||e.key===' '){if(source==='key'){e.preventDefault();finish(true);}else if(completed?.source==='key'){e.preventDefault();suppressUntil=performance.now()+50;completed=null;}}};
 const click=e=>{if(performance.now()<=suppressUntil){e.preventDefault();e.stopPropagation();suppressUntil=0;}};
 const blur=()=>{if(source)finish(false);};
 const handlers=[['pointerdown',down],['pointermove',move],['pointerup',up],['pointerleave',leave],['pointercancel',leave],['lostpointercapture',leave],['keydown',keydown],['keyup',keyup],['click',click],['blur',blur]];
 for(const [name,fn] of handlers)el.addEventListener(name,fn);
 return ()=>{stop();for(const [name,fn] of handlers)el.removeEventListener(name,fn);};
}

export function createSwipe({threshold=120,velocity=0.6}={}){
 let origin=null,lastY=0,lastT=0,speed=0;
 const offset=y=>origin===null?0:Math.max(0,y-origin);
 return {
  start(y,t){origin=y;lastY=y;lastT=t;speed=0;},
  move(y,t){if(origin===null)return 0;speed=t>lastT?Math.max(0,(y-lastY)/(t-lastT)):0;lastY=y;lastT=t;return offset(y);},
  end(y,t){if(origin===null)return 'stay';const distance=offset(y);if(y!==lastY&&t>lastT)speed=Math.max(0,(y-lastY)/(t-lastT));origin=null;return distance>=threshold||distance>=24&&speed>=velocity?'dismiss':'stay';}
 };
}
export function bindSwipe(sheet,handle,{threshold,velocity,onDismiss}={}){
 const swipe=createSwipe({threshold,velocity});let pointerId=null;
 const down=e=>{if(e.pointerType==='mouse'&&e.button!==0)return;pointerId=e.pointerId;swipe.start(e.clientY,performance.now());try{handle.setPointerCapture?.(pointerId);}catch{}};
 const move=e=>{if(e.pointerId===pointerId)sheet.style.setProperty('--drag',swipe.move(e.clientY,performance.now())+'px');};
 const end=e=>{if(e.pointerId!==pointerId)return;pointerId=null;const result=swipe.end(e.clientY,performance.now());sheet.style.setProperty('--drag','0px');if(result==='dismiss')onDismiss?.();};
 const cancel=e=>{if(e.pointerId!==pointerId)return;pointerId=null;swipe.end(e.clientY,performance.now());sheet.style.setProperty('--drag','0px');};
 const handlers=[['pointerdown',down],['pointermove',move],['pointerup',end],['pointercancel',cancel]];
 for(const [name,fn] of handlers)handle.addEventListener(name,fn);
 return ()=>{pointerId=null;sheet.style.setProperty('--drag','0px');for(const [name,fn] of handlers)handle.removeEventListener(name,fn);};
}
export function bindPress(el){
 const press=()=>{if(!disabled(el))el.dataset.pressed='1';};
 const clear=()=>{delete el.dataset.pressed;};
 const down=e=>{if(e.pointerType!=='mouse'||e.button===0)press();};
 const keydown=e=>{if(e.key==='Enter'||e.key===' ')press();};
 const keyup=e=>{if(e.key==='Enter'||e.key===' ')clear();};
 const handlers=[['pointerdown',down],['pointerup',clear],['pointerleave',clear],['pointercancel',clear],['blur',clear],['keydown',keydown],['keyup',keyup]];
 for(const [name,fn] of handlers)el.addEventListener(name,fn);
 return ()=>{clear();for(const [name,fn] of handlers)el.removeEventListener(name,fn);};
}
export function createLongPress({ms=500,slop=10}={}){
 let origin=null,startT=0,fired=false;
 return {
  start(x,y,t){origin={x,y};startT=t;fired=false;},
  move(x,y){if(origin&&Math.hypot(x-origin.x,y-origin.y)>slop)origin=null;},
  check(t){if(!origin||fired||t-startT<ms)return false;fired=true;return true;},
  end(){origin=null;}
 };
}
export function bindLongPress(el,{ms,slop,onLongPress}={}){
 const hold=createLongPress({ms,slop});let timer=0,pointerId=null,suppressUntil=0;
 const clear=()=>{if(timer)clearTimeout(timer);timer=0;pointerId=null;hold.end();};
 const down=e=>{
  if(e.pointerType==='mouse'||disabled(el)||pointerId!==null)return;
  pointerId=e.pointerId;hold.start(e.clientX,e.clientY,performance.now());
  timer=setTimeout(()=>{timer=0;if(hold.check(performance.now())){suppressUntil=performance.now()+50;onLongPress?.(e);}},ms??500);
 };
 const move=e=>{if(e.pointerId!==pointerId)return;hold.move(e.clientX,e.clientY);};
 const end=e=>{if(e.pointerId===pointerId){if(suppressUntil) suppressUntil=performance.now()+50;clear();}};
 const click=e=>{if(performance.now()<=suppressUntil){e.preventDefault();e.stopPropagation();suppressUntil=0;}};
 const handlers=[['pointerdown',down],['pointermove',move],['pointerup',end],['pointercancel',end],['pointerleave',end],['click',click]];
 for(const [name,fn] of handlers)el.addEventListener(name,fn);
 return ()=>{clear();for(const [name,fn] of handlers)el.removeEventListener(name,fn);};
}
