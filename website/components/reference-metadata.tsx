import { sourceCommit } from '@/lib/version';

type ReferenceMetadataProps = {
  pmokeVersion: string;
  schemaVersion: number;
};

export function ReferenceMetadata({ pmokeVersion, schemaVersion }: ReferenceMetadataProps) {
  return (
    <dl className="reference-metadata" aria-label="Reference metadata">
      <div>
        <dt>pmoke</dt>
        <dd>{pmokeVersion}</dd>
      </div>
      <div>
        <dt>config schema</dt>
        <dd>{schemaVersion}</dd>
      </div>
      <div>
        <dt>source</dt>
        <dd title={sourceCommit}>{sourceCommit.slice(0, 12)}</dd>
      </div>
    </dl>
  );
}
