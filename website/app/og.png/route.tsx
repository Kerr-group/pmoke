import { ImageResponse } from 'next/og';

export const dynamic = 'force-static';

const size = { width: 1200, height: 630 };
const traces = [
  { color: '#16d9d1', top: 392, turns: [-8, 17, -20, 13, -5, 21, -14, 8] },
  { color: '#ed4f9a', top: 452, turns: [13, -12, 19, -16, 10, -8, 16, -11] },
] as const;

export function GET() {
  return new ImageResponse(
    <div
      style={{
        position: 'relative',
        display: 'flex',
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        background: '#101517',
        color: '#f2f5f3',
        fontFamily: 'sans-serif',
      }}
    >
      {Array.from({ length: 11 }, (_, index) => (
        <div
          key={`vertical-${index}`}
          style={{ position: 'absolute', left: index * 120, top: 0, width: 1, height: 630, background: '#263235' }}
        />
      ))}
      {Array.from({ length: 6 }, (_, index) => (
        <div
          key={`horizontal-${index}`}
          style={{ position: 'absolute', left: 0, top: index * 126, width: 1200, height: 1, background: '#263235' }}
        />
      ))}
      <div style={{ position: 'absolute', left: 70, top: 62, display: 'flex', color: '#16d9d1', fontSize: 24 }}>
        PULSED-FIELD MOKE / REPRODUCIBLE MEASUREMENT
      </div>
      <div style={{ position: 'absolute', left: 64, top: 126, display: 'flex', fontSize: 150, fontWeight: 800 }}>
        pmoke
      </div>
      <div style={{ position: 'absolute', left: 72, top: 300, display: 'flex', fontSize: 31, color: '#aebbb8' }}>
        Capture the field pulse · rotate the phase · extract the Kerr angle
      </div>
      {traces.flatMap((trace, traceIndex) =>
        trace.turns.map((turn, index) => (
          <div
            key={`${trace.color}-${index}`}
            style={{
              position: 'absolute',
              left: 68 + index * 132,
              top: trace.top + turn,
              width: 142,
              height: 4,
              background: trace.color,
              transform: `rotate(${trace.turns[(index + 1) % trace.turns.length] - turn}deg)`,
              transformOrigin: 'left center',
              opacity: traceIndex === 0 ? 0.95 : 0.78,
            }}
          />
        )),
      )}
      <div style={{ position: 'absolute', right: 70, bottom: 52, display: 'flex', color: '#55d187', fontSize: 22 }}>
        RUST · WASM · STATIC
      </div>
    </div>,
    size,
  );
}
