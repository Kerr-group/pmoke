import Image from 'next/image';
import { basePath } from '@/lib/shared';

export function BrandIcon({ size = 22 }: { size?: number }) {
  return (
    <Image
      className="brand-icon"
      src={`${basePath}/pmoke_icon.png`}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
