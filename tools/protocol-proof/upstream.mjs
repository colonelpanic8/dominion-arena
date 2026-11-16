import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import {createHash} from 'node:crypto';
const require = createRequire(import.meta.url);
const ts = require('typescript');
export function upstream(root) {
if (!root) throw Error('provide the directory with the pinned head and body bundles');
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

return {context, load, run, factory, evidence, retry};
}
