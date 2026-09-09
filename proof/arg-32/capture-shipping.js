const fs = require('fs');
const puppeteer = require('puppeteer-core');
(async () => {
  const edge = 'C:\\\\Program Files (x86)\\\\Microsoft\\\\Edge\\\\Application\\\\msedge.exe';
  const browser = await puppeteer.launch({
    executablePath: edge,
    headless: true,
    defaultViewport: { width: 1440, height: 1100 },
    args: ['--no-sandbox']
  });
  const base = process.argv[2];
  async function shot(pageName, out) {
    const page = await browser.newPage();
    const url = 'http://127.0.0.1:8787/?page=' + pageName + '&days=7';
    await page.goto(url, { waitUntil: 'networkidle0', timeout: 60000 });
    await page.waitForSelector('#content .card, #content .kpis', { timeout: 20000 });
    await new Promise(r => setTimeout(r, 800));
    const title = await page.$eval('#title', el => el.textContent);
    const snippet = await page.$eval('#content', el => el.innerText.slice(0, 200).replace(/\s+/g, ' '));
    console.log(pageName, title, '|', snippet.slice(0, 100));
    await page.screenshot({ path: out, fullPage: true });
    console.log('wrote', out, fs.statSync(out).size);
    await page.close();
  }
  await shot('today', base + '/html-today.png');
  await shot('shipping', base + '/html-shipping.png');
  await shot('settings', base + '/html-settings.png');
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });
