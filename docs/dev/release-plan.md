# Release and packaging runbook

State: **halted at the user's request** (Aug 29, 2026). The `v0.3.0` tag
was pushed, triggered nothing (fork Actions quirk), and was deleted; zero
releases exist. Version `0.3.0` is already set in `Cargo.toml`. When the
user says go, run this top to bottom.

## 0. Cut the release

```bash
git tag v0.3.0 && git push origin v0.3.0
```

Then **verify a run appeared** (`gh run list --repo kreatzzz/woofer`): the
release workflow has no `workflow_dispatch`, only the `v*` tag trigger, and
once the tag push silently produced zero runs. If nothing fires, delete and
re-push the tag, or trigger from the Actions tab. The build takes
~15 minutes: Linux x64 + arm64 tarballs, Windows x64 + arm64 (zip + Inno
Setup `woofer-v…-setup.exe`), macOS universal DMG (unsigned — no Apple
secrets; users right-click → Open), `checksums.txt`. All land at
`github.com/kreatzzz/woofer/releases`.

## 1. Homebrew tap (~10 min, fully scriptable)

```bash
gh repo create kreatzzz/homebrew-tap --public --clone=false
git clone https://github.com/kreatzzz/homebrew-tap /tmp/tap && cd /tmp/tap
mkdir -p Casks
```

`Casks/woofer.rb`:

```ruby
cask "woofer" do
  version "0.3.0"
  sha256 "SHA256_OF_THE_DMG"   # from the release's checksums.txt

  url "https://github.com/kreatzzz/woofer/releases/download/v#{version}/woofer-v#{version}-macos-universal.dmg"
  name "Woofer"
  desc "Fast, lightweight, native Spotify client"
  homepage "https://github.com/kreatzzz/woofer"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Woofer.app"
end
```

Commit, push. Users: `brew install kreatzzz/tap/woofer`. Every future
release: bump `version` + `sha256` in one tap commit. Unsigned-DMG note
belongs in the README: right-click → Open, or
`xattr -cr /Applications/Woofer.app`.

## 2. AUR (~20 min, needs the user's aur.archlinux.org account + SSH key)

Two packages, each pushed to `aur@aur.archlinux.org:<pkg>.git`:

**`woofer`** (release tarball, x86_64):

```sh
pkgname=woofer
pkgver=0.3.0
pkgrel=1
pkgdesc="Fast, lightweight, native Spotify client built with Rust and egui"
arch=('x86_64')
url="https://github.com/kreatzzz/woofer"
license=('MIT')
depends=('alsa-lib' 'libpulse' 'wayland' 'libxkbcommon')
conflicts=('woofer-git')
source=("$pkgname-$pkgver.tar.gz::$url/releases/download/v$pkgver/woofer-v$pkgver-x86_64-unknown-linux-gnu.tar.gz")
sha256sums=('FILL_FROM_checksums.txt')
package() {
  cd "$srcdir/$pkgname-v$pkgver-x86_64-unknown-linux-gnu"
  install -Dm755 woofer "$pkgdir/usr/bin/woofer"
  install -Dm644 packaging/applications/woofer.desktop \
    "$pkgdir/usr/share/applications/woofer.desktop"
  install -Dm644 packaging/icons/woofer.svg \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/woofer.svg"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/woofer/LICENSE"
}
```

Validate with `makepkg -f` (or `namcap`), then
`git init && git add . && git commit && git push aur:woofer.git`. Test with
`yay -S woofer` in a clean chroot if possible.

**`woofer-git`**: same shape, `makedepends=('cargo' 'git')`,
`source=("$pkgname::git+$url.git")`, `pkgver()` from `git describe`,
build with `cargo build --release --locked`, same `package()` installs.

## 3. winget (Windows; the bureaucratic one)

1. Fork `microsoft/winget-pkgs` under `kreatzzz`, shallow-clone it.
2. Add `manifests/k/kreatzzz/Woofer/0.3.0/` with three YAML files:
   - `kreatzzz.Woofer.yaml` — `PackageIdentifier: kreatzzz.Woofer`,
     `PackageVersion: 0.3.0`, `PackageLocale`, `Publisher: kreatzzz`,
     `PackageName: Woofer`, `License: MIT`,
     `ShortDescription`, `Moniker: woofer`.
   - `kreatzzz.Woofer.installer.yaml` — `InstallerType: inno`, both
     architectures pointing at the GitHub `…-setup.exe` URLs, each with
     its sha256 from `checksums.txt`, and
     `InstallerSwitches: { Silent: /VERYSILENT /SUPPRESSMSGBOXES /NORESTART,
     SilentWithProgress: /SILENT }`, `AppsAndFeaturesEntries` with
     `ProductCode` if the .iss declares one.
   - `kreatzzz.Woofer.locale.en-US.yaml` — description, tags, homepage.
3. Validate locally with `winget validate` (on Windows) or the
   `winget-pkgs` CI, then open the PR with their template. The bot
   verifies the URLs and hashes; a human reviews (1-3 days). First-time
   submitters need a seasoned GitHub account — having the tap and AUR
   package public first helps the review.

Future versions: a new `0.3.1/` folder per release.

## After the first release

- Bump the tap (version + sha256) — one commit.
- `docs/_guide/download.md` currently describes upstream's channels;
  update it when Homebrew/AUR/winget are live.
- The update checker (`src/updates.rs`) watches
  `kreatzzz/woofer/releases/latest` — it starts working from the first
  real release.
