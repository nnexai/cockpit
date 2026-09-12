async page => {
  const viewports = [
    { label: "1440x900", width: 1440, height: 900 },
    { label: "800x1000", width: 800, height: 1000 },
    { label: "600x900", width: 600, height: 900 },
    { label: "480x900", width: 480, height: 900 },
  ];
  const originalViewport = typeof page.viewportSize === "function" ? page.viewportSize() : null;
  const failures = [];
  const inconclusive = [];
  const viewportResults = [];
  let pickerResult = { status: "INCONCLUSIVE", reason: "No reachable Files trigger was observed" };
  let pickerExercised = false;
  const initialPickerOpen = await page.locator(".file-picker").count() > 0;

  const waitForLayout = async () => {
    await page.evaluate(async () => {
      if (document.fonts?.ready) await document.fonts.ready;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
  };

  const inspectVisibleViewer = async () => page.evaluate(() => {
    const visible = element => {
      if (!element) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.display !== "none" && style.visibility !== "hidden" && Number(style.opacity) !== 0 && rect.width > 0 && rect.height > 0;
    };
    const firstVisible = (root, selectors) => {
      for (const selector of selectors) {
        const element = [...root.querySelectorAll(selector)].find(visible);
        if (element) return element;
      }
      return null;
    };
    const candidates = [
      {
        surface: "context",
        selector: ".context-viewer",
        documentSelector: ".context-document",
        listSelector: ".context-tree",
        lineSelectors: [".context-source-line"],
      },
      {
        surface: "review",
        selector: ".review-pane",
        documentSelector: ".review-diff",
        listSelector: ".review-files",
        lineSelectors: [".review-line", ".context-source-line"],
      },
    ];
    const candidate = candidates
      .map(item => ({ ...item, root: document.querySelector(item.selector) }))
      .filter(item => visible(item.root))
      .sort((left, right) => right.root.getBoundingClientRect().width - left.root.getBoundingClientRect().width)[0];
    if (!candidate) return null;
    const root = candidate.root;
    const documentPane = root.querySelector(candidate.documentSelector);
    const list = root.querySelector(candidate.listSelector);
    const line = firstVisible(root, candidate.lineSelectors);
    const control = firstVisible(root, [
      ".context-toolbar button, .context-toolbar select",
      ".review-toolbar button",
      ".review-toolbar select",
      ".review-toolbar input",
      ".review-source-controls button",
    ]);
    const metadata = firstVisible(root, [
      ".context-tree-meta, .context-tree-header",
      ".review-file small, .review-files > header small, .review-status",
    ]);
    const trigger = firstVisible(root, [".viewer-file-picker-trigger"]);
    const rootRect = root.getBoundingClientRect();
    const documentRect = documentPane?.getBoundingClientRect();
    const lineRect = line?.getBoundingClientRect();
    const styleInfo = element => element ? (() => {
      const style = getComputedStyle(element);
      return { family: style.fontFamily, size: style.fontSize, lineHeight: style.lineHeight };
    })() : null;
    const selected = candidate.surface === "context"
      ? root.querySelector(".context-tree-row.is-selected")?.getAttribute("data-context-path") ?? null
      : root.querySelector(".review-file.is-selected")?.getAttribute("data-file-id") ?? null;
    return {
      surface: candidate.surface,
      pane: { width: rootRect.width, height: rootRect.height },
      document: documentRect ? { width: documentRect.width, height: documentRect.height, textLength: documentPane.innerText.trim().length } : null,
      line: line && lineRect ? { className: line.className, width: lineRect.width, height: lineRect.height, style: styleInfo(line) } : null,
      control: styleInfo(control),
      metadata: styleInfo(metadata),
      list: list ? { display: getComputedStyle(list).display, width: list.getBoundingClientRect().width, visible: visible(list) } : null,
      trigger: trigger ? { visible: true, enabled: !trigger.matches(":disabled") } : { visible: false, enabled: false },
      selected,
    };
  });

  const checkObservation = (observation, label) => {
    if (!observation) {
      failures.push(`${label}: no visible Context or Review pane`);
      return { label, status: "FAIL", reason: "No visible Context or Review pane" };
    }
    const result = { label, surface: observation.surface, pane: observation.pane, document: observation.document, line: observation.line, control: observation.control, metadata: observation.metadata, list: observation.list, trigger: observation.trigger, status: "PASS" };
    if (!observation.document || observation.document.width <= 0 || observation.document.height <= 0 || observation.document.textLength === 0) {
      failures.push(`${label}: visible ${observation.surface} document has zero content`);
      result.status = "FAIL";
      result.reason = "Visible document has zero bounds or no rendered text";
    }
    if (!observation.line) {
      inconclusive.push(`${label}: visible ${observation.surface} has no source/diff line role`);
      result.status = result.status === "PASS" ? "INCONCLUSIVE" : result.status;
      result.reason = result.reason ?? "No visible source/diff line role to measure";
    } else {
      const style = observation.line.style;
      if (!style.family.toLowerCase().includes("plex mono") || style.size !== "13px" || style.lineHeight !== "19px") {
        failures.push(`${label}: ${observation.surface} source/diff typography is ${style.family} ${style.size}/${style.lineHeight}`);
        result.status = "FAIL";
        result.reason = "Source/diff typography does not match 13px/19px Plex Mono";
      }
    }
    if (!observation.control) {
      inconclusive.push(`${label}: visible ${observation.surface} has no control role`);
      result.status = result.status === "PASS" ? "INCONCLUSIVE" : result.status;
    } else if (!observation.control.family.toLowerCase().includes("plex sans") || observation.control.size !== "13px" || observation.control.lineHeight !== "20px") {
      failures.push(`${label}: ${observation.surface} control typography is ${observation.control.family} ${observation.control.size}/${observation.control.lineHeight}`);
      result.status = "FAIL";
      result.reason = "Control typography does not match 13px/20px Plex Sans";
    }
    const narrow = observation.pane.width <= 520;
    const listHidden = !observation.list || !observation.list.visible || observation.list.display === "none" || observation.list.width <= 0;
    if (narrow && (!listHidden || !observation.trigger.visible || !observation.trigger.enabled)) {
      failures.push(`${label}: narrow ${observation.surface} has no reachable Files trigger/list collapse`);
      result.status = "FAIL";
      result.reason = "Narrow pane did not collapse navigation to an enabled Files trigger";
    }
    if (!narrow && observation.list && !observation.list.visible) {
      failures.push(`${label}: wide ${observation.surface} hides its file navigation`);
      result.status = "FAIL";
      result.reason = "Wide pane unexpectedly hides file navigation";
    }
    return result;
  };

  try {
    await waitForLayout();
    const initial = await inspectVisibleViewer();
    let narrowSplit = { status: "INCONCLUSIVE", reason: "Run this script while the real viewer has an existing approximately 360px pane" };
    if (initial && initial.pane.width <= 380) {
      narrowSplit = checkObservation(initial, `narrow-split-${Math.round(initial.pane.width)}px`);
    }
    for (const viewport of viewports) {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await waitForLayout();
      const observation = await inspectVisibleViewer();
      const result = checkObservation(observation, viewport.label);
      viewportResults.push(result);
      if (observation && observation.pane.width <= 380 && narrowSplit.status === "INCONCLUSIVE") {
        narrowSplit = checkObservation(observation, `narrow-split-${Math.round(observation.pane.width)}px`);
      }
      if (observation?.trigger.visible && observation.trigger.enabled && !pickerExercised && !initialPickerOpen) {
        const before = observation.selected;
        const trigger = page.locator(`${observation.surface === "context" ? ".context-viewer" : ".review-pane"} .viewer-file-picker-trigger`).first();
        await trigger.click();
        const picker = page.locator(".file-picker");
        await picker.waitFor({ state: "visible", timeout: 1500 });
        // Escape must be delivered to the modal's focused search field. A page-level
        // Escape can be consumed by the host shortcut layer before the picker closes.
        await picker.press("Escape");
        await page.waitForTimeout(250);
        if (await page.locator(".file-picker:visible").count() > 0) {
          throw new Error("Files picker remained open after Escape");
        }
        const after = (await inspectVisibleViewer())?.selected ?? null;
        pickerExercised = true;
        if (before === null || after === null) {
          pickerResult = { status: "INCONCLUSIVE", surface: observation.surface, before, after, action: "open Files trigger and close with Escape", reason: "No selected file identity was rendered" };
          inconclusive.push(`${viewport.label}: Files picker could not verify a rendered selected file identity`);
        } else {
          pickerResult = before === after
            ? { status: "PASS", surface: observation.surface, selected: after, action: "open Files trigger and close with Escape" }
            : { status: "FAIL", surface: observation.surface, before, after, action: "open Files trigger and close with Escape" };
          if (before !== after) failures.push(`${viewport.label}: Files picker changed the selected file`);
        }
      }
    }
    return {
      kind: "production-viewer-width-check",
      productionBehaviorVerified: true,
      status: failures.length > 0 ? "FAIL" : inconclusive.length > 0 ? "INCONCLUSIVE" : "PASS",
      observedSurface: viewportResults.find(result => result.surface)?.surface ?? initial?.surface ?? null,
      viewports: viewportResults,
      narrowSplit,
      picker: pickerResult,
      failures,
      inconclusive,
      originalViewport,
    };
  } finally {
    if (!initialPickerOpen) {
      const picker = page.locator(".file-picker");
      if (await picker.count() > 0) await page.keyboard.press("Escape");
    }
    if (originalViewport) {
      await page.setViewportSize(originalViewport);
      await waitForLayout();
    }
  }
}
