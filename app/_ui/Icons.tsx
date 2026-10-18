import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function Icon({ size = 14, children, strokeWidth = 2, ...rest }: IconProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const CloseIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M18 6 6 18" />
    <path d="m6 6 12 12" />
  </Icon>
);

export const CheckIcon = (p: IconProps) => (
  <Icon strokeWidth={3} {...p}>
    <path d="M20 6 9 17l-5-5" />
  </Icon>
);

export const PlusIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M12 5v14" />
    <path d="M5 12h14" />
  </Icon>
);

export const TrashIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M3 6h18" />
    <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
    <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
  </Icon>
);

export const PencilIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
  </Icon>
);

export const CopyIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect width="13" height="13" x="9" y="9" rx="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  </Icon>
);

export const GearIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </Icon>
);

export const ExpandIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M8 3H3v5" />
    <path d="m3 3 6 6" />
    <path d="M16 3h5v5" />
    <path d="m21 3-6 6" />
    <path d="M8 21H3v-5" />
    <path d="m3 21 6-6" />
    <path d="M16 21h5v-5" />
    <path d="m21 21-6-6" />
  </Icon>
);

export const CollapseIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M9 3v6H3" />
    <path d="m3 3 6 6" />
    <path d="M15 3v6h6" />
    <path d="m21 3-6 6" />
    <path d="M9 21v-6H3" />
    <path d="m3 21 6-6" />
    <path d="M15 21v-6h6" />
    <path d="m21 21-6-6" />
  </Icon>
);

export const RefreshIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M20 11a8 8 0 0 0-14.7-4L3 10" />
    <path d="M3 4v6h6" />
    <path d="M4 13a8 8 0 0 0 14.7 4L21 14" />
    <path d="M21 20v-6h-6" />
  </Icon>
);

export const PowerIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M12 2v10" />
    <path d="M18.4 6.6a8 8 0 1 1-12.8 0" />
  </Icon>
);
