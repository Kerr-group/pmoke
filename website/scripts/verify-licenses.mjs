import { dependencyInventory, licenseFailures } from './release-metadata.mjs';

const inventory = await dependencyInventory();
const failures = licenseFailures(inventory);
if (failures.length > 0) throw new Error(failures.join('\n'));
console.log(
  `Verified dependency licenses across ${inventory.cargo.length} Cargo packages and ${Object.keys(inventory.npm).length} npm license groups.`,
);
