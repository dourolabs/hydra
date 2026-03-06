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

// Navigate to an issue with patches via the URL pattern
// i-seed00002 has patches - try navigating directly
await page.goto('http://localhost:3000/?selected=i-seed00002', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 3000));
console.log('URL:', page.url());

let bodyText = await page.evaluate(() => document.body?.innerText?.substring(0, 3000));
console.log('Page text:', bodyText?.substring(0, 500));

// Take screenshot
await page.screenshot({ path: '/tmp/issue-with-patches-desktop.png', fullPage: true });

// Check for patch preview elements
const hasPatchPreview = await page.evaluate(() => {
  const html = document.body?.innerHTML || '';
  return {
    hasPatchCard: html.includes('patchCard'),
    hasPatchPreview: html.includes('PatchPreview') || html.includes('patchPreview'),
    hasDiffViewer: html.includes('DiffViewer') || html.includes('diffViewer'),
    hasSpinner: html.includes('Spinner') || html.includes('spinner'),
    hasPatchId: html.includes('p-seed'),
  };
});
console.log('Patch elements:', JSON.stringify(hasPatchPreview));

// Try clicking on "Everything" tab to see all issues, then click on one with patches
await page.goto('http://localhost:3000/', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 2000));

// Click "Everything"
await page.evaluate(() => {
  const links = [...document.querySelectorAll('a, button, [role="tab"], [class*="nav"]')];
  for (const el of links) {
    if (el.textContent?.trim() === 'Everything') {
      el.click();
      return true;
    }
  }
  return false;
});
await new Promise(r => setTimeout(r, 3000));

// Now find "Migrate authentication to OAuth2" which has patches
const clickResult = await page.evaluate(() => {
  const allElements = [...document.querySelectorAll('*')];
  for (const el of allElements) {
    if (el.textContent?.includes('Migrate authentication') &&
        (el.tagName === 'SPAN' || el.tagName === 'P' || el.tagName === 'A')) {
      el.click();
      return { clicked: true, tag: el.tagName, text: el.textContent.substring(0, 100) };
    }
  }
  return { clicked: false };
});
console.log('Click result:', JSON.stringify(clickResult));
await new Promise(r => setTimeout(r, 3000));
console.log('URL after click:', page.url());
await page.screenshot({ path: '/tmp/issue-oauth-desktop.png', fullPage: true });

bodyText = await page.evaluate(() => document.body?.innerText?.substring(0, 3000));
console.log('OAuth page text:', bodyText?.substring(0, 500));

// Mobile viewport
await page.setViewport({ width: 375, height: 812 });
await new Promise(r => setTimeout(r, 1000));
await page.screenshot({ path: '/tmp/issue-oauth-mobile.png', fullPage: true });

await browser.close();
console.log('Done!');
