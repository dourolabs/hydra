# Style

Dark terminal theme — black background, green accent, monospace numerics. All styling goes through CSS Modules backed by the tokens in `packages/ui/src/theme/tokens.css`.

## CSS Modules only

```tsx
// wrong — inline styles bypass the token system
<div style={{ color: "#0f0", padding: 8 }}>…</div>

// wrong — global CSS file outside the module system
import "./issue-detail.css";

// correct
import styles from "./IssueDetail.module.css";
<div className={styles.row}>…</div>
```

No global stylesheets (other than `tokens.css` itself, imported once from `@hydra/ui`).

The `style={...}` prop is reserved for **runtime-dynamic values**: computed from a prop, state, or measurement at render time (e.g. a gradient stop, a drag transform, a CSS custom-property setter `style={{ "--foo": x } as React.CSSProperties}`). The following are NOT runtime-dynamic and must use CSS Modules / theme tokens instead:

- An imported color constant (`style={{ background: CHART_COLORS.created }}`) — use a CSS-var setter pattern that sets the variable in CSS Modules and reads it in the style block.
- A hard-coded token-name string (`style={{ color: "var(--c-accent)" }}`) — set the class instead.

Carve-outs:
- Test files (`__tests__/`, `*.test.tsx`) can use empty `style={{}}` mocks freely — the rule targets shipped components.
- Library-mandated `style` props (e.g. `react-window` `<List style={style}>` passthrough) — the third-party API requires it. Don't substitute; document the carve-out at the call site with a one-line comment.

## Use theme tokens

`packages/ui/src/theme/tokens.css` defines the design vocabulary. Reach for tokens before hard-coding values:

```css
/* wrong */
.row {
  padding: 8px 12px;
  font-family: monospace;
  color: #0f0;
}

/* correct */
.row {
  padding: var(--pad-y) var(--pad-x);
  font-family: var(--f-mono);
  color: var(--color-accent);
}
```

Tokens cover type families/scale, density (`--row-h`, `--pad-x`, `--pad-y`, `--gap`), the legacy `--space-*` ramp, and colour. If something you need isn't there, add a token rather than a one-off literal.

## Mobile

Hydra runs on the same React SPA on desktop and mobile. The rules below are the mobile lens on the rest of this doc — apply them by default when authoring or reviewing CSS modules, not as overrides for a desktop-first design. If something here conflicts with the desktop-shared rules above, mobile wins on layouts that ship at ≤768px.

### Breakpoints

One mobile breakpoint: **768px**. Use `@media (max-width: 768px)` (or `useIsMobile()` from `@hydra/ui`) consistently across modules.

```css
/* correct */
@media (max-width: 768px) {
  .panel { display: none; }
}

/* wrong — ad-hoc per-component breakpoints fragment the responsive surface */
@media (max-width: 1024px) { … }
@media (max-width: 900px)  { … }
```

A second small-mobile breakpoint at 480px is acceptable for genuinely cramped surfaces (chat composer, single-handed reach zones). Don't introduce others.

### Touch targets

All tappable controls render at a minimum **44×44px** on mobile.

- Use `<Button>` from `@hydra/ui` — the primitive enforces the floor below 768px across all variants and sizes. Reach for it before writing a `<button>` directly.
- For non-Button interactives (sidebar rows, table-row buttons, picker triggers, inline icons), apply `min-height: var(--touch-min)`.
- Never hard-code `min-height: 44px` — the token is `--touch-min` (44px on mobile, `unset` on desktop).

If a control is genuinely too small to meet the floor (a dense inline icon next to text), increase its hit area with padding or a transparent `::before` pseudo-element rather than visually enlarging the glyph.

### Safe area

Honor iOS safe areas via tokens. The tokens resolve to `env(safe-area-inset-*)` and are zero on non-notched devices, so the same CSS works everywhere.

- Top: `padding-top: var(--safe-top)` on top-anchored surfaces.
- Bottom: `padding-bottom: var(--safe-bottom)` on bottom-anchored surfaces (tab bar, FAB containers).
- Add `viewport-fit=cover` to the meta viewport tag in the document `<head>` — without it, `env(safe-area-inset-*)` returns 0 even on notched devices.

Apply to anything anchored to a viewport edge: top bar, bottom-tab nav, FABs, sticky composers.

### Mobile chrome budget

Mobile root pages — the landing page of a section (`/`, `/patches`, `/sessions`, `/chat`) — get zero non-essential top chrome. Every layer above the content stacks pixels the user can't get back.

- Suppress `<PageHead>` on mobile list roots. The bottom-tab nav identifies the section; the page name doesn't need to render twice. Keep `<PageHead>` on detail pages where the breadcrumb is the "back" affordance.
- Don't render hamburger menus on mobile. The bottom-tab "More" cell reaches the same drawers.
- Don't render breadcrumbs on mobile list-page roots (the single token is already in the bottom-tab). Keep them on detail pages.
- "Create new" actions belong in a section FAB, not the topbar.

When in doubt: each visible chrome layer must answer "what does the user get from this on a 375px viewport?" before it's allowed to ship.

### Composer / input patterns

Chat-style composers use single-line autogrow textareas with the Send button in the bottom-right corner of the input.

- Start at one line; autogrow up to a small cap (4-6 lines) before scrolling internally.
- The send button lives inside the textarea's bounding box (bottom-right corner), not as a separate row below.
- Don't render a row of secondary actions above or below the composer. Move "End chat" and similar actions into the chat header.

This pattern saves ~100px of vertical space at rest and matches the iMessage / WhatsApp / Slack composer affordance most users expect.

### Composing tappables and pickers

Groups of related pickers (status / project / assignee, view options, filters) `flex-wrap` to a second row when they don't fit — they do **not** switch to `flex-direction: column` on mobile.

```css
/* correct */
.metaRow {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
}

/* wrong — three picker rows stacked vertically reads as a form, not a control row */
@media (max-width: 768px) {
  .metaRow { flex-direction: column; }
}
```

A wrapped second row of compact pickers (~50px) is cheaper than three stacked rows (~150px) and preserves the row's identity as a related control group.

### Overlays vs inline indicators

Don't use transparent floating overlays on cards. If a card needs a secondary affordance (chat icon, status, expand toggle), render it inline in solid color in a content row.

- Solid color (e.g. `var(--acc)`, `var(--fg-1)`), not soft / translucent variants (`var(--acc-soft)`).
- Inline placement in a title row or footer row, not absolutely positioned on top of card content.
- Hover-reveal patterns don't translate to touch. If something is only useful when visible, make it always visible.

Transparent overlays sitting on top of card content read poorly on mobile (text shows through), create tap-target ambiguity (was that tap the card or the overlay?), and become invisible against varied background colors.

### Wrapping & overflow

Long content wraps; mobile root containers clip horizontal overflow defensively.

- Apply `overflow-wrap: anywhere` to any container rendering user-supplied text (issue titles, system event bodies, code snippets, links). The CSS default `normal` won't break unbroken strings (URLs, hashes), and `break-word` is too conservative.
- Add `min-width: 0` to flex children that contain long content. Flex defaults to `min-content`, which refuses to shrink below the longest unbreakable token — the #1 cause of horizontal scroll bugs.
- On mobile root containers (the outermost scroll container of any list page), add `overflow-x: hidden` as belt-and-suspenders. It prevents a single regression from breaking the entire page's horizontal scroll posture.

If you find yourself fighting horizontal scroll: open DevTools, set viewport to 375px, run `document.querySelectorAll('*').forEach(el => { if (el.scrollWidth > el.clientWidth) console.log(el); })` to bisect the offending descendant.

## Don't co-export hooks and components

React Fast Refresh requires each module's exports to be all components or all hooks. Mixing breaks HMR — every edit forces a full reload.

```ts
// wrong — IssueDetail.tsx
export function IssueDetail() { … }
export function useIssue(id: string) { … }   // hook in component file

// correct — colocated but split
// IssueDetail.tsx
export function IssueDetail() { … }

// useIssue.ts (sibling)
export function useIssue(id: string) { … }
```

## Feature module shape

Each feature in `packages/web/src/features/<name>/` keeps the component, its styles, and its data hook together:

```
features/issues/
  IssueDetail.tsx
  IssueDetail.module.css
  useIssue.ts
```

Current features: `activity`, `agents`, `analytics`, `auth`, `chat`, `dashboard`, `documents`, `filters`, `issues`, `labels`, `patches`, `principal`, `projects`, `related`, `repositories`, `search`, `secrets`, `sessions`, `toast`, `triggers`.

When adding a new feature directory under `features/`, also add it to this list — the doc serves as the authoritative inventory.

A hook consumed by only one feature belongs under that `features/<name>/` directory, not `packages/web/src/hooks/`. `hooks/` is reserved for cross-feature hooks (used from two or more features).

## See also

- [packages.md](./packages.md) — `@hydra/ui` component inventory.
- [react-query-and-sse.md](./react-query-and-sse.md) — where the `use<Entity>` hooks fit in.
