/**
 * App version shown in the sidebar. `__APP_VERSION__` (the Cargo workspace
 * version, the single source of truth) and `__APP_BUILD__` (`git describe
 * --tags --always --dirty`) are injected by `vite.config.ts` at build time,
 * so the frontend always reports the version of the binary it ships in.
 */

declare const __APP_VERSION__: string;
declare const __APP_BUILD__: string;

export interface VersionLabel {
  label: string;
  tooltip: string;
}

export function formatVersionLabel(version: string, build: string, isDev: boolean): VersionLabel {
  return {
    label: `v${version}${isDev ? '-dev' : ''}`,
    tooltip: `Build ${build || 'unknown'}`,
  };
}

/** Undefined-safe so vitest (which doesn't load vite.config.ts) can import this module. */
export function appVersionLabel(): VersionLabel {
  const version = typeof __APP_VERSION__ === 'string' ? __APP_VERSION__ : '0.0.0';
  const build = typeof __APP_BUILD__ === 'string' ? __APP_BUILD__ : 'unknown';
  return formatVersionLabel(version, build, import.meta.env.DEV);
}
