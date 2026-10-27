// Feedback (2026-10-27: "We're supposed to have feedback in all of our apps anyway, so people can give us feedback"). What the person types
// goes to 3D3D only when they press Send (feedback.rs): the words, the app's version and system, and, if the box stays ticked, how many
// printers of which kind. The page says exactly that before anything is sent, and shows the reference 3d3d.ca gives back.
import {el,button,busy,openSheet,section} from './next-ui.js';

const MIN=10,MAX=3000;
const token=()=>fetch('/api/printers',{cache:'no-store'}).then(r=>r.json()).then(d=>{if(!(d.token&&d.writable))throw Error('Feedback is sent from the Stella Scope window on the computer.');return d.token;});

export function sheetFeedback(){openSheet({title:'Feedback',icon:'i-chat',eyebrow:'TELL 3D3D WHAT TO FIX OR ADD',build:body=>{
 body.classList.add('nx-feedback');
 body.append(el('p','nx-note','What works, what does not, what you would add. It is read by the people who build Stella Scope. Leave an email if you would like an answer.'));
 const msg=el('textarea','nx-input nx-fb-text');msg.rows=7;msg.maxLength=MAX;msg.placeholder='What happened, or what would make Stella Scope better for your printers…';msg.setAttribute('aria-label','Your feedback');
 const count=el('small','nx-fb-count',`0 / ${MAX}`);
 const name=el('input','nx-input');name.type='text';name.maxLength=80;name.placeholder='Name (optional)';name.setAttribute('aria-label','Name');name.autocomplete='name';
 const email=el('input','nx-input');email.type='email';email.maxLength=254;email.placeholder='Email, for an answer (optional)';email.setAttribute('aria-label','Email');email.autocomplete='email';
 const setupBox=el('label','nx-fb-check'),setup=el('input');setup.type='checkbox';setup.checked=true;setupBox.append(setup,el('span','','Include how many printers of each kind I run (no names, addresses or files)'));
 const sends=el('p','nx-note nx-fb-sends','Sent with it: the app version, this computer\'s system (Linux, Windows…) and, if ticked above, the count of printers by kind. Nothing else, and only when you press Send.');
 const said=el('p','nx-note nx-fb-said');said.setAttribute('role','status');
 const ready=()=>{const n=msg.value.trim().length;count.textContent=`${msg.value.length} / ${MAX}`;send.disabled=n<MIN;};
 const send=button({icon:'i-chat',label:'Send to 3D3D',kind:'primary',size:'row',title:'Send to 3D3D',does:'Sends what you wrote to 3d3d.ca. Nothing is sent before you press this.',onPress:async b=>{
  if(b.dataset.busy!==undefined)return;said.textContent='';said.dataset.tone='';busy(send,true);
  try{const t=await token();const r=await fetch('/api/feedback',{method:'POST',headers:{'Content-Type':'application/json','X-Stella-Control-Token':t},body:JSON.stringify({message:msg.value,name:name.value,email:email.value,include_setup:setup.checked})});
   const d=await r.json().catch(()=>({}));if(!r.ok)throw Error(d.error||`Stella Scope answered with an error (${r.status}).`);
   said.dataset.tone='ok';said.textContent=`Sent. Thank you. Your reference is ${d.ref}${email.value.trim()?': an answer comes to '+email.value.trim()+'.':'.'}`;msg.value='';ready();}
  catch(e){said.dataset.tone='alarm';said.textContent=String(e.message||e);}
  finally{busy(send,false);ready();}}});
 msg.addEventListener('input',ready);
 const who=el('div','nx-grid2');who.append(name,email);
 body.append(section('Your message',msg,count),section('Who you are (optional)',who),setupBox,sends,send,said);
 ready();setTimeout(()=>msg.focus(),60);
}});}
