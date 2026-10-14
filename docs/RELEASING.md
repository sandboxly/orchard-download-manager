# Releasing Orchard Download Manager

Windows and Linux packages come from CI (`.github/workflows/ci.yml`); the
macOS dmg is built, signed and notarized locally.

## Every push and pull request

- **Tests** (ubuntu-22.04, windows-latest, macos-latest): clippy with
  `-D warnings`, `cargo test`, a release build and its `--self-test`.
- **Package** (windows-latest, ubuntu-22.04): the app folder
  (`packaging/stage.sh`: the executable plus `licenses/`), its `--self-test`,
  then
  - Windows: `Orchard-Download-Manager-Setup.exe` (Inno Setup,
    `packaging/windows/orchard-download-manager.iss`; keep its AppId) and
    `Orchard-Download-Manager-Windows.zip`, tested with a silent install,
    self-test and uninstall.
  - Linux: `Orchard-Download-Manager-Linux.tar.gz`,
    `Orchard-Download-Manager-Linux-x86_64.{deb,rpm}` (nfpm,
    `packaging/linux/nfpm.yaml`) and
    `Orchard-Download-Manager-Linux-x86_64.AppImage`, tested with apt (the runner
    and clean Ubuntu/Debian containers), dnf (Fedora container) and by
    running the AppImage and the unpacked tarball.
  - The files are the workflow artifacts `packages-Windows` and
    `packages-Linux`.

## Licenses

`licenses/` ships in every package (`licenses\` in the Windows install
folder, `/opt/orchard-download-manager/licenses` and
`/usr/share/doc/orchard-download-manager` on Linux): our `LICENSE` and
`THIRD-PARTY-RUST.txt`, generated from
`Cargo.lock` by `scripts/third-party-licenses.sh` (needs cargo-about). Rerun
it after changing dependencies or the version and commit the result; CI
warns when it is stale and regenerates it for the packages. The macOS script
gets the same folder with `packaging/stage.sh licenses - <dir>`.

## Making a release

1. Bump `version` in `Cargo.toml`, run `cargo check` (so `Cargo.lock`
   follows) and `scripts/third-party-licenses.sh`; commit and push to main
   and wait for a green CI run.
2. Tag and push: `git tag v1.2.3 && git push origin v1.2.3`. CI checks that
   the tag matches `Cargo.toml`, creates the draft release "Orchard Download
   Manager 1.2.3" if it doesn't exist and uploads the Windows and Linux
   packages to it (`gh release upload --clobber`, so a rerun replaces them).
   CI never publishes.
3. Build `Orchard-Download-Manager-macOS.dmg` locally from the tag with
   `packaging/macos/package.sh` and upload it to the draft:
   `gh release upload v1.2.3 dist/Orchard-Download-Manager-macOS.dmg`.
4. Write the release notes and publish the release on GitHub.
5. Update the Homebrew cask in `sandboxly/homebrew-tap` (version and the
   dmg's SHA-256).
