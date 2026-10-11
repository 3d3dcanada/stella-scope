import fs from 'node:fs';import {execFileSync} from 'node:child_process';
const repository='VoronDesign/Voron-2',commit='a192410e27ea345644ae5c4b29b4c9c40cbe1a73';
const tree=JSON.parse(execFileSync('gh',['api',`repos/${repository}/git/trees/${commit}?recursive=1`],{encoding:'utf8',maxBuffer:5000000}));
if(tree.truncated)throw Error('Official source tree truncated; cannot claim complete catalogue.');
const files=tree.tree.filter(x=>x.type==='blob'&&x.path.startsWith('STLs/')&&/\.stl$/i.test(x.path)).map(x=>({path:x.path,blob_sha:x.sha,bytes:x.size,commit,url:`https://github.com/${repository}/blob/${commit}/${x.path.split('/').map(encodeURIComponent).join('/')}`}));
if(files.length<100)throw Error('Official STL catalogue unexpectedly small; verify source.');
fs.writeFileSync('web/service-catalog.json',JSON.stringify({repository,commit,tree_complete:true,scope:'Official source file catalogue, not an as-built bill of materials. Variants and alternatives are not all installed.',files},null,2)+'\n');console.log(`Pinned ${files.length} official STL source references at ${commit}. No third-party geometry embedded.`);
