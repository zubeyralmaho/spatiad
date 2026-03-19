# Contributing to Spatiad

## Branch strategy

- `main`: always releasable
- `feat/*`: new features
- `fix/*`: bug fixes
- `docs/*`: documentation updates
- `chore/*`: maintenance tasks

Examples:

- `feat/dispatch-timeout-wheel`
- `fix/ws-reconnect-race`
- `docs/ws-protocol-v1`

## Commit convention

Spatiad uses Conventional Commits.

Format:

```text
type(scope): short summary
```

Valid `type` values:

- `feat`
- `fix`
- `docs`
- `refactor`
- `test`
- `chore`

Examples:

- `feat(dispatch): add radius expansion scheduler`
- `fix(ws): handle reconnect replay ordering`
- `docs(api): add webhook signature examples`

## Pull requests

1. Rebase branch onto `main`.
2. Keep PR focused to one concern.
3. Fill the PR template fully.
4. Add at least one required PR label: `feat`, `fix`, `docs`, `chore`, `refactor`, or `test`.
5. Ensure CI passes before requesting review.

## CODEOWNERS

Path ownership is enforced through `.github/CODEOWNERS`.
Any PR touching owned paths requires review by listed owners when code-owner review is enabled.

## Releases

Tagging a commit with `vX.Y.Z` triggers the release workflow and publishes release binaries for Linux and macOS.

## Local validation

### Rust

```bash
cd rust
cargo check
```

### TypeScript

```bash
cd typescript
pnpm install
pnpm -r build
```
