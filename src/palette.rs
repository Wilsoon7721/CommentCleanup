//! Ranking for the command palette.

pub fn score(query: &str, label: &str) -> Option<i32> {
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let mut position = 0;
    let mut total = 0;
    let mut previous: Option<usize> = None;
    for wanted in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = position + label[position..].iter().position(|c| *c == wanted)?;
        total += 1;
        if previous.is_some_and(|last| last + 1 == found) {
            total += 6;
        }
        if found == 0 || !label[found - 1].is_alphanumeric() {
            total += 8;
        }
        previous = Some(found);
        position = found + 1;
    }
    Some(total * 100 - label.len() as i32)
}

/// Indexes of the labels that match, best first. Ties keep their original order.
pub fn rank<'a>(query: &str, labels: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> =
        labels.enumerate().filter_map(|(index, label)| Some((score(query, label)?, index))).collect();
    if query.trim().is_empty() {
        return scored.into_iter().map(|(_, index)| index).collect();
    }
    scored.sort_by_key(|(score, index)| (-score, *index));
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: [&str; 5] = ["Find", "Find in All Files", "Fix All in This File", "Settings", "Go Through All"];

    fn best(query: &str) -> Vec<&'static str> {
        rank(query, LABELS.iter().copied()).into_iter().map(|index| LABELS[index]).collect()
    }

    #[test]
    fn letters_must_appear_in_order() {
        assert!(score("stg", "Settings").is_some());
        assert!(score("gts", "Settings").is_none());
    }

    #[test]
    fn an_empty_query_keeps_every_command_in_its_order() {
        assert_eq!(best(""), LABELS);
    }

    #[test]
    fn word_starts_and_exact_words_win() {
        assert_eq!(best("find")[0], "Find");
        assert_eq!(best("fiaf")[0], "Find in All Files");
        assert_eq!(best("gta"), ["Go Through All"]);
        assert_eq!(best("SETT"), ["Settings"]);
    }

    #[test]
    fn nothing_matching_gives_nothing() {
        assert!(best("zzz").is_empty());
    }
}
