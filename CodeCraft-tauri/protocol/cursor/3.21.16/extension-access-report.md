# Cursor 3.21.16：普通扩展原生接口可达性实测

2026-09-22，Windows x64。已实际启动安装的 Cursor，在独立用户数据目录和独立扩展目录中执行标准扩展宿主测试。普通开发扩展调用 `acquireAgentHostRuntime()` 时收到内置扩展访问拒绝；当前路线不能取得原生审批 runtime。记录见 [extension-access-runtime.json](extension-access-runtime.json)。

## 实验与结果

使用项目内 `scripts/cursor-probe/extension-access` 开发扩展，通过 Cursor 支持的 `--extensionDevelopmentPath` / `--extensionTestsPath` 运行。扩展声明 `cursor`、`cursorNoDeps`、`cursorAgentHost` proposals，并传入 `--enable-proposed-api=codecraft.cursor-api-access-probe`。这仍不能越过客户端的内置扩展限制。

| 检查 | 实际结果 |
| --- | --- |
| 普通测试扩展加载 | 成功，实际 extension ID 为 `codecraft.cursor-api-access-probe`，ExtensionMode 为 Test |
| `vscode.cursor` 命名空间 | 可见，枚举得到 `acquireAgentHostRuntime` 等名称；名称存在不代表调用获准 |
| 调用 `acquireAgentHostRuntime()` | 失败，客户端错误归类为 `BUILTIN_ONLY` |
| `anysphere.cursor-agent-exec` | 可见并激活，没有导出可调用 API |
| `anysphere.cursor-agent-host` / `cursor-local-agent-runtime` | 不在当前普通扩展宿主的扩展列表中；不能据此宣称其他宿主未加载它们 |
| 相关命令注册 | 仅找到 `workbench.action.devIsAgentHostEnabledForTesting`，没有从这次枚举得到原生提交入口 |
| 读取/提交真实原生工具审批 | 未执行；前置 runtime 获取已被拒绝 |

测试配置、缓存、临时文件和原始进程日志全部写入忽略的项目内 `scripts/cursor-probe/runs/extension-access-*` 目录。没有使用真实账户会话、复制账户凭据、修改用户 Cursor 配置、修改 Cursor 安装文件或伪装内置扩展身份。测试进程正常退出；所有子进程继承两核 CPU 亲和性。

提交的记录仅含版本、Cursor EXE SHA-256、从 Cursor EXE 的 PE header 读取的架构、探针代码哈希、接口名称和固定错误类别，不包含用户目录或原始进程日志。`evidenceKind` 是 `cursorExtensionRuntime`，不伪装成 Agent 工具执行 fixture。

## 官方接口复核

同日读取 [Cursor Hooks](https://cursor.com/cn/docs/hooks)：`preToolUse` 的 MCP 名称仍按 `MCP:<tool_name>` 表达，`beforeMCPExecution` 才明确提供 `mcp_server_name` 与服务器启动信息；后者公开输出没有 `updated_input`。文档仍不承诺通过 `preToolUse.permission = ask` 建立原生共享审批。先前 P0-03 的同名实测失败证据保持有效。

另核对 [SDK Bridge](https://cursor.com/cn/docs/sdk/bridge) 和 [TypeScript SDK](https://cursor.com/cn/docs/sdk/typescript)：Bridge 运行 SDK 智能体并使用独立 API key/本地 Bearer 认证；SDK 本地无头运行没有交互式人工批准流程，控制调用需用 hooks。此次文档核查没有发现它能附着到 IDE 已有原生工具卡片并共享其决定的承诺，因此没有把另起 SDK 智能体当作原生双端同步的替代实现。

## 对剩余工作的影响

- 已完成普通扩展路线的接口可达性实测；之前仅有源码限制线索，现在有实际调用拒绝记录。
- 完整 P0-09 仍未通过：尚未完成真实原生请求发现、提交和接受确认。P0-10/11 与 V27—V34 的依赖也未解除。
- 不更新 Agent/Ask/Plan 的能力通过记录；没有将开发模式、API 名称枚举或未登录隔离宿主的结果外推为全部运行模式的工具覆盖。
- 保持 v4 全部要求时，需要 Cursor 提供可信候选 MCP 身份和可供外部组件使用的原生审批通道，才能完成剩余产品交互及实机验收。修改 CodeCraft 的能力布尔值、放宽同名匹配或新增表单都不能补出这些接口。

## 复现

```powershell
. .\CodeCraft-tauri\scripts\cursor-probe\limit-cpu.ps1
node .\CodeCraft-tauri\scripts\cursor-probe\verify-extension-access.mjs 'D:\software\cursor\Cursor.exe'
```

输出包含新的隔离运行目录与报告路径。进程退出成功只表示测试完成，不表示取得 nativeShared 能力。
