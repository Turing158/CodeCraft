# Cursor 3.21.13 / Agent — P0 报告

日期：2026-09-21。平台：Windows x64。状态：**未执行完整 P0，所有能力未验证、未启用**。

本文件保留 3.21.13 的阶段记录：首次打开隔离项目时客户端提示登录，见 [runtime-preflight.json](../runtime-preflight.json)。用户随后完成登录，原会话继续到项目 MCP 初始化，但未完成同名工具实验。继续实施时客户端已更新到 3.21.16；新的实际工具调用和失败证据见 [3.21.16 Agent 报告](../../3.21.16/Agent/report.md)，不回填为旧版本的验证结果。

## 只读源码分析

[source-analysis.json](../source-analysis.json)保存客户端文件指纹、字节偏移和有限代码摘录。MCP 执行包装器使用 `MCP:<toolName>` 作为 `preToolUse.tool_name`，该调用传入的载荷没有服务器身份；`beforeMCPExecution` 才附加服务器字段。需要真实同名服务器实验验证影响，当前不注入票据、不豁免普通审批、不宣布 P0-03 失败或通过。

发现内部 `pendingDecisionProvider.requestApproval` 方法，但没有已验证的扩展承载方式、同一请求的外部提交、接受事件、状态查询和原子决定接口。nativeShared 保持未验证。模拟点击、基础 Hook 或自有面板不能代替它。

## 验收记录

P0-01—P0-11、V01—V34 的逐项状态见 [compatibility.json](compatibility.json)，当前全部为 `notRun`，`fixtures/` 没有真实工具事件。源码、启动观察与合成测试不自动写入 passed。

若后续仍需声明支持 3.21.13，必须在该构建重新核对工具可用性及实际配置来源，采集同名 MCP 调用、schema、身份和原生拒绝关联。身份成立后继续双事件注入及票据实验；原生接口可达后继续 P0-09—11。步骤见 [探针 README](../../../../scripts/cursor-probe/README.md)。

## 实施边界

本批交付采集器、只读源码检查、兼容性校验模型和协议测试。正式 Hook/MCP 分流、broker、生产票据、计划限制、安装升级、桌面/LAN 三类交互均尚未完成。原有产品与会话列表未改动。
