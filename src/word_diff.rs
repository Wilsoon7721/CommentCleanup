pub type Piece = (String, bool);

const MAX_CELLS: usize = 1_000_000;

fn tokens(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut previous: Option<u8> = None;
    for (index, c) in text.char_indices() {
        let class = match c {
            c if c.is_whitespace() => 0,
            c if c.is_alphanumeric() || c == '_' => 1,
            _ => 2,
        };
        if previous.is_some_and(|last| last != class || class == 2) {
            found.push(&text[start..index]);
            start = index;
        }
        previous = Some(class);
    }
    if start < text.len() {
        found.push(&text[start..]);
    }
    found
}

fn joined(tokens: &[&str], changed: &[bool]) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for (token, &flag) in tokens.iter().zip(changed) {
        match pieces.last_mut() {
            Some((text, last)) if *last == flag => text.push_str(token),
            _ => pieces.push(((*token).to_owned(), flag)),
        }
    }
    pieces
}

pub fn compare(before: &str, after: &str) -> (Vec<Piece>, Vec<Piece>) {
    let (old, new) = (tokens(before), tokens(after));
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (old_middle, new_middle) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    let mut old_changed = vec![false; old.len()];
    let mut new_changed = vec![false; new.len()];
    let (rows, columns) = (old_middle.len(), new_middle.len());
    if rows.saturating_mul(columns) > MAX_CELLS {
        old_changed[prefix..old.len() - suffix].fill(true);
        new_changed[prefix..new.len() - suffix].fill(true);
        return (joined(&old, &old_changed), joined(&new, &new_changed));
    }
    let mut lengths = vec![0u32; (rows + 1) * (columns + 1)];
    let at = |row: usize, column: usize| row * (columns + 1) + column;
    for row in (0..rows).rev() {
        for column in (0..columns).rev() {
            lengths[at(row, column)] = if old_middle[row] == new_middle[column] {
                lengths[at(row + 1, column + 1)] + 1
            } else {
                lengths[at(row + 1, column)].max(lengths[at(row, column + 1)])
            };
        }
    }
    let (mut row, mut column) = (0, 0);
    while row < rows || column < columns {
        if row < rows && column < columns && old_middle[row] == new_middle[column] {
            row += 1;
            column += 1;
        } else if column == columns || (row < rows && lengths[at(row + 1, column)] >= lengths[at(row, column + 1)]) {
            old_changed[prefix + row] = true;
            row += 1;
        } else {
            new_changed[prefix + column] = true;
            column += 1;
        }
    }
    (joined(&old, &old_changed), joined(&new, &new_changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(pieces: &[Piece]) -> Vec<&str> {
        pieces.iter().filter(|(_, changed)| *changed).map(|(text, _)| text.as_str()).collect()
    }

    fn whole(pieces: &[Piece]) -> String {
        pieces.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn only_the_words_that_differ_are_marked() {
        let (before, after) = compare("// reads the old file", "// reads the new file");
        assert_eq!(marked(&before), ["old"]);
        assert_eq!(marked(&after), ["new"]);
    }

    #[test]
    fn the_text_is_never_altered() {
        let (a, b) = ("/* one\n * two */", "// one two — três");
        let (before, after) = compare(a, b);
        assert_eq!(whole(&before), a);
        assert_eq!(whole(&after), b);
    }

    #[test]
    fn identical_text_has_nothing_marked() {
        let (before, after) = compare("// same", "// same");
        assert!(marked(&before).is_empty() && marked(&after).is_empty());
    }

    #[test]
    fn words_added_in_the_middle_leave_the_rest_alone() {
        let (before, after) = compare("// keep this", "// keep all of this");
        assert!(marked(&before).is_empty());
        assert_eq!(marked(&after), ["all of "]);
    }

    #[test]
    fn huge_comments_fall_back_to_marking_the_middle() {
        let a = (0..1500).map(|n| format!("a{n}")).collect::<Vec<_>>().join(" ");
        let b = (0..1500).map(|n| format!("b{n}")).collect::<Vec<_>>().join(" ");
        let (before, after) = compare(&a, &b);
        assert_eq!(whole(&before), a);
        assert_eq!(whole(&after), b);
    }
}
