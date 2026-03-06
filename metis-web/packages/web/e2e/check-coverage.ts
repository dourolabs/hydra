/**
 * Scenario coverage check: validates every required scenario in scenarios.md
 * has at least one corresponding tagged test in e2e/tests/**\/*.spec.ts.
 *
 * Scenarios marked with `(planned)` are excluded from the check.
 *
 * Usage: npx tsx e2e/check-coverage.ts
 */
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

const E2E_DIR = resolve(import.meta.dirname);
const SCENARIOS_PATH = join(E2E_DIR, "scenarios.md");
const TESTS_DIR = join(E2E_DIR, "tests");

// 1. Parse scenarios.md for all @tag:name entries (skip planned ones)
function extractScenarios(content: string): Map<string, string> {
  const scenarios = new Map<string, string>();
  for (const line of content.split("\n")) {
    // Match lines like: - `@auth:login` — Description
    const match = line.match(/^-\s+`(@[\w:.-]+)`\s*[—–-]\s*(.+)/);
    if (!match) continue;
    // Skip scenarios marked as (planned)
    if (/\(planned\)/.test(line)) continue;
    scenarios.set(match[1], match[2].trim());
  }
  return scenarios;
}

// 2. Scan test files for tag annotations
function collectTestTags(testsDir: string): Set<string> {
  const tags = new Set<string>();
  const files = readdirSync(testsDir).filter((f) => f.endsWith(".spec.ts"));
  for (const file of files) {
    const content = readFileSync(join(testsDir, file), "utf-8");
    for (const match of content.matchAll(/@[\w:.-]+/g)) {
      tags.add(match[0]);
    }
  }
  return tags;
}

// 3. Check coverage
const scenarios = extractScenarios(readFileSync(SCENARIOS_PATH, "utf-8"));
const testTags = collectTestTags(TESTS_DIR);

const uncovered: Array<{ tag: string; description: string }> = [];
for (const [tag, description] of scenarios) {
  if (!testTags.has(tag)) {
    uncovered.push({ tag, description });
  }
}

// 4. Report results
const total = scenarios.size;
const covered = total - uncovered.length;

if (uncovered.length > 0) {
  console.error(`\nScenario coverage check FAILED\n`);
  console.error(
    `${covered}/${total} scenarios covered. Missing ${uncovered.length}:\n`
  );
  for (const { tag, description } of uncovered) {
    console.error(`  ${tag} — ${description}`);
  }
  console.error(
    `\nAdd tests with the above tags, or mark scenarios as (planned) in scenarios.md.\n`
  );
  process.exit(1);
} else {
  console.log(`\nScenario coverage check passed: ${covered}/${total} scenarios covered.\n`);
}
