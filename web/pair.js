// Stella Scope phone pairing (src/lan.rs): trade the desktop's six-digit code for this device's key.
const code=document.getElementById('code'),go=document.getElementById('go'),msg=document.getElementById('msg');
async function pair(){const v=code.value.replace(/\D/g,'');if(v.length!==6){msg.textContent='Type the six digits shown on the desktop.';return;}
 go.disabled=true;msg.textContent='Pairing…';
 try{const r=await fetch('/api/pair',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({code:v})});const d=await r.json().catch(()=>({}));
  if(!r.ok)throw Error(d.error||'Pairing failed.');msg.textContent='Paired. Opening Stella Scope…';location.replace('/');}
 catch(e){msg.textContent=e.message;go.disabled=false;}}
go.addEventListener('click',pair);code.addEventListener('keydown',e=>{if(e.key==='Enter')pair();});code.focus();
// Arriving from the Stella app is a cross-site navigation, and the device cookie is SameSite=Strict,
// so a paired phone lands here first. A same-origin request does carry the cookie: if this device is
// already paired, go straight in (a navigation this page starts is same-site).
fetch('/api/tick',{credentials:'same-origin'}).then(r=>{if(r.ok)location.replace('/');}).catch(()=>{});
// The Stella app arrives with the code from the desktop's QR in the address (#code=123456): pair at once.
{const m=/[#&]code=(\d{6})/.exec(location.hash);if(m){code.value=m[1];history.replaceState(null,'',location.pathname);pair();}}
