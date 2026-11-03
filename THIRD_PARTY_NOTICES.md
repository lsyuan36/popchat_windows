# Third-party notices

PopChat Windows uses the third-party software, fonts and icons identified
below. Their licenses apply to those components; the application's MIT
license does not replace them. Copyright and permission notices, full
license texts, and upstream NOTICE files are distributed in
`licenses/dependencies/` in the portable release. In the source repository,
the same files are under `assets/licenses/dependencies/`.

`licenses/dependencies/index.json` maps every inventoried component/version
to its individual notice files and their provenance. The inventory is a
conservative superset of the locked normal/build dependency closure for
`x86_64-pc-windows-msvc`, not a claim that every listed build tool or
feature-unified dependency is linked into the executables. Root dev-only
edges and dependencies resolved exclusively for other platforms are not
intentionally included. No third-party crate source or font binaries are
copied into the license directory.

For crates offering alternative licenses, the upstream alternatives and
their supplied texts are retained rather than replacing component-specific
copyright notices with a generic MIT notice. Conjunctive and asset-specific
terms must also be observed. This is a best-effort notice inventory, not a
legal certification or an endorsement by the original authors.

## Source availability

Every Rust dependency listed below has a version-specific source download
link. These public crates.io archives contain the published source and
manifests used by Cargo; `Cargo.lock` records their checksums. The JSON
inventory also records repository links and, where available, the exact
upstream commit from the crate's `.cargo_vcs_info.json`. No application
profiles, credentials, installed fonts, or logs are included.

### option-ext 0.2.0 — Mozilla Public License 2.0

This unmodified dependency is distributed under **MPL-2.0**. Its complete
license is `licenses/dependencies/option-ext-0.2.0/LICENSE.txt`. Its exact
corresponding Source Code Form, available without charge under MPL-2.0:

- <https://crates.io/api/v1/crates/option-ext/0.2.0/download>
- <https://github.com/soc/option-ext/tree/272f22fc9ea1ac6b08f01704af52c4ac338df4e2>
- <https://docs.rs/crate/option-ext/0.2.0/source/>

See the accompanying `SOURCE.md`. The application has not modified these
covered source files. This notice does not claim that MPL requires the
unrelated application as a whole to be licensed under MPL or GPL.

### latex-rust 2.1.1 — embedded STIX and LaTeX-derived data

The Rust implementation is MIT OR Apache-2.0, Copyright 2026 Jeffrey S Carr.
Additional embedded materials have their own terms:

- **STIX Two Math 2.13**: SIL Open Font License 1.1. Copyright 2001–2021
  The STIX Fonts Project Authors; portions copyright 1998–2003 MicroPress,
  Inc. (Reserved Font Name TM Math), 1990 Elsevier, Inc., and 2010, 2012,
  2014 Adobe Systems Incorporated. STIX Fonts is a trademark of The
  Institute of Electrical and Electronics Engineers, Inc. The complete
  notices and OFL are in
  `licenses/dependencies/latex-rust-2.1.1/fonts/stix-two-math/OFL.txt`.
- **dvips color-name table**: LPPL 1.3c or later. Copyright 1994 David
  Carlisle and Sebastian Rahtz; 1995–1999 David Carlisle; 2000–2026 The
  LaTeX Project. The dependency's author converted the LaTeX graphics
  bundle's `dvipsnam.def` definitions to `data/dvipsnames.tsv`, reordered
  them alphabetically, and retained the upstream CMYK values. The original
  authors and The LaTeX Project do not maintain or support that modified
  file. Full license, modification notice and original upstream NOTICE:
  `licenses/dependencies/latex-rust-2.1.1/LPPL-1.3c.txt`,
  `LPPL-NOTICE.txt`, and `NOTICE`.

The exact dependency source (including the modified table and its header):
<https://crates.io/api/v1/crates/latex-rust/2.1.1/download> and
<https://github.com/jscarr64/LaTeX-Rust/tree/e377ecb4af876b8b75e62b8b83e8a9535499944c>.
The complete unmodified original graphics bundle is obtainable at
<https://github.com/latex3/latex2e/tree/develop/required/graphics> or
<https://ctan.org/pkg/graphics> (CTAN: `macros/latex/required/graphics`).
PopChat uses the published dependency without modifications.

## Embedded fonts and icons

### egui / epaint_default_fonts 0.32.3

The default-font feature embeds fonts with licenses separate from the Rust
code's MIT/Apache alternatives. Full texts and extracted font-specific
copyright notices are in
`licenses/dependencies/epaint_default_fonts-0.32.3/fonts/`:

| Font | Attribution and license | Full notice |
| --- | --- | --- |
| Ubuntu Light 0.83 | Copyright 2011 Canonical Ltd.; Ubuntu Font Licence 1.0 | `UFL.txt`, `COPYRIGHT.txt` |
| Noto Emoji 1.05 uh | Copyright 2013 Google Inc. All Rights Reserved.; OFL-1.1 | `OFL.txt`, `COPYRIGHT.txt` |
| Hack 3.003 | Copyright 2018 Source Foundry Authors; MIT; Bitstream Vera portions Copyright 2003 Bitstream, Inc.; Bitstream Vera License; DejaVu contributions public domain | `Hack-Regular.txt`, `COPYRIGHT.txt` |
| emoji-icon-font 1.1 | Copyright 2014 John Slegers; MIT | `emoji-icon-font-mit-license.txt` |

Exact upstream font package:
<https://crates.io/api/v1/crates/epaint_default_fonts/0.32.3/download>.
The Hack upstream reserved names are “Bitstream” and “Vera”. These fonts
are embedded in the executable; they are not separately duplicated in the
portable archive.

### Swei Gothic

The embedded, unmodified Swei Gothic CJK Traditional Chinese Regular font
is licensed under SIL OFL 1.1. Copyright (c) 2020 Chun yu Yao; its embedded
notice identifies the Reserved Font Name `Untitled6`. Swei Gothic by Max
(max32002) is based on Source Han Sans developed by Adobe and Google.
See `licenses/swei-gothic/SIL_Open_Font_License_1.1.txt`, `COPYRIGHT.txt`,
and `SOURCE.md` for the full license, original metadata and exact source.
The original font retains its embedded copyright/license metadata.

### Font Awesome Free 6.7.2

The 14 embedded Free Solid SVG icons are by Fonticons, Inc., Copyright
2024 Fonticons, Inc., <https://fontawesome.com>, under **CC BY 4.0**:
<https://creativecommons.org/licenses/by/4.0/>. Source:
<https://github.com/FortAwesome/Font-Awesome/tree/af620534bfc3c2d4cbefcfeec29603bbe7809e64/svgs/solid>.
Original SVG files retain their attribution comments. For display, PopChat
adds a white root fill in memory, applies palette tint and scaling, and
rasterizes the icons; it does not redraw their paths. No endorsement is
implied. See `licenses/font-awesome/NOTICE.md`, `README.md`, and the
original `LICENSE.txt`. The full CC BY 4.0 legal text is also included at
`licenses/dependencies/font-awesome-free-6.7.2/CC-BY-4.0.txt`.
Font Awesome font files and executable code are not distributed.

### Fonts not distributed

TT Norms, Arial, other installed/proprietary fonts, font-database test
fonts, and user-selected font files are **not** bundled. Runtime access
to a font already installed by the user does not make it part of this
distribution. The application and updater are unsigned artifacts;
these notices do not imply code signing or publisher endorsement.

## Upstream notice limitations

`hexf-parse 0.2.1` declares CC0-1.0 in its published manifest, and
`simd_helpers 0.1.0` declares MIT. Neither crate nor its pinned upstream
commit contains a standalone license file or copyright notice. Their
directories preserve the exact author/license declarations and supply the
standard declared-license text, clearly distinguished from upstream files.
No missing copyright year or holder has been invented. The missing upstream
notices remain a documented limitation requiring publisher review, rather
than an assertion that new permission has been obtained.

## Locked Rust dependency inventory

Full package-specific license, copyright and NOTICE files are in each
component's subdirectory of `licenses/dependencies/`; consult `index.json`
for the exact file list and original download locations. “Declared license”
is the published Cargo metadata, not a replacement for additional notices
such as the embedded-font and LPPL terms above.

| Package | Declared license | Exact published source |
| --- | --- | --- |
| [ab_glyph 0.2.32](licenses/dependencies/ab_glyph-0.2.32/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/ab_glyph/0.2.32/download) |
| [ab_glyph_rasterizer 0.1.10](licenses/dependencies/ab_glyph_rasterizer-0.1.10/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/ab_glyph_rasterizer/0.1.10/download) |
| [accesskit 0.19.0](licenses/dependencies/accesskit-0.19.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/accesskit/0.19.0/download) |
| [accesskit_consumer 0.28.0](licenses/dependencies/accesskit_consumer-0.28.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/accesskit_consumer/0.28.0/download) |
| [accesskit_windows 0.27.0](licenses/dependencies/accesskit_windows-0.27.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/accesskit_windows/0.27.0/download) |
| [accesskit_winit 0.27.0](licenses/dependencies/accesskit_winit-0.27.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/accesskit_winit/0.27.0/download) |
| [adler2 2.0.1](licenses/dependencies/adler2-2.0.1/) | 0BSD OR MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/adler2/2.0.1/download) |
| [aes 0.8.4](licenses/dependencies/aes-0.8.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/aes/0.8.4/download) |
| [ahash 0.8.12](licenses/dependencies/ahash-0.8.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ahash/0.8.12/download) |
| [aho-corasick 1.1.5](licenses/dependencies/aho-corasick-1.1.5/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/aho-corasick/1.1.5/download) |
| [aligned 0.4.3](licenses/dependencies/aligned-0.4.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/aligned/0.4.3/download) |
| [aligned-vec 0.6.4](licenses/dependencies/aligned-vec-0.6.4/) | MIT | [crate](https://crates.io/api/v1/crates/aligned-vec/0.6.4/download) |
| [anyhow 1.0.104](licenses/dependencies/anyhow-1.0.104/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/anyhow/1.0.104/download) |
| [arboard 3.7.0](licenses/dependencies/arboard-3.7.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/arboard/3.7.0/download) |
| [arg_enum_proc_macro 0.3.4](licenses/dependencies/arg_enum_proc_macro-0.3.4/) | MIT | [crate](https://crates.io/api/v1/crates/arg_enum_proc_macro/0.3.4/download) |
| [arrayref 0.3.9](licenses/dependencies/arrayref-0.3.9/) | BSD-2-Clause | [crate](https://crates.io/api/v1/crates/arrayref/0.3.9/download) |
| [arrayvec 0.7.8](licenses/dependencies/arrayvec-0.7.8/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/arrayvec/0.7.8/download) |
| [ash 0.38.0+1.3.281](licenses/dependencies/ash-0.38.0+1.3.281/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ash/0.38.0+1.3.281/download) |
| [as-slice 0.2.1](licenses/dependencies/as-slice-0.2.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/as-slice/0.2.1/download) |
| [async-trait 0.1.92](licenses/dependencies/async-trait-0.1.92/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/async-trait/0.1.92/download) |
| [atomic-waker 1.1.2](licenses/dependencies/atomic-waker-1.1.2/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/atomic-waker/1.1.2/download) |
| [autocfg 1.5.1](licenses/dependencies/autocfg-1.5.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/autocfg/1.5.1/download) |
| [av1-grain 0.2.5](licenses/dependencies/av1-grain-0.2.5/) | BSD-2-Clause | [crate](https://crates.io/api/v1/crates/av1-grain/0.2.5/download) |
| [avif-serialize 0.8.9](licenses/dependencies/avif-serialize-0.8.9/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/avif-serialize/0.8.9/download) |
| [av-scenechange 0.14.1](licenses/dependencies/av-scenechange-0.14.1/) | MIT | [crate](https://crates.io/api/v1/crates/av-scenechange/0.14.1/download) |
| [axum 0.7.9](licenses/dependencies/axum-0.7.9/) | MIT | [crate](https://crates.io/api/v1/crates/axum/0.7.9/download) |
| [axum-core 0.4.5](licenses/dependencies/axum-core-0.4.5/) | MIT | [crate](https://crates.io/api/v1/crates/axum-core/0.4.5/download) |
| [base64 0.22.1](licenses/dependencies/base64-0.22.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/base64/0.22.1/download) |
| [base64 0.23.1](licenses/dependencies/base64-0.23.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/base64/0.23.1/download) |
| [bit_field 0.10.3](licenses/dependencies/bit_field-0.10.3/) | Apache-2.0/MIT | [crate](https://crates.io/api/v1/crates/bit_field/0.10.3/download) |
| [bitflags 1.3.2](licenses/dependencies/bitflags-1.3.2/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/bitflags/1.3.2/download) |
| [bitflags 2.13.2](licenses/dependencies/bitflags-2.13.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/bitflags/2.13.2/download) |
| [bit-set 0.8.0](licenses/dependencies/bit-set-0.8.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/bit-set/0.8.0/download) |
| [bitstream-io 4.10.0](licenses/dependencies/bitstream-io-4.10.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/bitstream-io/4.10.0/download) |
| [bit-vec 0.8.0](licenses/dependencies/bit-vec-0.8.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/bit-vec/0.8.0/download) |
| [block-buffer 0.10.4](licenses/dependencies/block-buffer-0.10.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/block-buffer/0.10.4/download) |
| [block-padding 0.3.3](licenses/dependencies/block-padding-0.3.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/block-padding/0.3.3/download) |
| [built 0.8.1](licenses/dependencies/built-0.8.1/) | MIT | [crate](https://crates.io/api/v1/crates/built/0.8.1/download) |
| [bumpalo 3.20.3](licenses/dependencies/bumpalo-3.20.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/bumpalo/3.20.3/download) |
| [bytecount 0.6.9](licenses/dependencies/bytecount-0.6.9/) | Apache-2.0/MIT | [crate](https://crates.io/api/v1/crates/bytecount/0.6.9/download) |
| [bytemuck 1.25.2](licenses/dependencies/bytemuck-1.25.2/) | Zlib OR Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/bytemuck/1.25.2/download) |
| [bytemuck_derive 1.12.1](licenses/dependencies/bytemuck_derive-1.12.1/) | Zlib OR Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/bytemuck_derive/1.12.1/download) |
| [byteorder 1.5.0](licenses/dependencies/byteorder-1.5.0/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/byteorder/1.5.0/download) |
| [byteorder-lite 0.1.0](licenses/dependencies/byteorder-lite-0.1.0/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/byteorder-lite/0.1.0/download) |
| [bytes 1.12.1](licenses/dependencies/bytes-1.12.1/) | MIT | [crate](https://crates.io/api/v1/crates/bytes/1.12.1/download) |
| [calamine 0.26.1](licenses/dependencies/calamine-0.26.1/) | MIT | [crate](https://crates.io/api/v1/crates/calamine/0.26.1/download) |
| [cbc 0.1.2](licenses/dependencies/cbc-0.1.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cbc/0.1.2/download) |
| [cc 1.9.0](licenses/dependencies/cc-1.9.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cc/1.9.0/download) |
| [cfb 0.10.0](licenses/dependencies/cfb-0.10.0/) | MIT | [crate](https://crates.io/api/v1/crates/cfb/0.10.0/download) |
| [cfb 0.7.3](licenses/dependencies/cfb-0.7.3/) | MIT | [crate](https://crates.io/api/v1/crates/cfb/0.7.3/download) |
| [cfg_aliases 0.2.2](licenses/dependencies/cfg_aliases-0.2.2/) | MIT | [crate](https://crates.io/api/v1/crates/cfg_aliases/0.2.2/download) |
| [cfg-if 1.0.5](licenses/dependencies/cfg-if-1.0.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cfg-if/1.0.5/download) |
| [chacha20 0.10.2](licenses/dependencies/chacha20-0.10.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/chacha20/0.10.2/download) |
| [chrono 0.4.45](licenses/dependencies/chrono-0.4.45/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/chrono/0.4.45/download) |
| [cipher 0.4.4](licenses/dependencies/cipher-0.4.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cipher/0.4.4/download) |
| [clipboard-win 5.4.1](licenses/dependencies/clipboard-win-5.4.1/) | BSL-1.0 | [crate](https://crates.io/api/v1/crates/clipboard-win/5.4.1/download) |
| [codepage 0.1.3](licenses/dependencies/codepage-0.1.3/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/codepage/0.1.3/download) |
| [codespan-reporting 0.12.0](licenses/dependencies/codespan-reporting-0.12.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/codespan-reporting/0.12.0/download) |
| [color_quant 1.1.0](licenses/dependencies/color_quant-1.1.0/) | MIT | [crate](https://crates.io/api/v1/crates/color_quant/1.1.0/download) |
| [console 0.15.11](licenses/dependencies/console-0.15.11/) | MIT | [crate](https://crates.io/api/v1/crates/console/0.15.11/download) |
| [core_detect 1.0.0](licenses/dependencies/core_detect-1.0.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/core_detect/1.0.0/download) |
| [cpufeatures 0.2.17](licenses/dependencies/cpufeatures-0.2.17/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cpufeatures/0.2.17/download) |
| [cpufeatures 0.3.1](licenses/dependencies/cpufeatures-0.3.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/cpufeatures/0.3.1/download) |
| [crc32fast 1.5.3](licenses/dependencies/crc32fast-1.5.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crc32fast/1.5.3/download) |
| [crossbeam-channel 0.5.17](licenses/dependencies/crossbeam-channel-0.5.17/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crossbeam-channel/0.5.17/download) |
| [crossbeam-deque 0.8.8](licenses/dependencies/crossbeam-deque-0.8.8/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crossbeam-deque/0.8.8/download) |
| [crossbeam-epoch 0.9.21](licenses/dependencies/crossbeam-epoch-0.9.21/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crossbeam-epoch/0.9.21/download) |
| [crossbeam-utils 0.8.23](licenses/dependencies/crossbeam-utils-0.8.23/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crossbeam-utils/0.8.23/download) |
| [crypto-common 0.1.7](licenses/dependencies/crypto-common-0.1.7/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/crypto-common/0.1.7/download) |
| [cursor-icon 1.2.0](licenses/dependencies/cursor-icon-1.2.0/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/cursor-icon/1.2.0/download) |
| [data-url 0.3.2](licenses/dependencies/data-url-0.3.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/data-url/0.3.2/download) |
| [defmt 1.2.0](licenses/dependencies/defmt-1.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/defmt/1.2.0/download) |
| [defmt-macros 1.2.0](licenses/dependencies/defmt-macros-1.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/defmt-macros/1.2.0/download) |
| [defmt-parser 1.1.0](licenses/dependencies/defmt-parser-1.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/defmt-parser/1.1.0/download) |
| [deranged 0.5.9](licenses/dependencies/deranged-0.5.9/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/deranged/0.5.9/download) |
| [digest 0.10.7](licenses/dependencies/digest-0.10.7/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/digest/0.10.7/download) |
| [directories 6.0.0](licenses/dependencies/directories-6.0.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/directories/6.0.0/download) |
| [dirs-sys 0.5.0](licenses/dependencies/dirs-sys-0.5.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/dirs-sys/0.5.0/download) |
| [displaydoc 0.2.7](licenses/dependencies/displaydoc-0.2.7/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/displaydoc/0.2.7/download) |
| [document-features 0.2.12](licenses/dependencies/document-features-0.2.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/document-features/0.2.12/download) |
| [dpi 0.1.2](licenses/dependencies/dpi-0.1.2/) | Apache-2.0 AND MIT | [crate](https://crates.io/api/v1/crates/dpi/0.1.2/download) |
| [ecb 0.1.2](licenses/dependencies/ecb-0.1.2/) | MIT | [crate](https://crates.io/api/v1/crates/ecb/0.1.2/download) |
| [ecolor 0.32.3](licenses/dependencies/ecolor-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ecolor/0.32.3/download) |
| [eframe 0.32.3](licenses/dependencies/eframe-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/eframe/0.32.3/download) |
| [egui 0.32.3](licenses/dependencies/egui-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/egui/0.32.3/download) |
| [egui_glow 0.32.3](licenses/dependencies/egui_glow-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/egui_glow/0.32.3/download) |
| [egui-wgpu 0.32.3](licenses/dependencies/egui-wgpu-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/egui-wgpu/0.32.3/download) |
| [egui-winit 0.32.3](licenses/dependencies/egui-winit-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/egui-winit/0.32.3/download) |
| [either 1.19.0](licenses/dependencies/either-1.19.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/either/1.19.0/download) |
| [emath 0.32.3](licenses/dependencies/emath-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/emath/0.32.3/download) |
| [encode_unicode 1.0.0](licenses/dependencies/encode_unicode-1.0.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/encode_unicode/1.0.0/download) |
| [encoding_rs 0.8.42](licenses/dependencies/encoding_rs-0.8.42/) | (Apache-2.0 OR MIT) AND BSD-3-Clause | [crate](https://crates.io/api/v1/crates/encoding_rs/0.8.42/download) |
| [epaint 0.32.3](licenses/dependencies/epaint-0.32.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/epaint/0.32.3/download) |
| [epaint_default_fonts 0.32.3](licenses/dependencies/epaint_default_fonts-0.32.3/) | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | [crate](https://crates.io/api/v1/crates/epaint_default_fonts/0.32.3/download) |
| [equator 0.4.2](licenses/dependencies/equator-0.4.2/) | MIT | [crate](https://crates.io/api/v1/crates/equator/0.4.2/download) |
| [equator-macro 0.4.2](licenses/dependencies/equator-macro-0.4.2/) | MIT | [crate](https://crates.io/api/v1/crates/equator-macro/0.4.2/download) |
| [equivalent 1.0.3](licenses/dependencies/equivalent-1.0.3/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/equivalent/1.0.3/download) |
| [error-code 3.4.0](licenses/dependencies/error-code-3.4.0/) | BSL-1.0 | [crate](https://crates.io/api/v1/crates/error-code/3.4.0/download) |
| [euclid 0.22.14](licenses/dependencies/euclid-0.22.14/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/euclid/0.22.14/download) |
| [exr 1.74.2](licenses/dependencies/exr-1.74.2/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/exr/1.74.2/download) |
| [fastrand 2.5.0](licenses/dependencies/fastrand-2.5.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/fastrand/2.5.0/download) |
| [fax 0.2.7](licenses/dependencies/fax-0.2.7/) | MIT | [crate](https://crates.io/api/v1/crates/fax/0.2.7/download) |
| [fdeflate 0.3.7](licenses/dependencies/fdeflate-0.3.7/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/fdeflate/0.3.7/download) |
| [find-msvc-tools 0.1.15](licenses/dependencies/find-msvc-tools-0.1.15/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/find-msvc-tools/0.1.15/download) |
| [flate2 1.1.10](licenses/dependencies/flate2-1.1.10/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/flate2/1.1.10/download) |
| [float-cmp 0.9.0](licenses/dependencies/float-cmp-0.9.0/) | MIT | [crate](https://crates.io/api/v1/crates/float-cmp/0.9.0/download) |
| [fnv 1.0.7](licenses/dependencies/fnv-1.0.7/) | Apache-2.0 / MIT | [crate](https://crates.io/api/v1/crates/fnv/1.0.7/download) |
| [foldhash 0.1.5](licenses/dependencies/foldhash-0.1.5/) | Zlib | [crate](https://crates.io/api/v1/crates/foldhash/0.1.5/download) |
| [fontdb 0.24.0](licenses/dependencies/fontdb-0.24.0/) | MIT | [crate](https://crates.io/api/v1/crates/fontdb/0.24.0/download) |
| [font-types 0.12.7](licenses/dependencies/font-types-0.12.7/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/font-types/0.12.7/download) |
| [form_urlencoded 1.2.2](licenses/dependencies/form_urlencoded-1.2.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/form_urlencoded/1.2.2/download) |
| [futures 0.3.34](licenses/dependencies/futures-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures/0.3.34/download) |
| [futures-channel 0.3.34](licenses/dependencies/futures-channel-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-channel/0.3.34/download) |
| [futures-core 0.3.34](licenses/dependencies/futures-core-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-core/0.3.34/download) |
| [futures-executor 0.3.34](licenses/dependencies/futures-executor-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-executor/0.3.34/download) |
| [futures-io 0.3.34](licenses/dependencies/futures-io-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-io/0.3.34/download) |
| [futures-macro 0.3.34](licenses/dependencies/futures-macro-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-macro/0.3.34/download) |
| [futures-sink 0.3.34](licenses/dependencies/futures-sink-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-sink/0.3.34/download) |
| [futures-task 0.3.34](licenses/dependencies/futures-task-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-task/0.3.34/download) |
| [futures-util 0.3.34](licenses/dependencies/futures-util-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/futures-util/0.3.34/download) |
| [generic-array 0.14.7](licenses/dependencies/generic-array-0.14.7/) | MIT | [crate](https://crates.io/api/v1/crates/generic-array/0.14.7/download) |
| [getopts 0.2.24](licenses/dependencies/getopts-0.2.24/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/getopts/0.2.24/download) |
| [getrandom 0.2.17](licenses/dependencies/getrandom-0.2.17/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/getrandom/0.2.17/download) |
| [getrandom 0.3.4](licenses/dependencies/getrandom-0.3.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/getrandom/0.3.4/download) |
| [getrandom 0.4.3](licenses/dependencies/getrandom-0.4.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/getrandom/0.4.3/download) |
| [gif 0.14.2](licenses/dependencies/gif-0.14.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gif/0.14.2/download) |
| [gl_generator 0.14.0](licenses/dependencies/gl_generator-0.14.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/gl_generator/0.14.0/download) |
| [global-hotkey 0.7.0](licenses/dependencies/global-hotkey-0.7.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/global-hotkey/0.7.0/download) |
| [glow 0.16.0](licenses/dependencies/glow-0.16.0/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/glow/0.16.0/download) |
| [glutin 0.32.3](licenses/dependencies/glutin-0.32.3/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/glutin/0.32.3/download) |
| [glutin_egl_sys 0.7.1](licenses/dependencies/glutin_egl_sys-0.7.1/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/glutin_egl_sys/0.7.1/download) |
| [glutin_wgl_sys 0.6.1](licenses/dependencies/glutin_wgl_sys-0.6.1/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/glutin_wgl_sys/0.6.1/download) |
| [glutin-winit 0.5.0](licenses/dependencies/glutin-winit-0.5.0/) | MIT | [crate](https://crates.io/api/v1/crates/glutin-winit/0.5.0/download) |
| [gpu-alloc 0.6.2](licenses/dependencies/gpu-alloc-0.6.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gpu-alloc/0.6.2/download) |
| [gpu-allocator 0.27.0](licenses/dependencies/gpu-allocator-0.27.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gpu-allocator/0.27.0/download) |
| [gpu-alloc-types 0.3.1](licenses/dependencies/gpu-alloc-types-0.3.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gpu-alloc-types/0.3.1/download) |
| [gpu-descriptor 0.3.2](licenses/dependencies/gpu-descriptor-0.3.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gpu-descriptor/0.3.2/download) |
| [gpu-descriptor-types 0.2.0](licenses/dependencies/gpu-descriptor-types-0.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/gpu-descriptor-types/0.2.0/download) |
| [h2 0.4.20](licenses/dependencies/h2-0.4.20/) | MIT | [crate](https://crates.io/api/v1/crates/h2/0.4.20/download) |
| [half 2.7.1](licenses/dependencies/half-2.7.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/half/2.7.1/download) |
| [harfrust 0.12.0](licenses/dependencies/harfrust-0.12.0/) | MIT | [crate](https://crates.io/api/v1/crates/harfrust/0.12.0/download) |
| [hashbrown 0.15.5](licenses/dependencies/hashbrown-0.15.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/hashbrown/0.15.5/download) |
| [hashbrown 0.17.1](licenses/dependencies/hashbrown-0.17.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/hashbrown/0.17.1/download) |
| [heck 0.5.0](licenses/dependencies/heck-0.5.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/heck/0.5.0/download) |
| [hexf-parse 0.2.1](licenses/dependencies/hexf-parse-0.2.1/) | CC0-1.0 | [crate](https://crates.io/api/v1/crates/hexf-parse/0.2.1/download) |
| [http 1.5.0](licenses/dependencies/http-1.5.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/http/1.5.0/download) |
| [httparse 1.10.1](licenses/dependencies/httparse-1.10.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/httparse/1.10.1/download) |
| [http-body 1.1.0](licenses/dependencies/http-body-1.1.0/) | MIT | [crate](https://crates.io/api/v1/crates/http-body/1.1.0/download) |
| [http-body-util 0.1.5](licenses/dependencies/http-body-util-0.1.5/) | MIT | [crate](https://crates.io/api/v1/crates/http-body-util/0.1.5/download) |
| [httpdate 1.0.3](licenses/dependencies/httpdate-1.0.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/httpdate/1.0.3/download) |
| [hyper 1.12.0](licenses/dependencies/hyper-1.12.0/) | MIT | [crate](https://crates.io/api/v1/crates/hyper/1.12.0/download) |
| [hyper-rustls 0.27.10](licenses/dependencies/hyper-rustls-0.27.10/) | Apache-2.0 OR ISC OR MIT | [crate](https://crates.io/api/v1/crates/hyper-rustls/0.27.10/download) |
| [hyper-tls 0.6.0](licenses/dependencies/hyper-tls-0.6.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/hyper-tls/0.6.0/download) |
| [hyper-util 0.1.21](licenses/dependencies/hyper-util-0.1.21/) | MIT | [crate](https://crates.io/api/v1/crates/hyper-util/0.1.21/download) |
| [icu_collections 2.3.0](licenses/dependencies/icu_collections-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_collections/2.3.0/download) |
| [icu_locale_core 2.3.0](licenses/dependencies/icu_locale_core-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_locale_core/2.3.0/download) |
| [icu_normalizer 2.3.0](licenses/dependencies/icu_normalizer-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_normalizer/2.3.0/download) |
| [icu_normalizer_data 2.3.0](licenses/dependencies/icu_normalizer_data-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_normalizer_data/2.3.0/download) |
| [icu_properties 2.3.0](licenses/dependencies/icu_properties-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_properties/2.3.0/download) |
| [icu_properties_data 2.3.0](licenses/dependencies/icu_properties_data-2.3.0/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_properties_data/2.3.0/download) |
| [icu_provider 2.3.1](licenses/dependencies/icu_provider-2.3.1/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/icu_provider/2.3.1/download) |
| [idna 1.1.0](licenses/dependencies/idna-1.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/idna/1.1.0/download) |
| [idna_adapter 1.2.2](licenses/dependencies/idna_adapter-1.2.2/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/idna_adapter/1.2.2/download) |
| [image 0.25.10](licenses/dependencies/image-0.25.10/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/image/0.25.10/download) |
| [imagesize 0.15.0](licenses/dependencies/imagesize-0.15.0/) | MIT | [crate](https://crates.io/api/v1/crates/imagesize/0.15.0/download) |
| [image-webp 0.2.4](licenses/dependencies/image-webp-0.2.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/image-webp/0.2.4/download) |
| [imgref 1.12.3](licenses/dependencies/imgref-1.12.3/) | CC0-1.0 OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/imgref/1.12.3/download) |
| [indexmap 2.14.2](licenses/dependencies/indexmap-2.14.2/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/indexmap/2.14.2/download) |
| [indicatif 0.17.11](licenses/dependencies/indicatif-0.17.11/) | MIT | [crate](https://crates.io/api/v1/crates/indicatif/0.17.11/download) |
| [infer 0.19.0](licenses/dependencies/infer-0.19.0/) | MIT | [crate](https://crates.io/api/v1/crates/infer/0.19.0/download) |
| [inout 0.1.4](licenses/dependencies/inout-0.1.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/inout/0.1.4/download) |
| [ipnet 2.12.2](licenses/dependencies/ipnet-2.12.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ipnet/2.12.2/download) |
| [itertools 0.14.0](licenses/dependencies/itertools-0.14.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/itertools/0.14.0/download) |
| [itoa 1.0.18](licenses/dependencies/itoa-1.0.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/itoa/1.0.18/download) |
| [jiff 0.2.38](licenses/dependencies/jiff-0.2.38/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/jiff/0.2.38/download) |
| [jiff-core 0.1.1](licenses/dependencies/jiff-core-0.1.1/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/jiff-core/0.1.1/download) |
| [jiff-static 0.2.38](licenses/dependencies/jiff-static-0.2.38/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/jiff-static/0.2.38/download) |
| [jiff-tzdb 0.1.9](licenses/dependencies/jiff-tzdb-0.1.9/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/jiff-tzdb/0.1.9/download) |
| [jiff-tzdb-platform 0.1.3](licenses/dependencies/jiff-tzdb-platform-0.1.3/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/jiff-tzdb-platform/0.1.3/download) |
| [jobserver 0.1.35](licenses/dependencies/jobserver-0.1.35/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/jobserver/0.1.35/download) |
| [keyboard-types 0.6.2](licenses/dependencies/keyboard-types-0.6.2/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/keyboard-types/0.6.2/download) |
| [keyboard-types 0.7.0](licenses/dependencies/keyboard-types-0.7.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/keyboard-types/0.7.0/download) |
| [keyring 3.6.3](licenses/dependencies/keyring-3.6.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/keyring/3.6.3/download) |
| [khronos_api 3.1.0](licenses/dependencies/khronos_api-3.1.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/khronos_api/3.1.0/download) |
| [khronos-egl 6.0.0](licenses/dependencies/khronos-egl-6.0.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/khronos-egl/6.0.0/download) |
| [kurbo 0.13.1](licenses/dependencies/kurbo-0.13.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/kurbo/0.13.1/download) |
| [latex-rust 2.1.1](licenses/dependencies/latex-rust-2.1.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/latex-rust/2.1.1/download) |
| [lebe 0.5.3](licenses/dependencies/lebe-0.5.3/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/lebe/0.5.3/download) |
| [libc 0.2.191](licenses/dependencies/libc-0.2.191/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/libc/0.2.191/download) |
| [libloading 0.8.9](licenses/dependencies/libloading-0.8.9/) | ISC | [crate](https://crates.io/api/v1/crates/libloading/0.8.9/download) |
| [libm 0.2.16](licenses/dependencies/libm-0.2.16/) | MIT | [crate](https://crates.io/api/v1/crates/libm/0.2.16/download) |
| [litemap 0.8.3](licenses/dependencies/litemap-0.8.3/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/litemap/0.8.3/download) |
| [litrs 1.0.1](licenses/dependencies/litrs-1.0.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/litrs/1.0.1/download) |
| [lock_api 0.4.14](licenses/dependencies/lock_api-0.4.14/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/lock_api/0.4.14/download) |
| [log 0.4.34](licenses/dependencies/log-0.4.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/log/0.4.34/download) |
| [loop9 0.1.5](licenses/dependencies/loop9-0.1.5/) | MIT | [crate](https://crates.io/api/v1/crates/loop9/0.1.5/download) |
| [lopdf 0.38.0](licenses/dependencies/lopdf-0.38.0/) | MIT | [crate](https://crates.io/api/v1/crates/lopdf/0.38.0/download) |
| [lru-slab 0.1.3](licenses/dependencies/lru-slab-0.1.3/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/lru-slab/0.1.3/download) |
| [matchit 0.7.3](licenses/dependencies/matchit-0.7.3/) | MIT AND BSD-3-Clause | [crate](https://crates.io/api/v1/crates/matchit/0.7.3/download) |
| [maybe-rayon 0.1.1](licenses/dependencies/maybe-rayon-0.1.1/) | MIT | [crate](https://crates.io/api/v1/crates/maybe-rayon/0.1.1/download) |
| [md-5 0.10.6](licenses/dependencies/md-5-0.10.6/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/md-5/0.10.6/download) |
| [memchr 2.8.3](licenses/dependencies/memchr-2.8.3/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/memchr/2.8.3/download) |
| [memmap2 0.9.12](licenses/dependencies/memmap2-0.9.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/memmap2/0.9.12/download) |
| [memoffset 0.9.1](licenses/dependencies/memoffset-0.9.1/) | MIT | [crate](https://crates.io/api/v1/crates/memoffset/0.9.1/download) |
| [mime 0.3.17](licenses/dependencies/mime-0.3.17/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/mime/0.3.17/download) |
| [mime_guess 2.0.5](licenses/dependencies/mime_guess-2.0.5/) | MIT | [crate](https://crates.io/api/v1/crates/mime_guess/2.0.5/download) |
| [miniz_oxide 0.8.9](licenses/dependencies/miniz_oxide-0.8.9/) | MIT OR Zlib OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/miniz_oxide/0.8.9/download) |
| [miniz_oxide 0.9.1](licenses/dependencies/miniz_oxide-0.9.1/) | MIT OR Zlib OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/miniz_oxide/0.9.1/download) |
| [mio 1.2.4](licenses/dependencies/mio-1.2.4/) | MIT | [crate](https://crates.io/api/v1/crates/mio/1.2.4/download) |
| [moxcms 0.8.1](licenses/dependencies/moxcms-0.8.1/) | BSD-3-Clause OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/moxcms/0.8.1/download) |
| [muda 0.2.0](licenses/dependencies/muda-0.2.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/muda/0.2.0/download) |
| [multiversion_no_op 1.0.0](licenses/dependencies/multiversion_no_op-1.0.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/multiversion_no_op/1.0.0/download) |
| [naga 25.0.1](licenses/dependencies/naga-25.0.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/naga/25.0.1/download) |
| [native-tls 0.2.18](licenses/dependencies/native-tls-0.2.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/native-tls/0.2.18/download) |
| [new_debug_unreachable 1.0.6](licenses/dependencies/new_debug_unreachable-1.0.6/) | MIT | [crate](https://crates.io/api/v1/crates/new_debug_unreachable/1.0.6/download) |
| [no_std_io2 0.9.4](licenses/dependencies/no_std_io2-0.9.4/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/no_std_io2/0.9.4/download) |
| [nohash-hasher 0.2.0](licenses/dependencies/nohash-hasher-0.2.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/nohash-hasher/0.2.0/download) |
| [nom 8.0.0](licenses/dependencies/nom-8.0.0/) | MIT | [crate](https://crates.io/api/v1/crates/nom/8.0.0/download) |
| [nom_locate 5.0.0](licenses/dependencies/nom_locate-5.0.0/) | MIT | [crate](https://crates.io/api/v1/crates/nom_locate/5.0.0/download) |
| [noop_proc_macro 0.3.0](licenses/dependencies/noop_proc_macro-0.3.0/) | MIT | [crate](https://crates.io/api/v1/crates/noop_proc_macro/0.3.0/download) |
| [number_prefix 0.4.0](licenses/dependencies/number_prefix-0.4.0/) | MIT | [crate](https://crates.io/api/v1/crates/number_prefix/0.4.0/download) |
| [num-bigint 0.4.8](licenses/dependencies/num-bigint-0.4.8/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-bigint/0.4.8/download) |
| [num-complex 0.4.6](licenses/dependencies/num-complex-0.4.6/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-complex/0.4.6/download) |
| [num-conv 0.2.2](licenses/dependencies/num-conv-0.2.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-conv/0.2.2/download) |
| [num-derive 0.4.2](licenses/dependencies/num-derive-0.4.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-derive/0.4.2/download) |
| [num-integer 0.1.47](licenses/dependencies/num-integer-0.1.47/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-integer/0.1.47/download) |
| [num-rational 0.4.2](licenses/dependencies/num-rational-0.4.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-rational/0.4.2/download) |
| [num-traits 0.2.19](licenses/dependencies/num-traits-0.2.19/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/num-traits/0.2.19/download) |
| [once_cell 1.21.4](licenses/dependencies/once_cell-1.21.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/once_cell/1.21.4/download) |
| [open 5.4.4](licenses/dependencies/open-5.4.4/) | MIT | [crate](https://crates.io/api/v1/crates/open/5.4.4/download) |
| [option-ext 0.2.0](licenses/dependencies/option-ext-0.2.0/) | MPL-2.0 | [crate](https://crates.io/api/v1/crates/option-ext/0.2.0/download) |
| [ordered-float 4.6.0](licenses/dependencies/ordered-float-4.6.0/) | MIT | [crate](https://crates.io/api/v1/crates/ordered-float/4.6.0/download) |
| [owned_ttf_parser 0.25.1](licenses/dependencies/owned_ttf_parser-0.25.1/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/owned_ttf_parser/0.25.1/download) |
| [parking_lot 0.12.5](licenses/dependencies/parking_lot-0.12.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/parking_lot/0.12.5/download) |
| [parking_lot_core 0.9.12](licenses/dependencies/parking_lot_core-0.9.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/parking_lot_core/0.9.12/download) |
| [paste 1.0.15](licenses/dependencies/paste-1.0.15/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/paste/1.0.15/download) |
| [pastey 0.1.1](licenses/dependencies/pastey-0.1.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/pastey/0.1.1/download) |
| [percent-encoding 2.3.2](licenses/dependencies/percent-encoding-2.3.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/percent-encoding/2.3.2/download) |
| [pico-args 0.5.0](licenses/dependencies/pico-args-0.5.0/) | MIT | [crate](https://crates.io/api/v1/crates/pico-args/0.5.0/download) |
| [pin-project-lite 0.2.17](licenses/dependencies/pin-project-lite-0.2.17/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/pin-project-lite/0.2.17/download) |
| [pkg-config 0.3.34](licenses/dependencies/pkg-config-0.3.34/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/pkg-config/0.3.34/download) |
| [png 0.18.1](licenses/dependencies/png-0.18.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/png/0.18.1/download) |
| [polycool 0.4.0](licenses/dependencies/polycool-0.4.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/polycool/0.4.0/download) |
| [portable-atomic 1.15.0](licenses/dependencies/portable-atomic-1.15.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/portable-atomic/1.15.0/download) |
| [potential_utf 0.1.6](licenses/dependencies/potential_utf-0.1.6/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/potential_utf/0.1.6/download) |
| [powerfmt 0.2.1](licenses/dependencies/powerfmt-0.2.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/powerfmt/0.2.1/download) |
| [ppv-lite86 0.2.21](licenses/dependencies/ppv-lite86-0.2.21/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ppv-lite86/0.2.21/download) |
| [presser 0.3.1](licenses/dependencies/presser-0.3.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/presser/0.3.1/download) |
| [proc-macro2 1.0.107](licenses/dependencies/proc-macro2-1.0.107/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/proc-macro2/1.0.107/download) |
| [profiling 1.0.18](licenses/dependencies/profiling-1.0.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/profiling/1.0.18/download) |
| [profiling-procmacros 1.0.18](licenses/dependencies/profiling-procmacros-1.0.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/profiling-procmacros/1.0.18/download) |
| [pulldown-cmark 0.13.5](licenses/dependencies/pulldown-cmark-0.13.5/) | MIT | [crate](https://crates.io/api/v1/crates/pulldown-cmark/0.13.5/download) |
| [pulldown-cmark-escape 0.11.0](licenses/dependencies/pulldown-cmark-escape-0.11.0/) | MIT | [crate](https://crates.io/api/v1/crates/pulldown-cmark-escape/0.11.0/download) |
| [pulp 0.22.3](licenses/dependencies/pulp-0.22.3/) | MIT | [crate](https://crates.io/api/v1/crates/pulp/0.22.3/download) |
| [pulp-wasm-simd-flag 0.1.1](licenses/dependencies/pulp-wasm-simd-flag-0.1.1/) | MIT | [crate](https://crates.io/api/v1/crates/pulp-wasm-simd-flag/0.1.1/download) |
| [pxfm 0.1.30](licenses/dependencies/pxfm-0.1.30/) | BSD-3-Clause OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/pxfm/0.1.30/download) |
| [qoi 0.4.1](licenses/dependencies/qoi-0.4.1/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/qoi/0.4.1/download) |
| [quick-error 2.0.1](licenses/dependencies/quick-error-2.0.1/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/quick-error/2.0.1/download) |
| [quick-xml 0.31.0](licenses/dependencies/quick-xml-0.31.0/) | MIT | [crate](https://crates.io/api/v1/crates/quick-xml/0.31.0/download) |
| [quick-xml 0.37.5](licenses/dependencies/quick-xml-0.37.5/) | MIT | [crate](https://crates.io/api/v1/crates/quick-xml/0.37.5/download) |
| [quinn 0.11.12](licenses/dependencies/quinn-0.11.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/quinn/0.11.12/download) |
| [quinn-proto 0.11.19](licenses/dependencies/quinn-proto-0.11.19/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/quinn-proto/0.11.19/download) |
| [quinn-udp 0.5.16](licenses/dependencies/quinn-udp-0.5.16/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/quinn-udp/0.5.16/download) |
| [quote 1.0.47](licenses/dependencies/quote-1.0.47/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/quote/1.0.47/download) |
| [rand 0.10.3](licenses/dependencies/rand-0.10.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand/0.10.3/download) |
| [rand 0.9.5](licenses/dependencies/rand-0.9.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand/0.9.5/download) |
| [rand_chacha 0.9.0](licenses/dependencies/rand_chacha-0.9.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand_chacha/0.9.0/download) |
| [rand_core 0.10.1](licenses/dependencies/rand_core-0.10.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand_core/0.10.1/download) |
| [rand_core 0.9.5](licenses/dependencies/rand_core-0.9.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand_core/0.9.5/download) |
| [rand_pcg 0.10.2](licenses/dependencies/rand_pcg-0.10.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rand_pcg/0.10.2/download) |
| [range-alloc 0.1.5](licenses/dependencies/range-alloc-0.1.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/range-alloc/0.1.5/download) |
| [rangemap 1.8.0](licenses/dependencies/rangemap-1.8.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/rangemap/1.8.0/download) |
| [rav1e 0.8.1](licenses/dependencies/rav1e-0.8.1/) | BSD-2-Clause | [crate](https://crates.io/api/v1/crates/rav1e/0.8.1/download) |
| [ravif 0.13.0](licenses/dependencies/ravif-0.13.0/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/ravif/0.13.0/download) |
| [raw-cpuid 11.6.0](licenses/dependencies/raw-cpuid-11.6.0/) | MIT | [crate](https://crates.io/api/v1/crates/raw-cpuid/11.6.0/download) |
| [raw-window-handle 0.6.2](licenses/dependencies/raw-window-handle-0.6.2/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/raw-window-handle/0.6.2/download) |
| [rayon 1.12.0](licenses/dependencies/rayon-1.12.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rayon/1.12.0/download) |
| [rayon-core 1.13.0](licenses/dependencies/rayon-core-1.13.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rayon-core/1.13.0/download) |
| [read-fonts 0.41.0](licenses/dependencies/read-fonts-0.41.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/read-fonts/0.41.0/download) |
| [reborrow 0.5.5](licenses/dependencies/reborrow-0.5.5/) | MIT | [crate](https://crates.io/api/v1/crates/reborrow/0.5.5/download) |
| [regex 1.13.1](licenses/dependencies/regex-1.13.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/regex/1.13.1/download) |
| [regex-automata 0.4.18](licenses/dependencies/regex-automata-0.4.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/regex-automata/0.4.18/download) |
| [regex-syntax 0.8.11](licenses/dependencies/regex-syntax-0.8.11/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/regex-syntax/0.8.11/download) |
| [renderdoc-sys 1.1.0](licenses/dependencies/renderdoc-sys-1.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/renderdoc-sys/1.1.0/download) |
| [reqwest 0.12.28](licenses/dependencies/reqwest-0.12.28/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/reqwest/0.12.28/download) |
| [resvg 0.48.1](licenses/dependencies/resvg-0.48.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/resvg/0.48.1/download) |
| [rfd 0.17.2](licenses/dependencies/rfd-0.17.2/) | MIT | [crate](https://crates.io/api/v1/crates/rfd/0.17.2/download) |
| [rgb 0.8.53](licenses/dependencies/rgb-0.8.53/) | MIT | [crate](https://crates.io/api/v1/crates/rgb/0.8.53/download) |
| [ring 0.17.14](licenses/dependencies/ring-0.17.14/) | Apache-2.0 AND ISC | [crate](https://crates.io/api/v1/crates/ring/0.17.14/download) |
| [roxmltree 0.21.1](licenses/dependencies/roxmltree-0.21.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/roxmltree/0.21.1/download) |
| [rustc-hash 1.1.0](licenses/dependencies/rustc-hash-1.1.0/) | Apache-2.0/MIT | [crate](https://crates.io/api/v1/crates/rustc-hash/1.1.0/download) |
| [rustc-hash 2.1.3](licenses/dependencies/rustc-hash-2.1.3/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/rustc-hash/2.1.3/download) |
| [rustls 0.23.46](licenses/dependencies/rustls-0.23.46/) | Apache-2.0 OR ISC OR MIT | [crate](https://crates.io/api/v1/crates/rustls/0.23.46/download) |
| [rustls-pki-types 1.15.1](licenses/dependencies/rustls-pki-types-1.15.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rustls-pki-types/1.15.1/download) |
| [rustls-webpki 0.103.16](licenses/dependencies/rustls-webpki-0.103.16/) | ISC | [crate](https://crates.io/api/v1/crates/rustls-webpki/0.103.16/download) |
| [rustversion 1.0.23](licenses/dependencies/rustversion-1.0.23/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/rustversion/1.0.23/download) |
| [rwml 0.1.5](licenses/dependencies/rwml-0.1.5/) | MIT | [crate](https://crates.io/api/v1/crates/rwml/0.1.5/download) |
| [ryu 1.0.23](licenses/dependencies/ryu-1.0.23/) | Apache-2.0 OR BSL-1.0 | [crate](https://crates.io/api/v1/crates/ryu/1.0.23/download) |
| [schannel 0.1.30](licenses/dependencies/schannel-0.1.30/) | MIT | [crate](https://crates.io/api/v1/crates/schannel/0.1.30/download) |
| [scopeguard 1.2.0](licenses/dependencies/scopeguard-1.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/scopeguard/1.2.0/download) |
| [self_update 0.42.0](licenses/dependencies/self_update-0.42.0/) | MIT | [crate](https://crates.io/api/v1/crates/self_update/0.42.0/download) |
| [self-replace 1.5.0](licenses/dependencies/self-replace-1.5.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/self-replace/1.5.0/download) |
| [semver 1.0.28](licenses/dependencies/semver-1.0.28/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/semver/1.0.28/download) |
| [serde 1.0.229](licenses/dependencies/serde-1.0.229/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde/1.0.229/download) |
| [serde_core 1.0.229](licenses/dependencies/serde_core-1.0.229/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde_core/1.0.229/download) |
| [serde_derive 1.0.229](licenses/dependencies/serde_derive-1.0.229/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde_derive/1.0.229/download) |
| [serde_json 1.0.154](licenses/dependencies/serde_json-1.0.154/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde_json/1.0.154/download) |
| [serde_path_to_error 0.1.20](licenses/dependencies/serde_path_to_error-0.1.20/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde_path_to_error/0.1.20/download) |
| [serde_urlencoded 0.7.1](licenses/dependencies/serde_urlencoded-0.7.1/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/serde_urlencoded/0.7.1/download) |
| [sha2 0.10.9](licenses/dependencies/sha2-0.10.9/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/sha2/0.10.9/download) |
| [shlex 2.0.1](licenses/dependencies/shlex-2.0.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/shlex/2.0.1/download) |
| [simd_helpers 0.1.0](licenses/dependencies/simd_helpers-0.1.0/) | MIT | [crate](https://crates.io/api/v1/crates/simd_helpers/0.1.0/download) |
| [simd-adler32 0.3.10](licenses/dependencies/simd-adler32-0.3.10/) | MIT | [crate](https://crates.io/api/v1/crates/simd-adler32/0.3.10/download) |
| [simdutf8 0.1.5](licenses/dependencies/simdutf8-0.1.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/simdutf8/0.1.5/download) |
| [simplecss 0.2.2](licenses/dependencies/simplecss-0.2.2/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/simplecss/0.2.2/download) |
| [siphasher 1.0.4](licenses/dependencies/siphasher-1.0.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/siphasher/1.0.4/download) |
| [skrifa 0.44.0](licenses/dependencies/skrifa-0.44.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/skrifa/0.44.0/download) |
| [slab 0.4.12](licenses/dependencies/slab-0.4.12/) | MIT | [crate](https://crates.io/api/v1/crates/slab/0.4.12/download) |
| [slotmap 1.1.1](licenses/dependencies/slotmap-1.1.1/) | Zlib | [crate](https://crates.io/api/v1/crates/slotmap/1.1.1/download) |
| [smallvec 1.16.3](licenses/dependencies/smallvec-1.16.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/smallvec/1.16.3/download) |
| [smol_str 0.2.2](licenses/dependencies/smol_str-0.2.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/smol_str/0.2.2/download) |
| [socket2 0.6.5](licenses/dependencies/socket2-0.6.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/socket2/0.6.5/download) |
| [spirv 0.3.0+sdk-1.3.268.0](licenses/dependencies/spirv-0.3.0+sdk-1.3.268.0/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/spirv/0.3.0+sdk-1.3.268.0/download) |
| [stable_deref_trait 1.2.1](licenses/dependencies/stable_deref_trait-1.2.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/stable_deref_trait/1.2.1/download) |
| [static_assertions 1.1.0](licenses/dependencies/static_assertions-1.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/static_assertions/1.1.0/download) |
| [strict-num 0.1.1](licenses/dependencies/strict-num-0.1.1/) | MIT | [crate](https://crates.io/api/v1/crates/strict-num/0.1.1/download) |
| [stringprep 0.1.5](licenses/dependencies/stringprep-0.1.5/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/stringprep/0.1.5/download) |
| [strum 0.26.3](licenses/dependencies/strum-0.26.3/) | MIT | [crate](https://crates.io/api/v1/crates/strum/0.26.3/download) |
| [strum_macros 0.26.4](licenses/dependencies/strum_macros-0.26.4/) | MIT | [crate](https://crates.io/api/v1/crates/strum_macros/0.26.4/download) |
| [subtle 2.6.1](licenses/dependencies/subtle-2.6.1/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/subtle/2.6.1/download) |
| [svgtypes 0.16.1](licenses/dependencies/svgtypes-0.16.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/svgtypes/0.16.1/download) |
| [syn 2.0.119](licenses/dependencies/syn-2.0.119/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/syn/2.0.119/download) |
| [syn 3.0.9](licenses/dependencies/syn-3.0.9/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/syn/3.0.9/download) |
| [sync_wrapper 1.0.2](licenses/dependencies/sync_wrapper-1.0.2/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/sync_wrapper/1.0.2/download) |
| [synstructure 0.14.0](licenses/dependencies/synstructure-0.14.0/) | MIT | [crate](https://crates.io/api/v1/crates/synstructure/0.14.0/download) |
| [tempfile 3.27.0](licenses/dependencies/tempfile-3.27.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/tempfile/3.27.0/download) |
| [termcolor 1.4.1](licenses/dependencies/termcolor-1.4.1/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/termcolor/1.4.1/download) |
| [thiserror 1.0.69](licenses/dependencies/thiserror-1.0.69/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/thiserror/1.0.69/download) |
| [thiserror 2.0.21](licenses/dependencies/thiserror-2.0.21/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/thiserror/2.0.21/download) |
| [thiserror-impl 1.0.69](licenses/dependencies/thiserror-impl-1.0.69/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/thiserror-impl/1.0.69/download) |
| [thiserror-impl 2.0.21](licenses/dependencies/thiserror-impl-2.0.21/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/thiserror-impl/2.0.21/download) |
| [tiff 0.11.3](licenses/dependencies/tiff-0.11.3/) | MIT | [crate](https://crates.io/api/v1/crates/tiff/0.11.3/download) |
| [time 0.3.55](licenses/dependencies/time-0.3.55/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/time/0.3.55/download) |
| [time-core 0.1.9](licenses/dependencies/time-core-0.1.9/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/time-core/0.1.9/download) |
| [time-macros 0.2.32](licenses/dependencies/time-macros-0.2.32/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/time-macros/0.2.32/download) |
| [tiny-skia 0.12.0](licenses/dependencies/tiny-skia-0.12.0/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/tiny-skia/0.12.0/download) |
| [tiny-skia-path 0.12.0](licenses/dependencies/tiny-skia-path-0.12.0/) | BSD-3-Clause | [crate](https://crates.io/api/v1/crates/tiny-skia-path/0.12.0/download) |
| [tinystr 0.8.4](licenses/dependencies/tinystr-0.8.4/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/tinystr/0.8.4/download) |
| [tinyvec 1.13.3](licenses/dependencies/tinyvec-1.13.3/) | Zlib OR Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/tinyvec/1.13.3/download) |
| [tokio 1.54.1](licenses/dependencies/tokio-1.54.1/) | MIT | [crate](https://crates.io/api/v1/crates/tokio/1.54.1/download) |
| [tokio-macros 2.7.2](licenses/dependencies/tokio-macros-2.7.2/) | MIT | [crate](https://crates.io/api/v1/crates/tokio-macros/2.7.2/download) |
| [tokio-native-tls 0.3.1](licenses/dependencies/tokio-native-tls-0.3.1/) | MIT | [crate](https://crates.io/api/v1/crates/tokio-native-tls/0.3.1/download) |
| [tokio-rustls 0.26.6](licenses/dependencies/tokio-rustls-0.26.6/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/tokio-rustls/0.26.6/download) |
| [tokio-util 0.7.20](licenses/dependencies/tokio-util-0.7.20/) | MIT | [crate](https://crates.io/api/v1/crates/tokio-util/0.7.20/download) |
| [tower 0.5.3](licenses/dependencies/tower-0.5.3/) | MIT | [crate](https://crates.io/api/v1/crates/tower/0.5.3/download) |
| [tower-http 0.6.11](licenses/dependencies/tower-http-0.6.11/) | MIT | [crate](https://crates.io/api/v1/crates/tower-http/0.6.11/download) |
| [tower-layer 0.3.3](licenses/dependencies/tower-layer-0.3.3/) | MIT | [crate](https://crates.io/api/v1/crates/tower-layer/0.3.3/download) |
| [tower-service 0.3.3](licenses/dependencies/tower-service-0.3.3/) | MIT | [crate](https://crates.io/api/v1/crates/tower-service/0.3.3/download) |
| [tracing 0.1.44](licenses/dependencies/tracing-0.1.44/) | MIT | [crate](https://crates.io/api/v1/crates/tracing/0.1.44/download) |
| [tracing-attributes 0.1.31](licenses/dependencies/tracing-attributes-0.1.31/) | MIT | [crate](https://crates.io/api/v1/crates/tracing-attributes/0.1.31/download) |
| [tracing-core 0.1.36](licenses/dependencies/tracing-core-0.1.36/) | MIT | [crate](https://crates.io/api/v1/crates/tracing-core/0.1.36/download) |
| [tray-icon 0.2.0](licenses/dependencies/tray-icon-0.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/tray-icon/0.2.0/download) |
| [try-lock 0.2.5](licenses/dependencies/try-lock-0.2.5/) | MIT | [crate](https://crates.io/api/v1/crates/try-lock/0.2.5/download) |
| [ttf-parser 0.25.1](licenses/dependencies/ttf-parser-0.25.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/ttf-parser/0.25.1/download) |
| [type-map 0.5.1](licenses/dependencies/type-map-0.5.1/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/type-map/0.5.1/download) |
| [typenum 1.20.1](licenses/dependencies/typenum-1.20.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/typenum/1.20.1/download) |
| [unicase 2.10.0](licenses/dependencies/unicase-2.10.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicase/2.10.0/download) |
| [unicode-bidi 0.3.18](licenses/dependencies/unicode-bidi-0.3.18/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-bidi/0.3.18/download) |
| [unicode-ident 1.0.27](licenses/dependencies/unicode-ident-1.0.27/) | (MIT OR Apache-2.0) AND Unicode-3.0 | [crate](https://crates.io/api/v1/crates/unicode-ident/1.0.27/download) |
| [unicode-normalization 0.1.26](licenses/dependencies/unicode-normalization-0.1.26/) | (MIT OR Apache-2.0) AND Unicode-3.0 | [crate](https://crates.io/api/v1/crates/unicode-normalization/0.1.26/download) |
| [unicode-properties 0.1.4](licenses/dependencies/unicode-properties-0.1.4/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-properties/0.1.4/download) |
| [unicode-script 0.5.8](licenses/dependencies/unicode-script-0.5.8/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-script/0.5.8/download) |
| [unicode-segmentation 1.13.3](licenses/dependencies/unicode-segmentation-1.13.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-segmentation/1.13.3/download) |
| [unicode-vo 0.1.0](licenses/dependencies/unicode-vo-0.1.0/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-vo/0.1.0/download) |
| [unicode-width 0.2.2](licenses/dependencies/unicode-width-0.2.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/unicode-width/0.2.2/download) |
| [untrusted 0.9.0](licenses/dependencies/untrusted-0.9.0/) | ISC | [crate](https://crates.io/api/v1/crates/untrusted/0.9.0/download) |
| [url 2.5.8](licenses/dependencies/url-2.5.8/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/url/2.5.8/download) |
| [urlencoding 2.1.3](licenses/dependencies/urlencoding-2.1.3/) | MIT | [crate](https://crates.io/api/v1/crates/urlencoding/2.1.3/download) |
| [usvg 0.48.1](licenses/dependencies/usvg-0.48.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/usvg/0.48.1/download) |
| [utf8_iter 1.0.4](licenses/dependencies/utf8_iter-1.0.4/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/utf8_iter/1.0.4/download) |
| [uuid 1.28.0](licenses/dependencies/uuid-1.28.0/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/uuid/1.28.0/download) |
| [v_frame 0.3.9](licenses/dependencies/v_frame-0.3.9/) | BSD-2-Clause | [crate](https://crates.io/api/v1/crates/v_frame/0.3.9/download) |
| [version_check 0.9.5](licenses/dependencies/version_check-0.9.5/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/version_check/0.9.5/download) |
| [want 0.3.2](licenses/dependencies/want-0.3.2/) | MIT | [crate](https://crates.io/api/v1/crates/want/0.3.2/download) |
| [wasm-bindgen 0.2.129](licenses/dependencies/wasm-bindgen-0.2.129/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wasm-bindgen/0.2.129/download) |
| [wasm-bindgen-macro 0.2.129](licenses/dependencies/wasm-bindgen-macro-0.2.129/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wasm-bindgen-macro/0.2.129/download) |
| [wasm-bindgen-macro-support 0.2.129](licenses/dependencies/wasm-bindgen-macro-support-0.2.129/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wasm-bindgen-macro-support/0.2.129/download) |
| [wasm-bindgen-shared 0.2.129](licenses/dependencies/wasm-bindgen-shared-0.2.129/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wasm-bindgen-shared/0.2.129/download) |
| [webbrowser 1.2.4](licenses/dependencies/webbrowser-1.2.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/webbrowser/1.2.4/download) |
| [webpki-roots 1.0.10](licenses/dependencies/webpki-roots-1.0.10/) | CDLA-Permissive-2.0 | [crate](https://crates.io/api/v1/crates/webpki-roots/1.0.10/download) |
| [web-time 1.1.0](licenses/dependencies/web-time-1.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/web-time/1.1.0/download) |
| [weezl 0.1.12](licenses/dependencies/weezl-0.1.12/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/weezl/0.1.12/download) |
| [wgpu 25.0.2](licenses/dependencies/wgpu-25.0.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wgpu/25.0.2/download) |
| [wgpu-core 25.0.2](licenses/dependencies/wgpu-core-25.0.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wgpu-core/25.0.2/download) |
| [wgpu-core-deps-windows-linux-android 25.0.0](licenses/dependencies/wgpu-core-deps-windows-linux-android-25.0.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wgpu-core-deps-windows-linux-android/25.0.0/download) |
| [wgpu-hal 25.0.2](licenses/dependencies/wgpu-hal-25.0.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wgpu-hal/25.0.2/download) |
| [wgpu-types 25.0.0](licenses/dependencies/wgpu-types-25.0.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/wgpu-types/25.0.0/download) |
| [winapi 0.3.9](licenses/dependencies/winapi-0.3.9/) | MIT/Apache-2.0 | [crate](https://crates.io/api/v1/crates/winapi/0.3.9/download) |
| [winapi-util 0.1.11](licenses/dependencies/winapi-util-0.1.11/) | Unlicense OR MIT | [crate](https://crates.io/api/v1/crates/winapi-util/0.1.11/download) |
| [windows 0.58.0](licenses/dependencies/windows-0.58.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows/0.58.0/download) |
| [windows 0.61.3](licenses/dependencies/windows-0.61.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows/0.61.3/download) |
| [windows_x86_64_msvc 0.42.2](licenses/dependencies/windows_x86_64_msvc-0.42.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows_x86_64_msvc/0.42.2/download) |
| [windows_x86_64_msvc 0.52.6](licenses/dependencies/windows_x86_64_msvc-0.52.6/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows_x86_64_msvc/0.52.6/download) |
| [windows_x86_64_msvc 0.53.1](licenses/dependencies/windows_x86_64_msvc-0.53.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows_x86_64_msvc/0.53.1/download) |
| [windows-collections 0.2.0](licenses/dependencies/windows-collections-0.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-collections/0.2.0/download) |
| [windows-core 0.58.0](licenses/dependencies/windows-core-0.58.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-core/0.58.0/download) |
| [windows-core 0.61.2](licenses/dependencies/windows-core-0.61.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-core/0.61.2/download) |
| [windows-future 0.2.1](licenses/dependencies/windows-future-0.2.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-future/0.2.1/download) |
| [windows-implement 0.58.0](licenses/dependencies/windows-implement-0.58.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-implement/0.58.0/download) |
| [windows-implement 0.60.2](licenses/dependencies/windows-implement-0.60.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-implement/0.60.2/download) |
| [windows-interface 0.58.0](licenses/dependencies/windows-interface-0.58.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-interface/0.58.0/download) |
| [windows-interface 0.59.3](licenses/dependencies/windows-interface-0.59.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-interface/0.59.3/download) |
| [windows-link 0.1.3](licenses/dependencies/windows-link-0.1.3/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-link/0.1.3/download) |
| [windows-link 0.2.1](licenses/dependencies/windows-link-0.2.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-link/0.2.1/download) |
| [windows-numerics 0.2.0](licenses/dependencies/windows-numerics-0.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-numerics/0.2.0/download) |
| [windows-registry 0.6.1](licenses/dependencies/windows-registry-0.6.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-registry/0.6.1/download) |
| [windows-result 0.2.0](licenses/dependencies/windows-result-0.2.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-result/0.2.0/download) |
| [windows-result 0.3.4](licenses/dependencies/windows-result-0.3.4/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-result/0.3.4/download) |
| [windows-result 0.4.1](licenses/dependencies/windows-result-0.4.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-result/0.4.1/download) |
| [windows-strings 0.1.0](licenses/dependencies/windows-strings-0.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-strings/0.1.0/download) |
| [windows-strings 0.4.2](licenses/dependencies/windows-strings-0.4.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-strings/0.4.2/download) |
| [windows-strings 0.5.1](licenses/dependencies/windows-strings-0.5.1/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-strings/0.5.1/download) |
| [windows-sys 0.42.0](licenses/dependencies/windows-sys-0.42.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-sys/0.42.0/download) |
| [windows-sys 0.52.0](licenses/dependencies/windows-sys-0.52.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-sys/0.52.0/download) |
| [windows-sys 0.59.0](licenses/dependencies/windows-sys-0.59.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-sys/0.59.0/download) |
| [windows-sys 0.60.2](licenses/dependencies/windows-sys-0.60.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-sys/0.60.2/download) |
| [windows-sys 0.61.2](licenses/dependencies/windows-sys-0.61.2/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-sys/0.61.2/download) |
| [windows-targets 0.52.6](licenses/dependencies/windows-targets-0.52.6/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-targets/0.52.6/download) |
| [windows-targets 0.53.5](licenses/dependencies/windows-targets-0.53.5/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-targets/0.53.5/download) |
| [windows-threading 0.1.0](licenses/dependencies/windows-threading-0.1.0/) | MIT OR Apache-2.0 | [crate](https://crates.io/api/v1/crates/windows-threading/0.1.0/download) |
| [winit 0.30.13](licenses/dependencies/winit-0.30.13/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/winit/0.30.13/download) |
| [winreg 0.55.0](licenses/dependencies/winreg-0.55.0/) | MIT | [crate](https://crates.io/api/v1/crates/winreg/0.55.0/download) |
| [writeable 0.6.4](licenses/dependencies/writeable-0.6.4/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/writeable/0.6.4/download) |
| [xml-rs 0.8.29](licenses/dependencies/xml-rs-0.8.29/) | MIT | [crate](https://crates.io/api/v1/crates/xml-rs/0.8.29/download) |
| [xmlwriter 0.1.0](licenses/dependencies/xmlwriter-0.1.0/) | MIT | [crate](https://crates.io/api/v1/crates/xmlwriter/0.1.0/download) |
| [y4m 0.8.0](licenses/dependencies/y4m-0.8.0/) | MIT | [crate](https://crates.io/api/v1/crates/y4m/0.8.0/download) |
| [yoke 0.8.3](licenses/dependencies/yoke-0.8.3/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/yoke/0.8.3/download) |
| [yoke-derive 0.8.4](licenses/dependencies/yoke-derive-0.8.4/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/yoke-derive/0.8.4/download) |
| [zerocopy 0.8.63](licenses/dependencies/zerocopy-0.8.63/) | BSD-2-Clause OR Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/zerocopy/0.8.63/download) |
| [zerocopy-derive 0.8.63](licenses/dependencies/zerocopy-derive-0.8.63/) | BSD-2-Clause OR Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/zerocopy-derive/0.8.63/download) |
| [zerofrom 0.1.8](licenses/dependencies/zerofrom-0.1.8/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/zerofrom/0.1.8/download) |
| [zerofrom-derive 0.1.8](licenses/dependencies/zerofrom-derive-0.1.8/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/zerofrom-derive/0.1.8/download) |
| [zeroize 1.9.1](licenses/dependencies/zeroize-1.9.1/) | Apache-2.0 OR MIT | [crate](https://crates.io/api/v1/crates/zeroize/1.9.1/download) |
| [zerotrie 0.2.5](licenses/dependencies/zerotrie-0.2.5/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/zerotrie/0.2.5/download) |
| [zerovec 0.11.8](licenses/dependencies/zerovec-0.11.8/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/zerovec/0.11.8/download) |
| [zerovec-derive 0.11.6](licenses/dependencies/zerovec-derive-0.11.6/) | Unicode-3.0 | [crate](https://crates.io/api/v1/crates/zerovec-derive/0.11.6/download) |
| [zip 2.4.2](licenses/dependencies/zip-2.4.2/) | MIT | [crate](https://crates.io/api/v1/crates/zip/2.4.2/download) |
| [zlib-rs 0.6.8](licenses/dependencies/zlib-rs-0.6.8/) | Zlib | [crate](https://crates.io/api/v1/crates/zlib-rs/0.6.8/download) |
| [zmij 1.0.23](licenses/dependencies/zmij-1.0.23/) | MIT | [crate](https://crates.io/api/v1/crates/zmij/1.0.23/download) |
| [zopfli 0.8.4](licenses/dependencies/zopfli-0.8.4/) | Apache-2.0 | [crate](https://crates.io/api/v1/crates/zopfli/0.8.4/download) |
| [zune-core 0.5.3](licenses/dependencies/zune-core-0.5.3/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/zune-core/0.5.3/download) |
| [zune-inflate 0.2.54](licenses/dependencies/zune-inflate-0.2.54/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/zune-inflate/0.2.54/download) |
| [zune-jpeg 0.5.15](licenses/dependencies/zune-jpeg-0.5.15/) | MIT OR Apache-2.0 OR Zlib | [crate](https://crates.io/api/v1/crates/zune-jpeg/0.5.15/download) |
