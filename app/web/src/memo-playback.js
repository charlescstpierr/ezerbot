// Playback of a voice memo: a speed within the range of a bot's voice, and a jump
// to the start of a word. The audio element is injected so the rules are testable.
export const MEMO_SPEED_MIN = 0.5;
export const MEMO_SPEED_MAX = 2;
export const UNTIMED_WORD_NOTICE = "Ce mot n’a pas d’horodatage : la lecture continue.";

const inRange = (speed) => Number.isFinite(speed) && speed >= MEMO_SPEED_MIN && speed <= MEMO_SPEED_MAX;

export function formatMemoSpeed(speed) {
  return String(Math.round(speed * 100) / 100).replace(".", ",");
}

// « 1,5 » and « 1.5 » are both a speed; anything else, or a speed out of range, is not.
export function parseMemoSpeed(text) {
  const value = String(text ?? "").trim().replace(",", ".");
  if (!/^\d+(\.\d+)?$/.test(value)) return null;
  const speed = Number(value);
  return inRange(speed) ? speed : null;
}

// The speed of the bot at first; 1 when it is unknown or outside the range.
export function startingMemoSpeed(botSpeed) {
  return inRange(botSpeed) ? botSpeed : 1;
}

export function createMemoPlayer(audio, startingSpeed = 1) {
  let speed = startingMemoSpeed(startingSpeed);
  const apply = () => { audio.playbackRate = speed; };
  return {
    get speed() { return speed; },
    // Re-applies the speed: a media element may reset its rate when it loads.
    apply,
    setSpeed(text) {
      const next = parseMemoSpeed(text);
      if (next === null) {
        return { ok: false, speed, message: `Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse reste ${formatMemoSpeed(speed)}.` };
      }
      speed = next;
      apply();
      return { ok: true, speed, message: "" };
    },
    // A word is a place in the audio only when it has a start inside the recording.
    jump(word) {
      const start = word?.start_ms;
      if (!Number.isFinite(start) || start < 0) return { ok: false, message: UNTIMED_WORD_NOTICE };
      audio.currentTime = start / 1000;
      apply();
      const playing = audio.play?.();
      playing?.catch?.(() => {});
      return { ok: true, message: "" };
    },
  };
}
