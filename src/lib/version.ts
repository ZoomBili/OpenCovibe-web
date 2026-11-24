/**
 * App version, sourced from package.json at build time.
 *
 * Keeping package.json as the single source of truth prevents the sidebar
 * version from drifting from the release/FPK version.
 */
import pkg from "../../package.json";

export const APP_VERSION: string = pkg.version;
