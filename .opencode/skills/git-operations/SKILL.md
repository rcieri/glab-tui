---
name: git-operations
description: Use for ANY git operation — committing, branching, rebasing, changelog updates, or inspecting history. Triggers on git-related keywords.
---

## Git workflow for glab-tui

### Pre-commit checklist
```bash
git status
git diff --stat
git log --oneline -10
```

### Conventional commit format
```
<type>(<scope>): <subject>

<body>
```

Types: `feat`, `fix`, `refactor`, `docs`, `perf`, `test`, `chore`, `ci`.

### Branch hygiene
- Keep branches short-lived and rebased on `main`.
- Use `git rebase -i` to squash fixup commits before opening PRs/MRs.
- Never force-push to shared branches.

### Changelog
`CHANGELOG.md` follows Keep a Changelog format. Add entries under the correct section (`Added`, `Changed`, `Fixed`, `Removed`).

### Tagging
```bash
git tag -a v0.x.x -m "Release v0.x.x"
git push origin v0.x.x
```
