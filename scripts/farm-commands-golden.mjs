// Farm board WO-02: drives one fixed sequence of commands through a running service against the fake
// Moonraker (tools/fake-printers.rs) and prints the exact bodies the printer received, so the same run
// before and after the driver seam can be diffed byte for byte.
//   node scripts/farm-commands-golden.mjs http://127.0.0.1:7421 http://127.0.0.1:7126 > golden.json
const [svc='http://127.0.0.1:7421',fake='http://127.0.0.1:7126']=process.argv.slice(2);
const host=new URL(svc).host;
const j=async(url,opt={})=>{const r=await fetch(url,opt);const t=await r.text();try{return {code:r.status,body:JSON.parse(t)};}catch{return {code:r.status,body:t};}};
const set=b=>fetch(fake+'/__set',{method:'POST',body:JSON.stringify(b)});
const c=await j(svc+'/api/controls',{headers:{host}});const token=c.body.token;
const send=async(body)=>(await j(svc+'/api/commands',{method:'POST',headers:{host,origin:'http://'+host,'content-type':'application/json','x-stella-control-token':token},body:JSON.stringify({confirm:true,...body})})).code;
await fetch(fake+'/__reset',{method:'POST',body:'{}'});
await set({klippy:'ready',print_state:'standby',homed:'xyz',qgl_secs:0.4,load_secs:0.3,nozzle_target:0,bed_target:0});
const steps=[
 {id:'set_heater',target:'extruder',value:210},{id:'set_heater',target:'heater_bed',value:60},{id:'set_fan',value:0.5},
 {id:'speed_factor',value:120},{id:'extrude_factor',value:95},{id:'home'},{id:'qgl'},{id:'mesh'},{id:'jog',axis:'X',value:10},
 {id:'motors_off'},{id:'z_offset',value:0.02},{id:'extrude',value:5},{id:'filament_load',value:250},{id:'filament_unload',value:240},
 {id:'console',script:'M115'},{id:'macro',target:'LOAD_FILAMENT',params:{TEMP:'250'}},{id:'print_start',filename:'cube_20mm_PETG.gcode'},
 {id:'pause'},{id:'resume'},{id:'cancel'},
];
const codes=[];for(const s of steps){codes.push([s.id,await send(s)]);await new Promise(r=>setTimeout(r,120));}
const log=(await j(fake+'/__log')).body.filter(e=>e.method==='POST');
console.log(JSON.stringify({codes,posts:log.map(e=>({path:e.path.replace(/\?.*/,'?'+(e.path.split('?')[1]||'')),body:e.body}))},null,1));
