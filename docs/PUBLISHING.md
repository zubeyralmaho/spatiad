# Publishing and No-Clone Distribution

This guide explains how users can install Spatiad artifacts without cloning this repository.

## 1) Publish TypeScript SDK to npm

Users can then install with npm, pnpm, or yarn.

### One-time setup

```bash
npm login
```

### Release flow

```bash
cd typescript
pnpm install --frozen-lockfile
pnpm --filter @spatiad/sdk build
pnpm --filter @spatiad/sdk test
pnpm --filter @spatiad/sdk publish --no-git-checks --access public
```

### GitHub Actions option

Use `.github/workflows/publish-sdk.yml` with `workflow_dispatch` and a release tag.

Required secret:

- `NPM_TOKEN`: npm automation token with publish permission for `@spatiad/sdk`

After publish, end users install with:

```bash
npm i @spatiad/sdk
# or
pnpm add @spatiad/sdk
```

## 2) Publish container image

Container users do not need source code.

Example (GHCR):

```bash
docker build -t ghcr.io/<owner>/spatiad:latest -f Dockerfile .
docker push ghcr.io/<owner>/spatiad:latest
```

Consumers run with:

```bash
docker run -p 3000:3000 ghcr.io/<owner>/spatiad:latest
```

## 3) Publish binary assets and support one-command install

Build and upload release archives to GitHub Releases with this naming pattern:

- spatiad-vX.Y.Z-linux-x86_64.tar.gz
- spatiad-vX.Y.Z-linux-aarch64.tar.gz
- spatiad-vX.Y.Z-darwin-x86_64.tar.gz
- spatiad-vX.Y.Z-darwin-aarch64.tar.gz

Each archive should contain:

- spatiad-bin

Users can install via:

```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/spatiad/main/dist/install-spatiad.sh | sh -s -- v0.1.0
```

Or with explicit install dir:

```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/spatiad/main/dist/install-spatiad.sh | sh -s -- v0.1.0 /usr/local/bin
```

## npm vs pnpm

- Publishing target is npm registry.
- End users may use npm, pnpm, or yarn.
- pnpm is a package manager choice, not a separate distribution channel.
