/** The scan's errors split into restricted system locations and everything else. */
export function issueSplit(scan: {
  errors: number;
  restrictedSystemIssues?: number;
}) {
  const restricted = Math.min(
    scan.errors,
    Math.max(0, scan.restrictedSystemIssues ?? 0),
  );
  return { restricted, other: Math.max(0, scan.errors - restricted) };
}

/** Coverage is separate from whether a scan finished. Restricted system
 *  locations (SIP/TCC) and excluded mounts are expected on macOS and can't be worked
 *  around, so only other problems make coverage "limited". */
export function coverageLabel(scan: {
  incomplete?: boolean;
  errors: number;
  restrictedSystemIssues?: number;
}): "Limited coverage" | null {
  if (issueSplit(scan).other > 0 || (scan.incomplete && scan.errors === 0))
    return "Limited coverage";
  return null;
}
