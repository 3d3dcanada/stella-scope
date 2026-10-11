// Assets are embedded in the Rust executable. Check actual served bytes and parse as ESM:
// Node 22's automatic .js detection can return success for an invalid imported module.
import fs from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const base=process.env.HUD_URL;
assert.match(base||'',/^http:\/\/127\.0\.0\.1:\d+$/,'HUD_URL must be a local replay server');
const results=[];
for(const name of ['hud.js','hud.css','hud-scene.js','hud-graphs.js','hud-gcode.js','service.js','service-dials.js']){
 const source=await fs.readFile(new URL('../web/'+name,import.meta.url));
 const response=await fetch(base+'/'+name);assert.equal(response.status,200,name);
 const served=Buffer.from(await response.arrayBuffer());assert.ok(source.equals(served),name+' differs from the compiled binary; touch src/api.rs and rebuild');
 if(name.endsWith('.js')){
  const parsed=spawnSync(process.execPath,['--input-type=module','--check'],{input:served,encoding:'utf8'});
  assert.equal(parsed.status,0,name+' syntax: '+parsed.stderr);
 }
 results.push({name,sha256:createHash('sha256').update(served).digest('hex'),matches:true});
 console.log('PASS compiled asset '+name);
}
if(process.env.VALIDATION_DIR)await fs.writeFile(process.env.VALIDATION_DIR+'/asset-freshness.json',JSON.stringify(results,null,2)+'\n');
