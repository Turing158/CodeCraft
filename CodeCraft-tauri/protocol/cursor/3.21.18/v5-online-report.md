# Cursor 3.21.18 / Agent：v5 独立交互在线验收

2026-09-24，在真实 Cursor Agent 对话中调用生产 Release EXE，通过 CodeCraft 的 LAN 表单提交决定，再检查 Cursor 实际收到的返回值。使用项目内的隔离配置；未修改真实用户级 Cursor 配置。所有构建、测试和测试客户端进程限制为两个逻辑 CPU。

## 已验证结果

| 场景 | 真实观察 |
| --- | --- |
| MCP 加载 | `codecraft_cursor` 连接成功，7 个工具加载；项目列表和独立工作流创建成功。 |
| MCP 工具许可 | `beforeMCPExecution` 生成带服务器名称的审批；LAN 允许后，生产 Hook 记录 `transport: written`，Cursor 继续调用。 |
| 普通工具拒绝 | `preToolUse` 收到 Shell 的 `echo CodeCraft-v5-deny-test`；LAN 拒绝后，Cursor 显示 `Rejected: Denied in CodeCraft`，没有执行或重试。 |
| 问答 | 新业务 ID `live-question-20260924-02` 的 `transport` 题选择 `pass`，并填写 `LAN 实机回传验证 20260924-02`。Cursor 收到相同题目 ID、选项 ID、自由文本和 `status: decided`。 |
| 一次性领取 | 再次领取该问答返回 `status: alreadyDelivered`、`transport: written`，不含答案或 `decision`。 |
| 计划修改 | 版本 1 完整显示正文、阶段及任务；LAN 提交 `revise` 和“请增加结果只领取一次的验证。”，Cursor 收到原文及版本 1 的内容摘要。 |
| 计划修订和批准 | 新请求声明 `baseRevision: 1`、`supersedesClientRequestId: live-plan-20260924-01`，界面显示版本 2 和新增任务。LAN 批准后，Cursor 收到 `approved`、版本 2 及新摘要。 |
| 工作流关闭 | `codecraft_end_workflow` 返回 `{"closed":true}`；已交付结果仍保留。 |
| 过期边界 | 首次问答虽已记录答案，但领取晚于五分钟截止；Cursor 返回 `HUMAN_WAIT_EXPIRED`，没有收到过期答案。后续成功验收使用新业务 ID。 |

原生对话记录与独立工作流分别显示，没有根据名称或时间合并。计划验收仅审阅测试内容，没有执行计划中的任务。

## 证据与复现

- [脱敏实机记录](v5-online-evidence.json)：保留请求 ID、真实决定/交付状态，以及 Cursor 的 `afterAgentResponse` 原文。凭据、用户配置、私有文件内容和环境变量没有写入此文件。
- 记录 SHA-256：`E53061863A1B7029543AE12BACE9F302ADBBBC495A53F79FF7A4FE45A7E86603`。
- 本轮在线验收 EXE SHA-256：`88B7806D51173C36AA523CB42008FFFFB18F74B211DA279FBD3D356AB0FD494A`。
- 隔离 run：`v5-live-b9b4ff3a-988f-4b85-b553-a26f018cbcf5`。项目内忽略目录保留 `accepted-state.json` 和 `accepted-history.json` 原始检查点。
- `scripts/cursor-probe/prepare-live.ps1 -Launch` 可准备新的生产配置和隔离项目。准备脚本本身不产生实机通过结论；需要真实 Cursor 发起调用，并核对其返回对象。

此记录使用 `cursorIndependentRuntime` 类型，不作为旧 v4 原生同步或 P0 能力通过 fixture。

## 本轮修复

1. 前序会话修复 Windows 心跳文件短暂共享冲突：读取及原子替换在 250 毫秒内有限重试，持续占用仍报错并保留原文件。修复版 MCP 在本轮完整在线流程中保持连接，没有重现 `FILE_UNREADABLE`。
2. LAN 显式返回列表（包括手机 Escape）后保留浏览意图，后续快照不再自动带回审批详情。
3. Cursor 请求列表按请求 ID 复用按钮，快照刷新不再重建全部 DOM。独立浏览器回归在 pointer-down 与 pointer-up 之间强制刷新，点击仍触发正确请求，持续刷新后键盘焦点仍保留。

前端 53 个文件、316 项测试及 TypeScript、桌面/LAN 生产构建已通过。最终 Release 构建于本轮完成，耗时 3 分 18 秒；7 项正式 EXE CLI 检查全部通过，保留既有 dead-code/linker 警告。

最终 EXE SHA-256：`9F1254D69328308FAA3E5989124B61B845D2F56E52940317D1937D0498D5063B`。最终产物在隔离 run `v5-live-97c98612-a4fe-424b-803f-c762344c75e9` 中继续验收，2026-09-24 晚间再次核对哈希一致。此前完整 MCP/问答/计划流程与原始证据保持原样。

## 最终 EXE 补验状态（2026-09-24 晚间）

**尚未全部完成。** [首次补验记录](v5-final-validation-20260924.json) 已证实：760×900 和 390×844 下可以打开详情、通过返回按钮回到列表并在快照刷新后保持列表；390×844 下 Escape 同样通过。当时没有待处理请求，因此只计为基本导航通过。

[续验记录](v5-resumed-validation-20260924.json) 记录了随后恢复同一环境的结果：

| 补验项 | 结果 |
| --- | --- |
| 项目 MCP 加载 | 初始为 Disabled；恢复该隔离项目现有 `codecraft_cursor` 开关后，界面显示 Connected、7 tools enabled。修改前已将 Cursor 状态文件备份为带时间戳的 `.bak`。 |
| Plan 模式真实调用 | 初次因 MCP 被停用而未发现工具。启用后，`codecraft_list_projects` 到达生产 `beforeMCPExecution`，服务器身份正确；但审批未在 45 秒内完成，记录为 `expired / notStarted / HUMAN_WAIT_EXPIRED`，没有人工决定。 |
| Cursor 运行限制 | 重试会话明确显示 `You've hit your usage limit`，历史记录该工具为 `failedObserved`。没有创建独立工作流或计划请求，不能计为 Plan 审阅回传通过。 |
| LAN 终态显示 | 在 390×844 页面看到上述审批已结束，允许和拒绝按钮禁用。真实请求待处理期间的返回列表回归仍未完成。 |
| 桌面真实提交 | 用户已展开设置，截图可见浮窗；电脑操作工具仍未返回可定位的 CodeCraft 窗口。未完成真实提交，用户随后收起设置。 |
| Ask 模式 | 未发起新的真实问答；完整回传仍未验收。 |

继续验收需要先恢复 Cursor 的可用额度。随后用新业务 ID 完成真实待审批导航、桌面提交以及 Plan/Ask 结果回传；每项都应核对 Cursor 实际收到的结果。MCP 配置文件仍位于隔离项目内，此次只恢复 Cursor 对该现有项目来源的启用状态，没有安装用户级 MCP 配置。

## 覆盖边界

- 完整成功流程仅覆盖 Cursor **3.21.18 / Windows / Agent**。Plan 已观察到真实 MCP Hook，但完整审阅回传未通过；Ask 尚未实测。详见上方补验状态。
- 本次原生 Read 成功读取隔离 `sample.txt`，没有采集到对应 `preToolUse` 工具审批。不能据 Shell 成功宣称所有原生工具均受 Hook 控制。
- 8 次 Hook 等待超时保留为过期记录，不能计作人工审批成功。Hook 故障/超时按既定 D3-B 放行；人工问答/计划超时不返回许可。
- 双端抢答、跨连接、项目停用、取消、断连、重启和 stdout 不确定等边界已有核心自动测试；本文件不把这些合成测试改记为 Cursor 在线实测。
- `nativeShared` 始终为 false；不提供 Cursor 原生双端审批同步或跨原生工具的计划门禁。旧版本失败证据保持不变。
