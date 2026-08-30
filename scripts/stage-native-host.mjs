import { execFileSync } from "node:child_process";
import { access, copyFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const rustcMetadata = execFileSync("rustc", ["-vV"], {
  encoding: "utf8",
});
const targetTriple = /^host:\s+(.+)$/m.exec(rustcMetadata)?.[1]?.trim();

if (!targetTriple) {
  throw new Error("Unable to determine the Rust host target triple");
}

const executableSuffix = process.platform === "win32" ? ".exe" : "";
const targetRoot = resolve(process.env.CARGO_TARGET_DIR ?? "target");
const source = resolve(
  targetRoot,
  "release",
  `callback-native-host${executableSuffix}`,
);
const destination = resolve(
  "src-tauri",
  "binaries",
  `callback-native-host-${targetTriple}${executableSuffix}`,
);

await access(source);
await mkdir(dirname(destination), { recursive: true });
await copyFile(source, destination);

console.log(`Staged native host: ${destination}`);
