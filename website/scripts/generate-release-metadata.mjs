import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { componentsFrom, dependencyInventory, licenseFailures } from './release-metadata.mjs';

const output = path.resolve('out');
const metadata = path.join(output, '_meta');
const inventory = await dependencyInventory();
const failures = licenseFailures(inventory);
if (failures.length > 0) throw new Error(failures.join('\n'));

const sbom = {
  bomFormat: 'CycloneDX',
  specVersion: '1.6',
  version: 1,
  metadata: {
    component: {
      type: 'application',
      name: 'pmoke-documentation',
      version: process.env.GITHUB_SHA ?? 'development',
    },
  },
  components: componentsFrom(inventory),
};
await mkdir(metadata, { recursive: true });
await writeFile(path.join(metadata, 'sbom.cdx.json'), `${JSON.stringify(sbom, null, 2)}\n`);

const files = (await recursiveFiles(output))
  .filter((file) => path.basename(file) !== 'SHA256SUMS')
  .sort((left, right) => left.localeCompare(right));
const checksums = [];
for (const file of files) {
  const digest = createHash('sha256').update(await readFile(file)).digest('hex');
  checksums.push(`${digest}  ${path.relative(output, file).split(path.sep).join('/')}`);
}
await writeFile(path.join(output, 'SHA256SUMS'), `${checksums.join('\n')}\n`);
console.log(`Generated CycloneDX SBOM with ${sbom.components.length} components and ${files.length} checksums.`);

async function recursiveFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return (
    await Promise.all(
      entries.map((entry) => {
        const target = path.join(directory, entry.name);
        return entry.isDirectory() ? recursiveFiles(target) : [target];
      }),
    )
  ).flat();
}
