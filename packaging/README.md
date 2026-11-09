# Packaging

`cargo install --locked --path .` builds the GPUI application, including its `--help` and
`--version` CLI. Version tags continue to provide the Homebrew tap's source archives.
GPUI Kit enables runtime shaders, so packaging does not run a build-time Metal shader compiler.

Before publishing the next release, update the tap URL/checksum as usual and review its Linux
dependencies: pkgconf, fontconfig, freetype, libx11, libxcb, libxi, libxkbcommon, wayland,
and vulkan-loader. CI lists the Ubuntu development packages. The obsolete font license
installation must reference `assets/fonts/ComicMono-LICENSE.txt` rather than `OFL.txt`.
Do not update the live tap to unreleased source.

Linux packages should install `linux/difly.desktop` to `share/applications` and
`assets/icons/difly.svg` to `share/icons/hicolor/scalable/apps/difly.svg`. The application ID,
desktop file name and icon name all use `difly`. macOS registers the embedded icon at startup.
