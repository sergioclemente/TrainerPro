import { normalizeVoiceTranscript } from "./voiceSurface";

const BELOW_TWENTY = [
  "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
  "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
  "seventeen", "eighteen", "nineteen",
] as const;

const TENS = [
  "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
] as const;

const MAX_SPOKEN_INTEGER = 999;

export interface IntegerSlotDomain {
  min: number;
  max: number;
  step: number;
}

export interface IntegerSlotParser {
  parse: (transcript: string) => number | null;
}

function belowOneHundred(value: number): string {
  if (value < BELOW_TWENTY.length) return BELOW_TWENTY[value];
  const tens = Math.floor(value / 10);
  const remainder = value % 10;
  return remainder === 0 ? TENS[tens] : `${TENS[tens]} ${BELOW_TWENTY[remainder]}`;
}

function spokenInteger(value: number): string {
  if (value < 100) return belowOneHundred(value);
  const hundreds = Math.floor(value / 100);
  const remainder = value % 100;
  const prefix = `${BELOW_TWENTY[hundreds]} hundred`;
  return remainder === 0 ? prefix : `${prefix} ${belowOneHundred(remainder)}`;
}

function aliasesFor(value: number): readonly string[] {
  const aliases = [spokenInteger(value)];
  if (value === 100) aliases.push("a hundred");

  const hundreds = Math.floor(value / 100);
  const remainder = value % 100;
  if (hundreds > 0 && remainder >= 10) {
    aliases.push(`${BELOW_TWENTY[hundreds]} ${belowOneHundred(remainder)}`);
  } else if (hundreds > 0 && remainder > 0) {
    aliases.push(`${BELOW_TWENTY[hundreds]} oh ${BELOW_TWENTY[remainder]}`);
  }
  return aliases;
}

function buildSpokenIntegerVocabulary(): {
  values: ReadonlyMap<string, number>;
  longestAlias: number;
} {
  const values = new Map<string, number>();
  let longestAlias = 1;
  for (let value = 0; value <= MAX_SPOKEN_INTEGER; value += 1) {
    for (const alias of aliasesFor(value)) {
      values.set(alias, value);
      longestAlias = Math.max(longestAlias, alias.split(" ").length);
    }
  }
  return { values, longestAlias };
}

const SPOKEN_INTEGER_VOCABULARY = buildSpokenIntegerVocabulary();

function validateDomain(domain: IntegerSlotDomain): void {
  const values = [domain.min, domain.max, domain.step];
  if (!values.every(Number.isInteger) || domain.min < 0 || domain.max > MAX_SPOKEN_INTEGER) {
    throw new Error(`Integer voice slots must stay between 0 and ${MAX_SPOKEN_INTEGER}`);
  }
  if (domain.min > domain.max || domain.step <= 0) {
    throw new Error("Integer voice slot requires an ordered range and positive step");
  }
}

function accepts(domain: IntegerSlotDomain, value: number): boolean {
  return Number.isInteger(value) &&
    value >= domain.min &&
    value <= domain.max &&
    (value - domain.min) % domain.step === 0;
}

/**
 * Creates a bounded numeric slot for one command argument. The complete spoken
 * integer is recognized before its value is checked against the declared range.
 */
export function createIntegerSlotParser(domain: IntegerSlotDomain): IntegerSlotParser {
  validateDomain(domain);

  return {
    parse: (transcript) => {
      const normalized = normalizeVoiceTranscript(transcript);
      for (const match of normalized.matchAll(/\b\d+(?:\.\d+)?\b/g)) {
        const value = Number(match[0]);
        if (accepts(domain, value)) return value;
        return null;
      }

      const tokens = normalized
        .replace(/\band\b/g, " ")
        .replace(/\s+/g, " ")
        .trim()
        .split(" ");
      for (let start = 0; start < tokens.length; start += 1) {
        const available = Math.min(
          SPOKEN_INTEGER_VOCABULARY.longestAlias,
          tokens.length - start,
        );
        for (let length = available; length > 0; length -= 1) {
          const value = SPOKEN_INTEGER_VOCABULARY.values.get(
            tokens.slice(start, start + length).join(" "),
          );
          if (value !== undefined) return accepts(domain, value) ? value : null;
        }
      }
      return null;
    },
  };
}
