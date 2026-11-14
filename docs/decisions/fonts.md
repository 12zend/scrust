# Fonts

Scratch's costume editor offers the families `Sans Serif`, `Serif`, `Handwriting`, `Marker`, `Curly` and `Pixel`. scrust embeds OFL-licensed fonts for them (`crates/scrust-render/fonts`, licences alongside) and rewrites the family names in costume SVGs before parsing:

| Scratch family | Embedded font | Same as Scratch? |
|---|---|---|
| Sans Serif | Noto Sans Medium (`NotoSans-Medium.ttf`, converted from scratch-render-fonts' woff2 and renamed "Scratch Sans Serif") | yes |
| Serif | Source Serif 4 | successor of Source Serif Pro |
| Handwriting | Handlee | yes |
| Marker | Knewave | yes |
| Curly | Griffy | yes |
| Pixel | Noto Sans (fallback) | no: Grand9K Pixel is not available under a licence that allows bundling it here |

Family lists such as `"Arial Bold Rounded", Sans Serif` are mapped item by item. Say/think bubbles follow scratch-render's `TextBubbleSkin` (shape, stroke, wrapping at 170 px measured with the font's advances, `14px Helvetica, sans-serif`, Helvetica being a system font on macOS); monitors and the ask prompt use Noto Sans. macOS's LastResort font is removed from the font database: as a fallback it turned whole text runs into boxes. On native targets system fonts are loaded in addition, as fallback for scripts the embedded fonts do not cover.
