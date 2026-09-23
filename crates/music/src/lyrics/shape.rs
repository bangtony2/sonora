use std::time::Duration;

use crate::{Lyrics, LyricsLine, LyricsWord, Voice};

const LEAST: usize = 6;
const MATCHED: f64 = 0.7;
const KEPT: f64 = 0.5;
const SEATED: f64 = 0.9;
const LIMIT: usize = 3000;
const CELLS: usize = 2_000_000;
const TICKS: &[char] = &['\'', '\u{2019}', '\u{ff07}', '\u{2018}', '\u{00b4}', '`'];

struct Sung {
    start: Duration,
    end: Duration,
    key: String,
}

struct Slot {
    at: usize,
    key: String,
}

pub(crate) fn conform(worded: &Lyrics, guide: &Lyrics) -> Option<Lyrics> {
    let Lyrics::Synced { lines: guide } = guide else {
        return None;
    };
    let Lyrics::Synced { lines: worded } = worded else {
        return None;
    };
    if guide.is_empty() || worded.is_empty() {
        return None;
    }

    let sung = sung(worded);
    let slotted: Vec<Vec<Slot>> = guide.iter().map(|line| tokens(&line.text)).collect();
    let places: Vec<(usize, usize)> = slotted
        .iter()
        .enumerate()
        .flat_map(|(line, slots)| (0..slots.len()).map(move |slot| (line, slot)))
        .collect();
    if sung.len() < LEAST || places.len() < LEAST || sung.len() > LIMIT || places.len() > LIMIT {
        return None;
    }
    if sung.len() * places.len() > CELLS {
        return None;
    }

    let left: Vec<&str> = sung.iter().map(|word| word.key.as_str()).collect();
    let right: Vec<&str> = places
        .iter()
        .map(|(line, slot)| slotted[*line][*slot].key.as_str())
        .collect();
    let pairs = paired(&left, &right);
    if pairs.len() < LEAST
        || (pairs.len() as f64) < MATCHED * places.len() as f64
        || (pairs.len() as f64) < KEPT * sung.len() as f64
        || !seated(&pairs, &places, &slotted)
    {
        return None;
    }

    let mut spans: Vec<Option<(Duration, Duration)>> = vec![None; places.len()];
    for (word, place) in &pairs {
        spans[*place] = Some((sung[*word].start, sung[*word].end));
    }
    let spans = filled(spans, &hints(guide, &slotted));

    let mut lines: Vec<LyricsLine> = Vec::with_capacity(guide.len());
    let mut cursor = 0;
    for (index, line) in guide.iter().enumerate() {
        let slots = &slotted[index];
        if slots.is_empty() {
            continue;
        }
        let mine = &spans[cursor..cursor + slots.len()];
        cursor += slots.len();
        let words = worded_from(&line.text, slots, mine);
        let start = words.first().map(|word| word.start)?;
        let end = words.iter().map(|word| word.end).max()?;
        lines.push(LyricsLine {
            start,
            end: Some(end.max(start)),
            text: line.text.clone(),
            romanized: None,
            words: Some(words),
            secondary: Vec::new(),
            voice: Voice::Lead,
        });
    }
    super::lrc::normalize(&mut lines);
    if lines.len() < 2 {
        return None;
    }

    Some(Lyrics::Synced {
        lines: lines.into(),
    })
}

fn seated(pairs: &[(usize, usize)], places: &[(usize, usize)], slotted: &[Vec<Slot>]) -> bool {
    let mut held = vec![false; slotted.len()];
    for (_, place) in pairs {
        held[places[*place].0] = true;
    }
    let counted = slotted.iter().filter(|slots| !slots.is_empty()).count();
    let anchored = held
        .iter()
        .zip(slotted)
        .filter(|(held, slots)| **held && !slots.is_empty())
        .count();
    (anchored as f64) >= SEATED * counted as f64
}

fn sung(lines: &[LyricsLine]) -> Vec<Sung> {
    let mut sung = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match line.words.as_deref().filter(|words| !words.is_empty()) {
            Some(words) => sung.extend(timed(words)),
            None => {
                let until = line
                    .end
                    .or_else(|| lines.get(index + 1).map(|next| next.start))
                    .unwrap_or(line.start)
                    .max(line.start);
                sung.extend(spread(&line.text, line.start, until));
            }
        }
        for lane in &line.secondary {
            match lane.words.as_deref().filter(|words| !words.is_empty()) {
                Some(words) => sung.extend(timed(words)),
                None => {
                    let until = lane.end.unwrap_or(lane.start).max(lane.start);
                    sung.extend(spread(&lane.text, lane.start, until));
                }
            }
        }
    }
    sung
}

fn timed(words: &[LyricsWord]) -> Vec<Sung> {
    words
        .iter()
        .flat_map(|word| {
            tokens(&word.text).into_iter().map(move |slot| Sung {
                start: word.start,
                end: word.end.max(word.start),
                key: slot.key,
            })
        })
        .collect()
}

fn spread(text: &str, start: Duration, end: Duration) -> Vec<Sung> {
    let slots = tokens(text);
    let total: usize = slots.iter().map(|slot| slot.key.chars().count()).sum();
    if total == 0 {
        return Vec::new();
    }
    let span = end.saturating_sub(start);
    let mut sung = Vec::with_capacity(slots.len());
    let mut passed = 0usize;
    for slot in slots {
        let length = slot.key.chars().count();
        let from = start + span.mul_f64(passed as f64 / total as f64);
        passed += length;
        let to = start + span.mul_f64(passed as f64 / total as f64);
        sung.push(Sung {
            start: from,
            end: to,
            key: slot.key,
        });
    }
    sung
}

fn tokens(text: &str) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut open: Option<Slot> = None;
    for (at, letter) in text.char_indices() {
        if wide(letter) {
            slots.extend(open.take());
            slots.push(Slot {
                at,
                key: letter.to_lowercase().collect(),
            });
            continue;
        }
        if TICKS.contains(&letter) {
            continue;
        }
        match letter.is_alphanumeric() {
            true => open
                .get_or_insert_with(|| Slot {
                    at,
                    key: String::new(),
                })
                .key
                .extend(letter.to_lowercase()),
            false => slots.extend(open.take()),
        }
    }
    slots.extend(open);
    slots
}

fn wide(letter: char) -> bool {
    matches!(letter,
        '\u{3040}'..='\u{30ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{f900}'..='\u{faff}')
}

fn paired(left: &[&str], right: &[&str]) -> Vec<(usize, usize)> {
    let (rows, columns) = (left.len() + 1, right.len() + 1);
    let mut table = vec![0u32; rows * columns];
    for row in (0..left.len()).rev() {
        for column in (0..right.len()).rev() {
            let at = row * columns + column;
            table[at] = match akin(left[row], right[column]) {
                true => table[at + columns + 1] + 1,
                false => table[at + columns].max(table[at + 1]),
            };
        }
    }

    let mut pairs = Vec::new();
    let (mut row, mut column) = (0, 0);
    while row < left.len() && column < right.len() {
        let at = row * columns + column;
        if akin(left[row], right[column]) {
            pairs.push((row, column));
            row += 1;
            column += 1;
            continue;
        }
        match table[at + columns] >= table[at + 1] {
            true => row += 1,
            false => column += 1,
        }
    }
    pairs
}

fn akin(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let (short, long) = match left.len() <= right.len() {
        true => (left, right),
        false => (right, left),
    };
    short.len() >= 4 && long.len() - short.len() <= 2 && long.starts_with(short)
}

fn hints(guide: &[LyricsLine], slotted: &[Vec<Slot>]) -> Vec<(Duration, Duration)> {
    let mut hints = Vec::new();
    for (index, line) in guide.iter().enumerate() {
        let slots = &slotted[index];
        if slots.is_empty() {
            continue;
        }
        let until = line
            .end
            .or_else(|| guide.get(index + 1).map(|next| next.start))
            .unwrap_or(line.start)
            .max(line.start);
        let span = until.saturating_sub(line.start);
        let total: usize = slots
            .iter()
            .map(|slot| slot.key.chars().count().max(1))
            .sum();
        let mut passed = 0usize;
        for slot in slots {
            let start = line.start + span.mul_f64(passed as f64 / total as f64);
            passed += slot.key.chars().count().max(1);
            let end = line.start + span.mul_f64(passed as f64 / total as f64);
            hints.push((start, end));
        }
    }
    hints
}

fn shared(after: Duration, before: Duration, count: usize) -> Vec<(Duration, Duration)> {
    let span = before.saturating_sub(after);
    (0..count)
        .map(|step| {
            let from = after + span.mul_f64(step as f64 / count as f64);
            let to = after + span.mul_f64((step + 1) as f64 / count as f64);
            (from, to)
        })
        .collect()
}

fn filled(
    spans: Vec<Option<(Duration, Duration)>>,
    hints: &[(Duration, Duration)],
) -> Vec<(Duration, Duration)> {
    let mut settled: Vec<(Duration, Duration)> = Vec::with_capacity(spans.len());
    let mut index = 0;
    while index < spans.len() {
        if let Some(span) = spans[index] {
            settled.push(span);
            index += 1;
            continue;
        }
        let mut until = index;
        while until < spans.len() && spans[until].is_none() {
            until += 1;
        }
        let after = settled.last().map(|(_, end)| *end);
        let before = spans
            .get(until)
            .and_then(|span| *span)
            .map(|(start, _)| start);
        match (after, before) {
            (Some(after), Some(before)) if before > after => {
                settled.extend(shared(after, before, until - index));
            }
            _ => {
                for at in index..until {
                    let (mut start, mut end) = hints
                        .get(at)
                        .copied()
                        .unwrap_or_else(|| (after.unwrap_or_default(), after.unwrap_or_default()));
                    if let Some(after) = after {
                        start = start.max(after);
                        end = end.max(start);
                    }
                    if let Some(before) = before {
                        start = start.min(before);
                        end = end.min(before).max(start);
                    }
                    let floor = settled.last().map(|(_, end)| *end).unwrap_or(start);
                    settled.push((start.max(floor), end.max(start.max(floor))));
                }
            }
        }
        index = until;
    }
    settled
}

fn worded_from(text: &str, slots: &[Slot], spans: &[(Duration, Duration)]) -> Vec<LyricsWord> {
    let mut words = Vec::with_capacity(slots.len());
    for (index, slot) in slots.iter().enumerate() {
        let from = match index {
            0 => 0,
            _ => slot.at,
        };
        let until = slots
            .get(index + 1)
            .map_or(text.len(), |next| next.at.max(from));
        let (start, end) = spans[index];
        words.push(LyricsWord {
            start,
            end: end.max(start),
            text: text[from..until].to_owned(),
        });
    }
    words
}

/// Splits a plain line into space-delimited or CJK breakable word fragments,
/// preserving exact spacing and characters when concatenated.
pub fn plain_lyrics_fragments(line: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut start = 0;
    let mut spacing = false;
    let mut previous = None;
    for (index, letter) in line.char_indices() {
        if letter.is_whitespace() {
            spacing = true;
        } else if spacing || previous.is_some_and(|prev| is_breakable_cjk(prev, letter)) {
            fragments.push(line[start..index].to_owned());
            start = index;
            spacing = false;
        }
        previous = Some(letter);
    }
    if start < line.len() {
        fragments.push(line[start..].to_owned());
    }
    fragments
}

fn is_breakable_cjk(left: char, right: char) -> bool {
    let is_cjk = |c: char| {
        matches!(c,
            '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE10}'..='\u{FE19}'
            | '\u{FE30}'..='\u{FE6F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
        )
    };
    if !is_cjk(left) || !is_cjk(right) {
        return false;
    }
    !matches!(
        right,
        '、' | '。'
            | '，'
            | '．'
            | '！'
            | '？'
            | '：'
            | '；'
            | '」'
            | '』'
            | '）'
            | '】'
            | '〉'
            | '》'
            | '〕'
            | '・'
            | 'ー'
            | '…'
            | '々'
            | 'ゝ'
            | 'ゞ'
            | 'っ'
            | 'ッ'
    ) && !matches!(left, '「' | '『' | '（' | '【' | '〈' | '《' | '〔')
}

fn count_syllables(word: &str) -> usize {
    let clean: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect();
    if clean.is_empty() {
        return 1;
    }

    let is_vowel = |c: char| {
        matches!(
            c,
            'a' | 'e'
                | 'i'
                | 'o'
                | 'u'
                | 'y'
                | 'а'
                | 'е'
                | 'ё'
                | 'и'
                | 'о'
                | 'у'
                | 'ы'
                | 'э'
                | 'ю'
                | 'я'
                | 'ä'
                | 'ö'
                | 'ü'
                | 'é'
                | 'è'
                | 'ê'
                | 'à'
                | 'â'
                | 'î'
                | 'ô'
                | 'ù'
                | 'û'
        )
    };
    let mut count = 0;
    let mut in_vowel_group = false;

    for c in clean.chars() {
        if is_vowel(c) {
            if !in_vowel_group {
                count += 1;
                in_vowel_group = true;
            }
        } else {
            in_vowel_group = false;
        }
    }

    if count > 1 && clean.ends_with('e') && !clean.ends_with("le") && !clean.ends_with("ee") {
        count -= 1;
    }

    count.max(1)
}

/// Estimates word-by-word timestamps for a line of synced lyrics that lacks word timings.
pub fn estimate_line_words(
    text: &str,
    start: Duration,
    end: Option<Duration>,
    next_start: Option<Duration>,
) -> Option<Vec<LyricsWord>> {
    let fragments = plain_lyrics_fragments(text);
    if fragments.is_empty() || fragments.iter().all(|f| f.trim().is_empty()) {
        return None;
    }

    let available = match (end, next_start) {
        (Some(end), Some(next)) => end.min(next).saturating_sub(start),
        (Some(end), None) => end.saturating_sub(start),
        (None, Some(next)) => next.saturating_sub(start),
        (None, None) => Duration::from_secs(4),
    };

    let mut total_syllables = 0usize;
    let weights: Vec<f64> = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| {
            let clean = fragment.trim();
            let syllables = count_syllables(clean);
            total_syllables += syllables;

            let mut weight = (syllables as f64) * 2.0;

            // Trailing punctuation denotes a phrasing pause / held breath
            if clean.ends_with(',') || clean.ends_with(';') || clean.ends_with(':') {
                weight += 1.0;
            } else if clean.ends_with("...") || clean.ends_with('—') || clean.ends_with('-') {
                weight += 1.5;
            } else if clean.ends_with('!') || clean.ends_with('?') || clean.ends_with('.') {
                weight += 1.0;
            }

            // The final word in a line sustains across the remainder of the measure
            if index + 1 == fragments.len() {
                weight += 1.5;
            }

            weight
        })
        .collect();

    let estimated = Duration::from_millis((total_syllables as u64 * 320) + 400);

    let singing_duration = if available <= estimated {
        let pause = Duration::from_millis(200).min(available.mul_f64(0.08));
        available
            .saturating_sub(pause)
            .max(Duration::from_millis(400))
    } else if available >= Duration::from_secs(7) && available >= estimated + Duration::from_secs(3)
    {
        let max_sing = available.saturating_sub(Duration::from_secs(2));
        estimated.mul_f64(1.25).min(max_sing)
    } else {
        let pause = Duration::from_millis(280).min(available.mul_f64(0.08));
        available.saturating_sub(pause).max(estimated)
    };

    let total_weight: f64 = weights.iter().sum();
    if total_weight <= 0.0 {
        return None;
    }

    let mut words = Vec::with_capacity(fragments.len());
    let mut elapsed = Duration::ZERO;
    for (index, fragment) in fragments.into_iter().enumerate() {
        let weight = weights[index];
        let share = weight / total_weight;
        let delta = singing_duration.mul_f64(share);
        let word_start = start + elapsed;
        let word_end = if index + 1 == weights.len() {
            start + singing_duration
        } else {
            word_start + delta
        };
        elapsed += delta;
        words.push(LyricsWord {
            start: word_start,
            end: word_end.max(word_start),
            text: fragment,
        });
    }

    Some(words)
}

/// Populates estimated words for any lines in a synced lyrics sheet that lack word timings.
pub fn estimate_words_if_needed(lines: &[LyricsLine]) -> Vec<LyricsLine> {
    let mut prepared = lines.to_vec();
    for index in 0..prepared.len() {
        let next_start = lines.get(index + 1).map(|next| next.start);
        if prepared[index]
            .words
            .as_ref()
            .map_or(true, |w| w.is_empty())
        {
            prepared[index].words = estimate_line_words(
                &prepared[index].text,
                prepared[index].start,
                prepared[index].end,
                next_start,
            );
        }
        let line_end = prepared[index].end;
        for lane in &mut prepared[index].secondary {
            if lane.words.as_ref().map_or(true, |w| w.is_empty()) {
                lane.words =
                    estimate_line_words(&lane.text, lane.start, lane.end.or(line_end), next_start);
            }
        }
    }
    prepared
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    #[test]
    fn estimates_words_for_plain_synced_line() {
        let text = "Hold to the time that you know";
        let start = Duration::from_secs(10);
        let next = Duration::from_secs(14);
        let words =
            estimate_line_words(text, start, Some(next), Some(next)).expect("should produce words");

        assert_eq!(words.len(), 7);
        let joined: String = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(joined, text);

        for (i, word) in words.iter().enumerate() {
            assert!(word.start >= start);
            assert!(word.end >= word.start);
            if i + 1 < words.len() {
                assert!(words[i + 1].start >= word.start);
            }
        }
        assert!(words.last().unwrap().end <= next);
    }

    #[test]
    fn preserves_existing_word_timings() {
        let existing = LyricsWord {
            start: Duration::from_secs(2),
            end: Duration::from_secs(3),
            text: "Hello".to_owned(),
        };
        let lines = vec![LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(5)),
            text: "Hello world".to_owned(),
            romanized: None,
            words: Some(vec![existing.clone()]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        }];

        let result = estimate_words_if_needed(&lines);
        assert_eq!(result[0].words.as_ref().unwrap().len(), 1);
        assert_eq!(result[0].words.as_ref().unwrap()[0], existing);
    }
}
