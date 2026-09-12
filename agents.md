# File Tunnel backend API agent instructions

These instructions apply to this repository and every directory beneath it.

## Repository role

- This repository owns the File Tunnel control and data-plane reference API.
- A tunnel UUID is a routing identifier, never authorization.
- Preserve separate desktop and phone capabilities, digest-only credential
  storage, one-time pairing exchange, and one-time WebSocket event tickets.
- Keep pairing secrets in URI fragments and keep all capabilities, tickets,
  filenames, and user file metadata out of logs and telemetry.
- Treat filenames as metadata rather than storage paths. Preserve content,
  media-type, quota, expiry, cancellation, and lifecycle checks as fail-closed
  boundaries.
- Keep the executable Rust state machine and the formal model aligned whenever
  protocol behavior changes.

## Validation

- Run `nix develop --command agent-check` before completing a change.
- Run `bash scripts/formal-check.sh all` for protocol or capability changes.
- Never commit credentials, runtime `.env` files, build output, or private
  production data.

## Git workflow

- Keep changes focused and reviewable.
- Pull and merge remote work before pushing; avoid git rebase in favor of git merge.
- Never discard unrelated or uncommitted user work.

## Repository-local Git worktrees

- Create or use a Git worktree only when the human operator explicitly authorizes it for the current task. Concurrency or a dirty checkout is not permission by itself.
- Put every authorized worktree at `<repository-root>/tmp/worktrees/<name>`; from the repository root, use `./tmp/worktrees/<name>`. Never place worktrees beside repositories or organization directories.
- Keep `tmp`, `temp`, `tmp/worktrees`, and `temp/worktrees` ignored in the repository-root `.gitignore`. Do not commit files from those directories.
- Relocate or remove a worktree only when the operator explicitly requests it. Before removal, preserve and publish intended changes, verify its commit is represented on the target branch, and confirm there are no tracked, untracked, ignored-sensitive, or in-use files that must survive. Remove it with `git worktree remove <path>` without `--force`; never delete a worktree directory with `rm`.
