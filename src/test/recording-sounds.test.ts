import { afterEach, describe, expect, it, vi } from "vitest";
import {
  playRecordingSound,
  preloadRecordingSounds,
} from "../lib/recording-sounds";

describe("playRecordingSound", () => {
  const originalAudio = globalThis.Audio;

  afterEach(() => {
    globalThis.Audio = originalAudio;
    vi.restoreAllMocks();
  });

  it("plays the bundled recording sounds", () => {
    const play = vi.fn().mockResolvedValue(undefined);
    const pause = vi.fn();
    const load = vi.fn();
    const audio = vi.fn().mockImplementation(() => ({
      load,
      pause,
      play,
      preload: "",
      volume: 1,
      currentTime: 1,
    }));

    globalThis.Audio = audio as unknown as typeof Audio;

    playRecordingSound("start");
    playRecordingSound("pause");
    playRecordingSound("stop");

    expect(audio).toHaveBeenNthCalledWith(1, "/sounds/record-start.mp3");
    expect(audio).toHaveBeenNthCalledWith(2, "/sounds/record-pause.mp3");
    expect(audio).toHaveBeenNthCalledWith(3, "/sounds/record-end.mp3");
    expect(play).toHaveBeenCalledTimes(3);
  });

  it("preloads and reuses bundled recording sounds", () => {
    const play = vi.fn().mockResolvedValue(undefined);
    const pause = vi.fn();
    const load = vi.fn();
    const audio = vi.fn().mockImplementation(() => ({
      load,
      pause,
      play,
      preload: "",
      volume: 1,
      currentTime: 1,
    }));

    globalThis.Audio = audio as unknown as typeof Audio;

    preloadRecordingSounds();
    playRecordingSound("start");

    expect(audio).toHaveBeenCalledTimes(3);
    expect(load).toHaveBeenCalledTimes(3);
    expect(play).toHaveBeenCalledTimes(1);
  });

  it("ignores playback failures", () => {
    const play = vi.fn().mockRejectedValue(new Error("blocked"));

    globalThis.Audio = vi.fn().mockImplementation(() => ({
      load: vi.fn(),
      pause: vi.fn(),
      play,
      preload: "",
      volume: 1,
      currentTime: 1,
    })) as unknown as typeof Audio;

    expect(() => playRecordingSound("start")).not.toThrow();
  });
});
