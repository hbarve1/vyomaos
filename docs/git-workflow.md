# Git Workflow & Best Practices

VyomaOS follows a **Git Flow** branching model adapted for the project's
phase-driven development cycle.

---

## Branch Model

```
main          ← stable, release-ready code
  └── develop ← integration branch, all feature PRs target here
        ├── feat/046-apple-ui-fidelity
        ├── fix/drag-ghost-trail
        ├── docs/roadmap-and-mindmap
        └── ...
```

### Long-lived branches

| Branch    | Purpose                              | Protected |
|-----------|--------------------------------------|-----------|
| `main`    | Stable releases only                 | Yes       |
| `develop` | Integration — **PR target for all feature branches** | Yes |

### Short-lived branches

Created from `develop`, merged back into `develop` via PR, then deleted.

| Prefix      | Use case                          | Example                          |
|-------------|-----------------------------------|----------------------------------|
| `feat/`     | New features or phase work        | `feat/043-core`                  |
| `fix/`      | Bug fixes                         | `fix/drag-ghost-trail`           |
| `refactor/` | Code restructuring (no behavior change) | `refactor/compositor-split` |
| `docs/`     | Documentation only                | `docs/roadmap-and-mindmap`       |
| `spec/`     | Specification work                | `spec/quality-spec-improvements` |
| `chore/`    | Maintenance, CI, tooling          | `chore/cleanup-gitignore`        |
| `test/`     | Test infrastructure               | `test/actual-merge`              |
| `perf/`     | Performance improvements          | `perf/gui-60fps`                 |
| `ci/`       | CI/CD pipeline changes            | `ci/github-actions-test-pipeline`|

### Numbered feature branches

For phase-driven work, prefix with a three-digit phase number:

```
feat/046-apple-ui-fidelity
feat/043-core
042-interactive-window-management
```

---

## Development Lifecycle

### 1. Start a feature (with worktree)

All feature work **must** use a git worktree under `.worktrees/` for isolation.
Never commit directly on `main` or `develop`.

```sh
# Ensure develop is up to date
git fetch origin
git checkout develop
git pull origin develop

# Create a feature branch + worktree in one step
git worktree add .worktrees/feat-NNN-short-description -b feat/NNN-short-description develop

# Move into the worktree
cd .worktrees/feat-NNN-short-description
```

This gives you a separate working directory for the feature while keeping
`develop` clean in the main checkout.

### 2. Work on the feature

- Make small, focused commits (see commit conventions below).
- Keep your branch up to date with `develop`:

```sh
git fetch origin
git rebase origin/develop
```

### 3. Open a Pull Request

- **Always target `develop`** — never `main` directly.
- Use a clear title and description.
- Link related issues if any.

### 4. Review & merge

- Squash-merge or regular merge — maintainer's discretion.
- Delete the feature branch after merge.
- **Clean up the worktree**:

```sh
# From the main repo checkout (not from inside the worktree)
git worktree remove .worktrees/feat-NNN-short-description
git branch -d feat/NNN-short-description
```

### 5. Release to `main`

When `develop` is stable and ready for release:

```sh
git checkout main
git merge develop
git tag vX.Y.Z
git push origin main --tags
```

Only maintainers merge `develop` → `main`.

---

## Commit Message Conventions

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>

Optional body for non-obvious decisions.
```

### Types

| Type       | When to use                                    |
|------------|------------------------------------------------|
| `feat`     | New feature or capability                      |
| `fix`      | Bug fix                                        |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `docs`     | Documentation only                             |
| `spec`     | Specification or design documents              |
| `test`     | Adding or updating tests                       |
| `chore`    | Build, CI, tooling, dependencies               |
| `perf`     | Performance improvement                        |
| `style`    | Formatting, whitespace (no logic change)       |

### Scopes

Use the affected component as scope:

```
feat(compositor): wire animation alpha and context menu
fix(shell): handle empty input on backspace
spec(r43): FINAL File Coordination & Locking
docs(roadmap): add ROADMAP.md
chore(ci): add GitHub Actions test pipeline
```

### Rules

- Subject line: imperative mood, lowercase, no period, under 72 chars.
- Body: explain WHY, not WHAT (the diff shows what changed).
- One logical change per commit.

---

## Branch Protection Rules

### `main`
- No direct pushes — merge from `develop` only.
- Requires passing CI (when configured).
- Only maintainers can merge.

### `develop`
- No direct pushes — merge via PR from feature branches.
- Requires at least one approval (when team grows).
- CI must pass before merge.

---

## Common Scenarios

### Keeping a feature branch up to date

```sh
git fetch origin
git rebase origin/develop
# Resolve conflicts if any, then:
git push --force-with-lease
```

Use `--force-with-lease` (not `--force`) to avoid overwriting others' work.

### Fixing a bug on `develop`

```sh
git fetch origin
git worktree add .worktrees/fix-short-description -b fix/short-description origin/develop
cd .worktrees/fix-short-description
# fix, commit, push, open PR → develop
# then clean up:
cd ../..
git worktree remove .worktrees/fix-short-description
```

### Hotfix on `main` (critical production bug)

```sh
git fetch origin
git worktree add .worktrees/fix-hotfix -b fix/hotfix-description origin/main
cd .worktrees/fix-hotfix
# fix, commit, push, open PR → main
# After merge, also merge main back into develop:
git checkout develop
git merge main
git push origin develop
# Clean up:
cd ../..
git worktree remove .worktrees/fix-hotfix
```

### Reverting a bad merge

```sh
git revert -m 1 <merge-commit-hash>
# Creates a new commit that undoes the merge — safe and auditable.
```

---

## Do's and Don'ts

### Do

- Pull/rebase before starting new work.
- Write meaningful commit messages.
- Keep PRs focused — one feature or fix per PR.
- **Use worktrees** (`.worktrees/`) for all feature and fix work.
- Delete branches and remove worktrees after merge.
- Use `git rebase` to keep a clean history on feature branches.
- Run `make unit-test` before opening a PR.

### Don't

- **Don't commit directly to `main` or `develop`** — always use a feature branch + PR.
- Don't use `git push --force` — use `--force-with-lease`.
- Don't commit generated files (`out/`, `target/`).
- Don't commit secrets (`.env`, credentials, API keys).
- Don't create merge commits on feature branches — rebase instead.
- Don't leave stale branches or orphaned worktrees — clean up after merge.

---

## Git Worktrees

All development uses [git worktrees](https://git-scm.com/docs/git-worktree)
under the `.worktrees/` directory (already in `.gitignore`).

### Why worktrees?

- **Isolation**: Each feature gets its own working directory — no stashing or
  switching branches in the main checkout.
- **Parallel work**: Work on multiple features simultaneously without conflicts.
- **Clean main checkout**: `develop` in the root checkout stays pristine for
  quick reference and rebasing.

### Quick reference

```sh
# List active worktrees
git worktree list

# Create a new worktree + branch from develop
git worktree add .worktrees/<name> -b <branch> develop

# Remove a worktree after merge
git worktree remove .worktrees/<name>

# Prune stale worktree metadata
git worktree prune
```

### Directory layout

```
vyomaos/                          ← main checkout (always on develop)
├── .worktrees/                   ← gitignored
│   ├── feat-046-ui-fidelity/     ← worktree for feat/046-ui-fidelity
│   ├── fix-drag-ghost/           ← worktree for fix/drag-ghost
│   └── docs-api-reference/       ← worktree for docs/api-reference
├── supervisor/
├── apps/
└── ...
```

---

## Git Configuration Tips

```sh
# Rebase by default on pull (avoids merge commits)
git config pull.rebase true

# Prune deleted remote branches on fetch
git config fetch.prune true

# Sign commits (optional, recommended)
git config commit.gpgsign true
```
