# Issue tracker: GitHub Issues

本仓库的 agent-ready issues 跟踪在私有 GitHub 仓库：

- Repository: `Baker4real/codex-account-switch-local-patches`
- Tracking branch: `private/main`
- Default working branch in this local checkout: `main` tracking `private/main`

`.scratch/` 只用于草稿 PRD、本地分析材料或用户明确要求的离线 issue 文件；不要把它当作默认 issue tracker。

## Conventions

- Use `gh issue create --repo Baker4real/codex-account-switch-local-patches` for new work items.
- Apply the repo's triage labels from `docs/agents/triage-labels.md`, normally `bug` / `ready-for-agent` when the issue is already confirmed and agent-ready.
- Include `Tracking branch: private/main` in the issue body when the work belongs to the private patch stack.
- Implementation closeout comments should name the commit on `private/main` and the verification commands that passed.

## When a skill says "publish to the issue tracker"

Create or update a GitHub issue in `Baker4real/codex-account-switch-local-patches` unless the user explicitly asks for local markdown.

## When a skill says "fetch the relevant ticket"

Use `gh issue view <number> --repo Baker4real/codex-account-switch-local-patches`. If the user passes a `.scratch/` path, treat that path as local draft context rather than the canonical tracker.
