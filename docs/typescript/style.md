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

No global stylesheets (other than `tokens.css` itself, imported once from `@hydra/ui`) and no `style={...}` prop except where a value is genuinely dynamic at runtime (e.g. a computed gradient stop).

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

Current features: `activity`, `agents`, `auth`, `chat`, `dashboard`, `documents`, `filters`, `issues`, `labels`, `patches`, `principal`, `related`, `repositories`, `search`, `secrets`, `sessions`, `toast`.

## See also

- [packages.md](./packages.md) — `@hydra/ui` component inventory.
- [react-query-and-sse.md](./react-query-and-sse.md) — where the `use<Entity>` hooks fit in.
