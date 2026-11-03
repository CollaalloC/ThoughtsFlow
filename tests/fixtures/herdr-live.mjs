// Opt-in, installed-core contract smoke. Isolated state/config, no Agent or model.
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import readline from 'node:readline';
import assert from 'node:assert/strict';
if(process.env.TF_HERDR_LIVE!=='1')throw new Error('Set TF_HERDR_LIVE=1 to test the installed core in an isolated session (no model).');
const dir=mkdtempSync(join(tmpdir(),'tf-hc-'));
const session=`tf-${randomUUID().slice(0,8)}`;
const socket=process.platform==='win32'?`\\\\.\\pipe\\${session}`:join('/tmp',`${session}.sock`);
const env={...process.env,HC_SESSION:session,HC_SOCKET:socket,XDG_CONFIG_HOME:join(dir,'config'),XDG_STATE_HOME:join(dir,'state')};
delete env.HC_TERMINAL_ID;delete env.HC_SCOPE;
const binary=process.env.THOUGHTSFLOW_HERDR_BIN??'herdr-core';
const quote=s=>`'${s.replaceAll("'","'\\''")}'`;
mkdirSync(join(dir,'config','herdr-core','templates'),{recursive:true});
const program=join(dir,'fixture.mjs');
writeFileSync(program,`import readline from 'node:readline'; const status=s=>process.stdout.write('\\x1b]7501;state='+s+':app=fixture\\x1b\\\\');status('idle');readline.createInterface({input:process.stdin}).on('line',s=>{status('working');process.stdout.write('OBSERVED '+s+'\\n');status('done');});`);
writeFileSync(join(dir,'config','herdr-core','templates','fixture.md'),`---\ndescription: Fixture without a model\ncommand: ${quote(process.execPath)} ${quote(program)}\n---\n`);
const child=spawn(binary,['connect'],{env,stdio:['pipe','pipe','pipe']});
let stderr='';child.stderr.on('data',d=>{stderr=(stderr+d).slice(-2000);});
const rows=readline.createInterface({input:child.stdout})[Symbol.asyncIterator]();let id=0;let terminal;let stopped=false;
const call=async(method,params={})=>{
 child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:++id,method,params})+'\n');
 let timer;try{
  const next=await Promise.race([rows.next(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(`RPC timeout ${method}`)),60000);})]);
  assert.equal(next.done,false,stderr);const r=JSON.parse(next.value);assert.equal(r.id,id);assert.ok(!r.error,JSON.stringify(r.error));return r.result;
 }finally{clearTimeout(timer);}
};
try{
 const hello=await call('hello',{generation:1});assert.equal(hello.generation,1);
 const result=await call('terminal.run',{argv:['fixture','literal $(echo untouched)'],cwd:dir,no_parent:true,keep:false,wait:false,labels:{'thoughtsflow.test':session}});
 terminal=result.address;assert.equal(result.first_message.sent,true);assert.equal(result.template,'fixture');
 const read=await call('terminal.read',{terminal,lines:0});assert.ok(read.text.includes('literal $(echo untouched)'));
 const end=await call('terminal.kill',{terminal});assert.equal(end.state,'finished');
 assert.equal((await call('terminal.rm',{terminal})).removed,terminal);terminal=undefined;
 console.log(JSON.stringify({generation:hello.generation,version:hello.version,core:hello.core.id,firstMessageSent:true,screenObserved:true,modelCalled:false}));
}finally{
 // This endpoint was created by this script; never touch a shared user core.
 if(terminal) {try{await call('terminal.kill',{terminal});await call('terminal.rm',{terminal});}catch{}}
 try{await call('server.stop');stopped=true;}catch{}
 child.stdin.end();child.kill();
 if(stopped)rmSync(dir,{recursive:true,force:true});
 else console.error(`Retained isolated test state for inspection: ${dir}`);
}
