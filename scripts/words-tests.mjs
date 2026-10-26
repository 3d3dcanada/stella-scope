// Words test (finish-2 F07, 2026-10-19). The visible strings of the app are plain: none of the developer words the work order names
// (endpoint, token, websocket, driver, API, ...) and none of Klipper's own words on the pages every brand shares. It reads the string literals
// of web/*.js, drops what is code (addresses, header names, CSS, ids) and checks what is left against the lists below. A word that is right
// in one place (the OctoPrint "API key" a slicer asks for, "Service journal") is allowed there by name, with the reason.
// node scripts/words-tests.mjs
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
const web=path.join(path.dirname(fileURLToPath(import.meta.url)),'..','web');
const NEXT=['farm.js','farm-wall.js','next.js','next-ui.js','next-core.js','next-pages.js','next-farm.js','next-fleet.js','next-queue.js','next-models.js','next-notify.js','next-switch.js','next-stage.js','next-alarm.js','next-control.js','next-getmodels.js','pair.js','model-pick.js','model-view.js','model-run.js'];
// The wall, the queue, the alerts and the sheets every brand sees: no word of Klipper's engine here.
const BRAND_FREE=['farm.js','farm-wall.js','next-queue.js','next-fleet.js','next-notify.js','next-switch.js','pair.js'];

export function literals(src){
 const out=[],re=/'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\.)*)`/g;let m;
 // comments are not on screen
 const code=src.split('\n').map(l=>l.replace(/(^|[^:'"`\\])\/\/.*$/,'$1')).join('\n').replace(/\/\*[\s\S]*?\*\//g,'');
 while((m=re.exec(code))){const t=m[1]??m[2]??m[3];if(t&&/[A-Za-z]{3,}/.test(t))out.push(t);}
 return out;
}
// what is code, not words: addresses, header names, markup, class lists, ids with a dash or an underscore. Everything else is read as words.
const classList=t=>/\s/.test(t)&&t.split(/\s+/).every(w=>/^[a-z0-9_-]+$/.test(w))&&t.split(/\s+/).some(w=>/[-_0-9]/.test(w));
const codeish=t=>/^\//.test(t)||/^X-/.test(t)||/:\/\//.test(t)||/<[a-z!\/]/.test(t)||/^[{[$.#]/.test(t)||/[_]/.test(t)||/^[a-z][a-zA-Z0-9]*$/.test(t)||classList(t)||/^[\w\-.:#\[\]=>*,/ ()%+~]+$/.test(t)&&/[.#\[\]=>*:]/.test(t);
const BANNED=[
 [/\bendpoint(s)?\b/i,'endpoint'],[/\bweb ?socket/i,'websocket'],[/\bJSON\b/,'JSON'],[/\bpayload\b/i,'payload'],[/\bdaemon\b/i,'daemon'],[/\bHTTPS?\b/,'HTTP'],[/\bURL\b/,'URL'],
 [/\btoken\b/i,'token'],[/\bSHA-?256\b/i,'SHA-256'],[/\brecorder\b/i,'recorder'],[/\bHUD\b/,'HUD'],[/controls_enabled|remote_link|stella-link|stella-scope\.json/,'a settings name'],
 [/\bdriver\b/i,'driver'],[/\bAPI\b/,'API'],[/\bservice\b/i,'service'],[/\bretransmit/i,'retransmit'],[/\btoolpath\b/i,'toolpath'],[/\bfetch(es|ed|ing)?\b/i,'fetch'],
];
// where a banned word is right, by file, with the reason
const ALLOW={
 'API':{'next-fleet.js':'the OctoPrint "API key" is the name the slicer gives its own field','next-farm.js':'Moonraker\'s own "API key" field, Klipper only'},
 'driver':{'next-pages.js':'a motor driver is hardware (a TMC2209 chip), on Klipper-only pages','next.js':'a motor driver warning is hardware','next-core.js':'motor driver names in the motor panel'},
 'service':{'next-pages.js':'Service journal is the maintenance log; "service" is also what Klipper calls a camera program','next.js':'Service journal'},
};
const stripAllowed=(word,file,t)=>{
 if(word==='API'&&ALLOW.API[file])return t.replace(/API keys?/g,'');
 if(word==='driver'&&ALLOW.driver[file])return t.replace(/(motor |stepper )drivers?/gi,'').replace(/Driver current scale|DRIVER( WARNINGS)?|MOTOR WARNINGS/g,'');
 if(word==='service'&&ALLOW.service[file]!==undefined)return t.replace(/Service journal|service journal/g,'').replace(/c\.service/g,'');
 return t;};
// strings that are compared with, never shown: what the service reports as the reason controls are off; the old chip label next-core.js plainState() replaces
const IGNORE=new Set(['controls_enabled is false','SERVICE OFF','Stella Scope service is not running: this page cannot reach it, so nothing new arrives and no command can go. The page tries again by itself and picks up at once when it answers.']);
const failures=[];
for(const f of NEXT){
 const src=fs.readFileSync(path.join(web,f),'utf8');
 for(const raw of literals(src)){
  if(codeish(raw))continue;
  const t=raw.replace(/\$\{[^}]*\}/g,'');   // what is inside ${...} is code
  if(IGNORE.has(t))continue;
  for(const [re,name] of BANNED){const u=stripAllowed(name,f,t);if(re.test(u))failures.push(`${f}: "${t.slice(0,110)}" says ${name}`);}
  if(BRAND_FREE.includes(f)&&/\b(klipper|moonraker|klippy|mcu|firmware_restart|printer\.cfg)\b/i.test(t)&&!/gcode|\.gcode/.test(t))failures.push(`${f}: "${t.slice(0,110)}" has a Klipper word on a page every brand shares`);
 }
}
assert.deepEqual(failures,[],'\n'+failures.join('\n'));
console.log(`words: ${NEXT.length} files, no developer word on screen`);
