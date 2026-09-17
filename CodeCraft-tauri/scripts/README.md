# Scripts

本目录存放 CodeCraft 的打包、协议验证、测试夹具采集和导出脚本。

## 打包脚本

| 文件 | 用途 |
| --- | --- |
| `package-single-exe.ps1` | 构建不带安装器的便携式单文件 EXE。 |
| `package-single-exe-plus.ps1` | 构建增强版便携式单文件 EXE，包含额外的打包处理。 |

对应的 npm 命令：

```powershell
npm run package:single-exe
npm run package:single-exe-plus
```

## WorkBuddy 脚本

| 文件 | 用途 |
| --- | --- |
| `verify-workbuddy-plugin.mjs` | 验证 WorkBuddy 插件配置、输入校验和空响应行为。 |
| `verify-workbuddy-fixtures.mjs` | 验证 `protocol/workbuddy/5.5.3` 中的协议样例和运行时夹具。 |
| `verify-workbuddy-ui.mjs` | 使用浏览器自动化检查 WorkBuddy 只读提醒和局域网界面。 |
| `workbuddy-capture.mjs` | 使用隔离配置采集真实 WorkBuddy CLI 的协议事件。 |
| `workbuddy-fixture-hook.mjs` | 供 `workbuddy-capture.mjs` 调用的隔离测试 Hook。 |
| `export-workbuddy-fixtures.mjs` | 将隔离采集结果脱敏并导出到 `protocol/workbuddy/5.5.3/fixtures/runtime`。 |

常用命令：

```powershell
npm run test:workbuddy-plugin
npm run test:workbuddy-fixtures
node scripts/verify-workbuddy-ui.mjs
node scripts/workbuddy-capture.mjs tool-allow
node scripts/export-workbuddy-fixtures.mjs <capture-directory-name>
```

采集过程使用的临时文件位于 `.workbuddy-verification/`，该目录已被 `.gitignore` 忽略。导出后的脱敏协议夹具位于 `protocol/workbuddy/5.5.3/`，属于项目测试资料，应纳入版本控制。

## Kimi 脚本

| 文件 | 用途 |
| --- | --- |
| `kimi-protocol-probe.mjs` | 连接 Kimi Code CLI 和 CodeCraft Hook，采集并验证 Kimi 协议事件。 |

使用方式和前置条件见 `protocol/kimi/README.md`。该脚本只用于协议验证，不是 CodeCraft 运行时的启动依赖。

## 维护说明

删除本目录中的验证或采集脚本会使对应的 npm 命令、协议复现流程或文档示例失效。只运行程序或打包 EXE 时不需要手动执行这些脚本，但建议保留它们以便回归测试和重新生成协议夹具。

运行 Rust 构建或测试时，遵循项目约定设置 `$env:CARGO_BUILD_JOBS = '2'`，并将 Windows 子进程限制在两个逻辑核心内。
