import puppeteer from '/home/worker/.nvm/versions/node/v22.22.2/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();

// Set auth cookie directly
await page.setCookie({
  name: 'hydra_token',
  value: 'dev-token-12345',
  domain: 'localhost',
  path: '/',
});

const screenshots = [];

// Desktop viewport
await page.setViewport({ width: 1440, height: 900 });

// Dashboard page
await page.goto('http://localhost:3000/', { waitUntil: 'networkidle2', timeout: 30000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/.tmptnXT93/repo/desktop-dashboard.png', fullPage: false });
screenshots.push('desktop-dashboard.png');
console.log('Dashboard URL:', page.url());

// Issue detail page
await page.goto('http://localhost:3000/issues/i-001', { waitUntil: 'networkidle2', timeout: 30000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/.tmptnXT93/repo/desktop-issue.png', fullPage: false });
screenshots.push('desktop-issue.png');

// Settings page
await page.goto('http://localhost:3000/settings', { waitUntil: 'networkidle2', timeout: 30000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/.tmptnXT93/repo/desktop-settings.png', fullPage: false });
screenshots.push('desktop-settings.png');

// Mobile viewport
await page.setViewport({ width: 375, height: 812 });

await page.goto('http://localhost:3000/', { waitUntil: 'networkidle2', timeout: 30000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/.tmptnXT93/repo/mobile-dashboard.png', fullPage: false });
screenshots.push('mobile-dashboard.png');

await page.goto('http://localhost:3000/issues/i-001', { waitUntil: 'networkidle2', timeout: 30000 });
await new Promise(r => setTimeout(r, 2000));
await page.screenshot({ path: '/tmp/.tmptnXT93/repo/mobile-issue.png', fullPage: false });
screenshots.push('mobile-issue.png');

await browser.close();
console.log('Screenshots taken:', screenshots.join(', '));
