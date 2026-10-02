import type { SVGProps } from 'react'

// 簡單的線條圖示(不引入圖示套件)。純裝飾,一律 aria-hidden;意思要用文字表達
const base: SVGProps<SVGSVGElement> = {
  width: 18,
  height: 18,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
  'aria-hidden': true,
  focusable: false,
}

type P = SVGProps<SVGSVGElement>

export const CalendarIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="3" y="5" width="18" height="16" rx="3" />
    <path d="M3 10h18M8 3v4M16 3v4" />
  </svg>
)

export const ClockIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l3 2" />
  </svg>
)

export const UserIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="12" cy="8" r="4" />
    <path d="M4 21c0-4 3.6-6 8-6s8 2 8 6" />
  </svg>
)

export const CheckIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M5 12.5l4.5 4.5L19 7.5" />
  </svg>
)

export const MailIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="3" y="5" width="18" height="14" rx="3" />
    <path d="M3.5 7.5L12 13l8.5-5.5" />
  </svg>
)

export const ArrowRightIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M5 12h14M13 6l6 6-6 6" />
  </svg>
)

export const ChevronLeftIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M15 5l-7 7 7 7" />
  </svg>
)

export const ChevronRightIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M9 5l7 7-7 7" />
  </svg>
)

export const XIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M6 6l12 12M18 6L6 18" />
  </svg>
)

export const HomeIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M3 11l9-8 9 8M5 10v10h14V10" />
  </svg>
)

export const ListIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M9 6h12M9 12h12M9 18h12M4 6h.01M4 12h.01M4 18h.01" />
  </svg>
)

export const TagIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M3 12V4h8l10 10-8 8L3 12z" />
    <circle cx="7.5" cy="8.5" r="1" />
  </svg>
)

export const UsersIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="9" cy="8" r="3.5" />
    <path d="M2.5 20c0-3.6 2.9-5.5 6.5-5.5s6.5 1.9 6.5 5.5M16 4.6a3.5 3.5 0 010 6.8M18 14.8c2.2.6 3.5 2.2 3.5 5.2" />
  </svg>
)

export const ShieldIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M12 3l8 3v6c0 4.5-3.2 8-8 9-4.8-1-8-4.5-8-9V6l8-3z" />
    <path d="M9 12l2 2 4-4" />
  </svg>
)

export const CardIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="2.5" y="5" width="19" height="14" rx="3" />
    <path d="M2.5 10h19M6.5 15h4" />
  </svg>
)

export const LogOutIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M14 4h4a2 2 0 012 2v12a2 2 0 01-2 2h-4M10 8l-4 4 4 4M6 12h10" />
  </svg>
)

export const PlusIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M12 5v14M5 12h14" />
  </svg>
)

export const ExternalLinkIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5" />
  </svg>
)

export const CopyIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="9" y="9" width="11" height="11" rx="2.5" />
    <path d="M5 15V6.5A2.5 2.5 0 017.5 4H15" />
  </svg>
)

export const SwitchIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M7 4L3 8l4 4M3 8h14M17 20l4-4-4-4M21 16H7" />
  </svg>
)
