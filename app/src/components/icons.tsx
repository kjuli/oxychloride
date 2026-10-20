// 16px stroke icons drawn for this app (no icon font dependency).

import type { ReactElement, ReactNode, SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Svg(props: IconProps & { children: ReactNode }): ReactElement {
  const { children, ...rest } = props;
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
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

export const IconNew = (p: IconProps) => (
  <Svg {...p}>
    <path d="M9 1.75H4.25a1 1 0 0 0-1 1v10.5a1 1 0 0 0 1 1h7.5a1 1 0 0 0 1-1V5.5z" />
    <path d="M9 1.75V5.5h3.75M8 8v4M6 10h4" />
  </Svg>
);

export const IconOpen = (p: IconProps) => (
  <Svg {...p}>
    <path d="M1.75 12.5V3.25a1 1 0 0 1 1-1h3.1l1.5 1.75h5.4a1 1 0 0 1 1 1V6" />
    <path d="M1.75 12.5 3.6 7.1a1 1 0 0 1 .95-.68h9.6a.6.6 0 0 1 .57.8l-1.8 5.3a1 1 0 0 1-.95.68H2.75a1 1 0 0 1-1-1z" />
  </Svg>
);

export const IconSave = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2.25 3.25a1 1 0 0 1 1-1h7.6l2.9 2.9v7.6a1 1 0 0 1-1 1h-9.5a1 1 0 0 1-1-1z" />
    <path d="M5 2.25v3h5v-3M4.75 13.75V9.5h6.5v4.25" />
  </Svg>
);

export const IconSaveAs = (p: IconProps) => (
  <Svg {...p}>
    <path d="M8.5 13.75H3.25a1 1 0 0 1-1-1v-9.5a1 1 0 0 1 1-1h7.6l2.9 2.9V8" />
    <path d="M5 2.25v3h5v-3M10.5 14.25l.4-1.6 3-3a.85.85 0 0 1 1.2 1.2l-3 3z" />
  </Svg>
);

/** A hexagon: metamodels (the atacamite crystal of the brand mark). */
export const IconMetamodel = (p: IconProps) => (
  <Svg {...p}>
    <path d="M8 1.6 13.55 4.8v6.4L8 14.4 2.45 11.2V4.8z" />
    <path d="M8 1.6v6.4m0 0 5.55-3.2M8 8 2.45 4.8" />
  </Svg>
);

/** Connected nodes: instance models. */
export const IconModel = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="4" cy="4" r="1.9" />
    <circle cx="12" cy="6.5" r="1.9" />
    <circle cx="5.5" cy="12.25" r="1.9" />
    <path d="M5.8 4.6 10.2 5.9M4.35 5.85l.8 4.5M10.5 7.8 7.2 11.1" />
  </Svg>
);

/** A chain link: URI mappings. */
export const IconLink = (p: IconProps) => (
  <Svg {...p}>
    <path d="M6.6 9.4a2.6 2.6 0 0 0 3.7 0l2.4-2.4a2.6 2.6 0 0 0-3.7-3.7l-.9.9" />
    <path d="M9.4 6.6a2.6 2.6 0 0 0-3.7 0L3.3 9a2.6 2.6 0 0 0 3.7 3.7l.9-.9" />
  </Svg>
);

/** A box with an arrow leaving it: export to another format. */
export const IconExport = (p: IconProps) => (
  <Svg {...p}>
    <path d="M9.5 2.25h4.25V6.5M13.75 2.25 7.5 8.5" />
    <path d="M12 9.5v3.25a1 1 0 0 1-1 1H3.25a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1H6.5" />
  </Svg>
);

export const IconReload = (p: IconProps) => (
  <Svg {...p}>
    <path d="M13.25 8A5.25 5.25 0 1 1 11.6 4.2" />
    <path d="M12.25 1.75v3h-3" />
  </Svg>
);

export const IconCheck = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2.5 8.5 6 12l7.5-8" />
  </Svg>
);

export const IconRun = (p: IconProps) => (
  <Svg {...p} fill="currentColor" stroke="none">
    <path d="M4.5 2.6v10.8a.6.6 0 0 0 .9.52l8.6-5.4a.6.6 0 0 0 0-1.03L5.4 2.08a.6.6 0 0 0-.9.52z" />
  </Svg>
);

export const IconReset = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2.5 4.25h11M6.25 4.25V2.5h3.5v1.75M3.75 4.25l.7 9a1 1 0 0 0 1 .92h5.1a1 1 0 0 0 1-.92l.7-9" />
  </Svg>
);

export const IconClose = (p: IconProps) => (
  <Svg {...p} width="12" height="12">
    <path d="M4 4l8 8M12 4l-8 8" />
  </Svg>
);

export const IconChevron = (p: IconProps) => (
  <Svg {...p} width="12" height="12">
    <path d="M6 4l4 4-4 4" />
  </Svg>
);

export const IconTarget = (p: IconProps) => (
  <Svg {...p} width="12" height="12">
    <circle cx="8" cy="8" r="5.5" />
    <circle cx="8" cy="8" r="2" fill="currentColor" stroke="none" />
  </Svg>
);

// ---------------------------------------------------------------- status glyphs

export const GlyphSatisfied = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="glyph glyph-satisfied">
    <circle cx="7" cy="7" r="6" fill="currentColor" />
    <path d="M4.2 7.2 6.1 9l3.7-4" fill="none" stroke="var(--on-status)" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

export const GlyphViolated = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="glyph glyph-violated">
    <circle cx="7" cy="7" r="6" fill="currentColor" />
    <path d="M4.8 4.8l4.4 4.4M9.2 4.8 4.8 9.2" stroke="var(--on-status)" strokeWidth="1.6" strokeLinecap="round" />
  </svg>
);

export const GlyphError = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="glyph glyph-error">
    <path d="M7 1.2 13.2 12.3H.8z" fill="currentColor" strokeLinejoin="round" />
    <path d="M7 5.2v3.4" stroke="var(--on-status)" strokeWidth="1.5" strokeLinecap="round" />
    <circle cx="7" cy="10.4" r=".85" fill="var(--on-status)" />
  </svg>
);

export const GlyphWarning = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="glyph glyph-warning">
    <path d="M7 1.2 13.2 12.3H.8z" fill="currentColor" />
    <path d="M7 5.2v3.4" stroke="var(--on-status)" strokeWidth="1.5" strokeLinecap="round" />
    <circle cx="7" cy="10.4" r=".85" fill="var(--on-status)" />
  </svg>
);

export const GlyphInfo = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="glyph glyph-info">
    <circle cx="7" cy="7" r="6" fill="currentColor" />
    <path d="M7 6.3v3.6" stroke="var(--on-status)" strokeWidth="1.5" strokeLinecap="round" />
    <circle cx="7" cy="4.3" r=".85" fill="var(--on-status)" />
  </svg>
);

export function SeverityGlyph({ severity }: { severity: "error" | "warning" | "info" }) {
  if (severity === "error") return <GlyphError />;
  if (severity === "warning") return <GlyphWarning />;
  return <GlyphInfo />;
}
