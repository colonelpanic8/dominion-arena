import { writeFileSync } from 'node:fs';
import { upstream } from './upstream.mjs';

const { context, load, run, retry } = upstream(process.argv[2]);
for (const name of ['_get', 'extendBuffer', 'Reader', 'Writer', 'ChoiceQuestion', 'NumberQuestion', 'ComplexQuestion', 'DelayedQuestion', 'NameQuestion', 'CleanupWindow']) load(name);
const classes = run('[ChoiceQuestion,NumberQuestion,ComplexQuestion,DelayedQuestion,NameQuestion]');
context.game = {state: {cards: Array.from({length: 12}, (_, id) => ({cardName: id === 9 ? 'Village' : 'Copper'}))}};
const game = run('game');
const fixtures = [];
function description(w, type) {
  w.writeInt(type); w.writeInt(-1);
  for (let i = 0; i < 2; i++) { w.writeInt(0); w.writeInt(0); w.writeBoolean(false); }
}
function write(w, q) {
  w.writeInt(q.tag); description(w, q.type);
  if (q.tag === 0) {
    w.writeInt(q.min); w.writeInt(q.max);
    w.writeInt(q.items.length);
    if (q.items.length) {
      w.writeInt(q.element ?? 0);
      for (const item of q.items) {
        if (q.element === 7) {
          w.writeInts(item.names); w.writeBoolean(item.optional); w.writeBoolean(item.reorder); w.writeBoolean(item.enabled);
        } else w.writeInt(item);
      }
    }
    w.writeInt(q.decline ?? -1); w.writeInts(q.defaults ?? []); w.writeInts([]);
  } else if (q.tag === 1) {
    for (const n of [q.min,q.max,q.default,q.decline ?? -1]) w.writeInt(n);
  } else if (q.tag === 2) {
    w.writeInt(q.children.length); q.children.forEach(child => write(w,child));
  } else if (q.tag === 3) w.writeInt(q.decline ?? 4);
}
const choice = (type=2, min=0, max=3) => ({tag:0,type,min,max,items:[7,9,11],defaults:[]});
const number = () => ({tag:1,type:47,min:0,max:4,default:0});
const complex = (type, ...children) => ({tag:2,type,children});
function fixture(label, tree, selections, accepted=true, cleanup=false) {
  run('var w = new Writer()'); const w=run('w'); w.writeInt(42); write(w,tree);
  const bytes=Buffer.from(w.buffer);
  context.binary=bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
  const q=retry(()=>{
    const reader=run('new Reader(binary)'); const index=reader.readInt(), tag=reader.readInt();
    const parsed=classes[tag].parse(game,reader,[index]);
    if (reader.hasMessages()) throw Error('unconsumed question');
    return parsed;
  });
  let valid=true;
  for (const selection of selections) {
    let leaf=q;
    for (const branch of selection.path) {
      if (leaf.type === run('QuestionTypes.COMPLEX_OR')) leaf.accumulatedAnswers=[branch];
      leaf=leaf.subquestions[branch];
    }
    if (cleanup) {
      leaf.updateButtons=()=>{};
      const items=leaf.content.elements;
      for (let i=0;i<items.length;i++) items[i].isEnabled=selection.values.includes(i);
      run('CleanupWindow').prototype.updateAnswers.call({cleanupButtons:[{cleanupItems:[...selection.values.map(i=>items[i]),...items.filter((_,i)=>!selection.values.includes(i))]}]},leaf);
    } else if (selection.kind === 'decline') {
      leaf.accumulatedAnswers=leaf instanceof classes[3] ? [0] : [];
    } else {
      leaf.accumulatedAnswers=[];
      for (const value of selection.values) {
        if (leaf instanceof classes[0] && !leaf.type.isOrder) valid &&= leaf.canAccept(value);
        leaf.accumulatedAnswers.push(value);
      }
    }
  }
  if (valid !== accepted) throw Error(`bad oracle expectation: ${label}`);
  const expected=accepted ? Array.from(q.getAnswer()) : null;
  fixtures.push({label,hex:bytes.toString('hex'),selections,expected});
}
fixture('Temple rejects two copies with the same name',choice(4,2,2),[{path:[],kind:'select',values:[0,2]}],false);
fixture('Banish rejects different names',choice(42,1,3),[{path:[],kind:'select',values:[0,1]}],false);
fixture('Banish allows multiple copies of one name',choice(42,1,3),[{path:[],kind:'select',values:[2,0]}]);
fixture('Warehouse accepts duplicate names with distinct positions',choice(2,2,2),[{path:[],kind:'select',values:[0,2]}]);
fixture('Torturer chooses empty discard rather than Curse',complex(18,{...choice(2,0,0),items:[]},{tag:3,type:21}),[{path:[0],kind:'empty',values:[]}]);
fixture('AND can explicitly submit empty optional selections',complex(19,choice(),choice()),[{path:[0],kind:'empty',values:[]},{path:[1],kind:'empty',values:[]}]);
fixture('Nested AND names and coin/potion overpay',complex(19,{tag:4,type:20},complex(19,number(),{...number(),type:48})),[{path:[0],kind:'name',values:[31]},{path:[1,0],kind:'number',values:[2]},{path:[1,1],kind:'number',values:[3]}]);
fixture('Spend Coffers without selecting a mandatory play sibling',complex(19,choice(0,1,1),number()),[{path:[1],kind:'number',values:[2]}]);
fixture('Inactive name sibling sends an empty list',complex(19,{tag:4,type:20},number()),[{path:[1],kind:'number',values:[2]}]);
for (let button=0;button<8;button++) fixture(`Game button ${button} follows the shared action schema`,{...choice(34,1,1),element:6,items:[button]},[{path:[],kind:'select',values:[0]}]);
fixture('Default numeric sibling is included with lengths',complex(19,choice(2,1,3),number()),[{path:[0],kind:'select',values:[1]}]);
fixture('PRIORITIZE_CARDS permits a partial priority list',choice(33,0,3),[{path:[],kind:'select',values:[2,0],top_first:true}]);
fixture('Cleanup orders enabled subset and can skip optional topdeck',{
  ...choice(35,0,3),element:7,items:[
    {names:[2],optional:false,reorder:true,enabled:true},
    {names:[31],optional:true,reorder:true,enabled:true},
    {names:[2],optional:true,reorder:true,enabled:false},
  ],defaults:[0,1],
},[{path:[],kind:'select',values:[1,0]}],true,true);

const start=Number(process.env.PROTOCOL_SEED ?? 1), count=Number(process.env.PROTOCOL_SEEDS ?? 128);
if (!Number.isSafeInteger(start) || start < 0 || !Number.isSafeInteger(count) || count < 1 || count > 10000) throw Error('invalid seed range');
for (let seed=start;seed<start+count;seed++) {
  let state=seed>>>0;
  const next=n=>{state=(Math.imul(state,1664525)+1013904223)>>>0;return state%n;};
  function tree(depth=0) {
    if (depth<3 && next(3)===0) return complex(next(2)?18:19,...Array.from({length:2+next(2)},()=>tree(depth+1)));
    const kind=next(5);
    if (kind===0) return {...number(),default:next(5)};
    if (kind===1) return {tag:4,type:20};
    if (kind===2) return {tag:3,type:21,decline:4};
    if (kind===3) return {...choice(8,3,3),defaults:[0,1,2]};
    return {...choice(2,0,3),defaults:next(2)?[1]:[]};
  }
  const t=tree(), selections=[];
  function select(q,path=[]) {
    if (q.tag===2) {
      const chosen=q.type===18 ? [next(q.children.length)] : q.children.map((_,i)=>i).filter(i=>i===0 || q.children[i].tag===2 || next(3)!==0);
      for (const i of chosen) select(q.children[i],[...path,i]);
    } else if (q.tag===1) selections.push({path,kind:'number',values:[next(5)]});
    else if (q.tag===4) selections.push({path,kind:'name',values:[next(2)?31:2]});
    else if (q.tag===3) selections.push({path,kind:next(2)?'accept':'decline',values:[1]});
    else {
      const values=q.type===8 ? [0,1,2] : [0,1,2].slice(0,next(4));
      for (let i=values.length-1;i>0;i--) {const j=next(i+1);[values[i],values[j]]=[values[j],values[i]];}
      // The shared contract uses top-first order; upstream accumulates bottom-first.
      selections.push({path,kind:values.length?'select':'empty',values,top_first:q.type===8});
    }
  }
  select(t); fixture(`seed ${seed}`,t,selections);
}
writeFileSync(process.env.PROTOCOL_QUESTION_FIXTURES ?? new URL('../../crates/dominion-online/tests/fixtures/questions.json',import.meta.url),JSON.stringify(fixtures,null,2)+'\n');
console.log(`Generated ${fixtures.length} upstream question/answer cases (seeds ${start}..${start+count-1})`);
