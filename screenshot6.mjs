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

// Navigate to issue detail page for issue with patches
await page.goto('http://localhost:3000/issues/i-seed00002', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 5000));
console.log('URL:', page.url());

let bodyText = await page.evaluate(() => document.body?.innerText?.substring(0, 3000));
console.log('Page text:', bodyText?.substring(0, 1000));

// Desktop screenshot
await page.screenshot({ path: '/tmp/detail-desktop.png', fullPage: true });

// Check for PatchPreview
const patchInfo = await page.evaluate(() => {
  const html = document.body?.innerHTML || '';
  return {
    hasPatchCard: html.includes('patchCard'),
    hasPatchIdLink: html.includes('patchIdLink'),
    hasDiffViewer: html.includes('diffViewer') || html.includes('DiffViewer'),
    hasSpinner: html.includes('spinner'),
  };
});
console.log('Patch elements:', JSON.stringify(patchInfo));

// Mobile screenshot
await page.setViewport({ width: 375, height: 812 });
await new Promise(r => setTimeout(r, 1000));
await page.screenshot({ path: '/tmp/detail-mobile.png', fullPage: true });

// Also check issue with patches: i-seed00006 (rate limiting)
await page.setViewport({ width: 1440, height: 900 });
await page.goto('http://localhost:3000/issues/i-seed00006', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 5000));
await page.screenshot({ path: '/tmp/detail-ratelimit-desktop.png', fullPage: true });

await browser.close();
console.log('Done!');
