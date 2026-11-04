// Use the existing portal Playwright installation, or pass its module path for a
// worktree. No browser package or production dependency is installed here.
import { mkdir, writeFile } from 'node:fs/promises';
const { chromium } = await import(process.env.OMOBA_PLAYWRIGHT_MODULE || '../../omoba-web/node_modules/playwright/index.mjs');
const url = process.env.OMOBA_BROWSER_URL || 'http://127.0.0.1:3186/';
const output = process.env.OMOBA_BROWSER_EVIDENCE || '.agent/tasks/BROWSER-PRACTICE-20261028/raw';
await mkdir(output, {recursive: true});
const browser = await chromium.launch({ channel: 'chrome', headless: true, args: ['--enable-unsafe-swiftshader'] });
const page = await browser.newPage({viewport: {width: 1280, height: 720}, locale: 'en-US'});
const errors = [];
const messages = [];
page.on('pageerror', error => errors.push(error.message));
page.on('console', message => { if (['error','warning'].includes(message.type())) messages.push({type:message.type(),text:message.text()}); });
const state = () => page.evaluate(() => {
  const game = window.__omobaPractice;
  return {frames:game.omoba_practice_frames(),actions:game.omoba_practice_actions(),accepted:game.omoba_practice_accepted_actions(),mana:game.omoba_practice_mana(),x:game.omoba_practice_x(),z:game.omoba_practice_z(),round:game.omoba_practice_round()};
});
try {
  await page.goto(url, {waitUntil:'domcontentloaded'});
  await page.waitForFunction(() => window.__omobaPractice?.omoba_practice_frames?.() > 10, null, {timeout:90000});
  const before = await state();
  await page.locator('canvas').click({position:{x:640,y:350}});
  await page.keyboard.down('ArrowUp');
  await page.waitForTimeout(700);
  await page.keyboard.up('ArrowUp');
  await page.keyboard.down('ArrowRight');
  await page.waitForTimeout(700);
  await page.keyboard.up('ArrowRight');
  const moved = await state();
  if (Math.hypot(moved.x-before.x,moved.z-before.z) < 0.5 || moved.frames<=before.frames) throw Error('Actual simulation movement/frame progression was not observed');
  await page.mouse.move(850,270);
  for (const key of ['q','w','e','r','f','Space']) await page.keyboard.press(key,{delay:100});
  await page.waitForFunction(frames=>window.__omobaPractice.omoba_practice_frames()>frames+2,moved.frames);
  const combat = await state();
  if (combat.actions <= moved.actions || combat.accepted <= moved.accepted) throw Error('Combat inputs were not accepted by the authority');
  await page.keyboard.press('Escape',{delay:100});
  await page.waitForTimeout(150);
  const paused = await state();
  await page.keyboard.down('ArrowRight');await page.waitForTimeout(350);await page.keyboard.up('ArrowRight');
  const still = await state();
  if (Math.hypot(still.x-paused.x,still.z-paused.z)>0.01) throw Error('Pause failed to stop movement');
  await page.keyboard.press('Escape',{delay:100});
  await page.keyboard.press('n',{delay:100});
  await page.waitForFunction(round=>window.__omobaPractice.omoba_practice_round()>round,combat.round);
  const restarted = await state();
  await page.keyboard.press('3',{delay:100});
  await page.waitForFunction(round=>window.__omobaPractice.omoba_practice_round()>round,restarted.round);
  const selected = await state();
  await page.screenshot({path:output+'/browser-practice-1280x720.png'});
  const result={viewport:'1280x720',locale:'en-US',url,before,moved,combat,paused,still,restarted,selected,errors,messages};
  await writeFile(output+'/browser-smoke.json',JSON.stringify(result,null,2)+'\n');
  if (errors.length) throw Error(errors.join('\n'));
  console.log(JSON.stringify(result,null,2));
} catch(error) {
  await page.screenshot({path:output+'/browser-failure-1280x720.png'}).catch(()=>{});
  await writeFile(output+'/browser-failure.json',JSON.stringify({error:error.message,errors,messages},null,2)+'\n');
  throw error;
} finally { await browser.close(); }
