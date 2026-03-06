import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();

// Login with dev token
await page.goto('http://localhost:3000/login');
await page.waitForSelector('input', { timeout: 10000 });
const inputs = await page.$$('input');
if (inputs.length > 0) {
  await inputs[0].type('dev-token-12345');
  const buttons = await page.$$('button');
  for (const btn of buttons) {
    const text = await btn.evaluate(el => el.textContent);
    if (text && text.toLowerCase().includes('log')) {
      await btn.click();
      break;
    }
  }
}

await page.waitForNavigation({ waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 3000));

// Navigate to issues list to find an issue with patches
await page.goto('http://localhost:3000/issues', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 2000));

// Desktop screenshot of issues list
await page.setViewport({ width: 1440, height: 900 });
await page.screenshot({ path: '/tmp/issues-list-desktop.png', fullPage: true });

// Try to find and click on an issue that has patches
const issueLinks = await page.$$('a[href*="/issues/"]');
let navigated = false;
for (const link of issueLinks) {
  const href = await link.evaluate(el => el.getAttribute('href'));
  if (href && href.match(/\/issues\/i-/)) {
    await link.click();
    await page.waitForNavigation({ waitUntil: 'networkidle0', timeout: 10000 }).catch(() => {});
    await new Promise(r => setTimeout(r, 3000));
    navigated = true;
    break;
  }
}

if (navigated) {
  // Desktop screenshot of issue detail
  await page.setViewport({ width: 1440, height: 900 });
  await page.screenshot({ path: '/tmp/issue-detail-desktop.png', fullPage: true });

  // Mobile screenshot
  await page.setViewport({ width: 375, height: 812 });
  await new Promise(r => setTimeout(r, 1000));
  await page.screenshot({ path: '/tmp/issue-detail-mobile.png', fullPage: true });
} else {
  console.log('No issue link found, taking current page screenshot');
  await page.screenshot({ path: '/tmp/issue-detail-desktop.png', fullPage: true });
}

// Also try navigating directly to a known mock issue URL
await page.setViewport({ width: 1440, height: 900 });
await page.goto('http://localhost:3000/issues', { waitUntil: 'networkidle0', timeout: 10000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/issues-page-desktop.png', fullPage: true });

await browser.close();
console.log('Screenshots saved!');
