interface MetisLogoProps {
  size?: number;
  className?: string;
}

export function MetisLogo({ size = 24, className }: MetisLogoProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-label="Metis"
      role="img"
    >
      <path d="M4 28V4h4l8 12 8-12h4v24h-4V10l-8 12-8-12v18z" />
    </svg>
  );
}
