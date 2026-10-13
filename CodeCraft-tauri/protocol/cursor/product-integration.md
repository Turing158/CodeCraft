# Cursor 产品接入状态（2026-09-24，v5）

已完成公开 Hook 工具审批、独立 MCP 问答与计划审阅、桌面/LAN 共享表单，以及随 Hook 安装维护的项目接入与引导规则。2026-09-23 用户明确同意改用独立交互，将 Cursor 原生双端审批同步移出本版验收；约定见 [v5 范围](v5-contract.md)。旧版原生接口与身份绑定失败证据保留，不改记为通过。

## 使用

1. 运行 `src-tauri/target/release/codecraft-tauri.exe`，进入桌面设置 → Hook，点击 Cursor 安装。统一入口自动检测 Cursor，安装或修复用户级 Hook 和 MCP 配置；设置中不再提供单独的 Cursor 配置与诊断面板。
2. 安装同时读取 Cursor 本机保存的当前窗口项目，持久化项目文件身份并写入 `.cursor/rules/codecraft-workflow.mdc`；重新安装时修复已启用项目缺失的规则。历史工作区、远程目录和已明确停用的项目不会自动启用。先在 Cursor 打开新项目，再重新安装 Hook 即可接入该项目。冲突文件保留并通过 Hook 安装错误提示报告。
3. 让 Cursor 加载用户级 Hook/MCP 配置。MCP 服务名为 `codecraft_cursor`；调用 `codecraft_list_projects` 选择已启用项目，再用 `codecraft_start_workflow` 创建本任务的独立工作流。
4. `codecraft_ask_user` 和 `codecraft_review_plan` 返回待处理请求。调用方约每秒以 `codecraft_get_result` 领取结果；pending 不是许可。桌面或有审批权限的 LAN 连接可提交回答、计划决定和修改意见。答案与决定仅交付一次。
5. 放弃单个请求用 `codecraft_cancel_request`；任务完成或放弃后用 `codecraft_end_workflow` 释放工作流。重新连接或送达状态未知时，应以新业务 ID 发起新的人工审阅。

## 行为和边界

- 对 Cursor 实际发出的普通工具 `preToolUse` 事件审批；MCP 在具备服务器信息的 `beforeMCPExecution` 审批，跳过无法识别服务器的 `preToolUse` MCP 事件，不注入或修改工具参数。复用手动、风险、自动策略；问答和计划始终由人决定。3.21.18 / Agent 已实测 Shell 拒绝，Read 未产生对应审批事件，不能承诺覆盖全部原生工具。
- Hook 等待最多约 45 秒，辅助进程有界结束。故障、离线或超时遵循既定 D3-B，返回对应事件的合法放行响应。允许本次也不能覆盖 Cursor、沙箱或其他 Hook 的权限。界面区分决定已记录、已经写出和工具执行结果未确认。
- 独立 MCP 工作流绑定经过 Windows 进程身份与 Cursor 祖先核验的辅助进程连接，以及用户已启用的项目文件身份。工作流 ID 不是 Cursor 原生对话 ID，界面不会按名称或最近会话猜配。问答/计划人工截止为五分钟。
- 请求使用严格 schema、业务 ID 和内容摘要；重复内容不延长截止时间、不重复交付结果，改参冲突。计划替换检查版本及被替换业务 ID，两端相反决定只有一个能成功。项目停用、目录替换、断连及重启拒绝旧提交；stdout 不确定时不重发。
- 关闭工作流会取消未完成请求，保留已交付结果和送达未知状态。历史清理保留待处理请求及活动工作流的幂等记录；空工作流关闭后释放内部会话。文件通道清理已读取的响应回执，同时保留序列重放屏障。
- 桌面/LAN 共用三类表单及后端提交。LAN 保留认证、审批权限、自定义 CSRF 请求头、大小限制；不暴露安装或项目管理接口。安装、修复、卸载统一使用桌面设置 → Hook 的 Cursor 入口，项目引导规则随安装维护。
- 用户级 `hooks.json`、`mcp.json`、项目引导规则、项目登记与 CodeCraft 配置在同一个恢复事务中提交，修改前保留唯一 `.bak`，保留第三方条目并拒绝覆盖冲突内容。卸载同时移除托管规则和项目登记；旧版本卸载留下的规则在升级启动时补清理。安装状态也检查已启用项目的规则完整性。卸载后会话和待处理事项不再展示，已采集历史仍保留在本机；重新安装时重新登记当前打开的项目。
- **不提供原生审批同步和跨原生工具计划门禁**。`nativeShared` 始终为 false。独立计划批准仅是本次人工审阅结果；不承诺拦截未关联的 Shell、其他 MCP 或 Cursor Build。既有采集仅含接入后的观察记录，不导入早期原生历史。

## 本轮验证

2026-09-23—24 在 Windows 上限制为两个逻辑 CPU，Rust 使用 `CARGO_BUILD_JOBS=2`。以下累计 **463 项自动测试/检查通过**：

| 验证 | 结果 |
| --- | --- |
| 正式 Cursor 核心 Rust 测试 | 117 项 |
| 独立探针 Rust 测试 | 4 项 |
| 协议、证据及源码检查 | 18 项 |
| 前端 Vitest | 53 个文件，316 项 |
| Tauri LAN 快照代次/版本回归 | 1 项 |
| 正式 Release EXE CLI 检查 | 7 项 |

另通过核心/探针 fmt 和严格 Clippy、TypeScript、桌面与 LAN Vite 生产构建、Tauri Release 构建及 `git diff --check`。主程序仍有既有 dead-code/linker 警告，Cursor 核心与探针严格 Clippy 无警告。

新增边界验证包括：关闭时保留已交付和未知结果；140 次空工作流关闭后不积累内部会话；520 次历史请求后保留活动请求和幂等记录；计划替换拒绝旧表单及改参；通道回执清理后拒绝旧序列重放。核心还覆盖 Windows 子进程握手 → 生产 broker → 问答 → 一次领取，以及 broker 撤销后空闲 MCP 退出。

实际浏览器检查了桌面必答校验、单选、多选和文本提交，提交后的决定展示与只读表单；LAN 检查了工具拒绝、完整计划正文/阶段/任务、修改意见必填、修改结果记录及按钮禁用。页面使用明确标识的本地合成样例。简体中文、繁体中文与英文文案同步补齐。

复现命令（PowerShell，从 `CodeCraft-tauri` 运行）：

```powershell
. ./scripts/cursor-probe/limit-cpu.ps1
./scripts/cursor-probe/check.ps1
npx vitest run --maxWorkers=2
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib lan_server::tests::cursor_changes_and_epoch_restarts_wake_lan_streams -- --test-threads=2
cargo build --locked --release --manifest-path src-tauri/Cargo.toml
node scripts/cursor-probe/smoke-product.mjs src-tauri/target/release/codecraft-tauri.exe
```

DEV 合成预览：`/?cursor=1&interactions=1` 和 `/web/lan/index.html?cursor=1&interactions=1`。预览不会写真实配置，也不会交付真实工具许可。

## 实机证据范围

已完成 **Cursor 3.21.18 / Windows / Agent → 正式 Release EXE → LAN 人工提交 → Cursor** 的在线验收：MCP 加载、工具允许、Shell 拒绝、问答及自由文本回传、结果只领取一次、计划修改反馈、修订版本 2 的批准及工作流关闭。完整记录、请求状态和客户端原文见 [3.21.18 在线验收](3.21.18/v5-online-report.md)。桌面共享表单此前已完成合成 UI 检查；不把它记为桌面侧真实提交证据。全部配置位于项目内隔离目录，未安装到真实用户 Cursor 配置。

实机测试还暴露并修复 LAN 返回列表后被快照自动拉回详情、请求列表刷新重建按钮导致点击丢失的问题。Read 未触发工具审批、Hook 等待超时和首次问答过期均如实保留，不改记为通过。最终 EXE 的 760px/390px 基本导航及手机 Escape 已补验；当时无待处理请求，不能代替待审批导航回归。

晚间续验恢复了被 Cursor 停用的隔离项目 MCP 来源，确认 Connected 和 7 个工具。Plan 模式随后产生真实 `beforeMCPExecution`，但审批过期且 Cursor 提示 `You've hit your usage limit`，没有完成独立计划审阅。桌面真实提交、待审批导航及 Ask/Plan 完整回传仍未完成。当前状态与恢复条件见 [最终补验记录](3.21.18/v5-resumed-validation-20260924.json)。旧版原生同步不在 v5 范围内。

已保存的 Cursor 3.21.16 / Agent 实验仍表明：同名 MCP 的 `preToolUse` 无法绑定具体服务器；普通扩展调用原生运行时被拒绝为 `BUILTIN_ONLY`。见 [MCP 身份实测](3.21.16/Agent/report.md) 与 [扩展接口实测](3.21.16/extension-access-report.md)。这阻止的是旧 v4 原生同步方案；v5 使用单独身份的公开 Hook/MCP 工作流。未实测的客户端版本或模式不声明通过。

## 发布产物

- 文件：`src-tauri/target/release/codecraft-tauri.exe`
- SHA-256：`9F1254D69328308FAA3E5989124B61B845D2F56E52940317D1937D0498D5063B`
- 最终 Release 构建与 7 项正式 EXE CLI 检查通过，晚间复核产物哈希一致。窄屏基本导航已补验；真实待审批导航、桌面提交及 Ask/Plan 完整回传仍待完成，当前受 Cursor 可用额度限制。此前完整在线流程的 EXE 与证据哈希单独记录在验收报告中。
- 此轮保留工作区改动，未提交 Git commit。
