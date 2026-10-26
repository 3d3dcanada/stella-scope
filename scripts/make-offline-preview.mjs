import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import path from 'node:path';
const root=path.resolve(import.meta.dirname,'..'),web=path.join(root,'web');
const core=readFileSync(path.join(web,'workspace-core.js'),'utf8');
const coreNames=[...core.matchAll(/export (?:const|function|class) (\w+)/g)].map(m=>m[1]);
const scene=readFileSync(path.join(web,'workspace-scene.js'),'utf8').replace(/import \{([^}]+)\} from '[^']+';/,'const {$1}=Core;').replace(/export /g,'');
const app=readFileSync(path.join(web,'workspace.js'),'utf8').replace(/import \{([^}]+)\} from '\.\/workspace-core.js';/,'const {$1}=Core;').replace(/import \{([^}]+)\} from '\.\/workspace-scene.js';/,'const {$1}=Scene;');
const fixture=readFileSync(path.join(root,'scripts/browser-workspace.mjs'),'utf8');
const fixtureCode=fixture.slice(fixture.indexOf('const first='),fixture.indexOf('const server=http'));
const prelude=`${fixtureCode}
const originalReplace=history.replaceState.bind(history),originalPush=history.pushState.bind(history),fileBase=location.href.split('#')[0];
for(const key of ['replaceState','pushState']){history[key]=(a,b,url)=>{const u=new URL(url,'http://preview'),fn=key==='replaceState'?originalReplace:originalPush;fn(a,b,fileBase+u.hash);};}
window.fetch=async (input)=>{const u=new URL(String(input),'http://preview');if(u.pathname!=='/api/workspace')throw Error('Offline preview supports only synthetic workspace snapshots');return new Response(JSON.stringify(snapshot(u.searchParams)),{headers:{'content-type':'application/json'}});};
`;
const code=prelude+`\nconst Core=(()=>{${core.replace(/export /g,'')}\nreturn {${coreNames.join(',')}};})();\nconst Scene=(()=>{${scene}\nreturn {layout,PrinterScene};})();\n(()=>{${app}\n})();`;
let html=readFileSync(path.join(web,'workspace.html'),'utf8').replace('<link rel="stylesheet" href="/workspace.css">','<style>'+readFileSync(path.join(web,'workspace.css'),'utf8')+'</style>').replace('<script type="module" src="/workspace.js"></script>','');
html=html.replace('</body>','<script>'+code.replace(/<\/script/gi,'<\\/script')+'</script></body>');
mkdirSync(path.join(root,'preview'),{recursive:true});writeFileSync(path.join(root,'preview/Stella_Scope_0.3_Interactive_Preview.html'),html);
