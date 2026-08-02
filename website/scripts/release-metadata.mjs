import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const exec = promisify(execFile);

export const allowedLicenses = new Set([
  '0BSD',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'BSD',
  'BlueOak-1.0.0',
  'BSL-1.0',
  'CC-BY-4.0',
  'CC0-1.0',
  'ISC',
  'LGPL-3.0-or-later',
  'LLVM-exception',
  'MIT',
  'MIT OR Apache-2.0',
  'MPL-2.0',
  'Python-2.0',
  'Unicode-3.0',
  'Unlicense',
  'Zlib',
]);

export async function dependencyInventory() {
  const [{ stdout: pnpmOutput }, { stdout: cargoOutput }] = await Promise.all([
    exec('pnpm', ['licenses', 'list', '--json'], { maxBuffer: 32 * 1024 * 1024 }),
    exec('cargo', ['metadata', '--locked', '--format-version', '1'], {
      cwd: '..',
      maxBuffer: 64 * 1024 * 1024,
    }),
  ]);
  return {
    cargo: JSON.parse(cargoOutput).packages,
    npm: JSON.parse(pnpmOutput),
  };
}

export function licenseFailures(inventory) {
  const failures = [];
  for (const [license, packages] of Object.entries(inventory.npm)) {
    if (!allowedLicenses.has(license)) {
      failures.push(`npm license ${license}: ${packages.map((item) => item.name).join(', ')}`);
    }
  }
  for (const pkg of inventory.cargo) {
    if (!pkg.license) failures.push(`Cargo package without license metadata: ${pkg.name}@${pkg.version}`);
    else if (!licenseExpressionAllowed(pkg.license)) {
      failures.push(`Cargo license ${pkg.license}: ${pkg.name}@${pkg.version}`);
    }
  }
  return failures;
}

export function componentsFrom(inventory) {
  const components = [];
  for (const [license, packages] of Object.entries(inventory.npm)) {
    for (const pkg of packages) {
      for (const version of pkg.versions) {
        components.push({
          type: 'library',
          group: pkg.name.startsWith('@') ? pkg.name.slice(1).split('/')[0] : undefined,
          name: pkg.name.startsWith('@') ? pkg.name.split('/')[1] : pkg.name,
          version,
          licenses: [{ license: { id: license } }],
          purl: `pkg:npm/${encodeURIComponent(pkg.name)}@${encodeURIComponent(version)}`,
        });
      }
    }
  }
  for (const pkg of inventory.cargo) {
    components.push({
      type: 'library',
      name: pkg.name,
      version: pkg.version,
      ...(pkg.license ? { licenses: [{ expression: pkg.license }] } : {}),
      purl: `pkg:cargo/${encodeURIComponent(pkg.name)}@${encodeURIComponent(pkg.version)}`,
    });
  }
  return components
    .sort((left, right) => left.purl.localeCompare(right.purl))
    .filter((component, index, all) => index === 0 || component.purl !== all[index - 1].purl);
}

function licenseExpressionAllowed(expression) {
  if (allowedLicenses.has(expression)) return true;
  const identifiers = expression.match(/[A-Za-z0-9.-]+/gu) ?? [];
  return identifiers
    .filter((identifier) => identifier !== 'AND' && identifier !== 'OR' && identifier !== 'WITH')
    .every((identifier) => allowedLicenses.has(identifier));
}
