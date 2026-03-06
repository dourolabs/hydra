import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();
await page.setViewport({ width: 1440, height: 900 });

// Go to login page
await page.goto('http://localhost:3000/login', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 2000));

// Fill in the token and submit
const input = await page.$('input');
await input.type('dev-token-12345');
const button = await page.$('button');
await button.click();

// Wait for navigation (SPA redirect, may not trigger network navigation)
await new Promise(r => setTimeout(r, 5000));
console.log('After login URL:', page.url());
await page.screenshot({ path: '/tmp/after-login.png', fullPage: true });

const bodyText = await page.evaluate(() => document.body?.innerText?.substring(0, 2000));
console.log('Body text:', bodyText);

// Try to navigate to an issue page
// First check what mock issues exist
const response = await page.evaluate(async () => {
  const res = await fetch('/api/v1/issues/search', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({})
  });
  return { status: res.status, body: await res.text().catch(() => 'no body') };
});
console.log('Issues search response:', JSON.stringify(response).substring(0, 500));

// Navigate to issues list
await page.goto('http://localhost:3000/issues', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 3000));
console.log('Issues URL:', page.url());
await page.screenshot({ path: '/tmp/issues-list.png', fullPage: true });

// Find an issue link and click it
const issueLinks = await page.$$eval('a[href*="/issues/"]', links =>
  links.map(l => ({ href: l.getAttribute('href'), text: l.textContent })).slice(0, 5)
);
console.log('Issue links found:', JSON.stringify(issueLinks));

if (issueLinks.length > 0) {
  const firstIssueHref = issueLinks[0].href;
  await page.goto(`http://localhost:3000${firstIssueHref}`, { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
  await new Promise(r => setTimeout(r, 3000));

  // Desktop screenshot
  await page.screenshot({ path: '/tmp/issue-detail-desktop.png', fullPage: true });

  // Mobile screenshot
  await page.setViewport({ width: 375, height: 812 });
  await new Promise(r => setTimeout(r, 1000));
  await page.screenshot({ path: '/tmp/issue-detail-mobile.png', fullPage: true });

  console.log('Issue detail screenshots saved!');
}

await browser.close();
console.log('Done!');
