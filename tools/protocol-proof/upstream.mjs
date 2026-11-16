// Only hash-verified, locally supplied source enters the isolated consumer VM.
import { readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import vm from 'node:vm';
const ts = createRequire(import.meta.url)('typescript');
export const BODY_SHA256 = '53f5163219f75d782e971d9dd96337a016c1f4d2c23a51476f50700ec6a816dc';
export function extract(path) {
  if(statSync(path).isDirectory()) {
    const body=extract(join(path,'dominion-webclient-body-2.3.7.min.js'));
    const head=readFileSync(join(path,'dominion-webclient-head-2.3.7.min.js'),'utf8');
    if(createHash('sha256').update(head).digest('hex')!=='b64b4b166952d97929b502c0ca43bff213e97883d54f0f3d69aba2539e934627')throw Error('pinned upstream head SHA-256 mismatch');
    const file=ts.createSourceFile('head',head,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
    for(const statement of file.statements){if(ts.isFunctionDeclaration(statement)&&statement.name)body.declarations.set(statement.name.text,statement.getText(file));if(ts.isVariableStatement(statement))for(const d of statement.declarationList.declarations)if(ts.isIdentifier(d.name)&&d.initializer)body.declarations.set(d.name.text,`var ${d.getText(file)};`);}
    return body;
  }
  const source = readFileSync(path, 'utf8');
  if (createHash('sha256').update(source).digest('hex') !== BODY_SHA256) throw Error('pinned upstream bundle SHA-256 mismatch');
  const file = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const declarations = new Map(), registrations = new Map(), initializers = new Map();
  function expression(n) {
    if (ts.isBinaryExpression(n) && n.operatorToken.kind === ts.SyntaxKind.CommaToken) {expression(n.left); expression(n.right); return;}
    let root = ts.isBinaryExpression(n) ? n.left : ts.isCallExpression(n) ? n.expression : null;
    while (root && (ts.isPropertyAccessExpression(root) || ts.isElementAccessExpression(root))) root=root.expression;
    if (root && ts.isIdentifier(root) && root.text !== "app") {
      const list=initializers.get(root.text)??[];list.push(n.getText(file)+";");initializers.set(root.text,list);
    }
  }
  function visit(n) {
    if (ts.isFunctionDeclaration(n) && n.parent === file && n.name) declarations.set(n.name.text, n.getText(file));
    if (ts.isVariableDeclaration(n) && n.parent?.parent?.parent === file && ts.isIdentifier(n.name) && n.initializer) declarations.set(n.name.text, `var ${n.getText(file)};`);
    if (ts.isCallExpression(n) && ts.isPropertyAccessExpression(n.expression) && ['service','factory','controller','directive','filter'].includes(n.expression.name.text) && ts.isStringLiteral(n.arguments[0]) && n.arguments[1]) {
      const name = n.arguments[0].text, definition = n.arguments[1];
      const parts = ts.isArrayLiteralExpression(definition) ? [...definition.elements] : [definition];
      const fn = parts.pop();
      registrations.set(name, { kind: n.expression.name.text, dependencies: parts.map(x=>x.text), source: fn.getText(file) });
    }
    if (ts.isCallExpression(n) && ts.isPropertyAccessExpression(n.expression) && n.expression.name.text==='component' && ts.isStringLiteral(n.arguments[0]) && ts.isObjectLiteralExpression(n.arguments[1])) {
      const controller=n.arguments[1].properties.find(p=>ts.isPropertyAssignment(p)&&p.name.getText(file)==='controller');
      if(controller){const definition=controller.initializer;const parts=ts.isArrayLiteralExpression(definition)?[...definition.elements]:[definition];const fn=parts.pop();registrations.set(n.arguments[0].text,{kind:'component',dependencies:parts.map(x=>x.text),source:fn.getText(file)});}
    }
    ts.forEachChild(n, visit);
  }
  visit(file);
  for (const statement of file.statements) if(ts.isExpressionStatement(statement)) expression(statement.expression);
  return {source, declarations, registrations, initializers};
}
export function upstream(path, globals = {}) {
  const index = extract(path);
  function denied() {throw Error('upstream network access forbidden');}
  const context = vm.createContext({ VERSION:'2.3.7', TOURNAMENT_MATCH_REQUESTED_TYPE_MARKER:-1415139397, TextEncoder, TextDecoder, performance:{now:()=>0}, window:{performance:{now:()=>0}}, shouldUseTextEncoder:true, isDebug:false, console:{log(){},warn(){},error(){}}, timeLog(){}, fetch:denied, WebSocket:denied, XMLHttpRequest:denied, ...globals }, {codeGeneration:{strings:false,wasm:false}});
  const loaded = new Set(), evidence=[];
  function retry(fn) {
    for (let i=0;i<1000;i++) try {return fn();} catch(e) {
      const name = /^(\w+) is not defined$/.exec(e.message)?.[1];
      if (!name || !index.declarations.has(name) || loaded.has(name)) throw e;
      load(name);
    }
    throw Error('upstream dependency limit');
  }
  function load(name) {
    if (loaded.has(name)) return;
    const code = index.declarations.get(name);
    if (!code) throw Error(`missing pinned declaration ${name}`);
    retry(()=>vm.runInContext(code,context,{filename:`upstream:${name}`,timeout:2000}));
    loaded.add(name);evidence.push({name,sha256:BODY_SHA256});
    for (const code of index.initializers.get(name)??[]) retry(()=>vm.runInContext(code,context,{filename:`upstream:init:${name}`,timeout:2000}));
  }
  function run(code) {return retry(()=>vm.runInContext(code,context,{filename:'harness',timeout:5000}));}
  function factory(name) {
    const registration = index.registrations.get(name);
    if (!registration) throw Error(`missing pinned registration ${name}`);
    evidence.push({name:`service:${name}`,sha256:BODY_SHA256});
    return retry(()=>run(`(${registration.source})`));
  }
  return { ...index, context, loaded, evidence, load, run, retry, factory };
}
