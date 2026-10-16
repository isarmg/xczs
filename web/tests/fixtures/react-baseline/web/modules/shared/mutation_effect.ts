/** The only outcomes that write-capable UI modules may report to the listing.
 * `NOT_COMMITTED` is intentionally a no-op. `REFRESH_REQUIRED` means the
 * attempted write was rejected but also proved this listing snapshot stale;
 * it invalidates without claiming that our operation committed.
 */
export const MUTATION_EFFECT: Readonly<{
  COMMITTED: "committed";
  OUTCOME_UNKNOWN: "outcome-unknown";
  REFRESH_REQUIRED: "refresh-required";
  NOT_COMMITTED: "not-committed";
}> = Object.freeze({
  COMMITTED: "committed",
  OUTCOME_UNKNOWN: "outcome-unknown",
  REFRESH_REQUIRED: "refresh-required",
  NOT_COMMITTED: "not-committed",
});
