# Fonts

Scratch's costume editor offers the families `Sans Serif`, `Serif`, `Handwriting`, `Marker`, `Curly` and `Pixel`. scrust embeds OFL-licensed fonts for them (`crates/scrust-render/fonts`, licences alongside) and rewrites the family names in costume SVGs before parsing:

| Scratch family | Embedded font | Same as Scratch? |
|---|---|---|
| Sans Serif | Noto Sans | yes |
| Serif | Source Serif 4 | successor of Source Serif Pro |
| Handwriting | Handlee | yes |
| Marker | Knewave | yes |
| Curly | Griffy | yes |
| Pixel | Noto Sans (fallback) | no: Grand9K Pixel is not available under a licence that allows bundling it here |

Say/think bubbles, monitors and the ask prompt use Noto Sans (Scratch uses Helvetica Neue). On native targets system fonts are loaded in addition, as fallback for scripts the embedded fonts do not cover.
