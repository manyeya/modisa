
Default to using Bun instead of Node.js.

- Use `bun <file>` instead of `node <file>` or `ts-node <file>`
- Use `bun test` instead of `jest` or `vitest`
- Use `bun build <file.html|file.ts|file.css>` instead of `webpack` or `esbuild`
- Use `bun install` instead of `npm install` or `yarn install` or `pnpm install`
- Use `bun run <script>` instead of `npm run <script>` or `yarn run <script>` or `pnpm run <script>`
- Use `bunx <package> <command>` instead of `npx <package> <command>`
- Bun automatically loads .env, so don't use dotenv.

## APIs

- `Bun.serve()` supports WebSockets, HTTPS, and routes. Don't use `express`.
- `bun:sqlite` for SQLite. Don't use `better-sqlite3`.
- `Bun.redis` for Redis. Don't use `ioredis`.
- `Bun.sql` for Postgres. Don't use `pg` or `postgres.js`.
- `WebSocket` is built-in. Don't use `ws`.
- Prefer `Bun.file` over `node:fs`'s readFile/writeFile
- Bun.$`ls` instead of execa.

## Frontends

There is no web frontend. The UI is the OpenTUI client in `src/client`, which draws the `view` the server pushes. The docs site is a dependency-free static generator (`bun site/build.ts`, content in `site/content`); keep it that way, no Vite, React or HTML imports. Bun's API docs are in `node_modules/bun-types/docs/**.mdx`.

## Project structure

Code lives in feature folders under `src/` (`cli`, `core`, `protocol`, `config`, `server`, `client`, `platform`, `integrations`, `skills`, `plugins`); README's "Layout" section lists what goes where. Types shared by client and server go in `src/protocol/`: TypeScript types in `types.ts`, the wire contract as zod schemas in `schema.ts` (the server validates against it and `protocol.describe` publishes it). Change the two together, and never put them in a server or client file. Server features are modules that take the `ServerContext` (`src/server/context.ts`); client features are functions that take the `App` (`src/client/context.ts`).

Tests live in `test/`, not next to the code: `test/unit` for pure logic, `test/e2e` (and `test/e2e/ui`) for one feature per file, each in its own `sandbox()` session, `test/support` for shared helpers, and `test/fixtures` for captured screens. Put a helper in `test/support` instead of copying it between files.

Releases come from `.github/workflows/release.yml` on every push to `main` (versions from git tags, `[minor]`/`[major]` in a commit message to bump more); `staging` publishes a rolling prerelease. The binary's version is baked in with `--define BUILD_VERSION`; from source it's `<package.json version>-dev` (`src/core/version.ts`). The docs site is `site/` (`bun site/build.ts`).
