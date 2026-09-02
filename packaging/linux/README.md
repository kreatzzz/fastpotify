# Linux release formats

Woofer publishes two Linux formats for each supported architecture:

- The portable `tar.gz` contains the `woofer` executable, README, license, and
  desktop/icon assets. It is the fallback for distributions that do not run
  AppImage or that prefer their own package manager.
- The AppImage contains an AppDir and the non-glibc shared libraries discovered
  by `linuxdeploy`. It can be run without installation and is built on native
  x86_64 and arm64 runners. glibc remains a host requirement, as it does for
  every native Linux build; release runners provide the compatibility baseline.

There is deliberately no `.deb` or `.rpm` artifact yet. Those formats need
maintained maintainer scripts, dependency policy, signing keys, and a package
repository. The AUR package is Arch-specific and is maintained separately; it
is not a Linux-wide distribution channel.
