// Local synthetic UI verification; all approval submissions are intercepted.
// Run with two-core affinity, as required by AGENTS.md.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createServer } from 'vite';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const root = fileURLToPath(new URL('../../', import.meta.url));
const output = fileURLToPath(new URL('./runs/tool-ui/', import.meta.url));
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
window.cursorToolFixture = async (toolName, toolInput) => {
  await startupSequencePromise;
  controller.setCollapseDelay(60000);
  await controller.pointerEntered();
  const { createCursorInteractionPreview } = await import('/src/cursor-preview.ts');
  const snapshot = createCursorInteractionPreview();
  snapshot.version = (window.cursorToolVersion = (window.cursorToolVersion || 10) + 1);
  const request = snapshot.requests.find(r => r.body.kind === 'permission');
  request.requestId += snapshot.version;
  request.submission.requestId = request.requestId;
  request.body.content = {eventName: 'beforeMCPExecution', toolName, toolInput};
  snapshot.requests = [request];
  snapshot.sessions = snapshot.sessions.filter(s => s.key === request.sessionKey);
  window.cursorToolCalls = [];
  cursorView.submit = async (route, body) => { window.cursorToolCalls.push({route, body}); };
  cursorView.refresh = async () => {};
  renderCursorSnapshot(snapshot);
  openCursorRequest(request.requestId);
  syncPanelShape();
  return request.submission;
};` });
  });
  await page.goto('http://127.0.0.1:1422/?cursor=1');
  await page.waitForFunction(() => window.cursorToolFixture);
  await page.evaluate(() => window.cursorToolFixture('codecraft_cursor · codecraft_list_projects', {}));
  const review = page.locator('#permission-view .cursor-review-form');
  await review.waitFor({ state: 'visible' });
  assert.equal(await review.locator('.question-preview > .cursor-tool-input').textContent(), '读取已在 CodeCraft 中启用的项目列表。');
  assert.equal(await review.locator('.cursor-tool-details').count(), 0);
  await page.screenshot({ path: `${output}/desktop-projects.png` });
  const input = { projectId: '2f50621bca15e368b1393153e04d83bdac4e7aca43ed4f27053f871e0822ac9b', title: '问答界面验证', content: '<script>window.injected = true</script>\n第二行 😀' };
  const submission = await page.evaluate(input => window.cursorToolFixture('codecraft_cursor · codecraft_start_workflow', input), input);
  assert.equal(await review.locator('.question-preview > .cursor-tool-input').textContent(), '创建问答或计划审阅任务。');
  const details = review.locator('.cursor-tool-details');
  assert.equal(await details.getAttribute('open'), null);
  await details.locator('summary').focus();
  await page.keyboard.press('Enter');
  // The desktop toolbar animates its native clipping region after resizing.
  await page.waitForTimeout(500);
  assert.deepEqual(JSON.parse(await details.locator('pre').textContent()), input);
  assert.equal(await page.evaluate(() => window.injected), undefined);
  assert.ok(await details.locator('pre').isVisible());
  await page.screenshot({ path: `${output}/desktop-workflow.png` });
  await review.getByRole('button', { name: '允许一次', exact: false }).click();
  const calls = await page.evaluate(() => window.cursorToolCalls);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].route, 'permission');
  assert.deepEqual(calls[0].body.submission, submission);
  assert.deepEqual(calls[0].body.decision, { kind: 'permission', action: 'allowOnce' });

  // The mobile page uses the same LAN CursorView branch, without a live server
  // token or connection to an installed agent.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('http://127.0.0.1:1422/');
  await page.evaluate(async input => {
    const { CursorView } = await import('/src/cursor-view.ts');
    const { createCursorInteractionPreview } = await import('/src/cursor-preview.ts');
    const snapshot = createCursorInteractionPreview();
    const r = snapshot.requests.find(r => r.body.kind === 'permission');
    r.body.content = { eventName: 'beforeMCPExecution', toolName: 'codecraft_cursor · codecraft_start_workflow', toolInput: input };
    document.body.innerHTML = '<main id="lan-fixture"></main>';
    document.body.style.cssText = 'margin:0;background:#182228;color:#e1e7ee;overflow:auto';
    const view = new CursorView(async () => {}, async () => {});
    view.update(snapshot, false);
    view.show(document.querySelector('#lan-fixture'), r.requestId);
  }, input);
  const mobile = page.locator('#lan-fixture');
  await mobile.locator('.cursor-tool-details > summary').click();
  assert.deepEqual(JSON.parse(await mobile.locator('.cursor-tool-details > pre').textContent()), input);
  assert.equal(await mobile.getByRole('button', {name: '允许本次'}).isEnabled(), false);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  await page.screenshot({ path: `${output}/mobile-workflow.png` });
  assert.deepEqual(errors, []);
  console.log('Passed: desktop summaries, keyboard details, unchanged approval, escaped content, mobile read-only details and width.');
  console.log(`Screenshots: ${output}`);
} finally {
  await browser?.close();
  await server.close();
}
