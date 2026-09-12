async page => {
  const base = page.url().split('/planning/')[0] + '/planning/agent-control-plane-2026-09-12/';
  const output = '/home/nnex/dev/prj/cockpit/planning/agent-control-plane-2026-09-12/previews/';
  const results = [];
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  function check(condition, message) {
    if (!condition) throw new Error(message);
    results.push(message);
  }
  async function screenshot(name) { await page.screenshot({path:output + name + '.png'}); }
  async function viewportFit(label) {
    const fit = await page.evaluate(() => ({width:innerWidth, height:innerHeight, scrollWidth:document.documentElement.scrollWidth, scrollHeight:document.documentElement.scrollHeight}));
    check(fit.scrollWidth <= fit.width && fit.scrollHeight <= fit.height, label + ' fits the viewport');
  }
  await page.setViewportSize({width:1440,height:900});
  await page.goto(base + 'mocks/index.html?capture=1');
  await page.locator('#rail-resizer').focus();
  await page.keyboard.press('ArrowRight');
  check(await page.locator('#rail-resizer').getAttribute('aria-valuenow') === '232', 'Rail resize works with keyboard');
  await page.keyboard.press('Home');
  const rail = await page.locator('#rail-resizer').boundingBox();
  await page.mouse.move(rail.x + 3,rail.y + 180);
  await page.mouse.down();
  await page.mouse.move(rail.x + 51,rail.y + 180);
  await page.mouse.up();
  check(await page.locator('#rail-resizer').getAttribute('aria-valuenow') === '272', 'Rail resize works with pointer');
  await page.locator('#rail-resizer').focus();
  await page.keyboard.press('Home');
  await page.locator('#sidebar-collapse').click();
  check(!(await page.locator('#sidebar').isVisible()), 'Desktop sidebar collapses');
  await page.locator('#drawer-toggle').click();
  check(await page.locator('#sidebar').isVisible(), 'Desktop sidebar restores');
  await screenshot('shell-desktop');
  await page.locator('#commands-button').click();
  check(await page.locator('#command-search').evaluate(element => element === document.activeElement), 'Commands focuses search');
  await page.locator('#command-search').fill('files');
  check(await page.locator('#command-actions .action-pair:visible').count() === 1, 'Command search filters to Files');
  await page.locator('#command-search').fill('no such command');
  check(await page.locator('#command-empty').isVisible(), 'Command search has an empty state');
  await page.locator('#command-search').fill('');
  await screenshot('commands');
  await page.keyboard.press('Escape');
  check(await page.locator('#commands-button').evaluate(element => element === document.activeElement), 'Commands restores its opener on Escape');
  await page.locator('.space-row').last().click({button:'right'});
  check((await page.locator('#resource-title').textContent()).includes('Atlas Beta'), 'Right-click targets the clicked Space');
  await screenshot('space-menu');
  await page.keyboard.press('Escape');
  for (const [width,height] of [[800,1000],[600,900],[480,900]]) {
    await page.setViewportSize({width,height});
    await page.goto(base + 'mocks/index.html?capture=1');
    await viewportFit('Shell ' + width);
    check(await page.locator('.pane').count() === 2, 'Shell retains two panes at ' + width);
    const before = await page.locator('.pane-canvas').boundingBox();
    check(before.width === width, 'Closed drawer gives the canvas full width at ' + width);
    await screenshot('shell-' + width);
    await page.locator('#drawer-toggle').click();
    const during = await page.locator('.pane-canvas').boundingBox();
    check(JSON.stringify(before) === JSON.stringify(during), 'Opening drawer preserves canvas geometry at ' + width);
    check(await page.locator('.agent-row:visible').count() === 4, 'All four recorded agent rows are available at ' + width);
    if (width === 480) await screenshot('sidebar-480-open');
    await page.keyboard.press('Shift+Tab');
    check(await page.locator('#sidebar').evaluate(element => element.contains(document.activeElement)), 'Drawer retains keyboard focus at ' + width);
    await page.keyboard.press('Escape');
    check(await page.locator('#drawer-toggle').evaluate(element => element === document.activeElement), 'Drawer returns focus at ' + width);
  }
  await page.setViewportSize({width:800,height:1000});
  await page.goto(base + 'mocks/index.html?capture=1&direction=rail');
  await screenshot('rail-800-alternative');
  await page.goto(base + 'mocks/index.html?capture=1&state=stale');
  check(await page.locator('#recovery').isVisible(), 'Stale view has a local recovery action');
  await screenshot('stale-800');
  await page.locator('#retry').click();
  await page.waitForFunction(() => document.querySelector('#app').dataset.scenario === 'live');
  check(await page.locator('#recovery').isHidden(), 'Illustrative resync returns to the retained view');
  await page.goto(base + 'mocks/index.html?capture=1&state=observing');
  await page.locator('.terminal-content').first().click();
  check(await page.locator('#ownership').isHidden(), 'Illustrative selection needs no second Take focus action');

  await page.goto(base + 'mocks/surfaces.html');
  for (const [width,height] of [[1440,900],[800,1000],[600,900],[480,900]]) {
    await page.setViewportSize({width,height});
    for (const surface of ['context','review','setup','feedback','capture']) {
      await page.locator('#surface').selectOption(surface);
      check(await page.locator('.surface:visible').count() === 1, 'Only the chosen ' + surface + ' surface appears at ' + width);
      await viewportFit(surface + ' ' + width);
      const clipped = await page.locator('.mock').evaluate(element => element.scrollWidth > element.clientWidth + 1);
      check(!clipped, surface + ' content has no horizontal overflow at ' + width);
      if (width === 1440 || width === 480) await screenshot(surface + '-' + width);
    }
  }
  await page.setViewportSize({width:1440,height:900});
  await page.locator('#surface').selectOption('context');
  await page.locator('#narrow').click();
  check(await page.locator('.file-tree').isHidden(), 'File tree adapts to pane width on a wide desktop');
  await page.locator('#tree-toggle').click();
  check(await page.locator('.file-tree').isVisible(), 'Narrow file tree can reopen');
  await page.locator('[data-file="README.md"]').click();
  check((await page.locator('#file-content').textContent()).includes('# Cockpit'), 'File selection changes the document');
  check(await page.locator('.file-tree').isHidden(), 'Selecting a file closes the narrow tree');
  await screenshot('context-split-narrow');
  await page.locator('#narrow').click();
  await page.locator('#file-search').fill('README');
  check(await page.locator('[data-file]:visible').count() === 1, 'File search filters visible paths');
  await page.locator('#file-search').fill('');
  await page.locator('[data-folder="src"]').click();
  check(await page.locator('[data-folder-file="src"]:visible').count() === 0, 'Folder disclosure hides its children');
  await page.locator('#context-load').click();
  await page.locator('#context-load').click();
  check((await page.locator('#context-load-status').textContent()).includes('canceled'), 'Cancel retains the current document');
  await page.locator('#surface').selectOption('review');
  await page.locator('#review-scope').selectOption('branch');
  check(await page.locator('#base-field').isVisible(), 'Review base appears for branch scope');
  await page.locator('#review-scope').selectOption('local');
  check(await page.locator('#base-field').isHidden(), 'Review base is hidden for local scope');
  await page.locator('[data-diff="styles"]').click();
  check((await page.locator('#diff-path').textContent()).includes('styles.css'), 'Review file selection updates the diff');
  await page.locator('#surface').selectOption('setup');
  await page.locator('#repo').selectOption('cockpit');
  await page.locator('#trust').check();
  await page.locator('#path').fill('/tmp/ui-study-worktree');
  check((await page.locator('#effect-path').textContent()) === '/tmp/ui-study-worktree', 'Setup effect preview follows the input');
  await page.locator('#surface').selectOption('capture');
  await page.locator('#capture-open').click();
  await page.keyboard.press('e');
  await page.locator('[data-element]').hover();
  check(await page.locator('[data-element].hovered').count() === 1, 'Element annotation previews its target before click');
  await screenshot('capture-element-preview');
  await page.locator('[data-element]').click();
  await page.locator('#capture-draft').fill('Keep this annotation draft');
  await page.keyboard.press('v');
  check((await page.locator('#capture-draft').inputValue()).endsWith('v'), 'Tool shortcuts do not intercept editor typing');
  await page.keyboard.press('Escape');
  check(await page.locator('#annotation-editor').isHidden(), 'Escape closes the editor');
  check((await page.locator('#capture-draft').inputValue()).includes('Keep this annotation draft'), 'Escape retains the annotation draft');
  check(await page.locator('#gesture-tools').isVisible(), 'First Escape leaves the toolbar available');
  await page.keyboard.press('Escape');
  check(await page.locator('#gesture-tools').isHidden(), 'Second Escape closes the toolbar and returns to browse');
  await page.locator('#capture-open').click();
  await page.keyboard.press('v');
  await page.locator('.annotation-item').first().click();
  check(await page.locator('#annotation-editor').isVisible(), 'Select tool reopens an existing annotation');
  await page.locator('#delete-annotation').click();
  check(await page.locator('.annotation-item').count() === 0, 'Selected annotation can be deleted');
  await page.locator('#surface').selectOption('feedback');
  await page.locator('#state').selectOption('disconnected');
  check(await page.locator('#deliver').isDisabled(), 'Disconnected feedback disables delivery');
  check(await page.locator('#feedback-draft').isEditable(), 'Disconnected feedback retains editable drafts');
  await page.locator('#state-retry').click();
  await page.waitForFunction(() => document.querySelector('.mock').dataset.state === 'ready');
  for (const surface of ['context','review','setup','feedback','capture']) {
    await page.locator('#surface').selectOption(surface);
    await page.locator('#state').selectOption('empty');
    check(await page.locator('.empty-detail:visible').count() === 1, surface + ' has a distinct empty state');
  }
  check(errors.length === 0, 'No browser JavaScript errors');
  return {kind:'mock-only verification',productionBehaviorVerified:false,checks:results.length,results,errors};
}
