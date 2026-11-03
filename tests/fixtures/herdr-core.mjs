#!/usr/bin/env node
// Deterministic generation-1 RPC fixture; never invokes an Agent or shell.
import { readFileSync, writeFileSync, appendFileSync, existsSync } from 'node:fs';
import readline from 'node:readline';
const path=process.env.TF_AGENT_FIXTURE_STATE;
if(!path){process.stderr.write('Fixture requires an isolated state path\n');process.exit(2);}
if(process.argv.includes('--version')) { console.log('hc 0.1.17-fixture');process.exit(0); }
if(!process.argv.includes('connect'))process.exit(2);
const methods=['hello','templates.list','terminal.run','terminal.list','terminal.get','terminal.read','terminal.send','terminal.rm'];
const load=()=>existsSync(path)?JSON.parse(readFileSync(path,'utf8')):{next:1,terminals:[]};
readline.createInterface({input:process.stdin}).on('line',line=>{
 let q;try{q=JSON.parse(line);}catch{process.exitCode=1;return;}
 appendFileSync(`${path}.calls.jsonl`,JSON.stringify(q)+'\n');
 try {
  if(existsSync(`${path}.offline`))throw new Error('Fixture core intentionally offline');
  const s=load(), p=q.params??{};let result;
  const terminal=()=>{const t=s.terminals.find(t=>t.address===p.terminal);if(!t)throw new Error('not found');return t;};
  switch(q.method){
   case 'hello':result={generation:1,version:'0.1.17-fixture',core:{id:'fixture-core',name:'Isolated fixture'},methods,features:[],framed:false};break;
   case 'templates.list':result={templates:[{name:'reviewer',description:'Fixture review',command:'fixture'},{name:'writer',description:'Fixture implementation',command:'fixture'}]};break;
   case 'terminal.run': {
    if(p.line||!Array.isArray(p.argv)||p.argv.length!==2||!['reviewer','writer'].includes(p.argv[0])||p.keep!==false||p.no_parent!==true)throw new Error('invalid launch');
    const t={address:`t${s.next++}`,name:p.name,labels:p.labels,argv:['fixture'],cwd:p.cwd,template:p.argv[0],keep:false,created_ms:Date.now(),state:'live',status:{state:'idle',app:'fixture'},turn:0,screen:'exact'};s.terminals.push(t);
    result={...t,first_message:{sent:true,started:true,turn:0}};break;
   }
   case 'terminal.list':result={terminals:s.terminals.filter(t=>Object.entries(p.labels??{}).every(([k,v])=>t.labels[k]===v)),hidden_finished:0};break;
   case 'terminal.get':result=terminal();break;
   case 'terminal.read':terminal();if(p.lines!==0)throw new Error('passive screen only');result={text:'Herdr fixture output; no model was called',complete:true,how:'screen'};break;
   case 'terminal.send':{const t=terminal();if(p.force||p.if!=='idle'||t.status?.state!=='idle')throw new Error('blocked');t.state='finished';t.status=null;t.turn++;result={status:'idle',turn:0,started:true};break;}
   case 'terminal.rm':{const t=terminal();if(t.state!=='finished')throw new Error('live');s.terminals=s.terminals.filter(x=>x!==t);result={removed:t.address};break;}
   default:throw new Error('unsupported method');
  }
  writeFileSync(path,JSON.stringify(s));process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:q.id,result})+'\n');
 }catch(e){process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:q.id,error:{code:-32000,message:e.message,data:{kind:'unavailable'}}})+'\n');}
});
