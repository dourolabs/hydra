const puppeteer = require('puppeteer');

(async () => {
  const browser = await puppeteer.launch({
    headless: true,
    args: ['--no-sandbox', '--disable-setuid-sandbox'],
  });

  const page = await browser.newPage();
  await page.goto('http://localhost:3000/login');
  await page.waitForSelector('input', { timeout: 10000 });
  await page.type('input', 'dev-token-12345');
  await page.keyboard.press('Enter');
  await page.waitForNavigation({ waitUntil: 'networkidle2', timeout: 15000 }).catch(() => {});
  await new Promise(r => setTimeout(r, 2000));

  // My Issues page - desktop
  await page.setViewport({ width: 1440, height: 900 });
  await page.goto('http://localhost:3000/my-issues', { waitUntil: 'networkidle2', timeout: 15000 });
  await new Promise(r => setTimeout(r, 2000));
  await page.screenshot({ path: '/tmp/.tmpm6Q5Sx/repo/screenshot-my-issues-desktop.png', fullPage: true });
  console.log('My Issues desktop screenshot saved');

  // Everything page - desktop
  await page.goto('http://localhost:3000/everything', { waitUntil: 'networkidle2', timeout: 15000 });
  await new Promise(r => setTimeout(r, 2000));
  await page.screenshot({ path: '/tmp/.tmpm6Q5Sx/repo/screenshot-everything-desktop.png', fullPage: true });
  console.log('Everything desktop screenshot saved');

  // My Issues page - mobile
  await page.setViewport({ width: 375, height: 812 });
  await page.goto('http://localhost:3000/my-issues', { waitUntil: 'networkidle2', timeout: 15000 });
  await new Promise(r => setTimeout(r, 2000));
  await page.screenshot({ path: '/tmp/.tmpm6Q5Sx/repo/screenshot-my-issues-mobile.png', fullPage: true });
  console.log('My Issues mobile screenshot saved');

  await browser.close();
})();
