# MPL-2.0 corresponding source availability

The following registry packages are included unmodified in the release dependency
graph. Their covered source remains licensed under the Mozilla Public License 2.0.
ThoughtsFlow does not restrict any rights granted to that source by MPL-2.0.

The exact `.crate` files below are gzip-compressed tar source archives, not compiled
binaries. They are shipped in this repository and the installed application resources
at `licenses/third-party/mpl-source/`. They can be opened with common archive tools
or `tar -xzf package-version.crate`; no network account or charge is required.

The SHA-256 values match Cargo.lock. All original source comments and notices are
preserved in the archives. The full license text is linked from each package entry
in [DEPENDENCIES.md](DEPENDENCIES.md).

| Package | Included source | SHA-256 | Upstream archive |
| --- | --- | --- | --- |
| cssparser 0.36.0 | [archive](mpl-source/cssparser-0.36.0.crate) | `dae61cf9c0abb83bd659dab65b7e4e38d8236824c85f0f804f173567bda257d2` | [registry source](https://static.crates.io/crates/cssparser/cssparser-0.36.0.crate) |
| cssparser-macros 0.6.1 | [archive](mpl-source/cssparser-macros-0.6.1.crate) | `13b588ba4ac1a99f7f2964d24b3d896ddc6bf847ee3855dbd4366f058cfcd331` | [registry source](https://static.crates.io/crates/cssparser-macros/cssparser-macros-0.6.1.crate) |
| dtoa-short 0.3.5 | [archive](mpl-source/dtoa-short-0.3.5.crate) | `cd1511a7b6a56299bd043a9c167a6d2bfb37bf84a6dfceaba651168adfb43c87` | [registry source](https://static.crates.io/crates/dtoa-short/dtoa-short-0.3.5.crate) |
| option-ext 0.2.0 | [archive](mpl-source/option-ext-0.2.0.crate) | `04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d` | [registry source](https://static.crates.io/crates/option-ext/option-ext-0.2.0.crate) |
| selectors 0.36.1 | [archive](mpl-source/selectors-0.36.1.crate) | `c5d9c0c92a92d33f08817311cf3f2c29a3538a8240e94a6a3c622ce652d7e00c` | [registry source](https://static.crates.io/crates/selectors/selectors-0.36.1.crate) |

Official license: https://www.mozilla.org/en-US/MPL/2.0/

The inventory is a conservative cross-platform superset, so some packages are used
only while building or on other targets. Source is still included to make the release
self-contained. Future modifications to these covered sources must retain their
notices and make the modified corresponding source available under MPL-2.0.
