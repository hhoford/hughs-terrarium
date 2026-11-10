export type Glyph =
  | "god"
  | "swarm"
  | "queen"
  | "dragonfly"
  | "globe"
  | "pilot"
  | "plus"
  | "close"
  | "radar"
  | "camera"
  | "lock"
  | "unlock"
  | "strike"
  | "land"
  | "lift"
  | "energy"
  | "altitude"
  | "speed"
  | "warning"
  | "hold"
  | "reset"
  | "mineral"
  | "collect"
  | "deliver"
  | "dead"
  | "shield"
  | "mark";
export function Symbol({ name, size = 24 }: { name: Glyph; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {name === "god" ? (<><path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/><path d="M12 1v3M12 20v3"/></>) : name === "swarm" ? (<><path d="M12 5 5 18h14z"/><circle cx="12" cy="5" r="3" fill="currentColor"/><circle cx="5" cy="18" r="3" fill="currentColor"/><circle cx="19" cy="18" r="3" fill="currentColor"/></>) : name === "mark" ? (<><circle cx="12" cy="9" r="5"/><path d="m8 13 4 9 4-9M12 6v6M9 9h6"/></>) : name === "shield" ? (
        <>
          <path d="m12 2 9 4v6c0 5-5 9-9 11-4-2-9-6-9-11V6z" />
          <path d="m7 12 3 3 7-7" />
        </>
      ) : name === "mineral" ? (
        <>
          <path d="m12 2 9 7-3 11H6L3 9zM3 9h18M12 2 8 9l4 11 4-11z" />
        </>
      ) : name === "collect" ? (
        <>
          <path d="M3 14v7h18v-7M12 2v13m-5-5 5 5 5-5" />
        </>
      ) : name === "deliver" ? (
        <>
          <path d="M3 16v5h18v-5M12 16V3m-5 5 5-5 5 5" />
          <circle cx="19" cy="6" r="2" />
        </>
      ) : name === "dead" ? (
        <>
          <path d="M7 16h10l1-4a8 8 0 1 0-12 0zM9 16v5h6v-5M8 7l3 3m0-3-3 3m5-3 3 3m0-3-3 3" />
        </>
      ) : name === "queen" ? (
        <>
          <path d="M5 10 3 3l6 4 3-6 3 6 6-4-2 7" />
          <path d="m6 11-3 6 5-1 4 7 4-7 5 1-3-6" />
          <ellipse cx="12" cy="13" rx="5" ry="6" />
          <path d="M10 11h.1M14 11h.1" strokeWidth="3" />
        </>
      ) : name === "dragonfly" ? (
        <>
          <path d="M12 7v15" />
          <circle cx="12" cy="4" r="2" />
          <path d="M11 8C-1 0-1 12 11 11M13 8C25 0 25 12 13 11M11 12C1 9 1 19 11 15M13 12C23 9 23 19 13 15" />
        </>
      ) : name === "globe" ? (
        <>
          <circle cx="12" cy="12" r="10" />
          <ellipse cx="12" cy="12" rx="4.5" ry="10" />
          <path d="M2 12h20M4 6h16M4 18h16" />
        </>
      ) : name === "pilot" ? (
        <>
          <path d="m8 4 13 8-13 8z" />
          <path d="M3 5v14" />
        </>
      ) : name === "plus" ? (
        <path d="M12 4v16M4 12h16" />
      ) : name === "close" ? (
        <path d="m6 6 12 12M6 18 18 6" />
      ) : name === "radar" ? (
        <>
          <circle cx="12" cy="12" r="10" />
          <circle cx="12" cy="12" r="6" />
          <path d="m12 12 7-7M12 2v2M2 12h2M12 20v2M20 12h2" />
          <circle cx="9" cy="16" r="1" />
        </>
      ) : name === "camera" ? (
        <>
          <rect x="2" y="5" width="20" height="15" rx="3" />
          <circle cx="12" cy="12" r="4" />
          <path d="M7 5 9 2h6l2 3" />
        </>
      ) : name === "lock" || name === "unlock" ? (
        <>
          <rect x="5" y="10" width="14" height="12" rx="3" />
          {name === "lock" ? (
            <path d="M8 10V6a4 4 0 0 1 8 0v4" />
          ) : (
            <path d="M8 10V6a4 4 0 0 1 8 0" />
          )}
          <path d="M12 15v3" />
        </>
      ) : name === "strike" ? (
        <>
          <path d="M12 3c4 4 5 7 3 12H9C7 10 8 7 12 3Z" />
          <path d="m8 12-4 6 5-1M16 12l4 6-5-1M10 19l2 4 2-4" />
          <circle cx="12" cy="9" r="1.5" />
        </>
      ) : name === "land" || name === "lift" ? (
        <>
          <path d="M3 21h18M12 3v14" />
          {name === "land" ? (
            <path d="m7 12 5 5 5-5" />
          ) : (
            <path d="m7 8 5-5 5 5" />
          )}
        </>
      ) : name === "energy" ? (
        <path d="m13 2-9 12h7l-1 8 10-12h-7z" />
      ) : name === "altitude" ? (
        <>
          <path d="M4 21h16M12 18V3M7 8l5-5 5 5" />
        </>
      ) : name === "speed" ? (
        <>
          <path d="M3 15a9 9 0 1 1 18 0M12 12l5-5M6 17h12" />
          <circle cx="12" cy="12" r="1.5" />
        </>
      ) : name === "hold" ? (
        <>
          <circle cx="12" cy="12" r="9" />
          <path d="M9 8v8M15 8v8" />
        </>
      ) : name === "reset" ? (
        <>
          <path d="M4 10a8 8 0 1 1 1 7M4 3v7h7" />
        </>
      ) : (
        <>
          <path d="m12 2 11 19H1zM12 8v5M12 17h.1" />
        </>
      )}
    </svg>
  );
}
