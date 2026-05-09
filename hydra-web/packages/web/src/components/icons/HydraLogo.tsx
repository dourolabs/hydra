export function HydraLogo({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      {/* Three-headed serpent silhouette.
          Bold filled shape optimized for 16–32px rendering. */}
      <path
        fill="currentColor"
        d={[
          // Left head (diamond/fang shape)
          "M3.5 5.5 L5.5 2 L7.5 5.5 L5.5 7.5Z",
          // Left neck
          "M4.5 6.5 Q5 10 8 12 L10.5 13.5 L10.5 12 Q8 10.5 6.5 6.5Z",
          // Center head
          "M10 3 L12 0.5 L14 3 L12 5Z",
          // Center neck
          "M11 3.5 L11 13.5 L13 13.5 L13 3.5Z",
          // Right head (diamond/fang shape)
          "M16.5 5.5 L18.5 2 L20.5 5.5 L18.5 7.5Z",
          // Right neck
          "M17.5 6.5 Q17 10.5 13.5 13.5 L13.5 12 Q16 10 19.5 6.5Z",
          // Body trunk
          "M10.5 13 L10.5 20 L13.5 20 L13.5 13Z",
          // Forked tail
          "M10.5 20 L9 23 L11 22 L12 23.5 L13 22 L15 23 L13.5 20Z",
        ].join(" ")}
      />
    </svg>
  );
}
