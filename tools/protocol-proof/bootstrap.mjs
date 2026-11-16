// Run the original discovery/serverConnection/session code on an operator's loopback server.
import WebSocket from 'ws';
import assert from 'node:assert/strict';
import {OriginalClient} from './consumer.mjs';
import {loopback} from './client-harness.mjs';
export async function bootstrap(bundle,endpoint,account,{origin,report}={}) {
 const url=loopback(endpoint);if(url.pathname!=='/')throw Error('original connection requires a root endpoint');
 const c=new OriginalClient(bundle);const sockets=[];const transitions=[];let failure;
 origin??=`http://${url.host}`;
 const factory=c.u.factory('serverConnection');
 const real=new factory(c.get('metaBroadcaster'),c.get('serverToClientProcessor'),c.env.root,c.get('$injector'));
 class LocalSocket extends WebSocket {
  constructor(target){loopback(target);super(target,{origin});sockets.push(this);this.on('message',()=>queueMicrotask(()=>{try{c.env.pump();}catch(e){failure=e;}}));this.on('error',e=>{failure=e;});}
  send(bytes,...rest){const data=Buffer.from(bytes);transitions.push({direction:'outgoing',opcode:data.readInt32BE(),redacted:[1,13,24,45,46].includes(data.readInt32BE())});return super.send(bytes,...rest);}
 }
 c.u.context.WebSocket=LocalSocket;
 Object.assign(c.get('serverConnection'),real);
 // Methods close over the unmodified upstream instance; keep the existing DI identity.
 for(const key of Object.keys(real))if(typeof real[key]==='function')c.get('serverConnection')[key]=real[key];
 c.sendWire=bytes=>real.send(bytes);
 const wait=async fn=>{const deadline=Date.now()+10000;while(!fn()){if(failure)throw failure;if(Date.now()>deadline)throw Error('original bootstrap timed out');await new Promise(r=>setTimeout(r,5));}};
 const endpointInfo=()=>({host:url.hostname==='[::1]'?'[::1]':url.hostname,port:Number(url.port),ssl:false});
 try {
  for(const count of [2,1]){
   c.u.context.USED_ALPHA_CONNECTION=endpointInfo();c.u.context.USED_BETA_CONNECTION=count===2?endpointInfo():undefined;
   c.get('connection').initialize();await wait(()=>real.webSocket?.readyState===WebSocket.OPEN&&real.description==='best connection');
   assert.equal(sockets.length,count===2?3:4);
   if(count===2){assert.equal(c.u.context.USED_ALPHA_CONNECTION.serverState,c.call('ServerStates.UP_NORMAL'));assert.equal(c.u.context.USED_BETA_CONNECTION.serverState,c.call('ServerStates.UP_NORMAL'));}
   real.disconnect();await wait(()=>real.webSocket.readyState===WebSocket.CLOSED);
  }
  // A bad version goes through the actual special-envelope probe handler.
  c.u.context.USED_ALPHA_CONNECTION=endpointInfo();c.u.context.VERSION='invalid-local-version';
  c.call('var badInfo=USED_ALPHA_CONNECTION;client.get("connection").testConnection("bad version",badInfo,{playerId:undefined,sessionId:undefined},new TestConnectionHandler(badInfo,client.get("serverConnection")),function(){})');
  await wait(()=>c.call('badInfo.serverState===ServerStates.BAD_CLIENT_VERSION'));
  c.u.context.VERSION='2.3.7';
  c.u.context.USED_ALPHA_CONNECTION=endpointInfo();c.get('connection').initialize();await wait(()=>real.webSocket.readyState===WebSocket.OPEN&&real.description==='best connection');
  c.command('login',[account.username,account.password]);await wait(()=>c.get('sessionService').getSession()!==null);
  const playerId=c.get('meta').getMyId();
  // Session service persists only to isolated memory and emits the original producer.
  real.disconnect();await wait(()=>real.webSocket.readyState===WebSocket.CLOSED);
  c.get('connection').initialize();await wait(()=>real.webSocket.readyState===WebSocket.OPEN);
  c.get('sessionService').login();await wait(()=>real.expectedMessageCounter>0);
  assert.equal(c.get('meta').getMyId(),playerId);
  real.disconnect();await wait(()=>real.webSocket.readyState===WebSocket.CLOSED);
  c.get('connection').initialize();await wait(()=>real.webSocket.readyState===WebSocket.OPEN);
  c.get('sessionService').reconnect();await wait(()=>real.expectedMessageCounter>0);
  assert.equal(c.get('meta').getMyId(),playerId);
  real.disconnect();await wait(()=>real.webSocket.readyState===WebSocket.CLOSED);
  c.get('connection').initialize();await wait(()=>real.webSocket.readyState===WebSocket.OPEN);
  let denied;c.env.root.$on(c.call('Events.COMMAND_FAILED'),(_e,v)=>{denied=v;});
  c.command('loginWithSession',[playerId,'synthetic-invalid-session']);await wait(()=>denied!==undefined);
  assert.equal(denied.failureReason,c.call('FailureReasons.INVALID_SESSION_ID'));
  assert.equal(c.get('sessionService').getSession(),null);
  report?.push({name:'original-bootstrap',twoProbeEndpoints:true,singleDirectEndpoint:true,badVersion:true,passwordLogin:true,sessionLogin:true,sessionReconnect:true,invalidSession:true,transitions});
  return transitions;
 } finally {for(const socket of sockets)socket.terminate();}
}
