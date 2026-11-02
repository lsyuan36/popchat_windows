# Font Awesome Free control icons

Source: [FortAwesome/Font-Awesome](https://github.com/FortAwesome/Font-Awesome),
release **6.7.2**, commit **af620534bfc3c2d4cbefcfeec29603bbe7809e64**.
All 14 icons use the **Free Solid** weight for a consistent control set.

The SVG files and `LICENSE.txt` are unmodified downloads from:
`https://raw.githubusercontent.com/FortAwesome/Font-Awesome/af620534bfc3c2d4cbefcfeec29603bbe7809e64/`
(icons under `svgs/solid/`, license at the repository root).
No fonts, JavaScript, CSS, or upstream executable code are shipped here.

| PopChat control | Official SVG in `solid/` |
| --- | --- |
| Add | plus.svg |
| NewChat | comment-medical.svg |
| History | clock-rotate-left.svg |
| Copy | copy.svg |
| Check | check.svg |
| Fork | code-branch.svg |
| Retry | rotate-right.svg |
| Edit | pen.svg |
| Close | xmark.svg |
| Settings | gear.svg |
| Pin | thumbtack.svg |
| Send | arrow-up.svg |
| Stop | stop.svg |
| Expand | up-right-and-down-left-from-center.svg |

## Attribution and licensing

Font Awesome Free 6.7.2 by [Fonticons, Inc.](https://fontawesome.com),
Copyright 2024 Fonticons, Inc. Icons are licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
See the original `LICENSE.txt` and `NOTICE.md`. The upstream license also covers
code under MIT and fonts under SIL OFL 1.1; this subset contains only SVG icons.
Embedded attribution comments are retained in every SVG.

## Rendering

`src/ui/icons.rs` embeds these SVGs with `include_str!` and uses the existing
`resvg` dependency, not a system or icon font. The only runtime SVG change is
adding a white root fill to produce an RGBA mask; paths and viewBoxes are unchanged.
egui tints that mask with the current control color. No asset changes are made.

Glyphs fit, aspect-preserving and centered, inside a 16-point square in the
existing 28-point circular button. Per-context textures are cached lazily at
32/64/128 pixels, selected by physical pixels per point and capped at 128.
The cache has at most 42 entries regardless of accent or monitor changes,
using at most 1,204,224 RGBA bytes per CPU/GPU copy, freed with the context.
No SVG parsing/rasterization occurs on cache hits. PopChat's bespoke brand mark
is intentionally separate from these Font Awesome control icons.
