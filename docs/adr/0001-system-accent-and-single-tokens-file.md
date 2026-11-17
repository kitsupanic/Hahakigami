# Follow the macOS accent color, with one tokens file

Colors and type sizes were defined in three overlapping stylesheets (`style.css`, `native.css`, `workspace.css`), so load order decided the result and dark mode ended up with three different accents, one at 2.9:1 contrast. All color and type-size variables now live in `src/tokens.css`, loaded first, and a test rejects `:root` color blocks elsewhere, fonts below 10px, and hard-coded hex in chrome stylesheets. The accent follows the person's macOS accent (`AccentColor`) rather than a fixed app blue, as native Mac apps do; a fixed blue is only the fallback.

## Consequences

- There are two accent tokens: one for fills, with semibold labels so white text clears 3:1, and one for accent-colored text, which falls back to a darker blue (#0064D1, 5.61:1 on white) because system blue (#007AFF, 4.02:1) fails 4.5:1 for regular-weight 12px text.
- Treemap and sunburst colors in `branchColors.ts` are data colors, not chrome, and are exempt from the tokens rule.
- Minimum text is 11px; 10px is allowed only for count badges.
- An opt-in **Guardian theme** (the sidebar Theme swatches or View ▸ Guardian Theme, remembered per Mac) swaps in the website's carton, straw and cord-red palette on a neutral white board (not the site's cream paper), Archivo (bundled, no remote fonts), with the default theme's rounded corners and soft shadows. It lives in `tokens.css` under `:root[data-theme="guardian"]`; the default still follows the macOS accent. Guardian also swaps the data palette for warm print inks (`guardianUsageColors` in `uiFormat.ts`, Guardian colours in `fileTypes.ts`), checked with a palette validator on the white board `#fafaf8` (worst colour-blind pair 7.9, acceptable because tiles carry labels and gaps). Its light mode adds a doodle pattern (`src/guardian/doodles.svg`) printed only where there is no data, on the empty welcome screen and around the sunburst; never on carton or behind treemap tiles.
