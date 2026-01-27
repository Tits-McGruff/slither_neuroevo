/**
 * Runs the native dependency install using npm without relying on PATH resolution.
 *
 * @returns {void} No return value.
 */
function runPostinstall() {
  const { spawnSync } = require("node:child_process")
  const npmExecPath = process.env.npm_execpath

  const command = npmExecPath ? process.execPath : process.platform === "win32" ? "npm.cmd" : "npm"
  const args = npmExecPath ? [npmExecPath, "--prefix", "native", "install"] : ["--prefix", "native", "install"]

  const result = spawnSync(command, args, { stdio: "inherit" })
  if (result.error) {
    throw result.error
  }
  if (typeof result.status === "number") {
    process.exitCode = result.status
  } else {
    process.exitCode = 1
  }
}

runPostinstall()
