# Package-manager handoff

The release workflow builds and verifies the application assets. This
repository keeps package-manager templates, while the generated manifests
stay tied to the checksums of one concrete release.

After the explicit GitHub release publish pass has created its public assets,
download the release files and run:

```sh
bash packaging/release/prepare-packages.sh \
  --dist /path/to/woofer-release-assets \
  --version 0.4.0 \
  --output-dir /tmp/woofer-packages
```

The command refuses to run unless `checksums.txt` contains matching SHA-256
entries for the macOS universal DMG, the Linux x86_64 archive, and both
Windows installers. It writes:

- `homebrew/Casks/woofer.rb`, for the `kreatzzz/homebrew-tap` repository;
- `aur/woofer/PKGBUILD`, for the release AUR package;
- `aur/woofer-git/PKGBUILD`, for the rolling source AUR package; and
- `winget/manifests/k/kreatzzz/Woofer/0.4.0/`, for a winget-pkgs pull
  request.

The generated files contain real release hashes and are intentionally not
checked into this source repository. Do not replace a missing hash with a
placeholder: publish the release first, then regenerate the handoff.

Validate the AUR package with `makepkg --printsrcinfo` (and `namcap` when
available), the cask with `ruby -c`, and the winget directory with
`winget validate` on Windows or the winget-pkgs CI. The first macOS DMG may be
unsigned; keep the first-open note in the tap README until notarization is
configured.
