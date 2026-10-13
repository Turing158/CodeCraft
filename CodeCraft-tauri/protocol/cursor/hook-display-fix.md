# 工具审批正文与 Windows 中文编码

2026-09-28 检查确认了两个独立问题：

- 桌面和 LAN 直接把 `toolInput` JSON 当审批正文。`codecraft_list_projects` 没有参数，因此正文只有 `{}`；`codecraft_start_workflow` 则只显示项目 ID 和标题。
- 本机 Cursor 3.22.7 的 `out/vs/workbench/api/node/extensionHostProcess.js` 把 Hook JSON 以 UTF-8 写入临时文件，再通过 Windows PowerShell 的 `Get-Content -LiteralPath ... -Raw` 读取。未指定读取编码导致系统代码页误解码中文，随后 `$OutputEncoding = UTF8` 把乱码传给 Hook。已保存的 Hook 标题为 `闂瓟鐣岄潰楠岃瘉`，对应 MCP 工作流标题仍为 `问答界面验证`。

## 修复

`cursor-tool-display.ts` 为 CodeCraft MCP 操作提供中文、英文和繁体说明；普通工具优先显示命令或路径，无参数工具提供明确说明。桌面与 LAN 共用该格式，完整参数通过可用键盘展开的详情查看，并以纯文本渲染。显示层不改写原始参数、审批摘要或历史记录。

`scripts/cursor-probe/fix-hook-encoding.mjs` 为本机 Cursor 的已知 Hook 包装器补上 `Get-Content -Encoding UTF8`。脚本只接受唯一匹配的包装器，先保存同目录 `.bak`，重复运行不再次修改；未知版本布局会停止。此补丁作用于 Cursor 安装目录，需重新启动 Cursor；升级可能覆盖补丁，届时重新检查对应读取语句。它不修改 Cursor 的权限判断。

```powershell
[System.Diagnostics.Process]::GetCurrentProcess().ProcessorAffinity = [IntPtr]3
node scripts/cursor-probe/fix-hook-encoding.mjs D:/software/cursor/resources/app
```

旧记录中的乱码保留在原始参数详情中。新 Hook 调用从正确的 UTF-8 输入生成记录，不进行猜测性转码。

## 验证

- Cursor 前端及 LAN 相关测试 12 项通过，TypeScript 和两端生产构建通过。
- `node --test scripts/cursor-probe/verify-hook-encoding.mjs`：3 项通过，包括备份、幂等性、未知布局拒绝，以及真实 Windows PowerShell 管线中的中文、emoji、换行与嵌套 JSON 往返。
- `verify-tool-ui.mjs`：760×900 桌面和 390×844 LAN 渲染通过；检查操作说明、键盘展开、原始参数、纯文本渲染、只读控制和唯一审批回传，截图位于 `scripts/cursor-probe/runs/tool-ui/`。
- Release EXE 构建成功，生成的程序通过 7 项隔离 CLI 检查。
- 上述 UI 和管线验证为隔离测试，没有向 Cursor 发起真实审批。
