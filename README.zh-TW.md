<div align="center">

<img src="docs/assets/logo.png" alt="CodeCraft" width="112" height="112" />

# CodeCraft

**貼在螢幕頂端的一條小面板，讓你隨時看見 AI 程式設計助手在做什麼。**

專注於 AI 程式設計工作階段的輕量桌面工作台 · 為 Windows 打造

<p>
  <img src="https://img.shields.io/badge/平台-Windows-0078D4?style=flat-square" alt="平台 Windows" />
  <img src="https://img.shields.io/badge/版本-0.1.1-4C8BF5?style=flat-square" alt="版本 0.1.1" />
  <img src="https://img.shields.io/badge/技術-Rust%20%2B%20Tauri%202-DEA584?style=flat-square" alt="Rust + Tauri 2" />
  <img src="https://img.shields.io/badge/語言-簡中%20%2F%20繁中%20%2F%20EN-2EA043?style=flat-square" alt="多語言" />
</p>

[简体中文](README.md) · [English](README.en.md) · **繁體中文**

</div>

---

## 這是什麼？

如果你在用 **Claude Code**、**Codex**、**OpenCode**、**PI**、**DeepSeek Harness** 或 **ZCode** 這類「AI 程式設計助手」，你大概遇到過這些情況：

- 讓它做事之後，只能一直盯著黑色的命令列視窗，不知道它到底做完了沒有；
- 它中途要問你一句「這個命令能執行嗎」，你沒看見，它就一直卡在那裡等；
- 同時開了好幾個工作，視窗一多就徹底亂了。

CodeCraft 就是為了解決這件事。它平時只是螢幕最上方一條幾乎看不見的細線，滑鼠移上去就展開成一張卡片清單：誰在忙、誰卡住了、誰需要你按一下「同意」，一眼就知道。處理完，它自己縮回去。

> 簡單說：**它是 AI 助手的儀表板 + 門鈴**，不是又一個程式碼編輯器。

## 它能幫你做什麼

| | 能力 | 說明 |
| :---: | --- | --- |
| 📋 | **工作階段集中管理** | Claude Code、Codex、OpenCode、PI、Mimo、DeepSeek Harness 和 ZCode 的工作並排顯示，狀態一目了然：工作中、等待輸入、需要處理、已完成、失敗。 |
| ✅ | **一鍵批准** | 助手想執行某個命令、修改某個檔案時，跳到面板上，你按「允許一次」「一律允許」或「拒絕」，不用切回終端機。 |
| ❓ | **代它回答** | 助手提問時直接在面板裡選選項或寫補充說明，答案會回傳給它。 |
| 📝 | **確認計畫** | 助手列出行動計畫後，由你決定：按「執行計畫」讓它開始動手，或寫下要改的地方讓它先調整。 |
| 🔔 | **聲音提醒** | 內建音樂盒、貓貓、骨塊、經驗四套音效，也能換成自己的音訊；離開電腦也不會漏掉請求。 |
| 📱 | **手機上看** | 開啟區域網路控制台後，用手機瀏覽器掃描 QR Code 就能檢視工作階段，甚至遠端按批准（預設關閉）。 |
| 🎨 | **隨你打扮** | 深色 / 亮色主題、三段透明度、整體縮放、動畫開關、自訂「工作方塊」圖片。 |
| 🌏 | **三種語言** | 简体中文、繁體中文、English，切換即時生效。 |

## 已支援的 Agent

CodeCraft 自己不寫程式碼，它負責盯著下面這些 AI 程式設計助手。裝好對應的連接（見下一節第 2 步）後，它們的工作就會出現在面板上。

| Agent | 介紹說明 |
| --- | --- |
| <img src="docs/assets/agent-claude-code.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **Claude Code**<br /><sub>Anthropic</sub> | 支援最完整。工作階段狀態、工具呼叫、即時轉錄都能看，批准、回答提問、確認計畫都可以直接在面板裡完成，不用切回終端機。 |
| <img src="docs/assets/agent-codex.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **Codex**<br /><sub>OpenAI</sub> | 工作階段狀態與工具呼叫審批可以在面板裡處理。它的提問和計畫確認是唯讀的，只能回到原來的 Codex 視窗完成，面板會提供一個跳轉按鈕幫你切過去。 |
| <img src="docs/assets/agent-opencode.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **OpenCode**<br /><sub>opencode.ai</sub> | 工作階段狀態、工具呼叫、原生權限審批和提問都能在面板裡處理，權限決定支援「允許一次」「一律允許」「拒絕」，另有一個可選的全工具門禁模式，讓每個工具呼叫都先經過你確認。計畫確認暫未接入，需要回到 OpenCode 視窗完成。 |
| <img src="docs/assets/agent-mimo.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **Mimo**<br /><sub>Xiaomi</sub> | 透過使用者層級 Mimo 外掛同步工作階段、工具活動、回答、提問和計畫審閱。權限支援「允許一次」「一律允許」「拒絕」，也可開啟全工具門禁；計畫可以批准，或帶著回饋繼續規劃。 |
| <img src="docs/assets/agent-pi.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **PI** | 工作階段、工具活動、權限審批和提問可在面板與區域網路控制台處理；支援允許一次、工作階段內允許和拒絕，計畫確認暫未接入。 |
| <img src="docs/assets/agent-deepseek.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **DeepSeek Harness**<br /><sub>DeepSeek</sub> | 透過使用者層級原生外掛同步工作階段、回答、工具活動、提問和計畫審閱。權限嚴格使用 DSH 的一次性語意，只提供「允許一次」和「拒絕」；計畫可以批准，或帶著回饋繼續規劃。 |
| <img src="docs/assets/agent-zcode.svg" width="20" height="20" align="absmiddle" alt="" />&nbsp; **ZCode**<br /><sub>Z.ai</sub> | 透過官方七事件 Hook 同步外部 Desktop/CLI 工作階段、工具結果、最終回答、提問和計畫審閱。一般工具只提供「允許一次」和「拒絕」；提問與計畫即使在自動審批模式下也必須由人決定。 |

七個 Agent 可以同時開著，面板頂部的篩選按鈕能只看其中一家，或者「全部」一起看。

> DeepSeek Harness 目前是 Developer Preview。CodeCraft 優先適配 `@deepseek-ai/dsh@0.1.1-rc.2`，並相容 `0.1.2-alpha.2`；偵測不到版本或版本不在相容清單中時，本機橋接會拒絕互動並顯示相容性錯誤。

## 快速上手

**1. 安裝並啟動**

執行安裝檔後啟動 CodeCraft。它不會出現在工作列裡，請把滑鼠移到主螢幕**最上方中間**，那條細線就是它。

**2. 連接你的 AI 助手（關鍵一步）**

展開面板 → 按右上角 ⚙️ → **一般 → Hook 管理** → 按一下要連接的 Agent 完成安裝。

這一步在做什麼？CodeCraft 會往對應助手的設定裡加一個「通知掛鉤」，讓助手在開始工作、要呼叫工具、工作結束時主動告訴 CodeCraft 一聲。不裝它，面板會一直是空的。想撤銷隨時可以在同一處卸載，設定會被還原。

DeepSeek Harness 使用 `$DSH_HOME`（預設 `~/.dsh`）下的 `cordis.patch.yml` 和本機 ESM 外掛。CodeCraft 只維護帶自身 marker 的設定區塊，修改前會寫入 `.bak`，解除安裝時會保留其他外掛和原有 overlay。

ZCode 使用使用者層級的 `~/.zcode/cli/config.json`。CodeCraft 會以結構化方式合併七個官方 Hook 事件，修改前建立 `config.json.bak`，並保留既有 Hook、外掛、MCP 和未知欄位；在 Hook 管理中解除安裝時，只刪除 CodeCraft 自己的項目。目前首發適配 Windows，驗證基線為 ZCode Desktop `3.10.1`。

**3. 正常使用你的助手**

照常使用已連接的 Agent。接下來工作階段卡片就會自己出現在面板上；有請求要處理時，面板會自動展開提醒你。

### ZCode Developer Preview 說明

- ZCode Hook 邊界無法取得進行中的助手增量文字，只有本輪 `Stop` 後的最終回答；進行中只能看到狀態與工具活動。
- 問題審批沿用 CodeCraft 現有提醒規則，不播放原生音效；一般工具權限和計畫審批會播放提醒。
- `PreToolUse` 等待 CodeCraft 時，ZCode 自己的介面不顯示等待提示。極簡模式下請保持音效開啟，或透過系統匣和區域網路控制台查看待處理請求。
- CodeCraft 無法使用或審批逾時時，一般工具會退回 ZCode 原生權限流程；`AskUserQuestion` 與 `ExitPlanMode` 會放棄接管並保持 stdout 為空，避免回傳缺少答案的無效決定。
- Hook 是審批與觀測邊界，不是沙箱。Hook 程序失敗後的最終行為仍由 ZCode 執行階段決定。

如果設定頁顯示版本不相容、設定被修改或存在衝突，先確認 ZCode 版本和偵測路徑，再在 **Hook 管理** 中重新安裝以修復 CodeCraft 自有項目。修復不會覆蓋使用者的其他設定。

## 手機 / 平板遠端檢視

設定 → **區域網路** → 開啟「啟用服務」，會得到一個網址、一個 QR Code 和一串 32 位存取權杖。手機連同一個 Wi-Fi，開啟網址、填入權杖即可。

首次啟用時 Windows 會彈出防火牆提示，選擇「允許」。

關於安全，請留意幾點：

- 傳輸走的是區域網路內的普通 HTTP（未加密），**只在自己家或辦公室這類可信網路裡開**，別在咖啡廳公共 Wi-Fi 上用。
- 權杖等同於密碼。任何拿到網址 + 權杖的人都能看到你的工作階段內容。
- 預設網頁是**唯讀**的。只有你另外開啟「允許網頁提交決定」，對方才能代你按批准。
- 不用的時候把開關關掉，連接埠會立刻釋放。
- 權杖可以隨時「輪換」，輪換後已登入的瀏覽器會立即失效。

## 一些貼心的小設計

- **自動收起**：滑鼠移開約 0.25 秒後面板縮回細線；有工作在跑時會留一小條即時狀態。
- **自動清理**：閒置或已停止的工作階段超過設定時間（預設 30 分鐘）自動從清單移走，正在工作和等你處理的不會被動。
- **自動審批**：所有已連接 Agent 共用同一策略，可以手動逐個確認、只自動通過低風險工具，或自動通過一般工具審批；DSH 與 ZCode 不提供持久化「一律允許」按鈕，ZCode 的提問和計畫始終需要人工決定。
- **位置隨心**：頂部可以左右拖動，也能一鍵靠左、置中、靠右。

## 執行環境

- Windows 10 / 11（面板停靠在主顯示器頂部）
- 系統內建的 WebView2 執行階段（Win11 已內建）
- 需要至少安裝一個受支援的 Agent，CodeCraft 本身不包含 AI 模型，也不會替你呼叫任何 API

工作階段資料、設定和審批記錄都儲存在你自己電腦的本機目錄裡。

## 給開發者

專案主體在 [CodeCraft-tauri](CodeCraft-tauri)：前端是 TypeScript + Vite，後端是 Rust + Tauri 2，介面殼用系統 WebView2 算繪。

```powershell
cd CodeCraft-tauri
npm install
npm test            # Vitest 單元測試
npm run tauri dev   # 本機除錯
npm run tauri build # 打包 Windows 安裝檔（NSIS）
```

需要 Node.js、Rust stable（`x86_64-pc-windows-msvc`）、Visual Studio C++ 建置工具和 Windows SDK。更多說明見 [CodeCraft-tauri/README.md](CodeCraft-tauri/README.md)。

---

<div align="center">

**在一個安靜、緊湊的介面中，掌握程式設計助手的工作狀態。**

</div>
