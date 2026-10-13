# CodeCraft Cursor 工作流核心 v1

2026-09-22：P1 独立核心已实现业务 schema、串行请求状态机、单写入者持久化、受限文件通道、Windows 项目身份、票据事务、安装恢复和会话展示存储；已提取为正式 `cursor-core`，由 Tauri 与探针共同依赖。**生产采集/安装/桌面/LAN 已接入，审批工作流仍未启用。** 详见 [产品接入状态](../product-integration.md)。 这些是 CodeCraft 自有类型，不是 Cursor 原生协议能力声明。

## 已实现范围

| 模块 | 实现 |
| --- | --- |
| [cursor_schema.rs](../../../src-tauri/src/cursor_schema.rs) | 严格 JSON、自有工具输入、问答答案、计划任务/阶段关联、完整业务/内容摘要；去除 bridgeTicket 后才保存业务数据 |
| [cursor_workflow.rs](../../../src-tauri/src/cursor_workflow.rs) | 按安装/代次/可信会话/轮次/工具/业务 ID 去重；计划显式替换；人工决定版本比较；唯一结果 owner；传输租约、原生终止、期限和计划限制 |
| [cursor_store.rs](../../../src-tauri/src/cursor_store.rs) | 进程级排他锁、同目录临时文件、sync 和原子替换；成功持久化后才应答；新代次恢复；磁盘故障后停止接受操作 |
| [cursor_files.rs](../../../src-tauri/src/cursor_files.rs) | 自有目录标记、当前用户受保护 ACL、祖先路径/reparse point/硬链接检查、受控文件名、独占锁和有界原子文件操作 |
| [cursor_projects.rs](../../../src-tauri/src/cursor_projects.rs) | Windows 目录句柄、最终路径、卷 GUID/序列号/文件 ID/创建时间、完整多根集合及项目 revision；拒绝网络工作区 |
| [cursor_tickets.rs](../../../src-tauri/src/cursor_tickets.rs) | 随机票据的候选/激活/消费/撤销/过期；服务器和业务摘要绑定；与请求/计划登记共用 Model 和 Store 事务；statusOnly 与 RPC 重放隔离 |
| [cursor_transport.rs](../../../src-tauri/src/cursor_transport.rs) | 按角色分配连接凭据、按 appEpoch 隔离队列、3 秒快速命令、单调心跳、持久化认领和一次性回复；禁止重开旧代队列 |
| [cursor_install.rs](../../../src-tauri/src/cursor_install.rs) | 安装锁、逐文件 before/after hash、独立 `.bak`、阶段日志、manifest 最后提交、保留用户改动的恢复、Hook/MCP 自有条目结构化合并 |
| [cursor_history.rs](../../../src-tauri/src/cursor_history.rs) | observedOnly 稳定会话键、事件/语义去重、旧轮次隔离、只读恢复、损坏记录隔离、容量与时间保留、待处理请求可见性 |
| [cursor_workflow_tests.rs](../../../src-tauri/src/cursor_workflow_tests.rs) | 业务冲突、并发重试/修订/相反决定、旧事件隔离、重启、预算和跨语言摘要测试 |

`IssueTicket / ActivateTicket / ConsumeTicket` 已实现独立票据状态链路。发行随机票据，磁盘只保存其 SHA-256；票据的 Debug/Serialize 输出固定脱敏，只有专门的 `for_injection` 方法可供未来可信候选适配器写入自有参数。候选阶段不登记业务卡片或限制；激活与业务幂等/计划登记在同一持久化事务内完成，消费绑定唯一连接与原始 RPC ID。相同帧仅返回 ProtocolDuplicate；同一 ID 改参、改票据或转移结果 owner 被拒绝。

这些命令仍要求未来适配器先验证原生事件、有效服务器配置和 P0 能力；不能把调用 Rust 方法当成 Cursor 身份证据。`Activate / Enter` 保留为底层独立模型入口，不是生产可用的绕过方式。`Command` 不支持反序列化，未开放可由模型、LAN 或文件直接提交的特权命令入口。当前真实 Cursor 版本没有通过发行票据的前提，因此没有向 Cursor 注入这些票据。

`ConfirmGeneration` 必须由未来适配层在取得真实开始证据后调用；候选提示不能调用它。项目路径通过 `SetProjectPath` 同步更新文件身份表和模型 revision；票据激活/消费重新校验已报告根，用户提交及结果认领重开目录核对，普通限制查询在项目身份失效时返回 Unavailable。原生调用关联、实际工作区来源和客户端剩余预算仍由已验证适配层提供，不推断最近会话或父子关系。

## 自有业务输入

两类工具均要求 `schemaVersion: 1`、`clientRequestId`。ID 使用最多 128 字节的 ASCII 字母、数字、`_ . : -`。`bridgeTicket` 可省略，提供时要求非空字符串、最多 512 字节；它不参与业务摘要、不保存到业务快照，也不构成授权。

`codecraft_ask_user` 的 `questions` 为 1—32 项。每题提供：

- `questionId`、`prompt`、`options: [{optionId, label}]`。
- `multiSelect`、`allowFreeText`、`required`、`allowSkip`。
- `maxTextLength`：最多 4096 个 Unicode 字符；允许自由文本时必须大于 0。

每题最多 64 个选项，ID 不重复；标签可以相同。纯文本题使用空 options 与 `allowFreeText: true`。答案使用题目/选项 ID 关联，所有题目各出现一次；空必填答案、重复选项、未知 ID、非法多选及未经允许的跳过均拒绝。跳过与文本/选项答案互斥。不同题目的答案顺序不影响对应关系。

`codecraft_review_plan` 要求 `planId`、`baseRevision`、`title`、`overview`、完整 `markdown`、`todos`、`phases`。更新时另带 `supersedesClientRequestId`；首次提交该字段可省略或为 null。

- todo 使用 `{id, description, status, phaseId?}`，status 为 `pending / inProgress / completed / cancelled`，最多 256 项。
- phase 使用 `{id, title, description}`，最多 64 项；引用的 phaseId 必须存在。
- Markdown 最多 128 KiB，标题 2 KiB，概要 16 KiB；正文不会被静默截断后用于批准。
- 首次计划 baseRevision 为 0；更新必须比较当前 revision 和显式替换的 clientRequestId。新请求即使内容完全相同，也递增 revision 并重新等待人工决定。

JSON 总输入上限为 1 MiB。未知业务字段、重复键、不支持的版本、重复任务/阶段 ID 和无效引用均拒绝。schema 校验不 trim 正文、不更改换行或 Unicode。原始业务字段是否省略/null 仍参与 argumentsDigest；planContentDigest 只包含 v4 指定的完整审阅内容。

[共享摘要样例](fixtures/business-digests.json)由 Rust 与 JavaScript 两侧分别校验，覆盖 Unicode、CRLF/LF、相同内容的新审阅和票据排除；它们全部标为 synthetic。

## 状态与恢复行为

精确重试先于计划版本检查，只返回当前请求状态与必要版本元数据；不增加卡片、延长期限、接管 owner 或返回历史答案/批准。相同业务 ID 改参返回冲突。同一原生调用不能绑定两个业务请求，同一 MCP 连接与 RPC ID 也只有一个 owner；数字 ID 0 是有效 ID。

计划从可信登记开始建立临时限制，批准决定记录后仍保留限制，直至同一当前版本的结果在有效租约中报告 written。人工要求修改、拒绝或取消另存修订要求；后续请求的超时、断连、原生取消或配置变更不会把它抹去。独立计划分别限制。替换先建立新版本限制，再终止旧请求；旧事件/迟到写入只补充原请求，不解除新版本限制。

当前模型的普通工具策略查询只返回 `NormalPolicy / ReviewPending / RevisionRequired / Unavailable`，不等于工具允许决定，也没有改变现有全局审批策略。只读例外必须由未来经过验证的工具分类提供。`Store::gate` 先处理经过的期限，并把不可用存储映射为 Unavailable；适配层以后再按 D3-B 返回普通 Hook 的合法回退，自有 MCP 则拒绝无效交互。

使用同代单调时间：接入最多 60 秒，受票据发行后 600 秒和已验证客户端余量约束；进入服务后切换为最多 300 秒人工等待，不再由未消费票据 TTL 终止。扣除 10 秒余量后不足 30 秒，不建立 pending 表单，并结束对应临时等待。结果租约最多 5 秒；超时后记为 outcomeUnknown，不重新认领或自动重发。written 只说明本地写出，不是 Cursor 接受/执行确认。

Store 使用 `private_directory` 认领空目录或带有效自有标记的目录，设置当前进程用户 SID 的受保护 ACL，并由新文件继承。修改 ACL 前检查所有祖先、目录项及硬链接，拒绝借 junction 或硬链接修改其他位置。固定状态路径通过受控名称构造，持有 broker.lock，并通过同目录临时文件和 Windows `MoveFileExW` 替换 state.json。所有测试仅使用忽略的 `scripts/cursor-probe/runs/` 和 `src-tauri/cursor-core/runs/`；没有修改用户级 Cursor 配置。

状态文件当前上限 1 MiB，业务记录和票据各最多 1024 条；超限不删除 pending 或人工拒绝以腾空间。写入失败或结果不确定时停止接受后续命令，要求重新打开并生成新 appEpoch。损坏的已提交业务状态不会静默清空；孤立临时文件不作为已提交状态载入。展示历史另行保存，不参与业务授权或幂等标记清理。

重启保留已记录决定作为只读历史，旧 pending 变为 disconnected，已 claimed 的结果变为 outcomeUnknown；票据、旧 owner、lease、幂等索引、活动计划限制和会话轮次不会转移。旧代次提交、终止命令和传输回报不能获得新授权，重新请求必须重新绑定和人工决定。

## 文件通道、安装与会话存储

文件通道在同用户 ACL 内按 Hook/MCP/native 角色登记随机连接凭据，磁盘连接表只保存凭据哈希；使用不可反序列化的 Permit/Delivery 对象区分可信注册与收到的非可信载荷。最多 32 条连接，每连接 8 个待处理快速消息，队列 128 项，每代保留最多 1024 条认领记录。角色标记或文件存在不代表已验证原生能力；生产 Hook 采集已接入 Windows 进程身份校验、一次性文件握手和有界事件分发；MCP/native 授权未开放。

同一 UUID 消息先持久化认领，再交给调用方；重复不重放结果，写入结果不确定时不自动重试。UUID 文件名不承担消息顺序，按连接发送序号处理；原子写入中的 `.tmp` 不会被误当作消息。旧 appEpoch 不可重新打开，存活旧客户端看到新代心跳即拒绝旧提交/回复。客户端依据握手后的单调时间判断 10 秒失联，不能靠重读旧心跳或读取迟到的旧心跳续期。业务结果仍须另行核对 Store 的 owner/lease；通道成功写文件不等于 Cursor 接受结果。

安装器是独立文件事务引擎，目标父目录须已存在。预备阶段验证全部目标和输出并保存不覆盖历史的 `.bak`；逐步复核 before hash，最后验证输出/就绪条件后提交 manifest。中断恢复通过当前内容判断是否恢复，不能仅相信 completed 标记；用户后续修改或备份被改动均保留为冲突。日志写入失败后实例停止操作，需重新打开恢复。MCP/Hook 合并仅比较并替换上次记录的完整自有条目，不以名字或命令片段认领其他配置。产品配置已加入同一事务；目录创建、维护撤销、空闲辅助进程退出、移动安装路径修复和桌面设置已接入。manifest 不授予审批能力；项目工作流启用仍以 P0 通过为前提。

history 仅接收适配器规范化后的可信事件类型，不读取 Cursor 数据库、transcript_path 或未捕获消息。完成回答、工具和子代理摘要均为只读展示；不从助手文字推断审批。稳定键包含来源、安装 ID 和 conversation ID。重启不更新时间、不触发通知，旧 working 变为 interrupted；无新活动 10 分钟变为 unknown，不伪称工具完成。晚到 sessionStart/旧 generation 结束不会复活当前状态，同文本的不同事件不会被错误吞掉。

展示记录以 7 天、1000 会话、100 MiB 为保留边界，优先移除最旧非活跃记录；当前可提交请求的会话由可信 broker pins 强制保留。单会话最多 256 个展示项、2048 个事件幂等标记，正文截断明确标识；容量不足返回背压，不删除当前请求。损坏 JSON 移到独立 `.corrupt` 文件，其他会话可继续加载。快照不包含票据、连接凭据或文件操作路径，视图序号会跟随可见性/pins 的实际变化；所有历史内容始终只读，完整待提交表单仍来自业务存储。该投影已接入桌面/LAN 列表及既有清理、排序和来源标识。

## 验证与剩余工作

运行 `scripts/cursor-probe/check.ps1`：正式核心与独立探针分别运行 Rust 测试，另有 **17 项 Node 检查**；生产集成的验证记录见 [产品接入状态](../product-integration.md)。此前 87 项 Rust 基线覆盖 ACL、真实 Windows junction/目录更名、安装中断、票据事务、文件消息认领、失联心跳与历史恢复。脚本现在同时执行 `cargo fmt --check` 和 `cargo clippy --all-targets -- -D warnings`；全部通过，执行过程均限制为两个逻辑核心。

已覆盖 v4 的 V07/V08/V11/V12/V18—V24 中可独立验证的状态机行为，包括真实子进程在结果认领后直接退出的恢复测试。**这些是 CodeCraft 模型验证，不把对应 Cursor P0/V 项改记为实机通过。** 原生事件映射、端到端延迟、真实预算和 UI 行为仍需适配层及实机验证。

剩余工作集中于 P0 的 MCP 身份绑定与原生共享审批接口，以及依赖这些能力的真实轮次/终止映射、可提交三类视图、项目工作流启用和完整实机验收。具体未完成项见 [产品接入状态](../product-integration.md)。不得把本地模型、进程测试或浏览器合成预览计作 Cursor 实机通过。
