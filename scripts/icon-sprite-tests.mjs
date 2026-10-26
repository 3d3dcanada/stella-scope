// F06 (2026-10-19): "Buy a licence" had a blank space where its icon should be: the code named i-shop and the sprite has no such symbol. Every icon the page code names must be in the sprite.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
const web=new URL('../web/',import.meta.url).pathname;
const have=new Set([...fs.readFileSync(web+'next-icons.svg','utf8').matchAll(/id="(i-[a-z0-9-]+)"/g)].map(m=>m[1]));
test('every icon the page code names is a symbol of the sprite',()=>{
 const missing=[];
 for(const f of fs.readdirSync(web).filter(f=>f.endsWith('.js')))for(const m of fs.readFileSync(web+f,'utf8').matchAll(/'(i-[a-z0-9-]+)'/g))if(!have.has(m[1]))missing.push(f+': '+m[1]);
 assert.deepEqual([...new Set(missing)],[],'icons with no symbol show as a blank space');
 assert.ok(have.size>50,'the sprite was read');
});
