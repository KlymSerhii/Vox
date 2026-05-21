export function clamp(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.max(0, Math.min(1, n));
}

// Soft sine bell. Returns ~0 at i=0 and i=n-1, peaks at 1 in the
// middle. Used as an envelope on both height and opacity so the
// waveform fades into the pill rim.
export function edgeEnvelope(i: number, n: number): number {
  const t = (i + 0.5) / n;
  return Math.sin(Math.PI * t);
}
