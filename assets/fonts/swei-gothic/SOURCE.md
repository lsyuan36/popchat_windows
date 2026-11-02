# Bundled Swei Gothic TC Regular

- Project: 獅尾圓體 / Swei Gothic, https://github.com/max32002/swei-gothic
- Release/tag: `2.143` (font name-table version: `Version 2.143`)
- Pinned full commit: `c363a1d845aa782a71b7a068b5db0aff3f83ee86`
- Upstream file: `CJK TC/SweiGothicCJKtc-Regular.ttf`
- Download: https://raw.githubusercontent.com/max32002/swei-gothic/c363a1d845aa782a71b7a068b5db0aff3f83ee86/CJK%20TC/SweiGothicCJKtc-Regular.ttf
- Local file: `SweiGothicCJKtc-Regular.ttf`
- Size: **16,193,916 bytes** (about 15.44 MiB)
- SHA-256: `0cb199c63ec23fda70905decf82e1a7f3657092a653323063e9e5f50adfc3f7f`
- Git blob SHA-1: `4a1140e6edd39e72efeb0c9aacc3356f79b4caa9`
- License: SIL Open Font License 1.1; see `SIL_Open_Font_License_1.1.txt`.
- Original license size: 4,302 bytes; Git blob SHA-1: `77b17316cf1e8ab216bd6743996dfc67cc86216a` (verified byte-for-byte).
- Copyright and upstream credits: see `COPYRIGHT.txt` and the original TTF name table.

## Provenance verification

The GitHub API tag listing and commit endpoint confirmed that tag `2.143`
resolves to the pinned commit above. Its recursive Git tree identifies the TC
Regular binary with the size and blob SHA above. The downloaded binary's
SHA-256 is checked by the font tests. The TTF name table confirms version 2.143
and the copyright/license notice transcribed in `COPYRIGHT.txt`.

- Tags: https://api.github.com/repos/max32002/swei-gothic/tags
- Commit: https://api.github.com/repos/max32002/swei-gothic/commits/c363a1d845aa782a71b7a068b5db0aff3f83ee86
- Tree: https://api.github.com/repos/max32002/swei-gothic/git/trees/c363a1d845aa782a71b7a068b5db0aff3f83ee86?recursive=1
- Original license: https://raw.githubusercontent.com/max32002/swei-gothic/c363a1d845aa782a71b7a068b5db0aff3f83ee86/SIL_Open_Font_License_1.1.txt
- Upstream credits: https://github.com/max32002/swei-gothic/blob/c363a1d845aa782a71b7a068b5db0aff3f83ee86/README.md#著作權與授權

## Integration

Only the standard **TC Regular** weight is bundled to avoid shipping a second
large asset. TC uses Traditional Chinese regional glyph forms. Existing manually
selected installed fonts, including DemiLight, are not migrated or overwritten.

The original TTF is unmodified and embedded with Rust `include_bytes!`, so the
Chinese default needs neither a system-font installation nor network access.
At runtime, the existing cmap partition helper makes a cached in-memory copy
with only CJK mappings, preserving independent English and monospace choices.
It does not rewrite this asset, its outlines, or its copyright/license metadata.

The saved empty Chinese preference means the bundled default. The explicit
selection ID is `bundled:swei-gothic-tc`. An incomplete custom Chinese face still
falls back to this bundled font. Empty English preferences prefer the installed
TT Norms Pro regular face, then Arial, with egui only as a final fallback if both
are unavailable. Neither English font is bundled with this application.

Distributions should include both `SIL_Open_Font_License_1.1.txt` and
`COPYRIGHT.txt` with the application (and may include this provenance file).
The font remains under OFL 1.1, not the application's source-code license.
