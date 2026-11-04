// Explicit real-model test. Agent HOME/config/auth remain native; only core and sessions are isolated.
import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, isAbsolute } from 'node:path';
import { requireLiveAgentOptIn, executeDesktopCommand, planDesktopCommand } from '../../scripts/desktop-toolchain.ts';
requireLiveAgentOptIn();
const agent=process.env.TF_AGENT_LIVE_AGENT_BIN;
if(!isAbsolute(agent)||!existsSync(agent))throw new Error('TF_AGENT_LIVE_AGENT_BIN must name an existing absolute OMP executable.');
const core=process.env.THOUGHTSFLOW_HERDR_BIN??'herdr-core';
const root=mkdtempSync(join(tmpdir(),'tf-agent-live-'));
const session=`tf-live-${randomUUID().slice(0,8)}`;
const config=join(root,'config'),state=join(root,'state');
const socket=process.platform==='win32'?`\\\\.\\pipe\\${session}`:join('/tmp',`${session}.sock`);
const quote=s=>`'${s.replaceAll("'","'\\''")}'`;
mkdirSync(join(config,'herdr-core','templates'),{recursive:true});mkdirSync(state,{recursive:true});
writeFileSync(join(config,'herdr-core','config.toml'),`socket = ${JSON.stringify(socket)}\n`);
for(const role of ['producer','consumer']){
 mkdirSync(join(root,role));mkdirSync(join(root,`sessions-${role}`));
 writeFileSync(join(root,role,'AGENTS.md'),'This directory is an authorized integration test. Work only in this directory. Do not spawn agents, inspect credentials, change user configuration, install packages, push, publish or access other projects. Use existing native tools and approval policy.\n');
 writeFileSync(join(config,'herdr-core','templates',`live-${role}.md`),`---\ndescription: Live OMP ${role}; inherited model and harness\ncommand: ${quote(agent)} --session-dir ${quote(join(root,`sessions-${role}`))} --max-time 300 --max-cost 2\n---\nComplete only the assigned integration-test task in your working directory. Do not start subagents. Keep the final response concise so the result can be saved from the current screen.\n`);
}
const env={...process.env,TF_AGENT_LIVE_ROOT:root,TF_AGENT_LIVE_SESSION:session,
 THOUGHTSFLOW_HERDR_SESSION:session,THOUGHTSFLOW_HERDR_BIN:core,
 THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR:join(root,'app-data'),XDG_CONFIG_HOME:config,XDG_STATE_HOME:state};
delete env.TF_AGENT_FIXTURE_STATE;delete env.HC_TERMINAL_ID;delete env.HC_SCOPE;delete env.HC_SOCKET;
writeFileSync(join(root,'run.json'),JSON.stringify({session,agent,core,startedAt:new Date().toISOString(),modelOverride:false,approvalOverride:false},null,2));
console.log(`TF_LIVE_ROOT ${root}`);
try{
 process.exitCode=executeDesktopCommand(planDesktopCommand('wdio',{args:['run','wdio.webview.conf.ts','--spec','tests/webview/agents.live.spec.ts'],env}));
}finally{
 // Inspect and stop only this script's isolated core, after retaining observations.
 const listing=spawnSync(core,['ls','--all','--json'],{env,encoding:'utf8',timeout:15000});
 if(listing.status===0){
  try{
   const data=JSON.parse(listing.stdout);const terminals=data.terminals??[];
   writeFileSync(join(root,'cleanup-terminal-list.json'),JSON.stringify(data,null,2));
   for(const t of terminals){
    if(t.labels?.['thoughtsflow.mission'] && t.state==='live')spawnSync(core,['kill',t.address,'--json'],{env,encoding:'utf8',timeout:15000});
   }
  }catch{console.error('Could not parse isolated terminal list; original output retained by runtime.');}
 }
 const stopped=spawnSync(core,['server','stop','--json'],{env,encoding:'utf8',timeout:15000});
 writeFileSync(join(root,'cleanup.json'),JSON.stringify({exitCode:stopped.status,stdout:stopped.stdout,stderr:stopped.stderr},null,2));
 const sessions=[];
 for(const role of ['producer','consumer']){
  const walk=p=>{for(const e of readdirSync(p,{withFileTypes:true})){const file=join(p,e.name);if(e.isDirectory())walk(file);else if(e.name.endsWith('.jsonl')){
   let assistantMessages=0;const models=new Set(),tools=new Set();let tokens=0;
   for(const line of readFileSync(file,'utf8').split('\n').filter(Boolean)){try{const r=JSON.parse(line);const m=r.message;
    if(r.type==='model_change'){const model=r.modelId??r.model;if(typeof model==='string')models.add(r.provider?`${r.provider}/${model}`:model);}
    if(m?.role==='assistant'){assistantMessages++;if(m.model)models.add(`${m.provider??'unknown'}/${m.model}`);tokens+=m.usage?.totalTokens??0;for(const b of m.content??[])if(b.type==='toolCall')tools.add(b.name);}
   }catch{}}
   sessions.push({role,file,assistantMessages,models:[...models],tools:[...tools],tokens,sha256:createHash('sha256').update(readFileSync(file)).digest('hex')});
  }}};walk(join(root,`sessions-${role}`));
 }
 writeFileSync(join(root,'session-summary.json'),JSON.stringify(sessions,null,2));
 console.log(`TF_LIVE_SESSION_SUMMARY ${JSON.stringify(sessions.map(({file,...s})=>s))}`);
 console.log(`TF_LIVE_EVIDENCE_RETAINED ${root}`);
}
