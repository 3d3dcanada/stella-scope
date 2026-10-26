// Finish-2 F07 (2026-10-19): the Help page reads the user guide with a small reader of its own (web/help.js). The reader turns marks into page
// elements and never into markup, links go only where the app allows, and the guide itself uses only what the reader knows.   node --test scripts/help-tests.mjs
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {parse,inline,linkKind,imageOk,toc} from '../web/help.js';

test('headings, paragraphs, lists, notes, tables, code and pictures',()=>{
 const b=parse(['# Title','','Some **bold** and `code` and [a link](/farm).','','## Add a printer','','1. First','2. Second','   goes on','','- one','- two','','> A note','','| a | b |','|---|---|','| 1 | **2** |','','```','x = 1','```','','![The wall](/help/img/wall.jpg)','','---','','## Add a printer','Again.'].join('\n'));
 assert.deepEqual(b.map(x=>x.t),['h','p','h','ol','ul','quote','table','code','img','hr','h','p']);
 assert.deepEqual(b[0],{t:'h',n:1,text:'Title',id:'title'});
 assert.deepEqual(b[1].inl.map(x=>x.t),['text','b','text','code','text','a','text']);
 assert.equal(b[3].items.length,2);assert.equal(b[3].items[1].map(x=>x.s).join(''),'Second goes on','a wrapped line joins its item');
 assert.deepEqual(b[6].head.map(c=>c[0].s),['a','b']);assert.equal(b[6].rows[0][1][0].t,'b');
 assert.equal(b[7].text,'x = 1');
 assert.deepEqual(b[8],{t:'img',alt:'The wall',src:'/help/img/wall.jpg'});
 assert.equal(b[10].id,'add-a-printer-2','two sections of one name get different anchors');
 assert.deepEqual(toc(b).map(x=>x.id),['add-a-printer','add-a-printer-2']);
});

test('what is not a mark is text, and nothing is ever markup',()=>{
 const p=parse('<script>alert(1)</script> and <b>x</b> & *star* [broken(link)');
 assert.deepEqual(p.map(x=>x.t),['p']);
 assert.equal(p[0].inl.map(x=>x.s).join(''),'<script>alert(1)</script> and <b>x</b> & *star* [broken(link)','kept as the words they are: help.js sets textContent, never innerHTML');
 const src=fs.readFileSync(new URL('../web/help.js',import.meta.url),'utf8');
 assert.ok(!/innerHTML|insertAdjacentHTML|outerHTML|document\.write/.test(src),'the reader never builds markup from text');
 assert.deepEqual(inline('a `b` **c** [d](#e)').map(x=>x.t),['text','code','text','b','text','a']);
});

test('a link goes only where the app allows',()=>{
 for(const [h,k] of [['/farm','app'],['/help','app'],['/help?doc=licences','app'],['/help?doc=crates-link','app'],['#install','page'],['mailto:info@3d3d.ca','mail'],['https://3d3d.ca/licence','web'],['https://github.com/Psych0h3ad/3d-print-rig','web']])assert.equal(linkKind(h),k,h);
 for(const h of ['javascript:alert(1)','data:text/html,x','http://example.com','//evil.example','/api/commands','/api/printers','ftp://x','file:///etc/passwd','/help/../api/x','mailto:a b@c'])assert.equal(linkKind(h),null,h);
 assert.ok(imageOk('/help/img/wall.jpg')&&imageOk('/help/img/add-a-printer.png'));
 for(const s of ['/help/img/../x.jpg','https://x/y.jpg','/help/img/x.svg','/api/camera','data:image/png;base64,AAAA','/help/img/a/b.jpg'])assert.ok(!imageOk(s),s);
});

// The guide uses only what the reader knows: every picture is the app's own, every link goes where the app allows, no markup of its own.
test('the guide itself is readable by the reader',()=>{
 const text=fs.readFileSync(new URL('../docs/USER-GUIDE.md',import.meta.url),'utf8'),blocks=parse(text);
 assert.equal(blocks[0].t,'h');assert.equal(blocks[0].n,1,'it opens with its title');
 assert.ok(toc(blocks).length>=12,'a section for every part of the work order');
 const anchors=new Set(blocks.filter(b=>b.t==='h').map(b=>b.id));
 for(const b of blocks){
  if(b.t==='img'){assert.ok(imageOk(b.src),b.src);assert.ok(b.alt.length>15,'a caption that says what the picture shows: '+b.src);}
  const inl=b.inl||(b.items||[]).flat()||[];
  for(const x of [...inl,...(b.rows||[]).flat(2),...(b.head||[]).flat()])if(x?.t==='a'){const k=linkKind(x.href);assert.ok(k,'a link the app would not follow: '+x.href);if(k==='page')assert.ok(anchors.has(x.href.slice(1)),'a link to a section that is not there: '+x.href);}
 }
 assert.ok(!/<[a-z][^>]*>/i.test(text.replace(/`[^`]*`/g,'')),'no markup in the guide: it is read as words');
 assert.ok(!text.includes('—'),'no em dash');
 const pics=blocks.filter(b=>b.t==='img').map(b=>b.src);assert.equal(new Set(pics).size,pics.length,'no picture is shown twice');
});

test('a page with no contents list is one column',()=>{
 const js=fs.readFileSync(new URL('../web/help.js',import.meta.url),'utf8'),css=fs.readFileSync(new URL('../web/help.css',import.meta.url),'utf8');
 assert.match(js,/classList\.add\('hp-single'\)/);assert.match(css,/\.hp-main\.hp-single\{grid-template-columns:minmax\(0,1fr\)\}/);
});

test('Help is reachable from the farm menu and About, and the page is served with the CSP-safe files only',()=>{
 const farm=fs.readFileSync(new URL('../web/farm.js',import.meta.url),'utf8'),about=fs.readFileSync(new URL('../web/next-getmodels.js',import.meta.url),'utf8'),html=fs.readFileSync(new URL('../web/help.html',import.meta.url),'utf8');
 assert.match(farm,/item\('i-help','Help',/);assert.match(farm,/location\.href='\/help'/);
 assert.match(about,/'Help','Opens the user guide\.'/);
 assert.ok(!/<style|style=|onclick=|<script>[^<]/i.test(html),'the CSP allows no inline style or script');
 assert.match(html,/src="\/help\.js"/);
});
