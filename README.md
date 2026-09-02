<div align="center">

<img src="docs/assets/logo.png" alt="CodeCraft" width="112" height="112" />

# CodeCraft

**贴在屏幕顶端的一条小面板，让你随时看见 AI 编程助手在干什么。**

专注于 AI 编程会话的轻量桌面工作台 · 为 Windows 打造

<p>
  <img src="https://img.shields.io/badge/平台-Windows-0078D4?style=flat-square" alt="平台 Windows" />
  <img src="https://img.shields.io/badge/版本-0.1.1-4C8BF5?style=flat-square" alt="版本 0.1.1" />
  <img src="https://img.shields.io/badge/技术-Rust%20%2B%20Tauri%202-DEA584?style=flat-square" alt="Rust + Tauri 2" />
  <img src="https://img.shields.io/badge/语言-简中%20%2F%20繁中%20%2F%20EN-2EA043?style=flat-square" alt="多语言" />
</p>

**简体中文** · [English](README.en.md) · [繁體中文](README.zh-TW.md)

</div>

---

## 这是什么？

如果你在用 **Claude Code**、**Codex**、**OpenCode**、**PI**、**DeepSeek Harness** 或 **ZCode** 这类"AI 编程助手"，你大概遇到过这些情况：

- 让它干活之后，只能一直盯着黑色的命令行窗口，不知道它到底做完了没有；
- 它中途要问你一句"这个命令能执行吗"，你没看见，它就一直卡在那里等；
- 同时开了好几个任务，窗口一多就彻底乱了。

CodeCraft 就是为了解决这件事。它平时只是屏幕最上方一条几乎看不见的细线，鼠标移上去就展开成一张卡片列表：谁在忙、谁卡住了、谁需要你点一下"同意"，一眼就知道。处理完，它自己缩回去。

> 简单说：**它是 AI 助手的仪表盘 + 门铃**，不是又一个代码编辑器。

## 它能帮你做什么

| | 能力 | 说明 |
| :---: | --- | --- |
| 📋 | **会话集中管理** | Claude Code、Codex、OpenCode、PI、DeepSeek Harness 和 ZCode 的任务并排显示，状态一目了然：工作中、等待输入、需要处理、已完成、失败。 |
| ✅ | **一键批准** | 助手想执行某个命令、修改某个文件时，弹到面板上，你点"允许一次""始终允许"或"拒绝"，不用切回终端。 |
| ❓ | **代它回答** | 助手提问时直接在面板里选选项或写补充说明，答案会回传给它。 |
| 📝 | **确认计划** | 助手列出行动计划后，由你决定：点"实行计划"让它开始动手，或写下要改的地方让它先调整。 |
| 🔔 | **声音提醒** | 内置音符盒、猫猫、骨块、经验四套音效，也能换成自己的音频；离开电脑也不会漏掉请求。 |
| 📱 | **手机上看** | 打开局域网控制台后，用手机浏览器扫码就能查看会话，甚至远程点批准（默认关闭）。 |
| 🎨 | **随你打扮** | 深色 / 亮色主题、三档透明度、整体缩放、动画开关、自定义"工作方块"图片。 |
| 🌏 | **三种语言** | 简体中文、繁體中文、English，切换即时生效。 |

## 已支持的 Agent

CodeCraft 自己不写代码，它负责盯着下面这些 AI 编程助手。装好对应的连接（见下一节第 2 步）后，它们的任务就会出现在面板上。

| Agent | 介绍说明 |
| --- | --- |
| <img src="docs/assets/agent-claude-code.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **Claude Code**<br /><sub>Anthropic</sub> | 支持最完整。会话状态、工具调用、实时转录都能看，批准、回答提问、确认计划都可以直接在面板里完成，不用切回终端。 |
| <img src="docs/assets/agent-codex.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **Codex**<br /><sub>OpenAI</sub> | 会话状态与工具调用审批可以在面板里处理。它的提问和计划确认是只读的，只能回到原来的 Codex 窗口完成，面板会提供一个跳转按钮帮你切过去。 |
| <img src="docs/assets/agent-opencode.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **OpenCode**<br /><sub>opencode.ai</sub> | 会话状态、工具调用、原生权限审批和提问都能在面板里处理，权限决定支持"允许一次""始终允许""拒绝"，另有一个可选的全工具门禁模式，让每个工具调用都先经过你确认。计划确认暂未接入，需要回到 OpenCode 窗口完成。 |
| **PI** | 会话、工具活动、权限审批和提问可在面板与局域网控制台处理；支持允许一次、会话内允许和拒绝，计划确认暂未接入。 |
| <img src="docs/assets/agent-deepseek.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **DeepSeek Harness**<br /><sub>DeepSeek</sub> | 通过用户级原生插件同步会话、回答、工具活动、提问和计划审阅。权限严格使用 DSH 的一次性语义，只提供"允许一次"和"拒绝"；计划可以批准，或携带反馈继续规划。 |
| **ZCode**<br /><sub>Z.ai</sub> | 通过官方七事件 Hook 同步外部 Desktop/CLI 会话、工具结果、最终回答、提问和计划审阅。普通工具只提供"允许一次"和"拒绝"；提问与计划即使在全自动模式下也必须由人决定。 |

六个 Agent 可以同时开着，面板顶部的筛选按钮能只看其中一家，或者"全部"一起看。

> DeepSeek Harness 当前是 Developer Preview。CodeCraft 优先适配 `@deepseek-ai/dsh@0.1.1-rc.2`，并兼容 `0.1.2-alpha.2`；检测不到版本或版本不在兼容列表中时，本地桥会拒绝交互并显示兼容错误。

## 快速上手

**1. 安装并启动**

运行安装包后启动 CodeCraft。它不会出现在任务栏里，请把鼠标移到主屏幕**最上方中间**，那条细线就是它。

**2. 连接你的 AI 助手（关键一步）**

展开面板 → 点右上角 ⚙️ → **通用 → Hook 管理** → 点一下要连接的 Agent 完成安装。

这一步在做什么？CodeCraft 会往对应助手的配置里加一个"通知钩子"，让助手在开始工作、要调用工具、任务结束时主动告诉 CodeCraft 一声。不装它，面板会一直是空的。想撤销随时可以在同一处卸载，配置会被还原。

DeepSeek Harness 使用 `$DSH_HOME`（默认 `~/.dsh`）下的 `cordis.patch.yml` 和本地 ESM 插件。CodeCraft 只维护带自身 marker 的配置块，修改前会写入 `.bak`，卸载时保留其他插件和原有 overlay。

ZCode 使用用户级 `~/.zcode/cli/config.json`。CodeCraft 会结构化合并七个官方 Hook 事件，修改前创建 `config.json.bak`，并保留已有 Hook、插件、MCP 和未知字段；在 Hook 管理中卸载时，只删除 CodeCraft 自己的条目。当前首发适配 Windows，验证基线为 ZCode Desktop `3.10.1`。

**3. 正常使用你的助手**

照常在终端里使用已连接的 Agent。接下来会话卡片就会自己出现在面板上；有请求要处理时，面板会自动展开提醒你。

### ZCode Developer Preview 说明

- ZCode Hook 边界拿不到进行中的助手增量文本，只有本轮 `Stop` 后的最终回答；进行中只能看到状态与工具活动。
- 问题审批沿用 CodeCraft 现有提醒规则，不播放原生音效；普通工具权限和计划审批会播放提醒。
- `PreToolUse` 等待 CodeCraft 时，ZCode 自己的界面不显示等待提示。极简模式下请保持音效开启，或通过托盘和局域网控制台查看待处理请求。
- CodeCraft 不可用或审批超时时，普通工具会退回 ZCode 原生权限流程；`AskUserQuestion` 与 `ExitPlanMode` 会放弃接管并保持 stdout 为空，避免返回缺少答案的无效决定。
- Hook 是审批与观测边界，不是沙箱。Hook 进程失败后的最终行为仍由 ZCode 运行时决定。

如果设置页显示版本不兼容、配置被修改或存在冲突，先确认 ZCode 版本和检测路径，再在 **Hook 管理** 中重新安装以修复 CodeCraft 自有条目。修复不会覆盖用户的其他配置。

## 手机 / 平板远程查看

设置 → **局域网** → 打开"启用服务"，会得到一个网址、一个二维码和一串 32 位访问令牌。手机连同一个 Wi-Fi，打开网址、填令牌即可。

首次启用时 Windows 会弹出防火墙提示，选择"允许"。

关于安全，请留意几点：

- 传输走的是局域网内的普通 HTTP（未加密），**只在自己家或办公室这类可信网络里开**，别在咖啡馆公共 Wi-Fi 上用。
- 令牌等同于密码。任何拿到网址 + 令牌的人都能看到你的会话内容。
- 默认网页是**只读**的。只有你另外打开"允许网页提交决定"，对方才能代你点批准。
- 不用的时候把开关关掉，端口会立刻释放。
- 令牌可以随时"轮换"，轮换后已登录的浏览器会立即失效。

## 一些贴心的小设计

- **自动收起**：鼠标移开约 0.25 秒后面板缩回细线；有任务在跑时会留一小条实时状态。
- **自动清理**：空闲或已停止的会话超过设定时间（默认 30 分钟）自动从列表移走，正在工作和等你处理的不会被动。
- **自动审批**：所有已连接 Agent 共用同一策略，可以手动逐个确认、只自动通过低风险工具，或自动通过普通工具审批；DSH 与 ZCode 不提供持久化"始终允许"按钮，ZCode 的提问和计划始终需要人工决定。
- **位置随心**：顶部可以左右拖动，也能一键置左、居中、置右。

## 运行环境

- Windows 10 / 11（面板停靠在主显示器顶部）
- 系统自带的 WebView2 运行时（Win11 已内置）
- 需要至少安装一个受支持的 Agent，CodeCraft 本身不包含 AI 模型，也不会替你调用任何 API

会话数据、设置和审批记录都保存在你自己电脑的本地目录里。

## 给开发者

项目主体在 [CodeCraft-tauri](CodeCraft-tauri)：前端是 TypeScript + Vite，后端是 Rust + Tauri 2，界面壳用系统 WebView2 渲染。

```powershell
cd CodeCraft-tauri
npm install
npm test            # Vitest 单元测试
npm run tauri dev   # 本地调试
npm run tauri build # 打包 Windows 安装包（NSIS）
```

需要 Node.js、Rust stable（`x86_64-pc-windows-msvc`）、Visual Studio C++ 生成工具和 Windows SDK。更多说明见 [CodeCraft-tauri/README.md](CodeCraft-tauri/README.md)。

---

<div align="center">

**在一个安静、紧凑的界面中，掌握编程助手的工作状态。**

</div>
