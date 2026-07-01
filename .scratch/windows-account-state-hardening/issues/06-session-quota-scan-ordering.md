# Windows quota session 扫描跳过 reparse 并按 mtime 选最新

Status: ready-for-agent

## What to build

让 Windows dashboard quota 的本地 session JSONL 扫描更可靠：扫描 `sessions/` 时跳过 symlink、junction、reparse point 和可能形成循环的目录；选择候选 session 时按文件 mtime 判断最新，而不是按路径字典序近似判断。

## Acceptance criteria

- [ ] `collect_jsonl_files` 递归前使用 `symlink_metadata` 或等价机制识别并跳过 symlink/reparse directory。
- [ ] 扫描对 junction、目录 symlink、祖先回环、超大外部目录有保护，不会递归到 repo/control root 之外的意外位置。
- [ ] `session_files_descending` 按 mtime 降序排序，路径仅作为 tie-breaker。
- [ ] quota cache fast path 的 newest 判断与 mtime 排序一致，不再假设 lex-largest 是最新。
- [ ] `take(32)` 选中的是 mtime 最新的 32 个 JSONL。
- [ ] 测试覆盖：lex 较小但 mtime 更新的 JSONL 胜出；sessions 下 junction/symlink 不递归。

## Blocked by

None - can start immediately
