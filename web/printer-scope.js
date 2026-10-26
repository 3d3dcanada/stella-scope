// Which printer this page is about (farm board WO-03, 2026-10-12). The page lives at /p/<id>; every
// request it makes to /api/... goes to that printer's own routes /api/p/<id>/..., so the whole NEXT
// app, written for one printer, works for any of them without a line changed. At / (no id) nothing is
// rewritten and the service answers for its first printer, exactly as before. Farm-wide routes
// (/api/printers, pairing, notifications, capabilities) are never rewritten.
(()=>{
 const m=/^\/p\/([a-z0-9-]{1,32})(?:\/|$)/.exec(location.pathname),id=m?m[1]:null;
 window.__stellaPrinter=id;
 if(!id)return;
 const GLOBAL=/^\/api\/(printers|printer-models|pair|notify|capabilities|eye\/switch|licence|alerts|analytics|spools|farm|slicer|version|queue(?!\/start))(\/|\?|$)/;
 const scope=u=>typeof u==='string'&&u.startsWith('/api/')&&!u.startsWith('/api/p/')&&!GLOBAL.test(u)?'/api/p/'+id+u.slice(4):u;
 window.stellaScope=scope;
 const nativeFetch=window.fetch.bind(window);
 window.fetch=(input,init)=>{
  if(typeof input==='string')return nativeFetch(scope(input),init);
  if(input instanceof Request){try{const u=new URL(input.url);if(u.origin===location.origin){const s=scope(u.pathname+u.search);if(s!==u.pathname+u.search)return nativeFetch(new Request(s,input),init);}}catch{}}
  return nativeFetch(input,init);};
 const NativeES=window.EventSource;
 window.EventSource=class extends NativeES{constructor(url,opts){super(scope(String(url)),opts);}};
 // Images and the camera: <img src="/api/..."> is rewritten as it is set.
 const d=Object.getOwnPropertyDescriptor(HTMLImageElement.prototype,'src');
 if(d&&d.set)Object.defineProperty(HTMLImageElement.prototype,'src',{configurable:true,enumerable:d.enumerable,get:d.get,set(v){d.set.call(this,scope(String(v)));}});
 const setAttr=Element.prototype.setAttribute;
 Element.prototype.setAttribute=function(n,v){return setAttr.call(this,n,n==='src'&&this instanceof HTMLImageElement?scope(String(v)):v);};
})();
