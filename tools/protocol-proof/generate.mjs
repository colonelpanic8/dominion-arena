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

load('PriceCategories');load('Price');load('Writer');load('Reader');load('getByOrdinal');
context.sendPayload=null;
const proc=new (factory('serverToClientProcessor'))({send(event,payload){context.sendPayload=payload}},{},{},{get(){return {}}});
const targets=[17,18,21,23,24,25,30,31,42,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61];
const fixtures=[];
for(const flag of [false,true])for(const opcode of targets){
 context.flag=flag;
 context.opcode=opcode;
 let buffer;
 retry(()=>{
  run('var fw=new Writer()');
  const w=run('fw');
  const reader={
   readInt(){w.writeInt(0);return 0},readLong(){w.writeLong(100);return 100},readDate(){w.writeLong(100);return new Date(100)},
   readDouble(){w.writeDouble(1.25);return 1.25},readBoolean(){w.writeBoolean(flag);return flag},
   readString(){w.writeString('fixture');return 'fixture'},
   readBooleans(){w.writeInt(1);w.writeBoolean(flag);return [flag]},readOptionalEnum(type){w.writeInt(flag?0:-1);return flag?Object.values(type)[0]:undefined},readInts(){w.writeInts([0]);return [0]},readDoubles(){w.writeInt(1);w.writeDouble(1.25);return [1.25]},readStrings(){w.writeInt(1);w.writeString('fixture');return ['fixture']},
   readArrayOf(type){if(type===context.Price){w.writeInt(5);return [0,1,2,3,4].map(i=>{w.writeInt(i);w.writeInt(10);return new type(Object.values(context.PriceCategories)[i],10)})}w.writeInt(1);return [type.parse(this)]},
   readEnumArray(type){w.writeInt(1);w.writeInt(0);return [Object.values(type)[0]]},
   readEnumToObjectMap(type,parse){w.writeInt(1);w.writeInt(0);return new Map([[Object.values(type)[0],parse(this)]])}
  };
  context.sendPayload=null;
  proc.processors[opcode](reader);
  buffer=Buffer.from(w.buffer);
 });
 // Exercise the original Reader on the resulting bytes too.
 context.binary=buffer.buffer.slice(buffer.byteOffset,buffer.byteOffset+buffer.byteLength);
 retry(()=>{proc.processors[opcode](run('var actual=new Reader(binary);actual'));});
 if(run('actual.hasMessages()'))throw Error('unconsumed upstream '+opcode);
 fixtures.push({opcode,optional:flag,hex:buffer.toString('hex')});
}
writeFileSync(new URL('../../crates/dominion-online/tests/fixtures/ancillary.json', import.meta.url),JSON.stringify(fixtures,null,2)+'\n');
console.log('Generated '+fixtures.length+' upstream-verified synthetic messages');
