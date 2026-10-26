// What choosing a printer's 3D model needs, as plain functions the page and the tests share
// (polish WO-05, 2026-10-18). No DOM here.

// The catalogue keeps one label in Japanese (the INDX "reference nozzle"); the page speaks English (the owner, plain words).
const JA={'原本ノズル参照':'original nozzle, reference'};
const CJK=/[⺀-鿿＀-￯]+/g;
export const plainLabel=s=>{let t=String(s??'');for(const [k,v] of Object.entries(JA))t=t.split(k).join(v);return t.replace(CJK,'').replace(/\s*[·/]\s*(?=[·/]|$)/g,'').replace(/\s{2,}/g,' ').trim();};

// Every word typed must be somewhere in the text, in any order: "trident 300", "ender 3", "v2.4 voron".
export const matches=(query,text)=>{const hay=String(text||'').toLowerCase();return String(query||'').toLowerCase().split(/\s+/).filter(Boolean).every(w=>hay.includes(w));};

/** The files a choice needs: the machine (when its file is not here) and every chosen option not here yet.
 *  `tick` is true when any file still to get is not under an open licence (the person must say they read it first).
 *  `picked` is the cad_options array: [{option, tools?, active?}]. */
export function needs(machine,options,picked){
  const byId=new Map((options||[]).map(o=>[o.id,o]));
  // an unlicensed file says so in its badge, not in a second phrase
  const item=(kind,x)=>({kind,id:x.id,label:plainLabel(x.label),bytes:x.bytes||0,installed:!!x.installed,bundled:!!x.bundled,cls:x.licence?.class||'unlicensed',text:x.licence?.class==='unlicensed'?'':x.licence?.text||'',file:kind==='option'||x.has_file!==false});
  const items=[];
  if(machine)items.push(item('machine',machine));
  for(const c of picked||[]){const o=byId.get(c.option);if(o)items.push(item('option',o));}
  const todo=items.filter(i=>!i.installed&&i.file);
  return {items,todo,bytes:todo.reduce((a,i)=>a+i.bytes,0),tick:todo.some(i=>!i.bundled),noFile:items.find(i=>!i.file)||null};
}

/** The cad_options array for what is chosen: one entry per ticked option; a toolchanger carries its tools and the active dock. */
export function selection(rows){return rows.filter(r=>r.on).map(r=>r.value());}

/** A toolchanger with `n` docks: the tool ids it holds, keeping what was chosen for the docks that stay. */
export function dockTools(tools,n,keep=[]){return Array.from({length:n},(_,i)=>keep[i]&&tools.some(t=>t.id===keep[i])?keep[i]:tools[i%tools.length].id);}

/** Which options the chooser offers: only those Stella Scope can draw on this machine; toolheads are drawn as the tools of a bank. */
export const drawable=options=>(options||[]).filter(o=>o.drawable);
