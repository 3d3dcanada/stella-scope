// The look chosen on this device (2026-10-25: "be able to change the colour palette and how everything looks on the fly"): the colour,
// the finish and the dial style, put on <html> before the page draws so it never flashes another look. The Look panel changes them live.
(()=>{let p={};try{p=JSON.parse(localStorage.getItem('stella-paint')||'{}')||{};}catch{}
 const d=document.documentElement;d.dataset.accent=p.accent||'silver';d.dataset.finish=p.finish||'graphite';d.dataset.dials=p.dials||'led';d.dataset.chips=p.chips||'on';})();
