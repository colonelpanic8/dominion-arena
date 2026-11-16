import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {extract,upstream} from './upstream.mjs';
import {loopback,assertSemantic} from './client-harness.mjs';
test('no external endpoint or URL credential can reach the socket adapter',()=>{
 for(const url of ['wss://127.0.0.1/','ws://dominion.games/','ws://user:secret@127.0.0.1/','http://127.0.0.1/','ws://127.0.0.1.evil.example/'])assert.throws(()=>loopback(url));
 assert.equal(loopback('ws://127.0.0.1:45678/').port,'45678');
});
test('wrong bundle is rejected before any declaration selection',()=>{
 const dir=mkdtempSync(join(tmpdir(),'dominion-bundle-proof-'));try{const path=join(dir,'body.js');writeFileSync(path,'globalThis.secret=true;');assert.throws(()=>extract(path),/SHA-256 mismatch/);}finally{rmSync(dir,{recursive:true});}
});
if(process.env.DOMINION_ORIGINAL_BUNDLE)test('bundle environment has no host filesystem or external network capability',()=>{
 const u=upstream(process.env.DOMINION_ORIGINAL_BUNDLE);assert.equal(u.run('typeof process'),'undefined');assert.equal(u.run('typeof require'),'undefined');assert.throws(()=>u.run('fetch("https://dominion.games")'),/forbidden/);assert.throws(()=>u.run('new WebSocket("wss://dominion.games")'),/forbidden/);u.load('QuestionTypes');u.load('LANGUAGE');assert.equal(u.run('getPhrase(QuestionTypes.ORDER_CARDS.leftText)'),'Bottom');assert.equal(u.run('getPhrase(QuestionTypes.ORDER_CARDS.rightText)'),'Top');
});
if(process.env.DOMINION_ORIGINAL_BUNDLE)test('original table-list play and spectate producers preserve the join boolean meaning',async()=>{
 const {OriginalClient}=await import('./consumer.mjs');const c=new OriginalClient(process.env.DOMINION_ORIGINAL_BUNDLE);
 c.get('tables').play(100);const play=c.sent.at(-1);assert.equal(play.readInt32BE(),2);assert.equal(play.at(-1),1);
 c.get('tables').spectate(100);const spectate=c.sent.at(-1);assert.equal(spectate.readInt32BE(),2);assert.equal(spectate.at(-1),0);
});

test('semantic oracle cannot silently skip unknown or mismatched assertions',()=>{
 const facts={questions:[{text:'Trash a card',targetNames:['Copper'],children:[]}],players:[{index:0,counters:[{name:'COFFERS',kind:5,value:2}],projects:['Cathedral'],reminders:[{name:'Haven',count:1}]}]};
 assertSemantic(facts,{questionTexts:['Trash a card'],targetNames:[['Copper']],playerCounters:{0:{COFFERS:2}},projects:{0:['Cathedral']},reminders:{0:[{name:'Haven',count:1}]}},'fixture');
 assert.throws(()=>assertSemantic(facts,{questionText:['typo']},'fixture'),/unknown/);
 assert.throws(()=>assertSemantic(facts,{questionTexts:['Gain a card']},'fixture'),/prompt text/);
 assert.throws(()=>assertSemantic(facts,{playerCounters:{0:{COFFERS:3}}},'fixture'),/counter/);
});
