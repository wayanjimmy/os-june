import { motion, useMotionValue, useSpring, useTransform } from "framer-motion";
import { useEffect } from "react";
import type { AudioLevelDto } from "../../lib/tauri";

type WaveformProps = {
  level: AudioLevelDto;
};

const BAR_COUNT = 32;
// Spring tuned for a soft VU-meter feel — lower stiffness + higher damping
// reads as smoother / less twitchy than a snappy meter.
const BAR_SPRING = { stiffness: 180, damping: 30, mass: 0.7 };

export function Waveform({ level }: WaveformProps) {
  const targets = computeTargetPeaks(level);

  return (
    <div className="waveform" aria-label="Microphone activity">
      {targets.map((peak, index) => (
        <WaveformBar key={index} target={peak} />
      ))}
    </div>
  );
}

function WaveformBar({ target }: { target: number }) {
  const raw = useMotionValue(target);
  const eased = useSpring(raw, BAR_SPRING);
  const scaleY = useTransform(eased, (v) => visualPeakScale(v));

  useEffect(() => {
    raw.set(target);
  }, [raw, target]);

  return <motion.span style={{ scaleY }} />;
}

function computeTargetPeaks(level: AudioLevelDto) {
  const source =
    level.recentPeaks.length > 0
      ? level.recentPeaks.slice(-BAR_COUNT)
      : [level.rms, level.peak, level.rms];
  return Array.from({ length: BAR_COUNT }, (_, index) => {
    const sourceIndex = Math.floor((index / BAR_COUNT) * source.length);
    const peak = source[sourceIndex] ?? level.rms;
    const neighbor = source[sourceIndex - 1] ?? peak;
    const next = source[sourceIndex + 1] ?? peak;
    const rolloff = 0.78 + Math.sin(index * 0.85) * 0.12;
    return Math.max(0, (neighbor * 0.22 + peak * 0.56 + next * 0.22) * rolloff);
  });
}

// Floor is intentionally low (idle bars are a thin seismograph line) and the
// ceiling is the full bar height, so loud peaks travel a long visible
// distance. Tune FLOOR/GAIN to taste.
const FLOOR = 0.06;
const GAIN = 11;

export function visualPeakScale(peak: number) {
  const normalized = Math.max(0, Math.min(1, peak));
  if (normalized <= 0.002) {
    return FLOOR;
  }
  return Math.min(1, Math.max(FLOOR, Math.sqrt(normalized * GAIN)));
}
