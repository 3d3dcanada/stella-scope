// Alerts: which events to tell about, and ntfy (farm board WO-11, 2026-10-12). WhatsApp is set up beside it
// (next.js); this is the part that is the same for every channel. Desktop only: the settings are written
// from the Stella Scope window on the computer, never over the phone link.
import {el,button,toast,openSheet} from './next-ui.js';
import {key} from './next-control.js';

const post=(url,body)=>fetch(url,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)})
 .then(async r=>{const j=await r.json().catch(()=>({}));if(!r.ok)throw Error(j.error||`Stella Scope answered with an error (${r.status}).`);return j;});
const when=ts=>{const d=new Date(ts*1000);return d.toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'});};

/** The whole panel: one switch per kind of event, the ntfy address, a test, and what was said lately. */
export function alertsPanel(){
  const box=el('div','nx-sheetpick nx-alertpanel');
  const load=async()=>{
    box.replaceChildren();
    let d;try{d=await (await fetch('/api/notify',{cache:'no-store'})).json();}catch{box.append(el('p','nx-note','Not available here.'));return;}
    if(!d||!Array.isArray(d.events)){box.append(el('p','nx-note','Set up from the Stella Scope window on the computer.'));return;}
    const msg=el('p','nx-note');msg.setAttribute('role','status');
    // What to tell me about
    const list=el('div','nx-al-events');
    for(const e of d.events){
      // U05: a yes or a no is a latched key (the kit's toggle: lit when on), not a tick box
      const row=el('div','nx-al-row');const k=key({icon:'check',toggle:true,pressed:!!e.on,onPress:async on=>{try{await post('/api/notify/events',{events:{[e.kind]:on}});msg.textContent=on?'On.':'Off.';}catch(x){k.setPressed(!on);msg.textContent=x.message;}}});
      k.el.setAttribute('aria-label',e.label);row.append(k.el,el('span','',e.label));row.addEventListener('click',ev=>{if(!k.el.contains(ev.target))k.el.click();});list.append(row);}
    box.append(el('h4','nx-al-h','Tell me when'),list,el('p','nx-note','These go to every channel that is on: WhatsApp, ntfy, Discord and Slack. Every one also lands in Alerts and pops up in the app.'));
    // ntfy
    const url=el('input','nx-input');url.placeholder='https://ntfy.sh/your-own-long-random-name';url.value=d.ntfy?.url||'';url.setAttribute('aria-label','ntfy topic address');url.autocomplete='off';url.spellcheck=false;
    box.append(el('h4','nx-al-h','ntfy'),
      el('p','nx-note','ntfy sends each alert to the ntfy app on your phone. Use ntfy.sh or a server you run yourself, with a topic name that only you know: anyone who knows the name can read your alerts, so make it long and random. Plain http works for a server on this network; https works for ntfy.sh and your own server. It is off until you give an address. Tapping an alert on the phone opens that printer in the Stella app.'),
      url);
    const row=el('div','nx-md-btns');
    row.append(button({icon:'i-check',label:d.ntfy?.enabled?'Save':'Save and turn on',kind:'primary',size:'row',title:'Save ntfy',does:'Stores the address on this computer only and turns ntfy on.',onPress:async()=>{
        try{await post('/api/notify/ntfy',{url:url.value,enabled:true});toast('ntfy is on.','ok');load();}catch(x){msg.textContent=x.message;}}}));
    if(d.ntfy?.url)row.append(button({icon:'i-close',label:'Turn off',size:'row',title:'Turn ntfy off',does:'Forgets the address. Alerts keep going to WhatsApp if it is on.',onPress:async()=>{
        try{await post('/api/notify/ntfy',{url:''});toast('ntfy is off.','ok');load();}catch(x){msg.textContent=x.message;}}}));
    row.append(button({icon:'i-play',label:'Send a test',size:'row',title:'Send a test',does:'Sends one test alert to every channel that is on.',onPress:async()=>{
        try{const r=await post('/api/notify/test',{});msg.textContent=r.queued?'Sent. It should arrive within a minute.':'Nothing is on: set up WhatsApp or ntfy first.';setTimeout(load,2500);}catch(x){msg.textContent=x.message;}}}));
    box.append(row);
    if(d.ntfy?.last?.reply)box.append(el('p','nx-note',`ntfy, last: ${d.ntfy.last.reply}`));
    // Discord and Slack (2026-10-13): an incoming webhook each, https to their own hosts only.
    box.append(el('h4','nx-al-h','Discord and Slack'),el('p','nx-note','A webhook is a private address that lets Stella Scope post one line in your channel. Make one in the channel\'s settings and paste it here. Each alert names the printer. The address is kept on this computer only and never shown again.'));
    for(const [k,label,ph] of [['discord','Discord','https://discord.com/api/webhooks/...'],['slack','Slack','https://hooks.slack.com/services/...']]){
      const w=d.webhooks?.[k]||{},line=el('div','nx-al-hook');const inp=el('input','nx-input');inp.placeholder=w.set?`${label}: set${w.enabled?' and on':' but off'}. Paste a new address to replace it.`:ph;inp.setAttribute('aria-label',`${label} webhook address`);inp.autocomplete='off';inp.spellcheck=false;
      line.append(el('b','',label),inp,button({icon:'i-check',label:w.set&&!w.enabled?'Turn on':'Save',kind:'primary',size:'row',title:`Save ${label}`,does:`Stores the ${label} webhook and turns it on.`,onPress:async()=>{
          try{await post('/api/notify/webhook',{service:k,url:inp.value,enabled:true});toast(`${label} is on.`,'ok');load();}catch(x){msg.textContent=x.message;}}}));
      if(w.set)line.append(button({icon:'i-close',label:'Turn off',size:'row',title:`Turn ${label} off`,does:'Forgets the address.',onPress:async()=>{try{await post('/api/notify/webhook',{service:k,url:''});toast(`${label} is off.`,'ok');load();}catch(x){msg.textContent=x.message;}}}));
      box.append(line);}
    if(d.webhooks_last?.reply)box.append(el('p','nx-note',`Discord and Slack, last: ${d.webhooks_last.reply}`));
    box.append(msg);
    // What was said lately
    if(d.recent?.length){
      box.append(el('h4','nx-al-h','Lately'));
      const rec=el('div','nx-al-recent');
      for(const r of d.recent.slice(0,8)){
        const line=el('div','nx-al-line');line.append(el('time','',when(r.at)),el('span','',r.text),
          el('em','',r.sent?.length?'sent to '+r.sent.join(' and '):'not sent (off, or already said)'));rec.append(line);}
      box.append(rec);}
  };
  load();return box;
}

/** The farm page's own door to the same panel. */
export function sheetAlerts(){
  openSheet({title:'Alert settings',icon:'i-warn',wide:true,build:body=>{body.append(alertsPanel());}});
}
