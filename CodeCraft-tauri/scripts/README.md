# Scripts

本目录存放 CodeCraft 的打包、协议验证、测试夹具采集和导出脚本。

## Trae 产品回归

运行 `check-trae.ps1` 检查生产 Rust 核心、Clippy、格式、DTO、MCP 子进程场景和认证回环传输；`check-trae.ps1 -Full` 追加独立探针、主库全部回归、前端测试以及前后端构建。脚本限制为两个逻辑 CPU。`generate-trae-types.mjs` 从版本化 Schema 生成 TypeScript DTO，`--check` 仅核对一致性。`trae-desktop-preview.mjs` 提供桌面自动审批页的隔离测试服务；`trae-ui-fixture.html` 提供 LAN 只读问题/计划与工具审批的合成预览，均不发送真实批准。安装、状态、恢复和验收边界见 [Trae 集成指南](../protocol/trae/README.md)。

`verify-trae-chat.mjs` 将脱敏的真实无工作区 Chat 输入及对应 Solo 变体回放到构建后的 Hook 可执行文件，验证普通消息不被阻断以及工具校验仍然生效。默认使用 `src-tauri/target/debug/codecraft-tauri.exe`，也可将其他构建的绝对路径作为第一个参数。`check-trae.ps1 -Full` 已在构建完成后执行这项回放；它不连接真实 Trae 智能体，也不执行工具。

`verify-trae-desktop.mjs` 对 `trae-desktop-preview.mjs` 检查 Write 审批自动展开、完整参数和允许/拒绝/交回 Trae 三个按钮，确认没有“始终允许”；另验证关联计划的正文、路径、来源以及只读跳转，确保没有计划批准/修改入口。会话详情检查覆盖多轮角色、工具参数与结果、刷新保留展开状态、审批返回、会话隔离与移除，并在 390px LAN 页面复核详情和刷新。截图保存在 `src-tauri/trae-core/target/desktop-ui/`，包括 `tool-approval.png`、`plan-association.png`、`session-detail.png` 和 `lan-session-detail.png`。两端均为合成数据；不会连接真实 Trae 或提交真实批准。先在一个受两核限制的 PowerShell 中运行 `node scripts/trae-desktop-preview.mjs`，再在另一个受两核限制的 PowerShell 运行：

```powershell
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
node scripts/verify-trae-desktop.mjs
```

默认从项目环境加载 `playwright` 并使用其 Chromium。也可设置 `PLAYWRIGHT_MODULE` 为已有 Playwright 的 `index.mjs` 绝对路径、`PLAYWRIGHT_CHROMIUM_EXECUTABLE` 为已有 Edge/Chromium 可执行文件路径。`CODECRAFT_UI_TEST_URL` 默认为 `http://127.0.0.1:1420/`，仅接受 `127.0.0.1` 地址。此项浏览器检查需单独启动预览，不包含在 `check-trae.ps1` 中。本轮正式配置、合成界面与有限实机验收的范围见 [工具审批修复记录](../protocol/trae/tool-approval-fix-verification.json)。

`verify-trae-transport.mjs` 在隔离目录中启动生产核心测试宿主，验证回环事件交付、去重、凭据、代次、消息限制和已有审批决定的领取流程。普通运行包含 8 组场景；传入现有 Trae 沙箱配置后，追加 Chat、Solo 两组沙箱内传输检查。脚本只读现有配置，在隔离测试目录内保存日志并清理，不更改沙箱规则；审批是模拟数据，不执行工具。手动运行前限制两核，并先构建测试宿主：

追加 `--bundled` 使用正式发布的 Trae 3.3.102 能力配置，防止测试专用能力全部开启掩盖生产审批被关闭的问题。`check-trae.ps1` 同时运行默认配置与正式配置的传输回归；该参数也可与沙箱参数组合使用。

```powershell
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
cargo build --locked --manifest-path src-tauri/trae-core/Cargo.toml --example bridge_test_host
node scripts/verify-trae-transport.mjs
```

可选沙箱验证：用本机实际路径和现有配置名替换以下占位符。脚本自动设置沙箱 SDK 所需的 `TRAE_SANDBOX_CLI_PATH`、`TRAE_SANDBOX_LOG_DIR`、`TRAE_SANDBOX_DUMP_DIR`：

```powershell
node scripts/verify-trae-transport.mjs --sandbox-exe '<Trae安装目录>/resources/app/modules/sandbox/trae-sandbox.exe' --sandbox-storage '<APPDATA>/Trae CN/ModularData/ai-agent/sandbox' --sandbox-config '<现有配置名，不含.json>'
```

## Trae P0 探针

[trae-probe/](trae-probe/README.md) 是独立 Rust Hook/MCP 协议验证程序。运行 `trae-probe/check.ps1` 执行受两核限制的自动检查，运行 `trae-probe/prepare.ps1` 创建独立客户端测试项目。它不启用正式 Trae 集成；真实验证门槛与当前进度见 [Trae CN 3.3.102](../protocol/trae/3.3.102/README.md)。

## 打包脚本

| 文件 | 用途 |
| --- | --- |
| `package-single-exe.ps1` | 构建不带安装器的便携式单文件 EXE。 |
| `package-single-exe-plus.ps1` | 构建增强版便携式单文件 EXE，包含额外的打包处理。 |

对应的 npm 命令：

```powershell
npm run package:single-exe
npm run package:single-exe-plus
```

## WorkBuddy 脚本

| 文件 | 用途 |
| --- | --- |
| `verify-workbuddy-plugin.mjs` | 验证 WorkBuddy 插件配置、输入校验和空响应行为。 |
| `verify-workbuddy-fixtures.mjs` | 验证 `protocol/workbuddy/5.5.3` 中的协议样例和运行时夹具。 |
| `verify-workbuddy-ui.mjs` | 使用浏览器自动化检查 WorkBuddy 只读提醒和局域网界面。 |
| `workbuddy-capture.mjs` | 使用隔离配置采集真实 WorkBuddy CLI 的协议事件。 |
| `workbuddy-fixture-hook.mjs` | 供 `workbuddy-capture.mjs` 调用的隔离测试 Hook。 |
| `export-workbuddy-fixtures.mjs` | 将隔离采集结果脱敏并导出到 `protocol/workbuddy/5.5.3/fixtures/runtime`。 |

常用命令：

```powershell
npm run test:workbuddy-plugin
npm run test:workbuddy-fixtures
node scripts/verify-workbuddy-ui.mjs
node scripts/workbuddy-capture.mjs tool-allow
node scripts/export-workbuddy-fixtures.mjs <capture-directory-name>
```

采集过程使用的临时文件位于 `.workbuddy-verification/`，该目录已被 `.gitignore` 忽略。导出后的脱敏协议夹具位于 `protocol/workbuddy/5.5.3/`，属于项目测试资料，应纳入版本控制。

## Kimi 脚本

| 文件 | 用途 |
| --- | --- |
| `kimi-protocol-probe.mjs` | 连接 Kimi Code CLI 和 CodeCraft Hook，采集并验证 Kimi 协议事件。 |

使用方式和前置条件见 `protocol/kimi/README.md`。该脚本只用于协议验证，不是 CodeCraft 运行时的启动依赖。

## 维护说明

删除本目录中的验证或采集脚本会使对应的 npm 命令、协议复现流程或文档示例失效。只运行程序或打包 EXE 时不需要手动执行这些脚本，但建议保留它们以便回归测试和重新生成协议夹具。

运行 Rust 构建或测试时，遵循项目约定设置 `$env:CARGO_BUILD_JOBS = '2'`，并将 Windows 子进程限制在两个逻辑核心内。
