import { upstream, BODY_SHA256 } from './upstream.mjs';

// DOM/canvas host plumbing records no raster pixels and performs no I/O.
function environment() {
  let time=0, nextTimer=1;
  const timers=new Map(), listeners=new Map();
  const noop=()=>{};
  const drawing=new Proxy({measureText:text=>({width:String(text).length*8}),createLinearGradient:()=>({addColorStop:noop}),createRadialGradient:()=>({addColorStop:noop}),getImageData:()=>({data:new Uint8ClampedArray(4)}),canvas:null},{get:(o,k)=>k in o?o[k]:noop});
  function element(tag='div') {
    const e={tagName:tag.toUpperCase(),style:{removeProperty:noop,setProperty:noop},children:[],width:200,height:300,clientWidth:1200,clientHeight:800,offsetWidth:1200,offsetHeight:800,innerHTML:'',textContent:'',classList:{add:noop,remove:noop,toggle:noop,contains:()=>false},getContext:()=>drawing,getBoundingClientRect:()=>({x:0,y:0,left:0,top:0,right:1200,bottom:800,width:1200,height:800}),addEventListener:noop,removeEventListener:noop,setAttribute:noop,removeAttribute:noop,focus:noop,blur:noop,scrollTo:noop,scrollIntoView:noop,scrollTop:0,scrollHeight:800,querySelector:()=>null,querySelectorAll:()=>[],contains:()=>false};
    e.appendChild=c=>{e.children.push(c);c.parentNode=e;return c;}; e.append=(...children)=>children.forEach(c=>e.appendChild(typeof c==='string'?{textContent:c}:c)); e.removeChild=c=>{e.children=e.children.filter(x=>x!==c);c.parentNode=null;};e.remove=()=>e.parentNode?.removeChild(e);e.insertBefore=c=>e.appendChild(c);return e;
  }
  const doc={createElement:element,createTextNode:text=>({textContent:text}),getElementById:()=>null,querySelector:()=>null,querySelectorAll:()=>[],addEventListener:noop,removeEventListener:noop,documentElement:element('html'),body:element('body'),fonts:{ready:Promise.resolve(),check:()=>true,add:noop}};
  const storage=()=>{const m=new Map();return {setItem:(k,v)=>m.set(k,String(v)),getItem:k=>m.get(k)??null,removeItem:k=>m.delete(k),clear:()=>m.clear()};};
  const window={innerWidth:1280,innerHeight:900,devicePixelRatio:1,addEventListener:noop,removeEventListener:noop,performance:{now:()=>time},localStorage:storage(),sessionStorage:storage(),location:{hostname:'127.0.0.1',protocol:'http:',href:'http://127.0.0.1/',reload:noop},document:doc};
  const schedule=(fn,ms=0)=>{const id=nextTimer++;timers.set(id,{fn,at:time+ms,delay:ms});return id;};
  const globals={document:doc,window,location:window.location,navigator:{language:'en-US',userAgent:'offline-original-consumer'},performance:window.performance,setTimeout:schedule,clearTimeout:id=>timers.delete(id),setInterval:()=>0,clearInterval:noop,requestAnimationFrame:fn=>schedule(()=>fn(time),16),cancelAnimationFrame:id=>timers.delete(id),Image:class {constructor(){this.width=200;this.height=300;this.complete=true;}addEventListener(event,fn){if(event==='load')schedule(fn,0);}removeEventListener(){}set src(v){this._src=v;/* no network */}get src(){return this._src;}},errorHandler:{setUserDetails:noop},isLoadingLobby:false,isLoadingGame:false};
  const root={$new(){return this;},$on(event,fn){const list=listeners.get(event)??[];list.push(fn);listeners.set(event,list);return()=>listeners.set(event,list.filter(f=>f!==fn));},$broadcast(event,data){for(const fn of [...listeners.get(event)??[]])fn({name:event},data);},$apply(fn){fn?.();},$digest:noop,safeApply:noop};
  return {globals,root,element,pump(max=500){let count=0;while(count++<max){const entry=[...timers].filter(([,v])=>v.delay<=1000).sort((a,b)=>a[1].at-b[1].at)[0];if(!entry)return;timers.delete(entry[0]);time=entry[1].at;entry[1].fn();}throw Error('offline timer budget exceeded');}};
}

export class OriginalClient {
  constructor(bundle,{send=()=>{},playerId=10,username='Alice'}={}) {
    this.env=environment();this.u=upstream(bundle,{safeDigest:scope=>scope.$digest?.(),ObjectValues:Object.values,...this.env.globals});this.sent=[];this.events=[];this.counter=0;this.services=new Map();this.sendWire=send;
    const {context,load,run,retry}=this.u;
    this.call=code=>retry(()=>run(code));
    // Preload transitive declaration references before invoking stateful callbacks.
    const seen=new Set();
    this.prepare=code=>{for(const token of code.match(/\b[A-Za-z_$][\w$]*\b/g)??[]){if(seen.has(token)||!this.u.declarations.has(token))continue;seen.add(token);load(token);this.prepare(this.u.declarations.get(token));}};
    this.services.set('$rootScope',this.env.root);
    this.services.set('$injector',{get:name=>this.get(name)});
    this.services.set('serverConnection',{send:bytes=>{const data=Buffer.from(bytes);this.sent.push(data);this.sendWire(data);}});
    this.services.set('languageSelection',{hasChangedLanguage:()=>false});
    this.services.set('kingdomService',{updateLastSeenKingdom(){}});
    this.services.set('gameSoundService',{newQuestion(){},gameEndPing(){},notify(){}});
    this.services.set('soundService',{play(){}});
    this.services.set('$scope',this.env.root);this.services.set('debounce',(_id,fn)=>fn?.());this.services.set('$timeout',()=>{});
    for(const name of ['newsService','leaderboardService','autoplayService','playerPageService'])this.services.set(name,{});
    this.prepare(this.u.declarations.get('Reader'));load('Reader');load('LANGUAGE');this.prepare('setCardNameAssociations');
    this.get('serverMessenger');this.get('preferences');this.get('meta');this.get('relations');this.get('tableService');this.get('tables');this.get('sessionService');
    this.get('game');this.get('serverToClientProcessor');this.get('chat');
    this.mountGameArea=()=>{this.get('game').gameArea={reset(){},initialize(){},updateNameInput(){},gameButtonsArea:this.env.element(),gameTabArea:this.env.element(),cardStacksArea:this.env.element(),studyArea:this.env.element(),setStudyCard(){},shouldShowKingdomViewer(){return false;}};};
    this.mountGameArea();
    this.env.root.$on(this.call('Events.GAME_MODEL_CREATED'),()=>this.mountGameArea());
    context.client=this;
    this.call('var game=client.get("game");var meta=client.get("meta");var messenger=client.get("serverMessenger");');
    context.localPlayerId=playerId;context.localUsername=username;
    this.call('meta.model.me.id=localPlayerId;meta.model.me.name=localUsername;meta.playerNames[localPlayerId]=localUsername;');
    this.hash=BODY_SHA256;
  }
  get(name) {
    if(this.services.has(name))return this.services.get(name);
    const r=this.u.registrations.get(name);if(!r)throw Error(`unprovided original service ${name}`);
    this.prepare(r.source);
    const deps=r.dependencies.map(d=>this.get(d));const factory=this.u.factory(name);
    const value=r.kind==='factory'?factory(...deps):new factory(...deps);this.services.set(name,value);return value;
  }
  receive(bytes,{probe=false}={}) {
    const {context}=this.u;
    context.binary=bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
    this.call('var reader=new Reader(binary);');
    while(this.call('reader.hasMessages()')) {
      const counter=this.call('reader.readInt()');if(counter!==this.counter++)throw Error(`counter ${counter} expected ${this.counter-1}`);
      if(probe){const state=this.call('reader.readInt()');if(this.call('reader.hasMessages()'))throw Error('probe has trailing bytes');return {state};}
      const opcode=this.call('reader.readInt()');this.events.push(opcode);context.opcode=opcode;
      this.call('reader.offset-=4;client.get("serverToClientProcessor").process(reader);');
    }
    this.env.pump();
    return this.facts();
  }
  facts() {
    const facts=this.call(`({playerId:meta.getMyId(),tableId:meta.getMyTableId(),hasTable:client.get("tableService").hasTable(),tables:client.get("tables").getRawSummaries().map(t=>t.tableId),gameId:game.displayedGameId,players:game.state?game.state.players.map(p=>({id:p.playerId,index:p.index,name:p.name,isMe:p.isMe,counters:p.counters.map(c=>({kind:getOrdinal(CounterNames,c.counterName),name:Object.keys(CounterNames)[getOrdinal(CounterNames,c.counterName)],value:c.value})),reminders:p.activeReminders.map(r=>({name:r.cardName.name,count:r.frequency,blinking:r.blinking})),projects:p.boughtProjects.map(c=>c.name)})):[],cards:game.state?game.state.cards.map(c=>({id:c.id,name:c.cardName.name,question:c.question?c.question.questionIndex:null,answer:c.answer,stacks:c.cardStacks.length})):[],zones:game.state?game.state.zones.map(z=>({index:z.index,kind:z.zoneName,kindOrdinal:ZoneNames.indexOf(z.constructor),owner:z.owner.playerId,visible:z.isVisible,cards:z.cardStacks.map(s=>({ids:s.cards.map(c=>c.id),anonymous:s.anonymousCards}))})):[],questions:game.questionModel?game.questionModel.openQuestions.map(function describe(q){return {index:q.questionIndex,type:getOrdinal(QuestionTypes,q.type),story:getOrdinal(QuestionIds,q.story.questionId),min:q.min,max:q.max,decline:q.declineButtonId,answer:q.getAnswer(),associated:q.associatedCards.map(c=>c.id),text:(new LogLineParser(game.state,game.state.players,true)).parseStory(q.story).map(t=>t.text).join(''),children:q.subquestions?q.subquestions.map(describe):[],targetNames:q.content?q.content.elements.map(e=>q.content instanceof QuestionCardElements?game.state.cards[e].cardName.name:q.content instanceof QuestionBuyableCardElements?game.state.cards[e.id].cardName.name:q.content instanceof QuestionCardNameElements?e.name:q.content instanceof QuestionCardWithExplanationElements?game.state.cards[e.id].cardName.name:null):[],targets:q.content?q.content.elements.map(e=>typeof e==='number'?e:e.id!==undefined?{id:e.id,simple:e.isSimple}:e.association!==undefined?{association:e.association,name:getOrdinal(AbilityNames,e.name),label:e.label}:Object.keys(CardModes).includes(String(e))?String(e):e.toString()):[]};}):[],buttons:game.questionModel?game.questionModel.gameButtons.map(b=>({kind:b instanceof ShortcutButtonView?'shortcut':b instanceof AbilityButtonView?'ability':b instanceof NumericButtonView?'number':b instanceof DoneGameButtonView?'done':b instanceof DeclineGameButtonView?'decline':'button',text:b.text,answer:b.answer,visible:b.visibility,enabled:b.enabled})):[],chat:client.get('chat').chatLines.map(l=>({sender:l.sender,text:l.message})),animationQueue:game.animationDirector?game.animationDirector.queueElements.length:0,score:game.metaGameModel&&game.metaGameModel.gameResult?game.metaGameModel.gameResult.playerResults.map(p=>({playerId:p.playerId,rank:p.rank,totalPoints:p.score.totalPoints,usedTurns:p.score.usedTurns,resigned:p.resignIndex!==-1,parts:p.score.parts.map(s=>({card:s.cardName.name,points:s.points,count:s.frequency}))})):null})`);
    return JSON.parse(JSON.stringify(facts));
  }
  command(method,args=[]) {
    if(!/^[A-Za-z]\w*$/.test(method))throw Error('invalid producer method');
    this.u.context.args=args;this.u.context.method=method;
    this.call('messenger[method](...args)');
  }
  newTableDefault() {this.get('LobbyController');this.call('client.get("LobbyController").clickTab(LobbyTabs.NEW_TABLE)');}
  chat(message) {this.u.context.chatText=message;this.call('client.get("chat").message=chatText;client.get("chat").sendToTable()');}
  newTable(rules=[],change=false) {
    this.u.context.ruleSpecs=rules;
    this.call(`var rules=ruleSpecs.map(r=>{var id=typeof r.id==='string'?TableRuleIds[r.id]:getByOrdinal(TableRuleIds,r.id);var value=r.value;var args=r.arguments;
    if(id===TableRuleIds.REQUIRED_CARDS)args=(r.cards??args??[]).map(k=>new RequiredCard(CardNames[k]));
    if(id===TableRuleIds.USED_EXPANSIONS)args=(r.expansions??args??[]).map(k=>Expansions[k]);
    return new TableRule(id,value,args);});`);
    this.call(change?'rules.forEach(r=>messenger.changeTableRule(meta.getMyTableId(),r))':'messenger.newTableRequest(rules)');
  }
  autoAnswer() {
    this.call(`var q=game.questionModel.openQuestions[0];if(!q)throw Error('no question');
    while(q instanceof ComplexQuestion){if(q.type===QuestionTypes.COMPLEX_OR){var branch=q.subquestions.find(s=>s.content&&s.content.elements.length>0)??q.subquestions[0];q=branch;}else {q.subquestions.forEach(s=>{if(s instanceof ChoiceQuestion)for(var i=0;i<(s.min??0);i++)s.accumulateAnswer(i);});q.send();q=null;break;}}
    if(q){if(q instanceof NumberQuestion)q.answer(q.defaultAnswer);else if(q instanceof NameQuestion)q.answer(getOrdinal(CardNames,CardNames.COPPER));else if(q.type.isOrder)q.send();else if(q instanceof ChoiceQuestion){var indices=[];if(q.type===QuestionTypes.BUY){var names=['PROVINCE','GOLD','SILVER','DUCHY','ESTATE'];for(var key of names){var i=q.content.elements.findIndex(e=>game.state.cards[typeof e==='number'?e:e.id].cardName===CardNames[key]);if(i>=0){indices=[i];break;}}}else indices=Array.from({length:Math.max(q.min,Math.min(q.max,1))},(_,i)=>i);if(indices.length===0)q.answerDecline();else{indices.forEach(i=>q.answer(i));if(game.questionModel.openQuestions.length)q.send();}}else q.send();}`);
  }
  answer(action={}) {
    this.u.context.action=action;
    return this.call(`var q=game.questionModel.openQuestions[0];if(!q)throw Error("no original question");for(var branch of action.path??[])q=q.subquestions[branch];
    if(action.decline)q.answerDecline();else if(action.button!==undefined)q.buttons[action.button].onClick({offsetX:0,offsetY:0});else if(action.name!==undefined){var input=client.get('nameInput');input.question=q;input.entry=CardNames[action.name].name;if(!input.getWindowCards(input.entry).includes(CardNames[action.name]))throw Error('name missing from original name input');input.name(CardNames[action.name]);}else if(action.number!==undefined)q.answer(action.number);else if(action.send)q.send();else if(action.order){var zone=q.associatedCards[0].cardStacks.find(s=>s.location instanceof RevealZone).location;zone.cardStacks=action.order.map(i=>zone.cardStacks.find(s=>s.topCard.id===q.content.elements[i]));zone.enableOrdering(q);q.send();}else {for(var value of action.indices??[0])q.answer(value);if(game.questionModel.openQuestions.length&&action.done!==false)q.send();}`);
  }
}
