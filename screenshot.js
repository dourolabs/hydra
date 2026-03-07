const puppeteer = require('puppeteer');

(async () => {
  const browser = await puppeteer.launch({
    headless: true,
    args: ['--no-sandbox', '--disable-setuid-sandbox'],
  });

  // Login first
  const page = await browser.newPage();
  await page.goto('http://localhost:3000/login');
  await page.waitForSelector('input', { timeout: 10000 });
  await page.type('input', 'dev-token-12345');
  await page.keyboard.press('Enter');
  await page.waitForNavigation({ waitUntil: 'networkidle2', timeout: 15000 }).catch(() => {});
  await new Promise(r => setTimeout(r, 2000));

  // Desktop viewport - dashboard
  await page.setViewport({ width: 1440, height: 900 });
  await page.goto('http://localhost:3000/', { waitUntil: 'networkidle2', timeout: 15000 });
  await new Promise(r => setTimeout(r, 2000));
  await page.screenshot({ path: '/tmp/.tmpm6Q5Sx/repo/screenshot-desktop-dashboard.png', fullPage: true });
  console.log('Desktop dashboard screenshot saved');

  // Mobile viewport - dashboard
  await page.setViewport({ width: 375, height: 812 });
  await page.goto('http://localhost:3000/', { waitUntil: 'networkidle2', timeout: 15000 });
  await new Promise(r => setTimeout(r, 2000));
  await page.screenshot({ path: '/tmp/.tmpm6Q5Sx/repo/screenshot-mobile-dashboard.png', fullPage: true });
  console.log('Mobile dashboard screenshot saved');

  await browser.close();
})();
