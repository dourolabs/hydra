import puppeteer from '/home/worker/.nvm/versions/node/v22.22.1/lib/node_modules/puppeteer/lib/esm/puppeteer/puppeteer.js';

const browser = await puppeteer.launch({ headless: true, args: ['--no-sandbox'] });
const page = await browser.newPage();

// Go to the root and see what we get
await page.setViewport({ width: 1440, height: 900 });
await page.goto('http://localhost:3000', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 3000));
await page.screenshot({ path: '/tmp/app-root.png', fullPage: true });
console.log('Current URL:', page.url());

// Check for login form
const pageContent = await page.content();
const hasLogin = pageContent.includes('login') || pageContent.includes('Login') || pageContent.includes('token') || pageContent.includes('Token');
console.log('Has login elements:', hasLogin);

// Try to find any input or form
const inputs = await page.$$('input');
console.log('Number of inputs:', inputs.length);
const buttons = await page.$$('button');
console.log('Number of buttons:', buttons.length);

// Log all text on page
const bodyText = await page.evaluate(() => document.body?.innerText?.substring(0, 1000));
console.log('Page text:', bodyText);

// Try auth via cookie - set auth cookie and navigate
await page.setCookie({
  name: 'metis_token',
  value: 'dev-token-12345',
  domain: 'localhost',
  path: '/',
});

await page.goto('http://localhost:3000', { waitUntil: 'networkidle0', timeout: 15000 });
await new Promise(r => setTimeout(r, 3000));
await page.screenshot({ path: '/tmp/app-with-cookie.png', fullPage: true });
console.log('After cookie URL:', page.url());

const bodyText2 = await page.evaluate(() => document.body?.innerText?.substring(0, 1000));
console.log('After cookie text:', bodyText2);

await browser.close();
