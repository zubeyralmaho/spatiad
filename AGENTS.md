# AGENTS.md

Guidelines for AI agents contributing to Spatiad. This document complements [CLAUDE.md](./CLAUDE.md) with rules specific to automated or semi-automated contributions.

## General rules

1. **Read before you write.** Always read the relevant source files before proposing changes. Understand the existing patterns — do not guess.
2. **Minimal diffs.** Change only what is necessary. Do not refactor surrounding code, add unsolicited comments, or reformat files you did not modify.
3. **No hallucinated APIs.** If you are unsure whether a function, trait, or endpoint exists, search the codebase first. Do not invent method signatures.
4. **Preserve crate boundaries.** Types flow downward: `bin → api → dispatch → core → h3 → types`. Never introduce upward or circular dependencies.
5. **Run checks before declaring done.** At minimum: `cargo check` (Rust) and `pnpm -r build` (TypeScript). Run `cargo test` and `pnpm -r test` if your changes touch logic.

## Commit discipline

- Follow **Conventional Commits** exactly: `type(scope): summary`
- Valid types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`
- Valid scopes: crate names (`core`, `api`, `dispatch`, `h3`, `types`, `ws`, `bin`), `sdk`, `express-plugin`, `ci`, `docs`, `security`, `ops`
- Keep commits atomic — one logical change per commit
- Never use `--no-verify` to skip hooks

## Rust-specific

- **Edition 2021**, stable toolchain. Do not use nightly features.
- Error types: `thiserror` in library crates, `anyhow` in the binary crate.
- Logging: use `tracing` macros (`info!`, `warn!`, `error!`), never `println!` or `eprintln!`.
- New public types must derive `Debug`. API-facing types must also derive `Serialize, Deserialize`.
- When adding a new crate, add it to `rust/Cargo.toml` workspace members and use `workspace.dependencies` for shared deps.
- Prefer `Uuid` for identifiers and `chrono::DateTime<Utc>` for timestamps — these are the project-wide conventions.
- Do not add `unsafe` code without an explicit safety comment and a compelling reason.

## TypeScript-specific

- **Node 20+**, **pnpm 9** workspace.
- All packages use `strict: true` in tsconfig.
- SDK methods must handle retries internally — callers should not need retry logic.
- Exported types should match the Rust API contracts exactly (field names, casing, optionality).
- Do not add runtime dependencies without discussion. The SDK intentionally has zero deps (uses native `fetch`).

## Testing expectations

- **Rust**: New logic in `spatiad-core` or `spatiad-api` should include unit tests in the same file or a `#[cfg(test)] mod tests` block.
- **TypeScript**: New SDK methods should have corresponding tests.
- Integration tests live in `spatiad-api` and cover HTTP + WebSocket flows end-to-end against the in-memory engine.
- Do not mock the core engine in integration tests — use a real `Engine` instance.

## What NOT to do

- Do not add new files unless the task requires it. Prefer editing existing files.
- Do not add documentation files (README, guides) unless explicitly asked.
- Do not introduce new external crates or npm packages without justification.
- Do not modify CI workflows without explicit instruction.
- Do not change configuration defaults — they were chosen deliberately.
- Do not add feature flags, conditional compilation, or backwards-compatibility shims for in-progress work.
- Do not create abstractions for one-time operations. Three similar lines are better than a premature helper.

## Pull request expectations

- PR title follows Conventional Commit format.
- PR description includes: what changed, why, and how to test it.
- CI must pass. Do not mark a PR ready if checks are failing.
- Keep PRs focused on a single concern. Split unrelated changes into separate PRs.

## Security considerations

- Never commit secrets, tokens, or credentials.
- Validate all external input at the API boundary (`spatiad-api/src/validation.rs`).
- Webhook signing uses HMAC-SHA256 — do not weaken or skip signature verification.
- Rate limiting exists for a reason — do not remove or bypass it without discussion.
- Use `reqwest` with `rustls-tls` — do not introduce OpenSSL dependencies.
