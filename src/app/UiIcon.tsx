const paths = {
  more: "M5 12h.01M12 12h.01M19 12h.01",
  plus: "M12 5v14M5 12h14",
  expand: "M9 3H3v6m12-6h6v6M3 15v6h6m12-6v6h-6",
  edit: "m16 3 5 5L8 21H3v-5ZM14 5l5 5",
  trash: "M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7",
  file: "M14 3H5v18h14V8Zm0 0v5h5M8 12h8m-8 4h6",
  search: "M15 15l6 6M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0",
  sidebar: "M9 4v16M3 4h18v16H3Z",
  browser: "M3 5h18v14H3ZM3 9h18M6 7h.01M9 7h.01",
  grid: "M3 3h7v7H3Zm11 0h7v7h-7ZM3 14h7v7H3Zm11 0h7v7h-7Z",
  down: "m6 9 6 6 6-6",
  right: "m9 5 7 7-7 7",
  back: "M19 12H5m7 7-7-7 7-7",
  forward: "M5 12h14m-7-7 7 7-7 7",
  stop: "M6 6h12v12H6z",
  branch: "M6 7v10m12-10c0 7-12 4-12 10M8 5a2 2 0 1 1-4 0 2 2 0 0 1 4 0m0 14a2 2 0 1 1-4 0 2 2 0 0 1 4 0M20 5a2 2 0 1 1-4 0 2 2 0 0 1 4 0",
  info: "M12 11v6m0-10v1M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0",
  refresh: "M20 7v5h-5M4 17v-5h5M6 6a8 8 0 0 1 14 6M4 12a8 8 0 0 0 14 6",
  terminal: "m4 6 6 6-6 6m9 0h7",
  close: "m6 6 12 12M6 18 18 6",
  comment: "M21 15a3 3 0 0 1-3 3H8l-5 3V6a3 3 0 0 1 3-3h12a3 3 0 0 1 3 3ZM7 8h10M7 12h7",
} satisfies Record<string, string>;

export function UiIcon({ name }: { name: keyof typeof paths }) {
  return <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]} /></svg>;
}
