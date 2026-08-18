export const LANGUAGE_STORAGE_KEY = "codecraft.language";

const LANGUAGE_MENU_TRANSITION_MS = 180;
const LANGUAGE_MENU_FADE_MS = 150;

export const SUPPORTED_LOCALES = ["zh-CN", "zh-TW", "en"] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

export const DEFAULT_LOCALE: AppLocale = "zh-CN";

export interface LocaleOption {
  locale: AppLocale;
  label: string;
  mark: string;
}

export const LOCALE_OPTIONS: readonly LocaleOption[] = [
  { locale: "zh-CN", label: "简体中文", mark: "简" },
  { locale: "zh-TW", label: "繁體中文", mark: "繁" },
  { locale: "en", label: "English", mark: "EN" },
];

export const parseLocale = (value: string | null | undefined): AppLocale =>
  SUPPORTED_LOCALES.includes(value as AppLocale)
    ? (value as AppLocale)
    : DEFAULT_LOCALE;

/**
 * Simplified Chinese is the source language used by the existing markup and
 * render helpers. Keeping the source phrases here lets the current UI become
 * multilingual without mixing translation keys into session/user content.
 */
const ENGLISH: Record<string, string> = {
  "Claude Code 活跃会话": "Claude Code active sessions",
  "收起面板": "Collapse panel",
  "展开面板": "Expand panel",
  "筛选会话来源": "Filter session sources",
  "全部": "All",
  "打开设置": "Open settings",
  "设置": "Settings",
  "点击重新安装并连接 Claude Code Hook": "Click to reinstall and connect the Claude Code Hook",
  "Hook 异常": "Hook issue",
  "Hook 正常": "Hook connected",
  "Hook 未安装": "Hook not installed",
  "Claude Code 会话列表": "Claude Code session list",
  "Codex 会话列表": "Codex session list",
  "暂无活跃会话": "No active sessions",
  "暂无会话": "No sessions",
  "Codex Hook 状态": "Codex Hook status",
  "连接异常": "Connection issue",
  "返回会话列表": "Back to sessions",
  "返回": "Back",
  "Claude Code 会话": "Claude Code session",
  "工具调用": "Tool calls",
  "对话": "Conversation",
  "实时转录": "Live transcript",
  "回答代码助手的问题": "Answer the coding assistant's questions",
  "答题界面操作": "Question view actions",
  "第 1/1 问题": "Question 1 of 1",
  "完整问题": "Full question",
  "回答选项": "Answer options",
  "收起回答选项": "Collapse answer options",
  "展开回答选项": "Expand answer options",
  "额外信息": "Additional information",
  "输入你的回答或补充说明": "Enter your answer or additional context",
  "问题导航": "Question navigation",
  "上一步": "Previous",
  "前往 Codex": "Open in Codex",
  "下一步": "Next",
  "提交": "Submit",
  "确认代码助手工具调用": "Confirm the coding assistant's tool call",
  "确认选项": "Confirmation options",
  "操作": "Actions",
  "点击后立即回传": "Sent immediately when selected",
  "确认操作": "Confirm action",
  "允许一次": "Allow once",
  "允许": "Allow",
  "仅执行这一次，不保存规则": "Run once without saving a rule",
  "执行这个工具调用": "Run this tool call",
  "始终允许": "Always allow",
  "执行并记住，之后不再询问": "Run and remember; do not ask again",
  "拒绝": "Deny",
  "不执行这个工具并继续": "Skip this tool and continue",
  "确认实行计划": "Confirm plan execution",
  "计划": "Plan",
  "完整计划": "Full plan",
  "实行计划": "Run plan",
  "实行计划操作": "Plan actions",
  "实行计划  auto mode": "Run plan  auto mode",
  "以自动模式执行此计划": "Run this plan in automatic mode",
  "实行计划  approve edits": "Run plan  approve edits",
  "自动执行计划，编辑操作自动批准": "Run automatically and approve edits",
  "请输入需要修改的内容": "Describe what should be changed",
  "自定义指令": "Custom instructions",
  "设置分类": "Settings categories",
  "通用": "General",
  "风格": "Appearance",
  "音效": "Sounds",
  "局域网": "LAN",
  "关于": "About",
  "语言": "Language",
  "选择后会立即应用到整个 CodeCraft 界面。": "Changes apply immediately across CodeCraft.",
  "选择界面语言，当前为简体中文": "Choose interface language; current language is Simplified Chinese",
  "选择界面语言，当前为繁體中文": "Choose interface language; current language is Traditional Chinese",
  "选择界面语言，当前为English": "Choose interface language; current language is English",
  "界面语言": "Interface language",
  "简体中文": "Simplified Chinese",
  "繁體中文": "Traditional Chinese",
  "通用设置内容待添加": "General settings are coming soon",
  "自动清理会话": "Automatic session cleanup",
  "空闲或停止状态超过所选时间后，从会话列表中移除；工作中和等待处理的会话不会受影响。": "Remove idle or stopped sessions from the list after the selected time. Working sessions and sessions awaiting action are not affected.",
  "清理时间": "Cleanup delay",
  "选择清理时间，当前为5 分钟": "Choose cleanup delay; current value is 5 minutes",
  "选择清理时间，当前为10 分钟": "Choose cleanup delay; current value is 10 minutes",
  "选择清理时间，当前为30 分钟": "Choose cleanup delay; current value is 30 minutes",
  "选择清理时间，当前为1 小时": "Choose cleanup delay; current value is 1 hour",
  "选择清理时间，当前为自定义": "Choose cleanup delay; current value is Custom",
  "5 分钟": "5 minutes",
  "10 分钟": "10 minutes",
  "30 分钟": "30 minutes",
  "1 小时": "1 hour",
  "自定义时长": "Custom duration",
  "分钟": "minutes",
  "可设置 1 分钟至 7 天。": "Set a duration from 1 minute to 7 days.",
  "顶部位置": "Top position",
  "控制顶部区域的水平拖拽与窗口停靠位置。": "Control horizontal dragging and the panel's top docking position.",
  "启用顶部拖拽": "Enable top-area dragging",
  "从界面顶部拖动，在主屏顶部调整位置。": "Drag the top strip to reposition the panel along the top of the primary display.",
  "快速设置窗口位置": "Quickly set panel position",
  "置左": "Left",
  "居中": "Center",
  "置右": "Right",
  "自定义位置": "Custom position",
  "左侧": "Left",
  "右侧": "Right",
  "自动审批": "Automatic approvals",
  "Claude Code 与 Codex 共用此策略；风险审批只自动通过低风险命令。": "Claude Code and Codex share this policy; risk-based approval only allows low-risk commands automatically.",
  "手动审批": "Manual",
  "风险审批": "Risk-based",
  "收起/展开": "Collapse / expand",
  "设置面板自动收起和审批请求展开方式。": "Control automatic panel collapse and approval expansion.",
  "自动收起延迟": "Auto-collapse delay",
  "审批自动展开": "Expand approvals automatically",
  "收到审批请求时自动展开面板。": "Expand the panel when an approval request arrives.",
  "Hook 管理": "Hook management",
  "点击 Agent 可安装或卸载对应的 CodeCraft Hook。": "Select an agent to install or remove its CodeCraft Hook.",
  "刷新 Hook 状态": "Refresh Hook status",
  "刷新": "Refresh",
  "刷新中...": "Refreshing...",
  "处理中...": "Working...",
  "已安装的 Hook": "Installed Hooks",
  "点击列表项可卸载 Hook。": "Select an item to remove its Hook.",
  "暂无已安装的 Hook": "No Hooks installed",
  "未安装的 Hook": "Available Hooks",
  "点击列表项可安装 Hook。": "Select an item to install its Hook.",
  "所有可用 Hook 均已安装": "All available Hooks are installed",
  "主题模式": "Theme",
  "深色": "Dark",
  "亮色": "Light",
  "更换跟随系统主题的工作方块图片": "Change the working square image for the system theme",
  "更换深色主题的工作方块图片": "Change the working square image for the dark theme",
  "更换亮色主题的工作方块图片": "Change the working square image for the light theme",
  "点击更换图片，显示时会自动裁切为正方形": "Choose another image; it is center-cropped to a square when displayed",
  "选择工作方块图片": "Choose a working square image",
  "图片文件过大，请选择小于 10 MB 的文件。": "The image is too large. Choose a file smaller than 10 MB.",
  "请选择 PNG、SVG 或其他图片文件。": "Choose a PNG, SVG, or another image file.",
  "无法读取这张图片，请选择其他文件。": "This image could not be read. Choose another file.",
  "保存工作方块图片失败，请重试。": "The working square image could not be saved. Try again.",
  "工作方块图片已更新。": "The working square image has been updated.",
  "透明效果": "Transparency",
  "分别调节界面、卡片和文字的透明度。": "Adjust panel, card, and text opacity separately.",
  "界面透明度": "Panel opacity",
  "卡片透明度": "Card opacity",
  "文字透明度": "Text opacity",
  "0.10 透明": "0.10 Transparent",
  "1.00 不透明": "1.00 Opaque",
  "界面动画": "Interface motion",
  "控制过渡、状态动效以及内容切换动画。": "Control transitions, status effects, and content animations.",
  "跟随系统": "System",
  "开启": "On",
  "关闭": "Off",
  "动画速度": "Animation speed",
  "界面设置": "Interface layout",
  "按比例缩放全部元素，或分别调整界面尺寸。": "Scale every element together, or tune the interface dimensions separately.",
  "界面设置模式": "Interface layout mode",
  "比例": "Scale",
  "自定义": "Custom",
  "整体比例": "Overall scale",
  "文字大小": "Text size",
  "界面宽度": "Interface width",
  "最小高度": "Minimum height",
  "最大高度": "Maximum height",
  "内容边距": "Content padding",
  "紧凑": "Compact",
  "宽松": "Spacious",
  "音效提醒": "Sound alerts",
  "在工具调用、审批和会话状态变化时播放提醒。": "Play alerts for tool calls, approvals, and session status changes.",
  "启用音效提醒": "Enable sound alerts",
  "关闭后保留音效库与音量设置。": "Sound library and volume settings are kept when disabled.",
  "音量调节": "Volume",
  "静音": "Mute",
  "音效库": "Sound library",
  "预设中的提醒类事件共用一段音效，结束类事件共用另一段。": "Preset alerts share one sound, while completion events share another.",
  "音符盒": "Music box",
  "音符盒试听": "Music box preview",
  "试听音符盒提醒音": "Preview music box alert",
  "试听音符盒完成音": "Preview music box completion",
  "猫猫": "Cat",
  "猫猫试听": "Cat preview",
  "试听猫猫提醒音": "Preview cat alert",
  "试听猫猫完成音": "Preview cat completion",
  "骨块": "Bone block",
  "骨块试听": "Bone block preview",
  "试听骨块提醒音": "Preview bone block alert",
  "试听骨块完成音": "Preview bone block completion",
  "经验": "Experience",
  "经验试听": "Experience preview",
  "试听经验提醒音": "Preview experience alert",
  "试听经验完成音": "Preview experience completion",
  "自定": "Custom",
  "为 5 类事件分别选择": "Choose sounds for 5 events",
  "未选择音频": "No audio selected",
  "选择": "Choose",
  "试听工具调用音效": "Preview tool call sound",
  "权限审批": "Permission approval",
  "试听权限审批音效": "Preview permission approval sound",
  "计划实行": "Plan execution",
  "试听计划实行音效": "Preview plan execution sound",
  "会话成功": "Session succeeded",
  "试听会话成功音效": "Preview session success sound",
  "会话失败": "Session failed",
  "试听会话失败音效": "Preview session failure sound",
  "局域网控制台": "LAN console",
  "开启后同一网络下的浏览器可以查看会话并处理审批。": "Let browsers on the same network view sessions and handle approvals.",
  "未运行": "Stopped",
  "启用服务": "Enable service",
  "首次监听时 Windows 会弹出防火墙授权，请选择“允许”。": "Windows will request firewall access the first time; choose Allow.",
  "在线客户端": "Connected clients",
  "暂无访问记录": "No access recorded",
  "高级选项": "Advanced options",
  "端口": "Port",
  "范围 1024–65535，被占用时会提示并保持关闭。": "Range 1024–65535. The service stays off if the port is in use.",
  "应用": "Apply",
  "访问令牌": "Access token",
  "令牌等同于密码。轮换后已登录的浏览器会立即失效。": "Treat the token like a password. Rotation signs out connected browsers immediately.",
  "显示": "Show",
  "隐藏": "Hide",
  "复制": "Copy",
  "轮换": "Rotate",
  "确认轮换": "Confirm rotation",
  "访问地址": "Access addresses",
  "在手机浏览器里打开任一地址，然后输入访问令牌。": "Open any address in a mobile browser, then enter the access token.",
  "未找到局域网地址，请检查网络连接。": "No LAN address found. Check the network connection.",
  "远程审批": "Remote approvals",
  "关闭时网页只能查看；开启后拿到令牌的人可以代你决定。": "When off, the web console is read-only. When on, token holders can decide for you.",
  "允许网页提交决定": "Allow decisions from the web",
  "Codex 的问题与计划仍然只读，只能在原终端完成。": "Codex questions and plans remain read-only and must be completed in the original terminal.",
  "记录远程决定": "Log remote decisions",
  "写入 lan-audit.log，标记来源为局域网。": "Write to lan-audit.log and mark the source as LAN.",
  "安全提示": "Security notice",
  "局域网内是明文 HTTP，请只在可信网络中开启。": "LAN traffic uses plain HTTP. Enable it only on trusted networks.",
  "任何拿到地址和令牌的人都能查看会话内容。": "Anyone with the address and token can view session content.",
  "开启远程审批意味着对方可以代你允许工具调用。": "Remote approval lets other people allow tool calls for you.",
  "不使用时关闭开关，端口会立即释放。": "Turn the service off when unused to release the port immediately.",
  "专注于 AI 编程会话的轻量桌面工作台。": "A lightweight desktop workspace for AI coding sessions.",
  "为编程工作流而生": "Built for coding workflows",
  "在一个安静、紧凑的界面中掌握编程助手的工作状态。": "Follow your coding assistants in one calm, compact interface.",
  "会话集中管理": "Unified session management",
  "汇总 Claude Code 与 Codex 会话": "Bring Claude Code and Codex sessions together",
  "关键请求及时处理": "Handle key requests quickly",
  "快速响应审批、提问与计划确认": "Respond to approvals, questions, and plan confirmations",
  "应用信息": "Application information",
  "当前安装版本及运行环境。": "Installed version and runtime environment.",
  "版本": "Version",
  "平台": "Platform",
  "Windows 桌面端": "Windows desktop",
  "技术": "Technology",
  "工作中": "Working",
  "等待输入": "Waiting for input",
  "需要处理": "Needs attention",
  "工具调用失败": "Tool call failed",
  "等待审批": "Waiting for approval",
  "停止": "Stopped",
  "空闲": "Idle",
  "调用工具中": "Calling a tool",
  "其他": "Other",
  "输入你自己的回答": "Enter your own answer",
  "再聊一下": "Discuss first",
  "先和 Claude 讨论这个问题": "Discuss this question with Claude first",
  "可多选": "Multiple selections",
  "单选": "Single selection",
  "在原 Codex 中选择": "Choose in Codex",
  "修复中": "Repairing",
  "执行中": "Running",
  "失败": "Failed",
  "已完成": "Completed",
  "等待 Claude 调用工具": "Waiting for Claude to call a tool",
  "等待 Claude 产生可读取的转录输出": "Waiting for readable transcript output from Claude",
  "Codex Hook 工具调用": "Codex Hook tool call",
  "Codex Hook 请求执行操作": "Codex Hook requested an action",
  "CodeCraft 会话同步与审阅 Hook": "CodeCraft session sync and review Hook",
  "安装Hook中...": "Installing Hook...",
  "卸载Hook中...": "Removing Hook...",
  "未安装": "Not installed",
  "安装": "Install",
  "卸载": "Remove",
  "预览模式下不生成二维码": "QR codes are unavailable in preview mode",
  "二维码": "QR code",
  "已复制": "Copied",
  "复制失败": "Copy failed",
  "尚未生成": "Not generated yet",
  "复制失败，请手动选择令牌文本": "Copy failed. Select the token text manually.",
  "请先为该事件选择音频文件": "Choose an audio file for this event first",
  "启动失败": "Failed to start",
  "运行中 · 可审批": "Running · approvals enabled",
  "运行中 · 只读": "Running · read-only",
  "端口需要是 1024–65535 之间的整数": "Port must be an integer from 1024 to 65535",
  "端口需要在 1024–65535 之间": "Port must be between 1024 and 65535",
  "最近访问：刚刚": "Last access: just now",
  "开启后同一网络中拿到地址和令牌的人都能查看会话。再次点击开关以确认开启。": "Anyone on the same network with the address and token can view sessions. Toggle again to confirm.",
  "轮换会立即让已登录的浏览器失效。再次点击“轮换”以确认。": "Rotation immediately signs out connected browsers. Select Rotate again to confirm.",
  "已允许网页提交决定：拿到令牌的人可以代你允许工具调用。": "Web decisions are enabled: token holders can allow tool calls for you.",
  "网页可以提交 Claude 的权限、问题与计划决定，以及 Codex 的审批。": "The web console can submit Claude permissions, questions, and plans, plus Codex approvals.",
  "网页当前只读，只能查看会话内容。": "The web console is read-only and can only view session content.",
  "服务运行中，端口与令牌暂不可修改。关闭服务后即可调整。": "Port and token cannot be changed while the service is running. Turn it off first.",
  "此问题来自外部 Codex 会话，请在原终端或 Codex 桌面任务中完成回答。": "This question comes from an external Codex session. Answer it in the original terminal or Codex desktop task.",
  "外部会话 · 请在原 Codex 界面回答": "External session · answer in Codex",
  "Codex Hook 正常": "Codex Hook connected",
  "Codex Hook 未安装": "Codex Hook not installed",
  "Claude Code Hook 已连接": "Claude Code Hook connected",
  "Claude Code Hook 连接正常": "Claude Code Hook connected",
};

const TRADITIONAL_OVERRIDES: Record<string, string> = {
  "设置": "設定",
  "打开设置": "開啟設定",
  "设置分类": "設定分類",
  "通用": "一般",
  "风格": "外觀",
  "语言": "語言",
  "界面语言": "介面語言",
  "选择后会立即应用到整个 CodeCraft 界面。": "選擇後會立即套用到整個 CodeCraft 介面。",
  "应用": "套用",
  "应用信息": "應用程式資訊",
  "Windows 桌面端": "Windows 桌面版",
  "暂无会话": "暫無會話",
  "暂无活跃会话": "暫無活躍會話",
  "只读": "唯讀",
  "访问令牌": "存取權杖",
  "访问地址": "存取位址",
};

const TRADITIONAL_PHRASES: readonly [string, string][] = [
  ["简体中文", "簡體中文"],
  ["关于", "關於"],
  ["活跃", "活躍"],
  ["经验", "經驗"],
  ["修复", "修復"],
  ["复制", "複製"],
  ["当前", "目前"],
  ["来源", "來源"],
  ["确认", "確認"],
  ["代码", "程式碼"],
  ["客户端", "用戶端"],
  ["登录", "登入"],
  ["刷新", "重新整理"],
  ["运行", "執行"],
  ["点击", "點擊"],
  ["控制", "控制"],
  ["设置", "設定"],
  ["界面", "介面"],
  ["会话", "會話"],
  ["局域网", "區域網路"],
  ["工具调用", "工具呼叫"],
  ["只读", "唯讀"],
  ["访问令牌", "存取權杖"],
  ["访问地址", "存取位址"],
  ["桌面端", "桌面版"],
  ["信息", "資訊"],
  ["文件", "檔案"],
  ["图片", "圖片"],
  ["音频", "音訊"],
  ["远程", "遠端"],
  ["网页", "網頁"],
  ["二维码", "QR 圖碼"],
  ["端口", "連接埠"],
  ["应用", "套用"],
  ["计划", "計畫"],
];

const TRADITIONAL_CHARACTER_MAP: Record<string, string> = {
  "语": "語", "体": "體", "选": "選", "择": "擇", "后": "後",
  "应": "應", "个": "個", "设": "設", "动": "動", "审": "審",
  "确": "確", "关": "關", "来": "來", "于": "於", "跃": "躍",
  "经": "經", "检": "檢", "细": "細", "络": "絡", "监": "監",
  "墙": "牆", "户": "戶", "级": "級", "范": "範", "机": "機",
  "对": "對", "总": "總", "运": "運", "讨": "討", "单": "單",
  "产": "產", "数": "數", "画": "畫", "随": "隨", "变": "變",
  "静": "靜", "猫": "貓", "块": "塊", "听": "聽", "会": "會",
  "弹": "彈", "着": "著", "释": "釋", "专": "專", "紧": "緊",
  "凑": "湊", "统": "統",
  "与": "與", "风": "風", "险": "險", "过": "過", "话": "話",
  "调": "調", "连": "連", "暂": "暫", "异": "異",
  "返": "返", "实": "實", "录": "錄", "问": "問", "题": "題",
  "项": "項", "额": "額", "输": "輸", "补": "補", "导": "導",
  "骤": "驟", "许": "許", "执": "執", "则": "則", "终": "終",
  "询": "詢", "绝": "絕", "继": "繼", "认": "認", "划": "劃",
  "辑": "輯", "内": "內", "类": "類", "览": "覽", "筛": "篩",
  "简": "簡", "当": "當", "仅": "僅", "迟": "遲",
  "请": "請", "装": "裝", "载": "載", "状": "狀", "态": "態",
  "无": "無", "均": "均", "别": "別", "节": "節",
  "启": "啟", "闭": "閉", "库": "庫", "量": "量", "预": "預",
  "结": "結", "验": "驗", "为": "為", "权": "權", "败": "敗",
  "务": "務", "线": "線", "处": "處",
  "端": "端", "轮": "輪", "换": "換", "显": "顯", "复": "複",
  "码": "碼", "远": "遠", "决": "決", "写": "寫",
  "标": "標", "记": "記", "开": "開", "轻": "輕", "编": "編",
  "程": "程", "汇": "彙", "键": "鍵", "响": "響",
  "环": "環", "术": "術", "闲": "閒", "阅": "閱",
  "获": "獲", "测": "測", "击": "擊", "读": "讀", "发": "發",
  "现": "現", "网": "網", "浏": "瀏", "围": "圍", "占": "佔",
  "据": "據", "并": "並", "将": "將", "销": "銷", "滤": "濾",
  "创": "創", "这": "這", "门": "門", "间": "間",
  "刚": "剛", "钟": "鐘", "时": "時", "声": "聲",
};

const toTraditional = (source: string): string => {
  const override = TRADITIONAL_OVERRIDES[source];
  if (override) return override;

  let translated = source;
  for (const [simplified, traditional] of TRADITIONAL_PHRASES) {
    translated = translated.split(simplified).join(traditional);
  }
  return Array.from(translated, (character) =>
    TRADITIONAL_CHARACTER_MAP[character] ?? character
  ).join("");
};

const NATIVE_LANGUAGE_LABELS = new Set(["简体中文", "繁體中文"]);

const translationForSource = (source: string, locale: AppLocale): string => {
  if (locale === "zh-CN") return source;
  if (locale === "zh-TW") return toTraditional(source);
  if (NATIVE_LANGUAGE_LABELS.has(source)) return source;
  return ENGLISH[source] ?? source;
};

const reverseTranslations = new Map<string, string>();
for (const source of Object.keys(ENGLISH)) {
  reverseTranslations.set(source, source);
  reverseTranslations.set(toTraditional(source), source);
  reverseTranslations.set(ENGLISH[source], source);
}

interface FormattedTranslation {
  patterns: Record<AppLocale, RegExp>;
  format: Record<AppLocale, (...values: string[]) => string>;
}

const translateHookAction = (action: string, locale: AppLocale): string => {
  const isInstall = action === "安装" || action === "安裝" || action === "Install";
  if (locale === "zh-CN") return isInstall ? "安装" : "卸载";
  if (locale === "zh-TW") return isInstall ? "安裝" : "卸載";
  return isInstall ? "Install" : "Remove";
};

const translateFilterValue = (value: string, locale: AppLocale): string => {
  const source = reverseTranslations.get(value);
  return source ? translationForSource(source, locale) : value;
};

const formattedTranslations: FormattedTranslation[] = [
  {
    patterns: {
      "zh-CN": /^(\d+) 个活跃$/,
      "zh-TW": /^(\d+) 個活躍$/,
      en: /^(\d+) active$/,
    },
    format: {
      "zh-CN": (count) => `${count} 个活跃`,
      "zh-TW": (count) => `${count} 個活躍`,
      en: (count) => `${count} active`,
    },
  },
  {
    patterns: {
      "zh-CN": /^第 (\d+)\/(\d+) 问题$/,
      "zh-TW": /^第 (\d+)\/(\d+) 問題$/,
      en: /^Question (\d+) of (\d+)$/,
    },
    format: {
      "zh-CN": (current, total) => `第 ${current}/${total} 问题`,
      "zh-TW": (current, total) => `第 ${current}/${total} 問題`,
      en: (current, total) => `Question ${current} of ${total}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(\d+) 项执行中$/,
      "zh-TW": /^(\d+) 項執行中$/,
      en: /^(\d+) running$/,
    },
    format: {
      "zh-CN": (count) => `${count} 项执行中`,
      "zh-TW": (count) => `${count} 項執行中`,
      en: (count) => `${count} running`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(\d+) 项记录$/,
      "zh-TW": /^(\d+) 項記錄$/,
      en: /^(\d+) records$/,
    },
    format: {
      "zh-CN": (count) => `${count} 项记录`,
      "zh-TW": (count) => `${count} 項記錄`,
      en: (count) => `${count} records`,
    },
  },
  {
    patterns: {
      "zh-CN": /^目录：(.*)$/,
      "zh-TW": /^目錄：(.*)$/,
      en: /^Directory: (.*)$/,
    },
    format: {
      "zh-CN": (value) => `目录：${value}`,
      "zh-TW": (value) => `目錄：${value}`,
      en: (value) => `Directory: ${value}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^筛选会话来源，当前为(.+)$/,
      "zh-TW": /^篩選會話來源，目前為(.+)$/,
      en: /^Filter session sources; current filter is (.+)$/,
    },
    format: {
      "zh-CN": (value) => `筛选会话来源，当前为${translateFilterValue(value, "zh-CN")}`,
      "zh-TW": (value) => `篩選會話來源，目前為${translateFilterValue(value, "zh-TW")}`,
      en: (value) => `Filter session sources; current filter is ${translateFilterValue(value, "en")}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^选择界面语言，当前为(.+)$/,
      "zh-TW": /^選擇介面語言，目前為(.+)$/,
      en: /^Choose interface language; current language is (.+)$/,
    },
    format: {
      "zh-CN": (value) => `选择界面语言，当前为${value}`,
      "zh-TW": (value) => `選擇介面語言，目前為${value}`,
      en: (value) => `Choose interface language; current language is ${value}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^最近访问：(\d+) 分钟前$/,
      "zh-TW": /^最近存取：(\d+) 分鐘前$/,
      en: /^Last access: (\d+) minutes ago$/,
    },
    format: {
      "zh-CN": (count) => `最近访问：${count} 分钟前`,
      "zh-TW": (count) => `最近存取：${count} 分鐘前`,
      en: (count) => `Last access: ${count} minutes ago`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(.+)设置内容待添加$/,
      "zh-TW": /^(.+)設定內容待新增$/,
      en: /^(.+) settings are coming soon$/,
    },
    format: {
      "zh-CN": (section) => `${translateFilterValue(section, "zh-CN")}设置内容待添加`,
      "zh-TW": (section) => `${translateFilterValue(section, "zh-TW")}設定內容待新增`,
      en: (section) => `${translateFilterValue(section, "en")} settings are coming soon`,
    },
  },
  {
    patterns: {
      "zh-CN": /^最近访问：(\d+) 小时前$/,
      "zh-TW": /^最近存取：(\d+) 小時前$/,
      en: /^Last access: (\d+) hours ago$/,
    },
    format: {
      "zh-CN": (count) => `最近访问：${count} 小时前`,
      "zh-TW": (count) => `最近存取：${count} 小時前`,
      en: (count) => `Last access: ${count} hours ago`,
    },
  },
  {
    patterns: {
      "zh-CN": /^最近访问：(\d+) 天前$/,
      "zh-TW": /^最近存取：(\d+) 天前$/,
      en: /^Last access: (\d+) days ago$/,
    },
    format: {
      "zh-CN": (count) => `最近访问：${count} 天前`,
      "zh-TW": (count) => `最近存取：${count} 天前`,
      en: (count) => `Last access: ${count} days ago`,
    },
  },
  {
    patterns: {
      "zh-CN": /^正在保存“(.+)”…$/,
      "zh-TW": /^正在儲存「(.+)」…$/,
      en: /^Saving “(.+)”…$/,
    },
    format: {
      "zh-CN": (value) => `正在保存“${value}”…`,
      "zh-TW": (value) => `正在儲存「${value}」…`,
      en: (value) => `Saving “${value}”…`,
    },
  },
  {
    patterns: {
      "zh-CN": /^已保存“(.+)”$/,
      "zh-TW": /^已儲存「(.+)」$/,
      en: /^Saved “(.+)”$/,
    },
    format: {
      "zh-CN": (value) => `已保存“${value}”`,
      "zh-TW": (value) => `已儲存「${value}」`,
      en: (value) => `Saved “${value}”`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(安装|卸载) (.+) Hook$/,
      "zh-TW": /^(安裝|卸載) (.+) Hook$/,
      en: /^(Install|Remove) (.+) Hook$/,
    },
    format: {
      "zh-CN": (action, name) => `${translateHookAction(action, "zh-CN")} ${name} Hook`,
      "zh-TW": (action, name) => `${translateHookAction(action, "zh-TW")} ${name} Hook`,
      en: (action, name) => `${translateHookAction(action, "en")} ${name} Hook`,
    },
  },
  {
    patterns: {
      "zh-CN": /^未检测到 (.+)，无法操作 Hook$/,
      "zh-TW": /^找不到 (.+)，無法操作 Hook$/,
      en: /^(.+) was not detected; the Hook cannot be managed$/,
    },
    format: {
      "zh-CN": (name) => `未检测到 ${name}，无法操作 Hook`,
      "zh-TW": (name) => `找不到 ${name}，無法操作 Hook`,
      en: (name) => `${name} was not detected; the Hook cannot be managed`,
    },
  },
  {
    patterns: {
      "zh-CN": /^点击(安装|卸载) (.+) Hook$/,
      "zh-TW": /^點擊(安裝|卸載) (.+) Hook$/,
      en: /^(Install|Remove) (.+) Hook$/,
    },
    format: {
      "zh-CN": (action, name) => `点击${translateHookAction(action, "zh-CN")} ${name} Hook`,
      "zh-TW": (action, name) => `點擊${translateHookAction(action, "zh-TW")} ${name} Hook`,
      en: (action, name) => `${translateHookAction(action, "en")} ${name} Hook`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(安装|卸载) Hook 失败：(.*)$/,
      "zh-TW": /^(安裝|卸載) Hook 失敗：(.*)$/,
      en: /^(Install|Remove) Hook failed: (.*)$/,
    },
    format: {
      "zh-CN": (action, error) => `${translateHookAction(action, "zh-CN")} Hook 失败：${error}`,
      "zh-TW": (action, error) => `${translateHookAction(action, "zh-TW")} Hook 失敗：${error}`,
      en: (action, error) => `${translateHookAction(action, "en")} Hook failed: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^刷新 Hook 状态失败：(.*)$/,
      "zh-TW": /^重新整理 Hook 狀態失敗：(.*)$/,
      en: /^Refresh Hook status failed: (.*)$/,
    },
    format: {
      "zh-CN": (error) => `刷新 Hook 状态失败：${error}`,
      "zh-TW": (error) => `重新整理 Hook 狀態失敗：${error}`,
      en: (error) => `Refresh Hook status failed: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^读取 Codex 状态失败：(.*)$/,
      "zh-TW": /^讀取 Codex 狀態失敗：(.*)$/,
      en: /^Failed to read Codex status: (.*)$/,
    },
    format: {
      "zh-CN": (error) => `读取 Codex 状态失败：${error}`,
      "zh-TW": (error) => `讀取 Codex 狀態失敗：${error}`,
      en: (error) => `Failed to read Codex status: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^生成二维码失败：(.*)$/,
      "zh-TW": /^產生 QR 圖碼失敗：(.*)$/,
      en: /^Failed to generate QR code: (.*)$/,
    },
    format: {
      "zh-CN": (error) => `生成二维码失败：${error}`,
      "zh-TW": (error) => `產生 QR 圖碼失敗：${error}`,
      en: (error) => `Failed to generate QR code: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^轮换令牌失败：(.*)$/,
      "zh-TW": /^輪換權杖失敗：(.*)$/,
      en: /^Token rotation failed: (.*)$/,
    },
    format: {
      "zh-CN": (error) => `轮换令牌失败：${error}`,
      "zh-TW": (error) => `輪換權杖失敗：${error}`,
      en: (error) => `Token rotation failed: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^保存自定义音效失败：(.*)$/,
      "zh-TW": /^儲存自訂音效失敗：(.*)$/,
      en: /^Failed to save custom sound: (.*)$/,
    },
    format: {
      "zh-CN": (error) => `保存自定义音效失败：${error}`,
      "zh-TW": (error) => `儲存自訂音效失敗：${error}`,
      en: (error) => `Failed to save custom sound: ${error}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^点击复制 (.*)$/,
      "zh-TW": /^點擊複製 (.*)$/,
      en: /^Click to copy (.*)$/,
    },
    format: {
      "zh-CN": (value) => `点击复制 ${value}`,
      "zh-TW": (value) => `點擊複製 ${value}`,
      en: (value) => `Click to copy ${value}`,
    },
  },
  {
    patterns: {
      "zh-CN": /^(工作中|等待输入|需要处理|工具调用失败|停止|空闲|等待审批) · (.+)$/,
      "zh-TW": /^(工作中|等待輸入|需要處理|工具呼叫失敗|停止|閒置|等待審批) · (.+)$/,
      en: /^(Working|Waiting for input|Needs attention|Tool call failed|Stopped|Idle|Waiting for approval) · (.+)$/,
    },
    format: {
      "zh-CN": (status, title) => `${translateFilterValue(status, "zh-CN")} · ${title}`,
      "zh-TW": (status, title) => `${translateFilterValue(status, "zh-TW")} · ${title}`,
      en: (status, title) => `${translateFilterValue(status, "en")} · ${title}`,
    },
  },
];

const translateFormatted = (
  value: string,
  locale: AppLocale,
): string | undefined => {
  for (const translation of formattedTranslations) {
    for (const sourceLocale of SUPPORTED_LOCALES) {
      const match = value.match(translation.patterns[sourceLocale]);
      if (match) return translation.format[locale](...match.slice(1));
    }
  }
  return undefined;
};

export const translateText = (value: string, locale: AppLocale): string => {
  const normalized = value.split("\u00a0").join(" ");
  const source = reverseTranslations.get(normalized);
  if (source) return translationForSource(source, locale);
  return translateFormatted(normalized, locale) ?? value;
};

const sourceTextFor = (value: string): string | undefined => {
  const normalized = value.split("\u00a0").join(" ");
  const source = reverseTranslations.get(normalized);
  if (source) return source;
  const simplified = translateFormatted(normalized, "zh-CN");
  return simplified === undefined ? undefined : simplified;
};

const splitOuterWhitespace = (value: string) => {
  const start = value.search(/\S/);
  if (start < 0) return { leading: value, core: "", trailing: "" };
  let end = value.length;
  while (end > start && /\s/.test(value[end - 1])) end -= 1;
  return {
    leading: value.slice(0, start),
    core: value.slice(start, end),
    trailing: value.slice(end),
  };
};

const LOCALIZED_ATTRIBUTES = ["aria-label", "title", "placeholder"] as const;
type LocalizedAttribute = (typeof LOCALIZED_ATTRIBUTES)[number];

let activeLocale: AppLocale = DEFAULT_LOCALE;
let initialized = false;
let observer: MutationObserver | undefined;
const textSources = new WeakMap<Text, string>();
const attributeSources = new WeakMap<Element, Partial<Record<LocalizedAttribute, string>>>();

const localizeTextNode = (node: Text) => {
  const parts = splitOuterWhitespace(node.data);
  if (!parts.core) return;

  const previousSource = textSources.get(node);
  if (previousSource) {
    const currentTranslation = translateText(previousSource, activeLocale);
    if (parts.core === currentTranslation) return;
  }

  const source = sourceTextFor(parts.core);
  if (!source) return;
  textSources.set(node, source);
  const translated = translateText(source, activeLocale);
  if (translated !== parts.core) {
    node.data = `${parts.leading}${translated}${parts.trailing}`;
  }
};

const localizeAttribute = (element: Element, attribute: LocalizedAttribute) => {
  const current = element.getAttribute(attribute);
  if (!current) return;

  const stored = attributeSources.get(element) ?? {};
  const previousSource = stored[attribute];
  if (previousSource && current === translateText(previousSource, activeLocale)) return;

  const source = sourceTextFor(current);
  if (!source) return;
  stored[attribute] = source;
  attributeSources.set(element, stored);
  const translated = translateText(source, activeLocale);
  if (translated !== current) element.setAttribute(attribute, translated);
};

const localizeElement = (element: Element) => {
  for (const attribute of LOCALIZED_ATTRIBUTES) {
    localizeAttribute(element, attribute);
  }
  for (const child of element.childNodes) {
    localizeNode(child);
  }
};

const localizeNode = (node: Node) => {
  if (node.nodeType === Node.TEXT_NODE) {
    localizeTextNode(node as Text);
  } else if (node.nodeType === Node.ELEMENT_NODE) {
    localizeElement(node as Element);
  }
};

const applyLocaleToDocument = () => {
  document.documentElement.lang = activeLocale;
  document.documentElement.dataset.locale = activeLocale;
  localizeElement(document.documentElement);
};

const persistLocale = () => {
  try {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, activeLocale);
  } catch {
    // The current-window language still applies when storage is unavailable.
  }
};

const languageOptionButtons = () =>
  Array.from(
    document.querySelectorAll<HTMLButtonElement>(
      "#language-menu [data-locale]",
    ),
  );

const ensureLanguageSelectChevron = (
  trigger: HTMLButtonElement,
  options: HTMLButtonElement[],
) => {
  const firstOption = options[0];
  const firstSlot = firstOption?.querySelector<HTMLElement>(
    ".language-select__option-chevron-slot",
  );
  const triggerChevron = trigger.querySelector<SVGSVGElement>(
    ".language-select__chevron",
  );
  if (!firstSlot || !triggerChevron || firstSlot.firstElementChild) return;

  const chevron = triggerChevron.cloneNode(true) as SVGSVGElement;
  chevron.classList.add("language-select__option-chevron");
  firstSlot.append(chevron);
};

const syncLanguageSelect = () => {
  const trigger = document.querySelector<HTMLButtonElement>("#language-trigger");
  const mark = document.querySelector<HTMLElement>("#language-mark");
  const label = document.querySelector<HTMLElement>("#language-label");
  const options = languageOptionButtons();
  const selected = LOCALE_OPTIONS.find((option) => option.locale === activeLocale)!;

  if (mark) {
    mark.textContent = selected.mark;
    mark.classList.toggle("language-select__mark--wide", selected.mark === "EN");
  }
  if (label) label.textContent = translateText(selected.label, activeLocale);
  if (trigger) {
    trigger.setAttribute(
      "aria-label",
      translateText(`选择界面语言，当前为${selected.label}`, activeLocale),
    );
    ensureLanguageSelectChevron(trigger, options);
  }
  for (const option of options) {
    option.setAttribute("aria-selected", String(option.dataset.locale === activeLocale));
  }
};

const syncLanguageSelectWidth = (
  select: HTMLElement,
  menu: HTMLElement,
) => {
  const previousWidth = select.style.getPropertyValue("--language-select-width");
  const wasHidden = menu.hidden;
  const previousOpenState = menu.dataset.open;

  select.style.removeProperty("--language-select-width");
  menu.dataset.measuring = "true";
  menu.hidden = false;

  const measuredWidth = Math.ceil(menu.getBoundingClientRect().width);
  if (measuredWidth > 0) {
    select.style.setProperty("--language-select-width", `${measuredWidth}px`);
  } else if (previousWidth) {
    select.style.setProperty("--language-select-width", previousWidth);
  }

  delete menu.dataset.measuring;
  menu.hidden = wasHidden;
  if (previousOpenState === undefined) delete menu.dataset.open;
};

export const currentLocale = (): AppLocale => activeLocale;

export const setLocale = (locale: AppLocale) => {
  if (locale === activeLocale) {
    syncLanguageSelect();
    return;
  }
  activeLocale = locale;
  persistLocale();
  applyLocaleToDocument();
  syncLanguageSelect();
  document.dispatchEvent(
    new CustomEvent("codecraft:locale-change", { detail: { locale } }),
  );
};

const initializeLanguageSelect = () => {
  const select = document.querySelector<HTMLElement>("#language-select");
  const trigger = document.querySelector<HTMLButtonElement>("#language-trigger");
  const menu = document.querySelector<HTMLElement>("#language-menu");
  const options = languageOptionButtons();
  if (!select || !trigger || !menu || options.length !== LOCALE_OPTIONS.length) return;

  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  let openFrame: number | undefined;
  let selectionPending = false;
  let widthObserver: ResizeObserver | undefined;

  const selectedOption = options.find(
    (option) => option.dataset.locale === activeLocale,
  );
  if (selectedOption) menu.prepend(selectedOption);
  syncLanguageSelect();
  syncLanguageSelectWidth(select, menu);

  const lockVisibleWidth = () => {
    if (select.getBoundingClientRect().width <= 0) return;
    if (select.style.getPropertyValue("--language-select-width")) return;
    syncLanguageSelectWidth(select, menu);
  };

  lockVisibleWidth();
  if (typeof ResizeObserver !== "undefined") {
    widthObserver = new ResizeObserver(lockVisibleWidth);
    widthObserver.observe(select);
  }

  const animateLanguageOptionToFirst = async (
    option: HTMLButtonElement,
  ) => {
    const currentOptions = languageOptionButtons();
    const firstOption = currentOptions[0];
    if (!firstOption || firstOption === option) return;

    const previousTops = new Map(
      currentOptions.map((currentOption) => [
        currentOption,
        currentOption.getBoundingClientRect().top,
      ]),
    );
    const chevron = firstOption.querySelector<SVGSVGElement>(
      ".language-select__option-chevron",
    );
    const selectedChevronSlot = option.querySelector<HTMLElement>(
      ".language-select__option-chevron-slot",
    );
    if (chevron && selectedChevronSlot) {
      selectedChevronSlot.replaceChildren(chevron);
    }
    menu.prepend(option);

    const animations = languageOptionButtons()
      .map((currentOption) => {
        const previousTop = previousTops.get(currentOption);
        if (previousTop === undefined) return undefined;
        const offsetY = previousTop - currentOption.getBoundingClientRect().top;
        if (Math.abs(offsetY) < 0.5) return undefined;
        return currentOption.animate(
          [
            { transform: `translateY(${offsetY}px)` },
            { transform: "translateY(0)" },
          ],
          {
            duration: LANGUAGE_MENU_TRANSITION_MS,
            easing: "cubic-bezier(0.2, 0.7, 0.35, 0.95)",
          },
        );
      })
      .filter((animation): animation is Animation => animation !== undefined);

    await Promise.all(
      animations.map((animation) => animation.finished.catch(() => undefined)),
    );
  };

  const closeMenu = (restoreFocus = false) => {
    if (closeTimer !== undefined) clearTimeout(closeTimer);
    if (openFrame !== undefined) {
      window.cancelAnimationFrame(openFrame);
      openFrame = undefined;
    }
    if (menu.hidden) {
      if (restoreFocus) trigger.focus({ preventScroll: true });
      return;
    }
    trigger.setAttribute("aria-expanded", "false");
    menu.dataset.open = "false";
    closeTimer = setTimeout(() => {
      menu.hidden = true;
      closeTimer = undefined;
    }, LANGUAGE_MENU_TRANSITION_MS + LANGUAGE_MENU_FADE_MS);
    if (restoreFocus) trigger.focus({ preventScroll: true });
  };

  const openMenu = (focusIndex = 0) => {
    if (closeTimer !== undefined) {
      clearTimeout(closeTimer);
      closeTimer = undefined;
    }
    if (openFrame !== undefined) window.cancelAnimationFrame(openFrame);
    menu.dataset.open = "false";
    menu.hidden = false;
    openFrame = window.requestAnimationFrame(() => {
      openFrame = undefined;
      trigger.setAttribute("aria-expanded", "true");
      menu.dataset.open = "true";
    });
    languageOptionButtons()[Math.max(0, Math.min(focusIndex, options.length - 1))]?.focus({
      preventScroll: true,
    });
  };

  const selectLanguageOption = async (option: HTMLButtonElement) => {
    if (selectionPending) return;
    selectionPending = true;
    menu.dataset.reordering = "true";
    menu.setAttribute("aria-busy", "true");

    try {
      await animateLanguageOptionToFirst(option);
      setLocale(parseLocale(option.dataset.locale));
      closeMenu(true);
    } finally {
      selectionPending = false;
      delete menu.dataset.reordering;
      menu.removeAttribute("aria-busy");
    }
  };

  trigger.addEventListener("click", () => {
    if (trigger.getAttribute("aria-expanded") === "true") closeMenu();
    else openMenu();
  });
  trigger.addEventListener("keydown", (event) => {
    if (!["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) return;
    event.preventDefault();
    openMenu(event.key === "ArrowUp" ? options.length - 1 : 0);
  });

  options.forEach((option) => {
    option.addEventListener("click", () => {
      void selectLanguageOption(option);
    });
    option.addEventListener("keydown", (event) => {
      if (selectionPending) {
        event.preventDefault();
        return;
      }
      let nextIndex: number | undefined;
      const currentOptions = languageOptionButtons();
      const currentIndex = currentOptions.indexOf(option);
      if (event.key === "ArrowDown") nextIndex = (currentIndex + 1) % currentOptions.length;
      else if (event.key === "ArrowUp") nextIndex = (currentIndex - 1 + currentOptions.length) % currentOptions.length;
      else if (event.key === "Home") nextIndex = 0;
      else if (event.key === "End") nextIndex = currentOptions.length - 1;
      else if (event.key === "Escape") {
        event.preventDefault();
        closeMenu(true);
        return;
      }
      if (nextIndex === undefined) return;
      event.preventDefault();
      currentOptions[nextIndex]?.focus({ preventScroll: true });
    });
  });

  document.addEventListener("pointerdown", (event) => {
    if (trigger.getAttribute("aria-expanded") !== "true") return;
    if (event.target instanceof Node && !select.contains(event.target)) closeMenu();
  });

};

export const initializeI18n = () => {
  if (initialized) return;
  initialized = true;
  try {
    activeLocale = parseLocale(window.localStorage.getItem(LANGUAGE_STORAGE_KEY));
  } catch {
    activeLocale = DEFAULT_LOCALE;
  }

  applyLocaleToDocument();
  initializeLanguageSelect();
  observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      if (mutation.type === "characterData") {
        localizeTextNode(mutation.target as Text);
      } else if (mutation.type === "attributes") {
        localizeAttribute(
          mutation.target as Element,
          mutation.attributeName as LocalizedAttribute,
        );
      } else {
        for (const node of mutation.addedNodes) localizeNode(node);
      }
    }
  });
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
    attributeFilter: [...LOCALIZED_ATTRIBUTES],
  });
};
