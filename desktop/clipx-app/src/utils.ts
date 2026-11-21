import type { DiagnosticSnapshot } from "./types";

export function visibleDiagnosticLogs(
  snapshot: DiagnosticSnapshot | null,
  clearMarker: string | null,
): string[] {
  const logs = snapshot?.recentLogs ?? [];
  if (!clearMarker) return logs;
  const markerIndex = logs.lastIndexOf(clearMarker);
  return markerIndex >= 0 ? logs.slice(markerIndex + 1) : logs;
}

export function groupFingerprint(fp: string): string {
  return (
    fp
      .match(/.{1,4}/g)
      ?.slice(0, 7)
      .join("-") ?? fp
  );
}

export function timeAgo(ts: number, locale = "en-US"): string {
  const now = new Date();
  const date = new Date(ts);
  const diffSec = Math.floor((now.getTime() - date.getTime()) / 1000);

  if (diffSec < 60) return "just now";

  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "always" });

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return rtf.format(-diffMin, "minute");

  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return rtf.format(-diffHr, "hour");

  const diffDays = Math.floor(diffHr / 24);
  if (diffDays < 7) return rtf.format(-diffDays, "day");

  const diffWeeks = Math.floor(diffDays / 7);
  if (diffWeeks < 4) return rtf.format(-diffWeeks, "week");

  const diffMonths = Math.floor(diffDays / 30);
  if (diffMonths <= 3) return rtf.format(-diffMonths, "month");

  const isSameYear = date.getFullYear() === now.getFullYear();
  const options: Intl.DateTimeFormatOptions = {
    day: "numeric",
    month: "long",
    ...(isSameYear ? {} : { year: "numeric" }),
  };

  return new Intl.DateTimeFormat(locale, options).format(date);
}

export function formatDataSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(2)} ${units[i]}`;
}
