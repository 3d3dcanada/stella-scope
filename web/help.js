// Help (finish-2 F07, 2026-10-19): the user guide, inside the app. It shows docs/USER-GUIDE.md (built into the program, so it works with no internet),
// and the two notices About points to. No library: a small reader for the few marks the guide uses (headings, paragraphs, lists, pictures, notes,
// tables, code, bold, links), built as page elements, never as markup text, so a document can never run anything. scripts/help-tests.mjs.
const slug=t=>String(t).toLowerCase().replace(/[`*_]/g,'').replace(/[^a-z0-9]+/g,'-').replace(/^-+|-+$/g,'')||'section';
// Inline marks: `code`, **bold**, [text](address). Everything else is text.
export function inline(s){
 const out=[],re=/`([^`]+)`|\*\*([^*]+)\*\*|\[([^\]]+)\]\(([^)\s]+)\)/g;let at=0,m;
 while((m=re.exec(s))){if(m.index>at)out.push({t:'text',s:s.slice(at,m.index)});
  if(m[1]!==undefined)out.push({t:'code',s:m[1]});else if(m[2]!==undefined)out.push({t:'b',s:m[2]});else out.push({t:'a',s:m[3],href:m[4]});at=m.index+m[0].length;}
 if(at<s.length)out.push({t:'text',s:s.slice(at)});return out;}
const cells=l=>l.trim().replace(/^\||\|$/g,'').split('|').map(c=>c.trim());
// Lines to blocks. Unknown syntax is a paragraph, never an error: a guide is read, not compiled.
export function parse(md){
 const lines=String(md).replace(/\r/g,'').split('\n'),blocks=[],ids=new Map();let i=0;
 const id=t=>{let b=slug(t),n=ids.get(b)||0;ids.set(b,n+1);return n?b+'-'+(n+1):b;};
 while(i<lines.length){const L=lines[i];
  if(!L.trim()){i++;continue;}
  let m;
  if((m=/^(#{1,3})\s+(.*)$/.exec(L))){blocks.push({t:'h',n:m[1].length,text:m[2].trim(),id:id(m[2])});i++;continue;}
  if(/^```/.test(L)){const code=[];i++;while(i<lines.length&&!/^```/.test(lines[i]))code.push(lines[i++]);i++;blocks.push({t:'code',text:code.join('\n')});continue;}
  if(/^-{3,}\s*$/.test(L)){blocks.push({t:'hr'});i++;continue;}
  if((m=/^!\[([^\]]*)\]\(([^)\s]+)\)\s*$/.exec(L))){blocks.push({t:'img',alt:m[1],src:m[2]});i++;continue;}
  if(/^>\s?/.test(L)){const q=[];while(i<lines.length&&/^>\s?/.test(lines[i]))q.push(lines[i++].replace(/^>\s?/,''));blocks.push({t:'quote',inl:inline(q.join(' '))});continue;}
  if(/^\|/.test(L)&&/^\|?\s*:?-{3,}/.test(lines[i+1]||'')){const head=cells(L).map(inline);i+=2;const rows=[];while(i<lines.length&&/^\|/.test(lines[i]))rows.push(cells(lines[i++]).map(inline));blocks.push({t:'table',head,rows});continue;}
  if(/^(\s*[-*]\s+)/.test(L)||/^\s*\d+[.)]\s+/.test(L)){const ol=/^\s*\d+[.)]\s+/.test(L),items=[];
   while(i<lines.length&&(ol?/^\s*\d+[.)]\s+/:/^\s*[-*]\s+/).test(lines[i])){let t=lines[i++].replace(ol?/^\s*\d+[.)]\s+/:/^\s*[-*]\s+/,'');
    while(i<lines.length&&/^\s{2,}\S/.test(lines[i])&&!/^\s*([-*]|\d+[.)])\s+/.test(lines[i]))t+=' '+lines[i++].trim();items.push(inline(t));}
   blocks.push({t:ol?'ol':'ul',items});continue;}
  const p=[L.trim()];i++;while(i<lines.length&&lines[i].trim()&&!/^(#{1,3}\s|```|>|\||!\[|-{3,}\s*$|\s*[-*]\s+|\s*\d+[.)]\s+)/.test(lines[i]))p.push(lines[i++].trim());
  blocks.push({t:'p',inl:inline(p.join(' '))});}
 return blocks;}
// What a link may point at: this app's own pages, a place on the same page, a mail address, or the web (opened in its own tab).
export const linkKind=href=>/^#[\w-]+$/.test(href)?'page':!/\.\./.test(href)&&/^\/(farm|help|p\/[\w-]+)(\/[\w.-]+)*(\?[\w=&-]*)?(#[\w-]*)?$/.test(href)?'app':/^mailto:[\w.+-]+@[\w.-]+(\?[\w=&%.+-]*)?$/.test(href)?'mail':/^https:\/\/[\w.-]+(\/[\w\-./?=&%#~+:@]*)?$/.test(href)?'web':null;
// Pictures: only the guide's own, by name.
export const imageOk=src=>/^\/help\/img\/[\w.-]+\.(jpg|png)$/.test(src);
export const toc=blocks=>blocks.filter(b=>b.t==='h'&&b.n===2).map(b=>({id:b.id,text:b.text}));

// ---- the page (browser only) ----
function nodes(inl,el){
 return inl.map(x=>{if(x.t==='text')return document.createTextNode(x.s);
  if(x.t==='code'){const c=el('code');c.textContent=x.s;return c;}
  if(x.t==='b'){const b=el('strong');b.textContent=x.s;return b;}
  const k=linkKind(x.href);if(!k)return document.createTextNode(x.s);
  const a=el('a');a.textContent=x.s;a.href=x.href;if(k==='web'){a.target='_blank';a.rel='noopener noreferrer';}return a;});}
export function draw(blocks,host){
 const el=(t,c)=>{const e=document.createElement(t);if(c)e.className=c;return e;};host.replaceChildren();
 for(const b of blocks){let e;
  if(b.t==='h'){e=el('h'+b.n);e.id=b.id;e.textContent=b.text;}
  else if(b.t==='p'){e=el('p');e.append(...nodes(b.inl,el));}
  else if(b.t==='quote'){e=el('p','hp-note');e.append(...nodes(b.inl,el));}
  else if(b.t==='ul'||b.t==='ol'){e=el(b.t);for(const it of b.items){const li=el('li');li.append(...nodes(it,el));e.append(li);}}
  else if(b.t==='code'){e=el('pre');const c=el('code');c.textContent=b.text;e.append(c);}
  else if(b.t==='hr')e=el('hr');
  else if(b.t==='img'){if(!imageOk(b.src))continue;e=el('figure','hp-fig');const im=el('img');im.src=b.src;im.alt=b.alt;im.loading='lazy';const cap=el('figcaption');cap.textContent=b.alt;e.append(im,cap);}
  else if(b.t==='table'){const wrap=el('div','hp-table'),t=el('table'),th=el('thead'),tr=el('tr');for(const h of b.head){const c=el('th');c.append(...nodes(h,el));tr.append(c);}th.append(tr);t.append(th);
   const tb=el('tbody');for(const r of b.rows){const row=el('tr');for(const c of r){const td=el('td');td.append(...nodes(c,el));row.append(td);}tb.append(row);}t.append(tb);wrap.append(t);e=wrap;}
  if(e)host.append(e);}}
const DOCS={guide:['/help/guide.md','Stella Scope user guide'],licences:['/help/third-party.md','Licences of what is inside Stella Scope'],models:['/help/printer-models.md','Printer model licences'],'crates-link':['/help/crates-link.md','Crates inside the phone link'],'crates-desktop':['/help/crates-desktop.md','Crates inside the desktop window']};
async function boot(){
 const which=new URLSearchParams(location.search).get('doc')||'guide',[url,title]=DOCS[which]||DOCS.guide,doc=document.getElementById('hp-doc'),side=document.getElementById('hp-toc');
 document.title=title+' · Stella Scope';
 let text;try{const r=await fetch(url,{cache:'no-store'});if(!r.ok)throw Error('missing');text=await r.text();}catch{doc.textContent='This page could not be loaded. Close it and open Help again.';return;}
 const blocks=parse(text);draw(blocks,doc);
 const t=toc(blocks);side.replaceChildren();
 if(which==='guide'&&t.length){const h=document.createElement('b');h.textContent='In this guide';side.append(h);for(const x of t){const a=document.createElement('a');a.href='#'+x.id;a.textContent=x.text;side.append(a);}}
 else{side.hidden=true;doc.parentElement.classList.add('hp-single');}   // no table of contents: the page is one column, not a document in the first column of two
 if(location.hash)document.getElementById(decodeURIComponent(location.hash.slice(1)))?.scrollIntoView();}
if(typeof document!=='undefined'&&document.getElementById('hp-doc'))boot();
