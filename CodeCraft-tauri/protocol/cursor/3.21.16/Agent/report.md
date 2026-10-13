# Cursor 3.21.16 / Agent — P0 实机报告

> 本文保留同名 MCP 实验时的状态与证据。生产采集/安装/只读界面此后已接入，见 [最新产品状态](../../product-integration.md)；原生普通扩展入口的后续运行结果见 [接口实测](../extension-access-report.md)。P0-03 失败结论不变。

2026-09-21，Windows x64。**P0-03 不通过；v4 自有 MCP 问答／计划的候选身份前提不成立，功能未启用。** 完整 P0 和 P1—P6 尚未完成，nativeShared 仍未验证。

## 已执行实验

本次接续 3.21.13 探针工作时，安装客户端已更新到 3.21.16，因此重新生成隔离项目并建立独立版本记录。用户已登录；旧报告中的首次登录阻塞不再适用于本次运行。

使用单根隔离项目，项目级 `.cursor/hooks.json` 和 `.cursor/mcp.json`，两台本地 stdio 测试服务器均在 Cursor 的项目来源下启用。没有用户级 Hook/MCP 文件，没有修改原生权限策略；工具界面显示既有 `Reads / Allow all`。探针仅返回摘要，不执行命令、不写工作区、不批准真实工作流。所有 Cursor、探针及检查进程限制在两个逻辑核心。

探针版本 `0.1.0`，本次运行 EXE SHA-256：`fd999946ea50a08f8bdcf3ff6c9a33eba72947fd7b7094c954b5fe646a50bbd4`。客户端、实际配置指纹及 55 条脱敏记录见 [运行证据](fixtures/mcp-identity-collision.json)。采集文件原有的 `probeCapture` 标记保留在证据封装内；外围审核记录才是 `cursorRuntime`。随机盐、原始会话 ID、邮箱、路径及原票据不进入提交文件。

| 实验 | 实际调用 | 事件序号 | 实际结果 |
| --- | --- | --- | --- |
| 不同参数 | 两台服务器的 `codecraft_probe_echo`，分别使用 `owned-a` / `collision-a`；随后读取 `sample.txt` | pre 19/20，beforeMCP 21/24，tools/call 22/25，post 27/28；Read 29/30 | 客户端收到 `owned`、`collision`；读取成功 |
| 完全相同参数 | 两台服务器均传入 `schemaVersion: 1`、`clientRequestId: identical-a`、`payload: {message: same}` | pre 44/48，beforeMCP 45/50，tools/call 46/51，post 49/53 | 客户端再次收到两种服务器标签；两个 pre 载荷除 `tool_use_id` 外完全相同 |

两次均未提供或注入 `bridgeTicket`，没有出现逐调用原生确认卡片。第一轮候选事件交错出现，不能依据采集发布序号或相邻时间猜测调用归属。第二轮客户端界面显示 `Worked for 13s`；捕获时间戳仅用于复核本次实验，不作为支持等待预算。

## P0-03 的失败依据

`preToolUse` 的真实工具名均为 `MCP:codecraft_probe_echo`。载荷包含会话、轮次、工具输入、调用 ID、工作区、客户端版本及用户／transcript 元数据，但没有服务器名或启动命令。字段名称已与客户端 Hook 日志独立核对。

`beforeMCPExecution` 才出现不同的 `mcp_server_name` 和 `command`，且 `tool_input` 为 JSON 字符串；该事件没有 `tool_use_id`。它出现时已经错过 v4 中需要识别自有候选并修改输入的阶段。相同参数实验也排除了用业务 ID、正文或工作区识别来源的可能。不同的唯一调用 ID 能区分两次调用，却不证明哪次调用属于自有服务器。

因此 v4 §3.1(1) 要求的“无歧义定位自有候选工具”不成立。`compatibility.json` 将 P0-03 标为 `failed`，证据本身也保存 `checkResults.P0-03: failed`。不通过宽松工具名、最近会话、参数匹配、向第三方注入字段或假造 ACK 绕过该条件。

## 已观察但尚未完整验证的内容

- Windows Hook 输入带 UTF-8 BOM，工作区根采用 `/D:/...`；现有专用解析适配成功处理，MCP 和普通 JSON 仍保持严格解析。
- MCP 初始化请求使用协议 `2025-11-25`，包括 JSON-RPC ID `0`。原始 stdout 只记录已写出，不将 flush 视为客户端接受回执；协商返回值未单独保存。
- 成功的 `postToolUse.tool_use_id` 与对应 `preToolUse` 一致。没有执行原生拒绝、取消或超时，因此 P0-04 仍为 `notRun`，不能外推失败终止映射。
- Read 和两台测试 MCP 的被动事件已采集；schema 校验时点、超限、修改后传递、第三方 Hook 合并、全部工具类别和等待预算未验证。P0-01/02/05—08 保持 `notRun`。
- 两轮提示、结果与 stop 已捕获；sessionStart、重启恢复、乱序和父子关系未验证，不能据此宣称会话生命周期完整支持。

## 原生共享审批与后续条件

[源码检查](../source-analysis.json)在此构建中仍发现内部 `pendingDecisionProvider.requestApproval` 调用。捆绑的公开扩展声明未显示已查找的提交入口，但这不证明所有内部接口都不存在。尚无已验证的外部承载组件能够读取真实请求、提交同一决定、获得接受／取消通知、查询状态并证明原子裁决，因此 P0-09—11、V27—V34 均未通过。

2026-09-22 的[接口追踪](../native-interface-report.md)进一步定位到 Agent Exec → 扩展宿主 → 工作台的审批回调，以及 `cursor` / `cursorAgentHost` 仅限内置扩展的访问校验。另有带接受状态的交互注册表，但尚未证明它对应同一个原生工具审批。此为源码证据，不增加实机通过项；完整记录见 [native-interface-analysis.json](../native-interface-analysis.json)。

继续完整产品接入需要先取得以下证据：候选阶段可验证的服务器身份及安全输入传递；可靠的原生终止关联及等待预算；原生审批发现、提交、接受、查询和原子裁决接口。如果需要改变 v4 的固定绑定链路，应先修订方案与验收要求，不能把当前失败样本改记为成功。可独立设计／测试的 P1 模型不因此获得产品激活资格。

## 回归与交付范围

执行 `scripts/cursor-probe/check.ps1`：11 项 Rust 测试、10 项合成进程测试和 2 项已审核运行证据回归全部通过；独立探针构建通过。能力校验新增逐验收项结果校验：即使外层误写 `passed`，缺少明确通过结果、仍未执行或失败的证据都不能启用能力。实际用例完整性由运行审核负责，合成测试不提升任何客户端能力。产品未接入这些模块，因此未运行无关的 Tauri／前端全量构建。

此次交付协议探针、严格 JSON/JCS 基础模块、证据校验与上述运行记录。正式 broker、安装／更新恢复、桌面／LAN 三类交互尚未接入；没有新增会话列表设置入口。

2026-09-22 接续检查：11 项 Rust 测试、16 项 Node 检查及独立探针构建通过，新增 4 项源码扫描回归。原运行证据保持不变，已确认旧测试进程全部退出。
