# Attribution and distribution notices

## Original project

This is an independent, all-Rust Windows port of **PopChat**, originally created
by **Le Chen / lec77** for macOS using Swift, SwiftUI and AppKit:

**https://github.com/lec77/PopChat**

The original project is licensed under the MIT License, copyright
**2026 Le Chen**. Its copyright and MIT permission notice are retained in
[`LICENSE`](LICENSE). Windows port and modifications: **Evis Lai**, 2026.
This port is not an official release of the original macOS project, and does
not imply endorsement by its author.

## Third-party assets

- **Font Awesome Free 6.7.2** — official Solid icons, pinned upstream source and
  hashes under `assets/font-awesome`; distributed notices under
  `licenses/font-awesome`. Icons are CC BY 4.0; see the supplied notices for
  the upstream font/code licenses.
- **Swei Gothic / 獅尾圓體 TC Regular 2.143** — embedded Chinese default, under
  SIL Open Font License 1.1. Original copyright, license and provenance are in
  `assets/fonts/swei-gothic` and `licenses/swei-gothic` in binary releases.
- **TT Norms Pro and Arial are NOT bundled or redistributed.** The application
  may use an existing, locally installed copy. Their ownership and licensing
  remain with their respective rights holders.
- Rust dependencies and other embedded fonts have their own licenses; see
  `THIRD_PARTY_NOTICES.md` and `licenses/dependencies` in the release archive.
- The Windows Setup/uninstaller uses unmodified **NSIS 3.11** components.
  Its upstream copyright and license terms (including the LZMA linking
  exception) are installed at `licenses/nsis/COPYING`. NSIS source is available
  at https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/ .

The application's MIT license does not replace third-party licenses.
Binaries are not code-signed. A SHA-256 digest verifies file
integrity against the GitHub release metadata; it is not a publisher signature.
