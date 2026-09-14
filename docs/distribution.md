# Distribution — TODO

Goal: shepherd is easy to install, easy to upgrade, easy to remove, and anyone can check that a binary came from this repository.

Shipped on `dev` in `abe2680` (not released yet). Research behind these choices: [install channels](https://claude.ai/code/artifact/0a18e097-4456-4dc8-b54b-c7eeb0c2e535).

## Done (on dev, waiting for a release)

- [x] `shepherd uninstall [--purge] [--yes]`: integrations, the shared skill, sessions, state, and config with `--purge`; deletes the binary only when install.sh put it there
- [x] `install.sh --uninstall` hands off to `shepherd uninstall`
- [x] Hooks call the `shepherd` on `PATH` from a release binary, so per-version install directories (Homebrew, mise) don't break them on upgrade
- [x] `shepherd update` and the update badge print the Homebrew, mise or apt/dnf command instead of replacing a binary a package manager owns (`src/core/install.ts`)
- [x] Uninstall leaves no empty `"hooks": {}` in an agent's settings
- [x] Release workflow: .deb and .rpm packages via nfpm (`.github/nfpm.yaml`)
- [x] Release workflow: signed build provenance for every file (`actions/attest`)
- [x] Release workflow: Homebrew formula generated (`.github/scripts/homebrew-formula.ts`) and pushed to the tap when `HOMEBREW_TAP_TOKEN` is set
- [x] README and site: mise, packages, verifying a download, uninstalling
- [x] Verified locally: formula installs, passes `brew test` and uninstalls from a local tap; mise picks the right asset, runs and uninstalls; .deb and .rpm install, run and remove on Debian and Fedora

## Release it

- [ ] Wait for CI on `dev` (run 34845964852)
- [ ] Fast-forward `staging`, check the prerelease has the .deb/.rpm files and attestations
- [ ] Fast-forward `main` to publish the first release with packages and provenance
- [ ] Run `gh attestation verify shepherd-darwin-arm64 -R manyeya/shepherd` against the published file
- [ ] Install from the release with mise and one package, as a real user would

## Homebrew tap (needs the maintainer)

- [ ] Create the public repo `manyeya/homebrew-tap`
- [ ] Create a fine-grained token with Contents write on that repo only, and add it as the `HOMEBREW_TAP_TOKEN` secret on `manyeya/shepherd`
- [ ] Release to `main`, then confirm `Formula/shepherd.rb` landed in the tap
- [ ] `brew install manyeya/tap/shepherd` on a clean machine, then `shepherd uninstall && brew uninstall shepherd`
- [ ] Add Homebrew to the README install table and the site's install page (left out until the tap is live)

## Next channels (need accounts)

- [ ] npm: publish `@manyeya/shepherd` (`shepherd` is taken) with per-platform packages in `optionalDependencies` (`os`/`cpu` fields), using trusted publishing from GitHub Actions (npm CLI 11.5.1+, provenance generated automatically)
- [ ] npm: add `node_modules` to `installedBy` so updates defer to `npm update -g @manyeya/shepherd`
- [ ] apt/dnf repositories on Cloudsmith's free open-source plan, so `apt upgrade` and `dnf upgrade` pick up new versions
- [ ] AUR `shepherd-bin` PKGBUILD from the release binaries, regenerated each release with `updpkgsums`

## Later

- [ ] Apple Developer ID signing and notarization ($99/year), once there's a browser download: hardened runtime with `allow-jit`, `allow-unsigned-executable-memory` and `disable-library-validation` (libghostty's dylib)
- [ ] `flake.nix` with the prebuilt binary and `autoPatchelfHook` on Linux; add `/nix/store/` to `installedBy`
- [ ] homebrew-core once the repo clears the notability bar (225 stars when the author submits it): needs a build from source and self-update switched off
- [ ] install.sh verifies the attestation when `gh` is available
- [ ] aqua registry entry for `aqua:` users

## Not planned

- Snap: classic confinement needs a manual vetting request, and a multiplexer can't run sandboxed
- Flatpak: made for sandboxed desktop apps
- winget, Scoop: no Windows build (libghostty has none)
- Intel Mac and musl builds: libghostty ships no prebuilds for them

## Known issues found along the way

- [ ] `test/e2e/sessions.test.ts` leaves a `sess` server behind, and its `pgrep` then kills the wrong server on the next local run; target its own server by socket instead
- [ ] `test/e2e/reporting.test.ts` times out now and then under the full suite (`wait --timeout 5`); passes alone
- [ ] Programs that query a pane's background colour (OSC 11) get the server's default, not the theme; the server's terminal needs the theme colours too
- [ ] `stableSelf()` trusts that whatever is called `shepherd` on `PATH` is this program
- [ ] No regression test yet for the settings hover fix (pointer resting on the list while pressing arrow keys)
