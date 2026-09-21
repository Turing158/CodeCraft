import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, rename, rm } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
const base=resolve('src-tauri/trae-core/target');
const root=join(base,'process-tests',randomUUID());
const exe=join(base,'debug/examples/bridge_test_host.exe');
await mkdir(root,{recursive:true});
const children=[];
function child(mode) {
  const p=spawn(exe,[mode,root],{windowsHide:true,stdio:['pipe','pipe','pipe']});children.push(p);
  p.errors='';p.stderr.on('data',d=>p.errors+=d);p.messages=[];createInterface({input:p.stdout}).on('line',line=>{p.messages.push(JSON.parse(line));});
  p.send=v=>p.stdin.write(JSON.stringify(v)+'\n');
  return p;
}
async function until(fn,ms=8000) { const end=Date.now()+ms;while(Date.now()<end){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,30));}throw new Error('Timed out'); }
const read=async path=>{try{return JSON.parse(await readFile(path,'utf8'));}catch{return null;}};
let epoch;
async function command(command,commandId=randomUUID()) {
  const messageId=randomUUID(), target=join(root,'inbox',messageId+'.json'), temp=target+'.tmp';
  await writeFile(temp,JSON.stringify({schemaVersion:1,appEpoch:epoch,messageId,commandId,command}));await rename(temp,target);
  const reply=await until(()=>read(join(root,'replies',commandId+'.json')));
  return reply;
}
const q={schemaVersion:1,questions:[{questionId:'q',prompt:'Choose',kind:'single',options:[{optionId:'a',label:'Same',description:null},{optionId:'b',label:'Same',description:null}],required:true,allowText:false,minSelections:1,maxSelections:1}]};
async function bind(call) {
  const reply=await command({kind:'hook',identity:{installationId:'synthetic',traeInstanceId:'instance'},input:{hook_event_name:'PreToolUse',session_id:'session',cwd:root,workspace_roots:[root],tool_use_id:call,tool_name:'mcp__codecraft__codecraft_ask_user',llm_tool_name:'codecraft_ask_user',tool_input:q}});
  assert.equal(reply.error,null);return reply.result.output.hookSpecificOutput.updatedInput;
}
async function pending() { return until(async()=>{const s=await read(join(root,'state.json'));return s&&Object.values(s.requests).find(r=>r.state==='pending');}); }
async function decide(r) { const result=await command({kind:'decision',body:{schemaVersion:1,decisionId:randomUUID(),target:r.target,action:{kind:'question',answers:[{questionId:'q',status:'answered',selectedOptionIds:['b'],text:null}]}}});assert.equal(result.error,null); }
async function initialize(p) {p.send({jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'synthetic',version:'1'}}});await until(()=>p.messages.find(m=>m.id===1));p.send({jsonrpc:'2.0',method:'notifications/initialized'});}
let passed=0;
try {
  const store=child('store');epoch=(await until(()=>read(join(root,'heartbeat.json')))).appEpoch;
  const event=async (hook_event_name,extra={})=>command({kind:'hook',identity:{installationId:'synthetic',traeInstanceId:'instance'},input:{hook_event_name,session_id:'native-only',cwd:root,workspace_roots:[root],...extra}});
  const native=await event('PreToolUse',{tool_use_id:'question',tool_name:'AskUserQuestion',llm_tool_name:'AskUserQuestion',tool_input:{questions:[{question:'Which?',options:[{label:'A'}]}]}});
  assert.deepEqual(native.result.output,{});
  const nativeSession=Object.values((await read(join(root,'state.json'))).sessions).find(s=>s.sessionId==='native-only');
  assert.equal(nativeSession.nativeInteractions[0].kind,'question');passed++;
  await event('Notification',{notification_type:'ask_user_question',tool_use_id:'question',message:'Waiting'});
  const repeated=Object.values((await read(join(root,'state.json'))).sessions).find(s=>s.sessionId==='native-only');
  assert.equal(repeated.nativeInteractions.length,1);assert.equal(repeated.nativeInteractions[0].id,nativeSession.nativeInteractions[0].id);passed++;
  await event('Stop',{last_assistant_message:'done'});
  assert.equal(Object.values((await read(join(root,'state.json'))).sessions).find(s=>s.sessionId==='native-only').nativeInteractions.length,0);passed++;
  const chatFixture=JSON.parse(await readFile('protocol/trae/3.3.102/fixtures/workspace-less-chat.json','utf8'));
  const chatInput=chatFixture.input;
  const chatEvent=input=>command({kind:'hook',identity:{installationId:'synthetic',traeInstanceId:'instance'},input});
  const chatReply=await chatEvent(chatInput);assert.equal(chatReply.error,null);assert.deepEqual(chatReply.result.output,{});
  const chatSession=Object.values((await read(join(root,'state.json'))).sessions).find(s=>s.sessionId===chatInput.session_id);
  assert.equal(chatSession.cwd,'');assert.deepEqual(chatSession.workspaceRoots,[]);
  await chatEvent({...chatInput,hook_event_name:'Notification',notification_type:'idle_prompt',message:'Agent has completed the task'});
  assert.equal(Object.values((await read(join(root,'state.json'))).sessions).find(s=>s.sessionId===chatInput.session_id).status,'idle');passed++;
  const chatTool=await chatEvent({...chatInput,hook_event_name:'PreToolUse',tool_use_id:'unscoped-tool',tool_name:'RunCommand',llm_tool_name:'RunCommand',tool_input:{command:'echo test'}});
  assert.equal(chatTool.error.error.code,'INVALID_ARGUMENT');passed++;
  const p=child('mcp');await initialize(p);p.send({jsonrpc:'2.0',id:2,method:'tools/list',params:{}});
  const listed=await until(()=>p.messages.find(m=>m.id===2));assert.deepEqual(listed.result.tools.map(t=>t.name),['codecraft_ask_user','codecraft_review_plan']);passed++;
  p.send({jsonrpc:'2.0',id:3,method:'tools/call',params:{name:'codecraft_ask_user',arguments:q}});
  assert.equal((await until(()=>p.messages.find(m=>m.id===3))).result.structuredContent.error.code,'TICKET_INVALID');passed++;
  const args=await bind('call-one');p.send({jsonrpc:'2.0',id:4,method:'tools/call',params:{name:'codecraft_ask_user',arguments:args}});
  const r=await pending();await decide(r);const response=await until(()=>p.messages.find(m=>m.id===4));assert.equal(response.result.structuredContent.payload.answers[0].selectedOptionIds[0],'b');
  await until(async()=> (await read(join(root,'state.json')))?.requests[r.target.requestId].state==='delivered');passed++;
  p.send({jsonrpc:'2.0',id:5,method:'tools/call',params:{name:'codecraft_ask_user',arguments:args}});assert.equal((await until(()=>p.messages.find(m=>m.id===5))).result.structuredContent.operationId,r.target.requestId);passed++;
  const args2=await bind('call-two');p.send({jsonrpc:'2.0',id:6,method:'tools/call',params:{name:'codecraft_ask_user',arguments:args2}});const r2=await pending();
  p.send({jsonrpc:'2.0',id:7,method:'tools/call',params:{name:'codecraft_ask_user',arguments:args2}});
  await new Promise(r=>setTimeout(r,300));p.send({jsonrpc:'2.0',method:'notifications/cancelled',params:{requestId:7,reason:'subscriber cancelled'}});await new Promise(r=>setTimeout(r,300));
  assert.equal((await read(join(root,'state.json'))).requests[r2.target.requestId].state,'pending');await decide(r2);await until(()=>p.messages.find(m=>m.id===6));passed++;
  const args3=await bind('call-three');p.send({jsonrpc:'2.0',id:8,method:'tools/call',params:{name:'codecraft_ask_user',arguments:args3}});const r3=await pending();p.send({jsonrpc:'2.0',method:'notifications/cancelled',params:{requestId:8,reason:'original cancelled'}});
  await until(async()=> (await read(join(root,'state.json')))?.requests[r3.target.requestId].state==='cancelled');passed++;
  const invalid=child('mcp');await initialize(invalid);invalid.stdin.write('{"jsonrpc":"2.0","id":9,"id":10,"method":"ping"}\n');await until(()=>invalid.exitCode!==null);assert.notEqual(invalid.exitCode,0);passed++;
  const broken=child('mcp');await initialize(broken);broken.stdout.destroy();broken.send({jsonrpc:'2.0',id:22,method:'ping',params:{}});await until(()=>broken.exitCode!==null);assert.notEqual(broken.exitCode,0);passed++;
  p.stdin.end();await until(()=>p.exitCode!==null);passed++;
  store.stdin.end();await until(()=>store.exitCode!==null);
  const restarted=child('store');const previous=epoch;epoch=(await until(async()=>{const h=await read(join(root,'heartbeat.json'));return h?.appEpoch!==previous&&h;})).appEpoch;
  const old=await command({kind:'prepare',connection:'connection',operation:r.target.requestId});assert.equal(old.error.error.code,'REQUEST_EXPIRED');passed++;
  restarted.stdin.end();await until(()=>restarted.exitCode!==null);
  console.log(JSON.stringify({suite:'production Trae core synthetic process tests',passed,nativeObservationScenarios:3,workspaceLessChatScenarios:2,legacyMcpScenarios:10,runtimeTraeVerified:false}));
} finally {
  for(const p of children)if(p.exitCode===null)p.kill();
  await new Promise(r=>setTimeout(r,300));
  assert.ok(root.startsWith(join(base,'process-tests')+ '\\') || root.startsWith(join(base,'process-tests')+'/'));
  await rm(root,{recursive:true,force:true});
}
