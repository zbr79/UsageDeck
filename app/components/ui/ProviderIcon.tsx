import Image from "next/image";

export default function ProviderIcon({
  src,
  alt = "",
  className,
}: {
  src: string;
  alt?: string;
  className?: string;
}) {
  return (
    <Image
      src={src}
      alt={alt}
      width={14}
      height={14}
      unoptimized
      className={className}
    />
  );
}
