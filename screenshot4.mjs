import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();
await page.setViewport({ width: 1440, height: 900 });

// Login
await page.goto('http://localhost:3000/login', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 2000));
const input = await page.$('input');
await input.type('dev-token-12345');
const button = await page.$('button');
await button.click();
await new Promise(r => setTimeout(r, 5000));

// We're on the dashboard. Click on an issue to see detail
// First click on "Add dark mode support" which is an active issue
const issueRow = await page.evaluate(() => {
  const elements = [...document.querySelectorAll('*')];
  for (const el of elements) {
    if (el.textContent?.includes('Add dark mode support') && el.tagName !== 'BODY' && el.tagName !== 'HTML' && el.tagName !== 'DIV') {
      return { found: true, tag: el.tagName, text: el.textContent.substring(0, 100) };
    }
  }
  return { found: false };
});
console.log('Issue row:', JSON.stringify(issueRow));

// Click on the issue
await page.evaluate(() => {
  const rows = document.querySelectorAll('[class*="row"], [class*="Row"], [class*="issue"], [class*="Issue"], tr, li');
  for (const row of rows) {
    if (row.textContent?.includes('Add dark mode support')) {
      row.click();
      return true;
    }
  }
  return false;
});
await new Promise(r => setTimeout(r, 3000));
console.log('After click URL:', page.url());

// Take desktop screenshot
await page.screenshot({ path: '/tmp/issue-detail-desktop.png', fullPage: true });

// Check if the current page shows patches
const pageText = await page.evaluate(() => document.body?.innerText?.substring(0, 3000));
console.log('Detail page text:', pageText);

// Check for PatchPreview elements
const hasPatchPreview = await page.evaluate(() => {
  return document.querySelector('[class*="patchCard"]') !== null ||
         document.querySelector('[class*="PatchPreview"]') !== null ||
         document.body?.innerHTML?.includes('patchCard') === true;
});
console.log('Has PatchPreview:', hasPatchPreview);

// Try to get mock server issue data
const mockResponse = await page.evaluate(async () => {
  const res = await fetch('/api/v1/issues?inbox=true', { method: 'GET' });
  return { status: res.status, body: await res.text().catch(() => 'err') };
});
console.log('Mock API response:', JSON.stringify(mockResponse).substring(0, 500));

// Mobile screenshot
await page.setViewport({ width: 375, height: 812 });
await new Promise(r => setTimeout(r, 1000));
await page.screenshot({ path: '/tmp/issue-detail-mobile.png', fullPage: true });

await browser.close();
console.log('Done!');
