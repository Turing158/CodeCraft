// Exercise the production shared detail renderer with isolated Cursor records.
// No real agent connection, files, questions or approvals are used.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createServer } from 'vite';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const root = fileURLToPath(new URL('../../', import.meta.url));
const output = fileURLToPath(new URL('./runs/session-detail/', import.meta.url));
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: '127.0.0.1', port: 1422, strictPort: true } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE });
  const page = await browser.newPage({ viewport: { width: 760, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: await response.text() + `
window.cursorDetailFixture = async (pending = false) => {
  await startupSequencePromise;
  controller.setCollapseDelay(60000);
  await controller.pointerEntered();
  const { createCursorInteractionPreview } = await import('/src/cursor-preview.ts');
  const s = createCursorInteractionPreview(), now = Date.now();
  s.version = (window.cursorDetailVersion = (window.cursorDetailVersion || 10) + 1);
  const [question, plan, permission] = s.requests;
  const native = s.sessions[2], workflow = s.sessions[0];
  native.title = '检查输入文件并运行验证';
  native.activity = 'stopped'; native.hasCurrentRequest = false;
  native.items = [
    {kind:'tool',generation:'g1',callId:'read',name:'Read',outcome:'completedObserved'},
    {kind:'tool',generation:'g1',callId:'shell',name:'Shell',outcome:'unknown'},
    {kind:'response',generation:'g1',text:'## 检查结果\\n\\n已读取输入文件，并核对了构建配置。\\n\\n- 中文文件名显示正常。\\n- 输出保留原有格式。\\n\\n<script>window.injected = true</script>',truncated:false},
  ];
  permission.createdAt = now - 60000; permission.updatedAt = now - 50000;
  permission.actionable = false; permission.phase = 'decided'; permission.transport = 'written';
  permission.body.content = {eventName:'preToolUse',toolName:'Shell',toolInput:{command:'npm run build',cwd:'D:/项目/示例'}};
  permission.decision = {value:{kind:'permission',action:'allowOnce'}};
  workflow.title = '确认验证范围与实施计划';
  workflow.activity = pending ? 'working' : 'stopped'; workflow.hasCurrentRequest = pending;
  question.createdAt = now - 40000; question.updatedAt = now - 35000;
  question.actionable = pending; question.phase = pending ? 'pending' : 'decided'; question.transport = pending ? 'notStarted' : 'written';
  question.decision = pending ? null : {value:{kind:'answered',answers:[
    {questionId:'target',selectedOptionIds:['dev'],text:'先检查中文输入',skipped:false},
    {questionId:'checks',selectedOptionIds:['unit','build'],text:null,skipped:false},
  ]}};
  plan.sessionKey = workflow.key; plan.workflowId = question.workflowId;
  plan.createdAt = now - 30000; plan.updatedAt = now - 25000;
  plan.actionable = false; plan.phase = 'decided'; plan.transport = 'written';
  plan.decision = {value:{kind:'plan',action:'revise',feedback:'先核对输入，再运行构建。'}};
  s.sessions = [native, workflow];
  window.cursorDetailSnapshot = s;
  renderCursorSnapshot(s);
  openCursorSession(native.key);
  syncPanelShape();
};
window.cursorDetailOpen = key => { openCursorSession(key); syncPanelShape(); };
window.cursorDetailRefresh = () => {
  const s = structuredClone(window.cursorDetailSnapshot);
  s.version = ++window.cursorDetailVersion;
  s.sessions[0].updatedAt += 1000;
  s.sessions[0].items.push({kind:'response',generation:'g1',text:'追加的验证结果。',truncated:false});
  window.cursorDetailSnapshot = s;renderCursorSnapshot(s);syncPanelShape();
};
window.cursorDetailReference = (provider = 'claude') => {
  const now = Date.now();
  const common = {
    id:'reference-'+provider,title:provider === 'codex' ? 'Codex 会话详情' : 'Claude Code 会话详情',
    status:'stopped',startedAt:now,updatedAt:now,
    activities:[{id:'reference-read',tool:'Read',summary:'src/main.ts',status:'completed',startedAt:now,updatedAt:now}],
    outputs:[{id:'reference-output',text:'已检查共享会话详情。',role:'assistant',capturedAt:now}],
  };
  renderSessionDetail(provider === 'codex'
    ? {...common,cwd:'D:/项目/示例',pendingInteractionId:null}
    : {...common,question:null,permission:null,plan:null});
  switchContentView('detail');syncPanelShape();
};
window.cursorDetailLong = () => {
  const s = structuredClone(window.cursorDetailSnapshot);
  s.version = ++window.cursorDetailVersion;
  s.sessions[0].items = Array.from({length:24}, (_,i) => [
    {kind:'tool',generation:'long',callId:'tool-'+i,name:'Read',outcome:'completedObserved'},
    {kind:'response',generation:'long',text:'第 '+(i+1)+' 条记录：检查现有会话详情的工具调用与对话布局。',truncated:false},
  ]).flat();
  window.cursorDetailSnapshot = s;renderCursorSnapshot(s);
  openCursorSession(s.sessions[0].key);syncPanelShape();
};
window.cursorDetailEmpty = () => {
  const s = structuredClone(window.cursorDetailSnapshot);
  s.version = ++window.cursorDetailVersion;s.requests=[];
  s.sessions[0].items=[];s.sessions[0].hasCurrentRequest=false;
  window.cursorDetailSnapshot = s;renderCursorSnapshot(s);
  openCursorSession(s.sessions[0].key);
  switchContentView('detail');syncPanelShape();
};
window.cursorDetailNarrow = () => updateInterfaceSettings({mode:'custom',customWidth:360,customPadding:10});` });
  });
  await page.goto('http://127.0.0.1:1422/?cursor=1');
  await page.waitForFunction(() => window.cursorDetailFixture);
  await page.evaluate(() => window.cursorDetailFixture());
  const detail = page.locator('#session-detail-view');
  await detail.waitFor({ state: 'visible' });
  await page.waitForTimeout(500);
  const structure = () => detail.evaluate(node => ({
    classes: node.className,
    header: node.querySelector('header').className,
    sections: [...node.querySelectorAll(':scope > section')].map(s => ({
      className:s.className,title:s.querySelector('h2').textContent,
      padding:getComputedStyle(s).padding,border:getComputedStyle(s).borderRadius,
      background:getComputedStyle(s).backgroundColor,font:getComputedStyle(s.querySelector('h2')).font,
    })),
    activity: node.querySelector('.activity-item').className,
    output: node.querySelector('.output-entry').className,
  }));
  const cursorStructure = await structure();
  assert.deepEqual(cursorStructure.sections.map(s => s.title), ['工具调用','对话']);
  assert.equal(await detail.locator('#cursor-detail-notice,.cursor-request-list').count(),0);
  assert.equal(await detail.locator('#session-detail-context').isVisible(),false);
  assert.equal(await detail.locator('.activity-item').count(),3);
  assert.equal(await page.evaluate(() => window.injected),undefined);
  const args = detail.locator('.session-activity-details');
  await args.locator('summary').click();
  assert.match(await args.locator('pre').textContent(),/npm run build/);
  await page.evaluate(() => window.cursorDetailRefresh());
  assert.equal(await args.getAttribute('open'),'');
  assert.match(await detail.locator('#session-output').textContent(),/追加的验证结果/);
  await page.waitForTimeout(500);
  await page.screenshot({path:output+'/cursor-native.png'});
  await page.evaluate(() => window.cursorDetailReference());
  assert.deepEqual(await structure(),cursorStructure);
  assert.match(await detail.locator('#session-output header').textContent(),/^Claude Code · /);
  await page.waitForTimeout(500);
  await page.screenshot({path:output+'/shared-reference.png'});
  await page.evaluate(() => window.cursorDetailReference('codex'));
  assert.deepEqual(await structure(),cursorStructure);
  assert.match(await detail.locator('#session-output header').textContent(),/^Codex · /);
  await page.screenshot({path:output+'/codex-reference.png'});

  await page.evaluate(() => window.cursorDetailLong());
  await args.locator('summary').click();
  const scrollBefore = await detail.evaluate(node => {
    const tools = node.querySelector('#session-activity-list');
    const output = node.querySelector('#session-output');
    tools.scrollTop=60;output.scrollTop=120;
    return {tools:tools.scrollTop,output:output.scrollTop};
  });
  assert.ok(scrollBefore.tools > 0 && scrollBefore.output > 0);
  await page.evaluate(() => window.cursorDetailRefresh());
  assert.deepEqual(await detail.evaluate(node => ({
    tools:node.querySelector('#session-activity-list').scrollTop,
    output:node.querySelector('#session-output').scrollTop,
  })),scrollBefore,'refresh must preserve both reading positions');
  assert.equal(await args.getAttribute('open'),'');
  assert.equal(await args.locator('summary').evaluate(node => node === document.activeElement),true);
  await detail.locator('#session-output').evaluate(node => node.scrollTop=node.scrollHeight);
  await page.evaluate(() => window.cursorDetailRefresh());
  assert.ok(await detail.locator('#session-output').evaluate(node => node.scrollHeight-node.scrollTop-node.clientHeight < 2));
  await page.evaluate(() => window.cursorDetailReference());
  await page.evaluate(() => window.cursorDetailOpen('3'.repeat(64)));
  assert.equal(await args.getAttribute('open'),null,'expanded tools must not carry across sessions');
  assert.equal(await detail.locator('#session-activity-list').evaluate(node => node.scrollTop),0);
  await page.evaluate(() => window.cursorDetailEmpty());
  assert.equal(await detail.locator('.detail-empty').count(),2);
  assert.match(await detail.locator('#activity-summary').textContent(),/0 项记录/);
  await page.evaluate(() => window.cursorDetailFixture());

  await page.evaluate(() => window.cursorDetailOpen('1'.repeat(64)));
  const dialogue = detail.locator('#session-output');
  assert.equal(await dialogue.locator('.output-entry').count(),4);
  assert.match(await dialogue.textContent(),/先检查中文输入/);
  assert.match(await dialogue.textContent(),/先核对输入，再运行构建/);
  assert.equal(await detail.locator('#session-detail-context').isVisible(),false);
  await page.waitForTimeout(500);
  await page.screenshot({path:output+'/cursor-workflow.png'});
  await page.evaluate(() => window.cursorDetailFixture(true));
  await page.evaluate(() => window.cursorDetailOpen('1'.repeat(64)));
  assert.equal(await detail.isVisible(),true);
  await detail.getByRole('button',{name:'查看待处理事项'}).click();
  const question = page.locator('#question-view');
  await question.waitFor({state:'visible'});
  assert.equal(await question.locator('.cursor-recorded,.cursor-review-status').count(),0);
  await question.locator('.cursor-review-form .question-block__header').first().hover();
  await question.getByRole('button',{name:'返回',exact:true}).first().click();
  await detail.waitFor({state:'visible'});
  await page.evaluate(() => window.cursorDetailRefresh());
  assert.equal(await detail.isVisible(),true);
  assert.equal(await detail.getByRole('button',{name:'查看待处理事项'}).isVisible(),true);
  await page.setViewportSize({width:390,height:844});
  await page.evaluate(() => { window.cursorDetailNarrow(); document.documentElement.dataset.theme='light'; });
  await page.waitForTimeout(500);
  assert.ok(await detail.evaluate(node => node.scrollWidth <= node.clientWidth));
  assert.ok(await detail.evaluate(node => node.getBoundingClientRect().right <= window.innerWidth));
  await page.screenshot({path:output+'/cursor-light-narrow.png'});

  // Exercise the actual LAN detail page, including switching from another
  // provider's pending review. No live API requests or decisions are sent.
  const fixture = await page.evaluate(() => window.cursorDetailSnapshot);
  const lan = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  lan.on('pageerror', error => errors.push(error.message));
  await lan.route('**/web/lan/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: await response.text() + `
window.cursorLanFixture = async state => {
  const { toCursorUnifiedSession } = await import('/src/cursor-sessions.ts');
  const adapted = toCursorUnifiedSession(state.sessions[0],state.requests);
  const reference = {...adapted,id:'reference',title:'参考：其他助手的会话详情',permission:{
    id:'reference-permission',toolName:'Read',summary:'读取参考文件',cwd:null,
    canAlwaysAllow:false,capturedAt:Date.now(),
  }};
  window.cursorLanRaw = {generatedAt:Date.now(),allowApprovals:true,cursor:state,
    claude:{connected:true,integrationError:null,sessions:[reference]}};
  selectedSources.clear();
  applySnapshot(window.cursorLanRaw);
  selectEntry('cursor:'+state.sessions[0].key,'detail');
};
window.cursorLanOpen = key => selectEntry(key,'detail');
window.cursorLanRefresh = writable => {
  const raw = window.cursorLanRaw;
  raw.cursor.version++;raw.allowApprovals=writable;
  applySnapshot(raw);
};
window.cursorLanResolve = () => {
  const raw = window.cursorLanRaw;
  const r = raw.cursor.requests.find(r=>r.body.kind==='question');
  r.actionable=false;r.phase='decided';r.transport='written';
  r.decision={value:{kind:'answered',answers:[
    {questionId:'target',selectedOptionIds:['dev'],text:'局域网回答记录',skipped:false},
    {questionId:'checks',selectedOptionIds:['unit'],text:null,skipped:false},
  ]}};
  raw.cursor.sessions.find(s=>s.key===r.sessionKey).hasCurrentRequest=false;
  window.cursorLanRefresh(true);
};` });
  });
  await lan.goto('http://127.0.0.1:1422/web/lan/index.html?cursor=1');
  await lan.waitForFunction(() => window.cursorLanFixture);
  await lan.evaluate(state => window.cursorLanFixture(state), fixture);
  const lanDetail = lan.locator('#detail');
  await lanDetail.waitFor({state:'visible'});
  const lanStructure = () => lanDetail.evaluate(node => ({
    header:node.querySelector('.detail__header').className,
    sections:[...node.querySelectorAll(':scope > .detail__section')].map(s=>({
      className:s.className,title:s.querySelector('h3').textContent,
      padding:getComputedStyle(s).padding,border:getComputedStyle(s).borderRadius,
    })),
    activity:node.querySelector('.activity').className,
    output:node.querySelector('.output').className,
  }));
  const lanCursorStructure = await lanStructure();
  assert.deepEqual(lanCursorStructure.sections.map(s=>s.title),['工具调用','对话']);
  assert.equal(await lanDetail.locator('.cursor-request-list').count(),0);
  assert.equal(await lan.locator('#review').isVisible(),false);
  const lanArgs = lan.locator('#activity-list details');
  await lanArgs.locator('summary').click();
  assert.match(await lanArgs.locator('pre').textContent(),/npm run build/);
  await lan.evaluate(() => window.cursorLanRefresh(true));
  assert.equal(await lanArgs.getAttribute('open'),'');
  assert.equal(await lan.evaluate(() => window.injected),undefined);
  await lan.waitForTimeout(500);
  await lan.screenshot({path:output+'/cursor-lan-wide.png'});
  await lan.evaluate(() => window.cursorLanOpen('claude:reference'));
  assert.deepEqual(await lanStructure(),lanCursorStructure);
  await lan.setViewportSize({width:390,height:844});
  await lan.waitForTimeout(500);
  assert.equal(await lan.locator('#action-bar').isVisible(),true);
  await lan.evaluate(() => window.cursorLanOpen('cursor:'+'1'.repeat(64)));
  assert.equal(await lan.locator('#action-bar button').count(),0,'previous provider actions must not survive in Cursor details');
  assert.equal(await lan.locator('#action-bar').isVisible(),false);
  assert.match(await lan.locator('#output-list').textContent(),/先核对输入，再运行构建/);
  assert.equal(await lan.locator('#review .cursor-interactions').isVisible(),true);
  await lan.evaluate(() => window.cursorLanRefresh(false));
  assert.equal(await lan.getByRole('button',{name:'提交回答',exact:true}).isEnabled(),false);
  await lan.evaluate(() => window.cursorLanRefresh(true));
  assert.equal(await lan.getByRole('button',{name:'提交回答',exact:true}).isEnabled(),true);
  await lan.evaluate(() => window.cursorLanResolve());
  assert.equal(await lan.locator('#review').isVisible(),false);
  assert.match(await lan.locator('#output-list').textContent(),/局域网回答记录/);
  await lan.evaluate(() => document.documentElement.dataset.theme='light');
  await lan.waitForTimeout(500);
  assert.ok(await lanDetail.evaluate(node=>node.scrollWidth<=node.clientWidth));
  assert.ok(await lanDetail.evaluate(node=>node.getBoundingClientRect().right<=window.innerWidth));
  await lan.screenshot({path:output+'/cursor-lan-light-phone.png'});
  await lan.getByRole('button',{name:'返回会话列表',exact:true}).click();
  await lan.evaluate(() => window.cursorLanRefresh(true));
  assert.equal(await lanDetail.isVisible(),false);
  assert.deepEqual(errors,[]);
  console.log('Passed: Cursor/Claude/Codex shared layout/style, reading position and keyboard focus on refresh, empty state, tool details, safe transcript, workflow history, pending review/back/refresh, narrow light theme, provider switching and LAN read-only state.');
  console.log('Screenshots: '+output);
} finally {
  await browser?.close();
  await server.close();
}
