# Issue tracker: GitHub Issues

本仓库的 agent-ready issues 跟踪在私有 GitHub 仓库：

- Repository: `Baker4real/codex-account-switch-local-patches`
- Tracking branch: `main`
- Default working branch in this local checkout: `main` tracking `private/main`

`.scratch/` 只用于草稿 PRD、本地分析材料或用户明确要求的离线 issue 文件；不要把它当作默认 issue tracker。

## Conventions

- 创建：`gh issue create --repo Baker4real/codex-account-switch-local-patches --title "..." --body "..."`
- 读取：`gh issue view <number> --repo Baker4real/codex-account-switch-local-patches --comments`
- 列出：`gh issue list --repo Baker4real/codex-account-switch-local-patches --state open --json number,title,body,labels,comments`
- 评论：`gh issue comment <number> --repo Baker4real/codex-account-switch-local-patches --body "..."`
- 标签：`gh issue edit <number> --repo Baker4real/codex-account-switch-local-patches --add-label "..."` 或 `--remove-label "..."`
- 关闭：`gh issue close <number> --repo Baker4real/codex-account-switch-local-patches --comment "..."`

使用 `docs/agents/triage-labels.md` 定义的标签。已确认且可由 agent 直接执行的问题通常使用 `bug` / `ready-for-agent`。属于私有补丁主线的 issue 正文应包含 `Tracking branch: main`。实现收尾评论应注明 `main` 上的提交及通过的验证命令。

## Pull requests as a triage surface

**PRs as a request surface: no.**

不要把外部 PR 自动纳入 triage 请求队列。GitHub 的 issue 与 PR 共用编号空间；裸编号 `#42` 存在歧义时，先运行 `gh pr view 42 --repo Baker4real/codex-account-switch-local-patches`，失败后再读取对应 issue。

## When a skill says "publish to the issue tracker"

在 `Baker4real/codex-account-switch-local-patches` 创建或更新 GitHub issue，除非用户明确要求使用本地 Markdown。

## When a skill says "fetch the relevant ticket"

运行 `gh issue view <number> --repo Baker4real/codex-account-switch-local-patches --comments`。如果用户传入 `.scratch/` 路径，将其视为本地草稿上下文，而不是 canonical tracker。

## Wayfinding operations

`/wayfinder` 使用一个 map issue 和若干 child issues：

- Map：单个带 `wayfinder:map` 标签的 issue，正文保存 Notes、Decisions-so-far 和 Fog。
- Child ticket：优先使用 GitHub sub-issue 关联到 map；不可用时，在 map 的任务列表中加入 child，并在 child 正文顶部写 `Part of #<map>`。标签使用 `wayfinder:<type>`，其中 type 为 `research`、`prototype`、`grilling` 或 `task`。
- Blocking：优先使用 GitHub 原生 issue dependencies。调用 dependencies API 时传 blocker 的数字 database id，不要传 `#number` 或 `node_id`。原生依赖不可用时，在 child 正文顶部维护 `Blocked by: #<n>, #<n>`。
- Frontier：按 map 顺序查找仍开放、没有开放 blocker、且尚未分配负责人的第一个 child。
- Claim：`gh issue edit <n> --repo Baker4real/codex-account-switch-local-patches --add-assignee @me`，这是会话的首次写操作。
- Resolve：先评论答案，再关闭 child，最后把上下文指针与链接追加到 map 的 Decisions-so-far。
