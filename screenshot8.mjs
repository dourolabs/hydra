import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();
await page.setViewport({ width: 1440, height: 900 });

// Login
await page.goto('http://localhost:3000/login', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 2000));
const input = await page.$('input');
await input.type('dev-token-12345');
const btn = await page.$('button');
await btn.click();
await new Promise(r => setTimeout(r, 5000));

// Intercept API responses to check issue data
page.on('response', async response => {
  if (response.url().includes('/api/v1/issues/i-seed00002')) {
    try {
      const json = await response.json();
      console.log('API response patches:', JSON.stringify(json?.issue?.patches));
      console.log('API response keys:', Object.keys(json?.issue || {}));
    } catch {}
  }
});

await page.goto('http://localhost:3000/issues/i-seed00002', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 8000));

// Use React DevTools to inspect the component state
const reactState = await page.evaluate(() => {
  // Find the IssueDetail fiber node
  const rootEl = document.getElementById('root');
  const fiberKey = Object.keys(rootEl || {}).find(k => k.startsWith('__reactFiber'));
  if (!fiberKey) return { error: 'no fiber found' };

  // Traverse the fiber tree to find issue data
  let fiber = rootEl[fiberKey];
  let issueData = null;
  let depth = 0;
  const visited = new Set();

  function traverse(node) {
    if (!node || depth > 100 || visited.has(node)) return;
    visited.add(node);
    depth++;

    // Check memoizedProps for issue data
    const props = node.memoizedProps;
    if (props?.record?.issue?.patches) {
      issueData = {
        patches: props.record.issue.patches,
        issueId: props.record.issue_id,
      };
      return;
    }

    if (node.child) traverse(node.child);
    if (!issueData && node.sibling) traverse(node.sibling);
  }

  traverse(fiber);
  return issueData || { error: 'no issue data found in fiber tree' };
});
console.log('React state:', JSON.stringify(reactState));

await browser.close();
console.log('Done!');
