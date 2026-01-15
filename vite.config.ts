import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parse as parseToml } from "smol-toml";
import { defineConfig } from "vite";

/** Raw server TOML data used for UI defaults. */
interface ServerTomlConfig {
  /** Simulation server bind host from TOML. */
  host?: string;
  /** Simulation server bind port from TOML. */
  port?: number;
  /** UI dev server bind host from TOML. */
  uiHost?: string;
  /** UI dev server bind port from TOML. */
  uiPort?: number;
  /** Optional default WebSocket URL override from TOML. */
  publicWsUrl?: string;
}

/** Resolved UI defaults derived from the TOML config. */
interface UiDefaults {
  /** Resolved UI dev server host. */
  uiHost: string;
  /** Resolved UI dev server port. */
  uiPort: number;
  /** Optional WebSocket URL override to inject. */
  publicWsUrl: string;
  /** Resolved simulation server port. */
  serverPort: number;
  /** Hostname to advertise for HMR when needed. */
  hmrHost: string | undefined;
}

/**
 * Resolve the server TOML config path for UI defaults.
 * @returns Absolute config path.
 */
function resolveServerConfigPath(): string {
  const configPath = process.env["SERVER_CONFIG"] ?? "server/config.toml";
  return path.resolve(process.cwd(), configPath);
}

/**
 * Load TOML config data used for UI defaults.
 * @param filePath - TOML path to read.
 * @returns Parsed config data or an empty object on failure.
 */
function loadTomlConfig(filePath: string): ServerTomlConfig {
  if (!fs.existsSync(filePath)) return {};
  const raw = fs.readFileSync(filePath, "utf8");
  if (!raw.trim()) return {};
  try {
    const parsed = parseToml(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as ServerTomlConfig;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    console.warn(`[vite] Failed to parse ${filePath}: ${message}`);
    return {};
  }
}

/**
 * Coerce a TOML value into a usable port.
 * @param value - Raw TOML value.
 * @param fallback - Fallback port when invalid.
 * @returns Parsed port number.
 */
function coercePort(value: unknown, fallback: number): number {
  const parsed =
    typeof value === "number"
      ? Math.trunc(value)
      : typeof value === "string"
      ? Number.parseInt(value, 10)
      : Number.NaN;
  if (!Number.isFinite(parsed)) return fallback;
  if (parsed < 1 || parsed > 65535) return fallback;
  return parsed;
}

/**
 * Extract a hostname from a WebSocket URL string.
 * @param url - Raw URL string.
 * @returns Hostname or null when parsing fails.
 */
function extractHostFromWsUrl(url: string): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url);
    return parsed.hostname || null;
  } catch {
    return null;
  }
}

/**
 * Find a LAN IPv4 address for HMR when bound to all interfaces.
 * @returns First non-internal IPv4 address or null.
 */
function pickLanIpv4(): string | null {
  const interfaces = os.networkInterfaces();
  for (const entries of Object.values(interfaces)) {
    if (!entries) continue;
    for (const entry of entries) {
      if (entry.family !== "IPv4") continue;
      if (entry.internal) continue;
      return entry.address;
    }
  }
  return null;
}

/**
 * Resolve UI defaults from the server TOML config.
 * @param raw - Raw TOML config data.
 * @returns Resolved UI defaults.
 */
function resolveUiDefaults(raw: ServerTomlConfig): UiDefaults {
  const defaultServerPort = 5174;
  const defaultUiHost = "127.0.0.1";
  const defaultUiPort = 5173;

  const uiHost =
    typeof raw.uiHost === "string" && raw.uiHost.trim()
      ? raw.uiHost.trim()
      : defaultUiHost;
  const uiPort = coercePort(raw.uiPort, defaultUiPort);

  const publicWsUrl =
    typeof raw.publicWsUrl === "string" && raw.publicWsUrl.trim()
      ? raw.publicWsUrl.trim()
      : "";
  const serverPort = coercePort(raw.port, defaultServerPort);
  const publicHost = extractHostFromWsUrl(publicWsUrl);
  const hmrHost =
    uiHost && uiHost !== "0.0.0.0" && uiHost !== "::"
      ? uiHost
      : publicHost || pickLanIpv4() || undefined;

  return {
    uiHost,
    uiPort,
    publicWsUrl,
    serverPort,
    hmrHost
  };
}

/**
 * Build the Vite configuration using server TOML defaults.
 * @returns Vite config object.
 */
function buildViteConfig() {
  const configPath = resolveServerConfigPath();
  const rawConfig = loadTomlConfig(configPath);
  const defaults = resolveUiDefaults(rawConfig);
  const serverConfig = {
    open: true,
    host: defaults.uiHost,
    port: defaults.uiPort
  } as {
    open: boolean;
    host: string;
    port: number;
    hmr?: { host: string };
  };

  if (defaults.hmrHost) {
    serverConfig.hmr = { host: defaults.hmrHost };
  }

  return {
    // Fix EPERM/locking issues on network drives by moving cache to local temp
    cacheDir: path.join(os.tmpdir(), "slither-neuroevo-vite-cache"),
    root: ".",
    build: {
      outDir: "dist",
      emptyOutDir: true
    },
    assetsInclude: ["**/*.wasm"],
    test: {
      setupFiles: ["src/test/vitest.setup.ts"]
    },
    define: {
      __SLITHER_DEFAULT_WS_URL__: JSON.stringify(defaults.publicWsUrl),
      __SLITHER_SERVER_PORT__: JSON.stringify(defaults.serverPort)
    },
    server: serverConfig
  };
}

export default defineConfig(buildViteConfig);
