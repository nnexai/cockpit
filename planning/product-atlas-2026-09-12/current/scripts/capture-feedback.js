async page => {
  const panel = page.getByRole('dialog', { name: /Feedback ·/ });
  await panel.screenshot({
    path: '/home/nnex/dev/prj/cockpit/planning/product-atlas-2026-09-12/current/screenshots/browser-cockpit-feedback.png',
    scale: 'css',
    type: 'png',
  });
}
