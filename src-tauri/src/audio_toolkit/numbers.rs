//! Rewrites spelled-out numbers as digits ("vingt-cinq" → "25",
//! "ten percent" → "10%") in French and English transcripts.
//!
//! This runs on the model's text, so it gives the same result whichever
//! transcription model produced it. It is rule based and deliberately
//! conservative: a lone article ("un", "une", "one") stays a word, and any
//! run it cannot read as one well-formed number is split or left as is.

use once_cell::sync::Lazy;
use regex::Regex;

use super::text::OutputLanguageEvidence;

/// Letters, including combining accents.
static WORD_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\p{L}\p{M}]+").unwrap());

/// Non-breaking space, so "300 000" and "10 %" never wrap apart.
const NBSP: char = '\u{a0}';

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    French,
    English,
}

impl Lang {
    fn from_evidence(language: &OutputLanguageEvidence) -> Option<Self> {
        let code = language.language()?.to_ascii_lowercase();
        let base = code.split(['-', '_']).next().unwrap_or("");
        match base {
            "fr" => Some(Self::French),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    fn decimal_word(self) -> &'static str {
        match self {
            Self::French => "virgule",
            Self::English => "point",
        }
    }

    fn decimal_separator(self) -> char {
        match self {
            Self::French => ',',
            Self::English => '.',
        }
    }

    fn connector(self) -> &'static str {
        match self {
            Self::French => "et",
            Self::English => "and",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Zero,
    /// 1–9.
    Unit,
    /// 10–19.
    Teen,
    /// 20, 30 … 90.
    Tens,
    Hundred,
    /// 1 000 and above.
    Scale,
}

fn lookup(lang: Lang, word: &str) -> Option<(u64, Kind)> {
    use Kind::*;
    let entry = match lang {
        Lang::French => match word {
            "zéro" | "zero" => (0, Zero),
            "un" | "une" => (1, Unit),
            "deux" => (2, Unit),
            "trois" => (3, Unit),
            "quatre" => (4, Unit),
            "cinq" => (5, Unit),
            "six" => (6, Unit),
            "sept" => (7, Unit),
            "huit" => (8, Unit),
            "neuf" => (9, Unit),
            "dix" => (10, Teen),
            "onze" => (11, Teen),
            "douze" => (12, Teen),
            "treize" => (13, Teen),
            "quatorze" => (14, Teen),
            "quinze" => (15, Teen),
            "seize" => (16, Teen),
            "vingt" | "vingts" => (20, Tens),
            "trente" => (30, Tens),
            "quarante" => (40, Tens),
            "cinquante" => (50, Tens),
            "soixante" => (60, Tens),
            "septante" => (70, Tens),
            "huitante" | "octante" => (80, Tens),
            "nonante" => (90, Tens),
            "cent" | "cents" => (100, Hundred),
            "mille" | "mil" => (1_000, Scale),
            "million" | "millions" => (1_000_000, Scale),
            "milliard" | "milliards" => (1_000_000_000, Scale),
            _ => return None,
        },
        Lang::English => match word {
            "zero" => (0, Zero),
            "one" => (1, Unit),
            "two" => (2, Unit),
            "three" => (3, Unit),
            "four" => (4, Unit),
            "five" => (5, Unit),
            "six" => (6, Unit),
            "seven" => (7, Unit),
            "eight" => (8, Unit),
            "nine" => (9, Unit),
            "ten" => (10, Teen),
            "eleven" => (11, Teen),
            "twelve" => (12, Teen),
            "thirteen" => (13, Teen),
            "fourteen" => (14, Teen),
            "fifteen" => (15, Teen),
            "sixteen" => (16, Teen),
            "seventeen" => (17, Teen),
            "eighteen" => (18, Teen),
            "nineteen" => (19, Teen),
            "twenty" => (20, Tens),
            "thirty" => (30, Tens),
            "forty" => (40, Tens),
            "fifty" => (50, Tens),
            "sixty" => (60, Tens),
            "seventy" => (70, Tens),
            "eighty" => (80, Tens),
            "ninety" => (90, Tens),
            "hundred" => (100, Hundred),
            "thousand" => (1_000, Scale),
            "million" => (1_000_000, Scale),
            "billion" => (1_000_000_000, Scale),
            _ => return None,
        },
    };
    Some(entry)
}

struct Token {
    start: usize,
    end: usize,
    lower: String,
}

/// One number read from consecutive tokens `first..=last`.
struct Parsed {
    first: usize,
    last: usize,
    value: u64,
}

/// Builds one number at a time, refusing any word that cannot continue it.
#[derive(Default)]
struct Builder {
    total: u64,
    current: u64,
    /// Smallest scale (thousand, million…) used so far; the next must be smaller.
    last_scale: Option<u64>,
    has_hundred: bool,
    words: usize,
    /// Value of the previous word, for "quatre-vingt".
    prev: Option<(u64, Kind)>,
    zero: bool,
}

impl Builder {
    fn is_empty(&self) -> bool {
        self.words == 0
    }

    /// Adds a word if it continues the number; returns false otherwise.
    fn push(&mut self, lang: Lang, value: u64, kind: Kind) -> bool {
        if self.zero {
            return false;
        }
        let tail = self.current % 100;
        let ok = match kind {
            Kind::Zero => self.is_empty(),
            // "dix-sept" is French; "ten seven" is two numbers.
            Kind::Unit => tail.is_multiple_of(10) && (tail != 10 || lang == Lang::French),
            Kind::Teen => tail == 0 || (lang == Lang::French && (tail == 60 || tail == 80)),
            Kind::Tens => {
                tail == 0
                    || (lang == Lang::French
                        && value == 20
                        && self.prev == Some((4, Kind::Unit))
                        && tail == 4)
            }
            Kind::Hundred => {
                !self.has_hundred
                    && match lang {
                        Lang::French => self.current < 10,
                        Lang::English => (1..100).contains(&self.current),
                    }
            }
            Kind::Scale => {
                // "mille" stands alone in French; the others need a count.
                let has_count = self.current > 0 || (lang == Lang::French && value == 1_000);
                has_count && self.last_scale.is_none_or(|s| value < s)
            }
        };
        if !ok {
            return false;
        }

        match kind {
            Kind::Zero => self.zero = true,
            Kind::Tens if tail == 4 => self.current += 76, // quatre-vingt: 4 → 80
            Kind::Unit | Kind::Teen | Kind::Tens => self.current += value,
            Kind::Hundred => {
                self.current = self.current.max(1) * 100;
                self.has_hundred = true;
            }
            Kind::Scale => {
                let Some(add) = self.current.max(1).checked_mul(value) else {
                    return false;
                };
                self.total += add;
                self.current = 0;
                self.has_hundred = false;
                self.last_scale = Some(value);
            }
        }
        self.words += 1;
        self.prev = Some((value, kind));
        true
    }

    fn value(&self) -> u64 {
        self.total + self.current
    }
}

/// Splits a run of number words into the numbers it holds.
fn parse_run(lang: Lang, tokens: &[Token], run: &[usize]) -> Vec<Parsed> {
    let mut out = Vec::new();
    let mut builder = Builder::default();
    let mut first = run[0];
    let mut last = run[0];

    for &index in run {
        let word = tokens[index].lower.as_str();
        if word == lang.connector() {
            continue;
        }
        let Some((value, kind)) = lookup(lang, word) else {
            continue;
        };
        if !builder.push(lang, value, kind) {
            if !builder.is_empty() {
                out.push(Parsed {
                    first,
                    last,
                    value: builder.value(),
                });
            }
            builder = Builder::default();
            first = index;
            if !builder.push(lang, value, kind) {
                // A word that cannot start a number ("million" alone).
                builder = Builder::default();
                continue;
            }
        } else if builder.words == 1 {
            first = index;
        }
        last = index;
    }
    if !builder.is_empty() {
        out.push(Parsed {
            first,
            last,
            value: builder.value(),
        });
    }
    out
}

fn separator_is_joiner(sep: &str) -> bool {
    !sep.is_empty() && sep.chars().all(|c| c == '-' || c.is_whitespace())
}

fn separator_is_space(sep: &str) -> bool {
    !sep.is_empty() && sep.chars().all(char::is_whitespace)
}

/// Writes `value` with a thousands separator from 10 000 up, so years and
/// four-digit amounts stay "2026".
fn format_number(lang: Lang, value: u64) -> String {
    let digits = value.to_string();
    if value < 10_000 {
        return digits;
    }
    let sep = match lang {
        Lang::French => NBSP,
        Lang::English => ',',
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// A number the text should keep in words.
fn keep_as_words(lang: Lang, tokens: &[Token], parsed: &Parsed) -> bool {
    if parsed.first != parsed.last {
        return false;
    }
    // Digits of a decimal ("trois virgule un") or of a list ("deux, un").
    if parsed.first > 0 {
        let prev = tokens[parsed.first - 1].lower.as_str();
        if prev == lang.decimal_word() || lookup(lang, prev).is_some() {
            return false;
        }
    }
    let word = tokens[parsed.first].lower.as_str();
    match lang {
        // Articles.
        Lang::French if word == "un" || word == "une" => true,
        Lang::English if word == "one" => true,
        // "quoi de neuf", "tout neuf", "remis à neuf".
        Lang::French if word == "neuf" => {
            parsed.first > 0
                && matches!(
                    tokens[parsed.first - 1].lower.as_str(),
                    "de" | "à" | "tout" | "toute" | "toutes" | "tous" | "flambant"
                )
        }
        _ => false,
    }
}

/// Rewrites spelled-out numbers as digits when the transcript is French or
/// English; other languages, and text of unknown language, are returned as is.
pub fn convert_number_words(text: &str, language: &OutputLanguageEvidence) -> String {
    let Some(lang) = Lang::from_evidence(language) else {
        return text.to_string();
    };

    let tokens: Vec<Token> = WORD_PATTERN
        .find_iter(text)
        .map(|m| Token {
            start: m.start(),
            end: m.end(),
            lower: m.as_str().to_lowercase(),
        })
        .collect();
    if tokens.is_empty() {
        return text.to_string();
    }
    let sep_before = |i: usize| -> &str {
        let from = if i == 0 { 0 } else { tokens[i - 1].end };
        &text[from..tokens[i].start]
    };
    let is_number = |i: usize| lookup(lang, &tokens[i].lower).is_some();

    // Find runs of number words joined by spaces or hyphens.
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if !is_number(i) {
            i += 1;
            continue;
        }
        let mut run = vec![i];
        let mut j = i;
        loop {
            let next = j + 1;
            if next >= tokens.len() || !separator_is_joiner(sep_before(next)) {
                break;
            }
            if is_number(next) {
                run.push(next);
                j = next;
                continue;
            }
            // "vingt et un", "one hundred and five".
            let is_connector = tokens[next].lower == lang.connector()
                && next + 1 < tokens.len()
                && separator_is_joiner(sep_before(next + 1))
                && is_number(next + 1);
            if is_connector {
                run.push(next);
                run.push(next + 1);
                j = next + 1;
                continue;
            }
            break;
        }
        runs.push(run);
        i = j + 1;
    }

    // Read the numbers in each run, skipping runs glued to other words
    // ("dix-huitième", "l'un") or to digits the model already wrote ("25 mille").
    let mut numbers: Vec<Parsed> = Vec::new();
    for run in &runs {
        let first = run[0];
        let last = *run.last().unwrap();
        let before = sep_before(first);
        let glued_before = first > 0
            && !before.is_empty()
            && before.chars().all(|c| matches!(c, '-' | '\'' | '’'));
        let glued_after = last + 1 < tokens.len() && sep_before(last + 1) == "-";
        let starts_with_multiplier = matches!(
            lookup(lang, &tokens[first].lower),
            Some((_, Kind::Hundred | Kind::Scale))
        );
        let after_digits = starts_with_multiplier
            && text[..tokens[first].start]
                .trim_end()
                .ends_with(|c: char| c.is_ascii_digit());
        if glued_before || glued_after || after_digits {
            continue;
        }
        for parsed in parse_run(lang, &tokens, run) {
            if !keep_as_words(lang, &tokens, &parsed) {
                numbers.push(parsed);
            }
        }
    }

    // Rebuild the text, joining decimals and percentages onto their number.
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut n = 0;
    while n < numbers.len() {
        let number = &numbers[n];
        let mut end_token = number.last;
        let mut written = format_number(lang, number.value);

        // "trois virgule cinq" → "3,5", "three point one four" → "3.14".
        let decimal_at = number.last + 1;
        if decimal_at + 1 < tokens.len()
            && tokens[decimal_at].lower == lang.decimal_word()
            && separator_is_space(sep_before(decimal_at))
            && separator_is_space(sep_before(decimal_at + 1))
        {
            let mut digits = String::new();
            let mut m = n + 1;
            let mut expected = decimal_at + 1;
            while m < numbers.len()
                && numbers[m].first == expected
                && (m == n + 1 || separator_is_space(sep_before(expected)))
            {
                digits.push_str(&numbers[m].value.to_string());
                expected = numbers[m].last + 1;
                m += 1;
            }
            if !digits.is_empty() {
                written.push(lang.decimal_separator());
                written.push_str(&digits);
                end_token = numbers[m - 1].last;
                n = m - 1;
            }
        }

        // "dix pour cent" → "10 %", "ten percent" → "10%".
        let after = end_token + 1;
        let percent_end = match lang {
            Lang::French => {
                if after < tokens.len() && tokens[after].lower == "pourcent" {
                    Some(after)
                } else if after + 1 < tokens.len()
                    && tokens[after].lower == "pour"
                    && tokens[after + 1].lower == "cent"
                    && separator_is_space(sep_before(after + 1))
                {
                    Some(after + 1)
                } else {
                    None
                }
            }
            Lang::English => {
                if after < tokens.len() && tokens[after].lower == "percent" {
                    Some(after)
                } else if after + 1 < tokens.len()
                    && tokens[after].lower == "per"
                    && tokens[after + 1].lower == "cent"
                    && separator_is_space(sep_before(after + 1))
                {
                    Some(after + 1)
                } else {
                    None
                }
            }
        };
        if let Some(percent_end) = percent_end {
            if separator_is_space(sep_before(after)) {
                if lang == Lang::French {
                    written.push(NBSP);
                }
                written.push('%');
                end_token = percent_end;
                // "cent pour cent": the second "cent" is not a number.
                if n + 1 < numbers.len() && numbers[n + 1].first <= percent_end {
                    n += 1;
                }
            }
        }

        out.push_str(&text[cursor..tokens[number.first].start]);
        out.push_str(&written);
        cursor = tokens[end_token].end;
        n += 1;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(text: &str) -> String {
        convert_number_words(text, &OutputLanguageEvidence::UserSelected("fr".into()))
            .replace(NBSP, " ")
    }

    fn en(text: &str) -> String {
        convert_number_words(text, &OutputLanguageEvidence::UserSelected("en".into()))
    }

    #[test]
    fn french_basic_numbers() {
        assert_eq!(fr("J'ai vingt-cinq ans."), "J'ai 25 ans.");
        assert_eq!(fr("trois cent mille euros"), "300 000 euros");
        assert_eq!(fr("deux mille vingt-six"), "2026");
        assert_eq!(fr("quatre-vingt-dix-neuf"), "99");
        assert_eq!(fr("quatre-vingts personnes"), "80 personnes");
        assert_eq!(fr("soixante et onze"), "71");
        assert_eq!(fr("vingt et un"), "21");
        assert_eq!(fr("soixante-dix-sept"), "77");
        assert_eq!(fr("cent un"), "101");
        assert_eq!(fr("deux cents"), "200");
        assert_eq!(fr("mille deux cent cinquante"), "1250");
        assert_eq!(fr("un million deux cent mille"), "1 200 000");
        assert_eq!(fr("Dix heures trente"), "10 heures 30");
        assert_eq!(fr("zéro"), "0");
        assert_eq!(fr("nonante-cinq"), "95");
        assert_eq!(fr("dix-sept"), "17");
        assert_eq!(fr("quatre-vingt-onze"), "91");
        assert_eq!(fr("quatre-vingt-un"), "81");
        assert_eq!(fr("vingt et une pages"), "21 pages");
        assert_eq!(fr("En 2026 trois personnes"), "En 2026 3 personnes");
    }

    #[test]
    fn french_keeps_articles_and_idioms() {
        assert_eq!(fr("un dossier"), "un dossier");
        assert_eq!(fr("une réunion"), "une réunion");
        assert_eq!(fr("quoi de neuf ?"), "quoi de neuf ?");
        assert_eq!(fr("tout neuf"), "tout neuf");
        assert_eq!(fr("l'un des deux"), "l'un des 2");
        assert_eq!(fr("d'un million d'euros"), "d'un million d'euros");
        assert_eq!(fr("des millions"), "des millions");
        assert_eq!(fr("le dix-huitième siècle"), "le dix-huitième siècle");
        assert_eq!(fr("neuf personnes"), "9 personnes");
    }

    #[test]
    fn french_percent_and_decimals() {
        assert_eq!(fr("dix pour cent"), "10 %");
        assert_eq!(fr("cent pour cent"), "100 %");
        assert_eq!(fr("trois virgule cinq"), "3,5");
        assert_eq!(fr("deux virgule zéro cinq"), "2,05");
        assert_eq!(fr("trois virgule quatorze pour cent"), "3,14 %");
    }

    #[test]
    fn french_separate_numbers_stay_separate() {
        assert_eq!(fr("deux trois fois"), "2 3 fois");
        assert_eq!(fr("onze douze"), "11 12");
        assert_eq!(fr("25 mille"), "25 mille");
    }

    #[test]
    fn english_numbers() {
        assert_eq!(en("twenty-five people"), "25 people");
        assert_eq!(en("three hundred thousand"), "300,000");
        assert_eq!(en("one hundred and five"), "105");
        assert_eq!(en("two thousand twenty-six"), "2026");
        assert_eq!(en("nineteen hundred"), "1900");
        assert_eq!(en("ten percent"), "10%");
        assert_eq!(en("three point one four"), "3.14");
        assert_eq!(en("three point four one"), "3.41");
        assert_eq!(en("one of them"), "one of them");
        assert_eq!(en("one million dollars"), "1,000,000 dollars");
        assert_eq!(en("millions of people"), "millions of people");
    }

    #[test]
    fn other_languages_untouched() {
        let de = OutputLanguageEvidence::UserSelected("de".into());
        assert_eq!(convert_number_words("zwanzig", &de), "zwanzig");
        assert_eq!(
            convert_number_words("vingt", &OutputLanguageEvidence::Unknown),
            "vingt"
        );
    }
}
