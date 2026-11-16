import { writeFileSync } from 'node:fs';
import { upstream } from './upstream.mjs';
const {context, load, run, factory, retry} = upstream(process.argv[2]);

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
