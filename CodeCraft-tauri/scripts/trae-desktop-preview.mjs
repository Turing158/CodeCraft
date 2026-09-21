// Isolated Vite harness. The injected controls and fake invoke never enter a production build.
// Run in a PowerShell process restricted to affinity 3, like all project previews.
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));
const permissionArguments = {
  file_path: 'C:\\Synthetic\\acceptance-plan.md',
  content: '# 验收计划\n\n## 操作步骤\n1. 检查输入。\n2. 写入 ALLOW_OK。\n\n<script>fixture()</script>',
  options: { mode: 'safe', dryRun: false, count: 2 },
};
const harness = `
await startupSequencePromise;
controller.setCollapseDelay(60000);
await controller.pointerEntered();
const fake = emptyTraeSnapshot(); fake.connected = true; fake.appEpoch = 'synthetic-epoch';
fake.capabilities.toolApproval = true;
fake.sessions = [{source:'trae',sessionKey:'synthetic-session',sessionId:'native-test',installationId:'test',traeInstanceId:'test',cwd:'C:/Synthetic',workspaceRoots:['C:/Synthetic'],title:'Trae 只读与工具审批测试',status:'waitingForInput',turnEpoch:1,taskId:null,nativeNotice:null,nativeInteractions:[],output:'',updatedAt:new Date().toISOString(),activities:[]}];
const fixtureBar = document.createElement('aside'); fixtureBar.id = 'trae-fixture-controls';
fixtureBar.style.cssText = 'position:fixed;left:12px;bottom:12px;z-index:2147483647;max-width:360px;padding:12px;background:#172126;color:white;border:1px solid #789;border-radius:8px;font:13px system-ui';
const fixtureLog = document.createElement('pre'); fixtureLog.id='trae-fixture-result'; fixtureLog.style.cssText='white-space:pre-wrap;max-height:160px;overflow:auto'; fixtureLog.textContent='合成测试：不连接 Trae，不发送真实批准';
const applyFake = () => { fake.version++; renderTraeSnapshot(structuredClone(fake)); syncPanelShape(); };
let sequence = 0;
for (const [label, action] of [
 ['触发工具审批', () => { fake.sessions[0].nativeInteractions=[];fake.requests=[{target:{appEpoch:fake.appEpoch,sessionKey:'synthetic-session',taskId:null,turnEpoch:1,requestId:'tool-'+(++sequence),requestVersion:1},kind:'permission',channel:'hook',toolName:'Write',toolUseId:'call',arguments:${JSON.stringify(permissionArguments)},state:'pending',expiresAt:new Date(Date.now()+120000).toISOString(),plan:null,error:null}]; }],
 ['触发原生问题', () => { fake.requests=[];fake.sessions[0].nativeInteractions=[{id:'question-'+(++sequence),kind:'question',toolUseId:'native-q',arguments:{questions:[{question:'选择开发语言？',options:[{label:'Rust',description:'系统开发'},{label:'TypeScript',description:'界面开发'}]}]},message:'等待回答',readOnly:true,capturedAt:new Date().toISOString(),documentPath:null,plan:null}]; }],
 ['触发原生计划', () => { fake.requests=[];fake.sessions[0].nativeInteractions=[{id:'plan-'+(++sequence),kind:'plan',toolUseId:null,arguments:{},message:'等待审阅',readOnly:true,capturedAt:new Date().toISOString(),documentPath:'C:/Synthetic/.trae/documents/plan.md',planSource:'session_file',plan:'# 当前计划\\n\\n1. 查看输入文件。\\n2. 修改实现。\\n3. 执行测试。\\n\\n<script>bad()</script>'}]; }],
 ['重复同一事件', () => {}],
 ['结束当前事件', () => { fake.requests=[];fake.sessions[0].nativeInteractions=[]; }],
 ['模拟一次回传失败', () => { window.traeFixtureFail = true;fixtureLog.textContent='下一次工具决定将模拟网络失败'; }],
 ['准备会话详情', () => {
  fake.requests=[];fake.sessions[0].nativeInteractions=[];fake.sessions[0].status='working';
  fake.sessions[0].messages=[
   {id:'u1',role:'user',text:'第一轮：查看文件 <script>bad()</script>',at:'2026-09-20T01:00:00Z'},
   {id:'a1',role:'assistant',text:'第一轮回复：已读取文件。',at:'2026-09-20T01:00:01Z'},
   {id:'u2',role:'user',text:'第二轮：继续检查。',at:'2026-09-20T01:00:02Z'},
  ];
  fake.sessions[0].activities=[
   {id:'1:read',tool:'Read',toolUseId:'read',status:'completed',arguments:'file_path: C:/Synthetic/input.txt',result:'FIRST_TOOL_RESULT',at:new Date().toISOString()},
   {id:'2:read',tool:'Read',toolUseId:'read',status:'running',arguments:'file_path: C:/Synthetic/next.txt',at:new Date().toISOString()},
  ];
  fake.sessions=[fake.sessions[0],{...structuredClone(fake.sessions[0]),sessionKey:'second-session',sessionId:'second-native',title:'第二个独立会话',messages:[{id:'other',role:'assistant',text:'OTHER_SESSION_ONLY',at:new Date().toISOString()}],activities:[]}];
  switchContentView('sessions');
 }],
 ['追加会话回复', () => { fake.sessions[0].messages.push({id:'a2',role:'assistant',text:'第二轮回复：检查完成。',at:new Date().toISOString()});fake.sessions[0].activities[1].status='unknown';fake.sessions[0].status='stopped'; }],
 ['移除测试会话', () => { fake.requests=[];fake.sessions=[]; }],
]) { const b=document.createElement('button');b.textContent=label;b.style.cssText='margin:3px;padding:5px';b.onclick=()=>{ action();applyFake(); };fixtureBar.append(b); }
fixtureBar.append(fixtureLog);document.body.append(fixtureBar);
window.traeFixtureInvoke = async (command, args) => {
 if(command==='focus_trae_window'){fixtureLog.textContent='已请求前往 Trae 中处理';return;}
 if(command==='trae_get_snapshot')return structuredClone(fake);
 if(command==='trae_respond_permission'){
  const calls=JSON.parse(fixtureLog.dataset.calls||'[]');calls.push(args.body);fixtureLog.dataset.calls=JSON.stringify(calls);fixtureLog.textContent=JSON.stringify(calls,null,2);
  if(window.traeFixtureFail){window.traeFixtureFail=false;throw new Error('模拟网络失败，请重试');}
  fake.requests=[]; return {accepted:true};
 }
 throw new Error('Unsupported fixture command');
};
applyFake();
`;
const server = await createServer({ root, server: { host: '127.0.0.1', port: 1420, strictPort: true }, plugins: [{
  name: 'trae-synthetic-desktop', enforce: 'pre',
  transform(code, id) {
    if (!id.replaceAll('\\', '/').endsWith('/src/main.ts')) return;
    code = code.replace('import { invoke } from "@tauri-apps/api/core";', `import { invoke as originalInvoke } from "@tauri-apps/api/core";
const invoke: typeof originalInvoke = ((command, args) => window.traeFixtureInvoke && command.startsWith('trae_') || command === 'focus_trae_window' ? window.traeFixtureInvoke(command,args) : originalInvoke(command,args));`);
    return code + '\n' + harness;
  },
}] });
await server.listen();
server.printUrls();
