# Trae P0 协议探针

这是适配计划 v3 的第一阶段实现。它独立于 Tauri 编译，**不会注册或启用 CodeCraft 正式 Trae 集成**。测试工具为 `codecraft_probe_echo` 和 `codecraft_probe_delivery`，不接受正式问题/计划审批，不执行命令，不提供生产环境计划保护。

依赖锁定官方 Rust MCP SDK `rmcp 3.4.0`；生命周期、初始化、工具分发及协议取消由 SDK 处理。SDK 外有 1 MiB 消息限制、重复 JSON 键检查、同连接请求 ID 检查与 stdout flush 观测。业务参数按 RFC 8785 + SHA-256 比较。

## 自动验证

在 Windows PowerShell 中运行：

```powershell
.\CodeCraft-tauri\scripts\trae-probe\check.ps1
```

需要 Rust 1.88+、Windows C++ 链接工具和 Node.js。脚本设置 `CARGO_BUILD_JOBS=2`、两核进程亲和性，测试进程及探针也继承或主动收窄亲和性。生成内容都在本目录的 `target/` 和 `runs/` 中，已被 Git 忽略。

Rust 测试使用单调时钟输入验证票据和租约边界；Node 驱动实际探针子进程，检查文件通信和 MCP stdio 生命周期。输出明确标注 `evidenceKind: synthetic`、`traeClientExecuted: false`，不能作为 Trae 实测凭据。

## 准备真实客户端验证

```powershell
.\CodeCraft-tauri\scripts\trae-probe\prepare.ps1
```

脚本创建全新的独立项目、六事件项目 Hook、MCP 配置预览，并隐藏启动协调进程。它不启动 Trae、不写全局 Hook、不猜测 MCP 用户配置位置、不改变原生确认或沙箱设置。仅把生成的 `project/` 作为单根工作区打开；测试项目之外的 Hook 调用会被拒绝。

在 Trae 中启用该项目 Hook，并通过 MCP 设置界面加入 `mcp.preview.json` 中的 `codecraft_probe` 服务。保留这个准确的服务器名，否则不会注入票据。按照[实测清单](../../protocol/trae/3.3.102/README.md)完成验证。

直接命令示例（`$probe` 为构建后的绝对路径，`$run` 为新创建的 run 目录）：

```powershell
& $probe case --root $run --case inject
& $probe inspect --root $run
& $probe arm --root $run
& $probe release --root $run --operation '<inspect 返回的 operationId>'
& $probe transport --root $run --before-write-ms 1200
& $probe transport --root $run --after-flush-ms 6000
& $probe transport --root $run --drop-ack
& $probe transport --root $run
```

`arm` 独占创建测试计划文件并返回一次性启动指令；它只测试 Prompt 的绑定与上下文传递，不启用正式计划闸门。没有实测稳定 Prompt ID，所以重复启动码阻止 Prompt 并使旧轮次失效。一次 run 只支持一个绑定任务；多根、任务恢复和正式计划正文检查留给 P3。

`case` 仅影响 `PreToolUse`，支持 `observe`（默认，返回 ask）、`allow`、`deny`、`ask`、`inject`、`delay --delay-ms N`、`exit2`、`error`。显式 allow/deny 等故障实验只用于隔离项目。`inject` 仅对两个精确名称的探针 MCP 工具生效，其余工具返回 deny。禁止在真实工作项目上安装探针配置。

`codecraft_probe_echo` 自动回显 payload，用于检验嵌套字段完整性；`codecraft_probe_delivery` 等待 `release` 命令。后者只有在 MCP 响应实际 flush 后、5 秒租约内完成确认，才把诊断状态记为 `probeDeliveryConfirmed`。这不是生产计划批准。

## 数据与失效

协调器独占 `store.lock`，所有子进程通过 inbox/replies 交互；命令先刷盘日志再发布回复。发布使用同卷临时文件与不可覆盖的硬链接，避免覆盖已有不可变回复，要求支持硬链接的本地文件系统。journal 只存命令/回复摘要，不能用来恢复批准。重启生成新 epoch，票据和任务绑定不恢复。

默认控制通信 3 秒、心跳 2 秒、失效阈值 10 秒、轮询 200 ms；票据首次消费期限 10 分钟，消费后操作等待上限为 **探针实验值** 240 秒，并未验证 Trae 实际支持该等待时限。人工交互能力仍未就绪。

队列上限 1,024 条 / 64 MiB，其中控制消息保留 128 条 / 8 MiB；同时待决全局 128、每会话 16。一次实验最多 1,024 票据/操作、8,192 个命令去重记录、64 MiB journal。15 分钟后结果内容失效但保留命令墓碑，不能重新执行旧命令。达到上限应结束实验并建立新 run。P0 不实现产品长期保留、任务恢复、安装/卸载或 UI。

`events/` 包含**未经脱敏的本地原始输入**及宿主祖先进程候选信息，可能含提示词、文件路径和票据。所有 run 默认不提交；完成检查后按清单脱敏并单独导出所需证据。宿主 PID + 创建时间在重复会话/多实例实验完成前，只是候选身份，不是已验证规则。

实验后在 Trae 删除本次测试 MCP 服务、关闭测试工作区，并对照 `coordinator-process.json` 的 PID、创建时间和可执行文件停止本次协调进程。run 目录保留供检查；不提供宽泛删除脚本。若自行修改项目外已有配置，先创建不覆盖旧备份的 `*.bak`，只撤销此次实验拥有的项目。
