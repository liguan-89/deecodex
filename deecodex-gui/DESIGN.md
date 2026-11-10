# deecodex GUI DESIGN.md

## Visual Theme

deecodex GUI uses a **Frosted Console** design language for light mode: a macOS-like light canvas, Linear-style engineering density, and Raycast-like command-panel controls. The app is a desktop operations console, not a marketing page. Screens should feel compact, calm, precise, and repeatedly usable.

## Core Principles

- Use light translucent surfaces over a quiet `#f5f5f7` canvas.
- Prefer 8px radius for controls, cards, tables, panels, and grouped inputs.
- Use red-orange only for active state, primary actions, and warnings. Do not scatter it decoratively.
- Keep shadows subtle. Hairline borders matter more than glow.
- Use system sans for labels and UI chrome; use monospace only for IDs, URLs, tokens, model names, keys, logs, and code.
- Combine related controls into groups. Do not let short fields occupy entire rows.
- Tables and repeated cards should be dense but readable.
- Avoid uppercase labels in Chinese UI unless the content is a literal protocol/model name.

## Color Roles

- Canvas: `#f5f5f7`
- Surface: translucent white, usually `rgba(255,255,255,0.62)` to `0.82`
- Border: `rgba(29,29,31,0.10-0.18)`
- Text primary: `#1d1d1f`
- Text secondary: `#515154`
- Text muted: `#86868b`
- Accent: `#d94f3a`
- Success: `#2d8a5c`
- Warning: `#c77820`

## Components

- Buttons: 34-36px high, 8px radius, system sans, semibold, no uppercase transform.
- Inputs/selects/textareas: 34px minimum height, 8px radius, white translucent fill, fine border, 3px soft focus ring.
- Cards: 8px radius, fine border, very soft shadow. Active cards use a subtle accent ring.
- Sidebar navigation: unified SVG line icons, 36px item height, active red accent, no character-symbol placeholders.
- Tags/badges: 6px radius, 11px system sans, soft border and translucent fill.
- Section headers: title and description stack vertically; do not force a side-by-side layout when it compresses form fields.

## Anti-Patterns

- No oversized card layouts for one account or one item.
- No random symbolic glyphs as icons.
- No repeated explanatory copy in the same viewport.
- No neon glow in light mode.
- No broad red outlines unless it is a truly selected or active state.
- No form controls squeezed into a half-width container by accidental nesting.
