async page => {
  const out = '/home/nnex/dev/prj/cockpit/planning/product-atlas-2026-09-12/current';
  const shot = async (name) => page.screenshot({path: `${out}/${name}.png`, scale: 'css', type: 'png'});
  const snap = async () => page.waitForTimeout(900);

  await page.setViewportSize({width: 1440, height: 900}); await snap(); await shot('app-shell-desktop');
  await page.getByRole('button', {name: 'Collapse sidebar'}).click(); await snap(); await shot('app-sidebar-collapsed');
  await page.setViewportSize({width: 480, height: 900}); await snap(); await shot('app-drawer-480');
  await page.getByRole('button', {name: 'Close sidebar'}).click();
  await page.setViewportSize({width: 800, height: 1000}); await snap(); await shot('app-drawer-800');
  await page.setViewportSize({width: 1440, height: 900}); await snap();
  if (await page.getByRole('button', {name: 'Expand sidebar'}).count()) await page.getByRole('button', {name: 'Expand sidebar'}).click();
  await page.getByRole('button', {name: 'Commands'}).click(); await snap();
  await page.getByRole('textbox', {name: 'Find a command'}).fill('pane'); await shot('app-commands-search-desktop');
  await page.setViewportSize({width: 480, height: 900}); await snap(); await shot('app-commands-search-480'); await page.keyboard.press('Escape');
  await page.setViewportSize({width: 1440, height: 900}); await snap();
  await page.getByRole('button', {name: 'Pane actions for /t/c/r/cockpit'}).click(); await shot('app-resource-menu'); await page.keyboard.press('Escape');

  await page.getByRole('button', {name: 'Set up a task Space'}).click();
  await page.getByRole('textbox', {name: 'Task name · optional'}).fill('Atlas screenshots task');
  await page.getByRole('textbox', {name: 'Issue or review URL · optional'}).fill('https://github.com/nnexai/cockpit/issues/4');
  await shot('app-setup-draft'); await page.getByRole('button', {name: 'Continue to workspace'}).click();
  await page.getByRole('textbox', {name: 'Branch · required'}).fill('atlas-app-captures');
  await page.getByRole('checkbox', {name: 'I understand and consent to'}).check();
  await page.getByRole('button', {name: 'Review exact effects'}).click(); await shot('app-setup-review-plan');
  await page.getByRole('button', {name: 'Start Create linked worktree'}).click(); await snap(); await page.getByRole('button', {name: 'Done'}).click();
  const terminal = page.getByRole('textbox', {name: 'Terminal input'});
  await terminal.click(); await page.keyboard.type("printf '\\n<!-- atlas app screenshot fixture -->\\n' >> CONTEXT.md"); await page.keyboard.press('Enter'); await snap();

  await page.getByRole('button', {name: 'Commands'}).click(); await page.getByRole('option', {name: 'Open files right'}).click(); await snap();
  await page.getByRole('button', {name: 'sources', exact: true}).click(); await page.getByRole('button', {name: 'sources/provider-'}).click();
  await page.getByRole('button', {name: 'sources/provider-', exact: false}).last().click(); await snap();
  await page.getByRole('button', {name: 'asset-'}).click(); await snap(); await shot('app-files-source');
  await page.locator('.context-document-header').click(); await page.keyboard.press('Control+P'); await snap(); await shot('app-files-picker-dialog'); await page.keyboard.press('Escape');
  await shot('app-context-issue-1440');
  const sep = page.getByRole('separator', {name: 'Resize pane w2:p3 horizontally'}); const b = await sep.boundingBox();
  await page.mouse.move(b.x + b.width / 2, b.y + 180); await page.mouse.down(); await page.mouse.move(1075, b.y + 180, {steps: 14}); await page.mouse.up(); await snap(); await shot('app-context-split-360');
  await page.setViewportSize({width: 480, height: 900});
  await page.keyboard.press('Control+b'); await page.keyboard.press('z'); await snap(); await shot('app-context-zoom-480');

  await page.setViewportSize({width: 1440, height: 900}); await snap();
  await page.getByRole('textbox', {name: 'Terminal input'}).click(); await page.getByRole('button', {name: 'Commands'}).click(); await page.getByRole('option', {name: 'Open Review right'}).click(); await snap(); await shot('app-review-diff');
  const added = page.getByRole('button', {name: /\+<!-- atlas app screenshot fixture/}); if (await added.count()) { await added.first().click(); await page.getByRole('button', {name: 'C comment'}).click(); await page.getByRole('textbox', {name: 'Comment text'}).fill('Atlas portrait Review comment'); await shot('app-review-line-comment-desktop'); await page.getByRole('button', {name: 'Save comment'}).click(); }
  await page.setViewportSize({width: 480, height: 900});
  await page.getByRole('main', {name:'Unified diff'}).click({position:{x:10,y:180}});
  await page.keyboard.press('Control+b'); await page.keyboard.press('z'); await snap(); await shot('app-review-line-comment-480');
}
