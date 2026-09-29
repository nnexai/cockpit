/** Neutral placeholder rows while the first snapshot loads. No shimmer. */
export function SidebarSkeleton({ rows }: { rows: number }) {
  return <div className="sidebar-skeleton" aria-hidden="true">{Array.from({ length: rows }, (_, index) => <div className="skeleton-row" key={index}><i /><u style={{ width: `${33 + ((index * 17) % 30)}%` }} /></div>)}</div>;
}
