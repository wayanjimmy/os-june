const RECORDING_SOUND_PATHS = {
  start: "/sounds/record-start.mp3",
  pause: "/sounds/record-pause.mp3",
  stop: "/sounds/record-end.mp3",
} as const;

export type RecordingSound = keyof typeof RECORDING_SOUND_PATHS;

let audioConstructor: typeof Audio | undefined;
const audioElements = new Map<RecordingSound, HTMLAudioElement>();

function getRecordingAudio(sound: RecordingSound) {
  if (typeof Audio === "undefined") return;

  if (audioConstructor !== Audio) {
    audioElements.clear();
    audioConstructor = Audio;
  }

  const cachedAudio = audioElements.get(sound);
  if (cachedAudio) return cachedAudio;

  const audio = new Audio(RECORDING_SOUND_PATHS[sound]);
  audio.preload = "auto";
  audio.volume = 0.7;
  audio.load();
  audioElements.set(sound, audio);
  return audio;
}

export function preloadRecordingSounds() {
  (Object.keys(RECORDING_SOUND_PATHS) as RecordingSound[]).forEach((sound) => {
    getRecordingAudio(sound);
  });
}

export function playRecordingSound(sound: RecordingSound) {
  const audio = getRecordingAudio(sound);
  if (!audio) return;

  audio.pause();
  audio.currentTime = 0;
  audio.volume = 0.7;

  void audio.play().catch(() => {
    // Browsers and webviews may reject autoplay. Recording should continue.
  });
}
