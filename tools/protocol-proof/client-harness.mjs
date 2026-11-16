#!/usr/bin/env node
import {readFileSync,mkdirSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';
import {OriginalClient} from './consumer.mjs';
import {BODY_SHA256} from './upstream.mjs';

export const SIMULATED=['Angular scope event delivery, safeDigest bridge and dependency injection (original registered service callbacks); head ObjectValues=Object.values alias','DOM elements and canvas drawing/measurements without raster output','image load completion without reading/downloading image bytes','in-memory local/session storage; no host browser storage','deterministic short timer/animation-frame clock; no audio','game-area containers and remount on GAME_MODEL_CREATED; kingdom last-seen/sound service shell; unopened news/profile/stats tab services'];
export function loopback(endpoint) {
 const url=new URL(endpoint);
 if(url.protocol!=='ws:'||!['127.0.0.1','[::1]','localhost'].includes(url.hostname)||url.username||url.password)throw Error('only credential-free loopback ws endpoints are permitted');
 return url;
}
function writeReport(output,report) {
 mkdirSync(output,{recursive:true});writeFileSync(resolve(output,'consumer-report.json'),JSON.stringify(report,null,2)+'\n');
}
export function assertSemantic(facts,expected,name) {
 const known=new Set(['questionTexts','targetNames','playerCounters','projects','reminders']);
 for(const key of Object.keys(expected))assert(known.has(key),`${name}: unknown expectedSemantic field ${key}`);
 const leaves=questions=>questions.flatMap(q=>q.children.length?leaves(q.children):[q]);const questions=leaves(facts.questions);
 if(expected.questionTexts)assert.deepEqual(questions.map(q=>q.text),expected.questionTexts,`${name}: original prompt text`);
 if(expected.targetNames)assert.deepEqual(questions.map(q=>q.targetNames),expected.targetNames,`${name}: original targets`);
 for(const [seat,counters]of Object.entries(expected.playerCounters??{})){const player=facts.players.find(p=>p.index===Number(seat));assert(player,`${name}: counter player missing`);for(const[kind,value]of Object.entries(counters)){const counter=player.counters.find(c=>c.name===kind||String(c.kind)===kind);assert(counter,`${name}: counter ${kind} missing`);assert.equal(counter.value,value,`${name}: counter ${seat}/${kind}`);}}
 for(const [seat,projects]of Object.entries(expected.projects??{})){const player=facts.players.find(p=>p.index===Number(seat));assert(player,`${name}: project player missing`);for(const project of projects)assert(player.projects.includes(project),`${name}: project ${project} missing`);}
 for(const [seat,reminders]of Object.entries(expected.reminders??{})){const player=facts.players.find(p=>p.index===Number(seat));assert(player,`${name}: reminder player missing`);for(const reminder of reminders){const want=typeof reminder==='string'?{name:reminder}:reminder;const found=player.reminders.find(r=>r.name===want.name);assert(found,`${name}: reminder ${want.name} missing`);for(const[key,value]of Object.entries(want))assert.deepEqual(found[key],value,`${name}: reminder ${key}`);}}
}
async function verifyScores(client,endpoint,report) {
 const facts=client.facts();assert(facts.score?.length===facts.players.length,'original finished model missing player scores');
 const token=client.get('sessionService').getSession()?.sessionId;assert(token,'local session missing for authoritative score check');
 const base=loopback(endpoint);base.protocol='http:';base.pathname='/';
 const res=await fetch(new URL(`/api/games/g${facts.gameId}/view`,base),{headers:{Origin:base.origin,Authorization:`Bearer ${token}`},redirect:'error'});
 assert.equal(res.status,200,'authoritative finished seat view unavailable');
 const actual=(await res.json()).view.game.result?.players;assert(actual,'native finished result missing');
 for(const score of facts.score){const seat=facts.players.find(p=>p.id===score.playerId)?.index;const expected=actual.find(p=>p.index===seat);assert(expected,'result player identity mismatch');assert.equal(score.totalPoints,expected.vp,'original total VP');assert.equal(score.usedTurns,expected.turns,'original used turns');assert.equal(score.rank,expected.rank,'original rank');assert.equal(score.resigned,expected.resigned??false,'original resignation');}
 report.scoreComparisons??=[];report.scoreComparisons.push({gameId:facts.gameId,original:facts.score,authoritative:actual,matched:true});
}
class NativeParticipant {
 constructor(endpoint,account,report){this.base=new URL(endpoint);this.base.protocol='http:';this.base.pathname='/';this.account=account;this.report=report;this.cookie='';}
 async request(path,body){const res=await fetch(new URL(path,this.base),{method:body===undefined?'GET':'POST',headers:{Origin:this.base.origin,...this.cookie?{Cookie:this.cookie}:{},...body===undefined?{}:{'Content-Type':'application/json'}},...body===undefined?{}:{body:JSON.stringify(body)},redirect:'error'});if(!res.ok)throw Error(`native ${path.split('?')[0]} status ${res.status}`);const cookie=res.headers.get('set-cookie');if(cookie)this.cookie=cookie.split(';')[0];return res.status===204?null:res.json();}
 async login(){await this.request('/api/auth/login',{name:this.account.username,password:this.account.password});}
 async join(table){await this.request(`/api/tables/t${table}/join`,{});}
 async ready(table){await this.request(`/api/tables/t${table}/ready`,{ready:true});}
 async step(){const held=await this.request('/api/me/games');const game=held.find(g=>g.running);if(!game)return false;const path=`/api/games/${game.gameId}`;const state=await this.request(`${path}/view?seat=${game.seat}`);const d=state.view.decision;if(!d)return false;let answer={decision:d.id};if(d.nameAny)answer.name='Copper';else {let options=d.options??[];if(d.kind==='turn'){const buys=['Province','Gold','Silver','Duchy','Estate'];let choice=options.find(o=>o.action==='play');if(!choice)for(const name of buys){choice=options.find(o=>o.action==='buy'&&o.card===name);if(choice)break;}choice??=options.find(o=>/end/i.test(o.action))??options[0];answer.indices=choice?[choice.index]:[];}else answer.indices=options.slice(0,d.min??1).map(o=>o.index);}
 await this.request(`${path}/answer?seat=${game.seat}`,answer);this.report.nativeAnswers=(this.report.nativeAnswers??0)+1;return true;}
}
export async function main(argv=process.argv.slice(2)) {
 const opts={};for(let i=0;i<argv.length;i+=2){if(!argv[i].startsWith('--')||argv[i+1]===undefined)throw Error('arguments require --name value');opts[argv[i].slice(2)]=argv[i+1];}
 if(!opts.bundle||!opts.output||!opts.fixture)throw Error('usage: client-harness.mjs --bundle BODY --scenario fixtures|lifecycle-and-rules --fixture JSON --output DIR [--endpoint ws://127.0.0.1:PORT/]');
 const fixture=JSON.parse(readFileSync(opts.fixture,'utf8'));
 const report={bundleSha256:BODY_SHA256,level:'original-service-model-answer',graphicalAcceptance:false,simulated:SIMULATED,scenario:opts.scenario??'fixtures',facts:[],commands:[],cases:[],passed:false};
 const commandFact=(bytes,client)=>{const opcode=bytes.readInt32BE();const secret=[1,13,24,45,46].includes(opcode);report.commands.push({client,opcode,...secret?{redacted:true}:{hex:bytes.toString('hex')}});};
 try {
  if(report.scenario==='fixtures') {
   const cases=Array.isArray(fixture)&&fixture[0]?.messages?fixture:[{name:'fixtures',messages:Array.isArray(fixture)?fixture:fixture.messages,answer:fixture.answer,expected:fixture.expected}];
   for(const scenario of cases){
    report.activeCase=scenario.name;
    const c=new OriginalClient(opts.bundle,{send:b=>commandFact(b,0)});
    let facts;
    for(const f of scenario.messages){
     facts=c.receive(Buffer.from(f.hex,'hex'));report.facts.push({name:scenario.name,message:f.name,facts});
     if(f.expect)for(const[key,value]of Object.entries(f.expect))assert.deepEqual(JSON.parse(JSON.stringify(facts[key])),value,`${scenario.name}:${key}`);
     if(f.answer)c.answer(f.answer);
    }
    if(scenario.expected?.values||scenario.expected?.answer)assert(scenario.answer,`${scenario.name}: expected answer has no original action`);
    if(scenario.expectedSemantic)assertSemantic(facts,scenario.expectedSemantic,scenario.name);
    if(scenario.expected)for(const[key,value]of Object.entries(scenario.expected)){
     if(key==='answer'||key==='values')continue;
     assert(key in facts,`${scenario.name}: unknown expected fact ${key}`);assert.deepEqual(JSON.parse(JSON.stringify(facts[key])),value,`${scenario.name}:${key}`);
    }
    if(scenario.answer){
     c.answer(scenario.answer);
     const answer=c.sent.findLast(b=>b.readInt32BE()===37);assert(answer,`${scenario.name}: original answer not sent`);
     if(scenario.expected?.answer||scenario.expected?.values){const values=[];for(let i=0;i<answer.readInt32BE(8);i++)values.push(answer.readInt32BE(12+i*4));assert.deepEqual(values,scenario.expected.answer??scenario.expected.values,`${scenario.name}: original answer`);}
     report.facts.push({name:scenario.name,afterAnswer:c.facts()});
    }
    report.cases.push({name:scenario.name,semanticAssertions:Object.keys(scenario.expectedSemantic??{}),originalAnswerChecked:Boolean(scenario.expected?.answer||scenario.expected?.values),passed:true});
    report.originalServices=[...c.services.keys()].filter(n=>c.u.registrations.has(n));report.originalDeclarations=[...c.u.loaded];
   }
   delete report.activeCase;report.passed=true;
  } else {
   const endpoint=loopback(opts.endpoint).href;
   if(fixture.bootstrap){const {bootstrap}=await import('./bootstrap.mjs');await bootstrap(opts.bundle,endpoint,fixture.accounts[0],{origin:fixture.origin,report:report.facts});}
   const {default:WebSocket}=await import('ws');
   const clients=[],sockets=[];let native;let resumed=false;let failure;let changed=0;
   const wait=async predicate=>{const deadline=Date.now()+Number(fixture.timeoutMs??60000);while(!predicate()){if(failure)throw failure;if(Date.now()>deadline)throw Error('original-consumer scenario timed out');await new Promise(r=>setTimeout(r,10));}};
   try {
    for(let i=0;i<fixture.accounts.length;i++) {
     if(fixture.mixedNative===i){native=new NativeParticipant(endpoint,fixture.accounts[i],report);await native.login();continue;}
     const ws=new WebSocket(endpoint,{origin:fixture.origin??`http://${new URL(endpoint).host}`});sockets.push(ws);
     const c=new OriginalClient(opts.bundle,{send:b=>{commandFact(b,i);ws.send(b);}});clients.push(c);
     ws.on('message',b=>{try{const facts=c.receive(Buffer.from(b));report.facts.push({client:i,sequence:changed++,facts});}catch(e){failure=e;}});
     ws.on('error',e=>{failure=e;});
     await new Promise((resolve,reject)=>{ws.once('open',resolve);ws.once('error',reject);});
     c.command('login',[fixture.accounts[i].username,fixture.accounts[i].password]);
     await wait(()=>c.events.includes(2));
    }
    const resume=async index=>{
     const old=clients[index],before=old.facts(),session=old.get('sessionService').getSession();assert(session,'original session missing for resume');
     const original=sockets[index];original.terminate();
     const ws=new WebSocket(endpoint,{origin:fixture.origin??`http://${new URL(endpoint).host}`});sockets.push(ws);
     const fresh=new OriginalClient(opts.bundle,{send:b=>{commandFact(b,index);ws.send(b);}});
     ws.on('message',b=>{try{report.facts.push({client:index,resumed:true,sequence:changed++,facts:fresh.receive(Buffer.from(b))});}catch(e){failure=e;}});ws.on('error',e=>{failure=e;});
     await new Promise((resolve,reject)=>{ws.once('open',resolve);ws.once('error',reject);});
     fresh.u.context.resumeSession={playerId:session.playerId,sessionId:session.sessionId};
     fresh.call('LocalStore.setRaw("playerId",resumeSession.playerId);LocalStore.setRaw("sessionId",resumeSession.sessionId);client.get("sessionService").login();delete globalThis.resumeSession;');
     await wait(()=>fresh.events.includes(2)&&fresh.facts().gameId===before.gameId&&fresh.facts().questions.length>0);
     clients[index]=fresh;report.freshSessionResume={gameId:before.gameId,oldQuestion:before.questions[0].index,newQuestion:fresh.facts().questions[0].index,matched:true};
    };
    if(fixture.steps?.length) {
     for(const step of fixture.steps) {
      const c=clients[step.client??0];
      if(step.waitOpcode!==undefined)await wait(()=>c.events.includes(step.waitOpcode));
      if(step.command)c.command(step.command,step.args??[]);
      if(step.answer){await wait(()=>c.facts().questions.length>0);c.answer(step.answer);}
      if(step.expect)await wait(()=>Object.entries(step.expect).every(([k,v])=>JSON.stringify(c.facts()[k])===JSON.stringify(v)));
     }
    } else {
     const host=clients[0];const rules=fixture.rules??[{id:'REQUIRED_CARDS',cards:['VILLAGE','SMITHY','MARKET','FESTIVAL','LABORATORY','MERCHANT','COUNCIL_ROOM','CELLAR','CHAPEL','WORKSHOP']}];if(fixture.originalNewTable)host.newTableDefault();else host.newTable(rules);await wait(()=>host.facts().hasTable);
     if(fixture.originalNewTable)host.newTable(rules,true);
     const table=host.facts().tableId;
     if(native)await native.join(table);
     for(const c of clients.slice(1)){c.command('joinTable',[table,true]);await wait(()=>c.facts().tableId===table);}
     if(fixture.chat){for(const c of clients)c.chat('synthetic harness chat');await wait(()=>clients.every(c=>c.facts().chat.some(l=>l.text==='synthetic harness chat')));}
     for(let round=0;round<Number(fixture.games??2);round++) {
      const before=clients.map(c=>c.events.filter(x=>x===14).length);
      const previousGames=clients.map(c=>c.facts().gameId);
      for(const c of clients)c.command('startGameRequest',[table,true]);
      if(native)await native.ready(table);
      await wait(()=>clients.every(c=>c.events.filter(x=>x===14).length>before[clients.indexOf(c)])||clients.some(c=>c.facts().questions.length>0)||(native&&clients.every((c,i)=>c.facts().gameId!==previousGames[i])));
      for(let moves=0;!clients.every((c,i)=>c.events.filter(x=>x===14).length>before[i]);moves++) {
       if(moves>Number(fixture.maxDecisions??5000))throw Error('decision budget exceeded');
       if(fixture.sessionResume&&!resumed&&clients[0].facts().questions.length){await resume(0);resumed=true;}
       let answered=false;
       for(const c of clients)if(c.facts().questions.length){c.autoAnswer();answered=true;}
       if(native){await native.step();await new Promise(r=>setTimeout(r,10));if(failure)throw failure;}
       else {const sequence=changed;await wait(()=>changed!==sequence||clients.some(c=>c.facts().questions.length>0)||(native&&clients.every((c,i)=>c.facts().gameId!==previousGames[i])));}
       if(!answered&&failure)throw failure;
      }
      for(const c of clients)await verifyScores(c,endpoint,report);
      if(fixture.finalReconnect){for(const c of clients){const previous=c.facts(),finished=c.events.filter(x=>x===14).length;c.command('reconnectGame');await wait(()=>c.events.filter(x=>x===14).length>finished);assert.deepEqual(c.facts().score,previous.score,'finished replacement changed scores');assert.equal(c.facts().questions.length,0,'finished reconnect reopened question');await verifyScores(c,endpoint,report);}report.finalReconnect=true;}
     }
    }
    if(fixture.lifecycleSafety){const {lifecycleSafety}=await import('./lifecycle-safety.mjs');await lifecycleSafety({bundle:opts.bundle,endpoint,accounts:fixture.accounts,clients,sockets,report});}
    for(let i=0;i<clients.length;i++)report.facts.push({client:i,final:true,facts:clients[i].facts()});
    report.passed=true;
   } finally {for(const ws of sockets)ws.terminate();}
  }
 } catch(e) {report.error={name:e.name,message:e.message,stack:e.stack?.split('\n').filter(line=>line.trim().startsWith('at '))};writeReport(opts.output,report);throw e;}
 writeReport(opts.output,report);console.log(`Original service/model/answer ${report.scenario}: passed; report ${resolve(opts.output,'consumer-report.json')}`);
 return report;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main().catch(e=>{console.error(`${e.name}: ${e.message}`);process.exitCode=1;});
