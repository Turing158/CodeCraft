# Cursor P0 协议探针

> 2026-09-23：本文的 v4 接入限制为历史调查记录。经用户批准，当前产品改用 v5 独立 Hook/MCP 交互；使用和验证范围见 [最新产品状态](../../protocol/cursor/product-integration.md)。P0 失败证据原样保留。

适配计划 v4 的第一批实现：独立于 Tauri 的协议采集器、同名 MCP 假工具、严格 JSON/JCS 基础模块和兼容性证据校验。探针本身不会注册生产配置；正式采集接入已由独立 `cursor-core` 与 Tauri 提供，见 [产品状态](../../protocol/cursor/product-integration.md)。

已继续加入 [P1 独立工作流核心](../../protocol/cursor/codecraft-v1/README.md)：自有业务 schema、请求幂等、计划版本/限制、结果 owner/租约、单 broker 状态存储、ACL、项目文件身份、票据事务、文件通道、安装恢复和只读会话存储。`check.ps1` 分别检查正式核心和探针，运行格式、Rust 测试、构建、严格 Clippy 和 17 项 Node 检查。生产采集已接入，审批能力仍关闭。

Cursor 3.21.16 / Agent 的两轮实机实验已确认 `preToolUse` 使用 `MCP:<toolName>`，不带服务器身份；完全相同参数的两个候选载荷仅调用 ID 不同。P0-03 已记录失败，见[运行报告](../../protocol/cursor/3.21.16/Agent/report.md)。在没有可靠身份前，不向 Cursor 注入票据，不实现猜测会话的替代绑定。候选／激活／消费和安装事务已有独立模型与测试，生产采集、broker、安装维护与双端会话视图已接入；三类可提交审批仍依赖 P0。探针的 `codecraft_probe_echo` 只返回摘要，不是正式问答／计划工具。

## 自动检查

```powershell
.\CodeCraft-tauri\scripts\cursor-probe\check.ps1
```

需要 Rust、Windows C++ 链接工具和 Node.js。脚本设置 `CARGO_BUILD_JOBS=2`、`RUST_TEST_THREADS=2`，将进程亲和性收窄到原允许集合中的最多两核。每个探针进程也主动收窄亲和性，包括由 Cursor 启动的 Hook/MCP。

Rust 测试严格解析、重复键、RFC 8785 数字与 UTF-16 排序、内容摘要、合法回退及 MCP 协议边界。Node 测试实际启动本地探针子进程，检查文件竞争、错误输入、服务器隔离、ID 为 0、重放、隐私和能力校验。**这些均为合成测试，不是 Cursor P0 通过证据。**

## 隔离项目

```powershell
.\CodeCraft-tauri\scripts\cursor-probe\prepare.ps1
```

脚本在 `runs/<UUID>/project/` 建立独立项目，生成项目级 `.cursor/hooks.json` 和 `.cursor/mcp.json`。这里只用项目级配置观察协议；用户级生产安装由桌面设置提供。它不修改用户级 Cursor 配置、日常项目、原生权限或沙箱设置。只把生成的 `project/` 作为单根目录打开；不要打开其父目录，也不要接受打开父级 Git 仓库的提示。测试结束后关闭这个隔离项目即可移除其生效范围；run 保留供复核。

Hook 对全部列出的事件只作观察：权限事件返回合法 `permission: allow`，提示提交返回 `continue: true`，观察事件按公开 schema 返回空对象。未知受控事件以退出码 1 退出；这尚不构成目标版本故障行为的 P0 证明。已知权限事件在坏 JSON、磁盘故障、队列满或采集异常时仍返回 allow。日志失败不改变工具参数或创建审批等待。

两台测试 MCP 服务器 `codecraft_cursor_probe`、`codecraft_cursor_collision` 都提供 `codecraft_probe_echo`：

```json
{
  "schemaVersion": 1,
  "clientRequestId": "p0-owned-01",
  "payload": { "message": "hello", "nested": { "values": [0, true, null] } }
}
```

`bridgeTicket` 在 schema 中声明但非必填；即便显式传入也没有授权意义。正式工具名不由此探针提供。工具不执行命令、不读取工作区文件、不产生答案或计划批准。

登录 Cursor 后，在隔离项目进行以下最初实验，并分别记录 Agent/Ask/Plan 模式：

1. 检查两台 MCP 的实际加载来源和状态。只用假工具及 `sample.txt`，保留原生确认与第三方 Hook。
2. 请 Cursor 分别调用两台服务器的同名工具，使用不同的 `clientRequestId`；随后读取 `sample.txt`。
3. 对比 `preToolUse` 的 `tool_name`、实际字段结构与 `beforeMCPExecution` 的服务器字段。若两台服务器的候选事件无法无歧义区分，记录 P0-03 的失败证据，停止票据注入路线的激活。
4. 在原生工具确认中允许、拒绝及取消各一次，对比 `tool_use_id`、`generation_id`、事件顺序与 `postToolUseFailure`。超时不是终止关联通过证据。
5. 只有身份成立后才扩展注入／双事件实验；只有原生请求发现、提交、接受、查询与原子决定接口可实际接入后，才开始 P0-09—11。UI 点击本身不能证明原生桥接可用。

当前版本的[报告与能力记录](../../protocol/cursor/3.21.16/Agent/report.md)列出失败与尚未执行的项目；3.21.13 记录保留，不混用不同版本证据。账号登录必须由用户完成，本次已完成登录。

## 采集与证据

```powershell
$probe = '.\CodeCraft-tauri\scripts\cursor-probe\target\debug\codecraft-cursor-probe.exe'
& $probe inspect --root '<prepare 输出的 run 绝对路径>'
node .\CodeCraft-tauri\scripts\cursor-probe\inspect-client.mjs 'D:\software\cursor\resources\app'
```

`run.json` 保存本次构建 SHA-256、随机 run ID 和脱敏盐。构建改变后必须创建新 run；旧目录不能复用成新实验。`events/*.json` 只存字段结构、已知安全枚举与字符串的一致单向别名，不保存原票据、提示正文、路径、服务器凭据。工具输入为 JSON 字符串时另行严格解析后记录脱敏结构。一般错误仅记录错误码，stdout 只写协议 JSON。

`inspect-client.mjs` 只读扫描 Agent Host、Agent Exec、扩展宿主、工作台和公共声明文件，输出源码哈希、UTF-16 字符位置、有限上下文及调查线索。文件缺失或代码标记改变时返回 unknown，不沿用旧版结论；Node 的运行架构也不作为 Cursor 架构。它始终输出 `sourceAnalysis` 和 `supported: false`，不能替代实机验证。3.21.16 的[接口追踪报告](../../protocol/cursor/3.21.16/native-interface-report.md)记录了当前内置扩展限制及仍缺少的外部接入证据。

每 run 最多 4096 条记录、64 MiB，单条最多 1 MiB；采集锁最多等待 3 秒。使用同目录临时文件、flush/sync 和不可覆盖的硬链接发布，需要支持硬链接的本地文件系统。锁内序号表示采集提交顺序，**不自动证明 Cursor 原始事件先后**；同时保留接收时间、事件 ID 和关联别名。`inspect` 忽略未发布的临时文件。

采集文件固定标为 `evidenceKind: probeCapture`，`cursorClientExecuted: null`。直接手工喂入 Hook 也会产生这种文件，不能据文件存在认定真实 Cursor 调用。审核运行步骤、客户端版本／模式、配置来源及关联证据后，才能在版本／模式目录形成 `cursorRuntime` fixture；故障注入及合成样本独立归类。源码检索结果固定为 `sourceAnalysis`，启动前置检查为 `runtimePreflight`，两者都不能通过 P0。

## 能力记录校验

```powershell
& $probe compatibility --record '<模式目录>\compatibility.json' --runtime '<实际身份>.json' --tool Shell
```

实际身份必须独立读取，包含完整 Cursor 版本、模式、平台、架构、客户端与生效配置指纹；不能从待验证记录复制身份作为验证。校验返回基础 Hook、nativeShared、问答、计划四项结果，逐项核实对应 P0/V 项、工具覆盖、证据种类、哈希、身份与验收项绑定。`nativeShared` 必须额外具备 P0-09—11、V27—V34。未执行、失败、合成、缺失、被改动或来自不同版本／模式的证据均不可通过。

审核后的每个 runtime fixture 必须带 `identity`、`checkIds`、逐项 `checkResults`、`evidenceKind: cursorRuntime`、`cursorClientExecuted: true` 和 `probeInjected: false`；记录引用相对路径和 SHA-256，拒绝目录逃逸及 junction 跳出。只有该项 `checkResults` 明确为 `passed` 才可用于启用能力，缺失、失败和未执行均拒绝；归属该项不等于通过该项。通过标记仍依赖真实实验审核，不能靠手工编辑布尔值创造客户端能力。当前评估器仅由探针使用，**还未接入产品安装或运行握手**。

P1—P6 继续遵循 v4；桌面与 LAN 会话列表不新增设置入口，Cursor 采集配置已集中在现有桌面设置 → Hook。完整交互能力仍受上文 P0 阻塞限制。

## 正式 EXE 命令行验证

构建主程序后运行以下检查（默认使用 Debug EXE，也可传入 Release EXE 路径）：

```powershell
. .\CodeCraft-tauri\scripts\cursor-probe\limit-cpu.ps1
node .\CodeCraft-tauri\scripts\cursor-probe\smoke-product.mjs
node .\CodeCraft-tauri\scripts\cursor-probe\smoke-product.mjs .\CodeCraft-tauri\src-tauri\target\release\codecraft-tauri.exe
```

脚本检查 6 种 Hook 的离线/畸形输入响应及未启用 MCP 的退出，不运行 GUI。环境变量只指向项目内的隔离路径，不修改真实 Cursor 配置；这是产品入口的合成检查，不计入 Cursor P0 实机验收。前端开发服务器忽略探针的 `target` 与 `runs` 目录，避免监听构建中的 Windows EXE。

## 普通扩展原生接口可达性

`verify-extension-access.mjs` 启动实际 Cursor 的标准扩展测试宿主，使用完全隔离的项目内 profile、扩展目录及用户环境目录。它只检查 API 可见性、runtime 获取、指定内置扩展导出与相关命令名称，不提交原生决定。运行方法与本机结果见 [扩展接口实测](../../protocol/cursor/3.21.16/extension-access-report.md)。

3.21.16 实测 `acquireAgentHostRuntime()` 被普通扩展访问限制拒绝，即使显式开启 proposed API 也无法获取。API 名称存在、开发扩展成功加载及进程正常退出均不等于原生审批可用。报告标为 `cursorExtensionRuntime`，不作为 Agent/Ask/Plan 的能力通过 fixture。`verify.mjs` 当前共 18 项检查，新增回归核对该运行证据与探针源文件哈希。
