// Browser smoke test against trae-desktop-preview.mjs. All events and decisions
// are synthetic; the preview injects its harness only into the served module.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE
  ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const base = process.env.CODECRAFT_UI_TEST_URL || 'http://127.0.0.1:1420/';
assert.equal(new URL(base).hostname, '127.0.0.1');
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE });
const output = resolve('src-tauri/trae-core/target/desktop-ui');
await mkdir(output, { recursive: true });
try {
  const page = await browser.newPage({ viewport: { width: 740, height: 940 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(base);
  await page.getByRole('button', { name: '触发工具审批', exact: true }).waitFor();
  let passed = 0;
  for (const [id, decision] of [['permission-allow', 'allow'], ['permission-deny', 'deny'], ['permission-return-trae', 'ask']]) {
    await page.getByRole('button', { name: '触发工具审批', exact: true }).click();
    await page.locator('#permission-tool').filter({ hasText: 'Write' }).waitFor({ state: 'visible' });
    await page.waitForFunction(() => !document.querySelector('.panel-view--leaving'));
    const summary = page.locator('#permission-summary-text');
    const summaryText = await summary.textContent();
    assert.match(summaryText, /content:\n# 验收计划\n\n## 操作步骤/);
    assert.match(summaryText, /ALLOW_OK/);
    assert.ok(summaryText.includes('C:\\Synthetic\\acceptance-plan.md'));
    assert.ok(summaryText.includes('options:\n{\n  "mode": "safe"'));
    assert.equal(await summary.getAttribute('title'), null);
    assert.equal(await summary.locator('script').count(), 0);
    assert.equal(await summary.evaluate(el => getComputedStyle(el).whiteSpace), 'pre-wrap');
    assert.equal(await summary.evaluate(el => getComputedStyle(el).display), 'block');
    assert.ok(await summary.evaluate(el => el.scrollHeight > el.clientHeight));
    assert.equal(await page.locator(`#${id}`).isEnabled(), true);
    assert.equal(await page.locator('#permission-always-allow').isVisible(), false);
    if (passed === 0) await page.screenshot({ path: resolve(output, 'tool-approval.png') });
    await page.locator(`#${id}`).click();
    await page.waitForFunction(expected => {
      const calls = JSON.parse(document.querySelector('#trae-fixture-result').dataset.calls || '[]');
      return calls.at(-1)?.action.decision === expected;
    }, decision);
    passed++;
  }
  await page.getByRole('button', { name: '触发原生计划', exact: true }).click();
  await page.locator('#plan-summary-text').filter({ hasText: '当前计划' }).waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('.panel-view--leaving'));
  const planText = await page.locator('#plan-summary-text').textContent();
  assert.match(planText, /当前会话本轮/);
  assert.match(planText, /\.trae\/documents\/plan\.md/);
  assert.equal(await page.locator('#plan-open-trae').isVisible(), true);
  assert.equal(await page.locator('#plan-auto').isVisible(), false);
  assert.equal(await page.locator('#plan-custom-submit').isVisible(), false);
  assert.equal(await page.locator('#plan-custom-input').isVisible(), false);
  await page.screenshot({ path: resolve(output, 'plan-association.png') });
  await page.locator('#plan-open-trae').click();
  await page.getByText('已请求前往 Trae 中处理', { exact: true }).waitFor();
  assert.equal(JSON.parse(await page.locator('#trae-fixture-result').getAttribute('data-calls')).length, 3);
  passed++;

  // Opening Trae is only a handoff. Its next snapshot completes or replaces the review.
  assert.equal(await page.locator('#plan-view').isVisible(), true);
  await page.getByRole('button', { name: '触发原生问题', exact: true }).click();
  await page.locator('#question-view').waitFor({ state: 'visible' });
  await page.locator('#plan-view').waitFor({ state: 'hidden' });
  assert.equal(await page.locator('#plan-summary-text').textContent(), '');
  await page.getByRole('button', { name: '重复同一事件', exact: true }).click();
  assert.equal(await page.locator('#plan-view').isVisible(), false);
  assert.equal(await page.locator('#question-view').isVisible(), true);
  passed++;

  await page.getByRole('button', { name: '结束当前事件', exact: true }).click();
  await page.locator('#question-view').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '触发原生计划', exact: true }).click();
  await page.locator('#plan-view').waitFor({ state: 'visible' });
  await page.getByRole('button', { name: '结束当前事件', exact: true }).click();
  await page.locator('#plan-view').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '重复同一事件', exact: true }).click();
  assert.equal(await page.locator('#plan-view').isVisible(), false);
  assert.equal(await page.locator('#plan-summary-text').textContent(), '');
  assert.equal(JSON.parse(await page.locator('#trae-fixture-result').getAttribute('data-calls')).length, 3);
  passed++;
  await page.getByRole('button', { name: '准备会话详情', exact: true }).click();
  await page.locator('#trae-session-list button').filter({ hasText: 'Trae 只读与工具审批测试' }).click();
  await page.locator('#session-detail-view').waitFor({ state: 'visible' });
  assert.equal(await page.locator('#session-output article').count(), 3);
  assert.match(await page.locator('#session-output').textContent(), /第一轮回复/);
  assert.match(await page.locator('#session-output').textContent(), /用户/);
  assert.equal(await page.locator('#session-output script').count(), 0);
  assert.equal(await page.locator('#session-output-source').textContent(), '已采集记录');
  assert.equal(await page.locator('#session-activity-list [data-activity-status="running"]').count(), 1);
  const tool = page.locator('#session-activity-list details[data-activity-id="1:read"]');
  await tool.locator('summary').click();
  assert.match(await tool.locator('pre').textContent(), /FIRST_TOOL_RESULT/);
  await page.getByRole('button', { name: '追加会话回复', exact: true }).click();
  await page.locator('#session-output').filter({ hasText: '第二轮回复：检查完成。' }).waitFor();
  assert.equal(await page.locator('#session-output article').count(), 4);
  assert.equal(await tool.getAttribute('open'), '');
  assert.equal(await page.locator('#session-activity-list [data-activity-status="unknown"]').count(), 1);
  await page.screenshot({ path: resolve(output, 'session-detail.png') });
  passed++;

  await page.getByRole('button', { name: '触发工具审批', exact: true }).click();
  await page.locator('#permission-view').waitFor({ state: 'visible' });
  await page.locator('#permission-allow').click();
  await page.locator('#session-detail-view').waitFor({ state: 'visible' });
  assert.match(await page.locator('#session-output').textContent(), /第一轮回复/);
  await page.getByRole('button', { name: '触发原生计划', exact: true }).click();
  await page.locator('#plan-view').waitFor({ state: 'visible' });
  // Back controls expand on header hover, matching the desktop pointer flow.
  await page.locator('#plan-block-title').hover();
  await page.locator('#plan-back').click();
  await page.locator('#session-detail-view').waitFor({ state: 'visible' });
  await page.locator('#trae-detail-review').click();
  await page.locator('#plan-view').waitFor({ state: 'visible' });
  await page.getByRole('button', { name: '结束当前事件', exact: true }).click();
  await page.locator('#session-detail-view').waitFor({ state: 'visible' });
  passed++;

  await page.locator('#session-detail-back').click();
  await page.locator('#trae-session-list button').filter({ hasText: '第二个独立会话' }).click();
  await page.locator('#session-detail-view').waitFor({ state: 'visible' });
  assert.match(await page.locator('#session-output').textContent(), /OTHER_SESSION_ONLY/);
  assert.doesNotMatch(await page.locator('#session-output').textContent(), /第一轮回复/);
  await page.getByRole('button', { name: '移除测试会话', exact: true }).click();
  await page.locator('#session-detail-view').waitFor({ state: 'hidden' });
  assert.equal(await page.locator('#session-output').textContent(), '');
  passed++;

  const mobile = await browser.newPage({ viewport: { width: 390, height: 844 } });
  mobile.on('pageerror', error => errors.push(error.message));
  const now = new Date().toISOString();
  const session = {
    source: 'trae', sessionKey: 'lan-session', sessionId: 'native-lan', installationId: 'test', traeInstanceId: 'test',
    cwd: 'C:/Synthetic', workspaceRoots: ['C:/Synthetic'], title: 'Trae 局域网会话', status: 'working',
    turnEpoch: 1, taskId: null, nativeNotice: null, nativeInteractions: [], output: '', updatedAt: now,
    messages: [
      { id: 'user', role: 'user', text: '检查文件 <script>bad()</script>', at: now },
      { id: 'reply', role: 'assistant', text: 'FIRST_LAN_REPLY', at: now },
    ],
    activities: [{ id: '1:read', tool: 'Read', toolUseId: 'read', status: 'completed', arguments: 'C:/Synthetic/input.txt', result: 'LAN_TOOL_RESULT', at: now }],
  };
  const raw = { generatedAt: Date.now(), allowApprovals: false, trae: {
    source: 'trae', connected: true, appEpoch: 'lan-epoch', version: 1,
    sessions: [session, { ...structuredClone(session), sessionKey: 'lan-second', title: '第二个局域网会话', messages: [{ id: 'other', role: 'assistant', text: 'OTHER_LAN_SESSION', at: now }] }],
    requests: [], tasks: [], grants: [],
  } };
  await mobile.route('**/api/state', route => route.fulfill({ json: raw }));
  await mobile.route('**/api/events', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' }));
  // Expose only the synthetic snapshot entry point in the intercepted test module.
  await mobile.route('**/web/lan/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: await response.text() + '\n;globalThis.applyTraeTestSnapshot = applySnapshot;' });
  });
  await mobile.goto(new URL('/web/lan/index.html', base).href);
  await mobile.locator('#session-list button').filter({ hasText: 'Trae 局域网会话' }).click();
  await mobile.locator('#detail').waitFor({ state: 'visible' });
  assert.equal(await mobile.locator('#output-list .output').count(), 2);
  assert.match(await mobile.locator('#output-list').textContent(), /已采集/);
  assert.equal(await mobile.locator('#output-list script').count(), 0);
  const lanTool = mobile.locator('#activity-list details[data-activity-id="1:read"]');
  await lanTool.locator('summary').click();
  assert.match(await lanTool.locator('pre').textContent(), /LAN_TOOL_RESULT/);
  session.messages.push({ id: 'next', role: 'assistant', text: 'SECOND_LAN_REPLY', at: now });
  await mobile.evaluate(raw => globalThis.applyTraeTestSnapshot(raw), raw);
  assert.equal(await lanTool.getAttribute('open'), '');
  assert.equal(await mobile.locator('#output-list .output').count(), 3);
  assert.match(await mobile.locator('#output-list').textContent(), /SECOND_LAN_REPLY/);
  assert.equal(await mobile.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
  await mobile.screenshot({ path: resolve(output, 'lan-session-detail.png') });
  await mobile.locator('#detail-back').click();
  await mobile.locator('#session-list button').filter({ hasText: '第二个局域网会话' }).click();
  assert.match(await mobile.locator('#output-list').textContent(), /OTHER_LAN_SESSION/);
  assert.doesNotMatch(await mobile.locator('#output-list').textContent(), /FIRST_LAN_REPLY/);
  assert.equal(await lanTool.getAttribute('open'), null);
  raw.trae.sessions = [];
  await mobile.evaluate(raw => globalThis.applyTraeTestSnapshot(raw), raw);
  assert.equal(await mobile.locator('#detail').isVisible(), false);
  passed++;
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ suite: 'Trae desktop/LAN session details and reviews', passed, synthetic: true,
    screenshot: resolve(output, 'tool-approval.png'), pageErrors: errors }));
} finally {
  await browser.close();
}
