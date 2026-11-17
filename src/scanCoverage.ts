/** Coverage is separate from whether a scan finished or was saved. Restricted system
 *  locations (SIP/TCC) and excluded mounts are expected on macOS and can't be worked
 *  around, so only other problems make coverage "limited". */
export function coverageLabel(scan: {
  incomplete?: boolean;
  errors: number;
  restrictedSystemIssues?: number;
}): "Limited coverage" | null {
  const restricted = Math.min(
    scan.errors,
    Math.max(0, scan.restrictedSystemIssues ?? 0),
  );
  if (scan.errors > restricted || (scan.incomplete && scan.errors === 0))
    return "Limited coverage";
  return null;
}
