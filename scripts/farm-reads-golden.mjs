// Farm board WO-02/03: one fixed set of read routes, normalised (token, clocks and counters removed),
// so the status shapes before and after a change can be diffed.  node scripts/farm-reads-golden.mjs URL [PREFIX]
const [svc='http://127.0.0.1:7421',prefix='']=process.argv.slice(2);
const host=new URL(svc).host;
const get=async p=>{const r=await fetch(svc+prefix+p,{headers:{host}});const t=await r.text();try{return {code:r.status,body:JSON.parse(t)};}catch{return {code:r.status,body:t.slice(0,80)};}};
const strip=(v,k='')=>{
 if(Array.isArray(v))return v.map(x=>strip(x,k));
 if(v&&typeof v==='object')return Object.fromEntries(Object.entries(v).sort(([a],[b])=>a<b?-1:1).map(([a,b])=>[a,strip(b,a)]));
 if(['token','ts','server_time','last_success','last_attempt','age_seconds','sample_ts','at','captured_at','since_ts','cursor','first_ts','last_ts','time_in_queue','modified','time_added','updates_per_s_1m','query_ms_last','query_ms_mean','last_write_ms','mean_write_ms','frame_age_s','keepalive_ms','eventtime','uptime','audit_id','samples','events','errors','poll_overruns'].includes(k))return typeof v==='number'?0:v;
 return v;};
const routes=['/api/controls','/api/macros','/api/files','/api/console/help','/api/tick','/api/meta','/api/presets','/api/print/queue','/api/print/objects','/api/capabilities'];
const out={};for(const r of routes){const g=await get(r);out[r]={code:g.code,body:strip(g.body)};}
console.log(JSON.stringify(out,null,1));
