import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import readline from 'node:readline';
test('native JSON-RPC preserves literal task text and separates submission, lifecycle, and removal',async()=>{
 const dir=mkdtempSync(join(tmpdir(),'tf-hc-fixture-'));
 const child=spawn(process.execPath,['tests/fixtures/herdr-core.mjs','connect'],{env:{...process.env,TF_AGENT_FIXTURE_STATE:join(dir,'state.json')},stdio:['pipe','pipe','pipe']});
 const lines=readline.createInterface({input:child.stdout})[Symbol.asyncIterator]();let id=0;
 const call=async(method,params={})=>{child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:++id,method,params})+'\n');const next=await lines.next();assert.equal(next.done,false);const r=JSON.parse(next.value);assert.equal(r.id,id);return r;};
 try{
  const hello=await call('hello',{generation:1});assert.equal(hello.result.generation,1);
  const run=await call('terminal.run',{argv:['reviewer','literal $(echo x)'],cwd:dir,no_parent:true,keep:false,labels:{'thoughtsflow.mission':'mission'}});
  assert.equal(run.result.first_message.sent,true);assert.equal(run.result.state,'live');
  const terminal=run.result.address;
  assert.ok((await call('terminal.rm',{terminal})).error);
  assert.equal((await call('terminal.read',{terminal,lines:0})).result.how,'screen');
  assert.ok((await call('terminal.send',{terminal,text:'no force',force:true})).error);
  await call('terminal.send',{terminal,text:'reviewed',force:false,if:'idle'});
  assert.equal((await call('terminal.get',{terminal})).result.state,'finished');
  assert.equal((await call('terminal.rm',{terminal})).result.removed,terminal);
 }finally{child.stdin.end();await new Promise(resolve=>child.on('exit',resolve));rmSync(dir,{recursive:true,force:true});}
});
