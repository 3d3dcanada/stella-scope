// WO-11: the addresses the phone app is opened with (mobile/dist/links.js).
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const {parse,pick}=createRequire(import.meta.url)('../mobile/dist/links.js');
const A='a'.repeat(64),B='b'.repeat(64);

assert.deepEqual(parse(`stella://pair/${A}?code=123456`),{kind:'pair',link:A,code:'123456'});
assert.deepEqual(parse(`stella://link/${B}`),{kind:'link',link:B});
// An alert that was tapped: the printer, and the computer when the alert names it.
assert.deepEqual(parse('stella://p/core-one-l'),{kind:'printer',printer:'core-one-l',link:null});
assert.deepEqual(parse(`stella://p/voron-left?link=${A}`),{kind:'printer',printer:'voron-left',link:A});
assert.deepEqual(parse(`stella://p/voron-left/?link=${A.toUpperCase()}`),{kind:'printer',printer:'voron-left',link:A},'case is not significant in a link id');
// Anything that is not an address of ours is not acted on.
for(const bad of ['','hello','https://example.com/p/x','stella://p/','stella://p/a b','stella://p/../x','stella://p/'+'x'.repeat(40),'stella://pair/short?code=1','stella://other/x']){
  assert.equal(parse(bad).kind,'unknown',bad);
}
// A malformed link id is dropped, the printer is kept.
assert.deepEqual(parse('stella://p/stella?link=nothex'),{kind:'printer',printer:'stella',link:null});

// Which computer opens it: the one named; never one that is not saved; else the only or the last used.
const saved=[{id:A,name:'Shop'},{id:B,name:'Home'}];
assert.equal(pick(saved,A,B).name,'Shop');
assert.equal(pick(saved,'c'.repeat(64),A),null,'a computer this phone is not set up with is not added by an address');
assert.equal(pick([saved[1]],null,null).name,'Home','the only one');
assert.equal(pick(saved,null,B).name,'Home','the last used');
assert.equal(pick(saved,null,null),null,'two and no last: ask, do not guess');
assert.equal(pick([],null,null),null);
console.log('mobile link tests passed');
