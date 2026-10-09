modisa is a Rust binary: `cargo build`, `cargo test`, `cargo run -- <args>`. Bun is only for the docs site (`bun site/build.ts`), plugins (TypeScript, run by bun) and running the end-to-end suite (`parity/run.sh`).

## Runtime

Everything runs on one thread: a current-thread tokio runtime and a `LocalSet` (`src/main.rs`). Server state is `Rc<RefCell<Server>>` (`src/server/mod.rs`), client state `Rc<RefCell<App>>` (`src/client/mod.rs`). Borrow them only between awaits, never across one; work that could re-enter a borrow (a dialog opened from a key handler, a tick started from a request) goes in a task with `spawn_local`. Ask the kernel for process facts (`src/platform/procs.rs`) instead of spawning `ps` or `lsof`: agent detection asks every half second.

## Frontends

There is no web frontend. The UI is the ratatui client in `src/client`, drawn whole every frame from the `App` and the `view` the server pushes; a frame records what it put where (`app.hits`) for the mouse. Pane screens are `alacritty_terminal` terminals wrapped in `src/vt.rs`. The docs site is a dependency-free static generator (`bun site/build.ts`, content in `site/content`); keep it that way, no Vite, React or HTML imports. What it shows from the code (agents, every setting, integrations, the plugin directory) it gets from the binary: `modisa __site-data`.

## Project structure

Code lives in feature modules under `src/` (`cli`, `core`, `protocol`, `config`, `server`, `client`, `vt`, `platform`, `integrations`, `skills`, `plugins`); README's "Layout" section lists what goes where. Types shared by client and server go in `src/protocol/`: the types in `types.rs`, the wire contract in `schema.rs` and `describe.json` (what `protocol.describe` publishes). Change them together, and never put them in a server or client file. Request validation is hand-written and keeps modisa's own error messages.

This is a port of a TypeScript build, whose last commit is `b83399d53c96`. Comments that mention "the original", "the TypeScript" or a `.ts` file mean that build; read it with `git show b83399d53c96:<path>`.

Unit tests sit next to the code (`#[cfg(test)] mod tests`); captured agent screens are in `tests/fixtures`. The end-to-end suite is the TypeScript build's, run against this binary by `parity/run.sh`, which takes it out of git history (its last commit) and needs bun. Tests never touch the real `~/.local/state/modisa` or `~/.config/modisa`: give them scratch `MODISA_DIR` and `MODISA_CONFIG_DIR`.

Releases come from `.github/workflows/release.yml` on every push to `main` (versions from git tags, `[minor]`/`[major]` in a commit message to bump more); `staging` publishes a rolling prerelease. The binary's version is baked in from `MODISA_BUILD_VERSION` at build time; from source it's `<Cargo.toml version>-dev` (`src/core/version.rs`).

`src/config/agents/marks.ttf` (the agents' logo font) is a committed build artifact. Its generator was `.github/scripts/agent-logos.ts` in the TypeScript build (`git show b83399d53c96:.github/scripts/agent-logos.ts`); port it when a logo needs adding.
