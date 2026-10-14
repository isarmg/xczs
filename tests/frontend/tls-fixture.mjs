import { spawnSync } from "node:child_process";
import { chmodSync, lstatSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Browser tests need a real HTTPS origin, but no reusable private key in source.
export function generateTestTls(directory) {
  const metadata = lstatSync(directory);
  if (!metadata.isDirectory() || metadata.isSymbolicLink() ||
      metadata.uid !== process.getuid() || (metadata.mode & 0o7777) !== 0o700) {
    throw new Error("TLS fixture directory must be owned and private");
  }
  const cert = join(directory, "cert.pem");
  const key = join(directory, "key.pem");
  for (const path of [cert, key]) {
    writeFileSync(path, "", { flag: "wx", mode: 0o600 });
  }
  const result = spawnSync("openssl", [
    "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "1",
    "-subj", "/CN=localhost", "-addext", "basicConstraints=critical,CA:FALSE",
    "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1",
    "-keyout", key, "-out", cert,
  ], { stdio: ["ignore", "ignore", "pipe"], timeout: 15_000, maxBuffer: 64 * 1024 });
  if (result.error || result.status !== 0) {
    throw new Error("OpenSSL could not generate the temporary TLS fixture");
  }
  for (const path of [cert, key]) chmodSync(path, 0o600);
  return { cert, key };
}
