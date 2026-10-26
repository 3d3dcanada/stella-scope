// One-time guarded integration of the reviewed 0.3 sources with the 0.2 API.
import {readFileSync,writeFileSync,existsSync,mkdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
function blob(text){return createHash('sha1').update('blob '+Buffer.byteLength(text)+'\0').update(text).digest('hex');}
const source=readFileSync('src/api.rs','utf8');
if(blob(source)!=='580bf83d2693a7b394a07c574c2ea15df36ef88c')throw Error('api.rs differs from the inspected 0.2 baseline; reconcile before applying this patch.');
function once(text,old,replacement){if(text.split(old).length!==2)throw Error('Expected exactly one patch anchor: '+old);return text.replace(old,replacement);}
let api=once(source,'pub mod investigation;','pub mod investigation;\npub mod workspace;');
api=once(api,'        "/" | "/index.html" =>','        "/classic" =>');
const routes=`        "/" | "/index.html" | "/workspace" | "/workspace.html" | "/workspace/printer" | "/workspace/motion" | "/workspace/motors" | "/workspace/thermal" | "/workspace/leveling" | "/workspace/events" | "/workspace/recording" => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/workspace.html"))),
        "/workspace.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace.js"))),
        "/workspace-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace-core.js"))),
        "/workspace-scene.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/workspace-scene.js"))),
        "/workspace.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/workspace.css"))),
`;
api=once(api,'    match path {\n        "/classic"', '    match path {\n'+routes+'        "/classic"');
api=once(api,'    if ["/api/investigation","/api/export"].contains(&path)', '    if path=="/api/workspace" {return workspace::handle(method,target,db,cfg,status);}\n    if ["/api/investigation","/api/export"].contains(&path)');
writeFileSync('src/api.rs',api);
let cargo=readFileSync('Cargo.toml','utf8');cargo=once(cargo,'version = "0.2.0"','version = "0.3.0"');writeFileSync('Cargo.toml',cargo);
let lock=readFileSync('Cargo.lock','utf8');lock=once(lock,'name = "stella-scope"\nversion = "0.2.0"','name = "stella-scope"\nversion = "0.3.0"');writeFileSync('Cargo.lock',lock);
mkdirSync('docs',{recursive:true});if(existsSync('docs/README-0.2.md'))throw Error('Historical README already exists; do not overwrite it.');
const oldReadme=readFileSync('README.md','utf8');writeFileSync('docs/README-0.2.md',oldReadme);
writeFileSync('README.md',readFileSync('docs/WORKSPACE-0.3.md','utf8')+'\n\n## Recorder configuration and legacy investigation reference\n\nThe complete existing configuration/API guide is preserved in [the 0.2 guide](docs/README-0.2.md). All configuration keys retain their meanings and defaults. Its version-specific benchmark/test totals describe 0.2, not 0.3. This release changes no recording schema and adds no runtime dependency.\n');
console.log('Applied guarded API routes, version and README; original collector, database schema and old frontends preserved.');
