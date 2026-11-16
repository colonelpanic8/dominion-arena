#!/usr/bin/env node
// Inventory only: never fetch missing assets or invent an official application shell.
import{readFileSync,readdirSync,statSync,existsSync,mkdirSync,writeFileSync}from'node:fs';
import{resolve,join,relative}from'node:path';
import{createRequire}from'node:module';
import{extract,BODY_SHA256}from'./upstream.mjs';
const ts=createRequire(import.meta.url)('typescript');
const args={};for(let i=2;i<process.argv.length;i+=2)args[process.argv[i].slice(2)]=process.argv[i+1];
if(!args.bundle||!args.cache||!args.output)throw Error('usage: assets.mjs --bundle BODY --cache ASSETS_DIR --output DIR [--application ORIGINAL_APPLICATION_DIR]');
const index=extract(args.bundle),cache=resolve(args.cache),app=args.application?resolve(args.application):cache;
function files(root){const out=[];for(const e of readdirSync(root,{withFileTypes:true})){const p=join(root,e.name);if(e.isDirectory())out.push(...files(p));else if(e.isFile())out.push(p);}return out;}
const cached=files(cache),application=app===cache?cached:files(app);
const refs=new Set(),dynamic=new Set(),templates=new Set();
const sf=ts.createSourceFile(args.bundle,index.source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
function walk(n){
 if(ts.isStringLiteralLike(n)){
  const v=n.text;
  if(/^(?:images|sounds|fonts|templates|html|css|js)\//.test(v)){
   if(/\.(?:webp|png|jpg|jpeg|svg|gif|mp3|ogg|woff2?|ttf|otf|html|css|js)$/.test(v))refs.add(v);else dynamic.add(v);
  }
  if(ts.isPropertyAssignment(n.parent)&&n.parent.name?.getText(sf)==='templateUrl')templates.add(v);
 }
 ts.forEachChild(n,walk);
}
walk(sf);
let manifest;const manifestFile=join(cache,'manifest.json');if(existsSync(manifestFile))manifest=JSON.parse(readFileSync(manifestFile,'utf8'));
function strings(v){if(typeof v==='string')return [v];if(Array.isArray(v))return v.flatMap(strings);if(v&&typeof v==='object')return Object.values(v).flatMap(strings);return [];}
const manifestRefs=[...new Set(strings(manifest).filter(v=>/^(images|fonts|sounds)\//.test(v)))];
const missingManifest=manifestRefs.filter(v=>!existsSync(join(cache,v)));
const missingReferences=[...refs].sort().filter(v=>!existsSync(join(app,v))&&!existsSync(join(cache,v)));
const headName='js/dominion-webclient-head-2.3.7.min.js';
const requirements=[{component:'original HTML/bootstrap',path:'index.html',found:application.some(p=>p.endsWith('/index.html')&&/dominion-webclient|ng-app=["']webclient/.test(readFileSync(p,'utf8')))},{component:'hash-pinned original head bundle',path:headName,found:application.some(p=>/dominion-webclient-head-2\.3\.7(?:\.min)?\.js$/.test(p))},{component:'Angular runtime',path:'Angular runtime referenced by original HTML/head',found:application.some(p=>/angular[^/]*\.js$/.test(p))},{component:'official stylesheets',path:'stylesheet references from original HTML',found:application.some(p=>p.endsWith('.css'))},{component:'Angular HTML templates',path:'templateUrl references from original body',found:templates.size>0&&[...templates].every(p=>existsSync(join(app,p)))}];
const report={bundleSha256:BODY_SHA256,cache,application:app,manifestSchema:manifest?.schema,manifestVersion:manifest?.clientVersion,cachedFiles:cached.length,counts:{fonts:cached.filter(p=>/\.(otf|ttf|woff2?)$/.test(p)).length,images:cached.filter(p=>/\.(webp|png|jpg|svg)$/.test(p)).length,audio:cached.filter(p=>/\.(ogg|mp3)$/.test(p)).length},graphicalAcceptance:false,requirements,missingComponents:requirements.filter(r=>!r.found),missingManifestPaths:missingManifest,missingLiteralReferences:missingReferences,templateReferences:[...templates].sort(),dynamicPathPrefixes:[...dynamic].sort(),cachedRelativePaths:cached.map(p=>relative(cache,p)).sort(),languages:{embeddedTables:[...index.declarations.keys()].filter(n=>/^(English|German|Russian|French|Japanese|TChinese|Spanish)(CardNames|Phrases|QuestionDescriptions|AbilityNames)$/.test(n)),consumerLanguage:'English original tables'},headGlobals:{suppliedForExtractedConsumer:['VERSION=2.3.7','TOURNAMENT_MATCH_REQUESTED_TYPE_MARKER=-1415139397','ObjectValues=Object.values','safeDigest(scope) delegates to simulated Angular digest','loopback endpoint supplied by operator'],productionEndpointBootstrap:'absent; no production constants executed'},notes:['Manifest is the native asset-cache schema, not a complete official application distribution.','Exact literal paths and absent shell components are enumerated; dynamic path prefixes require a complete upstream HTML/head distribution to resolve.','Original model/service acceptance uses simulated image completions and canvas plumbing; it is independent of graphical acceptance.']};
mkdirSync(args.output,{recursive:true});const output=join(args.output,'assets-report.json');writeFileSync(output,JSON.stringify(report,null,2)+'\n');console.log(`Asset inventory: ${cached.length} cached files; ${report.missingComponents.length} missing shell components, ${missingReferences.length} missing literal paths; graphical acceptance unavailable. ${output}`);
