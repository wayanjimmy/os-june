import type { AudioLevelDto } from "../../lib/tauri";

type WaveformProps = {
  level: AudioLevelDto;
};

export function Waveform({ level }: WaveformProps) {
  const peaks =
    level.recentPeaks.length > 0
      ? level.recentPeaks
      : [level.peak, level.rms, level.peak];

  return (
    <div className="waveform" aria-label="Microphone activity">
      {peaks.slice(-24).map((peak, index) => (
        <span
          key={`${index}-${peak}`}
          style={{ transform: `scaleY(${Math.max(0.08, Math.min(1, peak))})` }}
        />
      ))}
    </div>
  );
}
