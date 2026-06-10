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
