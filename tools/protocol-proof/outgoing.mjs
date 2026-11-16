import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
import vm from 'node:vm';
import {createHash} from 'node:crypto';
const require = createRequire(import.meta.url);
const ts = require('typescript');
const root = process.argv[2];
if (!root) throw Error('usage: node generate.mjs DIRECTORY_WITH_PINNED_HEAD_AND_BODY');
const declarations = new Map(), factories = new Map(), evidence = [];
for (const part of ['head','body']) {
  const file = `${root}/dominion-webclient-${part}-2.3.7.min.js`;
  const code = readFileSync(file,'utf8');
  if(part==='body' && createHash('sha256').update(code).digest('hex')!=='53f5163219f75d782e971d9dd96337a016c1f4d2c23a51476f50700ec6a816dc')throw Error('unexpected upstream body hash');
  if(part==='head' && createHash('sha256').update(code).digest('hex')!=='b64b4b166952d97929b502c0ca43bff213e97883d54f0f3d69aba2539e934627')throw Error('unexpected upstream head hash');
  const sf = ts.createSourceFile(file,code,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
  function visit(n) {
    if (ts.isFunctionDeclaration(n) && n.parent === sf && n.name) declarations.set(n.name.text,{code:n.getText(sf),part,offset:n.getStart(sf)});
    if (ts.isVariableDeclaration(n) && n.parent?.parent?.parent === sf && ts.isIdentifier(n.name) && n.initializer) declarations.set(n.name.text,{code:`var ${n.getText(sf)};`,part,offset:n.getStart(sf)});
    if (ts.isCallExpression(n) && n.expression.getText(sf)==='webclient.service' && ts.isStringLiteral(n.arguments[0])) {
      factories.set(n.arguments[0].text,{code:(ts.isArrayLiteralExpression(n.arguments[1]) ? n.arguments[1].elements.at(-1) : n.arguments[1]).getText(sf),part,offset:n.getStart(sf)});
    }
    ts.forEachChild(n,visit);
  }
  visit(sf);
}
const context = vm.createContext({TextEncoder,TextDecoder,window:{performance:{now:()=>0}},performance:{now:()=>0},console:{log(){},error(){}},shouldUseTextEncoder:true,VERSION:'2.3.7',isDebug:false,timeLog(){},setTimeout(){throw Error('timers forbidden in offline proof');},fetch(){throw Error('network forbidden');}});
const loaded = new Set(), active = new Set();
function missing(e) { return e instanceof Error || e?.name==='ReferenceError' ? /^(\w+) is not defined$/.exec(e.message)?.[1] : null; }
function retry(fn) { for(let i=0;i<80;i++)try{return fn()}catch(e){const name=missing(e);if(!name||!declarations.has(name)||loaded.has(name))throw e;load(name)} throw Error('dependency limit'); }
function load(name) {
  if(loaded.has(name))return;
  if(active.has(name))throw Error(`cyclic ${name}`);
  const d=declarations.get(name);if(!d)throw Error(`No declaration ${name}`);
  active.add(name);
  retry(()=>vm.runInContext(d.code,context,{filename:`${d.part}@${d.offset}`,timeout:2000}));
  active.delete(name);loaded.add(name);evidence.push({name,part:d.part,offset:d.offset});
}
function run(code){return retry(()=>vm.runInContext(code,context,{timeout:2000}));}
function factory(name){const d=factories.get(name);evidence.push({name:`service:${name}`,part:d.part,offset:d.offset});return vm.runInContext(`(${d.code})`,context,{timeout:2000});}

for(const name of ['Writer','Reader','getOrdinal','getByOrdinal','ClientToServerIds','LANGUAGE']) {if(name!=='LANGUAGE')load(name);}
context.LANGUAGE={getName(){return 'English'}};
let sent;
context.messenger=new (factory('serverMessenger'))({send(bytes){sent=Buffer.from(bytes)}},{hasChangedLanguage(){return false}});
const output=[];
retry(()=>run("messenger.chatMessage('fixture','fixture','fixture')"));output.push({opcode:0,hex:sent.toString("hex")});
retry(()=>run("messenger.login('fixture','fixture')"));output.push({opcode:1,hex:sent.toString("hex")});
retry(()=>run("messenger.joinTable(100,true)"));output.push({opcode:2,hex:sent.toString("hex")});
retry(()=>run("messenger.sendInvite(100,0)"));output.push({opcode:3,hex:sent.toString("hex")});
retry(()=>run("messenger.changeTableRule(100,({id:TableRuleIds.RATED_GAME,value:true}))"));output.push({opcode:4,hex:sent.toString("hex")});
retry(()=>run("messenger.changeHost(100,0)"));output.push({opcode:5,hex:sent.toString("hex")});
retry(()=>run("messenger.leaveTable(100,0)"));output.push({opcode:6,hex:sent.toString("hex")});
retry(()=>run("messenger.sitDown(100)"));output.push({opcode:7,hex:sent.toString("hex")});
retry(()=>run("messenger.standUp(100,0)"));output.push({opcode:8,hex:sent.toString("hex")});
retry(()=>run("messenger.newTableRequest([])"));output.push({opcode:9,hex:sent.toString("hex")});
retry(()=>run("messenger.startGameRequest(100,true)"));output.push({opcode:10,hex:sent.toString("hex")});
retry(()=>run("messenger.requestUpdate(getByOrdinal(UpdateTypes,0))"));output.push({opcode:11,hex:sent.toString("hex")});
retry(()=>run("messenger.setUserPreference(({id:UserPrefIds.CONFIRM_ACTIONS,arguments:true}))"));output.push({opcode:12,hex:sent.toString("hex")});
retry(()=>run("messenger.loginWithCode('fixture')"));output.push({opcode:13,hex:sent.toString("hex")});
retry(()=>run("messenger.loginWithNewPassword('fixture','fixture')"));output.push({opcode:14,hex:sent.toString("hex")});
retry(()=>run("messenger.forgotPassword('fixture')"));output.push({opcode:15,hex:sent.toString("hex")});
retry(()=>run("messenger.changePassword('fixture','fixture')"));output.push({opcode:16,hex:sent.toString("hex")});
retry(()=>run("messenger.requestPurchasePrice(({storePackage:getByOrdinal(StorePackages,0),endDate:100}))"));output.push({opcode:17,hex:sent.toString("hex")});
retry(()=>run("messenger.purchase(({onlineProduct:{storePackage:getByOrdinal(StorePackages,0),endDate:100},purchaseTime:100,amountInDollarCents:0}))"));output.push({opcode:18,hex:sent.toString("hex")});
retry(()=>run("messenger.createAccount(({username:'fixture',password:'fixture',email:'fixture',gender:0}),'fixture')"));output.push({opcode:19,hex:sent.toString("hex")});
retry(()=>run("messenger.removeSessions('fixture','fixture',true)"));output.push({opcode:20,hex:sent.toString("hex")});
retry(()=>run("messenger.changeFriendStatus(({id:0,name:'fixture'}),true,true)"));output.push({opcode:21,hex:sent.toString("hex")});
retry(()=>run("messenger.requestAutomatch(new MatchingCriteria(0,true,0,0,new Map(),new Map()))"));output.push({opcode:22,hex:sent.toString("hex")});
retry(()=>run("messenger.addBot(100)"));output.push({opcode:23,hex:sent.toString("hex")});
retry(()=>run("messenger.loginWithSession(0,'fixture')"));output.push({opcode:24,hex:sent.toString("hex")});
retry(()=>run("messenger.changeMailStatus(100,getByOrdinal(MailStati,0))"));output.push({opcode:25,hex:sent.toString("hex")});
retry(()=>run("messenger.requestMailMessages([100])"));output.push({opcode:26,hex:sent.toString("hex")});
retry(()=>run("messenger.requestMailSummaries(100)"));output.push({opcode:27,hex:sent.toString("hex")});
retry(()=>run("messenger.requestLeaderboard(0,true)"));output.push({opcode:28,hex:sent.toString("hex")});
retry(()=>run("messenger.terminateGame()"));output.push({opcode:29,hex:sent.toString("hex")});
retry(()=>run("messenger.changeBlacklistStatus(({id:0,name:'fixture'}),true,true)"));output.push({opcode:30,hex:sent.toString("hex")});
retry(()=>run("messenger.continueWithBots(100)"));output.push({opcode:31,hex:sent.toString("hex")});
retry(()=>run("messenger.logout()"));output.push({opcode:32,hex:sent.toString("hex")});
retry(()=>run("messenger.validateBonusCode(0,'fixture')"));output.push({opcode:33,hex:sent.toString("hex")});
retry(()=>run("messenger.useBonusCode(0,'fixture',Object.values(Expansions)[0].name)"));output.push({opcode:34,hex:sent.toString("hex")});
retry(()=>run("messenger.reconnectGame()"));output.push({opcode:35,hex:sent.toString("hex")});
retry(()=>run("messenger.removeActiveSessions()"));output.push({opcode:36,hex:sent.toString("hex")});
retry(()=>run("messenger.answerQuestion(({questionIndex:0,list:[0]}))"));output.push({opcode:37,hex:sent.toString("hex")});
retry(()=>run("messenger.resign()"));output.push({opcode:38,hex:sent.toString("hex")});
retry(()=>run("messenger.undoRequest(0)"));output.push({opcode:39,hex:sent.toString("hex")});
retry(()=>run("messenger.timeoutRequest(({decisionIndex:0,playerIndex:0}))"));output.push({opcode:40,hex:sent.toString("hex")});
retry(()=>run("messenger.undoRequestGranted(({serialize(w){w.writeInt(0);w.writeInt(0)}}))"));output.push({opcode:41,hex:sent.toString("hex")});
retry(()=>run("messenger.undoRequestDenied(({serialize(w){w.writeInt(0);w.writeInt(0)}}))"));output.push({opcode:42,hex:sent.toString("hex")});
retry(()=>run("messenger.undoRequestCancelled(({serialize(w){w.writeInt(0);w.writeInt(0)}}))"));output.push({opcode:43,hex:sent.toString("hex")});
retry(()=>run("messenger.ping()"));output.push({opcode:44,hex:sent.toString("hex")});
retry(()=>run("var w=new Writer();w.writeInt(getOrdinal(ClientToServerIds,ClientToServerIds.REQUEST_SERVER_STATE));w.writeString(VERSION);w.writeOptionalInt(0);w.writeOptionalString('fixture');messenger.send(w.buffer)"));output.push({opcode:45,hex:sent.toString("hex")});
retry(()=>run("messenger.reconnectWithSession(0,'fixture')"));output.push({opcode:46,hex:sent.toString("hex")});
retry(()=>run("messenger.requestConnectToOtherServer()"));output.push({opcode:47,hex:sent.toString("hex")});
retry(()=>run("messenger.previewRemovalSelection(0,[0])"));output.push({opcode:48,hex:sent.toString("hex")});
retry(()=>run("messenger.endTurn()"));output.push({opcode:49,hex:sent.toString("hex")});
retry(()=>run("messenger.submitFeedback('fixture',({client_language:'English'}))"));output.push({opcode:50,hex:sent.toString("hex")});
retry(()=>run("messenger.animationCompensationAck()"));output.push({opcode:51,hex:sent.toString("hex")});
retry(()=>run("messenger.tournamentMatchCommand(100,0,0,getByOrdinal(TournamentTypes,0))"));output.push({opcode:52,hex:sent.toString("hex")});
retry(()=>run("messenger.setUserPreferenceForAllGames(({id:UserPrefIds.CONFIRM_ACTIONS,arguments:true}))"));output.push({opcode:53,hex:sent.toString("hex")});
retry(()=>run("messenger.requestCardStats(1.25,'fixture','fixture')"));output.push({opcode:54,hex:sent.toString("hex")});
retry(()=>run("messenger.requestCardPerTurn(1.25,'fixture',0)"));output.push({opcode:55,hex:sent.toString("hex")});
retry(()=>run("messenger.requestScorigami()"));output.push({opcode:56,hex:sent.toString("hex")});
retry(()=>run("messenger.requestPlayerProfile(0,getByOrdinal(RatingTypes,0),0,0)"));output.push({opcode:57,hex:sent.toString("hex")});
retry(()=>run("messenger.updatePlayerProfile('fixture','fixture',getByOrdinal(RatingTypes,0),'fixture')"));output.push({opcode:58,hex:sent.toString("hex")});
retry(()=>run("messenger.requestHeadToHead(0,'fixture',getByOrdinal(RatingTypes,0))"));output.push({opcode:59,hex:sent.toString("hex")});
retry(()=>run("messenger.requestPlayerAvatars([0])"));output.push({opcode:60,hex:sent.toString("hex")});
retry(()=>run("messenger.requestPlayerResults(0,getByOrdinal(RatingTypes,0),0)"));output.push({opcode:61,hex:sent.toString("hex")});
retry(()=>run("messenger.requestPlayerSearch('fixture')"));output.push({opcode:62,hex:sent.toString("hex")});
retry(()=>run("messenger.requestStatsViews()"));output.push({opcode:63,hex:sent.toString("hex")});
retry(()=>run("messenger.saveStatsView('fixture','fixture')"));output.push({opcode:64,hex:sent.toString("hex")});
retry(()=>run("messenger.deleteStatsView('fixture')"));output.push({opcode:65,hex:sent.toString("hex")});
retry(()=>run("messenger.requestStatsProfiles('fixture')"));output.push({opcode:66,hex:sent.toString("hex")});
retry(()=>run("messenger.requestStatsComparison('fixture',1.25,'fixture',1.25)"));output.push({opcode:67,hex:sent.toString("hex")});
writeFileSync(new URL('../../crates/dominion-online/tests/fixtures/outgoing.json',import.meta.url),JSON.stringify(output,null,2)+'\n');console.log('Verified '+output.length+' outgoing layouts');
