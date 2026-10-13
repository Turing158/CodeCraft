# Cursor 3.21.16 原生工具审批接口追踪

> 本文保留早期源码调查时的状态。生产采集、安装和桌面/LAN 展示此后已接入，见 [最新产品状态](../product-integration.md)；普通扩展访问拒绝现已取得 [真实运行记录](extension-access-report.md)。文末历史未完成项不作为当前产品进度。

2026-09-22，接续 P0 调查。**已找到内部工具审批回调链及扩展访问限制，尚未找到经过实机验证的 CodeCraft 接入入口。** P0-09—11 仍为 `notRun`，nativeShared 仍未支持。本次只读检查已安装文件，没有启动 Cursor、修改客户端或提交原生决定。

## 可复核的源码线索

[完整扫描记录](native-interface-analysis.json)保存五个文件的 SHA-256、命中位置与有限上下文。`offset` 是解码后 JavaScript 字符串的 UTF-16 code unit 偏移，不是文件字节偏移。所有记录均标为 `sourceAnalysis`、`cursorClientExecuted: false`。

| 层次 | 观察到的实现 | 可以证明的范围 |
| --- | --- | --- |
| Agent Host 工具执行 | MCP、文件等执行器调用 `pendingDecisionProvider.requestApproval`，MCP 参数包含 `providerIdentifier` 和 `toolCallId` | 内部执行阶段知道工具来源；不证明这些字段已暴露给候选 Hook 或普通扩展 |
| Agent Exec 审批适配 | `requestApproval` 将 `toolCallId` 与 `operation` 转交 `approvalHandler`；没有 toolCallId 的分支直接返回 approved | 存在内部回调；不能据函数存在推断所有工具都形成可共享的待审请求 |
| 扩展宿主到工作台 | `$requestAgentExecApproval` 路由至对应会话的 `approvalHandler` | 是请求审批的内部 RPC，不是向任意现有审批提交最终决定的公开入口 |
| 扩展 API 访问限制 | `cursor` / `cursorAgentHost` 等 proposal 对非内置扩展返回不可用；错误文案明确仅限内置扩展 | 普通扩展声明 `enabledApiProposals` 或添加启动参数，不能据此取得这些 API |
| Runtime 访问限制 | `acquireAgentHostRuntime` 还校验内置扩展身份；该构建绑定 `anysphere.cursor-agent-host` | 一个自有扩展不能直接套用内置扩展的 runtime 获取方式 |
| Agent Host 交互注册表 | `respondToInteraction` / `watchSessionInteractions`，以及 `ACCEPTED` / `ALREADY_RESOLVED` / `NOT_FOUND` 状态；响应时移除 pending 并发出 resolved 事件 | 是值得继续追踪的内部交互机制；尚未证明它承载上述工具审批、同一请求版本和两端等待 |
| 捆绑公共声明 | 未命中 `namespace cursor`、`nativeApprovalId`、`submitApproval` | 只表示该声明文件中没有这些具体标记，不代表所有可能接口都不存在 |

当前 Agent Host SHA-256 为 `28a304e5c466a347022d44a3d3f33760e15ac09aecdd2ee2053c914950db4535`，与 9 月 21 日 Agent 实验一致。新增文件的联合源码指纹为 `4c39d62b48ba069b9a78d11a39cb046cce2123c8d13f31f785b8744b136c7884`，只用于这次源码调查，不替换运行证据的 `clientFingerprint`。包元数据没有声明客户端架构，因此此次扫描记录为 null，不使用执行扫描的 Node 架构冒充客户端架构；既有实机报告的 Windows x64 身份保持原状。

## 对 v4 的影响

1. **候选 MCP 身份仍是已实测失败项。** 内部执行器知道服务器身份，不会使 `preToolUse` 自动获得该字段。9 月 21 日相同参数、不同服务器的碰撞证据及 P0-03 `failed` 结论保持有效。
2. **原生共享审批仍缺少可达性验证。** 找到内部回调、RPC 或交互状态名不等于 CodeCraft 能发现、提交并查询同一个原生工具审批。尤其不能将问答等交互注册表直接当成工具审批接口。
3. **源码访问限制不冒充实机失败。** 此次没有原生请求读取、两端提交、接受、取消、竞争或恢复实验，故 P0-09—11 保持 `notRun`。普通扩展直接使用上述内置 API 的承载假设已有明确反证。

继续 v4 全量实现，需要先给出可无歧义识别自有候选的有效字段/配置证据，以及能够访问同一工具审批请求的组件入口，再执行 P0 的绑定、终止、等待和双端竞争验证。如果选择替换固定绑定链路或缩小交付范围，应更新方案和验收矩阵；不能把基础 Hook 审批记作原生共享能力。独立 P1 模型仍可开发，但不能消除这些前置条件或启用产品功能。

## 本轮完成与剩余

本轮完成接口追踪、可重复的源码检查、版本变更时返回 unknown 的处理、相关回归和之前中断的交付核对。`inspect-client.mjs` 不再无条件输出旧版 MCP 身份判断；文件缺失、布局改变或只命中错误文案时，相关结论为 unknown，始终不产生能力通过记录。

两核限制下运行 `scripts/cursor-probe/check.ps1`：**11 项 Rust 测试、16 项 Node 检查全部通过，独立探针构建通过。** 其中 Node 检查包括 10 项合成进程测试、2 项既有运行证据回归及 4 项源码检查回归。这些通过结果不是新的 Cursor P0 通过证据。任务收尾时确认没有残留 Cursor 或 `codecraft-cursor-probe` 进程；原实验目录与脱敏证据保留。

P0 未完成项、正式 broker、票据与计划限制、安装恢复、桌面/LAN 三类交互仍未完成。当前交付是经过验证的探针及证据包，不能宣称 v4 全量接入完成。
