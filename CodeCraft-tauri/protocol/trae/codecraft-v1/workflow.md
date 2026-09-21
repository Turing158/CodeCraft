# CodeCraft 工作流

用户在 CodeCraft 创建或恢复受保护任务，将复制的启动指令发到目标 Trae 会话。只安装 MCP 不会启用计划保护。必须等待 UserPromptSubmit 返回绑定的 taskId、planId、documentPath 和 baseRevision。

先直接编辑指定计划文件，再调用 `codecraft_review_plan`，传入 schemaVersion=1、绑定的 planId、当前 baseRevision、documentPath 和完整 planMarkdown。bridgeTicket 由原生 Hook 注入，不要自行生成会话或任务身份。

只有 `completed` 且 decision=`approved` 才表示用户批准了该快照。`changes_requested` 时按反馈修订并重新提交；`rejected`、`cancelled`、`expired` 或 `failed` 时停止执行并向用户说明。后续工具仍须经过当前轮次的计划闸门和全局工具策略。新用户输入、Stop、文件变化、暂停、重启都可能使批准失效；不能复用旧批准。

需要用户输入时调用 `codecraft_ask_user`，传入 schemaVersion=1 和 questions。题目包含 questionId、prompt、kind、options、required、allowText、minSelections、maxSelections；每项包含 optionId、label、description（可为 null）。text 题 options=[]、allowText=true、选择上下限均为 0。按返回的 questionId/optionId 解释答案，不根据重复标签猜测。

原生 AskUserQuestion、Plan/Spec 卡片仍需在 Trae 中处理；MCP 结果不关闭这些卡片。工具允许已交给 Trae 后可能仍等待原生确认，CodeCraft 不能撤回；计划失效时请在 Trae 取消旧调用，再开始新的计划执行。
