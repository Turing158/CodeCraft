# 兼容与能力矩阵

当前产品仅保留 Hook 只读观察。ACP 托管会话及审批回传已移除；下表 ACP 测试宿主仅是历史 Hook 协议研究证据，不是产品功能。

| 环境 | 证据 | 观察 | 工具批准 | 问答 | 计划批准 | 流式输出 |
| --- | --- | --- | --- | --- | --- | --- |
| bundled CLI 2.137.1，print | 真实隔离运行 | 主要提示/工具/Stop 已验证 | 产品关闭；allow/deny 单例有效，ask 不保证等待 | 未验证 | 未验证 | 关闭 |
| bundled CLI 2.137.1，ACP 测试宿主 | 真实隔离运行 | 部分生命周期/工具已验证 | 产品关闭 | 原生授权在 Hook 前，阻塞 | 正文/版本无法关联，阻塞 | 关闭 |
| Desktop 37.10.3-24 / 插件 5.5.3-wb… | 本机元数据 | 部署后需真实会话验收 | 关闭 | 关闭 | 关闭 | 关闭 |
| 未知 CLI/Desktop | 无批准协议证明 | 有界尽力观察，显示实际自报版本 | 关闭 | 关闭 | 关闭 | 关闭 |

不开放“一次允许/会话允许”，不显示可提交审批按钮，不把原生决定算作 CodeCraft 决定。当前没有验证 PATH 中每个独立 codebuddy-code/cbc 安装，只验证 WorkBuddy bundled bin/codebuddy；其他安装需重采。

默认用户配置为 %USERPROFILE%/.workbuddy/settings.json；WORKBUDDY_HOME 可定位独立配置根。安装器只管理 CodeCraft owner 的本地 marketplace 和 enabledPlugins，不重写用户 hooks/MCP/permissions。用户/项目/local settings 的全部优先级尚未冻结。

升级或身份缺失不会启用双向。后续需新增成功 fixture、完整待决状态机、超时/断线/重启测试和授权流程，不能只翻转能力标记。
