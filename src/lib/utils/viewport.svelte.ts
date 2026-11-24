/**
 * Shared viewport state for responsive behavior.
 *
 * Most of the mobile adaptation lives in CSS (Tailwind breakpoints + the `touch:` variant),
 * but a handful of behaviors need JS:
 *   - the app shell's off-canvas sidebar (and closing it after navigation)
 *   - the chat "tool activity" panel (inline panel on desktop, drawer on phones)
 *   - Enter-to-send vs Enter-inserts-newline on touch keyboards
 *
 * The listeners are a process-wide singleton attached once at module load: the SPA never
 * tears them down, and no component owns the cleanup (a route component unmounting must not
 * freeze the shared state). SSR is disabled for this app, but every access is guarded.
 */

const MOBILE_QUERY = "(max-width: 1023px)";
const NARROW_QUERY = "(max-width: 639px)";
const COARSE_POINTER_QUERY = "(pointer: coarse)";

function matches(query: string): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia(query).matches;
}

export const viewport = $state({
  /** Phones + small tablets: off-canvas sidebars, compact chrome. Matches Tailwind `lg`. */
  mobile: matches(MOBILE_QUERY),
  /** Below Tailwind's `sm` breakpoint — the tightest layout tier. */
  narrow: matches(NARROW_QUERY),
  /** Coarse pointer (touch). Used for hit-target sizing and the Enter key behavior. */
  touch: matches(COARSE_POINTER_QUERY),
});

function sync(): void {
  viewport.mobile = matches(MOBILE_QUERY);
  viewport.narrow = matches(NARROW_QUERY);
  viewport.touch = matches(COARSE_POINTER_QUERY);
}

if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
  for (const query of [MOBILE_QUERY, NARROW_QUERY, COARSE_POINTER_QUERY]) {
    window.matchMedia(query).addEventListener("change", sync);
  }
  // Late import (e.g. after a client-side navigation) still starts from real values.
  sync();
}
