import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Play, Pause } from "lucide-react";
import { useTranslation } from "react-i18next";

const revokeBlobSource = (src: string | null) => {
  if (src?.startsWith("blob:")) URL.revokeObjectURL(src);
};

/** Number of bars drawn in a waveform. */
const WAVEFORM_BARS = 96;
/** Shortest bar, as a share of the waveform's height, so silence stays visible. */
const WAVEFORM_MIN_BAR = 0.08;
/** Decoded waveforms kept in memory, oldest dropped first. */
const WAVEFORM_CACHE_SIZE = 200;

const waveformCache = new Map<string, Promise<number[]>>();
let waveformDecoder: OfflineAudioContext | null = null;

/**
 * Reads an audio file and returns its loudness as `bars` values between 0 and 1
 * (peak of each slice, scaled so the loudest slice is 1).
 */
const loadWaveform = (src: string, bars: number): Promise<number[]> => {
  const cached = waveformCache.get(src);
  if (cached) return cached;

  const promise = (async () => {
    const response = await fetch(src);
    const data = await response.arrayBuffer();
    // Recordings are 16 kHz mono; decoding at that rate keeps it cheap.
    waveformDecoder ??= new OfflineAudioContext(1, 1, 16000);
    const buffer = await waveformDecoder.decodeAudioData(data);
    const samples = buffer.getChannelData(0);
    const sliceSize = Math.max(1, Math.floor(samples.length / bars));

    const peaks: number[] = [];
    let loudest = 0;
    for (let bar = 0; bar < bars; bar++) {
      const start = bar * sliceSize;
      const end = Math.min(start + sliceSize, samples.length);
      let peak = 0;
      for (let i = start; i < end; i++) {
        const value = Math.abs(samples[i]);
        if (value > peak) peak = value;
      }
      peaks.push(peak);
      if (peak > loudest) loudest = peak;
    }
    return peaks.map((peak) => (loudest > 0 ? peak / loudest : 0));
  })();

  waveformCache.set(src, promise);
  promise.catch(() => waveformCache.delete(src));
  if (waveformCache.size > WAVEFORM_CACHE_SIZE) {
    const oldest = waveformCache.keys().next().value;
    if (oldest !== undefined) waveformCache.delete(oldest);
  }
  return promise;
};

interface AudioPlayerProps {
  /** Audio source URL. If not provided, onLoadRequest must be provided. */
  src?: string;
  /** Called when play is clicked and no src is loaded yet. Should return the audio URL. */
  onLoadRequest?: () => Promise<string | null>;
  className?: string;
  autoPlay?: boolean;
  /**
   * Draw the recording's waveform in place of the progress bar. The audio is
   * loaded as soon as the player scrolls into view, not on first play.
   */
  showWaveform?: boolean;
}

interface AudioPlayerGroupContextValue {
  requestPlayback: (audio: HTMLAudioElement) => void;
  releasePlayback: (audio: HTMLAudioElement) => void;
}

const AudioPlayerGroupContext =
  createContext<AudioPlayerGroupContextValue | null>(null);

export const AudioPlayerGroup: React.FC<React.PropsWithChildren> = ({
  children,
}) => {
  const activeAudioRef = useRef<HTMLAudioElement | null>(null);
  const value = useMemo<AudioPlayerGroupContextValue>(
    () => ({
      requestPlayback: (audio) => {
        if (activeAudioRef.current !== audio) activeAudioRef.current?.pause();
        activeAudioRef.current = audio;
      },
      releasePlayback: (audio) => {
        if (activeAudioRef.current === audio) activeAudioRef.current = null;
      },
    }),
    [],
  );

  return (
    <AudioPlayerGroupContext.Provider value={value}>
      {children}
    </AudioPlayerGroupContext.Provider>
  );
};

export const AudioPlayer: React.FC<AudioPlayerProps> = ({
  src: initialSrc,
  onLoadRequest,
  className = "",
  autoPlay = false,
  showWaveform = false,
}) => {
  const { t } = useTranslation();
  const group = useContext(AudioPlayerGroupContext);
  const [isPlaying, setIsPlaying] = useState(false);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const [loadedSrc, setLoadedSrc] = useState<string | null>(initialSrc ?? null);
  const [isLoading, setIsLoading] = useState(false);
  const [peaks, setPeaks] = useState<number[] | null>(null);

  const audioRef = useRef<HTMLAudioElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const loadedSrcRef = useRef<string | null>(initialSrc ?? null);
  const loadPromiseRef = useRef<Promise<string | null> | null>(null);
  // Set when a click asked for playback, so a background load doesn't start it.
  const playOnLoadRef = useRef(false);
  const mountedRef = useRef(true);
  // Own a resolved source immediately, before its state/effect can commit.
  const pendingSrcRef = useRef<string | null>(null);
  const src = loadedSrc;
  const animationRef = useRef<number>();
  const dragTimeRef = useRef<number>(0);

  // Use refs to avoid stale closures in animation loop
  const isPlayingRef = useRef(false);
  const isDraggingRef = useRef(false);

  // Keep refs in sync with state
  useEffect(() => {
    isPlayingRef.current = isPlaying;
  }, [isPlaying]);

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  // Stable animation loop with no dependencies
  const tick = useCallback(() => {
    if (audioRef.current && !isDraggingRef.current) {
      const time = audioRef.current.currentTime;
      setCurrentTime(time);
    }

    if (isPlayingRef.current) {
      animationRef.current = requestAnimationFrame(tick);
    }
  }, []); // Empty dependency array is key!

  // Manage animation loop lifecycle
  useEffect(() => {
    if (isPlaying && !isDragging) {
      // Only start if not already running
      if (!animationRef.current) {
        animationRef.current = requestAnimationFrame(tick);
      }
    } else {
      // Stop animation loop
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    }

    return () => {
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    };
  }, [isPlaying, isDragging, tick]);

  // Audio event handlers
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const handleLoadedMetadata = () => {
      setDuration(audio.duration || 0);
      setCurrentTime(0);
    };

    const handleEnded = () => {
      group?.releasePlayback(audio);
      setIsPlaying(false);
      setCurrentTime(audio.duration || 0);
    };

    const handlePlay = () => {
      group?.requestPlayback(audio);
      setIsPlaying(true);
    };
    const handlePause = () => {
      group?.releasePlayback(audio);
      setIsPlaying(false);
    };

    audio.addEventListener("loadedmetadata", handleLoadedMetadata);
    audio.addEventListener("ended", handleEnded);
    audio.addEventListener("play", handlePlay);
    audio.addEventListener("pause", handlePause);

    return () => {
      group?.releasePlayback(audio);
      audio.removeEventListener("loadedmetadata", handleLoadedMetadata);
      audio.removeEventListener("ended", handleEnded);
      audio.removeEventListener("play", handlePlay);
      audio.removeEventListener("pause", handlePause);
    };
  }, [group]);

  // Auto-play when src becomes available (via onLoadRequest or autoPlay prop)
  const prevLoadedSrc = useRef<string | null>(null);
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    // Play when loadedSrc changes from null to a value after a click (lazy
    // load case). A load started only to draw the waveform doesn't play.
    if (loadedSrc && !prevLoadedSrc.current && playOnLoadRef.current) {
      playOnLoadRef.current = false;
      audio.play().catch((error) => {
        console.error("Auto-play failed:", error);
      });
    }
    // Or when autoPlay is set with initial src
    else if (autoPlay && initialSrc && !prevLoadedSrc.current) {
      audio.play().catch((error) => {
        console.error("Auto-play failed:", error);
      });
    }

    prevLoadedSrc.current = loadedSrc;
  }, [loadedSrc, autoPlay, initialSrc]);

  // Global drag handlers
  const handleMouseUp = useCallback(() => {
    if (isDragging) {
      setIsDragging(false);
      if (audioRef.current) {
        audioRef.current.currentTime = dragTimeRef.current;
        setCurrentTime(dragTimeRef.current);
      }
    }
  }, [isDragging]);

  useEffect(() => {
    if (isDragging) {
      document.addEventListener("mouseup", handleMouseUp);
      document.addEventListener("touchend", handleMouseUp);

      return () => {
        document.removeEventListener("mouseup", handleMouseUp);
        document.removeEventListener("touchend", handleMouseUp);
      };
    }
  }, [isDragging, handleMouseUp]);

  // Cleanup blob URLs on unmount
  useEffect(() => {
    if (pendingSrcRef.current === loadedSrc) pendingSrcRef.current = null;
    return () => {
      revokeBlobSource(loadedSrc);
    };
  }, [loadedSrc]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      revokeBlobSource(pendingSrcRef.current);
      pendingSrcRef.current = null;
    };
  }, []);

  /**
   * Fetches the audio source once, whether a click or the waveform asked for
   * it first; a second caller waits on the same request.
   */
  const ensureLoaded = useCallback((): Promise<string | null> => {
    if (loadedSrcRef.current) return Promise.resolve(loadedSrcRef.current);
    if (!onLoadRequest) return Promise.resolve(null);
    if (!loadPromiseRef.current) {
      loadPromiseRef.current = onLoadRequest()
        .then((newSrc) => {
          if (!mountedRef.current) {
            revokeBlobSource(newSrc);
            return null;
          }
          if (newSrc) {
            pendingSrcRef.current = newSrc;
            loadedSrcRef.current = newSrc;
            setLoadedSrc(newSrc);
          }
          return newSrc;
        })
        .finally(() => {
          loadPromiseRef.current = null;
        });
    }
    return loadPromiseRef.current;
  }, [onLoadRequest]);

  // Waveform: load the audio once the player is about to scroll into view,
  // so a long history doesn't read every recording at once.
  useEffect(() => {
    if (!showWaveform) return;
    const element = containerRef.current;
    if (!element) return;

    const load = () => {
      ensureLoaded().catch((error) => {
        console.error("Failed to load audio for waveform:", error);
      });
    };

    if (typeof IntersectionObserver === "undefined") {
      load();
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer.disconnect();
          load();
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [showWaveform, ensureLoaded]);

  useEffect(() => {
    if (!showWaveform || !loadedSrc) return;
    let cancelled = false;
    loadWaveform(loadedSrc, WAVEFORM_BARS)
      .then((values) => {
        if (!cancelled) setPeaks(values);
      })
      .catch((error) => {
        console.error("Failed to draw waveform:", error);
      });
    return () => {
      cancelled = true;
    };
  }, [showWaveform, loadedSrc]);

  const togglePlay = async () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (isLoading) return;

    try {
      if (isPlaying) {
        audio.pause();
      } else {
        // If no src loaded yet, request it
        if (!src && onLoadRequest) {
          setIsLoading(true);
          playOnLoadRef.current = true;
          try {
            // Playback will be triggered by the useEffect watching loadedSrc
            await ensureLoaded();
          } finally {
            if (!loadedSrcRef.current) playOnLoadRef.current = false;
            if (mountedRef.current) setIsLoading(false);
          }
        } else if (src) {
          await audio.play();
        }
      }
    } catch (error) {
      console.error("Playback failed:", error);
    }
  };

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newTime = parseFloat(e.target.value);
    dragTimeRef.current = newTime;
    setCurrentTime(newTime);

    if (!isDragging && audioRef.current) {
      audioRef.current.currentTime = newTime;
    }
  };

  const handleSliderMouseDown = () => {
    dragTimeRef.current = currentTime;
    setIsDragging(true);
  };

  const handleSliderTouchStart = () => {
    dragTimeRef.current = currentTime;
    setIsDragging(true);
  };

  const formatTime = (time: number): string => {
    if (!isFinite(time)) return "0:00";

    const minutes = Math.floor(time / 60);
    const seconds = Math.floor(time % 60);
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  };

  // Fix playhead positioning with better edge case handling
  const getProgressPercent = (): number => {
    if (duration <= 0) return 0;

    // Handle the end case - if we're within 0.1 seconds of the end, show 100%
    if (duration - currentTime < 0.1) return 100;

    const percent = (currentTime / duration) * 100;
    return Math.min(100, Math.max(0, percent));
  };

  const progressPercent = getProgressPercent();

  const seekInputProps = {
    type: "range",
    "aria-label": t("common.playbackPosition"),
    min: "0",
    max: duration || 0,
    step: "0.01",
    value: currentTime,
    onChange: handleSeek,
    onMouseDown: handleSliderMouseDown,
    onTouchStart: handleSliderTouchStart,
  } as const;

  const seekControl =
    showWaveform && peaks ? (
      // The waveform is drawn under a transparent range input, which keeps
      // clicking, dragging and the keyboard working as with the plain bar.
      <div className="relative flex-1 h-7 rounded-sm has-[input:focus-visible]:ring-1 has-[input:focus-visible]:ring-logo-primary">
        <svg
          className="absolute inset-0 w-full h-full pointer-events-none"
          viewBox={`0 0 ${peaks.length} 100`}
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          {peaks.map((peak, index) => {
            const height = Math.max(WAVEFORM_MIN_BAR, peak) * 100;
            const played =
              ((index + 0.5) / peaks.length) * 100 <= progressPercent;
            return (
              <rect
                key={index}
                x={index + 0.2}
                y={(100 - height) / 2}
                width={0.6}
                height={height}
                style={{
                  fill: played
                    ? "var(--color-logo-primary)"
                    : "var(--color-line-strong)",
                }}
              />
            );
          })}
        </svg>
        <input
          {...seekInputProps}
          className="absolute inset-0 w-full h-full m-0 opacity-0 cursor-pointer"
        />
      </div>
    ) : (
      <input
        {...seekInputProps}
        className={`flex-1 h-1 rounded-lg appearance-none cursor-pointer focus:outline-none focus:ring-1 focus:ring-logo-primary ${progressPercent >= 99.5 ? "[&::-webkit-slider-thumb]:translate-x-0.5 [&::-moz-range-thumb]:translate-x-0.5" : ""}`}
        style={{
          background: `linear-gradient(to right, var(--color-logo-primary) 0%, var(--color-logo-primary) ${progressPercent}%, var(--color-line) ${progressPercent}%, rgba(128, 128, 128, 0.2) 100%)`,
        }}
      />
    );

  return (
    <div ref={containerRef} className={`flex items-center gap-3 ${className}`}>
      <audio ref={audioRef} src={src ?? undefined} preload="metadata" />

      <button
        onClick={togglePlay}
        disabled={isLoading}
        className="transition-colors cursor-pointer text-text hover:text-logo-primary disabled:opacity-50"
        aria-label={isPlaying ? "Pause" : "Play"}
      >
        {isPlaying ? (
          <Pause width={20} height={20} fill="currentColor" />
        ) : (
          <Play width={20} height={20} fill="currentColor" />
        )}
      </button>

      <div className="flex-1 flex items-center gap-2">
        <span className="text-xs text-ink-2 min-w-[30px] tabular-nums">
          {formatTime(currentTime)}
        </span>

        {seekControl}

        <span className="text-xs text-ink-2 min-w-[30px] tabular-nums">
          {formatTime(duration)}
        </span>
      </div>
    </div>
  );
};
