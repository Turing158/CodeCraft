# Trae：自动工具审批与原生只读提醒

当前交互按 2026-09-19 用户确认调整：工具调用在 CodeCraft 审批并回传；原生问题和 Plan/Spec 自动打开对应的只读页面，底部提供“前往 Trae 中处理”。桌面复用 WorkBuddy 所使用的问题页和计划页。会话列表不再有“任务与配置”，无需创建 CodeCraft 任务、复制启动指令或安装 MCP 服务。

## 交互行为

| Trae 事件 | CodeCraft 行为 | 回传能力 |
| --- | --- | --- |
| 普通工具 PreToolUse | 自动进入工具审批页，显示工具及完整参数，提供允许、拒绝、交回 Trae | 通过原生 Hook 回传对应决定 |
| AskUserQuestion PreToolUse | 读取实际提供的题目/选项，自动进入只读问题页 | 不代答，不阻塞在工具审批页；由 Trae 继续原生交互 |
| ask_user_question 通知 | 自动显示只读问题提醒，与已关联工具事件去重 | 不回传答案 |
| document_review 通知 | 自动显示只读计划页 | 不回传批准、拒绝或修改意见 |

问题选项只供查看，不能选择或提交；计划页没有批准和修改输入。两页底部均为“前往 Trae 中处理”。桌面点击后切换到可唯一识别的 Trae CN 窗口；多窗口无法确定时提示错误，不猜选。LAN 保留相同按钮，提示到运行 Trae 的设备处理，沿用 WorkBuddy 的跨设备行为。

自动展开遵循 CodeCraft 全局审批自动展开设置。同一请求不会因为轮询重复弹出；返回列表后可以打开对应会话详情，通过“查看待处理事项”重新查看。当前交互结束后，下一条待处理交互继续显示。问题与计划只读状态不会因 LAN 写权限开启而改变。

## 会话详情

桌面端和 LAN 均可点击 Trae 会话查看已采集记录：用户提问、各轮最终回复，以及工具参数、结果和状态。工具详情可展开，收到新快照时保持当前会话内的展开状态；切换会话不会混入另一会话的内容。从详情进入审批或只读问题/计划页后，返回时仍显示对应会话。

记录来自接入后的 UserPromptSubmit、Stop、PreToolUse、PostToolUse Hook，不导入 Trae 接入前的完整历史，也不提供逐字流式输出。记录随 CodeCraft 状态保存，兼容升级前仅有最后一条回复的数据。重复的最终回复不会在同一轮连续追加；不同轮的相同回复仍分别保留。工具未返回结果或被中断时显示结果未知，不把空 PostToolUse 当作执行成功；重启后未完成的工具同样标记为未知。

每个会话最多保存 256 条消息和 200 条工具记录，两类内容分别限制约 512 KiB；单条消息保留前 32,768 个 Unicode 字符，工具参数和结果分别保留前 8,192 个字符。超过限制时显示截断说明。沿用现有会话清理策略：超过 24 小时未更新且没有关联任务、请求或授权需要保留的会话会被清理，详情页不是永久聊天档案。

## 内容及生命周期

问题内容来自实际的 `tool_input.questions`，支持展示题目、选项、说明和多个问题。只收到通知时显示通知文字；没有完整内容时明确引导前往 Trae，不编造题目。

计划正文优先读取通知明确提供的 `document_path` / `documentPath`（或 `tool_input.document_path`），相对路径以事件 cwd 为基准。通知未提供路径时，关联**同一会话、当前轮次**中成功写入的计划文件：配对 Write/Edit 的 PreToolUse 与 PostToolUse，要求调用 ID、工具、cwd 和参数一致，返回的 `changes` 包含该文件及 `new_content`，且与磁盘正文相符。Trae 的空 PostToolUse 不视为写入成功。

最终路径必须位于该会话工作区的 `.trae/documents/` 或 `.trae/specs/` 内，文件与规范化正文均受 128 KiB 限制。同一个文件的后续成功编辑更新其记录；存在多份候选时显示无法确定的提示，不遍历目录或挑选最新文件。展示前再次验证内容哈希，避免读取其他会话或外部编辑覆盖后的内容。通知明确提供了无效路径时，不改用其他候选文件。

关联缓存仅保留在当前运行进程，新的用户输入、SessionStart、Stop、idle_prompt 及 CodeCraft 重启会清除它。缓存受数量限制，超限时停止自动关联。页面显示正文、路径及“当前会话本轮生成”的来源说明，仍只有“前往 Trae 中处理”，不会自动批准计划。旧通知不会从日志或目录补录，更新后需在同一轮重新生成计划并触发审阅。

相同工具 ID 的问题通知与工具事件合并并保留较完整题目；不把缺少关联 ID 的通知随意分配给多个问题中的一个。匹配的 PostToolUse 会清除对应问题提醒，新的用户输入、SessionStart、Stop、idle_prompt 会清除旧提醒；后续普通工具活动收起计划/未关联提醒。收起仅表示等待已结束或会话继续活动，不等于 CodeCraft 已替用户批准原生卡片。CodeCraft 重启后不重放旧原生提醒。

Trae 无工作区的普通 Chat 和 Solo 实测会发送 `agent_type: "chat"` 或 `"solo_agent"`、`cwd: "."`、`workspace_roots: []`。其消息提交、会话开始/结束与通知按观察事件处理；缺少项目路径、宿主信息或 CodeCraft 未连接都不会因此阻断正常消息。能够确认来源并连接时记录会话及只读通知，`idle_prompt` 将会话标记为空闲。不会用 CodeCraft 的进程目录补造工作区，也不会把这一例外用于工具批准；工具事件继续要求有效工作区与完整调用信息。同一会话失去原有工作区时，使尚未交付的旧批准失效。

## 安装与兼容状态

在设置的 Hook 集成中安装 Trae Hook。安装器只管理六个官方事件：SessionStart、UserPromptSubmit、PreToolUse、PostToolUse、Stop、Notification。外部配置改写前保存唯一 `*.bak`，保留用户 Hook 和未知字段；卸载只移除自己的条目。

**Trae CN 3.3.102 的工具审批已启用。** PreToolUse 创建 CodeCraft 审批请求，按全局审批模式处理；手动模式显示工具名和参数，等待允许、拒绝或交回 Trae。已停用的 MCP/任务方案的整体验证状态不再阻止当前工具审批。未知版本仍交回 Trae 原生确认。

Hook 配置期限为 150 秒，CodeCraft 预留 30 秒交付时间，最长等待用户 120 秒。`hookTimeoutSeconds` 表示实际安装的配置期限，`verifiedHookTimeoutSeconds` / `verifiedVersion` 只表示已采集的验证证据，两者分开记录。启用功能不等于宣布所有工具、原生交互及超时边界均已实机验证，范围见 [版本记录](3.3.102/README.md)。

旧 CodeCraft MCP 问答、计划、创建/控制任务的产品入口已停用，调用会明确失败。原生问题与计划无需 MCP。曾手动加入旧 `codecraft` MCP 服务时，请从 Trae 的 MCP 设置移除。核心中的旧协议代码和独立探针保留作为回归资料，不是当前产品的使用入口。

## 回传与故障处理

桌面 `trae_respond_permission` 与 LAN `POST /api/trae/permission` 共用严格后端校验。提交包含原样 target（App 代次、会话、请求版本等）及稳定 decisionId；失败重试复用 ID，其他端已决定或请求变化时拒绝旧决定。LAN 先校验认证、CSRF 和写权限。

Trae 沙箱中的 Hook 通过经过认证的本机回环连接交付事件，由 CodeCraft 桌面进程写入原有文件队列。Hook 仅读取受保护目录中的连接信息，不再尝试从沙箱向该目录写文件，避免 `Access denied / hit restricted` 导致事件丢失。连接仅监听 `127.0.0.1`，使用随机端口、随机凭据、App 代次与请求身份校验，并限制消息大小、等待时间和工作线程数量。该连接只能上报 Hook 事件或领取已有决定，不能创建用户批准或控制任务；桌面与 LAN 的审批仍走原有校验流程。

请求期限、文件队列、唯一状态写入者和持久化机制保留。Windows 数据目录仅授予当前用户访问权限。重启使旧允许失效；已经交付到 Trae 的允许不能假定可撤回，原生模式也可能要求二次确认。失联、未知来源和未验证的能力不会自动放行。此次修复不需要修改 Trae 沙箱规则或重新安装 Hook；更新并重启 CodeCraft 后，下一次 Trae 消息事件即可建立会话，不会导入此前丢失的聊天历史。

## 验证

在 `CodeCraft-tauri` 目录运行 `scripts/check-trae.ps1`，或使用 `-Full` 加上主库、前端和构建回归。脚本限制两核，设置 `CARGO_BUILD_JOBS=2`。

桌面合成预览：在两核 PowerShell 中运行 `node scripts/trae-desktop-preview.mjs`，打开输出的本地地址。独立 Vite 插件仅在该测试服务中注入模拟事件与回传，生产构建没有测试入口。按钮可触发工具、原生问题、原生计划、重复事件、结束事件及一次回传失败。

运行 `node scripts/verify-trae-desktop.mjs` 自动核对工具审批展开、参数显示以及允许/拒绝/交回 Trae 三个按钮，并检查桌面及 LAN 的多轮详情、刷新、会话切换和审批返回，具体 Playwright 配置见 [脚本说明](../../scripts/README.md)。该脚本只对上述合成预览提交模拟决定。

LAN 组件合成预览位于 `scripts/trae-ui-fixture.html`。检查问题/计划只有跳转按钮、工具三种决定、只读访问不可审批，以及结束后收起。所有模拟内容明确标为 synthetic。

本轮验证记录见 [native-review-verification.json](native-review-verification.json)。[旧产品验证记录](production-verification.json)保留为此前 MCP/任务方案的历史记录，不能作为当前功能清单。

无工作区 Chat 的 `Stopped by Hooks` 修复及生产程序输入回放见 [chat-workspace-fix-verification.json](chat-workspace-fix-verification.json)。可以运行 `node scripts/verify-trae-chat.mjs` 重放已脱敏的实机输入；`check-trae.ps1 -Full` 在构建后自动运行它。输入回放不代表完整工具审批实机验收。

沙箱写入被拒绝与无工作区 Solo 的后续修复见 [sandbox-session-verification.json](sandbox-session-verification.json)。`verify-trae-transport.mjs` 验证传输、认证与事件持久化，并支持使用本机原有 Trae 沙箱配置复测。2026-09-19 的真实 Trae Solo 对话已在原沙箱中完成 UserPromptSubmit、Stop、Notification 三个 Hook，退出码均为 0；CodeCraft 保存了对应会话和 `CODECRAFT_SESSION_OK` 回复。此项记录验证消息链路，不代表完整工具审批或桌面列表画面已实机核验。

随后修复了正式能力配置阻止创建工具审批的问题，见 [tool-approval-fix-verification.json](tool-approval-fix-verification.json) 与 [脱敏实机证据](3.3.102/fixtures/tool-approval-delivery.json)。真实 Trae CN 3.3.102 已观察到 CodeCraft 自动弹出审批、会话列表及“Hook 已连接”。允许和拒绝均经正常桌面决定 IPC 回传：允许后 Read 返回文件内容，Trae 回复 `ALLOW_CONFIRMED`；拒绝后回复 `DENY_CONFIRMED`，两条请求均为已交付。真实决定由严格限定测试会话与只读文件的验收脚本提交，原生窗口按钮未自动点击；按钮点击另在合成浏览器预览验证。

两次真实等待超时均在约 120 秒后交回 Trae 原生确认。已交回或过期的卡片不能迁回 CodeCraft，复测时应触发新的工具调用。当前实机范围限于 Read；写入/命令副作用、完整重启并发矩阵及其他版本仍需单独验证。Trae 在拒绝 Read 后也会发送内容为空的 PostToolUse，不能仅凭该事件判定工具成功执行。

计划正文关联修复见 [plan-association-verification.json](plan-association-verification.json)。已采集到真实 Write 成功返回计划路径和完整正文、随后 `document_review` 仅提供 NotifyUser 提示的输入序列；该序列已脱敏并纳入核心回归。界面自动化另外验证正文、路径、来源说明及只读跳转按钮。输入回放与合成界面验证的范围分别记录，不将它们表述为新的完整客户端验收。
