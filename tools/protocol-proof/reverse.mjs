// Offline verification of Rust server bytes through the pinned upstream parsers.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';
import vm from 'node:vm';
const ts = createRequire(import.meta.url)('typescript');
const [bodyFile, fixtureFile] = process.argv.slice(2);
if (!bodyFile) throw Error('usage: node reverse.mjs BODY_2.3.7.js [SERVER_FIXTURES.json]');
const source = readFileSync(bodyFile, 'utf8');
assert.equal(createHash('sha256').update(source).digest('hex'), '53f5163219f75d782e971d9dd96337a016c1f4d2c23a51476f50700ec6a816dc');
const file = ts.createSourceFile(bodyFile, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const declarations = new Map();
function visit(n) {
  if (ts.isFunctionDeclaration(n) && n.parent === file && n.name) declarations.set(n.name.text, n.getText(file));
  if (ts.isVariableDeclaration(n) && n.parent?.parent?.parent === file && ts.isIdentifier(n.name) && n.initializer) declarations.set(n.name.text, `var ${n.getText(file)};`);
  ts.forEachChild(n, visit);
}
visit(file);
const context = vm.createContext({TextEncoder,TextDecoder,window:{performance:{now:()=>0}},performance:{now:()=>0},shouldUseTextEncoder:true,isDebug:false,console:{log(){},error(){}},timeLog(){},
  setTimeout(){throw Error('timers forbidden');},fetch(){throw Error('network forbidden');}});
const loaded = new Set();
function retry(fn) {
  for(let i=0;i<100;i++)try{return fn();}catch(error){
    const name=/^(\w+) is not defined$/.exec(error.message)?.[1];
    if(!name || !declarations.has(name) || loaded.has(name))throw error;
    load(name);
  }
  throw Error('dependency limit');
}
function load(name) {
  if(loaded.has(name))return;
  retry(()=>vm.runInContext(declarations.get(name), context, {filename:name,timeout:2000}));
  loaded.add(name);
}
function run(code){
  const offset=context.reader?.offset;
  return retry(()=>{if(offset!==undefined)context.reader.offset=offset;return vm.runInContext(code,context,{timeout:2000});});
}
for(const name of ['Reader','getByOrdinal','GameState','CostReduction','TemporaryEffects','ChoiceQuestion','NumberQuestion','ComplexQuestion','DelayedQuestion','NameQuestion','GameFinished','TableDetailsProcessor','AutoPlaySettings','TableOptions'])load(name);
for (const [name,count] of [['QuestionIds',736],['AbilityNames',403],['CardModes',221],['GameButtons',8],['DeclineButtonIds',71]]) {
  load(name);assert.equal(run(`Object.keys(${name}).length`),count,`pinned ordinal count: ${name}`);
}
// The actual GameState.parse and its read methods run unchanged. Only rendering
// objects and post-parse effects are inert: they neither read nor write bytes.
run(`
var Player=function(state,id,index,reminders,projects){Object.assign(this,{state,playerId:id,index,reminders,projects,counters:[]});};
var Zone=function(state,index,pileName,owner,creator,trait,visible){Object.assign(this,{state,index,pileName,owner,creator,trait,visible,cardStacks:[],tokens:[],counters:[]});this.initialize=function(ids){this.ids=ids;};this.addMarker=function(){};};
var RevealZone=Zone;
var CardObject=function(state,id,cardName,pileName){Object.assign(this,{state,id,cardName,pileName});this.checkIfOrphaned=function(){};};
var Counter=function(state,index,kind,owner,zone,card,value,visible){Object.assign(this,{state,index,kind,owner,zone,card,value,visible});};
var Token=function(state,index,kind,owner,zone,flipped){Object.assign(this,{state,index,kind,owner,zone,flipped});};
CostReduction.prototype.execute=function(){};
TemporaryEffects.prototype.applyTo=function(){};
`);
// Preserve each constructor ordinal and name from the pinned source array.
const zoneArray = /ZoneNames=\[([^\]]+)\]/.exec(source)[1].split(',');
context.zoneArray=zoneArray;
run('var ZoneNames=zoneArray.map(function(name){return class extends Zone {constructor(...args){super(...args);this.kind=name;}};});');
const eventNames=['CardMove','CounterChange','PileUpdate','TurnDescription','Shuffle','Inspection','CostReduction','TokenChange','ZoneCreated','ZoneDestroyed','Rename','BehaviourChange','PilesStatus','RemindersChange','ImageChange','Association','CardStatus','ProjectBought','ChangeCardTypes','PileReorder','ProjectRemoved'];
for(const name of eventNames)load(name);
context.eventNames=eventNames;
function parseEvent(){run('var eventTag=reader.readInt();var parser=globalThis[eventNames[eventTag]];var event=parser.parse.length===1?parser.parse(reader):parser.parse(game,reader)');}
const fixtures=JSON.parse(readFileSync(fixtureFile ?? new URL('../../crates/dominion-online/tests/fixtures/server.json',import.meta.url),'utf8'));
let counter=0;
context.game={};
for(const fixture of fixtures){
  const bytes=Buffer.from(fixture.hex,'hex');
  context.binary=bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
  run('var reader=new Reader(binary)');
  assert.equal(run('reader.readInt()'),counter++);
  const opcode=run('reader.readInt()');
  switch(opcode){
    case 13:
      run('var table=(new TableDetailsProcessor()).process(reader)');
      assert.equal(run('table.tableId'),1);assert.deepEqual(Array.from(run('table.players')),[10,20]);
      assert.deepEqual(Array.from(run('table.playerNames')),['Alice','Bob']);break;
    case 38:
      assert.equal(run('reader.readBoolean()'),false);assert.equal(run('reader.readLong()'),7);
      run('game.state=new GameState(game);game.state.parse(reader)');
      assert.deepEqual(Array.from(run('game.state.players.map(p=>p.playerId)')),[10,20]);
      assert.deepEqual(Array.from(run('game.state.cards.map(c=>c.cardName.name)')),['Copper','Copper','Silver']);
      assert.equal(run('game.state.zones[0].kind'),'HandZone');
      assert.deepEqual(Array.from(run('game.state.zones[1].ids')),[-1,-1]);
      assert.equal(run('game.state.activeTurn.turnNumber'),12);
      run('var autoplay=AutoPlaySettings.parse(reader);var options=TableOptions.parse(reader)');
      assert.equal(run('getOrdinal(TimerPresetOptions,options.timerSettings.timerPresetOption)'),4);
      const n=run('reader.readInt()');assert.equal(n,fixture.name==='snapshot-events'?21:0);for(let i=0;i<n;i++)parseEvent();
      break;
    case 37:
      run('var index=reader.readInt();var tag=reader.readInt();var question=[ChoiceQuestion,NumberQuestion,ComplexQuestion,DelayedQuestion,NameQuestion][tag].parse(game,reader,[index])');
      assert.equal(run('question.questionIndex[0]'),42);
      if(fixture.name==='question'){assert.equal(run('question.max'),1);assert.deepEqual(Array.from(run('question.content.elements')),[0]);}
      if(fixture.name==='complex'){assert.equal(run('question.subquestions.length'),2);assert.deepEqual(Array.from(run('question.subquestions[1].questionIndex')),[42,1]);}
      break;
    case 14:
      run('var finished=GameFinished.parse(reader)');
      assert.equal(run('finished.gameResult.gameId'),7);assert.equal(run('finished.tableDetails.tableId'),1);
      assert.deepEqual(Array.from(run('finished.gameResult.playerResults.map(p=>p.rank)')),[1,2]);
      assert.deepEqual(Array.from(run('finished.gameResult.playerResults.map(p=>p.score.usedTurns)')),[12,11]);
      assert.equal(run('finished.continueAllowed'),false);assert.equal(run('finished.matchCompleted'),true);break;
    case 32: parseEvent();break;
    case 36: break;
    default: throw Error(`unexpected opcode ${opcode}`);
  }
  assert.equal(run('reader.hasMessages()'),false,`unconsumed bytes: ${fixture.name}`);
}
console.log(`Verified ${fixtures.length} Rust-encoded core messages with pinned upstream parsers; body SHA-256 verified; no network.`);
