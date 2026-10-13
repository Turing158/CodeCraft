# Cursor 会话详情统一布局

Cursor 详情复用桌面与 LAN 各自的共用详情渲染器：标题、状态、工具调用和对话区域沿用其他助手的布局。移除了 Cursor 专属说明块与历史请求列表，采集范围说明保留在“已采集记录”的提示中。

- 工具审批记录进入工具调用区，可展开原始参数与审批结果；允许工具执行不会被标记成执行成功。
- 问题、回答、计划和审阅意见进入对话区，按会话键关联，并保留内容角色与时间。
- 桌面通过“查看待处理事项”打开现有审批页，返回和刷新保持在详情页。审批确认成功后返回进入审批前的页面，有新的待处理请求时继续显示下一项。
- LAN 在共用详情页中显示当前待处理请求，完成后保留对话历史；切换来源时清空旧的底部审批按钮。

## 共用桌面详情组件（2026-09-29）

工具调用与对话内容统一由 `src/session-detail.ts` 渲染，Cursor、Claude Code、Codex 等来源均使用这一组件。继续使用现有 `detail-shell`、`activity-item` 和 `output-entry` 布局与样式，不添加 Cursor 专属详情样式。来源名称由现有助手配置提供，避免其他助手的回复被标成 Trae。

同一会话刷新时保留工具展开状态、键盘焦点和两块内容的阅读位置；对话原本位于底部时跟随新输出，切换会话后重置阅读状态。`verify-session-detail.mjs` 使用独立的 Claude 与 Codex 数据对照 Cursor 的结构、字体、背景、边距和圆角，并覆盖长记录刷新、空状态和跨会话切换。

## 验证

2026-09-28 的前端测试共 329 项通过。2026-09-29 继续完成以下检查：

- `verify-session-detail.mjs` 检查桌面与 LAN 的共用结构及样式、参数展开、刷新保留展开状态、Markdown 文本安全、问答与计划历史、待处理请求、返回列表、手机亮色布局和只读状态。
- 回归检查先复现 LAN 从 Claude 切换到 Cursor 后残留两个审批按钮的问题；清空共用操作按钮后检查通过。
- TypeScript 与桌面、LAN 生产构建通过。
- Release EXE 构建通过，生成程序通过 7 项隔离 CLI 检查；产物为 `src-tauri/target/release/codecraft-tauri.exe`。

这些界面检查使用隔离的合成快照，不提交真实审批。截图保存在 `scripts/cursor-probe/runs/session-detail/`。

## 审批页面滞留修复（2026-09-29）

原先三类审批提交后只刷新表单、禁用按钮，没有退出桌面审批页；自动显示下一项又要求当前处于会话列表，因此 Cursor 已继续执行时，旧页面仍会占住界面。

现在提交接口确认已记录决定后，桌面和 LAN 立即结束该次人工等待，不等待后续快照读取、决定交付或工具执行完成。客户端按请求、应用实例、请求版本及内容摘要保留确认，防止在途旧快照重新打开已处理请求；更新版本的请求不会继承旧确认。其他端处理、过期、取消、替换、断连终态及请求移除，也会触发桌面页面退出。临时离线与提交失败保留表单，新请求和用户主动导航不受旧响应影响。

`verify-review-lifecycle.mjs` 验证上述行为，包括三类页面在刷新不返回时的退出、连续审批、同 ID 新版本收到旧响应，以及 LAN 页面退出。此轮 333 项前端测试、该生命周期检查和原会话详情检查通过。

从 `CodeCraft-tauri` 运行（Playwright 未安装在项目时，设置 `PLAYWRIGHT_MODULE` 与 `PLAYWRIGHT_CHROMIUM_EXECUTABLE`）：

```powershell
. ./scripts/cursor-probe/limit-cpu.ps1
node scripts/cursor-probe/verify-session-detail.mjs
npm run build
cargo build --locked --release --manifest-path src-tauri/Cargo.toml
node scripts/cursor-probe/smoke-product.mjs src-tauri/target/release/codecraft-tauri.exe
```
