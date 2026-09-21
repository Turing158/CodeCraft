# Trae CN 3.3.102：P0 验证记录

> 2026-09-19 交互调整：当前产品为工具审批回传 + 原生问题/Plan/Spec 自动只读提醒，见 [现行说明](../README.md)。下方 MCP 票据与显式任务实验保留为原方案记录，不再是当前用户操作流程。Read 允许/拒绝与等待超时已获有限实机证据；原生通知字段、写入副作用和完整兼容矩阵仍需验证。

状态：**工具审批已启用，MCP/任务入口已停用；完整兼容验证仍未完成。** 工具审批不再依赖旧 MCP/任务方案的整体验证状态，参见 [capabilities.json](capabilities.json)。配置期限与实测期限分别记录，不将功能启用标为全部验证通过。

计划正文关联修复：20:39 的真实输入表明，`document_review` 只包含 `Tool 'NotifyUser' requires user confirmation` 和审阅调用 ID；之前同一会话的 Write 事件及成功返回则包含 `.trae/documents/` 中的计划路径与正文。现在使用当前轮次配对的成功 Write/Edit 记录，在唯一候选且内容未变时补全只读计划页。见[脱敏原始输入](fixtures/plan-document-association.json)及[回放和界面验证记录](../plan-association-verification.json)。文件写入与审阅的工具 ID 不同，关联依据为会话、轮次和唯一计划文件，不将它标作 Trae 显式提供的文档链接。

2026-09-19 工具审批修复：真实 Read/Write 已到达 Hook，却因旧整体验证开关直接返回 `ask`，没有创建审批请求。启用匹配版本的独立工具审批能力后，已看到 CodeCraft 自动弹出的三按钮审批页、会话列表和连接标签。在原 Trae 沙箱下，以正常桌面决定 IPC 分别提交一次只读 Read 的允许和拒绝，Trae 对应回复 `ALLOW_CONFIRMED` / `DENY_CONFIRMED`，CodeCraft 均记录为 `delivered`；两次未提交决定的请求在约 120 秒后返回 `ask` 并出现 Trae 原生确认。拒绝后的 PostToolUse 内容为空，不能将它当作执行成功。见[运行证据](fixtures/tool-approval-delivery.json)和[本轮验证记录](../tool-approval-fix-verification.json)。

本次真实决定由严格限定测试会话与文件的脚本提交，并非自动点击原生 CodeCraft 窗口；三个按钮的点击与回传在独立合成浏览器预览中通过。没有进行写入副作用、全部工具或完整重启/并发矩阵验证，`verifiedVersion` 与完整验证标志保持未通过。以下按时间保留早期证据，其当时的能力状态不代表上述当前状态。

2026-09-19 新增部分实机证据：用户普通 Chat 触发的 UserPromptSubmit 输入包含 `cwd: "."`、空工作区列表，原实现误返回 `block`，客户端显示 `Stopped by Hooks`。已保存[脱敏输入](fixtures/workspace-less-chat.json)，修复普通 Chat 观察事件的处理，并通过生产 Hook 程序回放。该记录仅证明已观察到的 Chat 输入形态与对应修复，不代表工具允许/拒绝/回传已完成实机验收，详见[修复验证记录](../chat-workspace-fix-verification.json)。

2026-09-19 后续实机修复：原文件传输在 `ExecEnv: sandbox` 中写 CodeCraft 数据目录时被拒绝，会话事件因此丢失。改用认证回环传输后，保留原沙箱配置，真实 Solo 对话的 UserPromptSubmit、Stop、Notification 全部退出 0 并返回 `{}`，Trae 回复 `CODECRAFT_SESSION_OK`，CodeCraft 持久化了同一会话和回复。另以原沙箱配置验证了无工作区 Chat、Solo 事件，并补齐 Solo 的观察事件处理。见[脱敏运行证据](fixtures/sandbox-session-delivery.json)及[验证记录](../sandbox-session-verification.json)。桌面自动化未能识别 CodeCraft 悬浮窗口，因此列表画面未作实机通过声明；本次没有更新工具审批能力开关。

2026-09-18 只读核对本机安装：`product.json.appVersion=3.3.102`、基座版本 `1.107.1`。`trae-cn.cmd --help` 成功，列出 chat、`--add-mcp` 等入口；它不证明用户已经登录、启用 Hook 或 MCP。采集时没有运行中的 Trae CN 进程，未发现 `%USERPROFILE%/.trae-cn` 和 `%APPDATA%/Trae CN/User` 用户配置。当时工具环境没有可操作 Trae 原生窗口的能力，因此未启动真实智能体会话，未写全局配置，也未采集客户端工具执行结果。

## 已实现的 P0 工具

[独立 Rust 探针](../../../scripts/trae-probe/README.md)提供六事件采集、固定事件校验、Windows 祖先进程证据、隔离项目配置、文件协调器、官方 MCP SDK 服务、票据完整性验证、会话/轮次隔离、显式启动绑定、重复请求、取消和 flush 后交付确认。提供写出前延迟、flush 后延迟、丢弃确认的故障开关。

自动化测试只证明 CodeCraft 探针自身的协议行为。它们不能确认 Trae 是否注入票据、是否在 Hook 后校验输入、是否保留延迟确认参数、是否触发稳定事件，以及是否真的阻止文件写入。

本轮结果：10 项 Rust 单元测试、13 组真实探针子进程/MCP stdio 场景通过，探针构建、Clippy（全部 target、警告视为错误）、rustfmt 和 PowerShell 脚本语法检查通过。观察到本次构建与探针进程的 Windows 亲和性掩码均为 `3`，构建并发为 `2`。完整检查范围记录在 [synthetic-verification.json](synthetic-verification.json)。上述记录属于 P0 探针检查。2026-09-19 已完成正式 Rust 核心、桌面和 LAN 接入，并完成主库回归、前端回归、生产进程验证及主程序构建；具体范围见 [产品验证记录](../production-verification.json) 和 [使用指南](../README.md)。产品自动化仍不构成 Trae 客户端证据。

## 真实运行清单

先运行 `scripts/trae-probe/prepare.ps1`，在 Trae 打开返回的独立 `project/`。保留原生运行方式和确认设置；启用该项目 Hook，并在 Trae MCP 管理界面加入预览中的 `codecraft_probe` 服务。若沙箱不可访问探针，记录失败，不自动切换本地运行。

| 阶段 | 实际操作 | 必须记录的结果 |
| --- | --- | --- |
| P0.1 | 分别运行 observe、allow、deny、ask、error、exit2、delay（例如 151000ms）；在 delay 时强制结束**本次** Hook | 六事件输入、事件顺序、返回/退出码、Trae 开始等待与结束时间；用隔离目录中的哨兵文件证明被拒绝 Write/RunCommand 未产生副作用 |
| P0.2 | case=inject，调用 `codecraft_probe_echo`，payload 含嵌套对象、数组、中文、引号、空值 | MCP 回显与原业务参数相同、只有顶层 bridgeTicket 被替换；Hook 捕获真实 session/tool_use_id；模型确实收到工具结果 |
| P0.3 | 保留 MCP 原生确认，分别立即确认和等待超过 10 分钟再确认；添加独立改写参数 Hook（修改前备份配置） | 票据保留/丢失、参数修改顺序、哈希不匹配明确失败、过期没有新票据；撤销仅属于实验的第二 Hook |
| P0.4 | 同项目两个 Trae 会话并发；重发相同调用、改变参数、重连 MCP | 不串会话、一次逻辑操作、跨连接票据拒绝；确认客户端重试是否复用 tool_use_id |
| P0.5 | `arm` 后只在其中一个会话发送完整 launchPrompt | 首个受保护工具前的 UserPromptSubmit 已完成绑定，additionalContext 中 planId/路径正确，重复码被阻止，另一会话不被绑定；探针绑定本身不是正式计划保护 |
| P0.6 | 新 Prompt、“继续”、重复 Prompt、Stop、恢复会话、App/Trae 重启 | 候选宿主 PID+创建时间连续性；真实 Prompt ID 是否存在；旧票据/交付不能恢复；未验证时采用保守失效 |
| P0.7 | 调用 `codecraft_probe_delivery`，inspect 获取 operationId，再 release；分别启用 before-write 延迟、after-flush 延迟和 drop-ack，并在对应阶段取消/强制结束 MCP | 仅匹配且未过期的确认产生 probeDeliveryConfirmed；有结果但无确认不激活；验证 Trae 客户端真实取消与等待时限 |
| P0.8 | 在隔离项目用 allow 放行一次测试写入，暂停原生确认，然后模拟计划失效/Stop 再确认 | 是否仍执行、是否存在原生取消/完成事件；不得把已经交给 Trae 的允许记为已撤回 |

该旧方案要求普通 Read/Write/Edit、RunCommand、原生 AskUserQuestion、文档通知和 MCP 分别采集，P0.2、P0.4、P0.5、P0.7 是其 MCP 工作流的必过项。现行工具审批的启用状态与各项验证证据分开维护，不再受已停用 MCP 工作流的门槛控制。

原生问题和 Plan/Spec 的答复仍由 Trae 接收。MCP 回显成功不能关闭或替代原生卡片，原生 ask/allow/deny 的副作用要在 Trae 侧检查。

## 证据格式与复测条件

不要把 `--synthetic` 输出放进运行 fixture。每份正式 runtime fixture 必须包括产品/基座版本、触发步骤、运行方式、脱敏原始输入/输出、退出码、事件顺序、客户端实际行为和副作用断言；时间与关联 ID 应保留可比较关系。用一致代号替换用户名、真实路径和会话 ID，移除票据、启动码及不相关提示内容。探针只捕获原始数据，不自动把任何 fixture 标为通过。

正式接入代码现已包括全局安装/修复/卸载及 Claude 去重、主 App 唯一写入者和恢复、工具策略与单次允许、生产 MCP 问答/计划契约、计划正文/路径校验、任务生命周期及桌面/LAN 全流程（MCP/任务产品入口现已停用）。生产核心与 P0 探针分开构建和验证；P0 协调器仍是独立实验设施。正式工具审批已经启用，Read 的有限实机结果见本文开头，完整兼容验证仍未完成。

上面的 P0.1–P0.8 表保留为旧方案的实验清单。当前工具审批复测使用已安装的六事件 Hook，不需要添加 MCP 服务；分别核对界面弹出、允许、拒绝、交回 Trae、超时、重启和重复请求。能力启用与各项运行证据分别维护。

参考官方协议（2026-09-18 再次读取）：[Hook 配置详解](https://docs.trae.cn/ide_hook-configuration-reference.md)。协议规定不等于本机运行验证。
