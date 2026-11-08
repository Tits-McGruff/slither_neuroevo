/** Optional real-browser regression run. Requires an explicitly disposable server.
 * Uses the Chromium/Puppeteer already installed for Mermaid; no test framework is added.
 */
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
const puppeteer = createRequire(import.meta.resolve('@mermaid-js/mermaid-cli'))('puppeteer');
const url = process.argv.find(arg => arg.startsWith('--url='))?.slice(6);
if (!url || !process.argv.includes('--disposable')) throw new Error('Use --url=http://…/?server=ws://… --disposable. This test changes the run and imports a small brain fixture. Never target owner state.');
const output = path.resolve(process.env.WEBUI_SCREENSHOTS ?? '.cache/webui-review/screenshots');
await mkdir(output, {recursive:true});
const browser = await puppeteer.launch({headless:'new',executablePath:process.env.WEBUI_BROWSER});
const page = await browser.newPage();
const errors = []; const consoleErrors = []; const commands = []; const checks = []; const metrics = {};
metrics.failedResponses=[];
page.on('response',response=>{if(response.status()>=400)metrics.failedResponses.push({url:response.url(),status:response.status()});});
page.on('pageerror', error => errors.push(error.message));
page.on('console', message => {if(message.type()==='error') consoleErrors.push(message.text());});
await page.evaluateOnNewDocument(()=>{
  // Observe real transports, solely to interrupt this test's connection later.
  const NativeWebSocket = window.WebSocket;
  window.reviewSockets = [];
  window.WebSocket = class extends NativeWebSocket {
    constructor(...args) { super(...args); window.reviewSockets.push(this); }
  };
});
const cdp = await page.target().createCDPSession();
await cdp.send('Network.enable');
cdp.on('Network.webSocketFrameSent', ({response}) => { try { commands.push(JSON.parse(response.payloadData)); } catch { /* Binary transport. */ } });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const shot = async name => { await page.screenshot({path:path.join(output, `${name}.png`)}); };
const check = (name, condition) => { assert.ok(condition, name); checks.push(name); };
const tab = async id => { await page.click(`[data-tab="tab-${id}"]`); await delay(300); };
const input = async (selector, value) => { await page.$eval(selector, (element, value) => { element.value = value; element.dispatchEvent(new Event('input', {bubbles:true})); }, value); };
const openPanel = async () => { if (await page.$eval('#panel', el => el.hidden)) await page.click('#panelToggle'); };
const unlock = async () => { if (await page.$eval('#settingsControls', el => getComputedStyle(el).display === 'none')) await page.click('#toggleSettingsLock'); };
const svgPoint = async (id, dx=70, dy=22) => page.$eval(`#graphDiagram [data-node-id="${id}"] rect`, (rect, {dx,dy}) => {
  const svg = rect.ownerSVGElement; const point = svg.createSVGPoint();
  point.x = Number(rect.getAttribute('x')) + dx; point.y = Number(rect.getAttribute('y')) + dy;
  const screen = point.matrixTransform(svg.getScreenCTM()); return {x:screen.x,y:screen.y};
}, {dx,dy});
const loadGraph = async spec => {
  await page.$eval('#graphSpecInput', el => {el.closest('details').open = true;});
  await input('#graphSpecInput', JSON.stringify(spec));
  await page.click('#graphSpecApply');
};
try {
  await page.setViewport({width:1440,height:900,deviceScaleFactor:1});
  await page.goto(url);
  await page.bringToFront();
  await page.waitForSelector('#joinSpectate:not([disabled])');
  await page.waitForFunction(()=>document.querySelector('.join-card').getAnimations().every(animation=>animation.playState==='finished'));
  await shot('desktop-join');
  check('join autofocus and visible focus', await page.$eval('#joinName', el => el === document.activeElement && getComputedStyle(el).outlineStyle !== 'none'));
  await page.focus('#joinSpectate'); await page.keyboard.press('Tab');
  check('join keyboard focus stays in the dialog', await page.$eval('#joinOverlay', el => el.contains(document.activeElement)));
  await page.click('#joinSpectate');
  check('hidden join controls are inert', await page.$eval('#joinOverlay', el => el.inert && el.getAttribute('aria-hidden') === 'true'));
  await shot('desktop-locked');
  check('observation and export accessible while settings locked', await page.$eval('#fitArena', el => !el.disabled && el.offsetHeight > 0) && await page.$eval('#btnExport', el=>!el.disabled));
  await page.focus('[data-tab="tab-settings"]'); await page.keyboard.press('ArrowRight');
  check('arrow navigation selects and focuses next tab', await page.$eval('[data-tab="tab-viz"]', el => el === document.activeElement && el.getAttribute('aria-selected') === 'true'));
  await tab('stats');
  const statsSize = await page.$eval('#statsCanvas', el => ({width:el.clientWidth,height:el.clientHeight,bw:el.width,bh:el.height}));
  check('Stats uses a deliberate 300 CSS-pixel height', statsSize.height === 300 && statsSize.bh === 300);
  check('Stats selector is horizontal flex', await page.$eval('.stats-selector', el => getComputedStyle(el).display === 'flex' && new Set([...el.children].map(child=>child.getBoundingClientRect().top)).size === 1));
  await shot('desktop-stats-fitness');
  await page.click('[data-stats="diversity"]'); await delay(300); await shot('desktop-stats-diversity');
  await page.click('[data-stats="complexity"]'); await delay(300); await shot('desktop-stats-complexity');
  await page.evaluate(() => {
    window.work = {hof:0,stats:0,log:0,buffers:0};
    for (const [id,key] of [['statsInfo','stats'],['godModeLog','log'],['hofTable','hof']]) new MutationObserver(()=>window.work[key]++).observe(document.getElementById(id),{childList:true,subtree:true});
    for (const key of ['width','height']) {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype,key);
      Object.defineProperty(HTMLCanvasElement.prototype,key,{...descriptor,set(value) {if(this.id==='statsCanvas') window.work.buffers++;descriptor.set.call(this,value);}});
    }
  });
  await delay(1500);
  metrics.statsWorkOver1500ms = await page.evaluate(()=>window.work);
  check('unchanged Stats buffer and log are retained', metrics.statsWorkOver1500ms.buffers === 0 && metrics.statsWorkOver1500ms.log === 0);
  await tab('hof');
  await page.evaluate(()=>{window.work.hof=0;window.hofButton=document.querySelector('#hofTable button');window.hofButton?.focus();});
  await delay(1500);
  metrics.hofWorkOver1500ms = await page.evaluate(()=>window.work.hof);
  check('Hall of Fame retains DOM and focus during world rendering', metrics.hofWorkOver1500ms === 0 && await page.evaluate(()=>!window.hofButton || window.hofButton===document.activeElement));
  await shot('desktop-hof');
  if (await page.$('#hofTable button:not(:disabled)')) {
    await page.click('#hofTable button:not(:disabled)'); await delay(500);
    check('champion Spawn remains clickable', await page.$eval('[data-tab="tab-viz"]',el=>el.getAttribute('aria-selected')==='true'));
  }
  await tab('viz'); await shot('desktop-visualizer');
  await tab('settings'); await unlock(); await shot('desktop-unlocked');
  const initialResets = commands.filter(command=>command.type==='reset').length;
  await page.$eval('#defaults', el=>el.click());
  check('Stage defaults sends no reset', commands.filter(command=>command.type==='reset').length === initialResets);
  await input('#snakes', '12');
  const resetDraft = await page.$eval('[data-path="worldRadius"]',el=>({value:el.value,id:el.id}));
  await input('#'+resetDraft.id, '4200');
  await input('#simSpeed', '1.05'); await delay(500);
  check('authoritative live replies preserve reset-only drafts', await page.$eval('#'+resetDraft.id,el=>el.value)==='4200');
  check('pending configuration visible outside collapsed groups', await page.$eval('#pendingConfiguration',el=>!el.hidden));
  await page.$eval('#baselineBotSeedRandomize',el=>el.click());
  await page.$eval('#baselineBotSeed',el=>el.dispatchEvent(new Event('blur')));
  await delay(100);
  check('baseline seed changes stage without an unexpected reset',commands.filter(command=>command.type==='reset').length===initialResets);
  const labels = await page.$$eval('#settingsContainer input, #settingsContainer select', elements=>elements.filter(el=>!el.id || !document.querySelector(`label[for="${el.id}"]`) && !el.closest('label') && !el.getAttribute('aria-label')).map(el=>el.dataset.path));
  check('dynamic settings have associated labels', labels.length===0);
  await page.$eval('#graphSpecInput',el=>el.closest('details').open=true);
  await input('#graphSpecInput','{invalid json'); await page.click('#graphSpecApply');
  check('invalid graph JSON produces actionable error without reset', await page.$eval('#graphSpecStatus',el=>el.textContent.includes('not valid JSON')));
  await shot('graph-validation-error');
  // Read sensor width from the current graph form, without reconstructing population artifacts.
  await page.click('#graphReset');
  const current = JSON.parse(await page.$eval('#graphSpecInput',el=>el.value));
  const width = current.nodes.find(node=>node.type==='Input').outputSize;
  const wide = {type:'graph',outputSize:2,nodes:[{id:'input',type:'Input',outputSize:width}, ...Array.from({length:6},(_,i)=>({id:`n${i}`,type:'Dense',inputSize:i?2:width,outputSize:2}))],edges:Array.from({length:6},(_,i)=>({from:i?`n${i-1}`:'input',to:`n${i}`})),outputs:[{nodeId:'n5'}]};
  const tall = {type:'graph',outputSize:2,nodes:[{id:'input',type:'Input',outputSize:width},...Array.from({length:10},(_,i)=>({id:`n${i}`,type:'Dense',inputSize:width,outputSize:2}))],edges:Array.from({length:10},(_,i)=>({from:'input',to:`n${i}`})),outputs:[{nodeId:'n9'}]};
  for (const [name,spec] of [['wide',wide],['tall',tall]]) {
    await loadGraph(spec);
    for (const expanded of [false,true]) {
      if (expanded) await page.click('#graphDiagramToggle');
      await page.$eval('#graphDiagram',el=>el.scrollIntoView({block:'center'}));
      const before = await page.$eval('#graphDiagram [data-node-id="n1"] rect',el=>Number(el.getAttribute('x')));
      const start=await svgPoint('n1');
      await page.mouse.move(start.x,start.y); await page.mouse.down(); await page.mouse.move(start.x+6,start.y+2,{steps:5}); await page.mouse.up();
      const after=await page.$eval('#graphDiagram [data-node-id="n1"] rect',el=>Number(el.getAttribute('x')));
      check(`${name} graph node dragging ${expanded?'expanded':'sidebar'}`,after>before);
      const handle = await svgPoint('n1',140,22); const target = await svgPoint('n2');
      await page.mouse.move(handle.x,handle.y); await page.mouse.down(); await page.mouse.move(target.x,target.y,{steps:5});
      check(`${name} connection target hit testing ${expanded?'expanded':'sidebar'}`,await page.$('#graphDiagram .connect-target')!==null);
      await page.evaluate(()=>document.getElementById('graphDiagram').dispatchEvent(new PointerEvent('pointercancel',{bubbles:true,pointerId:1})));
      await page.mouse.up();
      check(`${name} pointer cancellation clears connection`,await page.$('#graphDiagram .ghost.visible')===null);
      const countBefore=await page.$$eval('#graphEdges .graph-row',elements=>elements.length);
      const from=await svgPoint('n1',140,22);const to=await svgPoint('n2');
      await page.mouse.move(from.x,from.y);await page.mouse.down();await page.mouse.move(to.x,to.y,{steps:5});await page.mouse.up();
      const countAfter=await page.$$eval('#graphEdges .graph-row',elements=>elements.length);
      check(`${name} connection release ${expanded?'expanded':'sidebar'} preserves edge semantics`,name==='wide'?countAfter===countBefore:countAfter>=countBefore);
      const edge=await page.$('#graphDiagram .graph-diagram-edge-hit'); await edge.click();
      check(`${name} edge selection opens inspector`,await page.$eval('#graphDiagramInspector',el=>el.textContent.includes('Edge')));
      if(expanded) {await shot(`editor-${name}`);await page.keyboard.press('Escape');check('Escape closes editor and restores focus '+name,await page.$eval('#graphDiagramToggle',el=>el===document.activeElement&&!el.closest('.fullscreen')));}
    }
  }
  await page.$eval('#graphDiagram [data-node-id="n1"]',el=>el.focus()); await page.keyboard.press('Enter');
  check('keyboard diagram selection opens node form',await page.$eval('#graphDiagramInspector',el=>el.textContent.includes('Node: n1')));
  await page.click('#graphReset');
  await page.$eval('#graphApply',el=>el.click());
  await delay(100);
  check('Stage graph sends no reset',commands.filter(command=>command.type==='reset').length===initialResets);
  const pending = await page.$eval('#apply',el=>{el.click(); const disabled=el.disabled;el.click();return disabled;});
  check('reset disables duplicate submission',pending);
  await page.waitForFunction(()=>document.getElementById('resetStatus').textContent==='Reset applied.');
  check('one reset for one staged apply',commands.filter(command=>command.type==='reset').length===initialResets+1);
  check('authoritative replacement clears pending notice',await page.$eval('#pendingConfiguration',el=>el.hidden));
  // Portable fixture remains opaque File data to the UI and native admission remains authoritative.
  await tab('library'); await page.waitForSelector('#brainLibraryImport:not([disabled])');
  await input('#brainLibraryName','UI verification fixture');
  const upload=await page.$('#brainLibraryFile'); await upload.uploadFile(path.resolve('native/fixtures/portable-brain-v1.slither-brain'));
  await page.click('#brainLibraryImport'); await page.waitForSelector('.brain-library-item');
  await page.waitForFunction(()=>!document.getElementById('brainLibraryImport').disabled && document.getElementById('brainLibraryStatus').textContent.includes('Imported'));
  await page.$eval('.brain-library-item button',el=>el.click()); await delay(300);
  await page.$eval('#brainLibraryDetails',el=>el.scrollIntoView({block:'start'}));await shot('brain-library-details');
  check('library compatibility explains structural differences',await page.$eval('#brainLibraryDetails',el=>el.textContent.includes('sensors')&&el.textContent.includes('Source:')));
  let confirmSeen=false;
  page.once('dialog', async dialog=>{confirmSeen=dialog.message().includes('Permanently delete');await dialog.dismiss();});
  await page.$$eval('.brain-library-item button',elements=>elements.find(el=>el.textContent==='Delete').click()); await delay(300);
  check('saved brain deletion requires confirmation',confirmSeen && await page.$('.brain-library-item')!==null);
  await tab('distillation'); await shot('distillation');
  await tab('settings');
  const duration = await page.$eval('[data-path="generationSeconds"]',el=>({id:el.id,value:el.value}));
  await input('#'+duration.id,'bad duration');
  await page.$eval('#'+duration.id,el=>el.closest('details').open=false);
  await page.$eval('#apply',el=>el.click());
  check('validation reveals an invalid control in a collapsed group',await page.$eval('#'+duration.id,el=>el.closest('details').open&&el.getAttribute('aria-invalid')==='true'));
  await page.$eval('#'+duration.id,el=>el.scrollIntoView({block:'center'}));await shot('settings-validation-error');
  await input('#'+duration.id,duration.value);
  for (const [width,height] of [[1440,900],[1280,720],[1024,768],[768,1024],[390,844]]) {
    await page.setViewport({width,height,deviceScaleFactor:width===390?2:1}); await openPanel();
    await tab('settings'); await unlock();
    await page.$eval('#panel',el=>el.scrollTop=0); await shot(`${width}x${height}-settings`);
    check(`${width} has no horizontal overflow`,await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    check(`${width} panel controls do not overflow horizontally`,await page.$eval('#panel',el=>el.scrollWidth<=el.clientWidth+1));
    await tab('stats');
    check(`${width} Stats display and backing buffer agree`,await page.$eval('#statsCanvas',el=>el.height===Math.round(el.clientHeight*devicePixelRatio)&&el.width===Math.round(el.clientWidth*devicePixelRatio)));
    await shot(`${width}x${height}-stats`);
    await tab('viz');
    check(`${width} Visualizer display and backing buffer agree`,await page.$eval('#vizCanvas',el=>el.clientHeight>=260&&el.clientHeight<=400&&el.height===Math.round(el.clientHeight*devicePixelRatio)&&el.width===Math.round(el.clientWidth*devicePixelRatio)));
    await shot(`${width}x${height}-visualizer`);
    await tab('settings'); await page.$eval('#graphDiagramToggle',el=>el.scrollIntoView({block:'center'})); await page.click('#graphDiagramToggle');
    await page.$eval('#graphDiagram [data-node-id]',el=>el.focus());await page.keyboard.press('Enter');await shot(`${width}x${height}-editor`);
    check(`${width} expanded editor stays in viewport`,await page.$eval('#graphDiagramWrap',el=>{const r=el.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.bottom<=innerHeight;}));
    await page.keyboard.press('Escape');await page.click('#panelToggle');
    check(`${width} collapse exposes full world canvas`,await page.$eval('#c',el=>el.clientWidth===innerWidth));await shot(`${width}x${height}-world`);
  }
  await page.setViewport({width:1024,height:768,deviceScaleFactor:1});await openPanel();
  await tab('settings');await page.$eval('#panel',el=>el.scrollTop=0);
  await page.addStyleTag({content:'.panel {font-size:16.25px} .panel .meta,.setting .foot {font-size:15px}'});
  await shot('text-scale-125');
  check('125% text scaling retains reachable panel without horizontal overflow',await page.$eval('#panel',el=>el.scrollWidth<=el.clientWidth+1&&el.scrollHeight>0));
  await page.evaluate(()=>window.reviewSockets.at(-1).close());
  await page.waitForFunction(()=>document.querySelector('#connectionStatus').classList.contains('connecting'));
  await shot('reconnecting');
  await page.waitForFunction(()=>document.querySelector('#connectionStatus').classList.contains('server'));
  check('reconnection restores spectator without hidden focus',await page.$eval('#joinOverlay',el=>el.inert&&!el.contains(document.activeElement)));
  check('navigation and reconnect issue no extra resets or new runs',commands.filter(command=>command.type==='reset').length===initialResets+1&&commands.filter(command=>command.type==='newRun').length===0);
  const phone=await browser.newPage();phone.on('pageerror',error=>errors.push(error.message));
  await phone.setViewport({width:390,height:844,deviceScaleFactor:2,isMobile:true,hasTouch:true});await phone.goto(url);await phone.bringToFront();await phone.waitForSelector('#joinSpectate:not([disabled])');
  await phone.waitForFunction(()=>document.querySelector('.join-card').getAnimations().every(animation=>animation.playState==='finished'));
  await phone.screenshot({path:path.join(output,'phone-touch-join.png')});
  check('fresh phone session starts with a collapsed panel',await phone.$eval('#panel',el=>el.hidden));
  await phone.tap('#joinSpectate');await phone.tap('#panelToggle');await phone.tap('[data-tab="tab-stats"]');await delay(300);
  check('touch spectating and tab navigation work',await phone.$eval('[data-tab="tab-stats"]',el=>el.getAttribute('aria-selected')==='true'));
  await phone.screenshot({path:path.join(output,'phone-touch-stats.png')});await phone.close();
  check('no browser console errors',consoleErrors.length===0);
  check('no browser runtime exceptions',errors.length===0);
} catch (error) {
  await shot('failure');
  console.log('failure focus',await page.evaluate(()=>({active:document.activeElement?.id,toggle:document.getElementById('graphDiagramToggle').getBoundingClientRect().toJSON(),panel:document.getElementById('panel').getBoundingClientRect().toJSON()})));
  throw error;
} finally {
  metrics.browser=await browser.version();
  await writeFile(path.join(output,'results.json'),JSON.stringify({checks,passed:checks.length,errors,consoleErrors,metrics,resetCommands:commands.filter(command=>command.type==='reset').length},null,2));
  console.log(JSON.stringify({passed:checks.length,errors,consoleErrors,metrics,output},null,2));
  await browser.close();
}
