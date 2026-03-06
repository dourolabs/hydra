import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();
await page.setViewport({ width: 1440, height: 900 });

// Capture console logs
const consoleLogs = [];
page.on('console', msg => consoleLogs.push(`${msg.type()}: ${msg.text()}`));
page.on('pageerror', err => consoleLogs.push(`ERROR: ${err.message}`));

// Login
await page.goto('http://localhost:3000/login', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 2000));
const input = await page.$('input');
await input.type('dev-token-12345');
const btn = await page.$('button');
await btn.click();
await new Promise(r => setTimeout(r, 5000));

// Navigate to issue detail
await page.goto('http://localhost:3000/issues/i-seed00002', { waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 8000));

// Check DOM for PatchPreview-related elements
const domCheck = await page.evaluate(() => {
  // Look for the PatchPreview container or any element between description and progress
  const allDivs = document.querySelectorAll('div');
  const descDiv = [...allDivs].find(d => d.className?.includes('description'));
  const progressPanel = [...document.querySelectorAll('[class*="sectionTitle"]')].find(el => el.textContent?.includes('Progress'));

  // Check the issue data from React's internal state
  let patchPreviewContainer = null;
  for (const div of allDivs) {
    if (div.className?.includes('container') && div.parentElement?.className?.includes('detail')) {
      patchPreviewContainer = { className: div.className, children: div.childElementCount, html: div.innerHTML.substring(0, 200) };
    }
  }

  // Look for elements with patchCard class
  const patchCards = document.querySelectorAll('[class*="patchCard"]');
  const patchIdLinks = document.querySelectorAll('[class*="patchIdLink"]');
  const spinners = document.querySelectorAll('[class*="spinner"], [class*="Spinner"]');
  const errors = document.querySelectorAll('[class*="error"]');

  return {
    patchCards: patchCards.length,
    patchIdLinks: patchIdLinks.length,
    spinners: spinners.length,
    errors: [...errors].map(e => e.textContent?.substring(0, 100)),
    patchPreviewContainer,
    descDiv: descDiv ? { className: descDiv.className, nextSibling: descDiv.nextElementSibling?.className } : null,
  };
});
console.log('DOM check:', JSON.stringify(domCheck, null, 2));

// Check network requests for patch data
const networkLogs = [];
page.on('requestfinished', async req => {
  if (req.url().includes('patch')) {
    const resp = req.response();
    networkLogs.push({ url: req.url(), status: resp?.status() });
  }
});

// Reload to capture network requests
await page.reload({ waitUntil: 'networkidle0', timeout: 15000 }).catch(() => {});
await new Promise(r => setTimeout(r, 5000));
console.log('Network patch requests:', JSON.stringify(networkLogs));

// Take scrollable screenshot
await page.screenshot({ path: '/tmp/detail-full-desktop.png', fullPage: true });

// Check console logs for errors
console.log('Console logs:', consoleLogs.filter(l => l.includes('error') || l.includes('Error') || l.includes('patch') || l.includes('Patch')).join('\n'));

await browser.close();
console.log('Done!');
