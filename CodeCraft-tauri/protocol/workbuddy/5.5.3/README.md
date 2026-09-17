# WorkBuddy 协议与只读交付包

**WorkBuddy 仅提供只读观察，审批统一在 WorkBuddy 中处理。** 参见 [实施状态](implementation-status.md)、[P0 实测决定](P0-decision.md)、[兼容矩阵](compatibility.md)、[排障回滚](troubleshooting.md)。

CodeCraft 已移除 ACP 托管会话、工具/问题/计划回传接口及 LAN 写入路由。桌面详情中的每张交互卡底部提供“前往WorkBuddy中处理”按钮，切换到已打开的 WorkBuddy 窗口；未找到窗口时提示先打开 WorkBuddy。局域网页面提供同名按钮，提示在运行 WorkBuddy 的设备上处理。

问题、计划、权限请求会打开对应的只读提醒页，底部按钮按请求所属的会话定位 WorkBuddy 窗口。点击跳转不会关闭提醒；收到原生完成、拒绝、取消或失败结果后自动关闭，结束本轮交互时清理剩余提醒。同一会话的多个请求依次展示，普通工具调用只记入会话详情。

5.5.3 是既有插件基线标签；实际 CLI 2.137.1，Desktop 版本文件 37.10.3-24，不应混作同一版本。

## 证据分层

- fixtures/runtime/*.runtime.json：13 组真实 bundled CLI 脱敏 Hook/原生授权证据，含采集时 SHA-256；本地确定性模型和隔离配置。**未覆盖 Desktop UI。**
- fixtures 根目录全部旧文件：文档/解析器形状样例；**包括未带 .synthetic 后缀的三个 JSON，也不是运行证明。**
- schema.json：只读捕获模型说明，不是完整官方 schema，更不证明答案/计划字段被支持。

生产插件始终返回空 JSON，不转发决定。原生规则仍可能自行允许只读工具，不表示 CodeCraft 批准了它。

## 自动验证

在 CodeCraft-tauri 中打开 PowerShell，只限制当前验证 shell 及子进程：

~~~powershell
$verificationProcess = [System.Diagnostics.Process]::GetCurrentProcess()
$verificationProcess.ProcessorAffinity = [IntPtr]3
$env:CARGO_BUILD_JOBS = '2'
npm test -- --maxWorkers=2
npm run test:workbuddy-plugin
npm run test:workbuddy-fixtures
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=2
cargo build --manifest-path src-tauri/Cargo.toml
~~~

每个命令后检查 $LASTEXITCODE；测试通过不是双向验收通过。

可选浏览器检查：在同样的两核 shell 中运行 Vite（127.0.0.1:1421），再执行 `node scripts/verify-workbuddy-ui.mjs`。需可用的 Playwright 和 Chromium；可用 PLAYWRIGHT_MODULE 指向其 index.mjs、PLAYWRIGHT_CHROMIUM_EXECUTABLE 指向浏览器。测试只向本地开发模块注入访问点，生产代码没有调试入口；截图保存在 .workbuddy-verification/ui。它验证 CodeCraft 前端，不等于 WorkBuddy Desktop 端到端测试。

## 重采真实 CLI

需本机可信 bundled CLI 和 Node；默认入口 D:/software/WorkBuddy/resources/app.asar.unpacked/cli/bin/codebuddy，可用 WORKBUDDY_CLI_ENTRY 覆盖。延续两核 shell：

~~~powershell
node scripts/workbuddy-capture.mjs tool-allow
node scripts/workbuddy-capture.mjs question-single
node scripts/workbuddy-capture.mjs plan-allow
~~~

其他场景：tool-deny、tool-ask、read、failure、question-multiple、question-text、plan-deny、timeout、fallback。普通工具 ACP 变体设置 $env:CODECRAFT_CAPTURE_TRANSPORT='acp'，完成后移除此变量。

所有测试文件位于 .workbuddy-verification/<scenario>-<random>/；云模型被本地 HTTP 替代，不批准原生交互。原始 capture.json 含运行时提示，不应共享。用本次生成的目录名导出：

~~~powershell
node scripts/export-workbuddy-fixtures.mjs <本次目录名>
~~~

导出仅写项目内协议目录，拒绝旧的非法 ExitPlanMode.plan 测例。policy 是测试意图，hooks[].response 才是实际 Hook 输出。判断结果必须结合工具结果、文件副作用和原生请求，不能只看退出码。
