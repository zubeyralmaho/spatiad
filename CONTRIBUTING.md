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
4. Ensure CI passes before requesting review.

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
