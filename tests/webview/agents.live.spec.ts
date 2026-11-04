import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { $, browser, expect } from '@wdio/globals';
import { button, controlByLabel, createAgentWorkspace, focusMainWindow, pinMainWindow, selectOptionByText, waitForBodyText } from './support';
import type { AgentEvidence, AgentHandoff, AgentOperation } from '../../src/shared/contracts/agents';
async function invoke<T>(command:string,args:Record<string,unknown>):Promise<T>{
 const reply=await browser.executeAsync((cmd:string,input:Record<string,unknown>,done:(r:unknown)=>void)=>{
  const api=(window as unknown as {__TAURI_INTERNALS__:{invoke:(cmd:string,args:Record<string,unknown>)=>Promise<unknown>}}).__TAURI_INTERNALS__;
  void api.invoke(cmd,input).then(r=>done({ok:true,result:r}),e=>done({ok:false,error:e}));
 },command,args) as {ok:boolean;result:{data:T};error:unknown};
 if(!reply.ok)throw new Error(JSON.stringify(reply.error));return reply.result.data;
}

describe('Explicit opt-in: real OMP through herdr-core and native ThoughtsFlow',()=>{
 it('inherits native configuration and reuses selected context and a saved observation across agents',async()=>{
  const root=process.env.TF_AGENT_LIVE_ROOT;
  if(process.env.TF_AGENT_LIVE!=='1'||!root||process.env.TF_AGENT_FIXTURE_STATE)throw new Error('Explicit isolated real-agent runner required');
  const marker=`TF_CTX_${randomUUID()}`;
  await pinMainWindow();
  await createAgentWorkspace({name:'真实 Herdr 工作流',goal:'验证配置继承、上下文交接、文件执行与证据复用'});
  const workspaceId=await (await $('[aria-label="本地工作区"]')).getValue();
  await invoke('update_workspace',{input:{id:workspaceId,systemPrompt:`Integration-test reference data: context_code=${marker}; operands=[11,17,23]. These facts are supplied only through selected ThoughtsFlow context.`}});
  await (await button('连接 herdr-core')).click();
  await waitForBodyText('已连接');
  const report:{workspaceId:string;marker:string;stages:unknown[];evidence?:AgentEvidence}={workspaceId,marker,stages:[]};
  let saved:AgentEvidence|undefined;
  for(const role of ['producer','consumer'] as const){
   console.log(`TF_LIVE_STAGE ${role} preparing`);
   await (await controlByLabel('工作目录')).setValue(join(root,role));
   await (await controlByLabel('协作目标','textarea')).setValue(`真实 ${role} ${marker.slice(-8)}`);
   await (await button('创建协作')).click();
   await waitForBodyText('已连接 · 每 5 秒更新进展');
   const missionId=await (await $('[aria-label="历史协作"]')).getValue();
   await (await controlByLabel('任务标题')).setValue(`真实 ${role}`);
   const spec=role==='producer'
    ? 'Read context_code and operands only from the explicitly selected ThoughtsFlow context below. Use native tools to calculate their sum and generate a fresh random UUID nonce. Write artifact.json in your CURRENT working directory with JSON keys context_code, nonce, sum. Verify the file by reading it. Do not inspect any other directories, credentials, user settings, or launch subagents. In your final response print exactly one short line beginning TF_RESULT_A followed by the complete artifact JSON. Do not exit your Agent.'
    : 'Use ONLY the Saved Agent output observation explicitly provided below as the source of the previous result. Do not search session files, other directories or previous artifacts. Extract its TF_RESULT_A JSON and its Evidence and SHA256 headers. Use native tools to write and read back artifact.json in your CURRENT directory with keys context_code, source_nonce, doubled_sum, source_evidence_id, source_content_hash. doubled_sum is twice the saved sum. Do not change settings or start subagents. Finish with one short TF_RESULT_B line containing the JSON. Do not exit your Agent.';
   await (await controlByLabel('任务说明','textarea')).setValue(spec);
   await selectOptionByText(await controlByLabel('Agent 模板','select'),`live-${role} · Live OMP ${role}; inherited model and harness`);
   if(role==='producer')await (await $('input[type="checkbox"]')).click();
   else await (await $(`input[data-evidence-id="${saved!.id}"]`)).click();
   await (await button('预览交接内容')).click();
   const preview=await $('[aria-label="交接预览"]');await preview.waitForDisplayed();
   const text=await (await preview.$('pre')).getText();
   if(role==='producer')expect(text).toContain(marker);
   else{expect(text).toContain(saved!.id);expect(text).toContain(saved!.contentHash);expect(text).toContain('TF_RESULT_A');}
   await (await button('启动 Agent 任务')).click();
   // Do not repeat a click or task when delivery is uncertain.
   await browser.waitUntil(async()=>{
    const ops=await invoke<AgentOperation[]>('agent_operations',{missionId});
    return ops.some(o=>o.kind==='start-task'&&o.status!=='pending');
   },{timeout:85000,interval:1000,timeoutMsg:'No durable launch outcome; inspect the retained DB, never redispatch'});
   const operations=await invoke<AgentOperation[]>('agent_operations',{missionId});
   const op=operations.find(o=>o.kind==='start-task')!;
   writeFileSync(join(root,`${role}-launch.json`),JSON.stringify(op,null,2));
   expect(op.status).toBe('succeeded');
   const handoff=await invoke<AgentHandoff>('agent_get_handoff',{input:{missionId,operationId:op.id}});
   expect(handoff.preview.text).toBe(text);
   console.log(`TF_LIVE_STAGE ${role} dispatched ${op.id}`);
   await focusMainWindow();
   const task=await $(`[aria-label="任务 真实 ${role}"]`);await task.waitForDisplayed();
   const artifact=join(root,role,'artifact.json');
   await browser.waitUntil(async()=>{
    const label=await task.getText();
    if(label.includes('Agent 报告错误')||label.includes('等待处理'))throw new Error(`Native Agent requires attention: ${label}`);
    return existsSync(artifact)&&(label.includes('Agent 报告完成')||label.includes('空闲'));
   },{timeout:250000,interval:1000,timeoutMsg:'Native task did not finish; retained receipt/session required for diagnosis'});
   const actual=JSON.parse(readFileSync(artifact,'utf8'));
   expect(actual.context_code).toBe(marker);
   if(role==='producer'){expect(actual.sum).toBe(51);expect(actual.nonce).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i);}
   else{
    const original=JSON.parse(readFileSync(join(root,'producer','artifact.json'),'utf8'));
    expect(actual.source_nonce).toBe(original.nonce);expect(actual.doubled_sum).toBe(102);
    expect(actual.source_evidence_id).toBe(saved!.id);expect(actual.source_content_hash).toBe(saved!.contentHash);
    expect(handoff.preview.evidenceIds).toEqual([saved!.id]);
   }
   await (await task.$('details.agent-workspace__output > summary')).click();
   await (await button('读取输出')).click();
   await expect(await task.$('pre')).toHaveText(expect.stringContaining(role==='producer'?'TF_RESULT_A':'TF_RESULT_B'));
   await (await button('保存当前页观察（重新读取）')).click();
   await waitForBodyText('输出观察已保存，可在下一任务中选择。');
   const evidence=await invoke<AgentEvidence[]>('agent_evidence',{workspaceId});
   const current=evidence.find(e=>e.handoffOperationId===op.id)!;expect(current).toBeDefined();
   if(role==='producer'){saved=current;report.evidence=current;}
   report.stages.push({role,missionId,operation:op,handoff,artifact:actual,evidence:current});
   writeFileSync(join(root,'workflow-evidence.json'),JSON.stringify(report,null,2));
   console.log(`TF_LIVE_STAGE ${role} artifact-and-evidence verified`);
  }
  await browser.refresh();await pinMainWindow();
  const stored=await invoke<AgentEvidence>('agent_get_evidence',{input:{workspaceId,id:saved!.id}});
  expect(stored.contentHash).toBe(saved!.contentHash);expect(stored.text).toBe(saved!.text);
  await browser.saveScreenshot(join(root,'workflow.png'));
  writeFileSync(join(root,'passed.json'),JSON.stringify({passed:true,workspaceId,producerEvidence:saved!.id,modelCalls:'verified separately from native session logs'},null,2));
 });
});
