// Original-consumer regressions for canonical Arena membership and game-policy lifetime.
import WebSocket from 'ws';
import assert from 'node:assert/strict';
import {OriginalClient} from './consumer.mjs';
import {loopback} from './client-harness.mjs';
export async function lifecycleSafety({bundle,endpoint,accounts,clients,sockets,report}) {
 const base=loopback(endpoint);base.protocol='http:';base.pathname='/';
 const owned=[];let failure;
 const wait=async predicate=>{const end=Date.now()+10000;while(!predicate()){if(failure)throw failure;if(Date.now()>end)throw Error('original lifecycle-safety transition timed out');await new Promise(r=>setTimeout(r,5));}};
 const api=async account=>{let cookie='';const request=async(path,body,method=body===undefined?'GET':'POST')=>{const res=await fetch(new URL(path,base),{method,headers:{Origin:base.origin,...cookie?{Cookie:cookie}:{},...body!==undefined?{'Content-Type':'application/json'}:{}},...body!==undefined?{body:JSON.stringify(body)}:{},redirect:'error'});assert(res.ok,`native safety ${path}: ${res.status}`);if(res.headers.get('set-cookie'))cookie=res.headers.get('set-cookie').split(';')[0];return res.status===204?null:res.json();};await request('/api/auth/login',{name:account.username,password:account.password});return request;};
 const http=await Promise.all(accounts.slice(0,2).map(api));
 const c=clients[0],session=c.get('sessionService').getSession();assert(session,'original local authority missing');
 const connect=async()=>{
  const ws=new WebSocket(endpoint,{origin:base.origin});owned.push(ws);
  const original=new OriginalClient(bundle,{send:b=>{const opcode=b.readInt32BE();report.commands.push({client:'safety',opcode,...[1,13,24,45,46].includes(opcode)?{redacted:true}:{hex:b.toString('hex')}});ws.send(b);}});
  ws.on('message',b=>{try{original.receive(Buffer.from(b));}catch(e){failure=e;}});ws.on('error',e=>{failure=e;});
  await new Promise((resolve,reject)=>{ws.once('open',resolve);ws.once('error',reject);});
  original.u.context.localResume={playerId:session.playerId,sessionId:session.sessionId};
  original.call('LocalStore.setRaw("playerId",localResume.playerId);LocalStore.setRaw("sessionId",localResume.sessionId);client.get("sessionService").login();delete globalThis.localResume;');
  await wait(()=>original.events.includes(2));return {ws,c:original};
 };
 const result={};
 try {
  const finished=c.facts();assert(finished.score,'safety starts after ordinary completion');const table=`t${finished.tableId}`;
  const info=await http[0](`/api/tables/${table}`);
  await http[0](`/api/tables/${table}/spec`,{...info.spec,undo:true,timed:true},'PUT');
  const refresh=c.events.filter(n=>n===14).length;c.command('reconnectGame');await wait(()=>c.events.filter(n=>n===14).length>refresh);assert.deepEqual(c.facts().score,finished.score,'mutable next rules corrupted retained original result');
  const retained=await connect();await wait(()=>retained.c.facts().score!==null);assert.deepEqual(retained.c.facts().score,finished.score);retained.ws.terminate();result.retainedResultAfterNextRuleEdit=true;
  let lobbyReturns=0;c.env.root.$on(c.call('Events.RETURN_TO_LOBBY'),()=>lobbyReturns++);
  const before=lobbyReturns;c.get('tableService').leave();await wait(()=>!c.facts().hasTable&&lobbyReturns>before);assert.equal((await http[0]('/api/lobby')).table,null);result.postFinishLeave=true;
  c.command('joinTable',[finished.tableId,true]);await wait(()=>c.facts().hasTable);result.rejoinAfterLeave=true;
  const removed=lobbyReturns;await http[1](`/api/tables/${table}/seats`,[{player:accounts[1].username},'open'],'PUT');await wait(()=>!c.facts().hasTable&&lobbyReturns>removed);result.hostKick=true;
  c.newTableDefault();await wait(()=>c.facts().hasTable&&c.facts().tableId!==finished.tableId);const next=c.facts().tableId;
  const nativeLeave=lobbyReturns;await http[0](`/api/tables/t${next}/leave`,{});await wait(()=>!c.facts().hasTable&&lobbyReturns>nativeLeave);result.nativeLeave=true;
  c.newTableDefault();await wait(()=>c.facts().hasTable);const voluntary=lobbyReturns;c.get('tableService').leave();await wait(()=>!c.facts().hasTable&&lobbyReturns>voluntary);result.voluntaryLeave=true;result.returnToLobbyEvents=lobbyReturns;
  for(const socket of sockets)socket.terminate();
  for(const request of http){const lobby=await request('/api/lobby');if(lobby.table)await request(`/api/tables/${lobby.table.id}/leave`,{});}
  const spec={...info.spec,kingdom:['Village','Smithy','Market','Festival','Laboratory','Merchant','Council Room','Cellar','Chapel','Workshop'],sets:['Base'],slots:[],colonies:false,shelters:false,rated:false,timed:false,undo:true,randomizeOrder:false};
  let undo=await http[0]('/api/tables',{spec});await http[1](`/api/tables/${undo.id}/join`,{});await http[0](`/api/tables/${undo.id}/ready`,{ready:true});undo=await http[1](`/api/tables/${undo.id}/ready`,{ready:true});assert(undo.game,'native undo game did not start');
  const nativeAnswer=async()=>{for(const request of http){const view=await request(`/api/games/${undo.game}/view`);const decision=view.view.decision;if(decision){await request(`/api/games/${undo.game}/answer`,{decision:decision.id,indices:[decision.options[0].index]});return true;}}return false;};
  assert(await nativeAnswer(),'native initial decision missing');
  const rejected=await connect();await wait(()=>rejected.c.events.includes(1));assert.equal(rejected.c.facts().gameId,-1);assert.equal(rejected.c.facts().questions.length,0);
  const pong=rejected.c.events.filter(n=>n===36).length;rejected.c.command('ping');await wait(()=>rejected.c.events.filter(n=>n===36).length>pong);
  assert(await nativeAnswer(),'native continuation after refusal failed');
  await new Promise(r=>setTimeout(r,200));const failures=rejected.c.events.filter(n=>n===1).length;
  await new Promise(r=>setTimeout(r,200));assert.equal(rejected.c.events.filter(n=>n===1).length,failures,'refusal flooded without state change');assert(failures<=4,'unbounded compatibility failures');
  rejected.c.call('meta.broadcast(Events.LOGOUT)');await wait(()=>rejected.ws.readyState===WebSocket.CLOSED);assert.equal(rejected.c.get('sessionService').getSession(),null);
  assert(await nativeAnswer(),'native continuation after official logout failed');result.undoRefusal={failureCount:failures,pingResponsive:true,logoutResponsive:true,nativeContinued:true};
  report.lifecycleSafety={...result,passed:true};
 } finally {for(const socket of owned)socket.terminate();}
}
