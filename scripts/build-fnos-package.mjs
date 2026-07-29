import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { gzipSync, gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDir, "..");

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) throw new Error(`Unknown argument: ${arg}`);
    const key = arg.slice(2);
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value`);
    args[key] = value;
    index += 1;
  }
  return args;
}

function writeString(buffer, offset, length, value) {
  const encoded = Buffer.from(value, "utf8");
  if (encoded.length > length) throw new Error(`Tar field is too long: ${value}`);
  encoded.copy(buffer, offset);
}

function writeOctal(buffer, offset, length, value) {
  const encoded = value.toString(8).padStart(length - 1, "0") + "\0";
  writeString(buffer, offset, length, encoded);
}

function tarHeader(entry) {
  const header = Buffer.alloc(512, 0);
  writeString(header, 0, 100, entry.name);
  writeOctal(header, 100, 8, entry.mode);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, entry.data.length);
  writeOctal(header, 136, 12, 0);
  header.fill(0x20, 148, 156);
  writeString(header, 156, 1, entry.directory ? "5" : "0");
  writeString(header, 257, 6, "ustar\0");
  writeString(header, 263, 2, "00");
  writeString(header, 265, 32, "root");
  writeString(header, 297, 32, "root");
  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  writeString(header, 148, 8, checksum.toString(8).padStart(6, "0") + "\0 ");
  return header;
}

async function directoryEntries(rootPath) {
  const entries = [];

  async function walk(currentPath) {
    const children = await readdir(currentPath, { withFileTypes: true });
    children.sort((left, right) => left.name.localeCompare(right.name));
    for (const child of children) {
      const fullPath = join(currentPath, child.name);
      const archivePath = relative(rootPath, fullPath).split(sep).join("/");
      if (child.isDirectory()) {
        entries.push({
          name: `${archivePath}/`,
          directory: true,
          mode: 0o755,
          data: Buffer.alloc(0),
        });
        await walk(fullPath);
      } else if (child.isFile()) {
        const executable = archivePath.startsWith("cmd/") || archivePath.startsWith("server/");
        const data = await readFile(fullPath);
        if (archivePath.startsWith("cmd/") && data.includes(0x0d)) {
          throw new Error(`fnOS command script must use LF line endings: ${fullPath}`);
        }
        entries.push({
          name: archivePath,
          directory: false,
          mode: executable ? 0o755 : 0o644,
          data,
        });
      } else {
        throw new Error(`Unsupported package entry: ${fullPath}`);
      }
    }
  }

  await walk(rootPath);
  return entries;
}

async function createTarGz(rootPath) {
  const chunks = [];
  for (const entry of await directoryEntries(rootPath)) {
    chunks.push(tarHeader(entry));
    if (!entry.directory) {
      chunks.push(entry.data);
      const padding = (512 - (entry.data.length % 512)) % 512;
      if (padding) chunks.push(Buffer.alloc(padding, 0));
    }
  }
  chunks.push(Buffer.alloc(1024, 0));
  return gzipSync(Buffer.concat(chunks), { level: 9, mtime: 0 });
}

function readTarFile(archive, wantedName) {
  const tar = gunzipSync(archive);
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/, "");
    const sizeText = header.subarray(124, 136).toString("ascii").replace(/\0.*$/, "").trim();
    const size = Number.parseInt(sizeText || "0", 8);
    const dataOffset = offset + 512;
    if (basename(name) === wantedName)
      return Buffer.from(tar.subarray(dataOffset, dataOffset + size));
    offset = dataOffset + Math.ceil(size / 512) * 512;
  }
  throw new Error(`${wantedName} was not found in release archive`);
}

function validateElf(binary, expectedMachine, label) {
  if (binary.length < 64 || binary.subarray(0, 4).toString("hex") !== "7f454c46") {
    throw new Error(`${label} input is not an ELF binary`);
  }
  const machine = binary.readUInt16LE(18);
  if (machine !== expectedMachine) {
    throw new Error(`${label} ELF machine is ${machine}, expected ${expectedMachine}`);
  }
}

async function loadBinary(args, version, arch) {
  const explicit = args[arch.key];
  const targetPath = join(root, "src-tauri", "target", arch.target, "release", "opencovibe-server");
  if (explicit) return readFile(resolve(explicit));
  if (existsSync(targetPath)) return readFile(targetPath);

  const archivePath = join(
    root,
    "dist-release",
    `opencovibe-server_v${version}_linux_${arch.key}.tar.gz`,
  );
  if (!existsSync(archivePath)) {
    throw new Error(`Missing ${arch.key} binary. Build releases first or pass --${arch.key} PATH.`);
  }
  return readTarFile(await readFile(archivePath), "opencovibe-server");
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const packageJson = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
  const version = String(args.version || packageJson.version).replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`Invalid version: ${version}`);

  const architectures = [
    { key: "amd64", target: "x86_64-unknown-linux-gnu", machine: 62, output: "x86_64" },
    { key: "arm64", target: "aarch64-unknown-linux-gnu", machine: 183, output: "aarch64" },
  ];
  const binaries = new Map();
  for (const architecture of architectures) {
    const binary = await loadBinary(args, version, architecture);
    validateElf(binary, architecture.machine, architecture.key);
    binaries.set(architecture.output, binary);
  }

  const temporaryRoot = await mkdtemp(join(tmpdir(), "opencovibe-fnos-"));
  try {
    const appRoot = join(temporaryRoot, "app");
    const packageRoot = join(temporaryRoot, "package");
    await mkdir(join(appRoot, "server"), { recursive: true });
    await mkdir(join(appRoot, "ui", "images"), { recursive: true });
    await cp(join(root, "fnos", "app", "ui", "config"), join(appRoot, "ui", "config"));
    await cp(
      join(root, "src-tauri", "icons", "64x64.png"),
      join(appRoot, "ui", "images", "icon_64.png"),
    );
    await cp(
      join(root, "src-tauri", "icons", "128x128@2x.png"),
      join(appRoot, "ui", "images", "icon_256.png"),
    );
    for (const [architecture, binary] of binaries) {
      await writeFile(join(appRoot, "server", `opencovibe-server-${architecture}`), binary);
    }

    await mkdir(packageRoot, { recursive: true });
    await cp(join(root, "fnos", "cmd"), join(packageRoot, "cmd"), { recursive: true });
    await cp(join(root, "fnos", "config"), join(packageRoot, "config"), { recursive: true });
    await cp(join(root, "fnos", "wizard"), join(packageRoot, "wizard"), { recursive: true });
    await cp(join(root, "src-tauri", "icons", "128x128@2x.png"), join(packageRoot, "ICON.PNG"));
    await cp(join(root, "src-tauri", "icons", "128x128@2x.png"), join(packageRoot, "ICON_256.PNG"));

    const appArchive = await createTarGz(appRoot);
    await writeFile(join(packageRoot, "app.tgz"), appArchive);
    const checksum = createHash("md5").update(appArchive).digest("hex");
    const manifest = (await readFile(join(root, "fnos", "manifest.template"), "utf8"))
      .replaceAll("__VERSION__", version)
      .replaceAll("__CHECKSUM__", checksum);
    await writeFile(join(packageRoot, "manifest"), manifest, "utf8");

    const outputDirectory = resolve(args.output || join(root, "dist-fnos"));
    await mkdir(outputDirectory, { recursive: true });
    const outputPath = join(outputDirectory, `opencovibe-web_v${version}.fpk`);
    const packageArchive = await createTarGz(packageRoot);
    await writeFile(outputPath, packageArchive);
    const packageChecksum = createHash("sha256").update(packageArchive).digest("hex");
    await writeFile(
      `${outputPath}.sha256`,
      `${packageChecksum}  ${basename(outputPath)}\n`,
      "utf8",
    );
    const outputStat = await stat(outputPath);
    console.log(`[OpenCovibe] fnOS package: ${outputPath}`);
    console.log(`[OpenCovibe] Version: v${version}`);
    console.log(`[OpenCovibe] app.tgz MD5: ${checksum}`);
    console.log(`[OpenCovibe] Package SHA256: ${packageChecksum}`);
    console.log(`[OpenCovibe] Size: ${outputStat.size} bytes`);
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(`[OpenCovibe] ${error.message}`);
  process.exitCode = 1;
});
