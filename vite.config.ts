import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parse as parseToml } from "smol-toml";
import { defineConfig } from "vite";

/** Raw server TOML data used for UI defaults. */
interface ServerTomlConfig {
  host?: string;
  port?: number;
  uiHost?: string;
  uiPort?: number;
  publicWsUrl?: string;
}

/** Resolved UI defaults derived from the TOML config. */
interface UiDefaults {
  uiHost: string;
  uiPort: number;
  publicWsUrl: string;
}

/**
 * Resolve the server TOML config path for UI defaults.
 * @returns Absolute config path.
 */
function resolveServerConfigPath(): string {
  const configPath = process.env.SERVER_CONFIG ?? "server/config.toml";
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
 * Resolve UI defaults from the server TOML config.
 * @param raw - Raw TOML config data.
 * @returns Resolved UI defaults.
 */
function resolveUiDefaults(raw: ServerTomlConfig): UiDefaults {
  const defaultServerHost = "127.0.0.1";
  const defaultServerPort = 5174;
  const defaultUiHost = "127.0.0.1";
  const defaultUiPort = 5173;

  const uiHost =
    typeof raw.uiHost === "string" && raw.uiHost.trim()
      ? raw.uiHost.trim()
      : defaultUiHost;
  const uiPort = coercePort(raw.uiPort, defaultUiPort);

  const serverHost =
    typeof raw.host === "string" && raw.host.trim()
      ? raw.host.trim()
      : defaultServerHost;
  const serverPort = coercePort(raw.port, defaultServerPort);

  const publicWsUrl =
    typeof raw.publicWsUrl === "string" && raw.publicWsUrl.trim()
      ? raw.publicWsUrl.trim()
      : "";
  const clientHost =
    serverHost === "0.0.0.0" || serverHost === "::" ? "localhost" : serverHost;
  const resolvedPublicWsUrl = publicWsUrl || `ws://${clientHost}:${serverPort}`;

  return {
    uiHost,
    uiPort,
    publicWsUrl: resolvedPublicWsUrl
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

  return {
    // Fix EPERM/locking issues on network drives by moving cache to local temp
    cacheDir: path.join(os.tmpdir(), "slither-neuroevo-vite-cache"),
    root: ".",
    build: {
      outDir: "dist",
      emptyOutDir: true
    },
    define: {
      __SLITHER_DEFAULT_WS_URL__: JSON.stringify(defaults.publicWsUrl)
    },
    server: {
      open: true,
      host: defaults.uiHost,
      port: defaults.uiPort
    }
  };
}

export default defineConfig(buildViteConfig);
