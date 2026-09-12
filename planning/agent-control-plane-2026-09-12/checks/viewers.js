async page => {
  await page.setViewportSize({width:1440,height:900});
  await page.goto('http://127.0.0.1:4186/planning/agent-control-plane-2026-09-12/mocks/viewers.html');
  const frames = page.frames().filter(frame => frame.parentFrame());
  if (frames.length !== 2) throw new Error('Viewer comparison requires two frames');
  const results = [];
  for (const frame of frames) {
    await frame.evaluate(async () => { await document.fonts.ready; });
    const state = await frame.evaluate(() => ({
      surface:document.querySelector('#surface').value,
      labHidden:getComputedStyle(document.querySelector('.labbar')).display === 'none',
      fits:document.documentElement.scrollWidth <= innerWidth,
    }));
    if (!state.labHidden || !state.fits) throw new Error('Embedded viewer overflow or extra lab header');
    results.push(state.surface + ' comparison fits without a duplicate design toolbar');
  }
  await page.screenshot({path:'planning/agent-control-plane-2026-09-12/previews/unified-viewers.png'});
  const review = frames.find(frame => frame.url().includes('surface=review'));
  const footer = review.locator('.paste-bar');
  if (!(await footer.isHidden())) throw new Error('Empty review has a delivery footer');
  results.push('Empty review hides delivery controls');
  await review.locator('.review .reader-extra summary').click();
  await review.locator('#review-draft').fill('Sample draft for the design check');
  if (!(await footer.isVisible())) throw new Error('Draft has no delivery controls');
  results.push('A review draft exposes delivery controls');
  await review.locator('#review-draft').fill('');
  if (!(await footer.isHidden())) throw new Error('Cleared draft keeps empty delivery controls');
  results.push('Clearing the draft removes empty delivery controls');
  return {kind:'mock-only viewer comparison',productionBehaviorVerified:false,checks:results.length,results};
}
