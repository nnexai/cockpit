import { relativeTime } from "../library/libraryState";

export function Time({ value }: { value: string | null | undefined }) {
  const label = value ? relativeTime(value, Date.now()) : null;
  return label && value ? <time dateTime={value} title={new Date(value).toLocaleString()}>{label}</time> : null;
}
