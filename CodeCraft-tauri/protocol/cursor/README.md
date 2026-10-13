# Cursor 适配实施记录

2026-09-21：已接续 v4 实施并完成两轮 Cursor 3.21.16 / Agent 同名 MCP 实机实验。**P0-03 失败：候选 Hook 没有可区分服务器的身份。** 独立探针、协议基础和证据校验已落地；完整 P0 与 P1—P6 产品功能未完成，四类能力均未启用。

2026-09-22：补齐原生审批内部回调链与内置扩展访问限制的源码证据，修正版本检查脚本在未命中时仍输出旧结论的问题；11 项 Rust 测试、16 项 Node 检查通过，之前中断的交付核对已完成。当前版本仍不满足 v4 全量接入前提。

2026-09-24 最新产品接入：经用户授权修订为 [v5 独立交互](v5-contract.md)，已接通公开 Hook 工具审批、独立 MCP 问答/计划、项目启停及桌面/LAN 表单。累计 463 项自动测试/检查通过，并完成 [Cursor 3.21.18 / Agent 的生产 EXE 与 LAN 在线验收](3.21.18/v5-online-report.md)：工具允许/拒绝、问答回传、唯一领取、计划反馈与修订批准、工作流关闭。原生双端同步移出本版范围。使用方式、覆盖限制和产物见 [产品接入状态](product-integration.md)。下文 v4 阶段记录作为历史保留。

最新接口实测：已在隔离的真实 Cursor 扩展宿主中调用 `acquireAgentHostRuntime()`，普通开发扩展收到 `BUILTIN_ONLY`；可见内置执行扩展没有导出 API。公开 SDK Bridge 文档亦不承诺接管 IDE 现有审批。见 [运行记录与边界](3.21.16/extension-access-report.md)。这不是原生工具审批通过证据。

前一阶段实施：已新增 P1 独立业务 schema、请求/计划状态机、单写入者持久化、当前用户 ACL、Windows 项目文件身份、一次性票据事务、按代隔离文件通道、可恢复安装日志和 observedOnly 会话存储。87 项 Rust 测试、17 项 Node 检查、探针构建、fmt 和严格 Clippy 均通过。该段为此前探针阶段基线；当前产品接入见上方更新，Cursor 实机兼容结论不变。

- [探针与复现步骤](../../scripts/cursor-probe/README.md)
- [P1 工作流核心：实现、数据契约、验证与剩余工作](codecraft-v1/README.md)
- [Cursor 3.21.16 Agent 实机结论](3.21.16/Agent/report.md)
- [3.21.16 原生接口追踪与本轮交付状态](3.21.16/native-interface-report.md)
- [3.21.16 原生接口源码记录](3.21.16/native-interface-analysis.json)
- [3.21.16 脱敏运行证据](3.21.16/Agent/fixtures/mcp-identity-collision.json)
- [3.21.16 Ask 状态](3.21.16/Ask/report.md) / [Plan 状态](3.21.16/Plan/report.md)
- [Cursor 3.21.13 Agent 报告](3.21.13/Agent/report.md)
- [Ask 报告](3.21.13/Ask/report.md)
- [Plan 报告](3.21.13/Plan/report.md)
- [只读源码分析](3.21.13/source-analysis.json)

能力结论按版本／模式／工具保存，不能用安装状态代替。`sourceAnalysis`、`runtimePreflight`、`probeCapture` 和 `synthetic` 均不计为 P0 通过；只有经过审核的真实运行 fixture 可以作为 `cursorRuntime` 证据。捕获范围为 `observedOnly`。

登录已由用户完成，旧版首次登录观察保留为历史。当前阻塞是实测协议前提：两台服务器使用完全相同参数时，`preToolUse` 载荷除调用 ID 外完全一致；服务器名在不能公开修改输入的 `beforeMCPExecution` 阶段才出现。原生审批内部方法也尚未形成经验证的双向接入接口。后续须取得满足 v4 的身份绑定及原生控制接口证据；不能按工具名、正文或最近会话放宽识别。

证据归属和证据结论分开校验：`checkIds` 指定对应验收项，`checkResults` 指定每项 `passed / failed / notRun`。外层兼容记录误写 passed 时，失败或缺少结论的证据仍被拒绝。
